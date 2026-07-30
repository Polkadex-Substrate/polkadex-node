use crate::Runtime;
use frame_support::{
    traits::{OnRuntimeUpgrade, Get, GetStorageVersion},
    weights::Weight
};
use sp_std::marker::PhantomData;
use polkadex_primitives::auction::FeeDistribution;
use sp_runtime::{BoundToRuntimeAppPublic, KeyTypeId, RuntimeAppPublic};
use sp_core::{Encode, Decode};
use sp_runtime::traits::AccountIdConversion;

// Type alias for the old Thea pallet that was removed
type OldTheaPublic = thea::ecdsa::AuthorityId;
pub struct InitOcexFeeConfig<T>(PhantomData<T>);
impl<T: pallet_ocex_lmp::Config> OnRuntimeUpgrade for InitOcexFeeConfig<T> {
    fn on_runtime_upgrade() -> Weight {
        use pallet_ocex_lmp::FeeDistributionConfig;
        
        if FeeDistributionConfig::<T>::get().is_none() {
            let default_config = FeeDistribution {
                burn_ration: 50u8, // 50% burn
                recipient_address: T::TreasuryPalletId::get().into_account_truncating(),
                auction_duration: 100u32.into(), // 100 blocks
            };
            FeeDistributionConfig::<T>::put(default_config);
            log::info!("✅ Initialized OCEX FeeDistributionConfig");
        }
        
        T::DbWeight::get().reads_writes(1, 1)
    }
}

/// Old session keys structure (with thea, without mixnet and beefy)
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct OldSessionKeys {
    pub grandpa: <crate::Grandpa as BoundToRuntimeAppPublic>::Public,
    pub babe: <crate::Babe as BoundToRuntimeAppPublic>::Public,
    pub im_online: <crate::ImOnline as BoundToRuntimeAppPublic>::Public,
    pub authority_discovery: <crate::AuthorityDiscovery as BoundToRuntimeAppPublic>::Public,
    pub orderbook: <crate::OCEX as BoundToRuntimeAppPublic>::Public,
    pub thea: OldTheaPublic,
}

impl sp_runtime::traits::OpaqueKeys for OldSessionKeys {
    type KeyTypeIdProviders = ();

    fn key_ids() -> &'static [KeyTypeId] {
        &[
            <<crate::Grandpa as BoundToRuntimeAppPublic>::Public>::ID,
            <<crate::Babe as BoundToRuntimeAppPublic>::Public>::ID,
            <<crate::ImOnline as BoundToRuntimeAppPublic>::Public>::ID,
            <<crate::AuthorityDiscovery as BoundToRuntimeAppPublic>::Public>::ID,
            <<crate::OCEX as BoundToRuntimeAppPublic>::Public>::ID,
            <OldTheaPublic as RuntimeAppPublic>::ID,
        ]
    }

    fn get_raw(&self, key_type: KeyTypeId) -> &[u8] {
        match key_type {
            <<crate::Grandpa as BoundToRuntimeAppPublic>::Public>::ID => self.grandpa.as_ref(),
            <<crate::Babe as BoundToRuntimeAppPublic>::Public>::ID => self.babe.as_ref(),
            <<crate::ImOnline as BoundToRuntimeAppPublic>::Public>::ID => self.im_online.as_ref(),
            <<crate::AuthorityDiscovery as BoundToRuntimeAppPublic>::Public>::ID => self.authority_discovery.as_ref(),
            <<crate::OCEX as BoundToRuntimeAppPublic>::Public>::ID => self.orderbook.as_ref(),
            <OldTheaPublic as RuntimeAppPublic>::ID => self.thea.as_ref(),
            _ => &[],
        }
    }
}

/// Transform old session keys to new session keys (add mixnet and beefy)
/// Following Substrate best practices: initialize new keys to dummy values
/// Validators must call author_rotateKeys and session.setKeys post-upgrade
fn transform_session_keys(_account: crate::AccountId, old_keys: OldSessionKeys) -> crate::SessionKeys {
    use sp_core::crypto::UncheckedFrom;
    use sp_mixnet::types::AuthorityId as MixnetId;
    use sp_consensus_beefy::ecdsa_crypto::AuthorityId as BeefyId;

    // Use dummy keys as recommended by Substrate documentation
    // "initialize the keys to a (unique) dummy value with the expectation
    // that all validators should invoke set_keys before those keys are actually required"
    let dummy_beefy_key = BeefyId::unchecked_from([0u8; 33]);
    let dummy_mixnet_key = MixnetId::unchecked_from([0u8; 32]);

    // For production: validators will generate real keys via author_rotateKeys
    let (beefy_key, mixnet_key) = (dummy_beefy_key, dummy_mixnet_key);

    crate::SessionKeys {
        grandpa: old_keys.grandpa,
        babe: old_keys.babe,
        im_online: old_keys.im_online,
        authority_discovery: old_keys.authority_discovery,
        orderbook: old_keys.orderbook,
        mixnet: mixnet_key,
        beefy: beefy_key,
    }
}

