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

use crate::types::TradingPair;
use parity_scale_codec::{Decode, Encode, MaxEncodedLen, DecodeWithMemTracking};
use rust_decimal::{
	prelude::{One, Zero},
	Decimal,
};
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use sp_std::collections::btree_map::BTreeMap;
use sp_std::vec::Vec;

/// Market tier classification for LMP reward parameters.
/// Tier1 is the highest tier (tightest spread / deepest market).
/// Defaults to Tier3 (safe default for storage migration).
#[derive(
	Decode, Encode, DecodeWithMemTracking, TypeInfo, Clone, Copy, Debug,
	Eq, PartialEq, MaxEncodedLen, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
pub enum MarketTier {
	#[default]
	Tier3,
	Tier2,
	Tier1,
}

/// LMP Epoch config
#[derive(
	Decode, Encode, TypeInfo, Copy, Clone, Debug, DecodeWithMemTracking, Eq, PartialEq, Serialize, Deserialize, Default,
)]
pub struct LMPConfig {
	pub epoch: u16,
	pub index: u16,
}

/// One minute LMP Q Score report
#[derive(Decode, Encode, TypeInfo, Clone, Debug, DecodeWithMemTracking, Eq, PartialEq, Serialize, Deserialize)]
pub struct LMPOneMinuteReport<AccountId: Ord> {
	pub market: TradingPair,
	pub epoch: u16,
	pub index: u16, // Sample index out of 40,320 samples.
	// Sum of individual scores
	pub total_score: Decimal,
	// Final Scores of all eligible main accounts
	pub scores: BTreeMap<AccountId, Decimal>,
}

#[derive(Clone, Debug, Encode, Decode, DecodeWithMemTracking, Eq, PartialEq, TypeInfo, Serialize, Deserialize)]
pub struct LMPMarketConfigWrapper {
	pub trading_pair: TradingPair,
	pub market_weightage: u128,
	pub min_fees_paid: u128,
	pub min_maker_volume: u128,
	pub max_spread: u128,
	pub min_depth: u128,
	pub tier: MarketTier,
}

/// LMP Configuration for a market
#[derive(
	Decode,
	Encode,
	DecodeWithMemTracking,
	TypeInfo,
	Clone,
	Copy,
	Debug,
	Default,
	Eq,
	PartialEq,
	MaxEncodedLen,
	PartialOrd,
	Ord,
	Serialize,
	Deserialize,
)]
pub struct LMPMarketConfig {
	// % of Rewards allocated to each market from the pool
	pub weightage: Decimal,
	// Min fees that should be paid to be eligible for rewards
	pub min_fees_paid: Decimal,
	// Min maker volume for a marker to be eligible for rewards
	pub min_maker_volume: Decimal,
	// Max spread from mid-market price an Order can have to be eligible for LMP
	// We use quoted spread here, so the formula is
	// spread ( in % )  = ((midpoint - order price)/midpoint)*100
	// midpoint = average of best bid and ask price.

	// refer: https://en.wikipedia.org/wiki/Bid–ask_spread
	pub max_spread: Decimal,
	// Minimum depth an Order must have to be eligible for LMP
	// In Base asset. ( it is basically (qty-filled_qty) of that order )
	// For example, if the order book shows that at a price of $10,000 (quote asset),
	// there are 5 BTC (base asset) available to buy or sell,
	// the order depth at that price level is 5 BTC.
	pub min_depth: Decimal,
	/// Market tier — determines Q-score exponents and maker rebate rate.
	/// Defaults to Tier3 for storage migration safety.
	pub tier: MarketTier,
}

/// A DMM (Designated Market Maker) commitment for a specific epoch and trading pair.
/// All amounts are in smallest on-chain units (u128).
#[derive(
	Encode, Decode, DecodeWithMemTracking, TypeInfo, Clone, Debug,
	PartialEq, Eq, MaxEncodedLen, Serialize, Deserialize,
)]
pub struct DMMCommitment<AccountId: MaxEncodedLen> {
	pub account: AccountId,
	/// Maximum spread commitment in basis points (on-chain u128).
	pub max_spread: u128,
	/// Minimum depth commitment in base asset smallest units.
	pub min_depth: u128,
	/// Committed uptime percentage (0–100).
	pub committed_uptime: u8,
	/// Stipend amount in PDEX smallest units.
	pub stipend: u128,
}

/// LMP Configuration for an epoch
#[serde_as]
#[derive(
	Decode, Encode, TypeInfo, Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Serialize, Deserialize, DecodeWithMemTracking,
)]
pub struct LMPEpochConfig {
	/// Total rewards given in this epoch for market making
	pub total_liquidity_mining_rewards: Decimal,
	/// Total rewards given in this epoch for trading
	pub total_trading_rewards: Decimal,
	/// Market Configurations
	#[serde_as(as = "Vec<(_, _)>")]
	pub config: BTreeMap<TradingPair, LMPMarketConfig>,
	/// Max number of accounts rewarded
	pub max_accounts_rewarded: u16,
	/// Claim safety period
	pub claim_safety_period: u32,
}

