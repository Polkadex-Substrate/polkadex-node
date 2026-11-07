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

//! Benchmarks of the HFT pallet's calls, which give `weights::pallet_hyper_fungible_token`: `send`
//! on each path (PDEX into escrow; an asset into custody, which costs more than a burn) with `d`
//! bytes of `call_data`, paying a bridge fee to the collector and a relayer fee; `register_token`
//! over `c` new chains; `update_token` pointing `c` registered chains at new contracts. The
//! scenarios are shared with a native test.

use crate::{
	constants::currency::PDEX, AccountId, Assets, Balance, Balances, HyperFungibleToken,
	IsmpParams, PolkadexAssetId, Runtime, RuntimeOrigin,
};
use frame_support::{
	assert_ok,
	traits::{fungibles::Mutate, Currency, Get},
};
use ismp::host::StateMachine;
use pallet_hyper_fungible_token::types::{ChainConfig, SendParams, TokenRegistration, TokenUpdate};
use sp_core::H160;
use sp_std::{collections::btree_map::BTreeMap, vec};

/// The longest `call_data` benchmarked, which MaxCallData is set to; a higher cap extrapolates.
pub const MAX_CALL_DATA: u32 = 64 * 1024;
/// The most chains a registration or update is benchmarked with.
pub const MAX_CHAINS: u32 = 32;
/// An asset with 6 decimals, registered for custody: a send moves it into the pallet account.
pub const TOKEN: u128 = 1_984;
/// The first number of the contracts an update points chains at.
pub const REPLACED: u32 = 1_000;
/// The collector's PDEX before a send, so a native fee shows as the increase.
const COLLECTOR_FUNDS: Balance = 10 * PDEX;
/// The relayer fee of a send, in PDEX: the existential deposit, so it opens the fee account.
const RELAYER_FEE: Balance = PDEX;
const DEST: StateMachine = StateMachine::Evm(1);

/// The amount sent and the bridge fee, in the units of `asset`.
fn amount_and_fee(asset: u128) -> (Balance, Balance) {
	if asset == PolkadexAssetId::get() {
		(100 * PDEX, PDEX)
	} else {
		(100_000_000, 1_000_000)
	}
}

/// The EVM chains `1..=c` (the first is the send's destination), contracts numbered from `base`.
fn chains(c: u32, base: u32) -> BTreeMap<StateMachine, ChainConfig> {
	let chain = |i: u32| {
		let mut contract = [0u8; 20];
		contract[..4].copy_from_slice(&(base + i).to_le_bytes());
		(StateMachine::Evm(i), ChainConfig { token_contract: H160(contract), decimals: 18 })
	};
	(1..=c).map(chain).collect()
}

/// The registration of `asset` on `c` new chains.
pub fn registration(asset: u128, native: bool, c: u32) -> TokenRegistration<u128> {
	TokenRegistration { local_id: asset, native, chains: chains(c, 0) }
}

/// Registers `asset` on `c` new chains.
pub fn register_chains(asset: u128, native: bool, c: u32) {
	let registration = registration(asset, native, c);
	assert_ok!(HyperFungibleToken::register_token(RuntimeOrigin::root(), registration));
}

/// An update pointing `asset`'s `c` registered chains at new contracts, the costliest change per
/// chain: each drops its old reverse entry and writes four new ones.
pub fn replace_chains(asset: u128, c: u32) -> TokenUpdate<u128> {
	TokenUpdate { asset_id: asset, add_chains: chains(c, REPLACED), remove_chains: vec![] }
}

/// Checks that `asset`'s first `c` chains point at the contracts numbered from `base`.
pub fn assert_contracts(asset: u128, c: u32, base: u32) {
	for (chain, config) in chains(c, base) {
		let contract = pallet_hyper_fungible_token::TokenContracts::<Runtime>::get(chain, asset);
		assert_eq!(contract, Some(config.token_contract.0.to_vec()));
	}
}

/// Creates `TOKEN` (sufficient, 6 decimals) and mints `balance` of it to `holder`.
fn token(holder: &AccountId, balance: Balance) {
	let owner = sp_runtime::MultiAddress::Id(holder.clone());
	assert_ok!(Assets::force_create(RuntimeOrigin::root(), TOKEN.into(), owner, true, 1));
	let (name, symbol) = (b"Token".to_vec(), b"TKN".to_vec());
	let root = RuntimeOrigin::root();
	assert_ok!(Assets::force_set_metadata(root, TOKEN.into(), name, symbol, 6, false));
	assert_ok!(<Assets as Mutate<AccountId>>::mint_into(TOKEN, holder, balance));
}

