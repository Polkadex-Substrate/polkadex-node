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

//! Weights for the forced-exit pallet.
//!
//! PLACEHOLDER VALUES — these are hand-estimated and must be replaced by generated
//! benchmarks before this pallet is enabled on any live runtime. `force_withdraw` in
//! particular scales with proof depth and must be benchmarked against a realistic tree
//! (see the pallet README for the target sizing).

use frame_support::weights::Weight;

/// Weight functions needed by the pallet.
pub trait WeightInfo {
	fn request_withdrawal() -> Weight;
	fn cancel_request() -> Weight;
	fn trigger_settlement_freeze() -> Weight;
	/// `d` is the merkle proof depth (bounded at 64 by `BoundedProof`).
	fn force_withdraw(d: u32) -> Weight;
	fn claim_shortfall() -> Weight;
	fn resume_settlement() -> Weight;
	fn purge_stale() -> Weight;
}

impl WeightInfo for () {
	fn request_withdrawal() -> Weight {
		Weight::from_parts(40_000_000, 0)
			.saturating_add(Weight::from_parts(0, 4_000))
	}

	fn cancel_request() -> Weight {
		Weight::from_parts(35_000_000, 0)
			.saturating_add(Weight::from_parts(0, 3_000))
	}

	fn trigger_settlement_freeze() -> Weight {
		Weight::from_parts(30_000_000, 0)
			.saturating_add(Weight::from_parts(0, 3_000))
	}

	fn force_withdraw(d: u32) -> Weight {
		// Base cost (includes clearing up to MaxPendingRequests) plus one blake2_256 of a
		// 65-byte pre-image per proof level; proof_size grows ~33 bytes per sibling node.
		Weight::from_parts(80_000_000, 0)
			.saturating_add(Weight::from_parts(1_500_000, 0).saturating_mul(d.into()))
			.saturating_add(Weight::from_parts(0, 6_000))
			.saturating_add(Weight::from_parts(0, 40).saturating_mul(d.into()))
	}

	fn claim_shortfall() -> Weight {
		Weight::from_parts(45_000_000, 0)
			.saturating_add(Weight::from_parts(0, 4_000))
	}

	fn resume_settlement() -> Weight {
		Weight::from_parts(35_000_000, 0)
			.saturating_add(Weight::from_parts(0, 3_000))
	}

	fn purge_stale() -> Weight {
		Weight::from_parts(35_000_000, 0)
			.saturating_add(Weight::from_parts(0, 3_000))
	}
}
