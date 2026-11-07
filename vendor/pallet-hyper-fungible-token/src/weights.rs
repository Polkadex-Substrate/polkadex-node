// Copyright (C) Polytope Labs Ltd.
// SPDX-License-Identifier: Apache-2.0
// Modified by The Polkadex Authors in 2026; see CHANGES.md.

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

//! Weight information for the hyper-fungible-token pallet

use polkadot_sdk::sp_runtime::Weight;

/// Polkadex: `send` is weighed by path (PDEX, or another asset held or burnt) and `call_data`
/// length `d`; `register_token` and `update_token` by the number of chains `c`.
pub trait WeightInfo {
	fn send_native(d: u32) -> Weight;
	fn send_asset(d: u32) -> Weight;
	fn register_token(c: u32) -> Weight;
	fn update_token(c: u32) -> Weight;
}

impl WeightInfo for () {
	fn send_native(_d: u32) -> Weight {
		Weight::from_parts(100_000_000, 0)
	}
	fn send_asset(_d: u32) -> Weight {
		Weight::from_parts(100_000_000, 0)
	}
	fn register_token(_c: u32) -> Weight {
		Weight::from_parts(100_000_000, 0)
	}
	fn update_token(_c: u32) -> Weight {
		Weight::from_parts(100_000_000, 0)
	}
}
