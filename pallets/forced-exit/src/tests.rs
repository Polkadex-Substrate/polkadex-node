// This file is part of Polkadex.
//
// Copyright (c) 2026 the polkadex-node contributors.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

//! Tests for the forced-exit escape hatch.
//!
//! The suite is organised around the threat model: the mechanism is only worth having if a
//! user can always get out, and only safe if nobody can get out with more than they own.

use crate::{
	merkle,
	mock::*,
	traits::{Custody, SettlementNotifier},
	types::{BalanceLeaf, FreezeEvidence, ProofNode},
	Error, Freeze, LastFinalized, UnprocessedDeposits,
};
use frame_support::{assert_noop, assert_ok};
use polkadex_primitives::AssetId;
use sp_core::H256;

/// Builds a leaf set, commits its root as a finalized snapshot, and returns the leaves.
fn commit_book(
	balances: Vec<(u64, AssetId, u128, u128)>,
	state_change_id: u64,
) -> Vec<BalanceLeaf<u64>> {
	let mut leaves: Vec<BalanceLeaf<u64>> = balances
		.into_iter()
		.map(|(account, asset, free, in_orders)| BalanceLeaf { account, asset, free, in_orders })
		.collect();
	let root = merkle::compute_root(&mut leaves);
	<ForcedExit as SettlementNotifier<u64>>::on_snapshot_finalized(root, state_change_id);
	leaves
}

/// Produces the inclusion proof for one leaf of a committed book.
fn prove(leaves: &[BalanceLeaf<u64>], target: &BalanceLeaf<u64>) -> Vec<ProofNode> {
	let mut leaves = leaves.to_vec();
	merkle::build_proof(&mut leaves, target).expect("leaf is in the book")
}

// --- merkle format -------------------------------------------------------------------

#[test]
fn merkle_root_is_order_independent_and_proofs_verify() {
	let leaves = vec![
		BalanceLeaf { account: ALICE, asset: USDT, free: 10 * UNIT, in_orders: 0 },
		BalanceLeaf { account: BOB, asset: USDT, free: 5 * UNIT, in_orders: 2 * UNIT },
		BalanceLeaf { account: CHARLIE, asset: BTC, free: 1 * UNIT, in_orders: 0 },
	];

	let mut forward = leaves.clone();
	let mut reversed: Vec<_> = leaves.iter().cloned().rev().collect();
	assert_eq!(merkle::compute_root(&mut forward), merkle::compute_root(&mut reversed));

	let root = merkle::compute_root(&mut forward);
	for leaf in &leaves {
		let proof = prove(&leaves, leaf);
		assert_eq!(merkle::root_from_proof(leaf, &proof), root);
	}
}

#[test]
fn odd_leaf_counts_promote_rather_than_duplicate() {
	// A duplicated trailing leaf would let a 3-leaf book collide with a 4-leaf book whose
	// last two leaves are identical. Promotion makes the two roots differ.
	let mut three = vec![
		BalanceLeaf { account: ALICE, asset: USDT, free: 1, in_orders: 0 },
		BalanceLeaf { account: BOB, asset: USDT, free: 2, in_orders: 0 },
		BalanceLeaf { account: CHARLIE, asset: USDT, free: 3, in_orders: 0 },
	];
	let mut four = three.clone();
	four.push(BalanceLeaf { account: CHARLIE, asset: USDT, free: 3, in_orders: 0 });

	assert_ne!(merkle::compute_root(&mut three), merkle::compute_root(&mut four));
}

// --- withdrawal requests -------------------------------------------------------------

#[test]
fn request_withdrawal_records_and_reserves() {
	new_test_ext().execute_with(|| {
		assert_ok!(ForcedExit::request_withdrawal(
			RuntimeOrigin::signed(ALICE),
			USDT,
			5 * UNIT
		));
		assert_eq!(Balances::reserved_balance(ALICE), 10);
		assert!(crate::Requests::<Test>::get(0).is_some());
	});
}

#[test]
fn pending_requests_are_bounded() {
	new_test_ext().execute_with(|| {
		for _ in 0..4 {
			assert_ok!(ForcedExit::request_withdrawal(RuntimeOrigin::signed(ALICE), USDT, UNIT));
		}
		assert_noop!(
			ForcedExit::request_withdrawal(RuntimeOrigin::signed(ALICE), USDT, UNIT),
			Error::<Test>::TooManyRequests
		);
	});
}

