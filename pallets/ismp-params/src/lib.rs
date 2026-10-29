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

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;
	use ismp::host::StateMachine;

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	// FRAME 2512 appends `frame_system::Config<RuntimeEvent: From<Event<Self>>>` itself.
	#[pallet::config]
	pub trait Config: frame_system::Config {}

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// The coprocessor was set, or cleared with `None`.
		CoprocessorSet { coprocessor: Option<StateMachine> },
	}

	/// The coprocessor state machine; `None` until Root sets it.
	#[pallet::storage]
	#[pallet::unbounded] // StateMachine has no MaxEncodedLen; it encodes to at most 9 bytes.
	pub type Coprocessor<T> = StorageValue<_, StateMachine, OptionQuery>;

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
	}

	impl<T: Config> Get<Option<StateMachine>> for Pallet<T> {
		fn get() -> Option<StateMachine> {
			Coprocessor::<T>::get()
		}
	}
}
