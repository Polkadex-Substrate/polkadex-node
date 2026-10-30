// This file is part of Polkadex.
//
// Copyright (C) 2026 The Polkadex Authors.
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

//! Runtime 393: democracy and council votes count PDEX bonded for staking. From runtime 392 the
//! bond is on a hold, outside the free balance; `StakedBalance` adds it back for the vote checks.

// The voting pallets and the pre-392 stash set-up use the legacy Currency traits.
#![allow(deprecated)]

use crate::*;
use frame_support::{
	assert_noop, assert_ok,
	dispatch::DispatchInfo,
	traits::{
		fungible::Mutate, Hooks, LockableCurrency, ReservableCurrency, StorePreimage,
		WithdrawReasons,
	},
};
use frame_system::pallet_prelude::BlockNumberFor;
use pallet_democracy::{
	AccountVote, Conviction, ReferendumIndex, ReferendumInfo, Vote, VoteThreshold,
};
use pallet_transaction_payment::OnChargeTransaction;
use parity_scale_codec::Encode;
use sp_runtime::{transaction_validity::TransactionValidityError, MultiAddress, TokenError};

const PDEX: Balance = crate::constants::currency::PDEX;
const STAKING_LOCK: [u8; 8] = *b"staking ";
const DEMOCRACY_LOCK: [u8; 8] = *b"democrac";

fn account(n: u8) -> AccountId {
	AccountId::from([n; 32])
}

fn signed(n: u8) -> RuntimeOrigin {
	RuntimeOrigin::signed(account(n))
}

fn new_ext() -> sp_io::TestExternalities {
	let mut ext: sp_io::TestExternalities =
		frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap().into();
	ext.execute_with(|| frame_system::Pallet::<Runtime>::set_block_number(1));
	ext
}

fn set_free(n: u8, amount: Balance) -> AccountId {
	let who = account(n);
	let _ = <Balances as Mutate<AccountId>>::set_balance(&who, amount);
	who
}

fn staked(who: &AccountId) -> Balance {
	pallet_staking::asset::staked::<Runtime>(who)
}

/// `n` holds `total` PDEX and bonds `bonded` of it, which the staking hold keeps from 392 on.
fn staker(n: u8, total: Balance, bonded: Balance) -> AccountId {
	let who = set_free(n, total);
	assert_ok!(Staking::bond(signed(n), bonded, pallet_staking::RewardDestination::Staked));
	assert_eq!(staked(&who), bonded);
	who
}

/// Puts a stash back in its pre-392 form, the bond under the `staking ` lock inside the free
/// balance, as pallet-staking's `testing_utils::migrate_to_old_currency` does.
fn to_lock(who: &AccountId) {
	let (bond, reasons) = (staked(who), WithdrawReasons::all());
	<Balances as LockableCurrency<AccountId>>::set_lock(STAKING_LOCK, who, bond, reasons);
	assert_ok!(pallet_staking::asset::kill_stake::<Runtime>(who));
	assert_ok!(frame_system::Pallet::<Runtime>::inc_consumers(who));
}

fn lock_of(who: &AccountId, id: [u8; 8]) -> Balance {
	pallet_balances::Locks::<Runtime>::get(who).iter().find(|l| l.id == id).map_or(0, |l| l.amount)
}

/// A referendum on a remark, enacted one block after it ends.
fn referendum() -> ReferendumIndex {
	let call = RuntimeCall::System(frame_system::Call::remark { remark: vec![] });
	let proposal = <Preimage as StorePreimage>::bound(call).unwrap();
	Democracy::internal_start_referendum(proposal, VoteThreshold::SimpleMajority, 1)
}

fn aye(balance: Balance) -> AccountVote<Balance> {
	AccountVote::Standard { vote: Vote { aye: true, conviction: Conviction::Locked1x }, balance }
}

/// Ayes, nays and turnout of an ongoing referendum.
fn tally(r: ReferendumIndex) -> (Balance, Balance, Balance) {
	match pallet_democracy::ReferendumInfoOf::<Runtime>::get(r) {
		Some(ReferendumInfo::Ongoing(s)) => (s.tally.ayes, s.tally.nays, s.tally.turnout),
		_ => panic!("referendum {r} is not ongoing"),
	}
}

