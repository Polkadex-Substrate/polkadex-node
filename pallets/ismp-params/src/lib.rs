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

//! # ISMP Params Pallet
//!
//! The Hyperbridge coprocessor that `pallet-ismp` trusts (`Config::Coprocessor`), set by Root so
//! one wasm serves the rehearsal and mainnet. It is `None` until set, and relay-chain proofs fail
//! with `CoprocessorNotSet` until then.
//!
//! The bridge fees of `pallet-hyper-fungible-token`: a fee per (asset, destination) in that asset,
//! the account the fees go to, and the longest `call_data` a send may carry (0 by default, so
//! none). A send fails while its fee or the collector is unset. `FeeOrigin` sets the fees and the
//! cap, so they can follow gas prices; only Root sets the collector.

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;
	use ismp::host::StateMachine;
	use pallet_hyper_fungible_token::types::BridgeFee;

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	// FRAME 2512 appends `frame_system::Config<RuntimeEvent: From<Event<Self>>>` itself.
	#[pallet::config]
	pub trait Config: frame_system::Config {
		/// The id of a bridged asset.
		type AssetId: Member + Parameter;
		/// A fee amount, in the local units of the asset sent.
		type Balance: Member + Parameter + Copy;
		/// Sets the bridge fees and the `call_data` cap.
		type FeeOrigin: EnsureOrigin<Self::RuntimeOrigin>;
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// The coprocessor was set, or cleared with `None`.
		CoprocessorSet { coprocessor: Option<StateMachine> },
		/// The fee for sending an asset to a destination was set, or cleared with `None`.
		BridgeFeeSet { asset_id: T::AssetId, dest: StateMachine, fee: Option<T::Balance> },
		/// The account the bridge fees go to was set.
		FeeCollectorSet { collector: T::AccountId },
		/// The longest `call_data` a send may carry was set.
		MaxCallDataSet { max: u32 },
	}

	#[pallet::error]
	pub enum Error<T> {
		/// The fee collector must be an existing account.
		CollectorNotFound,
	}

	/// The coprocessor state machine; `None` until Root sets it.
	#[pallet::storage]
	#[pallet::unbounded] // StateMachine has no MaxEncodedLen; it encodes to at most 9 bytes.
	pub type Coprocessor<T> = StorageValue<_, StateMachine, OptionQuery>;

	/// The fee for sending an asset to a destination, in that asset; unset fails the send.
	#[pallet::storage]
	#[pallet::unbounded] // StateMachine has no MaxEncodedLen.
	pub type BridgeFees<T: Config> = StorageDoubleMap<
		_,
		Blake2_128Concat,
		T::AssetId,
		Blake2_128Concat,
		StateMachine,
		T::Balance,
		OptionQuery,
	>;

	/// The account the bridge fees go to; unset fails every send.
	#[pallet::storage]
	pub type FeeCollector<T: Config> = StorageValue<_, T::AccountId, OptionQuery>;

	/// The longest `call_data` a send may carry, in bytes; 0 until set, so none.
	#[pallet::storage]
	pub type MaxCallData<T> = StorageValue<_, u32, ValueQuery>;

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Sets the coprocessor, or clears it with `None`. Root only.
		#[pallet::call_index(0)]
		#[pallet::weight(T::DbWeight::get().writes(1))]
		pub fn set_coprocessor(
			origin: OriginFor<T>,
			coprocessor: Option<StateMachine>,
		) -> DispatchResult {
			ensure_root(origin)?;
			Coprocessor::<T>::set(coprocessor);
			Self::deposit_event(Event::CoprocessorSet { coprocessor });
			Ok(())
		}

		/// Sets the fee for sending `asset_id` to `dest`, or clears it with `None`. `FeeOrigin`.
		#[pallet::call_index(1)]
		#[pallet::weight(T::DbWeight::get().writes(1))]
		pub fn set_bridge_fee(
			origin: OriginFor<T>,
			asset_id: T::AssetId,
			dest: StateMachine,
			fee: Option<T::Balance>,
		) -> DispatchResult {
			T::FeeOrigin::ensure_origin(origin)?;
			BridgeFees::<T>::set(&asset_id, dest, fee);
			Self::deposit_event(Event::BridgeFeeSet { asset_id, dest, fee });
			Ok(())
		}

		/// Sets the account the bridge fees go to, which must exist. Root only.
		#[pallet::call_index(2)]
		#[pallet::weight(T::DbWeight::get().reads_writes(1, 1))]
		pub fn set_fee_collector(origin: OriginFor<T>, collector: T::AccountId) -> DispatchResult {
			ensure_root(origin)?;
			ensure!(
				frame_system::Pallet::<T>::account_exists(&collector),
				Error::<T>::CollectorNotFound
			);
			FeeCollector::<T>::put(&collector);
			Self::deposit_event(Event::FeeCollectorSet { collector });
			Ok(())
		}

		/// Sets the longest `call_data` a send may carry, in bytes. `FeeOrigin`.
		#[pallet::call_index(3)]
		#[pallet::weight(T::DbWeight::get().writes(1))]
		pub fn set_max_call_data(origin: OriginFor<T>, max: u32) -> DispatchResult {
			T::FeeOrigin::ensure_origin(origin)?;
			MaxCallData::<T>::put(max);
			Self::deposit_event(Event::MaxCallDataSet { max });
			Ok(())
		}
	}

	impl<T: Config> Get<Option<StateMachine>> for Pallet<T> {
		fn get() -> Option<StateMachine> {
			Coprocessor::<T>::get()
		}
	}

	// The fee hook of `pallet-hyper-fungible-token`'s `send`.
	impl<T: Config> BridgeFee<T::AccountId, T::AssetId, T::Balance> for Pallet<T> {
		fn fee(asset: &T::AssetId, dest: &StateMachine) -> Option<(T::AccountId, T::Balance)> {
			Some((FeeCollector::<T>::get()?, BridgeFees::<T>::get(asset, dest)?))
		}

		fn max_call_data() -> u32 {
			MaxCallData::<T>::get()
		}
	}
}
