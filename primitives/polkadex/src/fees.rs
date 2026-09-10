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

//! In this module defined operations fee related types.

use codec::{Decode, Encode};
use rust_decimal::{prelude::FromPrimitive, Decimal};
use scale_info::TypeInfo;

#[cfg(feature = "std")]
use serde::{Deserialize, Serialize};

/// Defines structure of the fee configuration.
#[derive(Copy, Clone, Encode, Decode, PartialEq, Debug, TypeInfo)]
#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
pub struct FeeConfig {
	/// Market fee fraction.
	pub maker_fraction: Decimal,
	/// Trade fee fraction.
	pub taker_fraction: Decimal,
}

impl Default for FeeConfig {
	fn default() -> Self {
		Self {
			maker_fraction: Decimal::from_f64(0.001).unwrap(),
			taker_fraction: Decimal::from_f64(0.001).unwrap(),
		}
	}
}

impl FeeConfig {
	/// SECURITY (M16): validate that fee fractions are within [0, 1].
	///
	/// A fraction > 1 would make the computed fee exceed the trade credit — the surplus is
	/// silently lost through `saturating_sub`. A negative fraction would credit the user
	/// instead of charging them, draining the fee pot. Governance or operator paths that
	/// set a FeeConfig must call this before accepting the config.
	pub fn validate(&self) -> Result<(), &'static str> {
		let zero = Decimal::ZERO;
		let one = Decimal::ONE;
		if self.maker_fraction < zero || self.maker_fraction > one {
			return Err("FeeConfig: maker_fraction must be in [0, 1]");
		}
		if self.taker_fraction < zero || self.taker_fraction > one {
			return Err("FeeConfig: taker_fraction must be in [0, 1]");
		}
		Ok(())
	}
}