/// Runs the referendum to its end block, where it passes; returns that block.
fn end_referendum(r: ReferendumIndex) -> BlockNumberFor<Runtime> {
	let end = match pallet_democracy::ReferendumInfoOf::<Runtime>::get(r) {
		Some(ReferendumInfo::Ongoing(s)) => s.end,
		_ => panic!("referendum {r} is not ongoing"),
	};
	frame_system::Pallet::<Runtime>::set_block_number(end);
	<Democracy as Hooks<BlockNumberFor<Runtime>>>::on_initialize(end);
	assert!(matches!(
		pallet_democracy::ReferendumInfoOf::<Runtime>::get(r),
		Some(ReferendumInfo::Finished { approved: true, .. })
	));
	end
}

/// `n` stands for the council, reserving the candidacy bond.
fn candidate(n: u8) -> AccountId {
	let who = set_free(n, 200 * PDEX);
	let count = pallet_elections_phragmen::Candidates::<Runtime>::decode_len().unwrap_or(0) as u32;
	assert_ok!(Elections::submit_candidacy(signed(n), count));
	who
}

/// The council voting bond for `votes` candidates.
fn voting_bond(votes: Balance) -> Balance {
	<Runtime as pallet_elections_phragmen::Config>::VotingBondBase::get() +
		<Runtime as pallet_elections_phragmen::Config>::VotingBondFactor::get() * votes
}

fn council_stake(who: &AccountId) -> Balance {
	pallet_elections_phragmen::Voting::<Runtime>::get(who).stake
}

fn transfer(n: u8, amount: Balance) -> sp_runtime::DispatchResult {
	Balances::transfer_keep_alive(signed(n), MultiAddress::Id(account(99)), amount)
}

/// Takes a transaction fee the way the runtime's transaction payment does.
fn fee(who: &AccountId, amount: Balance) -> Result<(), TransactionValidityError> {
	let call = RuntimeCall::System(frame_system::Call::remark { remark: vec![] });
	<<Runtime as pallet_transaction_payment::Config>::OnChargeTransaction as OnChargeTransaction<
		Runtime,
	>>::withdraw_fee(who, &call, &DispatchInfo::default(), amount, 0)
	.map(drop)
}

// Brief test 1: a stash fully bonded on the hold by migrate_currency votes with all of it.
#[test]
fn migrated_stash_votes_with_its_whole_bond() {
	new_ext().execute_with(|| {
		let stash = staker(1, 1_001 * PDEX, 1_000 * PDEX);
		to_lock(&stash);
		assert_ok!(Staking::migrate_currency(signed(9), stash.clone()));
		assert_eq!((lock_of(&stash, STAKING_LOCK), staked(&stash)), (0, 1_000 * PDEX));
		// Only the existential deposit is free, and 392 checked votes against that alone.
		assert_eq!(Balances::free_balance(&stash), PDEX);

		let r = referendum();
		assert_ok!(Democracy::vote(signed(1), r, aye(1_001 * PDEX)));
		assert_eq!(tally(r), (1_001 * PDEX, 0, 1_001 * PDEX));
		assert_eq!(lock_of(&stash, DEMOCRACY_LOCK), 1_001 * PDEX);
		// One planck above free plus staked is refused.
		assert_noop!(
			Democracy::vote(signed(1), r, aye(1_001 * PDEX + 1)),
			pallet_democracy::Error::<Runtime>::InsufficientFunds
		);
	});
}

// Brief test 2: the same stash delegates its whole balance; the delegate's vote carries it.
#[test]
fn migrated_stash_delegates_its_whole_bond() {
	new_ext().execute_with(|| {
		let stash = staker(1, 1_001 * PDEX, 1_000 * PDEX);
		to_lock(&stash);
		assert_ok!(Staking::migrate_currency(signed(9), stash.clone()));
		let delegate = set_free(2, 10 * PDEX);
		let to = || MultiAddress::Id(delegate.clone());
		assert_noop!(
			Democracy::delegate(signed(1), to(), Conviction::Locked1x, 1_001 * PDEX + 1),
			pallet_democracy::Error::<Runtime>::InsufficientFunds
		);
		assert_ok!(Democracy::delegate(signed(1), to(), Conviction::Locked1x, 1_001 * PDEX));
		assert_eq!(lock_of(&stash, DEMOCRACY_LOCK), 1_001 * PDEX);

		let r = referendum();
		assert_ok!(Democracy::vote(signed(2), r, aye(10 * PDEX)));
		assert_eq!(tally(r), (1_011 * PDEX, 0, 1_011 * PDEX));
	});
}

