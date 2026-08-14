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

//! Merkle commitment over snapshot balances.
//!
//! The tree format is consensus-critical and must be reproducible by any third party from
//! chain data alone, so it is specified here rather than inherited from a library:
//!
//! * **Leaf order** — leaves are sorted ascending by the SCALE encoding of `(account, asset)`.
//! * **Leaf hash** — `blake2_256(0x00 ++ scale(BalanceLeaf))`.
//! * **Internal node** — `blake2_256(0x01 ++ left ++ right)`.
//! * **Odd levels** — a lone trailing node is *promoted* unchanged to the next level. It is
//!   never duplicated: duplication makes two distinct leaf sets produce the same root.
//! * **Empty tree** — the zero hash.
//!
//! The domain-separation prefixes make a leaf pre-image un-interpretable as an internal node,
//! which is what stops a forged proof from presenting an internal node as a balance leaf.

use crate::types::{BalanceLeaf, ProofNode};
use parity_scale_codec::Encode;
use sp_core::H256;
use sp_io::hashing::blake2_256;
use sp_std::vec::Vec;

/// Domain separation tag for leaf hashes.
const LEAF_PREFIX: u8 = 0x00;
/// Domain separation tag for internal node hashes.
const NODE_PREFIX: u8 = 0x01;

/// Hashes a balance leaf into its merkle leaf hash.
pub fn hash_leaf<AccountId: Encode>(leaf: &BalanceLeaf<AccountId>) -> H256 {
	let mut pre_image = Vec::with_capacity(1 + leaf.encoded_size());
	pre_image.push(LEAF_PREFIX);
	leaf.encode_to(&mut pre_image);
	H256::from(blake2_256(&pre_image))
}

/// Hashes two child hashes into their parent.
pub fn hash_nodes(left: &H256, right: &H256) -> H256 {
	let mut pre_image = [0u8; 65];
	pre_image[0] = NODE_PREFIX;
	pre_image[1..33].copy_from_slice(left.as_bytes());
	pre_image[33..65].copy_from_slice(right.as_bytes());
	H256::from(blake2_256(&pre_image))
}

/// Replays a merkle proof from `leaf` and returns the root it implies.
///
/// The caller compares the result against the trusted on-chain root; this function never
/// decides validity itself.
pub fn root_from_proof<AccountId: Encode>(
	leaf: &BalanceLeaf<AccountId>,
	proof: &[ProofNode],
) -> H256 {
	let mut running = hash_leaf(leaf);
	for node in proof {
		running = if node.sibling_is_left {
			hash_nodes(&node.sibling, &running)
		} else {
			hash_nodes(&running, &node.sibling)
		};
	}
	running
}

/// Builds a merkle root from a full leaf set.
///
/// Used by tests and by off-chain proof builders. It is deliberately not called from an
/// extrinsic: the runtime never holds the full balance set.
pub fn compute_root<AccountId: Encode + Ord + Clone>(
	leaves: &mut Vec<BalanceLeaf<AccountId>>,
) -> H256 {
	if leaves.is_empty() {
		return H256::zero();
	}
	leaves.sort_by(|a, b| (&a.account, a.asset).encode().cmp(&(&b.account, b.asset).encode()));
	let mut level: Vec<H256> = leaves.iter().map(hash_leaf).collect();
	while level.len() > 1 {
		let mut next = Vec::with_capacity(level.len().div_ceil(2));
		let mut pairs = level.chunks_exact(2);
		for pair in &mut pairs {
			next.push(hash_nodes(&pair[0], &pair[1]));
		}
		// Promote a lone trailing node rather than duplicating it.
		if let Some(last) = pairs.remainder().first() {
			next.push(*last);
		}
		level = next;
	}
	level[0]
}

/// Builds the inclusion proof for `index` within `leaves`.
///
/// Mirrors [`compute_root`] exactly; used by tests and off-chain proof builders.
pub fn build_proof<AccountId: Encode + Ord + Clone>(
	leaves: &mut Vec<BalanceLeaf<AccountId>>,
	target: &BalanceLeaf<AccountId>,
) -> Option<Vec<ProofNode>> {
	if leaves.is_empty() {
		return None;
	}
	leaves.sort_by(|a, b| (&a.account, a.asset).encode().cmp(&(&b.account, b.asset).encode()));
	let target_hash = hash_leaf(target);
	let mut level: Vec<H256> = leaves.iter().map(hash_leaf).collect();
	let mut index = level.iter().position(|h| *h == target_hash)?;
	let mut proof = Vec::new();
	while level.len() > 1 {
		let sibling_index = if index % 2 == 0 { index + 1 } else { index - 1 };
		// A promoted lone node has no sibling at this level and contributes no proof step.
		if sibling_index < level.len() {
			proof.push(ProofNode {
				sibling: level[sibling_index],
				sibling_is_left: index % 2 == 1,
			});
		}
		let mut next = Vec::with_capacity(level.len().div_ceil(2));
		let mut pairs = level.chunks_exact(2);
		for pair in &mut pairs {
			next.push(hash_nodes(&pair[0], &pair[1]));
		}
		if let Some(last) = pairs.remainder().first() {
			next.push(*last);
		}
		index /= 2;
		level = next;
	}
	Some(proof)
}
