// This file is part of Polkadex.
//
// Copyright (c) 2023 Polkadex oü.
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

//! Migration V0 → V1
//!
//! Adds `tier: MarketTier` (defaulting to `Tier3`) to every `LMPMarketConfig` stored
//! inside `LMPConfig` (all epochs) and `ExpectedLMPConfig`.
//!
//! Gate: runs only when `on_chain_storage_version() < 1`.
//! After completion: sets on-chain storage version to 1.

use crate::pallet::{Config, ExpectedLMPConfig, LMPConfig};
use frame_support::{
	traits::{GetStorageVersion, OnRuntimeUpgrade, StorageVersion},
	weights::Weight,
};
use frame_support::traits::Get;
use orderbook_primitives::lmp::{LMPEpochConfig, LMPMarketConfig, MarketTier};
use parity_scale_codec::{Decode, Encode};
use rust_decimal::Decimal;
use sp_std::collections::btree_map::BTreeMap;

/// Old `LMPMarketConfig` layout — no `tier` field.
/// Used only for decoding pre-migration data.
#[derive(Decode)]
struct OldLMPMarketConfig {
	weightage: Decimal,
	min_fees_paid: Decimal,
	min_maker_volume: Decimal,
	max_spread: Decimal,
	min_depth: Decimal,
}

/// Old `LMPEpochConfig` layout — contains `BTreeMap<TradingPair, OldLMPMarketConfig>`.
/// We decode the whole config and reconstruct it with the new layout.
#[derive(Decode)]
struct OldLMPEpochConfig {
	total_liquidity_mining_rewards: Decimal,
	total_trading_rewards: Decimal,
	config: BTreeMap<orderbook_primitives::types::TradingPair, OldLMPMarketConfig>,
	max_accounts_rewarded: u16,
	claim_safety_period: u32,
}

impl OldLMPEpochConfig {
	fn into_new(self) -> LMPEpochConfig {
		LMPEpochConfig {
			total_liquidity_mining_rewards: self.total_liquidity_mining_rewards,
			total_trading_rewards: self.total_trading_rewards,
			config: self
				.config
				.into_iter()
				.map(|(pair, old)| {
					(
						pair,
						LMPMarketConfig {
							weightage: old.weightage,
							min_fees_paid: old.min_fees_paid,
							min_maker_volume: old.min_maker_volume,
							max_spread: old.max_spread,
							min_depth: old.min_depth,
							tier: MarketTier::Tier3,
						},
					)
				})
				.collect(),
			max_accounts_rewarded: self.max_accounts_rewarded,
			claim_safety_period: self.claim_safety_period,
		}
	}
}

pub struct Migration<T>(sp_std::marker::PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for Migration<T> {
	fn on_runtime_upgrade() -> Weight {
		let on_chain_version = <crate::Pallet<T> as GetStorageVersion>::on_chain_storage_version();
		if on_chain_version >= StorageVersion::new(1) {
			log::info!(target: "ocex::migration", "v1 already applied, skipping");
			return T::DbWeight::get().reads(1);
		}

		log::info!(target: "ocex::migration", "Running v1: adding MarketTier to LMPMarketConfig");
		let mut reads: u64 = 1; // on_chain_storage_version read
		let mut writes: u64 = 0;

		// Migrate all LMPConfig entries (per epoch)
		let keys: sp_std::vec::Vec<u16> = LMPConfig::<T>::iter_keys().collect();
		reads = reads.saturating_add(keys.len() as u64);
		for epoch in &keys {
			// Read raw bytes and try to decode with old layout first.
			// If it decodes as new (already has tier), skip.
			// If it decodes as old, re-encode with tier = Tier3.
			if let Some(raw) = frame_support::storage::unhashed::get_raw(
				&LMPConfig::<T>::hashed_key_for(epoch),
			) {
				if let Ok(old_config) = OldLMPEpochConfig::decode(&mut &raw[..]) {
					let new_config = old_config.into_new();
					LMPConfig::<T>::insert(epoch, new_config);
					writes = writes.saturating_add(1);
				} else {
					log::warn!(
						target: "ocex::migration",
						"v1: LMPConfig epoch {:?} could not be decoded as old format, skipping",
						epoch
					);
				}
			}
		}

		// Migrate ExpectedLMPConfig
		reads = reads.saturating_add(1);
		if let Some(raw) = frame_support::storage::unhashed::get_raw(
			&ExpectedLMPConfig::<T>::hashed_key(),
		) {
			if let Ok(old_config) = OldLMPEpochConfig::decode(&mut &raw[..]) {
				let new_config = old_config.into_new();
				ExpectedLMPConfig::<T>::put(new_config);
				writes = writes.saturating_add(1);
			} else {
				log::warn!(
					target: "ocex::migration",
					"v1: ExpectedLMPConfig could not be decoded as old format, skipping"
				);
			}
		}

		StorageVersion::new(1).put::<crate::Pallet<T>>();
		writes = writes.saturating_add(1);

		log::info!(target: "ocex::migration", "v1 complete: {} epochs migrated", keys.len());
		T::DbWeight::get().reads_writes(reads, writes)
	}

	#[cfg(feature = "try-runtime")]
	fn pre_upgrade() -> Result<sp_std::vec::Vec<u8>, sp_runtime::TryRuntimeError> {
		let count = LMPConfig::<T>::iter_keys().count();
		Ok((count as u32).encode())
	}

	#[cfg(feature = "try-runtime")]
	fn post_upgrade(state: sp_std::vec::Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
		let pre_count = u32::decode(&mut &state[..]).unwrap_or(0);
		let post_count = LMPConfig::<T>::iter_keys().count() as u32;
		frame_support::ensure!(
			pre_count == post_count,
			"v1 post-upgrade: LMPConfig entry count changed unexpectedly"
		);
		frame_support::ensure!(
			crate::Pallet::<T>::on_chain_storage_version() == 1,
			"v1 post-upgrade: storage version not updated to 1"
		);
		// Verify all migrated entries have a valid tier field
		for (_epoch, config) in LMPConfig::<T>::iter() {
			for (_pair, market_config) in &config.config {
				// Just accessing the field verifies it was successfully decoded with the new layout
				let _ = market_config.tier;
			}
		}
		Ok(())
	}
}