// Brief test 3: a staker's council vote records free plus staked PDEX, less the voting bond, as
// the stake did when the bond sat under a lock inside the free balance; the election uses it.
#[test]
fn staker_votes_in_the_council_election_with_its_stake() {
	new_ext().execute_with(|| {
		let stash = staker(1, 1_100 * PDEX, 1_000 * PDEX);
		let member = candidate(3);
		assert_ok!(Elections::vote(signed(1), vec![member.clone()], 1_100 * PDEX));
		let stake = 1_100 * PDEX - voting_bond(1);
		assert_eq!(council_stake(&stash), stake);
		assert_eq!(lock_of(&stash, ElectionsPhragmenPalletId::get()), stake);

		let term = <Runtime as pallet_elections_phragmen::Config>::TermDuration::get();
		frame_system::Pallet::<Runtime>::set_block_number(term);
		<Elections as Hooks<BlockNumberFor<Runtime>>>::on_initialize(term);
		let members: Vec<_> = pallet_elections_phragmen::Members::<Runtime>::get()
			.into_iter()
			.map(|m| (m.who, m.stake))
			.collect();
		assert_eq!(members, vec![(member, stake)]);
	});
}

// Brief test 4: the vote lock and the staking hold work together. Transfers and fees use what the
// vote leaves free; unbond and withdraw work under the lock; unlock clears it after the vote.
#[test]
fn vote_lock_and_staking_hold_work_together() {
	new_ext().execute_with(|| {
		let stash = staker(1, 1_100 * PDEX, 1_000 * PDEX);
		let r = referendum();
		assert_ok!(Democracy::vote(signed(1), r, aye(1_050 * PDEX)));
		// Of the 100 free PDEX, 50 are under the vote lock.
		assert_noop!(transfer(1, 51 * PDEX), TokenError::Frozen);
		assert_ok!(fee(&stash, PDEX));
		assert_ok!(transfer(1, 40 * PDEX));

		// Half the bond is unbonded and withdrawn: it leaves the hold but the vote lock keeps it.
		assert_ok!(Staking::unbond(signed(1), 500 * PDEX));
		pallet_staking::CurrentEra::<Runtime>::put(
			<Runtime as pallet_staking::Config>::BondingDuration::get(),
		);
		assert_ok!(Staking::withdraw_unbonded(signed(1), 0));
		assert_eq!((staked(&stash), Balances::free_balance(&stash)), (500 * PDEX, 559 * PDEX));
		assert_noop!(transfer(1, 10 * PDEX), TokenError::Frozen);

		// The vote ends; its conviction lock runs one locking period past the end.
		let end = end_referendum(r);
		assert_ok!(Democracy::remove_vote(signed(1), r));
		assert_ok!(Democracy::unlock(signed(2), MultiAddress::Id(stash.clone())));
		assert_eq!(lock_of(&stash, DEMOCRACY_LOCK), 1_050 * PDEX);
		let unlock_at = end + <Runtime as pallet_democracy::Config>::VoteLockingPeriod::get();
		frame_system::Pallet::<Runtime>::set_block_number(unlock_at);
		assert_ok!(Democracy::unlock(signed(2), MultiAddress::Id(stash.clone())));
		assert_eq!(lock_of(&stash, DEMOCRACY_LOCK), 0);
		assert_ok!(transfer(1, 500 * PDEX));
	});
}

// Brief test 5: an account that does not stake votes as before, up to its free balance; its
// reserved deposits do not count.
#[test]
fn accounts_without_stake_vote_as_before() {
	new_ext().execute_with(|| {
		set_free(4, 100 * PDEX);
		let r = referendum();
		assert_noop!(
			Democracy::vote(signed(4), r, aye(100 * PDEX + 1)),
			pallet_democracy::Error::<Runtime>::InsufficientFunds
		);
		assert_ok!(Democracy::vote(signed(4), r, aye(100 * PDEX)));

		let member = candidate(3);
		let voter = set_free(5, 100 * PDEX);
		assert_ok!(Elections::vote(signed(5), vec![member.clone()], 200 * PDEX));
		assert_eq!(council_stake(&voter), 100 * PDEX - voting_bond(1));

		let depositor = set_free(6, 100 * PDEX);
		assert_ok!(<Balances as ReservableCurrency<AccountId>>::reserve(&depositor, 30 * PDEX));
		assert_noop!(
			Democracy::vote(signed(6), r, aye(70 * PDEX + 1)),
			pallet_democracy::Error::<Runtime>::InsufficientFunds
		);
		assert_ok!(Elections::vote(signed(6), vec![member], 100 * PDEX));
		assert_eq!(council_stake(&depositor), 70 * PDEX - voting_bond(1));
	});
}

