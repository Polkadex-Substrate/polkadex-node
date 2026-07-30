// Copyright (C) Polytope Labs Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Helper implementations for the hyper-fungible-token pallet

use alloc::string::ToString;
use polkadot_sdk::*;
use sp_core::U256;

use crate::{Config, Pallet, PALLET_ID};

impl<T: Config> Pallet<T> {
	/// Returns the pallet's custodial account for holding native assets
	pub fn pallet_account() -> T::AccountId {
		use frame_support::PalletId;
		use sp_runtime::traits::AccountIdConversion;
		PalletId(*b"hft__acc").into_account_truncating()
	}

	/// Returns true if the given module ID matches this pallet's well-known ID.
	/// Used by the runtime's IsmpRouter to route messages to this pallet.
	pub fn is_module(id: &[u8]) -> bool {
		id == PALLET_ID.to_bytes()
	}
}

/// Converts an ERC20 U256 amount to a local balance type
///
/// Scales the value between ERC20 and local decimal precision in both directions.
/// The target type must implement `FromStr`.
pub fn convert_to_balance<B: core::str::FromStr>(
	value: U256,
	erc_decimals: u8,
	local_decimals: u8,
) -> Result<B, B::Err> {
	let adjusted = if erc_decimals >= local_decimals {
		value / U256::from(10u128.pow((erc_decimals - local_decimals) as u32))
	} else {
		value * U256::from(10u128.pow((local_decimals - erc_decimals) as u32))
	};
	adjusted.to_string().parse::<B>()
}

/// Converts a local u128 balance to an ERC20 U256 amount
///
/// Scales the value between local and ERC20 decimal precision in both directions.
pub fn convert_to_erc20(value: u128, erc_decimals: u8, local_decimals: u8) -> U256 {
	if erc_decimals >= local_decimals {
		U256::from(value) * U256::from(10u128.pow((erc_decimals - local_decimals) as u32))
	} else {
		U256::from(value) / U256::from(10u128.pow((local_decimals - erc_decimals) as u32))
	}
}
