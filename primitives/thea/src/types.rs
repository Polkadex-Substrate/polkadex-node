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

//! Definition of types used for `Thea` related operations.

use frame_support::__private::log;
use parity_scale_codec::{Decode, Encode, DecodeWithMemTracking};
use polkadex_primitives::{AssetId, UNIT_BALANCE};
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};
use sp_core::H160;
#[cfg(not(feature = "std"))]
use sp_std::vec::Vec;
use sp_std::{cmp::Ordering, collections::btree_map::BTreeMap};

use crate::extras::ExtraData;
use crate::{Network, ValidatorSetId};

/// Defines the message structure.
#[derive(
	Clone, Encode, Decode, TypeInfo, Debug, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize,
)]
pub struct SignedMessage<Signature> {
	pub validator_set_id: ValidatorSetId,
	pub message: Message,
	pub signatures: BTreeMap<u32, Signature>,
}

impl<Signature> SignedMessage<Signature> {
	/// Create a new signed message
	///
	/// # Arguments
	///
	/// * `message` - The message to be signed
	/// * `validator_set_id` - The validator set id
	/// * `auth_index` - The index of the authority signing the message
	/// * `signature` - The signature of the authority
	///
	/// # Returns
	///
	/// * `Self` - The signed message
	pub fn new(
		message: Message,
		validator_set_id: ValidatorSetId,
		auth_index: u32,
		signature: Signature,
	) -> Self {
		let mut signatures = BTreeMap::new();
		signatures.insert(auth_index, signature);
		Self { validator_set_id, message, signatures }
	}

	/// Add a signature to the signed message
	///
	/// # Arguments
	///
	/// * `message` - The message to be signed
	/// * `validator_set_id` - The validator set id
	/// * `auth_index` - The index of the authority signing the message
	/// * `signature` - The signature of the authority
	pub fn add_signature(
		&mut self,
		message: Message,
		validator_set_id: ValidatorSetId,
		auth_index: u32,
		signature: Signature,
	) {
		if self.message != message {
			// silently drop if message is different
			log::error!(target:"thea", "Thea Message is not same");
			return;
		}
		// AUTHENTICATION HARDENING (2026-08-14): never merge signatures across
		// validator sets. The original code advanced the stored set id whenever
		// a newer (or attacker-chosen higher) id arrived, clearing all prior
		// signatures. This allowed a single signature from any set — including a
		// fabricated non-existent one — to reset quorum progress and ultimately
		// meet a collapsed threshold. Drop cross-set contributions silently.
		if self.validator_set_id != validator_set_id {
			log::error!(
				target: "thea",
				"Rejecting signature with mismatched validator_set_id: expected {}, got {}",
				self.validator_set_id,
				validator_set_id,
			);
			return;
		}
		self.signatures.insert(auth_index, signature);
	}

	/// Check if the signed message has reached the threshold
	///
	/// # Arguments
	///
	/// * `max_len` - The maximum length of the validator set
	///
	/// # Returns
	///
	/// * `bool` - True if the threshold is reached
	pub fn threshold_reached(&self, max_len: usize) -> bool {
		// AUTHENTICATION HARDENING (2026-08-14).
		//
		// (1) An empty authority set can never reach quorum.
		//     The original Percent-based calculation gives 67% * 0 = 0, and
		//     signatures.len() >= 0 is always true — an empty-set message was
		//     treated as finalised with zero signatures.
		if max_len == 0 {
			return false;
		}
		// (2) Use strict integer ceiling arithmetic so the threshold never floors
		//     to a value that is achievable with fewer signers than intended.
		//     (2 * n) / 3 + 1 is the smallest integer strictly greater than 2n/3,
		//     guaranteeing a genuine 2/3 supermajority for any set size.
		let threshold = (2 * max_len) / 3 + 1;
		self.signatures.len() >= threshold
	}

	/// Check if the signed message contains the signature of the authority
	///
	/// # Arguments
	///
	/// * `auth_index` - The index of the authority
	///
	/// # Returns
	///
	/// * `bool` - True if the signature is present
	pub fn contains_signature(&self, auth_index: &u32) -> bool {
		self.signatures.contains_key(auth_index)
	}
}

