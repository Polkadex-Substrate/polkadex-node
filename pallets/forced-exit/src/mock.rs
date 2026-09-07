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

//! Mock runtime for pallet-forced-exit tests.

use crate as pallet_forced_exit;
use crate::traits::Custody;

use frame_support::{
	derive_impl,
	pallet_prelude::DispatchResult,
	parameter_types,
	traits::{ConstU128, ConstU32},
};
use frame_system::EnsureRoot;
use polkadex_primitives::AssetId;
use sp_runtime::BuildStorage;
use sp_std::collections::btree_map::BTreeMap;

pub const ALICE: u64 = 1;
pub const BOB: u64 = 2;
pub const CHARLIE: u64 = 3;

pub const USDT: AssetId = AssetId::Asset(1);
pub const BTC: AssetId = AssetId::Asset(2);

/// One unit in the engine's fixed-point balance representation.
pub const UNIT: u128 = 1_000_000_000_000;

type Block = frame_system::mocking::MockBlock<Test>;

frame_support::construct_runtime!(
	pub enum Test {
		System: frame_system,
		Balances: pallet_balances,
		ForcedExit: pallet_forced_exit,
	}
);

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Block = Block;
	type AccountData = pallet_balances::AccountData<u128>;
}

#[derive_impl(pallet_balances::config_preludes::TestDefaultConfig)]
impl pallet_balances::Config for Test {
	type Balance = u128;
	type AccountStore = System;
	type ExistentialDeposit = ConstU128<1>;
	type ReserveIdentifier = [u8; 8];
}

parameter_types! {
	pub const RequestServiceTimeout: u64 = 100;
	pub const SnapshotLivenessTimeout: u64 = 200;
	pub const RequestDeposit: u128 = 10;
	pub const MinimumDisputeWindow: u64 = 50;
}

impl pallet_forced_exit::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type NativeCurrency = Balances;
	type Custody = MockCustody;
	type GovernanceOrigin = EnsureRoot<u64>;
	type RequestServiceTimeout = RequestServiceTimeout;
	type SnapshotLivenessTimeout = SnapshotLivenessTimeout;
	type MaxPendingRequests = ConstU32<4>;
	type RequestDeposit = RequestDeposit;
	type MinimumDisputeWindow = MinimumDisputeWindow;
	type WeightInfo = ();
}

thread_local! {
	/// Custody pool holdings per asset.
	static CUSTODY: core::cell::RefCell<BTreeMap<AssetId, u128>> =
		core::cell::RefCell::new(BTreeMap::new());
	/// Amounts actually released to users, for assertions.
	static RELEASED: core::cell::RefCell<BTreeMap<(u64, AssetId), u128>> =
		core::cell::RefCell::new(BTreeMap::new());
}

/// Stand-in for the settlement pallet's custody account.
pub struct MockCustody;

impl Custody<u64> for MockCustody {
	fn release(who: &u64, asset: AssetId, amount: u128) -> DispatchResult {
		CUSTODY.with(|custody| {
			let mut custody = custody.borrow_mut();
			let holding = custody.entry(asset).or_default();
			*holding = holding.saturating_sub(amount);
		});
		RELEASED.with(|released| {
			let mut released = released.borrow_mut();
			let paid = released.entry((*who, asset)).or_default();
			*paid = paid.saturating_add(amount);
		});
		Ok(())
	}

	fn custody_balance(asset: AssetId) -> u128 {
		CUSTODY.with(|custody| custody.borrow().get(&asset).copied().unwrap_or_default())
	}
}

/// Seeds the custody pool for an asset.
pub fn fund_custody(asset: AssetId, amount: u128) {
	CUSTODY.with(|custody| {
		let mut custody = custody.borrow_mut();
		let holding = custody.entry(asset).or_default();
		*holding = holding.saturating_add(amount);
	});
}

/// Total released to an account for an asset.
pub fn released(who: u64, asset: AssetId) -> u128 {
	RELEASED.with(|released| released.borrow().get(&(who, asset)).copied().unwrap_or_default())
}

/// Builds a test externality with funded accounts and an empty custody pool.
pub fn new_test_ext() -> sp_io::TestExternalities {
	CUSTODY.with(|custody| custody.borrow_mut().clear());
	RELEASED.with(|released| released.borrow_mut().clear());

	let mut storage = frame_system::GenesisConfig::<Test>::default().build_storage().unwrap();
	pallet_balances::GenesisConfig::<Test> {
		balances: vec![(ALICE, 1_000), (BOB, 1_000), (CHARLIE, 1_000)],
		..Default::default()
	}
	.assimilate_storage(&mut storage)
	.unwrap();

	let mut ext: sp_io::TestExternalities = storage.into();
	ext.execute_with(|| System::set_block_number(1));
	ext
}

/// Advances the block number by `n`.
pub fn run_to_block(n: u64) {
	System::set_block_number(n);
}
