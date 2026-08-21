use crate::Runtime;
use frame_support::{
    // ensure, // unused now that RebuildLmpPoolIdIndex try-runtime is commented out
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
#[allow(dead_code)]
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

/// Old session keys structure (with thea + orderbook, without mixnet and beefy).
/// Uses raw key types so this compiles even when pallet_ocex_lmp is not in construct_runtime.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct OldSessionKeys {
    pub grandpa: <crate::Grandpa as BoundToRuntimeAppPublic>::Public,
    pub babe: <crate::Babe as BoundToRuntimeAppPublic>::Public,
    pub im_online: <crate::ImOnline as BoundToRuntimeAppPublic>::Public,
    pub authority_discovery: <crate::AuthorityDiscovery as BoundToRuntimeAppPublic>::Public,
    /// Raw orderbook key type — does not require pallet_ocex_lmp::Config to be implemented.
    pub orderbook: pallet_ocex_lmp::sr25519::AuthorityId,
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
            <pallet_ocex_lmp::sr25519::AuthorityId as RuntimeAppPublic>::ID,
            <OldTheaPublic as RuntimeAppPublic>::ID,
        ]
    }

    fn get_raw(&self, key_type: KeyTypeId) -> &[u8] {
        match key_type {
            <<crate::Grandpa as BoundToRuntimeAppPublic>::Public>::ID => self.grandpa.as_ref(),
            <<crate::Babe as BoundToRuntimeAppPublic>::Public>::ID => self.babe.as_ref(),
            <<crate::ImOnline as BoundToRuntimeAppPublic>::Public>::ID => self.im_online.as_ref(),
            <<crate::AuthorityDiscovery as BoundToRuntimeAppPublic>::Public>::ID => self.authority_discovery.as_ref(),
            <pallet_ocex_lmp::sr25519::AuthorityId as RuntimeAppPublic>::ID => self.orderbook.as_ref(),
            <OldTheaPublic as RuntimeAppPublic>::ID => self.thea.as_ref(),
            _ => &[],
        }
    }
}

