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

//! ISMP messages built as relayers deliver them, for the runtime tests and the `ismp_messages`
//! benchmarks.

use sp_core::U256;
use sp_std::{vec, vec::Vec};

/// A 32-byte ABI word holding `n`.
fn word(n: usize) -> Vec<u8> {
	let mut word = vec![0u8; 32];
	word[24..].copy_from_slice(&(n as u64).to_be_bytes());
	word
}

/// ABI `bytes`: the length, then the data padded to a whole word.
fn abi_bytes(data: &[u8]) -> Vec<u8> {
	let mut out = word(data.len());
	out.extend_from_slice(data);
	out.resize(out.len() + (32 - data.len() % 32) % 32, 0);
	out
}

/// The body of an HFT transfer as the EVM token contracts encode it:
/// `abi.encode(Message(bytes from, bytes to, uint256 amount, bytes data))`.
pub fn transfer_body(from: &[u8], to: &[u8], amount: U256, data: &[u8]) -> Vec<u8> {
	let (from, to, data) = (abi_bytes(from), abi_bytes(to), abi_bytes(data));
	let head = 4 * 32; // the struct's four head words; the offsets below count from its start
	let mut body = word(32); // offset of the struct, a dynamic tuple
	body.extend(word(head));
	body.extend(word(head + from.len()));
	body.extend(amount.to_big_endian());
	body.extend(word(head + from.len() + to.len()));
	body.extend(from);
	body.extend(to);
	body.extend(data);
	body
}