/// A send of `asset` by `sender` with `d` bytes of `call_data`, set up so the fee hook moves a fee
/// to an existing collector other than the sender; it also pays a relayer fee and sets a timeout.
/// Returns the collector and the call's parameters.
pub fn priced_send(
	sender: &AccountId,
	asset: u128,
	d: u32,
) -> (AccountId, SendParams<u128, Balance>) {
	let pdex = asset == PolkadexAssetId::get();
	let (amount, fee) = amount_and_fee(asset);
	let collector = AccountId::from([0xc0; 32]);
	let _ = <Balances as Currency<AccountId>>::make_free_balance_be(&collector, COLLECTOR_FUNDS);
	// Exactly what the send moves: its last debit in each currency reaps the sender's account.
	let pdex_funds = if pdex { amount + fee + RELAYER_FEE } else { RELAYER_FEE };
	let _ = <Balances as Currency<AccountId>>::make_free_balance_be(sender, pdex_funds);
	if !pdex {
		token(sender, amount + fee);
	}
	// Custody for an asset too: the move into the pallet account opens its asset and System
	// accounts, which costs more than a burn, so send_asset covers both branches.
	register_chains(asset, true, 1);
	// A real time, so the timeout reads one (UnixTime logs an error for 0).
	pallet_timestamp::Now::<Runtime>::put(1_800_000_000_000u64);
	assert_ok!(IsmpParams::set_fee_collector(RuntimeOrigin::root(), collector.clone()));
	assert_ok!(IsmpParams::set_bridge_fee(RuntimeOrigin::root(), asset, DEST, Some(fee)));
	assert_ok!(IsmpParams::set_max_call_data(RuntimeOrigin::root(), MAX_CALL_DATA));
	let params = SendParams {
		asset_id: asset,
		destination: DEST,
		// An EVM chain takes a 20-byte recipient, as send requires.
		recipient: vec![0x77; 20].try_into().expect("20 bytes fit"),
		amount,
		timeout: 3_600,
		relayer_fee: RELAYER_FEE,
		call_data: Some(vec![0x55; d as usize]),
	};
	(collector, params)
}

/// What a priced send leaves: the fee with the collector and the transfer's refund terms.
pub fn assert_sent(collector: &AccountId, asset: u128) {
	let paid = if asset == PolkadexAssetId::get() {
		Balances::free_balance(collector) - COLLECTOR_FUNDS
	} else {
		Assets::balance(TOKEN, collector)
	};
	assert_eq!(paid, amount_and_fee(asset).1);
	assert_eq!(pallet_hyper_fungible_token::RefundTerms::<Runtime>::iter().count(), 1);
}

/// The benchmarks' own pallet type; they drive the HFT pallet directly.
#[cfg(feature = "runtime-benchmarks")]
pub struct Pallet<T: Config>(core::marker::PhantomData<T>);
#[cfg(feature = "runtime-benchmarks")]
pub trait Config: frame_system::Config {}
#[cfg(feature = "runtime-benchmarks")]
impl Config for Runtime {}

#[cfg(feature = "runtime-benchmarks")]
#[frame_benchmarking::v2::benchmarks]
mod benchmarks {
	use super::*;
	use frame_benchmarking::v2::*;

	#[benchmark]
	fn send_native(d: Linear<0, { MAX_CALL_DATA }>) {
		let sender: AccountId = whitelisted_caller();
		let (collector, params) = priced_send(&sender, PolkadexAssetId::get(), d);
		#[block]
		{
			assert_ok!(HyperFungibleToken::send(RuntimeOrigin::signed(sender), params));
		}
		assert_sent(&collector, PolkadexAssetId::get());
	}

	#[benchmark]
	fn send_asset(d: Linear<0, { MAX_CALL_DATA }>) {
		let sender: AccountId = whitelisted_caller();
		let (collector, params) = priced_send(&sender, TOKEN, d);
		#[block]
		{
			assert_ok!(HyperFungibleToken::send(RuntimeOrigin::signed(sender), params));
		}
		assert_sent(&collector, TOKEN);
	}

	#[benchmark]
	fn register_token(c: Linear<0, { MAX_CHAINS }>) {
		let registration = registration(TOKEN, false, c);
		#[block]
		{
			assert_ok!(HyperFungibleToken::register_token(RuntimeOrigin::root(), registration));
		}
		assert_contracts(TOKEN, c, 0);
	}

	#[benchmark]
	fn update_token(c: Linear<0, { MAX_CHAINS }>) {
		register_chains(TOKEN, false, c);
		let update = replace_chains(TOKEN, c);
		#[block]
		{
			assert_ok!(HyperFungibleToken::update_token(RuntimeOrigin::root(), update));
		}
		assert_contracts(TOKEN, c, REPLACED);
	}
}
