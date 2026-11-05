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

//! Paid bridge transfers: `HyperFungibleToken::send` pays the fee that `IsmpParams` holds for its
//! (asset, destination) to the fee collector, on top of the amount, before the lock or burn.

use super::*;
use frame_support::{
	assert_noop, assert_ok,
	traits::{fungibles, Currency},
};
use pallet_hyper_fungible_token::{
	types::{ChainConfig, SendParams, TokenRegistration},
	Error as HftError, Event as HftEvent,
};
use pallet_ismp::offchain::{leaf_default_key, Leaf};
use sp_core::H160;
use sp_runtime::{traits::Dispatchable, DispatchError, DispatchResult, MultiAddress, TokenError};

const PDEX: Balance = crate::constants::currency::PDEX;
const USD: u128 = 1_000_000; // one USDT, 6 decimals
const PDEX_ID: u128 = POLKADEX_NATIVE_ASSET_ID;
const USDT: u128 = 1_984;
const ETH: StateMachine = StateMachine::Evm(1);
const BSC: StateMachine = StateMachine::Evm(56);

fn account(n: u8) -> AccountId {
	AccountId::from([n; 32])
}

fn sender() -> AccountId {
	account(71)
}

fn collector() -> AccountId {
	account(72)
}

fn fund(who: &AccountId, pdex: Balance) {
	let _ = <Balances as Currency<AccountId>>::make_free_balance_be(who, pdex);
}

fn mint(who: &AccountId, usdt: u128) {
	assert_ok!(<Assets as fungibles::Mutate<AccountId>>::mint_into(USDT, who, usdt));
}

/// Registers `asset` to Ethereum and BSC: escrowed on send when `native`, else burnt.
fn register(asset: u128, native: bool, eth_decimals: u8, bsc_decimals: u8) {
	let token_contract = H160::repeat_byte(asset as u8 + 1);
	let chain = |decimals: u8| ChainConfig { token_contract, decimals };
	let chains = [(ETH, chain(eth_decimals)), (BSC, chain(bsc_decimals))].into();
	let registration = TokenRegistration { local_id: asset, native, chains };
	assert_ok!(HyperFungibleToken::register_token(RuntimeOrigin::root(), registration));
}

/// PDEX (escrowed) and USDT (burnt, 6 decimals) registered to Ethereum and BSC. The sender holds
/// 1,000 of each, the collector the existential deposit and the USDT minimum.
fn new_test_ext() -> sp_io::TestExternalities {
	let mut ext: sp_io::TestExternalities = frame_system::GenesisConfig::<Runtime>::default()
		.build_storage()
		.unwrap()
		.into();
	ext.execute_with(|| {
		System::set_block_number(1); // events are kept from block 1
		fund(&sender(), 1_000 * PDEX);
		fund(&collector(), PDEX);
		let owner = MultiAddress::Id(collector());
		assert_ok!(Assets::force_create(RuntimeOrigin::root(), USDT.into(), owner, true, 1));
		let (name, symbol) = (b"Tether USD".to_vec(), b"USDT".to_vec());
		assert_ok!(Assets::force_set_metadata(
			RuntimeOrigin::root(),
			USDT.into(),
			name,
			symbol,
			6,
			false
		));
		mint(&sender(), 1_000 * USD);
		mint(&collector(), 1);
		register(PDEX_ID, true, 18, 18);
		register(USDT, false, 6, 18);
	});
	ext
}

/// Fees as governance sets them: per asset and destination, in the asset sent.
fn set_fees() {
	for (asset, dest, fee) in [
		(PDEX_ID, ETH, 30 * PDEX),
		(PDEX_ID, BSC, 10 * PDEX),
		(USDT, ETH, 3 * USD),
		(USDT, BSC, USD),
	] {
		assert_ok!(IsmpParams::set_bridge_fee(RuntimeOrigin::root(), asset, dest, Some(fee)));
	}
	assert_ok!(IsmpParams::set_fee_collector(RuntimeOrigin::root(), collector()));
}

fn params(asset: u128, dest: StateMachine, amount: Balance) -> SendParams<u128, Balance> {
	SendParams {
		asset_id: asset,
		destination: dest,
		recipient: vec![7u8; 20].try_into().unwrap(),
		amount,
		timeout: 0,
		relayer_fee: 0,
		call_data: None,
	}
}

/// `send` through the runtime dispatch, which rolls a failed send back as a block would.
fn send(who: &AccountId, params: SendParams<u128, Balance>) -> DispatchResult {
	RuntimeCall::HyperFungibleToken(pallet_hyper_fungible_token::Call::send { params })
		.dispatch(RuntimeOrigin::signed(who.clone()))
		.map(|_| ())
		.map_err(|e| e.error)
}

