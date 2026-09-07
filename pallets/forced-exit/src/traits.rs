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

//! Interfaces between the forced-exit pallet and the settlement pallet.
//!
//! The two directions are kept as separate traits so neither pallet needs a Cargo dependency
//! on the other: the settlement pallet calls [`SettlementNotifier`] (implemented here), and
//! this pallet calls [`Custody`] (implemented there).

use frame_support::pallet_prelude::DispatchResult;
use polkadex_primitives::AssetId;
use sp_core::H256;

/// Custody operations this pallet needs from the settlement pallet.
pub trait Custody<AccountId> {
	/// Transfers `amount` of `asset` out of the custody account to `who`.
	///
	/// Implementations must not apply venue-level fees to a forced exit: a user leaving a
	/// dead venue is not a trading action.
	fn release(who: &AccountId, asset: AssetId, amount: u128) -> DispatchResult;

	/// Current custody holdings of `asset`, used to detect shortfalls.
	fn custody_balance(asset: AssetId) -> u128;
}

/// Events the settlement pallet reports into this pallet.
///
/// Implemented by [`crate::Pallet`]. The settlement pallet holds this as an associated type
/// so the coupling stays one-way and testable.
pub trait SettlementNotifier<AccountId> {
	/// Reports a snapshot that has cleared its dispute window and is now final.
	///
	/// `state_change_id` must be strictly increasing; the implementation rejects rewinds.
	fn on_snapshot_finalized(balances_root: H256, state_change_id: u64);

	/// Reports that the withdrawals for the listed request ids were included in the snapshot
	/// finalized at `state_change_id`.
	///
	/// **Contract:** this must be called only while finalizing the snapshot whose withdrawal
	/// set actually contains these requests — servicing is payment-by-inclusion, never a bare
	/// assertion. The implementation ignores calls whose `state_change_id` does not match the
	/// currently finalized snapshot, so a censoring engine cannot destroy a user's
	/// unserviced-request freeze evidence without committing the payment on-chain.
	fn on_requests_serviced(who: &AccountId, request_ids: &[u64], state_change_id: u64);

	/// Reports a deposit that is not yet covered by any finalized snapshot.
	fn on_deposit(who: &AccountId, asset: AssetId, amount: u128);

	/// Reports that a finalized snapshot now accounts for `amount` of previously unprocessed
	/// deposits, so the forced-exit top-up no longer needs to cover them.
	///
	/// Without this the top-up would double-pay: once from the snapshot leaf that already
	/// includes the deposit, and again from the unprocessed-deposit tally.
	fn on_deposits_settled(who: &AccountId, asset: AssetId, amount: u128);

	/// Whether settlement is frozen. The settlement pallet must reject snapshots and
	/// deposits when this is `true`.
	fn is_frozen() -> bool;
}
