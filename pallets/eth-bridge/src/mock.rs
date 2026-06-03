// This file is part of Polkadex.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

use crate::{pallet as eth_bridge, BridgeAssets};
use frame_support::{
    pallet_prelude::DispatchResult,
    parameter_types,
    traits::{AsEnsureOriginWithArg, ConstU32},
};
use frame_system::{EnsureRoot, EnsureSigned};
use sp_core::H256;
use sp_runtime::{
    traits::{BlakeTwo256, IdentityLookup},
    BuildStorage,
};
use sp_std::cell::RefCell;
use sp_std::vec::Vec;

pub type Balance = u128;
pub type AssetId = u128;
pub type AccountId = u64;
type Block = frame_system::mocking::MockBlock<Test>;

// ── Runtime ────────────────────────────────────────────────────────────────

frame_support::construct_runtime!(
    pub enum Test {
        System:    frame_system,
        Balances:  pallet_balances,
        Assets:    pallet_assets,
        EthBridge: eth_bridge,
    }
);

// ── System ─────────────────────────────────────────────────────────────────

parameter_types! {
    pub const BlockHashCount: u64 = 250;
    pub const SS58Prefix: u8 = 42;
}

impl frame_system::Config for Test {
    type BaseCallFilter = frame_support::traits::Everything;
    type BlockWeights = ();
    type BlockLength = ();
    type RuntimeOrigin = RuntimeOrigin;
    type RuntimeCall = RuntimeCall;
    type Hash = H256;
    type Hashing = BlakeTwo256;
    type AccountId = AccountId;
    type Lookup = IdentityLookup<Self::AccountId>;
    type RuntimeEvent = RuntimeEvent;
    type BlockHashCount = BlockHashCount;
    type DbWeight = ();
    type Version = ();
    type PalletInfo = PalletInfo;
    type AccountData = pallet_balances::AccountData<Balance>;
    type OnNewAccount = ();
    type OnKilledAccount = ();
    type SystemWeightInfo = ();
    type SS58Prefix = SS58Prefix;
    type OnSetCode = ();
    type MaxConsumers = ConstU32<16>;
    type Nonce = u32;
    type Block = Block;
    type RuntimeTask = ();
    type ExtensionsWeightInfo = ();
    type SingleBlockMigrations = ();
    type MultiBlockMigrator = ();
    type PreInherents = ();
    type PostInherents = ();
    type PostTransactions = ();
}

// ── Balances ───────────────────────────────────────────────────────────────

parameter_types! {
    pub const ExistentialDeposit: u32 = 1;
    pub const MaxLocks: u32 = 50;
    pub const MaxReserves: u32 = 50;
}

impl pallet_balances::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = ();
    type Balance = Balance;
    type DustRemoval = ();
    type ExistentialDeposit = ExistentialDeposit;
    type AccountStore = frame_system::Pallet<Test>;
    type ReserveIdentifier = [u8; 8];
    type RuntimeHoldReason = ();
    type RuntimeFreezeReason = ();
    type FreezeIdentifier = ();
    type MaxLocks = MaxLocks;
    type MaxReserves = MaxReserves;
    type MaxFreezes = ();
    type DoneSlashHandler = ();
}

// ── Assets ─────────────────────────────────────────────────────────────────

parameter_types! {
    pub const AssetDeposit: Balance = 100;
    pub const ApprovalDeposit: Balance = 1;
    pub const StringLimit: u32 = 50;
    pub const MetadataDepositBase: Balance = 10;
    pub const MetadataDepositPerByte: Balance = 1;
}

impl pallet_assets::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type Balance = u128;
    type RemoveItemsLimit = ();
    type AssetId = AssetId;
    type AssetIdParameter = parity_scale_codec::Compact<u128>;
    type Currency = Balances;
    type CreateOrigin = AsEnsureOriginWithArg<EnsureSigned<AccountId>>;
    type ForceOrigin = EnsureRoot<AccountId>;
    type AssetDeposit = AssetDeposit;
    type AssetAccountDeposit = AssetDeposit;
    type MetadataDepositBase = MetadataDepositBase;
    type MetadataDepositPerByte = MetadataDepositPerByte;
    type ApprovalDeposit = ApprovalDeposit;
    type StringLimit = StringLimit;
    type Freezer = ();
    type Extra = ();
    type CallbackHandle = ();
    type WeightInfo = ();
    type Holder = ();
    type ReserveData = ();
}

// ── Mock BridgeAssets ──────────────────────────────────────────────────────

thread_local! {
    /// Records (polkadex_asset_id, recipient, amount) for each mint call.
    pub static MINTED: RefCell<Vec<(u128, AccountId, u128)>> = RefCell::new(Vec::new());
    /// Records (polkadex_asset_id, from, amount) for each burn call.
    pub static BURNED: RefCell<Vec<(u128, AccountId, u128)>> = RefCell::new(Vec::new());
    /// Simulated balances keyed by (polkadex_asset_id, account).
    pub static BALANCES: RefCell<sp_std::collections::btree_map::BTreeMap<(u128, AccountId), u128>>
        = RefCell::new(sp_std::collections::btree_map::BTreeMap::new());
}

pub struct MockBridgeAssets;

impl BridgeAssets<AccountId> for MockBridgeAssets {
    fn mint(polkadex_asset_id: u128, recipient: &AccountId, amount: u128) -> DispatchResult {
        MINTED.with(|m| m.borrow_mut().push((polkadex_asset_id, *recipient, amount)));
        BALANCES.with(|b| {
            *b.borrow_mut().entry((polkadex_asset_id, *recipient)).or_insert(0) += amount;
        });
        Ok(())
    }

    fn burn(polkadex_asset_id: u128, from: &AccountId, amount: u128) -> DispatchResult {
        let ok = BALANCES.with(|b| {
            let mut map = b.borrow_mut();
            let bal = map.entry((polkadex_asset_id, *from)).or_insert(0);
            if *bal >= amount {
                *bal -= amount;
                true
            } else {
                false
            }
        });
        if !ok {
            return Err(sp_runtime::DispatchError::Other("InsufficientBalance"));
        }
        BURNED.with(|b| b.borrow_mut().push((polkadex_asset_id, *from, amount)));
        Ok(())
    }
}

pub fn minted_tokens() -> Vec<(u128, AccountId, u128)> {
    MINTED.with(|m| m.borrow().clone())
}

pub fn burned_tokens() -> Vec<(u128, AccountId, u128)> {
    BURNED.with(|b| b.borrow().clone())
}

/// Seed a mock pallet-assets balance so a user can initiate a withdrawal in tests.
pub fn set_balance(polkadex_asset_id: u128, account: AccountId, amount: u128) {
    BALANCES.with(|b| {
        b.borrow_mut().insert((polkadex_asset_id, account), amount);
    });
}

// ── EthBridge pallet ───────────────────────────────────────────────────────

impl eth_bridge::Config for Test {
    type BridgeAssets = MockBridgeAssets;
    type WeightInfo = eth_bridge::TestWeightInfo;
}

// ── Test externalities ─────────────────────────────────────────────────────

pub fn new_test_ext() -> sp_io::TestExternalities {
    MINTED.with(|m| m.borrow_mut().clear());
    BURNED.with(|b| b.borrow_mut().clear());
    BALANCES.with(|b| b.borrow_mut().clear());
    let t = frame_system::GenesisConfig::<Test>::default().build_storage().unwrap();
    sp_io::TestExternalities::new(t)
}