fn hft_events() -> Vec<HftEvent<Runtime>> {
	System::events()
		.into_iter()
		.filter_map(|record| match record.event {
			RuntimeEvent::HyperFungibleToken(event) => Some(event),
			_ => None,
		})
		.collect()
}

/// The last send paid `fee` to the collector, then sent `amount` alone (TokenSent unchanged).
fn assert_paid(from: &AccountId, asset: u128, dest: StateMachine, fee: Balance, amount: Balance) {
	let events = hft_events();
	let [.., paid, sent] = &events[..] else { panic!("a send emits two events") };
	let collector = collector();
	let from = from.clone();
	assert_eq!(*paid, HftEvent::BridgeFeePaid { from, collector, asset_id: asset, dest, fee });
	assert!(
		matches!(sent, HftEvent::TokenSent { amount: a, dest: d, .. } if *a == amount && *d == dest)
	);
}

/// The send after `before` events moved no fee: one TokenSent with `amount`, no BridgeFeePaid.
fn assert_no_fee_event(before: usize, amount: Balance) {
	let events = hft_events();
	assert!(matches!(&events[before..], [HftEvent::TokenSent { amount: a, .. }] if *a == amount));
}

// A send pays the fee of its (asset, destination) to the collector, on top of the amount and in
// the asset sent, before the lock (PDEX) or the burn (USDT). The collector's own send pays nothing
// and records no fee.
#[test]
fn send_pays_the_fee_on_top_in_the_asset_sent() {
	new_test_ext().execute_with(|| {
		set_fees();
		let escrow = HyperFungibleToken::pallet_account();
		assert_ok!(send(&sender(), params(PDEX_ID, ETH, 100 * PDEX)));
		assert_paid(&sender(), PDEX_ID, ETH, 30 * PDEX, 100 * PDEX);
		assert_eq!(Balances::free_balance(sender()), 870 * PDEX);
		assert_eq!(Balances::free_balance(collector()), 31 * PDEX);
		assert_eq!(Balances::free_balance(&escrow), 100 * PDEX);

		let supply = Assets::total_supply(USDT);
		assert_ok!(send(&sender(), params(USDT, BSC, 50 * USD)));
		assert_paid(&sender(), USDT, BSC, USD, 50 * USD);
		assert_eq!(Assets::balance(USDT, sender()), 949 * USD);
		assert_eq!(Assets::balance(USDT, collector()), 1 + USD);
		assert_eq!(Assets::total_supply(USDT), supply - 50 * USD);

		let before = hft_events().len();
		assert_ok!(send(&collector(), params(PDEX_ID, BSC, 20 * PDEX)));
		assert_no_fee_event(before, 20 * PDEX);
		assert_eq!(Balances::free_balance(collector()), 11 * PDEX);
		assert_eq!(Balances::free_balance(&escrow), 120 * PDEX);
	});
}

// Fail closed: with no fee for the pair, or no collector, the send is refused and nothing moves.
// A cleared fee refuses again; a zero fee is a set fee and records no fee.
#[test]
fn send_without_fee_or_collector_is_refused() {
	new_test_ext().execute_with(|| {
		let refused = |asset: u128, dest: StateMachine| {
			let not_set = HftError::<Runtime>::BridgeFeeNotSet;
			assert_noop!(send(&sender(), params(asset, dest, USD)), not_set);
		};
		refused(PDEX_ID, ETH);
		assert_ok!(IsmpParams::set_bridge_fee(RuntimeOrigin::root(), PDEX_ID, ETH, Some(PDEX)));
		refused(PDEX_ID, ETH); // no collector yet
		assert_ok!(IsmpParams::set_fee_collector(RuntimeOrigin::root(), collector()));
		refused(PDEX_ID, BSC); // no fee for these pairs
		refused(USDT, ETH);
		assert_ok!(IsmpParams::set_bridge_fee(RuntimeOrigin::root(), PDEX_ID, ETH, None));
		refused(PDEX_ID, ETH);
		assert_ok!(IsmpParams::set_bridge_fee(RuntimeOrigin::root(), PDEX_ID, ETH, Some(0)));
		let before = hft_events().len();
		assert_ok!(send(&sender(), params(PDEX_ID, ETH, 10 * PDEX)));
		assert_no_fee_event(before, 10 * PDEX);
	});
}

// call_data is capped by MaxCallData: none at the default of 0, up to the cap once raised.
#[test]
fn call_data_longer_than_the_cap_is_refused() {
	new_test_ext().execute_with(|| {
		set_fees();
		let with_data = |len: usize| SendParams {
			call_data: Some(vec![1u8; len]),
			..params(PDEX_ID, ETH, 10 * PDEX)
		};
		assert_noop!(send(&sender(), with_data(1)), HftError::<Runtime>::CallDataTooLong);
		assert_ok!(send(&sender(), with_data(0)));
		assert_ok!(IsmpParams::set_max_call_data(RuntimeOrigin::root(), 64));
		assert_noop!(send(&sender(), with_data(65)), HftError::<Runtime>::CallDataTooLong);
		assert_ok!(send(&sender(), with_data(64)));
	});
}

