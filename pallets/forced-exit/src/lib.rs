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

//! # Forced Exit (escape hatch)
//!
//! Guarantees that a user can recover their custody balance without the settlement engine's
//! cooperation. Custody liveness must not depend on engine liveness.
//!
//! ## Mechanism
//!
//! 1. **On-chain withdrawal requests** — [`Pallet::request_withdrawal`] records a request in
//!    runtime storage with a deadline. An ignored user becomes an objective on-chain fact
//!    rather than a support ticket.
//! 2. **Permissionless freeze** — when settlement demonstrably fails (no finalized snapshot
//!    within `SnapshotLivenessTimeout`, or a request unserviced past `RequestServiceTimeout`),
//!    *anyone* may call [`Pallet::trigger_settlement_freeze`]. There is no privileged origin
//!    on this path, by design.
//! 3. **Forced exit** — while frozen, [`Pallet::force_withdraw`] pays out a user's balance as
//!    committed in the last finalized snapshot, proved by merkle inclusion, plus any deposits
//!    the chain witnessed after that snapshot.
//!
//! ## What stops the hatch being used to steal
//!
//! The hatch never trusts the claimant's assertion of their balance and never trusts the
//! operator's cooperation. It pays only what the validators last notarised:
//!
//! * Balances already traded away are not claimable — the finalized root reflects the trade,
//!   and proofs are only accepted against the *current* finalized root.
//! * Funds locked in open orders are only claimable *after* a freeze, at which point open
//!   orders are void because no engine exists to fill them.
//! * A withdrawal already approved in a snapshot has already reduced the user's balance in
//!   that same snapshot, so the normal claim path and the hatch draw on disjoint amounts.
//! * Each `(account, asset)` may exit once per settlement epoch ([`ExitClaimed`]).
//!
//! ## Dependency on settlement-layer remediation
//!
//! Forced exit is only as sound as the snapshot it trusts. This pallet MUST NOT be enabled in
//! a runtime whose settlement pallet still accepts unauthenticated snapshots, permits signer
//! duplication below threshold, executes withdrawals without a dispute window, or allows the
//! snapshot nonce to rewind. See `docs/` and the settlement pallet's outstanding remediation
//! items; [`Config::MinimumDisputeWindow`] documents the assumption but cannot enforce it.

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

pub mod merkle;
pub mod traits;
pub mod types;
pub mod weights;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

pub use weights::WeightInfo;

#[frame_support::pallet]
pub mod pallet {
	use crate::{
		merkle,
		traits::{Custody, SettlementNotifier},
		types::*,
		WeightInfo,
	};
	use frame_support::{
		pallet_prelude::*,
		traits::{Currency, ReservableCurrency},
	};
	use frame_system::pallet_prelude::*;
	use polkadex_primitives::AssetId;
	use sp_core::H256;
	use sp_runtime::traits::{Saturating, Zero};

	pub type BalanceOf<T> =
		<<T as Config>::NativeCurrency as Currency<<T as frame_system::Config>::AccountId>>::Balance;

	#[pallet::config]
	pub trait Config: frame_system::Config {
		/// Overarching event type.
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		/// Native currency, used only for the anti-spam request deposit.
		type NativeCurrency: ReservableCurrency<Self::AccountId>;

		/// Access to the custody account holding user funds.
		type Custody: Custody<Self::AccountId>;

		/// Origin permitted to resume settlement after a freeze.
		///
		/// Note the asymmetry: freezing is permissionless, resuming is governed. Recovery is a
		/// privileged action; evacuation is not.
		type GovernanceOrigin: EnsureOrigin<Self::RuntimeOrigin>;

		/// How long the engine has to service an on-chain withdrawal request.
		#[pallet::constant]
		type RequestServiceTimeout: Get<BlockNumberFor<Self>>;

		/// How long the chain tolerates no finalized snapshot before settlement is deemed dead.
		#[pallet::constant]
		type SnapshotLivenessTimeout: Get<BlockNumberFor<Self>>;

		/// Maximum concurrent withdrawal requests per account.
		#[pallet::constant]
		type MaxPendingRequests: Get<u32>;

		/// Reserved per request, returned when the request is serviced.
		#[pallet::constant]
		type RequestDeposit: Get<BalanceOf<Self>>;

		/// Documents the dispute window this pallet assumes the settlement pallet enforces
		/// before reporting a snapshot as finalized. Informational: enforcement belongs to the
		/// settlement pallet.
		#[pallet::constant]
		type MinimumDisputeWindow: Get<BlockNumberFor<Self>>;