#[test]
fn servicing_requests_refunds_the_deposit() {
	new_test_ext().execute_with(|| {
		assert_ok!(ForcedExit::request_withdrawal(RuntimeOrigin::signed(ALICE), USDT, UNIT));
		assert_eq!(Balances::reserved_balance(ALICE), 10);

		<ForcedExit as SettlementNotifier<u64>>::on_requests_serviced(&ALICE, 0);

		assert_eq!(Balances::reserved_balance(ALICE), 0);
		assert!(crate::Requests::<Test>::get(0).is_none());
	});
}

// --- freeze conditions ---------------------------------------------------------------

#[test]
fn snapshot_liveness_freeze_respects_the_boundary() {
	new_test_ext().execute_with(|| {
		commit_book(vec![(ALICE, USDT, 10 * UNIT, 0)], 1);
		let finalized_at = LastFinalized::<Test>::get().unwrap().at;

		// Exactly at the timeout is not yet a failure.
		run_to_block(finalized_at + 200);
		assert_noop!(
			ForcedExit::trigger_settlement_freeze(
				RuntimeOrigin::signed(BOB),
				FreezeEvidence::SnapshotLiveness
			),
			Error::<Test>::FreezeConditionNotMet
		);

		// One block past it, anyone may trip the freeze.
		run_to_block(finalized_at + 201);
		assert_ok!(ForcedExit::trigger_settlement_freeze(
			RuntimeOrigin::signed(BOB),
			FreezeEvidence::SnapshotLiveness
		));
		assert!(Freeze::<Test>::get().is_frozen());
	});
}

#[test]
fn unserviced_request_freeze_respects_the_boundary() {
	new_test_ext().execute_with(|| {
		commit_book(vec![(ALICE, USDT, 10 * UNIT, 0)], 1);
		assert_ok!(ForcedExit::request_withdrawal(RuntimeOrigin::signed(ALICE), USDT, UNIT));

		run_to_block(101);
		assert_noop!(
			ForcedExit::trigger_settlement_freeze(
				RuntimeOrigin::signed(ALICE),
				FreezeEvidence::UnservicedRequest(0)
			),
			Error::<Test>::FreezeConditionNotMet
		);

		run_to_block(102);
		assert_ok!(ForcedExit::trigger_settlement_freeze(
			RuntimeOrigin::signed(ALICE),
			FreezeEvidence::UnservicedRequest(0)
		));
	});
}

#[test]
fn freeze_evidence_must_reference_a_real_request() {
	new_test_ext().execute_with(|| {
		run_to_block(10_000);
		assert_noop!(
			ForcedExit::trigger_settlement_freeze(
				RuntimeOrigin::signed(ALICE),
				FreezeEvidence::UnservicedRequest(42)
			),
			Error::<Test>::UnknownRequest
		);
	});
}

// --- forced exit ---------------------------------------------------------------------

fn freeze_now() {
	run_to_block(100_000);
	assert_ok!(ForcedExit::trigger_settlement_freeze(
		RuntimeOrigin::signed(BOB),
		FreezeEvidence::SnapshotLiveness
	));
}

#[test]
fn forced_exit_pays_free_plus_locked_balance() {
	new_test_ext().execute_with(|| {
		fund_custody(USDT, 100 * UNIT);
		let leaves = commit_book(
			vec![(ALICE, USDT, 7 * UNIT, 3 * UNIT), (BOB, USDT, 5 * UNIT, 0)],
			1,
		);
		freeze_now();

		let alice_leaf = leaves.iter().find(|l| l.account == ALICE).unwrap().clone();
		let proof = prove(&leaves, &alice_leaf);

		assert_ok!(ForcedExit::force_withdraw(
			RuntimeOrigin::signed(ALICE),
			USDT,
			7 * UNIT,
			3 * UNIT,
			proof
		));
		// Locked funds are payable because a frozen venue has no open orders.
		assert_eq!(released(ALICE, USDT), 10 * UNIT);
	});
}