// Fees and MaxCallData: Root or more than half of the council. The collector: Root only, and an
// existing account. Each setter emits its event.
#[test]
fn bridge_fee_setters_check_their_origins() {
	use pallet_ismp_params::{Error as ParamsError, Event as ParamsEvent};
	let council = |ayes: u32| -> RuntimeOrigin {
		pallet_collective::RawOrigin::<AccountId, CouncilCollective>::Members(ayes, 5).into()
	};
	new_test_ext().execute_with(|| {
		for origin in [RuntimeOrigin::signed(sender()), council(2)] {
			assert_noop!(
				IsmpParams::set_bridge_fee(origin.clone(), USDT, ETH, Some(USD)),
				DispatchError::BadOrigin
			);
			assert_noop!(IsmpParams::set_max_call_data(origin, 64), DispatchError::BadOrigin);
		}
		for origin in [RuntimeOrigin::root(), council(3)] {
			assert_ok!(IsmpParams::set_bridge_fee(origin.clone(), USDT, ETH, Some(USD)));
			let set =
				ParamsEvent::<Runtime>::BridgeFeeSet { asset_id: USDT, dest: ETH, fee: Some(USD) };
			System::assert_last_event(set.into());
			assert_ok!(IsmpParams::set_max_call_data(origin, 64));
			System::assert_last_event(ParamsEvent::<Runtime>::MaxCallDataSet { max: 64 }.into());
		}
		for origin in [RuntimeOrigin::signed(sender()), council(5)] {
			assert_noop!(
				IsmpParams::set_fee_collector(origin, collector()),
				DispatchError::BadOrigin
			);
		}
		assert_noop!(
			IsmpParams::set_fee_collector(RuntimeOrigin::root(), account(99)),
			ParamsError::<Runtime>::CollectorNotFound
		);
		assert_ok!(IsmpParams::set_fee_collector(RuntimeOrigin::root(), collector()));
		let set = ParamsEvent::<Runtime>::FeeCollectorSet { collector: collector() };
		System::assert_last_event(set.into());
	});
}

// The sender must hold the amount plus the fee: one unit short and nothing moves, the fee
// included, since the dispatch is transactional. Exactly the amount plus the fee passes.
#[test]
fn send_without_amount_plus_fee_is_refused() {
	new_test_ext().execute_with(|| {
		set_fees();
		let short = account(73);
		fund(&short, 130 * PDEX - 1);
		mint(&short, 4 * USD - 1);
		let pdex = params(PDEX_ID, ETH, 100 * PDEX);
		assert_noop!(send(&short, pdex), TokenError::FundsUnavailable);
		assert_noop!(send(&short, params(USDT, ETH, USD)), TokenError::FundsUnavailable);
		assert_ok!(send(&short, params(PDEX_ID, ETH, 100 * PDEX - 1)));
		assert_eq!(Balances::free_balance(&short), 0);
	});
}

// A timed-out send refunds the amount alone; the fee stays with the collector.
#[test]
fn timeout_refund_keeps_the_fee() {
	let mut ext = new_test_ext();
	let commitments = ext.execute_with(|| {
		set_fees();
		assert_ok!(send(&sender(), params(PDEX_ID, ETH, 100 * PDEX)));
		assert_ok!(send(&sender(), params(USDT, ETH, 50 * USD)));
		let sent = hft_events().into_iter().filter_map(|event| match event {
			HftEvent::TokenSent { commitment, .. } => Some(commitment),
			_ => None,
		});
		sent.collect::<Vec<_>>()
	});
	assert_eq!(commitments.len(), 2);
	// pallet-ismp indexes each request offchain under its commitment; read them back.
	ext.persist_offchain_overlay();
	let offchain = ext.offchain_db();
	let requests: Vec<Request> = commitments
		.iter()
		.map(|commitment| {
			let leaf = offchain.get(&leaf_default_key(*commitment)).expect("an indexed request");
			match Leaf::decode(&mut &leaf[..]).expect("a leaf") {
				Leaf::Request(request) => request,
				Leaf::GetResponse(_) => panic!("a request leaf"),
			}
		})
		.collect();
	ext.execute_with(|| {
		for request in requests {
			assert_ok!(HyperFungibleToken::default().on_timeout(request));
		}
		assert_eq!(Balances::free_balance(sender()), 970 * PDEX);
		assert_eq!(Balances::free_balance(collector()), 31 * PDEX);
		assert_eq!(Assets::balance(USDT, sender()), 997 * USD);
		assert_eq!(Assets::balance(USDT, collector()), 1 + 3 * USD);
	});
}
