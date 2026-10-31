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

//! Conservative placeholder weights for unsigned ISMP messages, until the `ismp_messages`
//! benchmarks (crate::ismp_benchmarks) replace this file: 80 µs per GRANDPA signature, 0.4 ms per
//! request with its HFT callback, 10 ns per proof byte.

use core::marker::PhantomData;
use frame_support::{traits::Get, weights::Weight};

/// Weight functions for `ismp_messages`.
pub struct WeightInfo<T>(PhantomData<T>);
impl<T: frame_system::Config> pallet_ismp::weights::MessageWeightInfo for WeightInfo<T> {
	fn handle_consensus(k: u32) -> Weight {
		Weight::from_parts(100_000_000, 0)
			.saturating_add(Weight::from_parts(80_000_000, 0).saturating_mul(k.into()))
			.saturating_add(T::DbWeight::get().reads_writes(6, 5))
	}
	fn handle_requests(n: u32, b: u32) -> Weight {
		Weight::from_parts(150_000_000, 0)
			.saturating_add(Weight::from_parts(400_000_000, 0).saturating_mul(n.into()))
			.saturating_add(Weight::from_parts(10_000, 0).saturating_mul(b.into()))
			.saturating_add(T::DbWeight::get().reads_writes(6, 2))
			.saturating_add(T::DbWeight::get().reads_writes(6, 5).saturating_mul(n.into()))
	}
}