pub const THEA_HOLD_REASON: [u8; 8] = *b"theaRela";

#[derive(Clone, Encode, Decode, TypeInfo, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum NetworkType {
	Parachain,
	Evm,
}

#[derive(Clone, Encode, Decode, TypeInfo, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct NetworkConfig {
	pub fork_period: u32,
	pub min_stake: u128,
	pub fisherman_stake: u128,
	pub network_type: NetworkType,
}

impl Default for NetworkConfig {
	fn default() -> Self {
		Self {
			fork_period: 20,
			min_stake: 1000 * UNIT_BALANCE,
			fisherman_stake: 100 * UNIT_BALANCE,
			network_type: NetworkType::Parachain,
		}
	}
}

impl NetworkConfig {
	pub fn new(fork_period: u32, min_stake: u128, fisherman_stake: u128, is_evm: bool) -> Self {
		let key_type = if is_evm { NetworkType::Evm } else { NetworkType::Parachain };
		Self { fork_period, min_stake, fisherman_stake, network_type: key_type }
	}
}

#[derive(
	Clone, Encode, Decode, TypeInfo, Debug, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize,
)]
pub struct MisbehaviourReport<AccountId, Balance> {
	pub reported_msg: IncomingMessage<AccountId, Balance>,
	pub fisherman: AccountId,
	pub stake: Balance,
}

#[derive(
	Clone, Encode, Decode, TypeInfo, Debug, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize,
)]
pub struct IncomingMessage<AccountId, Balance> {
	pub message: Message,
	pub relayer: AccountId,
	pub stake: Balance,
	pub execute_at: u32,
}

/// Define the type of thea message
#[derive(
	Clone, Encode, Decode, DecodeWithMemTracking, TypeInfo, Debug, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize,
)]
pub enum PayloadType {
	ScheduledRotateValidators,
	ValidatorsRotated,
	L1Deposit,
}

/// Defines the message structure.
#[derive(
	Clone, Encode, Decode, DecodeWithMemTracking, TypeInfo, Debug, Eq, PartialEq, Ord, PartialOrd, Deserialize, Serialize,
)]
pub struct Message {
	/// Block number.
	pub block_no: u64,
	/// Message nonce (e.g. identifier).
	pub nonce: u64,
	/// Network
	pub network: Network,
	/// Defines how the payload must be decoded
	pub payload_type: PayloadType,
	/// Payload of the message.
	pub data: Vec<u8>,
}

/// Defines the destination of a thea message
#[derive(
	Copy,
	Clone,
	Encode,
	Decode,
	TypeInfo,
	Debug,
	Eq,
	PartialEq,
	Ord,
	PartialOrd,
	Serialize,
	Deserialize,
)]
pub enum Destination {
	Solochain,
	Parachain,
	Aggregator,
}

/// Defines structure of the deposit.
///
/// Deposit is relative to the "solochain".
#[derive(Encode, Decode, Clone, TypeInfo, PartialEq, Debug)]
pub struct Deposit<AccountId> {
	/// Identifier of the deposit.
	pub id: H160, // Unique identifier
	/// Receiver of the deposit.
	pub recipient: AccountId,
	/// Asset identifier.
	pub asset_id: AssetId,
	/// Amount of the deposit.
	pub amount: u128,
	/// Extra data.
	pub extra: ExtraData,
}

impl<AccountId> Deposit<AccountId> {
	pub fn amount_in_native_decimals(&self, metadata: AssetMetadata) -> u128 {
		metadata.convert_to_native_decimals(self.amount)
	}

	pub fn amount_in_foreign_decimals(&self, metadata: AssetMetadata) -> u128 {
		metadata.convert_from_native_decimals(self.amount)
	}
}

