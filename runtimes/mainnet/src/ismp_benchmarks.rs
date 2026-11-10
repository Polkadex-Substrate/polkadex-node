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

//! Benchmarks of unsigned ISMP messages through `Ismp::handle_unsigned`, which give
//! `weights::ismp_messages`: a GRANDPA update finalised by `k` authorities (one signature per 136
//! bytes of proof); a batch of `n` transfers into the HFT pallet at a minimal proof; and one
//! transfer with `b` bytes of unrelated proof nodes on top of its own proof.

use crate::{ismp_fixtures::*, Ismp, Runtime, RuntimeOrigin};
use frame_benchmarking::v2::*;
use frame_support::assert_ok;
use ismp::messaging::Message;
use sp_core::{crypto::KeyTypeId, ecdsa, ed25519};
use sp_std::{vec, vec::Vec};

/// The benchmarks' own pallet type; they drive `pallet_ismp` directly.
pub struct Pallet<T: Config>(core::marker::PhantomData<T>);
pub trait Config: frame_system::Config {}
impl Config for Runtime {}

const KEYS: KeyTypeId = KeyTypeId(*b"ismb");

/// `k` ed25519 keys from the keystore, generated once and reused by later runs.
fn authorities(k: u32) -> Vec<ed25519::Public> {
	let mut keys = sp_io::crypto::ed25519_public_keys(KEYS);
	while keys.len() < k as usize {
		keys.push(sp_io::crypto::ed25519_generate(KEYS, None));
	}
	keys.truncate(k as usize);
	keys
}

/// `n` transfers in with `b` bytes of unrelated proof nodes, set up to deliver. Each carries signed
/// call_data from its own keystore key, so on_accept runs its ECDSA verify and dispatch, the
/// heaviest path.
fn signed_batch(n: u32, b: u32) -> Message {
	let keys: Vec<ecdsa::Public> =
		(0..n).map(|_| sp_io::crypto::ecdsa_generate(ECDSA_KEYS, None)).collect();
	let sign = |i: u32, msg: &[u8; 32]| {
		sp_io::crypto::ecdsa_sign_prehashed(ECDSA_KEYS, &keys[i as usize], msg)
			.expect("a keystore key")
	};
	let (message, commitment) = transfers_in(n, b, Some(&sign));
	setup_transfers_in(n, commitment);
	message
}

#[benchmarks]
mod benchmarks {
	use super::*;

	// The per-request cost at a minimal proof. Runs first: its setup is quick, so a fixture error
	// fails in a minute rather than after the long consensus run.
	#[benchmark]
	fn handle_requests(n: Linear<1, 64>) {
		let message = signed_batch(n, 0);
		#[block]
		{
			assert_ok!(Ismp::handle_unsigned(RuntimeOrigin::none(), vec![message]));
		}
	}

	// The per-proof-byte cost at one request.
	#[benchmark]
	fn handle_proof(b: Linear<0, 262_144>) {
		let message = signed_batch(1, b);
		#[block]
		{
			assert_ok!(Ismp::handle_unsigned(RuntimeOrigin::none(), vec![message]));
		}
	}

	#[benchmark]
	fn handle_consensus(k: Linear<1, 1928>) {
		let keys = authorities(k);
		assert_ok!(Ismp::create_consensus_client(RuntimeOrigin::root(), grandpa_client(&keys)));
		let sign = |key: &ed25519::Public, payload: &[u8]| {
			sp_io::crypto::ed25519_sign(KEYS, key, payload).expect("a keystore key")
		};
		let update = grandpa_update(&keys, sign);
		#[block]
		{
			assert_ok!(Ismp::handle_unsigned(RuntimeOrigin::none(), vec![update]));
		}
	}
}