// Brief test 5: votes cast before the upgrade keep their weight. A stash votes on the pre-392 lock,
// the 393 upgrade runs (its 392 one-off migrations skip), then migrate_currency moves the bond to
// the hold: tally, council stake and locks stay, the vote can be removed and fees can be paid.
#[test]
fn votes_cast_before_the_upgrade_keep_their_weight() {
	new_ext().execute_with(|| {
		let stash = staker(1, 1_011 * PDEX, 1_000 * PDEX);
		to_lock(&stash);
		let r = referendum();
		assert_ok!(Democracy::vote(signed(1), r, aye(1_000 * PDEX)));
		let member = candidate(3);
		assert_ok!(Elections::vote(signed(1), vec![member], 1_011 * PDEX));
		let state = || {
			(
				tally(r),
				pallet_democracy::VotingOf::<Runtime>::get(&stash).encode(),
				pallet_elections_phragmen::Voting::<Runtime>::get(&stash).encode(),
				lock_of(&stash, DEMOCRACY_LOCK),
				lock_of(&stash, ElectionsPhragmenPalletId::get()),
			)
		};
		let before = state();
		assert_eq!(before.0, (1_000 * PDEX, 0, 1_000 * PDEX));
		assert_eq!(council_stake(&stash), 1_011 * PDEX - voting_bond(1));

		let at_392 = frame_system::LastRuntimeUpgradeInfo::from(sp_version::RuntimeVersion {
			spec_version: 392,
			..VERSION
		});
		frame_system::LastRuntimeUpgrade::<Runtime>::put(at_392);
		let _ = Executive::execute_on_runtime_upgrade();
		assert_eq!(state(), before);

		assert_ok!(Staking::migrate_currency(signed(9), stash.clone()));
		assert_eq!(state(), before);
		assert_eq!((staked(&stash), lock_of(&stash, STAKING_LOCK)), (1_000 * PDEX, 0));
		// The vote locks now exceed the free balance; the fee comes out of what they leave.
		assert!(Balances::free_balance(&stash) < before.4);
		assert_ok!(fee(&stash, PDEX));
		assert_ok!(Democracy::remove_vote(signed(1), r));
		assert_eq!(tally(r), (0, 0, 0));
	});
}

// 392 leaves the old `staking ` lock on a stash whose ledger changes before migrate_currency (on
// mainnet usually a restaked reward). Votes still see free plus staked; migrate_currency removes
// the lock and keeps the whole bond on the hold.
#[test]
fn migrate_currency_clears_a_lock_left_beside_the_hold() {
	new_ext().execute_with(|| {
		let stash = staker(1, 1_100 * PDEX, 1_000 * PDEX);
		to_lock(&stash);
		assert_ok!(Staking::bond_extra(signed(1), 10 * PDEX));
		assert_eq!(staked(&stash), 1_010 * PDEX);
		assert_eq!(lock_of(&stash, STAKING_LOCK), 1_000 * PDEX);
		assert_eq!(Balances::free_balance(&stash), 90 * PDEX);
		assert_ok!(fee(&stash, PDEX));

		let r = referendum();
		assert_ok!(Democracy::vote(signed(1), r, aye(1_099 * PDEX)));

		assert_ok!(Staking::migrate_currency(signed(9), stash.clone()));
		assert_eq!(lock_of(&stash, STAKING_LOCK), 0);
		assert_eq!(staked(&stash), 1_010 * PDEX);
		assert_eq!(pallet_staking::Ledger::<Runtime>::get(&stash).unwrap().total, 1_010 * PDEX);
		frame_system::Pallet::<Runtime>::assert_has_event(RuntimeEvent::Staking(
			pallet_staking::Event::CurrencyMigrated { stash, force_withdraw: 0 },
		));
	});
}