/// Defines the structure of the withdraw.
///
/// Withdraw is relative to solochain
#[derive(Encode, Decode, DecodeWithMemTracking, Clone, TypeInfo, PartialEq, Debug)]
pub struct Withdraw {
	/// Identifier of the withdrawal.
	pub id: H160,
	// Unique identifier
	/// Asset identifier.
	pub asset_id: AssetId,
	/// Amount of the withdrawal.
	pub amount: u128,
	/// Receiver of the withdrawal.
	pub destination: Vec<u8>,
	/// Fee Asset Id
	pub fee_asset_id: Option<AssetId>,
	/// Fee Amount
	pub fee_amount: Option<u128>,
	/// Defines if withdraw operation is blocked.
	pub is_blocked: bool,
	/// Extra data.
	pub extra: ExtraData,
}

/// Metadata of asset's decimals
#[derive(Encode, Decode, DecodeWithMemTracking, Clone, TypeInfo, PartialEq, Debug, Copy)]
pub struct AssetMetadata {
	pub decimal: u8,
}

impl AssetMetadata {
	pub fn new(decimal: u8) -> Option<AssetMetadata> {
		if decimal < 1 {
			return None;
		}
		Some(AssetMetadata { decimal })
	}

	/// Convert the foreign asset amount to native decimal configuration
	pub fn convert_to_native_decimals(&self, amount: u128) -> u128 {
		let diff = 12 - self.decimal as i8;
		match diff.cmp(&0) {
			Ordering::Less => {
				// casting should not fail as diff*-1 is positive
				amount.saturating_div(10u128.pow((-diff) as u32))
			},
			Ordering::Equal => amount,
			Ordering::Greater => amount.saturating_mul(10u128.pow(diff as u32)),
		}
	}

	/// Convert the foreign asset amount from native decimal configuration
	pub fn convert_from_native_decimals(&self, amount: u128) -> u128 {
		let diff = 12 - self.decimal as i8;

		match diff.cmp(&0) {
			Ordering::Less => {
				// casting should not fail as diff*-1 is positive
				amount.saturating_mul(10u128.pow((-diff) as u32))
			},
			Ordering::Equal => amount,
			Ordering::Greater => amount.saturating_div(10u128.pow(diff as u32)),
		}
	}
}

/// Overarching type used by aggregator to collect signatures from
/// authorities for a given Thea message
#[derive(Deserialize, Serialize, Clone)]
pub struct ApprovedMessage {
	/// Thea message
	pub message: Message,
	/// index of the authority from on-chain list
	pub index: u16,
	/// ECDSA signature of authority
	pub signature: Vec<u8>,
	/// Destination network
	pub destination: Destination,
}

#[cfg(test)]
mod tests {
	use crate::types::AssetMetadata;
	use polkadex_primitives::UNIT_BALANCE;

	#[test]
	pub fn test_decimal_conversion() {
		// Decimal is greater
		let greater = AssetMetadata::new(18).unwrap();
		assert_eq!(
			greater.convert_to_native_decimals(1_000_000_000_000_000_000_u128),
			UNIT_BALANCE
		);
		assert_eq!(
			greater.convert_from_native_decimals(UNIT_BALANCE),
			1_000_000_000_000_000_000_u128
		);
		assert_eq!(
			greater.convert_to_native_decimals(1_234_567_891_234_567_890_u128),
			1_234_567_891_234_u128
		);
		assert_eq!(
			greater.convert_from_native_decimals(1_234_567_891_234_u128),
			1_234_567_891_234_000_000_u128
		);

		// Decimal is same
		let same = AssetMetadata::new(12).unwrap();
		assert_eq!(same.convert_to_native_decimals(UNIT_BALANCE), UNIT_BALANCE);
		assert_eq!(same.convert_from_native_decimals(UNIT_BALANCE), UNIT_BALANCE);

		// Decimal is lesser
		let smaller = AssetMetadata::new(8).unwrap();
		assert_eq!(smaller.convert_to_native_decimals(100_000_000), UNIT_BALANCE);
		assert_eq!(smaller.convert_from_native_decimals(UNIT_BALANCE), 100_000_000);
		assert_eq!(smaller.convert_to_native_decimals(12_345_678u128), 123_456_780_000u128);
		assert_eq!(smaller.convert_from_native_decimals(123_456_789_123u128), 12_345_678u128);
	}
}
