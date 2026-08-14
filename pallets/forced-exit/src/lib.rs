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
//!    committed in the last finalized snapshot (merkle inclusion proof) plus any deposits the
//!    chain witnessed after that snapshot. If custody cannot cover a verified claim, the
//!    unpaid remainder is recorded in [`ShortfallOwed`] and stays claimable via
//!    [`Pallet::claim_shortfall`] once custody is replenished.
//! 4. **Resume** — [`Pallet::resume_settlement`] restarts the venue under governance. The
//!    epoch bump lapses prior claims, deposit tallies, and shortfall records without an
//!    unbounded storage sweep; the liveness baseline resets so stale requests cannot
//!    instantly re-trip the freeze.
//!
//! ## Epochs and the liveness baseline
//!
//! All per-user claim state ([`ExitClaimed`], [`UnprocessedDeposits`], [`ShortfallOwed`]) is
//! keyed by the current [`ExitEpoch`]. Resuming increments the epoch, so old entries lapse
//! lazily (reclaimable via [`Pallet::purge_stale`]) instead of being swept in one call.
//!
//! **Consequence for governance:** the snapshot supplied to `resume_settlement` is the sole
//! authority for the new epoch. It MUST fold in (a) balances of users who did not force-exit,
//! (b) their deposits that no snapshot had covered, and (c) any unpaid [`ShortfallOwed`]
//! remainders. Whatever the resume root omits, the runtime will no longer honour.
//!
//! [`LivenessBaseline`] records the later of pallet activation and the last resume. Both
//! freeze conditions measure from it, so neither a fresh deployment on a long-running chain
//! nor a just-resumed venue with old requests can be frozen before the engine has had a full
//! timeout window to act.
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
		traits::{Currency, ReservableCurrency, StorageVersion},
	};
	use frame_system::pallet_prelude::*;
	use polkadex_primitives::AssetId;
	use sp_core::H256;
	use sp_runtime::traits::{Saturating, Zero};

	pub type BalanceOf<T> =
		<<T as Config>::NativeCurrency as Currency<<T as frame_system::Config>::AccountId>>::Balance;

	/// A merkle path can never legitimately exceed the tree depth of a 2^64-leaf tree.
	pub type BoundedProof = BoundedVec<ProofNode, ConstU32<64>>;

	const STORAGE_VERSION: StorageVersion = StorageVersion::new(1);

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

		/// Reserved per request, returned when the request is serviced or cancelled.
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
	#[pallet::storage_version(STORAGE_VERSION)]
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

	/// The later of pallet activation and the last resume. Both freeze conditions measure
	/// from this, so a fresh deployment or a just-resumed venue cannot be frozen before the
	/// engine has had a full timeout window.
	#[pallet::storage]
	pub type LivenessBaseline<T: Config> = StorageValue<_, BlockNumberFor<T>, OptionQuery>;

	/// Outstanding on-chain withdrawal requests: id → (owner, request, reserved deposit).
	///
	/// The deposit actually reserved is stored so the exact amount is unreserved later, even
	/// if `RequestDeposit` changes across a runtime upgrade.
	#[pallet::storage]
	pub type Requests<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		u64,
		(T::AccountId, WithdrawalRequest<BlockNumberFor<T>>, BalanceOf<T>),
	>;

	/// Request ids outstanding per account, for lookup and bounding.
	#[pallet::storage]
	pub type AccountRequests<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, BoundedVec<u64, T::MaxPendingRequests>, ValueQuery>;

	/// Next request id to allocate.
	#[pallet::storage]
	pub type NextRequestId<T: Config> = StorageValue<_, u64, ValueQuery>;

	/// Deposits witnessed on-chain but not yet covered by a finalized snapshot, keyed by the
	/// epoch they were made in. Entries from lapsed epochs are purgeable; the resume root is
	/// required to account for them.
	#[pallet::storage]
	pub type UnprocessedDeposits<T: Config> = StorageDoubleMap<
		_,
		Blake2_128Concat,
		(u32, T::AccountId),
		Blake2_128Concat,
		AssetId,
		u128,
		ValueQuery,
	>;

	/// Settlement epoch. Incremented on resume so prior claim state lapses without a storage
	/// sweep — clearing maps of unbounded size in one call is not feasible.
	#[pallet::storage]
	pub type ExitEpoch<T: Config> = StorageValue<_, u32, ValueQuery>;

	/// Records a completed forced exit for `(epoch, account, asset)`.
	#[pallet::storage]
	pub type ExitClaimed<T: Config> =
		StorageDoubleMap<_, Blake2_128Concat, (u32, T::AccountId), Blake2_128Concat, AssetId, ()>;

	/// Verified but unpaid claim remainders (custody shortfall), claimable while the same
	/// epoch's freeze persists via [`Pallet::claim_shortfall`]. The resume root is required
	/// to fold any remainders still here into the next epoch.
	#[pallet::storage]
	pub type ShortfallOwed<T: Config> = StorageDoubleMap<
		_,
		Blake2_128Concat,
		(u32, T::AccountId),
		Blake2_128Concat,
		AssetId,
		u128,
		ValueQuery,
	>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// A withdrawal request was recorded on-chain.
		WithdrawalRequested { who: T::AccountId, id: u64, asset: AssetId, amount: u128 },
		/// The owner cancelled a withdrawal request.
		RequestCancelled { who: T::AccountId, id: u64 },
		/// The engine serviced the listed requests inside a finalized snapshot.
		RequestsServiced { who: T::AccountId, count: u32 },
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
		/// A verified claim could not be paid in full from custody; the remainder is recorded
		/// in [`ShortfallOwed`]. Signals engine or bridge insolvency and warrants immediate
		/// investigation.
		CustodyShortfall { who: T::AccountId, asset: AssetId, owed: u128, paid: u128 },
		/// A previously recorded shortfall remainder was (partially) paid.
		ShortfallClaimed { who: T::AccountId, asset: AssetId, paid: u128, remaining: u128 },
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
		/// The caller does not own this withdrawal request.
		NotRequestOwner,
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
		/// Custody currently holds nothing of this asset; retry once replenished.
		CustodyEmpty,
		/// Requested amount is zero.
		ZeroAmount,
		/// The referenced epoch has not lapsed yet.
		EpochNotLapsed,
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		/// Seeds the liveness baseline when the pallet first appears on a running chain, so
		/// the snapshot-liveness clock cannot be measured from block zero and trip a freeze
		/// in the first block after deployment.
		fn on_runtime_upgrade() -> Weight {
			if LivenessBaseline::<T>::get().is_none() {
				LivenessBaseline::<T>::put(frame_system::Pallet::<T>::block_number());
				T::DbWeight::get().reads_writes(1, 1)
			} else {
				T::DbWeight::get().reads(1)
			}
		}
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Records a withdrawal request on-chain, starting the engine's service clock.
		///
		/// The request is what makes an ignored withdrawal provable. Reserves
		/// [`Config::RequestDeposit`], refunded when the request is serviced, cancelled, or
		/// cleared by a forced exit.
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

			let deposit = T::RequestDeposit::get();
			T::NativeCurrency::reserve(&who, deposit)?;

			Requests::<T>::insert(
				id,
				(who.clone(), WithdrawalRequest { id, asset, amount, requested_at: now }, deposit),
			);

			Self::deposit_event(Event::WithdrawalRequested { who, id, asset, amount });
			Ok(())
		}

		/// Cancels the caller's own withdrawal request and refunds its deposit.
		#[pallet::call_index(1)]
		#[pallet::weight(T::WeightInfo::cancel_request())]
		pub fn cancel_request(origin: OriginFor<T>, id: u64) -> DispatchResult {
			let who = ensure_signed(origin)?;
			let (owner, _request, deposit) =
				Requests::<T>::get(id).ok_or(Error::<T>::UnknownRequest)?;
			ensure!(owner == who, Error::<T>::NotRequestOwner);

			Requests::<T>::remove(id);
			AccountRequests::<T>::mutate(&who, |ids| ids.retain(|existing| *existing != id));
			T::NativeCurrency::unreserve(&who, deposit);

			Self::deposit_event(Event::RequestCancelled { who, id });
			Ok(())
		}

		/// Freezes settlement. Permissionless: any account may call it, and the runtime
		/// accepts it only if the presented evidence is objectively true on-chain.
		///
		/// Both conditions measure from [`LivenessBaseline`] as well as their own clock, so
		/// neither a fresh deployment nor a just-resumed venue with pre-freeze requests can
		/// be frozen before the engine has had a full timeout window to act.
		#[pallet::call_index(2)]
		#[pallet::weight(T::WeightInfo::trigger_settlement_freeze())]
		pub fn trigger_settlement_freeze(
			origin: OriginFor<T>,
			evidence: FreezeEvidence,
		) -> DispatchResult {
			ensure_signed(origin)?;
			ensure!(!Freeze::<T>::get().is_frozen(), Error::<T>::SettlementFrozen);

			let now = frame_system::Pallet::<T>::block_number();
			let baseline = LivenessBaseline::<T>::get().unwrap_or_else(Zero::zero);
			let trigger = match evidence {
				FreezeEvidence::SnapshotLiveness => {
					// With no snapshot ever finalized the clock runs from the baseline, so a
					// venue that never settles anything is still escapable, while a freshly
					// activated pallet on an old chain is not instantly freezable.
					let last_at = LastFinalized::<T>::get()
						.map(|snapshot| snapshot.at)
						.unwrap_or_else(Zero::zero)
						.max(baseline);
					ensure!(
						now.saturating_sub(last_at) > T::SnapshotLivenessTimeout::get(),
						Error::<T>::FreezeConditionNotMet
					);
					FreezeTrigger::SnapshotLiveness
				},
				FreezeEvidence::UnservicedRequest(id) => {
					let (_who, request, _deposit) =
						Requests::<T>::get(id).ok_or(Error::<T>::UnknownRequest)?;
					let clock_start = request.requested_at.max(baseline);
					ensure!(
						now.saturating_sub(clock_start) > T::RequestServiceTimeout::get(),
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
		/// `free` and `in_orders` reconstruct the caller's leaf in the finalized balances
		/// root; `proof` is its merkle inclusion path. Deposits witnessed after that snapshot
		/// are added, since the chain saw them arrive. If custody cannot cover the verified
		/// claim, the paid portion is released and the remainder recorded in
		/// [`ShortfallOwed`] — the call still succeeds so the shortfall is permanently
		/// visible on-chain and claimable later via [`Pallet::claim_shortfall`].
		///
		/// The caller's outstanding withdrawal requests are cleared and their deposits
		/// refunded: a dead engine will never service them.
		#[pallet::call_index(3)]
		#[pallet::weight(T::WeightInfo::force_withdraw(proof.len() as u32))]
		pub fn force_withdraw(
			origin: OriginFor<T>,
			asset: AssetId,
			free: u128,
			in_orders: u128,
			proof: BoundedProof,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			let balances_root = match Freeze::<T>::get() {
				FreezeStatus::Frozen { balances_root, .. } => balances_root,
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

			let pending_deposits = UnprocessedDeposits::<T>::get((epoch, who.clone()), asset);
			let owed = committed.saturating_add(pending_deposits);
			ensure!(owed > 0, Error::<T>::NothingToClaim);

			// Exits are first-come-first-served against real custody. A shortfall should be
			// impossible; if one occurs, pay what exists and record the remainder so the debt
			// survives on-chain and remains claimable after replenishment.
			let available = T::Custody::custody_balance(asset);
			let paid = owed.min(available);
			let unpaid = owed.saturating_sub(paid);

			if paid > 0 {
				T::Custody::release(&who, asset, paid)?;
			}

			ExitClaimed::<T>::insert((epoch, who.clone()), asset, ());
			UnprocessedDeposits::<T>::remove((epoch, who.clone()), asset);
			Self::clear_account_requests(&who);

			if unpaid > 0 {
				ShortfallOwed::<T>::insert((epoch, who.clone()), asset, unpaid);
				Self::deposit_event(Event::CustodyShortfall {
					who: who.clone(),
					asset,
					owed,
					paid,
				});
			}
			if paid > 0 {
				Self::deposit_event(Event::ForcedExit { who, asset, amount: paid });
			}
			Ok(())
		}

		/// Pays out a previously recorded shortfall remainder once custody holds funds again.
		///
		/// No proof is needed: the claim was verified when the shortfall was recorded.
		#[pallet::call_index(4)]
		#[pallet::weight(T::WeightInfo::claim_shortfall())]
		pub fn claim_shortfall(origin: OriginFor<T>, asset: AssetId) -> DispatchResult {
			let who = ensure_signed(origin)?;
			ensure!(Freeze::<T>::get().is_frozen(), Error::<T>::SettlementNotFrozen);

			let epoch = ExitEpoch::<T>::get();
			let remaining = ShortfallOwed::<T>::get((epoch, who.clone()), asset);
			ensure!(remaining > 0, Error::<T>::NothingToClaim);

			let available = T::Custody::custody_balance(asset);
			let paid = remaining.min(available);
			ensure!(paid > 0, Error::<T>::CustodyEmpty);

			T::Custody::release(&who, asset, paid)?;

			let still_owed = remaining.saturating_sub(paid);
			if still_owed > 0 {
				ShortfallOwed::<T>::insert((epoch, who.clone()), asset, still_owed);
			} else {
				ShortfallOwed::<T>::remove((epoch, who.clone()), asset);
			}

			Self::deposit_event(Event::ShortfallClaimed { who, asset, paid, remaining: still_owed });
			Ok(())
		}

		/// Resumes settlement under a fresh snapshot after a freeze.
		///
		/// The supplied root is the sole authority for the new epoch: governance MUST verify
		/// out-of-band that it commits to the un-exited remainder — including unsnapshotted
		/// deposits of non-exited users and any unpaid [`ShortfallOwed`] remainders — and
		/// that its total does not exceed custody. This pallet cannot verify that without the
		/// full balance set. The epoch bump lapses all prior claim state, and the liveness
		/// baseline resets so pre-freeze requests cannot instantly re-trip the freeze.
		#[pallet::call_index(5)]
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
			LivenessBaseline::<T>::put(now);
			Freeze::<T>::put(FreezeStatus::<BlockNumberFor<T>>::Live);

			Self::deposit_event(Event::SettlementResumed { epoch, balances_root, state_change_id });
			Ok(())
		}

		/// Frees claim-state storage from a lapsed epoch. Permissionless housekeeping: the
		/// entries are unreachable once their epoch has passed.
		#[pallet::call_index(6)]
		#[pallet::weight(T::WeightInfo::purge_stale())]
		pub fn purge_stale(
			origin: OriginFor<T>,
			epoch: u32,
			who: T::AccountId,
			asset: AssetId,
		) -> DispatchResult {
			ensure_signed(origin)?;
			ensure!(epoch < ExitEpoch::<T>::get(), Error::<T>::EpochNotLapsed);

			ExitClaimed::<T>::remove((epoch, who.clone()), asset);
			UnprocessedDeposits::<T>::remove((epoch, who.clone()), asset);
			ShortfallOwed::<T>::remove((epoch, who), asset);
			Ok(())
		}
	}

	impl<T: Config> Pallet<T> {
		/// Removes all of an account's outstanding requests, refunding each stored deposit.
		/// Bounded by `MaxPendingRequests`.
		fn clear_account_requests(who: &T::AccountId) {
			let ids = AccountRequests::<T>::take(who);
			for id in ids {
				if let Some((_owner, _request, deposit)) = Requests::<T>::take(id) {
					T::NativeCurrency::unreserve(who, deposit);
				}
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

		fn on_requests_serviced(
			who: &T::AccountId,
			request_ids: &[u64],
			state_change_id: u64,
		) {
			if Freeze::<T>::get().is_frozen() {
				return;
			}
			// Servicing is only honoured on the back of the currently finalized snapshot:
			// the settlement pallet must call this while finalizing the snapshot whose
			// withdrawal set actually contains these requests. A bare assertion with a stale
			// or future state_change_id clears nothing, so a censoring engine cannot destroy
			// a user's freeze evidence without committing the payment on-chain.
			let Some(current) = LastFinalized::<T>::get() else { return };
			if current.state_change_id != state_change_id {
				return;
			}
			let mut cleared: u32 = 0;
			let mut retained = AccountRequests::<T>::get(who);
			for id in request_ids {
				if let Some((owner, _request, deposit)) = Requests::<T>::get(id) {
					if owner == *who {
						Requests::<T>::remove(id);
						T::NativeCurrency::unreserve(who, deposit);
						retained.retain(|existing| existing != id);
						cleared = cleared.saturating_add(1);
					}
				}
			}
			AccountRequests::<T>::insert(who, retained);
			if cleared > 0 {
				Self::deposit_event(Event::RequestsServiced { who: who.clone(), count: cleared });
			}
		}

		fn on_deposit(who: &T::AccountId, asset: AssetId, amount: u128) {
			// Defense in depth: the settlement pallet must refuse deposits while frozen (the
			// funds should never enter custody), so a tally here would only create a
			// double-claim path if that check is ever missed.
			if Freeze::<T>::get().is_frozen() {
				return;
			}
			let epoch = ExitEpoch::<T>::get();
			UnprocessedDeposits::<T>::mutate((epoch, who.clone()), asset, |pending| {
				*pending = pending.saturating_add(amount)
			});
		}

		fn on_deposits_settled(who: &T::AccountId, asset: AssetId, amount: u128) {
			let epoch = ExitEpoch::<T>::get();
			UnprocessedDeposits::<T>::mutate((epoch, who.clone()), asset, |pending| {
				*pending = pending.saturating_sub(amount)
			});
		}

		fn is_frozen() -> bool {
			Freeze::<T>::get().is_frozen()
		}
	}
}