/// Transform old session keys (6 keys: grandpa, babe, im_online, auth_discovery, orderbook, thea)
/// to new session keys (6 keys: grandpa, babe, im_online, auth_discovery, mixnet, beefy).
/// orderbook and thea are dropped; mixnet and beefy are initialised to dummy values.
/// Validators must call author_rotateKeys + session.setKeys post-upgrade.
fn transform_session_keys(_account: crate::AccountId, old_keys: OldSessionKeys) -> crate::SessionKeys {
    use sp_core::crypto::UncheckedFrom;
    use sp_mixnet::types::AuthorityId as MixnetId;
    use sp_consensus_beefy::ecdsa_crypto::AuthorityId as BeefyId;

    let dummy_beefy_key = BeefyId::unchecked_from([0u8; 33]);
    let dummy_mixnet_key = MixnetId::unchecked_from([0u8; 32]);

    crate::SessionKeys {
        grandpa: old_keys.grandpa,
        babe: old_keys.babe,
        im_online: old_keys.im_online,
        authority_discovery: old_keys.authority_discovery,
        // orderbook dropped — pallet_ocex_lmp removed from construct_runtime
        mixnet: dummy_mixnet_key,
        beefy: dummy_beefy_key,
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

/// C6 Migration — RebuildLmpPoolIdIndex
///
/// Adds a reverse index `pool_id → (market, market_maker)` to the LMP pallet
/// so that OCEX egress callbacks can resolve the correct `Pools` key.
///
/// Before spec 391 the index did not exist.  For any pools created before this
/// upgrade the callbacks would have failed with `UnknownPool` (or been silently
/// swallowed via the `()` no-op wiring).  On mainnet, zero pools existed at
/// upgrade time (confirmed via RPC), so the migration is a zero-write no-op in
/// practice.  It is included anyway so the index is populated if any pools were
/// created on a fork or testnet.
#[allow(dead_code)]
pub struct RebuildLmpPoolIdIndex;

// impl OnRuntimeUpgrade for RebuildLmpPoolIdIndex {
//     // CrowdSourceLMP (pallet_lmp) removed from construct_runtime — impl commented out.
//     // Re-enable together with the pallet when CrowdSourceLMP is re-added.
//     fn on_runtime_upgrade() -> Weight {
//         use pallet_lmp::pallet::{PoolIdIndex, Pools};
//
//         let db = <Runtime as frame_system::Config>::DbWeight::get();
//         let mut reads: u64 = 0;
//         let mut writes: u64 = 0;
//
//         for (market, market_maker, config) in Pools::<Runtime>::iter() {
//             reads += 1;
//             PoolIdIndex::<Runtime>::insert(&config.pool_id, (market, market_maker));
//             writes += 1;
//         }
//
//         log::info!(
//             target: "runtime::migration",
//             "🏊 RebuildLmpPoolIdIndex: populated {} pool_id → (market, market_maker) entries",
//             writes,
//         );
//
//         db.reads_writes(reads, writes)
//     }
//
//     #[cfg(feature = "try-runtime")]
//     fn pre_upgrade() -> Result<sp_std::vec::Vec<u8>, sp_runtime::TryRuntimeError> {
//         use pallet_lmp::pallet::Pools;
//         use parity_scale_codec::Encode;
//         let count = Pools::<Runtime>::iter().count() as u64;
//         log::info!(
//             target: "runtime::migration",
//             "RebuildLmpPoolIdIndex pre_upgrade: {} pools found",
//             count
//         );
//         Ok(count.encode())
//     }
//
//     #[cfg(feature = "try-runtime")]
//     fn post_upgrade(state: sp_std::vec::Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
//         use pallet_lmp::pallet::{PoolIdIndex, Pools};
//         use parity_scale_codec::Decode;
//         let pool_count = u64::decode(&mut &state[..]).unwrap_or(0);
//         let index_count = PoolIdIndex::<Runtime>::iter().count() as u64;
//         ensure!(
//             index_count == pool_count,
//             "RebuildLmpPoolIdIndex: index count does not match pool count"
//         );
//         log::info!(
//             target: "runtime::migration",
//             "RebuildLmpPoolIdIndex post_upgrade: {} index entries for {} pools ✅",
//             index_count, pool_count
//         );
//         Ok(())
//     }
// }
impl OnRuntimeUpgrade for RebuildLmpPoolIdIndex {
    fn on_runtime_upgrade() -> Weight {
        // CrowdSourceLMP (pallet_lmp) removed from construct_runtime.
        // This migration is a no-op until the pallet is re-enabled.
        Weight::zero()
    }
}

/// C9 Migration — PruneStaleIngressMessages
///
/// `IngressMessages` was never pruned before spec 391.  Every block since
/// genesis has accumulated an entry even after the enclave processed and
/// discarded the corresponding messages.  This migration removes all entries
/// for blocks up to and including `last_processed_blk` from the most recent
/// accepted snapshot.
///
/// # Safety
///
/// We iterate only over *keys* (no value decode), so the Vec→BoundedVec type
/// change introduced in spec 391 cannot cause a decode failure here.
/// Any remaining entries (blocks not yet processed) have their values left
/// untouched on disk; the new runtime decodes them as BoundedVec, which
/// succeeds because each real block accumulates far fewer than OBIngressLimit
/// (500) messages under normal operation and block-weight constraints.
#[allow(dead_code)]
pub struct PruneStaleIngressMessages;

// impl OnRuntimeUpgrade for PruneStaleIngressMessages {
//     // OCEX (pallet_ocex_lmp) removed from construct_runtime — impl commented out.
//     // Re-enable together with the pallet when OCEX is re-added.
//     fn on_runtime_upgrade() -> Weight {
//         use pallet_ocex_lmp::{IngressMessages, SnapshotNonce, Snapshots};
//         use sp_runtime::SaturatedConversion;
//
//         let db = <Runtime as frame_system::Config>::DbWeight::get();
//         let mut reads: u64 = 0;
//         let mut writes: u64 = 0;
//
//         let nonce = SnapshotNonce::<Runtime>::get();
//         reads += 1;
//
//         if nonce == 0 {
//             log::info!(target: "runtime::migration", "PruneStaleIngressMessages: no snapshot yet, nothing to prune");
//             return db.reads(reads);
//         }
//
//         let last_processed: polkadex_primitives::BlockNumber = match Snapshots::<Runtime>::get(nonce) {
//             Some(snapshot) => { reads += 1; snapshot.last_processed_blk },
//             None => {
//                 reads += 1;
//                 log::warn!(target: "runtime::migration", "PruneStaleIngressMessages: snapshot {} not found, skipping", nonce);
//                 return db.reads(reads);
//             }
//         };
//
//         let stale_keys: sp_std::vec::Vec<frame_system::pallet_prelude::BlockNumberFor<Runtime>> =
//             IngressMessages::<Runtime>::iter_keys()
//                 .filter(|k| { let block: polkadex_primitives::BlockNumber = (*k).saturated_into(); block <= last_processed })
//                 .collect();
//
//         let removed = stale_keys.len();
//         reads += removed as u64;
//         writes += removed as u64;
//
//         for key in stale_keys {
//             IngressMessages::<Runtime>::remove(key);
//         }
//
//         log::info!(
//             target: "runtime::migration",
//             "🧹 PruneStaleIngressMessages: removed {} stale IngressMessages entries (blocks ≤ {})",
//             removed, last_processed,
//         );
//
//         db.reads_writes(reads, writes)
//     }
//
//     #[cfg(feature = "try-runtime")]
//     fn pre_upgrade() -> Result<sp_std::vec::Vec<u8>, sp_runtime::TryRuntimeError> {
//         use pallet_ocex_lmp::{IngressMessages, SnapshotNonce};
//         use parity_scale_codec::Encode;
//         let total = IngressMessages::<Runtime>::iter_keys().count() as u64;
//         let nonce = SnapshotNonce::<Runtime>::get();
//         log::info!(target: "runtime::migration", "PruneStaleIngressMessages pre_upgrade: {} IngressMessages entries, snapshot_nonce = {}", total, nonce);
//         Ok(total.encode())
//     }
//
//     #[cfg(feature = "try-runtime")]
//     fn post_upgrade(state: sp_std::vec::Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
//         use pallet_ocex_lmp::IngressMessages;
//         use parity_scale_codec::Decode;
//         let before = u64::decode(&mut &state[..]).unwrap_or(0);
//         let after = IngressMessages::<Runtime>::iter_keys().count() as u64;
//         log::info!(target: "runtime::migration", "PruneStaleIngressMessages post_upgrade: {} → {} IngressMessages entries ({} removed)", before, after, before.saturating_sub(after));
//         Ok(())
//     }
// }
impl OnRuntimeUpgrade for PruneStaleIngressMessages {
    fn on_runtime_upgrade() -> Weight {
        // OCEX (pallet_ocex_lmp) removed from construct_runtime.
        // This migration is a no-op until the pallet is re-enabled.
        Weight::zero()
    }
}

/// Clear orphaned OrmlVesting storage and remove the "ormlvest" currency lock from
/// every affected account.
///
/// Background: OrmlVesting was removed from construct_runtime without a cleanup migration.
/// The pallet applied `Currency::set_lock(*b"ormlvest", account, amount, ...)` to each
/// beneficiary.  Without this migration, those 13 accounts can never remove the lock
/// (no `claim()` extrinsic exists after pallet removal), so their vested tokens are
/// permanently frozen.
///
/// Key layout for OrmlVesting::VestingSchedules (StorageMap<Blake2_128Concat, AccountId, …>):
///   [0..16]  twox128("OrmlVesting")       = d84892f1db5f9dfd80c521d0a5647650
///   [16..32] twox128("VestingSchedules")  = 9c806850c4ee3bc06ba62b096318fe38
///   [32..48] blake2_128(account_id)       (transparent hash prefix)
///   [48..80] account_id raw bytes         (32 bytes, AccountId32)
pub struct ClearOrmlVestingLocks<T>(PhantomData<T>);

impl<T> OnRuntimeUpgrade for ClearOrmlVestingLocks<T>
where
    T: pallet_balances::Config + frame_system::Config,
    T::AccountId: Decode,
{
    fn on_runtime_upgrade() -> Weight {
        use frame_support::traits::LockableCurrency;

        // twox128("OrmlVesting") = d84892f1db5f9dfd80c521d0a5647650
        const PALLET_PREFIX: [u8; 16] = [
            0xd8, 0x48, 0x92, 0xf1, 0xdb, 0x5f, 0x9d, 0xfd,
            0x80, 0xc5, 0x21, 0xd0, 0xa5, 0x64, 0x76, 0x50,
        ];
        // twox128("OrmlVesting") ++ twox128("VestingSchedules")
        // twox128("VestingSchedules") = 9c806850c4ee3bc06ba62b096318fe38
        const SCHEDULES_PREFIX: [u8; 32] = [
            0xd8, 0x48, 0x92, 0xf1, 0xdb, 0x5f, 0x9d, 0xfd, 0x80, 0xc5, 0x21, 0xd0, 0xa5, 0x64, 0x76, 0x50,
            0x9c, 0x80, 0x68, 0x50, 0xc4, 0xee, 0x3b, 0xc0, 0x6b, 0xa6, 0x2b, 0x09, 0x63, 0x18, 0xfe, 0x38,
        ];
        // Lock identifier used by orml-vesting: b"ormlvest"
        const VESTING_LOCK_ID: frame_support::traits::LockIdentifier = *b"ormlvest";

        let mut accounts_cleared: u32 = 0;
        let mut reads: u64 = 0;
        let mut writes: u64 = 0;

        // Iterate VestingSchedules keys to discover affected accounts.
        // We only iterate keys (no value decode) so this is safe even if the
        // VestingScheduleOf type is no longer available in the runtime.
        let mut next_key = SCHEDULES_PREFIX.to_vec();
        loop {
            reads += 1;
            match sp_io::storage::next_key(&next_key) {
                Some(key) if key.starts_with(&SCHEDULES_PREFIX) => {
                    // Blake2_128Concat: 32 bytes prefix + 16 bytes hash + 32 bytes raw key
                    if key.len() >= 80 {
                        let account_raw = &key[48..80];
                        if let Ok(account) = T::AccountId::decode(&mut &account_raw[..]) {
                            <pallet_balances::Pallet<T> as LockableCurrency<T::AccountId>>::remove_lock(
                                VESTING_LOCK_ID,
                                &account,
                            );
                            writes += 1;
                            accounts_cleared += 1;
                            log::info!(
                                target: "runtime::migration",
                                "ClearOrmlVestingLocks: removed ormlvest lock for account {:?}",
                                account
                            );
                        }
                    }
                    next_key = key;
                }
                _ => break,
            }
        }

        // Wipe the entire OrmlVesting storage prefix (VestingSchedules + StorageVersion if any).
        let loops = match sp_io::storage::clear_prefix(&PALLET_PREFIX, None) {
            sp_io::KillStorageResult::AllRemoved(n) => n,
            sp_io::KillStorageResult::SomeRemaining(n) => n,
        };
        writes += loops as u64;

        log::info!(
            target: "runtime::migration",
            "ClearOrmlVestingLocks: unlocked {} accounts, ran {} clear_prefix iterations",
            accounts_cleared,
            loops,
        );

        T::DbWeight::get().reads_writes(reads, writes)
    }

    #[cfg(feature = "try-runtime")]
    fn pre_upgrade() -> Result<sp_std::vec::Vec<u8>, sp_runtime::TryRuntimeError> {
        // twox128("OrmlVesting") = d84892f1db5f9dfd80c521d0a5647650
        const PALLET_PREFIX: [u8; 16] = [
            0xd8, 0x48, 0x92, 0xf1, 0xdb, 0x5f, 0x9d, 0xfd,
            0x80, 0xc5, 0x21, 0xd0, 0xa5, 0x64, 0x76, 0x50,
        ];

        let mut count: u32 = 0;
        let mut next_key = PALLET_PREFIX.to_vec();
        loop {
            match sp_io::storage::next_key(&next_key) {
                Some(key) if key.starts_with(&PALLET_PREFIX) => {
                    count += 1;
                    next_key = key;
                }
                _ => break,
            }
        }
        log::info!(
            target: "runtime::migration",
            "ClearOrmlVestingLocks pre_upgrade: {} OrmlVesting storage keys found",
            count
        );
        Ok(count.encode())
    }

    #[cfg(feature = "try-runtime")]
    fn post_upgrade(state: sp_std::vec::Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
        // twox128("OrmlVesting") = d84892f1db5f9dfd80c521d0a5647650
        const PALLET_PREFIX: [u8; 16] = [
            0xd8, 0x48, 0x92, 0xf1, 0xdb, 0x5f, 0x9d, 0xfd,
            0x80, 0xc5, 0x21, 0xd0, 0xa5, 0x64, 0x76, 0x50,
        ];

        // Verify no OrmlVesting keys remain.
        let still_present = sp_io::storage::next_key(&PALLET_PREFIX)
            .map(|k| k.starts_with(&PALLET_PREFIX))
            .unwrap_or(false);
        ensure!(
            !still_present,
            "ClearOrmlVestingLocks: OrmlVesting storage was not fully cleared!"
        );
        let pre_count = u32::decode(&mut &state[..]).unwrap_or(0);
        log::info!(
            target: "runtime::migration",
            "ClearOrmlVestingLocks post_upgrade: {} keys cleared, 0 remaining ✅",
            pre_count
        );
        Ok(())
    }
}