/// Migration to add mixnet and beefy to session keys
pub struct UpgradeSessionKeys;

const UPGRADE_SESSION_KEYS_FROM_SPEC: u32 = 378; // Migration runs when upgrading from spec 378 to 379

impl OnRuntimeUpgrade for UpgradeSessionKeys {
    fn on_runtime_upgrade() -> Weight {
        if crate::System::last_runtime_upgrade_spec_version() > UPGRADE_SESSION_KEYS_FROM_SPEC {
            log::info!("Skipping session keys upgrade: already applied");
            return <Runtime as frame_system::Config>::DbWeight::get().reads(1);
        }

        log::info!("🔧 Starting session keys migration - adding mixnet and beefy");

        // Upgrade the session keys using the transformation function
        pallet_session::Pallet::<Runtime>::upgrade_keys::<OldSessionKeys, _>(transform_session_keys);

        log::info!("✅ Session keys migration completed");

        // Return appropriate weight for the migration
        <Runtime as frame_system::Config>::DbWeight::get().reads_writes(100, 100)
    }

    #[cfg(feature = "try-runtime")]
    fn pre_upgrade() -> Result<sp_std::vec::Vec<u8>, sp_runtime::TryRuntimeError> {
        use frame_support::ensure;
        use sp_runtime::traits::OpaqueKeys;
        use sp_std::vec::Vec;

        if crate::System::last_runtime_upgrade_spec_version() > UPGRADE_SESSION_KEYS_FROM_SPEC {
            log::warn!("Skipping session keys migration pre-upgrade check: already applied");
            return Ok(Vec::new());
        }

        log::info!("🔍 Pre-upgrade check for session keys migration");

        // Verify new keys contain mixnet and beefy
        use sp_mixnet::types::AuthorityId as MixnetId;
        use sp_consensus_beefy::ecdsa_crypto::AuthorityId as BeefyId;

        let new_key_ids = crate::SessionKeys::key_ids();
        ensure!(
            new_key_ids.iter().find(|&k| *k == <MixnetId as RuntimeAppPublic>::ID).is_some(),
            "New session keys should contain Mixnet key"
        );
        ensure!(
            new_key_ids.iter().find(|&k| *k == <BeefyId as RuntimeAppPublic>::ID).is_some(),
            "New session keys should contain Beefy key"
        );

        // Get current queued keys count
        let queued_keys = pallet_session::QueuedKeys::<Runtime>::get();
        log::info!("Found {} queued keys before migration", queued_keys.len());

        Ok((queued_keys.len() as u32).encode())
    }

    #[cfg(feature = "try-runtime")]
    fn post_upgrade(state: sp_std::vec::Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
        use frame_support::ensure;
        use sp_runtime::traits::OpaqueKeys;

        if crate::System::last_runtime_upgrade_spec_version() > UPGRADE_SESSION_KEYS_FROM_SPEC {
            log::warn!("Skipping session keys migration post-upgrade check: already applied");
            return Ok(());
        }

        let pre_count: u32 = Decode::decode(&mut &state[..])
            .map_err(|_| "Failed to decode pre-upgrade state")?;

        let post_keys = pallet_session::QueuedKeys::<Runtime>::get();
        let post_count = post_keys.len() as u32;

        log::info!("🔍 Post-upgrade check: {} keys before, {} keys after", pre_count, post_count);

        // Ensure we have the same number of keys after migration
        ensure!(pre_count == post_count, "Key count mismatch after migration");

        // Verify new keys have mixnet and beefy
        use sp_mixnet::types::AuthorityId as MixnetId;
        use sp_consensus_beefy::ecdsa_crypto::AuthorityId as BeefyId;

        for (account_id, keys) in post_keys.iter() {
            let mixnet_raw = keys.get_raw(<MixnetId as RuntimeAppPublic>::ID);
            let beefy_raw = keys.get_raw(<BeefyId as RuntimeAppPublic>::ID);

            ensure!(!mixnet_raw.is_empty(), "Mixnet key missing after migration");
            ensure!(!beefy_raw.is_empty(), "Beefy key missing after migration");

            log::debug!("✓ Account {:?} has mixnet and beefy keys", account_id);
        }

        log::info!("✅ Post-upgrade verification passed");
        Ok(())
    }
}

