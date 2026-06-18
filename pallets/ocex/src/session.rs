// This file is part of Polkadex.
//
// Copyright (c) 2022-2023 Polkadex oü.
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

use crate::pallet::IngressMessages;
use crate::{
	pallet::{
		Config, DMMRegistry, ExpectedLMPConfig, FeesCollected, LMPConfig, Pallet,
		VolatilityActive,
	},
	FinalizeLMPScore, LMPEpoch,
};
use frame_support::traits::{Currency, ExistenceRequirement, Get};
use num_traits::Zero;
use sp_runtime::Saturating;
use frame_system::pallet_prelude::BlockNumberFor;
use orderbook_primitives::traits::LiquidityMiningCrowdSourcePallet;
use sp_runtime::{traits::AccountIdConversion, SaturatedConversion};

const EPOCH_LENGTH: u32 = 201600u32; // 28 days in blocks

impl<T: Config> Pallet<T> {
	pub(crate) fn should_start_new_epoch(n: BlockNumberFor<T>) -> bool {
		n.saturated_into::<u32>() % EPOCH_LENGTH == 0
	}

	/// Starts new liquidity mining epoch
	pub fn start_new_epoch(n: BlockNumberFor<T>) {
		if let Some(config) = <ExpectedLMPConfig<T>>::get() {
			let mut current_epoch: u16 = <LMPEpoch<T>>::get();
			// P3: Transfer 25% of accumulated fees for the expiring epoch to the LMP rewards account.
			Self::distribute_lmp_fee_split(current_epoch);
			// P4: Clear VolatilityActive flags for all pairs at epoch boundary.
			let _ = <VolatilityActive<T>>::clear(u32::MAX, None);
			// P5: Reserve DMM stipends for the incoming epoch from the LMP rewards account.
			Self::reserve_dmm_stipends(current_epoch.saturating_add(1));
			//This is to handle the corner case when epoch is 0
			if current_epoch == 0 && !<LMPConfig<T>>::contains_key(current_epoch) {
				<LMPConfig<T>>::insert(current_epoch, config.clone());
			}
			if <FinalizeLMPScore<T>>::get().is_none() {
				<FinalizeLMPScore<T>>::put(current_epoch);
			}
			current_epoch = current_epoch.saturating_add(1);
			<LMPEpoch<T>>::put(current_epoch);
			<LMPConfig<T>>::insert(current_epoch, config.clone());
			// Notify Liquidity Crowd sourcing pallet about new epoch
			T::CrowdSourceLiqudityMining::new_epoch(current_epoch);

			<IngressMessages<T>>::mutate(n, |ingress_messages| {
				ingress_messages.push(orderbook_primitives::ingress::IngressMessages::NewLMPEpoch(
					current_epoch,
				));
				ingress_messages
					.push(orderbook_primitives::ingress::IngressMessages::LMPConfig(config))
			});
		}
	}

	pub(crate) fn should_stop_accepting_lmp_withdrawals(n: BlockNumberFor<T>) -> bool {
		// Triggers 7200 blocks ( or approx 1 day before epoch change)
		n.saturated_into::<u32>().saturating_add(7200) % EPOCH_LENGTH == 0
	}

	pub(crate) fn stop_accepting_lmp_withdrawals() {
		let current_epoch: u16 = <LMPEpoch<T>>::get();
		T::CrowdSourceLiqudityMining::stop_accepting_lmp_withdrawals(current_epoch)
	}

	/// P5: For every confirmed DMM in the incoming epoch, reserve their stipend
	/// by transferring from the LMP rewards account to the pallet account.
	/// The pallet account holds the funds until `claim_dmm_stipend` is called.
	/// Failures are logged but do not abort epoch transition.
	pub(crate) fn reserve_dmm_stipends(new_epoch: u16) {
		let rewards_account: T::AccountId =
			T::LMPRewardsPalletId::get().into_account_truncating();
		let pallet_account = Self::get_pallet_account();
		// Iterate all pairs that have DMM registrations for the new epoch.
		for (pair, commitments) in <DMMRegistry<T>>::iter_prefix(new_epoch) {
			let total_stipend: crate::BalanceOf<T> = commitments
				.iter()
				.fold(Zero::zero(), |acc: crate::BalanceOf<T>, c| {
					acc.saturating_add(c.stipend.saturated_into())
				});
			if total_stipend.is_zero() {
				continue;
			}
			if let Err(e) = T::NativeCurrency::transfer(
				&rewards_account,
				&pallet_account,
				total_stipend,
				ExistenceRequirement::KeepAlive,
			) {
				log::warn!(
					target: "ocex",
					"DMM stipend reservation failed for epoch {:?} pair {:?}: {:?}",
					new_epoch, pair, e
				);
			}
		}
	}

	/// P3: Drain `FeesCollected` for `epoch`, compute 25%, transfer to LMP rewards account.
	/// Uses `KeepAlive` to avoid inadvertently killing the source account.
	/// Failures are logged but do not abort epoch transition (parachain safety).
	pub(crate) fn distribute_lmp_fee_split(epoch: u16) {
		let rewards_account: T::AccountId =
			T::LMPRewardsPalletId::get().into_account_truncating();
		let pallet_account = Self::get_pallet_account();
		for (pair, fees) in <FeesCollected<T>>::drain_prefix(epoch) {
			let lmp_cut = fees / 4u32.into();
			if lmp_cut.is_zero() {
				continue;
			}
			if let Err(e) = T::NativeCurrency::transfer(
				&pallet_account,
				&rewards_account,
				lmp_cut,
				ExistenceRequirement::KeepAlive,
			) {
				log::warn!(
					target: "ocex",
					"LMP fee split transfer failed for epoch {:?} pair {:?}: {:?}",
					epoch, pair, e
				);
			}
		}
	}
}