		/// Weight information.
		type WeightInfo: WeightInfo;
	}

	#[pallet::pallet]
	#[pallet::without_storage_info]
	pub struct Pallet<T>(_);

	/// Settlement liveness state.
	#[pallet::storage]
	#[pallet::getter(fn freeze_status)]
	pub type Freeze<T: Config> = StorageValue<_, FreezeStatus<BlockNumberFor<T>>, ValueQuery>;

	/// Last snapshot reported as finalized by the settlement pallet.
	#[pallet::storage]
	#[pallet::getter(fn last_finalized)]
	pub type LastFinalized<T: Config> =
		StorageValue<_, FinalizedSnapshot<BlockNumberFor<T>>, OptionQuery>;

	/// Outstanding on-chain withdrawal requests, by id.
	#[pallet::storage]
	pub type Requests<T: Config> =
		StorageMap<_, Blake2_128Concat, u64, (T::AccountId, WithdrawalRequest<BlockNumberFor<T>>)>;

	/// Request ids outstanding per account, for lookup and bounding.
	#[pallet::storage]
	pub type AccountRequests<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, BoundedVec<u64, T::MaxPendingRequests>, ValueQuery>;

	/// Next request id to allocate.
	#[pallet::storage]
	pub type NextRequestId<T: Config> = StorageValue<_, u64, ValueQuery>;

	/// Deposits witnessed on-chain but not yet covered by a finalized snapshot.
	#[pallet::storage]
	pub type UnprocessedDeposits<T: Config> =
		StorageDoubleMap<_, Blake2_128Concat, T::AccountId, Blake2_128Concat, AssetId, u128, ValueQuery>;

	/// Settlement epoch. Incremented on resume so prior exit claims lapse without a storage
	/// sweep — clearing a claim map of unbounded size in one call is not feasible.
	#[pallet::storage]
	pub type ExitEpoch<T: Config> = StorageValue<_, u32, ValueQuery>;