// =============================================================================
// Pallet Storage Version Migrations
// =============================================================================

/// Generic storage version bump for any pallet. Bumps on-chain version to
/// match in-code version if behind; no-op if already current.
pub struct StorageVersionMigration<P>(PhantomData<P>);
impl<P> OnRuntimeUpgrade for StorageVersionMigration<P>
where
    P: GetStorageVersion<InCodeStorageVersion = frame_support::traits::StorageVersion>
        + frame_support::traits::PalletInfoAccess,
{
    fn on_runtime_upgrade() -> Weight {
        let current = P::on_chain_storage_version();
        let target = P::in_code_storage_version();
        if current < target {
            log::info!("🔧 Updating {} storage version from {:?} to {:?}", P::name(), current, target);
            target.put::<P>();
            <Runtime as frame_system::Config>::DbWeight::get().reads_writes(1, 1)
        } else {
            <Runtime as frame_system::Config>::DbWeight::get().reads(1)
        }
    }
}

/// Migration for pallet-staking storage version update
pub struct StakingStorageVersionMigration<T>(PhantomData<T>);
impl<T: pallet_staking::Config> OnRuntimeUpgrade for StakingStorageVersionMigration<T> {
    fn on_runtime_upgrade() -> Weight {
        let current = pallet_staking::Pallet::<T>::on_chain_storage_version();
        let target = pallet_staking::Pallet::<T>::in_code_storage_version();

        if current < target {
            log::info!("🔧 Updating Staking pallet storage version from {:?} to {:?}", current, target);
            target.put::<pallet_staking::Pallet<T>>();
            T::DbWeight::get().reads_writes(1, 1)
        } else {
            T::DbWeight::get().reads(1)
        }
    }
}

/// Migration for pallet-session v0 -> v1
pub struct SessionStorageVersionMigration<T>(PhantomData<T>);
impl<T: pallet_session::Config> OnRuntimeUpgrade for SessionStorageVersionMigration<T> {
    fn on_runtime_upgrade() -> Weight {
        let current = pallet_session::Pallet::<T>::on_chain_storage_version();
        let target = pallet_session::Pallet::<T>::in_code_storage_version();

        if current < target {
            log::info!("🔧 Updating Session pallet storage version from {:?} to {:?}", current, target);
            target.put::<pallet_session::Pallet<T>>();
            T::DbWeight::get().reads_writes(1, 1)
        } else {
            T::DbWeight::get().reads(1)
        }
    }
}

/// Migration for pallet-grandpa v4 -> v5
pub struct GrandpaStorageVersionMigration<T>(PhantomData<T>);
impl<T: pallet_grandpa::Config> OnRuntimeUpgrade for GrandpaStorageVersionMigration<T> {
    fn on_runtime_upgrade() -> Weight {
        let current = pallet_grandpa::Pallet::<T>::on_chain_storage_version();
        let target = pallet_grandpa::Pallet::<T>::in_code_storage_version();

        if current < target {
            log::info!("🔧 Updating GRANDPA pallet storage version from {:?} to {:?}", current, target);
            target.put::<pallet_grandpa::Pallet<T>>();
            T::DbWeight::get().reads_writes(1, 1)
        } else {
            T::DbWeight::get().reads(1)
        }
    }
}

/// Migration for pallet-identity v1 -> v2
pub struct IdentityStorageVersionMigration<T>(PhantomData<T>);
impl<T: pallet_identity::Config> OnRuntimeUpgrade for IdentityStorageVersionMigration<T> {
    fn on_runtime_upgrade() -> Weight {
        let current = pallet_identity::Pallet::<T>::on_chain_storage_version();
        let target = pallet_identity::Pallet::<T>::in_code_storage_version();

        if current < target {
            log::info!("🔧 Updating Identity pallet storage version from {:?} to {:?}", current, target);
            target.put::<pallet_identity::Pallet<T>>();
            T::DbWeight::get().reads_writes(1, 1)
        } else {
            T::DbWeight::get().reads(1)
        }
    }
}

