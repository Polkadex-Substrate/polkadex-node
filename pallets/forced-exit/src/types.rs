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

//! Types shared by the forced-exit pallet.

use parity_scale_codec::{Decode, DecodeWithMemTracking, Encode, MaxEncodedLen};
use polkadex_primitives::AssetId;
use scale_info::TypeInfo;
use sp_core::H256;
use sp_std::vec::Vec;

/// A single account's balance in a settlement snapshot.
///
/// This is the pre-image of a merkle leaf in the snapshot's `balances_root`. `free` and
/// `in_orders` are kept separate so the leaf mirrors the engine's own accounting; a forced
/// exit pays out the sum of both, because a frozen venue has no open orders by definition.
#[derive(Clone, Encode, Decode, DecodeWithMemTracking, TypeInfo, Debug, PartialEq, Eq)]
pub struct BalanceLeaf<AccountId> {
	/// Main (funding) account the balance belongs to.
	pub account: AccountId,
	/// Asset the balance is denominated in.
	pub asset: AssetId,
	/// Balance not committed to any order, in `UNIT_BALANCE` fixed-point.
	pub free: u128,
	/// Balance locked in open orders, in `UNIT_BALANCE` fixed-point.
	pub in_orders: u128,
}

impl<AccountId: Encode> BalanceLeaf<AccountId> {
	/// Total claimable amount for this leaf under a freeze.
	pub fn total(&self) -> u128 {
		self.free.saturating_add(self.in_orders)
	}
}

/// One step of a merkle inclusion proof.
#[derive(Clone, Encode, Decode, DecodeWithMemTracking, TypeInfo, Debug, PartialEq, Eq)]
pub struct ProofNode {
	/// Hash of the sibling subtree at this level.
	pub sibling: H256,
	/// Whether the sibling sits to the left of the running hash.
	pub sibling_is_left: bool,
}

/// A merkle inclusion proof: sibling hashes ordered leaf-to-root.
pub type MerkleProof = Vec<ProofNode>;

/// A snapshot that has passed its dispute window and is therefore final.
#[derive(Clone, Encode, Decode, DecodeWithMemTracking, TypeInfo, Debug, PartialEq, Eq, MaxEncodedLen)]
pub struct FinalizedSnapshot<BlockNumber> {
	/// Merkle root over all `(account, asset)` balance leaves at this snapshot.
	pub balances_root: H256,
	/// Engine state-change watermark this snapshot commits to.
	pub state_change_id: u64,
	/// Block at which the snapshot became final.
	pub at: BlockNumber,
}

/// Settlement liveness state of the venue.
#[derive(Clone, Encode, Decode, DecodeWithMemTracking, TypeInfo, Debug, PartialEq, Eq, MaxEncodedLen)]
pub enum FreezeStatus<BlockNumber> {
	/// Venue is operating normally; forced exit is unavailable.
	Live,
	/// Settlement is frozen. Trading and deposits are over; only exits remain.
	Frozen {
		/// Block at which the freeze tripped.
		at: BlockNumber,
		/// Root that forced exits are proved against (the last finalized snapshot).
		balances_root: H256,
		/// State-change watermark of that root.
		state_change_id: u64,
	},
}

impl<BlockNumber> Default for FreezeStatus<BlockNumber> {
	fn default() -> Self {
		FreezeStatus::Live
	}
}

impl<BlockNumber> FreezeStatus<BlockNumber> {
	/// Whether settlement is currently frozen.
	pub fn is_frozen(&self) -> bool {
		matches!(self, FreezeStatus::Frozen { .. })
	}
}

/// Why a freeze was tripped. Recorded for post-mortem and for the resume motion.
#[derive(Clone, Encode, Decode, DecodeWithMemTracking, TypeInfo, Debug, PartialEq, Eq, MaxEncodedLen)]
pub enum FreezeTrigger {
	/// No finalized snapshot within `SnapshotLivenessTimeout`.
	SnapshotLiveness,
	/// An on-chain withdrawal request went unserviced past `RequestServiceTimeout`.
	UnservicedWithdrawal,
}

/// Evidence a caller presents to trip a freeze.
///
/// The caller points at the specific fact that proves settlement has failed, so the runtime
/// verifies one storage read instead of scanning for a failure. Both variants are objective
/// on-chain facts: there is no discretionary path into a freeze.
#[derive(Clone, Encode, Decode, DecodeWithMemTracking, TypeInfo, Debug, PartialEq, Eq, MaxEncodedLen)]
pub enum FreezeEvidence {
	/// No snapshot has been finalized within `SnapshotLivenessTimeout`.
	SnapshotLiveness,
	/// This request id has gone unserviced past `RequestServiceTimeout`.
	UnservicedRequest(u64),
}

/// An on-chain withdrawal request awaiting engine settlement.
#[derive(Clone, Encode, Decode, DecodeWithMemTracking, TypeInfo, Debug, PartialEq, Eq, MaxEncodedLen)]
pub struct WithdrawalRequest<BlockNumber> {
	/// Monotonic request identifier.
	pub id: u64,
	/// Asset requested.
	pub asset: AssetId,
	/// Amount requested, in `UNIT_BALANCE` fixed-point.
	pub amount: u128,
	/// Block the request was recorded on-chain.
	pub requested_at: BlockNumber,
}