	/// Records a completed forced exit for `(epoch, account, asset)`.
	#[pallet::storage]
	pub type ExitClaimed<T: Config> =
		StorageDoubleMap<_, Blake2_128Concat, (u32, T::AccountId), Blake2_128Concat, AssetId, ()>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// A withdrawal request was recorded on-chain.
		WithdrawalRequested { who: T::AccountId, id: u64, asset: AssetId, amount: u128 },
		/// The engine serviced requests for an account up to the given id.
		RequestsServiced { who: T::AccountId, up_to_request_id: u64 },
		/// A snapshot was reported finalized.
		SnapshotFinalized { balances_root: H256, state_change_id: u64 },
		/// Settlement froze. Trading is over; only exits remain.
		SettlementFrozen {
			trigger: FreezeTrigger,
			at: BlockNumberFor<T>,
			balances_root: H256,
			state_change_id: u64,
		},
		/// A forced exit paid out.
		ForcedExit { who: T::AccountId, asset: AssetId, amount: u128 },
		/// A verified claim could not be paid in full from custody. Signals engine or bridge
		/// insolvency and warrants immediate investigation.
		CustodyShortfall { who: T::AccountId, asset: AssetId, owed: u128, paid: u128 },
		/// Settlement resumed under a fresh snapshot.
		SettlementResumed { epoch: u32, balances_root: H256, state_change_id: u64 },
	}

	#[pallet::error]
	pub enum Error<T> {
		/// The venue is frozen; this action is no longer available.
		SettlementFrozen,
		/// The venue is live; forced exit is unavailable.
		SettlementNotFrozen,
		/// Too many concurrent requests for this account.
		TooManyRequests,
		/// No such withdrawal request.
		UnknownRequest,
		/// The presented evidence does not establish a settlement failure.
		FreezeConditionNotMet,
		/// The merkle proof does not reproduce the finalized balances root.
		InvalidProof,
		/// This account and asset already exited in the current epoch.
		AlreadyExited,
		/// No finalized snapshot exists, so no committed balance can be proved.
		NoFinalizedSnapshot,
		/// Nothing is claimable for this account and asset.
		NothingToClaim,
		/// Requested amount is zero.
		ZeroAmount,
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Records a withdrawal request on-chain, starting the engine's service clock.
		///
		/// The request is what makes an ignored withdrawal provable. Reserves
		/// [`Config::RequestDeposit`], refunded when the engine services it.
		#[pallet::call_index(0)]
		#[pallet::weight(T::WeightInfo::request_withdrawal())]
		pub fn request_withdrawal(
			origin: OriginFor<T>,
			asset: AssetId,
			amount: u128,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;
			ensure!(!Freeze::<T>::get().is_frozen(), Error::<T>::SettlementFrozen);
			ensure!(amount > 0, Error::<T>::ZeroAmount);

			let id = NextRequestId::<T>::mutate(|next| {
				let id = *next;
				*next = next.saturating_add(1);
				id
			});
			let now = frame_system::Pallet::<T>::block_number();

			AccountRequests::<T>::try_mutate(&who, |ids| {
				ids.try_push(id).map_err(|_| Error::<T>::TooManyRequests)
			})?;

			T::NativeCurrency::reserve(&who, T::RequestDeposit::get())?;

			Requests::<T>::insert(
				id,
				(who.clone(), WithdrawalRequest { id, asset, amount, requested_at: now }),
			);

			Self::deposit_event(Event::WithdrawalRequested { who, id, asset, amount });
			Ok(())
		}

		/// Freezes settlement. Permissionless: any account may call it, and the runtime accepts
		/// it only if the presented evidence is objectively true on-chain.
		#[pallet::call_index(1)]
		#[pallet::weight(T::WeightInfo::trigger_settlement_freeze())]
		pub fn trigger_settlement_freeze(
			origin: OriginFor<T>,
			evidence: FreezeEvidence,
		) -> DispatchResult {
			ensure_signed(origin)?;
			ensure!(!Freeze::<T>::get().is_frozen(), Error::<T>::SettlementFrozen);

			let now = frame_system::Pallet::<T>::block_number();
			let trigger = match evidence {
				FreezeEvidence::SnapshotLiveness => {
					// With no snapshot ever finalized the clock runs from genesis, so a venue
					// that never settles anything is still escapable.
					let last_at = LastFinalized::<T>::get()
						.map(|snapshot| snapshot.at)
						.unwrap_or_else(Zero::zero);
					ensure!(
						now.saturating_sub(last_at) > T::SnapshotLivenessTimeout::get(),
						Error::<T>::FreezeConditionNotMet
					);
					FreezeTrigger::SnapshotLiveness
				},
				FreezeEvidence::UnservicedRequest(id) => {
					let (_who, request) =
						Requests::<T>::get(id).ok_or(Error::<T>::UnknownRequest)?;
					ensure!(
						now.saturating_sub(request.requested_at) > T::RequestServiceTimeout::get(),
						Error::<T>::FreezeConditionNotMet
					);
					FreezeTrigger::UnservicedWithdrawal
				},
			};

			// A venue with no finalized snapshot has no committed balances; exits then cover
			// only chain-witnessed deposits, which is the correct outcome.
			let (balances_root, state_change_id) = LastFinalized::<T>::get()
				.map(|snapshot| (snapshot.balances_root, snapshot.state_change_id))
				.unwrap_or((H256::zero(), 0));

			Freeze::<T>::put(FreezeStatus::Frozen { at: now, balances_root, state_change_id });
			Self::deposit_event(Event::SettlementFrozen {
				trigger,
				at: now,
				balances_root,
				state_change_id,
			});
			Ok(())
		}

		/// Withdraws a user's committed balance directly from custody while frozen.
		///
		/// `free` and `in_orders` reconstruct the caller's leaf in the finalized balances root;
		/// `proof` is its merkle inclusion path. Any deposits witnessed after that snapshot are
		/// added, since the chain saw them arrive and no snapshot signature is needed to prove
		/// them.
		#[pallet::call_index(2)]
		#[pallet::weight(T::WeightInfo::force_withdraw(proof.len() as u32))]
		pub fn force_withdraw(
			origin: OriginFor<T>,
			asset: AssetId,
			free: u128,
			in_orders: u128,
			proof: MerkleProof,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			let (balances_root, _state_change_id) = match Freeze::<T>::get() {
				FreezeStatus::Frozen { balances_root, state_change_id, .. } =>
					(balances_root, state_change_id),
				FreezeStatus::Live => return Err(Error::<T>::SettlementNotFrozen.into()),
			};

			let epoch = ExitEpoch::<T>::get();
			ensure!(
				!ExitClaimed::<T>::contains_key((epoch, who.clone()), asset),
				Error::<T>::AlreadyExited
			);

			let committed = if balances_root == H256::zero() {
				// Nothing was ever committed, so nothing may be claimed against the root.
				ensure!(free == 0 && in_orders == 0, Error::<T>::NoFinalizedSnapshot);
				0
			} else {
				let leaf = BalanceLeaf { account: who.clone(), asset, free, in_orders };
				ensure!(
					merkle::root_from_proof(&leaf, &proof) == balances_root,
					Error::<T>::InvalidProof
				);
				leaf.total()
			};

			let pending_deposits = UnprocessedDeposits::<T>::get(&who, asset);
			let owed = committed.saturating_add(pending_deposits);
			ensure!(owed > 0, Error::<T>::NothingToClaim);

			// Exits are first-come-first-served against real custody. A shortfall should be
			// impossible; if one occurs it is reported rather than silently truncated.
			let available = T::Custody::custody_balance(asset);
			let paid = owed.min(available);
			if paid < owed {
				Self::deposit_event(Event::CustodyShortfall { who: who.clone(), asset, owed, paid });
			}
			ensure!(paid > 0, Error::<T>::NothingToClaim);

			T::Custody::release(&who, asset, paid)?;

			ExitClaimed::<T>::insert((epoch, who.clone()), asset, ());
			UnprocessedDeposits::<T>::remove(&who, asset);

			Self::deposit_event(Event::ForcedExit { who, asset, amount: paid });
			Ok(())
		}

		/// Resumes settlement under a fresh snapshot after a freeze.
		///
		/// Governance must satisfy itself that the incoming root commits only to the un-exited
		/// remainder; this pallet cannot verify that without the full balance set. Bumping the
		/// epoch lapses prior exit claims so the new root is authoritative.
		#[pallet::call_index(3)]
		#[pallet::weight(T::WeightInfo::resume_settlement())]
		pub fn resume_settlement(
			origin: OriginFor<T>,
			balances_root: H256,
			state_change_id: u64,
		) -> DispatchResult {
			T::GovernanceOrigin::ensure_origin(origin)?;
			ensure!(Freeze::<T>::get().is_frozen(), Error::<T>::SettlementNotFrozen);

			let now = frame_system::Pallet::<T>::block_number();
			let epoch = ExitEpoch::<T>::mutate(|epoch| {
				*epoch = epoch.saturating_add(1);
				*epoch
			});

			LastFinalized::<T>::put(FinalizedSnapshot { balances_root, state_change_id, at: now });
			Freeze::<T>::put(FreezeStatus::<BlockNumberFor<T>>::Live);

			Self::deposit_event(Event::SettlementResumed { epoch, balances_root, state_change_id });
			Ok(())
		}
	}

	impl<T: Config> Pallet<T> {
		/// Removes a serviced request and refunds its deposit.
		fn clear_request(who: &T::AccountId, id: u64) {
			if Requests::<T>::take(id).is_some() {
				T::NativeCurrency::unreserve(who, T::RequestDeposit::get());
				AccountRequests::<T>::mutate(who, |ids| ids.retain(|existing| *existing != id));
			}
		}
	}

	impl<T: Config> SettlementNotifier<T::AccountId> for Pallet<T> {
		fn on_snapshot_finalized(balances_root: H256, state_change_id: u64) {
			// A frozen venue accepts no further snapshots; recovery runs through
			// `resume_settlement` so it stays a governed, visible act.
			if Freeze::<T>::get().is_frozen() {
				return;
			}
			// Reject rewinds: an older snapshot must never replace a newer one.
			if let Some(current) = LastFinalized::<T>::get() {
				if state_change_id <= current.state_change_id {
					return;
				}
			}
			let now = frame_system::Pallet::<T>::block_number();
			LastFinalized::<T>::put(FinalizedSnapshot { balances_root, state_change_id, at: now });
			Self::deposit_event(Event::SnapshotFinalized { balances_root, state_change_id });
		}

		fn on_requests_serviced(who: &T::AccountId, up_to_request_id: u64) {
			let ids = AccountRequests::<T>::get(who);
			for id in ids.into_iter().filter(|id| *id <= up_to_request_id) {
				Self::clear_request(who, id);
			}
			Self::deposit_event(Event::RequestsServiced {
				who: who.clone(),
				up_to_request_id,
			});
		}

		fn on_deposit(who: &T::AccountId, asset: AssetId, amount: u128) {
			UnprocessedDeposits::<T>::mutate(who, asset, |pending| {
				*pending = pending.saturating_add(amount)
			});
		}

		fn on_deposits_settled(who: &T::AccountId, asset: AssetId, amount: u128) {
			UnprocessedDeposits::<T>::mutate(who, asset, |pending| {
				*pending = pending.saturating_sub(amount)
			});
		}

		fn is_frozen() -> bool {
			Freeze::<T>::get().is_frozen()
		}
	}
}