#[test]
fn forced_exit_is_unavailable_while_the_venue_is_live() {
	new_test_ext().execute_with(|| {
		fund_custody(USDT, 100 * UNIT);
		let leaves = commit_book(vec![(ALICE, USDT, 7 * UNIT, 0)], 1);
		let leaf = leaves[0].clone();
		let proof = prove(&leaves, &leaf);

		assert_noop!(
			ForcedExit::force_withdraw(RuntimeOrigin::signed(ALICE), USDT, 7 * UNIT, 0, proof),
			Error::<Test>::SettlementNotFrozen
		);
	});
}

#[test]
fn inflated_balance_claims_are_rejected() {
	new_test_ext().execute_with(|| {
		fund_custody(USDT, 100 * UNIT);
		let leaves = commit_book(vec![(ALICE, USDT, 7 * UNIT, 0), (BOB, USDT, 5 * UNIT, 0)], 1);
		freeze_now();

		let alice_leaf = leaves.iter().find(|l| l.account == ALICE).unwrap().clone();
		let proof = prove(&leaves, &alice_leaf);

		// Same proof, larger claimed balance: the leaf hash changes, so the root does not match.
		assert_noop!(
			ForcedExit::force_withdraw(
				RuntimeOrigin::signed(ALICE),
				USDT,
				700 * UNIT,
				0,
				proof
			),
			Error::<Test>::InvalidProof
		);
	});
}

#[test]
fn one_users_proof_cannot_be_replayed_by_another() {
	new_test_ext().execute_with(|| {
		fund_custody(USDT, 100 * UNIT);
		let leaves = commit_book(vec![(ALICE, USDT, 7 * UNIT, 0), (BOB, USDT, 5 * UNIT, 0)], 1);
		freeze_now();

		let alice_leaf = leaves.iter().find(|l| l.account == ALICE).unwrap().clone();
		let proof = prove(&leaves, &alice_leaf);

		// The leaf binds the account, so Bob cannot present Alice's path.
		assert_noop!(
			ForcedExit::force_withdraw(RuntimeOrigin::signed(BOB), USDT, 7 * UNIT, 0, proof),
			Error::<Test>::InvalidProof
		);
	});
}

#[test]
fn traded_away_balance_is_not_claimable() {
	new_test_ext().execute_with(|| {
		fund_custody(USDT, 100 * UNIT);
		fund_custody(BTC, 100 * UNIT);

		// Alice starts with USDT and no BTC.
		let opening = commit_book(
			vec![(ALICE, USDT, 10 * UNIT, 0), (BOB, BTC, 4 * UNIT, 0)],
			1,
		);
		let opening_usdt = opening.iter().find(|l| l.account == ALICE).unwrap().clone();
		let stale_proof = prove(&opening, &opening_usdt);

		// She spends the USDT for BTC; the next finalized book records the swap.
		let closing = commit_book(
			vec![(ALICE, BTC, 2 * UNIT, 0), (BOB, USDT, 10 * UNIT, 0), (BOB, BTC, 2 * UNIT, 0)],
			2,
		);
		freeze_now();

		// The spent USDT cannot be recovered: proofs are only accepted against the current
		// finalized root, and Alice no longer has a USDT leaf in it.
		assert_noop!(
			ForcedExit::force_withdraw(
				RuntimeOrigin::signed(ALICE),
				USDT,
				10 * UNIT,
				0,
				stale_proof
			),
			Error::<Test>::InvalidProof
		);

		// What she actually owns is claimable.
		let alice_btc = closing.iter().find(|l| l.account == ALICE).unwrap().clone();
		let proof = prove(&closing, &alice_btc);
		assert_ok!(ForcedExit::force_withdraw(
			RuntimeOrigin::signed(ALICE),
			BTC,
			2 * UNIT,
			0,
			proof
		));
		assert_eq!(released(ALICE, BTC), 2 * UNIT);
		assert_eq!(released(ALICE, USDT), 0);
	});
}