/// Migration for pallet-child-bounties v0 -> v1
pub struct ChildBountiesStorageVersionMigration<T>(PhantomData<T>);
impl<T: pallet_child_bounties::Config> OnRuntimeUpgrade for ChildBountiesStorageVersionMigration<T> {
    fn on_runtime_upgrade() -> Weight {
        let current = pallet_child_bounties::Pallet::<T>::on_chain_storage_version();
        let target = pallet_child_bounties::Pallet::<T>::in_code_storage_version();

        if current < target {
            log::info!("🔧 Updating ChildBounties pallet storage version from {:?} to {:?}", current, target);
            target.put::<pallet_child_bounties::Pallet<T>>();
            T::DbWeight::get().reads_writes(1, 1)
        } else {
            T::DbWeight::get().reads(1)
        }
    }
}

// =============================================================================
// Offences Storage Cleanup
// =============================================================================

// =============================================================================
// Balances frozen field repair
// =============================================================================

/// Old on-chain account data (pallet_balances v0) stored `misc_frozen` and
/// `fee_frozen` as separate fields. The new v1 format stores a single `frozen`
/// that must be `>= max(all_locks)`. Accounts whose old `misc_frozen` was zero
/// but had a lock (e.g. fee-only reasons) are found with `frozen = 0` by the
/// new try_state check. This migration corrects the `frozen` field to be at
/// least the maximum of all existing locks.
pub struct FixBalancesFrozen;
impl OnRuntimeUpgrade for FixBalancesFrozen {
    fn on_runtime_upgrade() -> Weight {
        let mut fixed: u64 = 0;
        for (who, locks) in pallet_balances::Locks::<Runtime>::iter() {
            let max_lock = locks.iter().map(|l| l.amount).max().unwrap_or_default();
            if max_lock == 0 {
                continue;
            }
            // T::AccountStore = frame_system::Pallet<Runtime>, so balance data
            // lives in frame_system::Account (NOT pallet_balances::Account).
            frame_system::Account::<Runtime>::mutate(&who, |info| {
                if max_lock > info.data.frozen {
                    info.data.frozen = max_lock;
                    fixed += 1;
                }
            });
        }
        log::info!("🔧 Fixed frozen field for {} accounts with stale locks", fixed);
        <Runtime as frame_system::Config>::DbWeight::get()
            .reads_writes(fixed * 2 + 1, fixed)
    }
}

// =============================================================================
// Council prime cleanup
// =============================================================================

/// The council prime on-chain is not in the members list (pre-existing state
/// inconsistency from the mainnet fork). Clear it so the invariant holds.
pub struct FixCouncilPrime;
impl OnRuntimeUpgrade for FixCouncilPrime {
    fn on_runtime_upgrade() -> Weight {
        use pallet_collective::Instance1 as CouncilCollective;
        if let Some(prime) = pallet_collective::Prime::<Runtime, CouncilCollective>::get() {
            let members = pallet_collective::Members::<Runtime, CouncilCollective>::get();
            if !members.contains(&prime) {
                pallet_collective::Prime::<Runtime, CouncilCollective>::kill();
                log::info!("🔧 Cleared Council prime (not a member)");
                return <Runtime as frame_system::Config>::DbWeight::get().reads_writes(2, 1);
            }
        }
        <Runtime as frame_system::Config>::DbWeight::get().reads(2)
    }
}

/// Clear all Offences::Reports entries. These encode `IdentificationTuple`
/// (from `pallet_session::historical`) which changed between spec versions,
/// making existing entries undecodable with the new runtime types.
/// Old offence records are stale processed slash data — safe to clear.
/// Uses raw prefix clearing because entries can't be decoded with new types.
pub struct ClearOffenceReports;
impl OnRuntimeUpgrade for ClearOffenceReports {
    fn on_runtime_upgrade() -> Weight {
        let result = frame_support::storage::migration::clear_storage_prefix(
            b"Offences",
            b"Reports",
            b"",
            None,
            None,
        );
        log::info!(
            "🧹 Cleared {} Offences::Reports entries (maybe_cursor={})",
            result.backend,
            result.maybe_cursor.is_some()
        );
        <Runtime as frame_system::Config>::DbWeight::get().writes(result.backend as u64 + 1)
    }
}