impl Default for LMPEpochConfig {
	fn default() -> Self {
		Self {
			total_liquidity_mining_rewards: Default::default(),
			total_trading_rewards: Default::default(),
			config: Default::default(),
			max_accounts_rewarded: 20,
			claim_safety_period: 50400,
		}
	}
}

impl<AccountId: MaxEncodedLen> DMMCommitment<AccountId> {
	pub fn is_valid_uptime(&self) -> bool {
		self.committed_uptime <= 100
	}
}

impl LMPEpochConfig {
	/// Checks the integrity of current config
	pub fn verify(&self) -> bool {
		// Check if market weightage adds upto 1.0
		let mut total_percent = Decimal::zero();

		for config in self.config.values() {
			total_percent = total_percent.saturating_add(config.weightage);
		}

		if total_percent != Decimal::one() {
			return false;
		}

		true
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use parity_scale_codec::{Decode, Encode};
	use rust_decimal::prelude::One;

	fn make_market_config(tier: MarketTier) -> LMPMarketConfig {
		LMPMarketConfig {
			weightage: Decimal::one(),
			min_fees_paid: Decimal::zero(),
			min_maker_volume: Decimal::zero(),
			max_spread: Decimal::zero(),
			min_depth: Decimal::zero(),
			tier,
		}
	}

	#[test]
	fn market_tier_default_is_tier3() {
		assert_eq!(MarketTier::default(), MarketTier::Tier3);
	}

	#[test]
	fn market_tier_scale_roundtrip() {
		for tier in [MarketTier::Tier1, MarketTier::Tier2, MarketTier::Tier3] {
			let encoded = tier.encode();
			let decoded = MarketTier::decode(&mut &encoded[..]).expect("decode failed");
			assert_eq!(tier, decoded);
		}
	}

	#[test]
	fn market_tier_ordering() {
		assert!(MarketTier::Tier1 > MarketTier::Tier2);
		assert!(MarketTier::Tier2 > MarketTier::Tier3);
		assert!(MarketTier::Tier1 > MarketTier::Tier3);
	}

	#[test]
	fn lmp_market_config_default_tier_is_tier3() {
		let config = make_market_config(MarketTier::default());
		assert_eq!(config.tier, MarketTier::Tier3);
	}

	#[test]
	fn lmp_market_config_scale_roundtrip_with_tier() {
		let config = make_market_config(MarketTier::Tier2);
		let encoded = config.encode();
		let decoded = LMPMarketConfig::decode(&mut &encoded[..]).expect("decode failed");
		assert_eq!(decoded.tier, MarketTier::Tier2);
		assert_eq!(decoded.weightage, config.weightage);
	}

	#[test]
	fn dmm_commitment_scale_roundtrip() {
		let c: DMMCommitment<[u8; 32]> = DMMCommitment {
			account: [1u8; 32],
			max_spread: 50,
			min_depth: 1_000_000,
			committed_uptime: 90,
			stipend: 5_000_000_000_000,
		};
		let encoded = c.encode();
		let decoded = DMMCommitment::<[u8; 32]>::decode(&mut &encoded[..]).expect("decode failed");
		assert_eq!(decoded.account, c.account);
		assert_eq!(decoded.max_spread, c.max_spread);
		assert_eq!(decoded.min_depth, c.min_depth);
		assert_eq!(decoded.committed_uptime, c.committed_uptime);
		assert_eq!(decoded.stipend, c.stipend);
	}

	#[test]
	fn dmm_commitment_uptime_boundary() {
		let c0: DMMCommitment<[u8; 32]> =
			DMMCommitment { account: [0u8; 32], max_spread: 0, min_depth: 0, committed_uptime: 0, stipend: 0 };
		assert!(c0.is_valid_uptime());
		let c100: DMMCommitment<[u8; 32]> =
			DMMCommitment { account: [0u8; 32], max_spread: 0, min_depth: 0, committed_uptime: 100, stipend: 0 };
		assert!(c100.is_valid_uptime());
		let c101: DMMCommitment<[u8; 32]> =
			DMMCommitment { account: [0u8; 32], max_spread: 0, min_depth: 0, committed_uptime: 101, stipend: 0 };
		assert!(!c101.is_valid_uptime());
	}

	#[test]
	fn lmp_epoch_config_verify_still_works_with_tier() {
		use crate::types::TradingPair;
		use polkadex_primitives::AssetId;
		let pair = TradingPair { base: AssetId::Polkadex, quote: AssetId::Asset(1) };
		let mut config = LMPEpochConfig::default();
		config.config.insert(pair, make_market_config(MarketTier::Tier1));
		assert!(config.verify(), "verify() should pass with one pair weightage = 1.0");
	}
}