#[test]
fn double_claims_are_rejected() {
	new_test_ext().execute_with(|| {
		fund_custody(USDT, 100 * UNIT);
		let leaves = commit_book(vec![(ALICE, USDT, 7 * UNIT, 0), (BOB, USDT, 1 * UNIT, 0)], 1);
		freeze_now();

		let leaf = leaves.iter().find(|l| l.account == ALICE).unwrap().clone();
		let proof = prove(&leaves, &leaf);

		assert_ok!(ForcedExit::force_withdraw(
			RuntimeOrigin::signed(ALICE),
			USDT,
			7 * UNIT,
			0,
			proof.clone()
		));
		assert_noop!(
			ForcedExit::force_withdraw(RuntimeOrigin::signed(ALICE), USDT, 7 * UNIT, 0, proof),
			Error::<Test>::AlreadyExited
		);
		assert_eq!(released(ALICE, USDT), 7 * UNIT);
	});
}

#[test]
fn deposits_made_after_the_last_snapshot_are_recoverable() {
	new_test_ext().execute_with(|| {
		fund_custody(USDT, 100 * UNIT);
		let leaves = commit_book(vec![(ALICE, USDT, 2 * UNIT, 0), (BOB, USDT, 1 * UNIT, 0)], 1);

		// The chain witnessed this deposit; no snapshot covers it yet.
		<ForcedExit as SettlementNotifier<u64>>::on_deposit(&ALICE, USDT, 5 * UNIT);
		freeze_now();

		let leaf = leaves.iter().find(|l| l.account == ALICE).unwrap().clone();
		let proof = prove(&leaves, &leaf);

		assert_ok!(ForcedExit::force_withdraw(
			RuntimeOrigin::signed(ALICE),
			USDT,
			2 * UNIT,
			0,
			proof
		));
		assert_eq!(released(ALICE, USDT), 7 * UNIT);
		assert_eq!(UnprocessedDeposits::<Test>::get(ALICE, USDT), 0);
	});
}

#[test]
fn settled_deposits_are_not_paid_twice() {
	new_test_ext().execute_with(|| {
		fund_custody(USDT, 100 * UNIT);
		<ForcedExit as SettlementNotifier<u64>>::on_deposit(&ALICE, USDT, 5 * UNIT);

		// The engine folds the deposit into the next book and says so.
		let leaves = commit_book(vec![(ALICE, USDT, 5 * UNIT, 0), (BOB, USDT, 1 * UNIT, 0)], 2);
		<ForcedExit as SettlementNotifier<u64>>::on_deposits_settled(&ALICE, USDT, 5 * UNIT);
		freeze_now();

		let leaf = leaves.iter().find(|l| l.account == ALICE).unwrap().clone();
		let proof = prove(&leaves, &leaf);

		assert_ok!(ForcedExit::force_withdraw(
			RuntimeOrigin::signed(ALICE),
			USDT,
			5 * UNIT,
			0,
			proof
		));
		// Paid once from the leaf, not again from the pending tally.
		assert_eq!(released(ALICE, USDT), 5 * UNIT);
	});
}

#[test]
fn custody_shortfall_is_reported_and_pays_what_exists() {
	new_test_ext().execute_with(|| {
		// Custody holds less than the book commits: an engine or bridge fault.
		fund_custody(USDT, 3 * UNIT);
		let leaves = commit_book(vec![(ALICE, USDT, 10 * UNIT, 0), (BOB, USDT, 1 * UNIT, 0)], 1);
		freeze_now();

		let leaf = leaves.iter().find(|l| l.account == ALICE).unwrap().clone();
		let proof = prove(&leaves, &leaf);

		assert_ok!(ForcedExit::force_withdraw(
			RuntimeOrigin::signed(ALICE),
			USDT,
			10 * UNIT,
			0,
			proof
		));
		assert_eq!(released(ALICE, USDT), 3 * UNIT);
		System::assert_has_event(
			crate::Event::CustodyShortfall {
				who: ALICE,
				asset: USDT,
				owed: 10 * UNIT,
				paid: 3 * UNIT,
			}
			.into(),
		);
	});
}

// --- snapshot integrity --------------------------------------------------------------

#[test]
fn snapshot_rewind_is_ignored() {
	new_test_ext().execute_with(|| {
		commit_book(vec![(ALICE, USDT, 1 * UNIT, 0)], 5);
		let current = LastFinalized::<Test>::get().unwrap();

		// An older state-change id must never replace a newer one.
		commit_book(vec![(ALICE, USDT, 999 * UNIT, 0)], 4);
		assert_eq!(LastFinalized::<Test>::get().unwrap().balances_root, current.balances_root);
	});
}

