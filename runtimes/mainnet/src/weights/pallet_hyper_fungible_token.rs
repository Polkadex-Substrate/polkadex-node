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

//! Weights for `pallet_hyper_fungible_token` until its benchmarks run: the crate's flat
//! placeholders, and on `send` the bridge fee hook on top.

use core::marker::PhantomData;
use frame_support::{traits::Get, weights::Weight};
use pallet_hyper_fungible_token::WeightInfo as HftWeights;

/// Weight functions for `pallet_hyper_fungible_token`.
pub struct WeightInfo<T>(PhantomData<T>);
impl<T: frame_system::Config> HftWeights for WeightInfo<T> {
	// The hook reads MaxCallData, FeeCollector, BridgeFees and both accounts, and moves the fee.
	fn send() -> Weight {
		<() as HftWeights>::send().saturating_add(T::DbWeight::get().reads_writes(5, 4))
	}
	fn register_token(c: u32) -> Weight {
		<() as HftWeights>::register_token(c)
	}
	fn update_token(c: u32) -> Weight {
		<() as HftWeights>::update_token(c)
	}
}