#[test]
fn snapshots_are_refused_once_frozen() {
	new_test_ext().execute_with(|| {
		commit_book(vec![(ALICE, USDT, 1 * UNIT, 0)], 1);
		let frozen_root = LastFinalized::<Test>::get().unwrap().balances_root;
		freeze_now();

		commit_book(vec![(ALICE, USDT, 999 * UNIT, 0)], 2);
		assert_eq!(LastFinalized::<Test>::get().unwrap().balances_root, frozen_root);
	});
}

#[test]
fn withdrawal_requests_are_refused_once_frozen() {
	new_test_ext().execute_with(|| {
		commit_book(vec![(ALICE, USDT, 1 * UNIT, 0)], 1);
		freeze_now();
		assert_noop!(
			ForcedExit::request_withdrawal(RuntimeOrigin::signed(ALICE), USDT, UNIT),
			Error::<Test>::SettlementFrozen
		);
	});
}

// --- resume --------------------------------------------------------------------------

#[test]
fn resume_requires_governance_and_lapses_prior_claims() {
	new_test_ext().execute_with(|| {
		fund_custody(USDT, 100 * UNIT);
		let leaves = commit_book(vec![(ALICE, USDT, 4 * UNIT, 0), (BOB, USDT, 1 * UNIT, 0)], 1);
		freeze_now();

		let leaf = leaves.iter().find(|l| l.account == ALICE).unwrap().clone();
		let proof = prove(&leaves, &leaf);
		assert_ok!(ForcedExit::force_withdraw(
			RuntimeOrigin::signed(ALICE),
			USDT,
			4 * UNIT,
			0,
			proof
		));

		// A user cannot resume the venue.
		assert_noop!(
			ForcedExit::resume_settlement(RuntimeOrigin::signed(ALICE), H256::repeat_byte(9), 2),
			sp_runtime::DispatchError::BadOrigin
		);

		// Governance restarts it under a fresh book covering the un-exited remainder.
		let mut remainder = vec![BalanceLeaf { account: BOB, asset: USDT, free: 1 * UNIT, in_orders: 0 }];
		let new_root = merkle::compute_root(&mut remainder);
		assert_ok!(ForcedExit::resume_settlement(RuntimeOrigin::root(), new_root, 2));
		assert!(!Freeze::<Test>::get().is_frozen());

		// The epoch bumped, so a later freeze allows a fresh exit under the new book.
		run_to_block(300_000);
		assert_ok!(ForcedExit::trigger_settlement_freeze(
			RuntimeOrigin::signed(BOB),
			FreezeEvidence::SnapshotLiveness
		));
		let bob_leaf = remainder[0].clone();
		let bob_proof = prove(&remainder, &bob_leaf);
		assert_ok!(ForcedExit::force_withdraw(
			RuntimeOrigin::signed(BOB),
			USDT,
			1 * UNIT,
			0,
			bob_proof
		));
		assert_eq!(released(BOB, USDT), 1 * UNIT);
	});
}

// --- the property that matters -------------------------------------------------------

#[test]
fn every_account_recovers_its_full_balance_when_the_engine_dies() {
	new_test_ext().execute_with(|| {
		let account_count = 100u64;
		let mut book = Vec::new();
		let mut expected = 0u128;
		for account in 1..=account_count {
			let free = (account as u128) * UNIT;
			let in_orders = (account as u128) * UNIT / 2;
			book.push((account, USDT, free, in_orders));
			expected += free + in_orders;
		}
		fund_custody(USDT, expected);

		let leaves = commit_book(book, 1);

		// The engine stops publishing. Nobody privileged does anything.
		freeze_now();

		for account in 1..=account_count {
			let leaf = leaves.iter().find(|l| l.account == account).unwrap().clone();
			let proof = prove(&leaves, &leaf);
			assert_ok!(ForcedExit::force_withdraw(
				RuntimeOrigin::signed(account),
				USDT,
				leaf.free,
				leaf.in_orders,
				proof
			));
			assert_eq!(released(account, USDT), leaf.free + leaf.in_orders);
		}

		// Custody is drained to the unit: no shortfall, no residue.
		assert_eq!(MockCustody::custody_balance(USDT), 0);
	});
}
