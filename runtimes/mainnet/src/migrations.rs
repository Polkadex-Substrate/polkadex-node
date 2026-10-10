use crate::Runtime;
use frame_support::{
    // ensure, // unused now that RebuildLmpPoolIdIndex try-runtime is commented out
    traits::{OnRuntimeUpgrade, Get, GetStorageVersion},
    weights::{Weight, constants::RocksDbWeight},
    migrations::RemovePallet,
    parameter_types,
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
///
/// F-030: guarded to run once (upgrading into spec 392) — this iterates every
/// account with a lock, which is wasted work forever after the fix is applied,
/// and re-running it on new, unrelated future state would be incorrect.
pub struct FixBalancesFrozen;

const FIX_BALANCES_FROZEN_FROM_SPEC: u32 = 391;

impl OnRuntimeUpgrade for FixBalancesFrozen {
    fn on_runtime_upgrade() -> Weight {
        if crate::System::last_runtime_upgrade_spec_version() > FIX_BALANCES_FROZEN_FROM_SPEC {
            log::warn!("Skipping FixBalancesFrozen: already applied");
            return <Runtime as frame_system::Config>::DbWeight::get().reads(1);
        }
        let mut iterated: u64 = 0;
        let mut mutated: u64 = 0;
        let mut fixed: u64 = 0;
        for (who, locks) in pallet_balances::Locks::<Runtime>::iter() {
            iterated += 1;
            let max_lock = locks.iter().map(|l| l.amount).max().unwrap_or_default();
            if max_lock == 0 {
                continue;
            }
            // T::AccountStore = frame_system::Pallet<Runtime>, so balance data
            // lives in frame_system::Account (NOT pallet_balances::Account).
            mutated += 1;
            frame_system::Account::<Runtime>::mutate(&who, |info| {
                if max_lock > info.data.frozen {
                    info.data.frozen = max_lock;
                    fixed += 1;
                }
            });
        }
        log::info!("🔧 Fixed frozen field for {} accounts with stale locks", fixed);
        // `iterated` covers the Locks::iter() read for every account visited (including
        // those skipped for having no lock); `mutated` covers the Account read+write
        // that `mutate` performs unconditionally for every account with a nonzero lock,
        // whether or not `fixed` was actually bumped.
        <Runtime as frame_system::Config>::DbWeight::get()
            .reads_writes(iterated + mutated + 1, mutated)
    }
}

// =============================================================================
// Council prime cleanup
// =============================================================================

/// The council prime on-chain is not in the members list (pre-existing state
/// inconsistency from the mainnet fork). Clear it so the invariant holds.
///
/// F-030: guarded to run once (upgrading into spec 392) — harmless to re-run
/// (only fires when the invariant is actually broken), but guarded anyway for
/// consistency with the other one-shot migrations and to avoid an unnecessary
/// storage read on every future upgrade forever.
pub struct FixCouncilPrime;

const FIX_COUNCIL_PRIME_FROM_SPEC: u32 = 391;

impl OnRuntimeUpgrade for FixCouncilPrime {
    fn on_runtime_upgrade() -> Weight {
        if crate::System::last_runtime_upgrade_spec_version() > FIX_COUNCIL_PRIME_FROM_SPEC {
            log::warn!("Skipping FixCouncilPrime: already applied");
            return <Runtime as frame_system::Config>::DbWeight::get().reads(1);
        }
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
///
/// F-030: guarded to run once (upgrading into spec 392). Without this guard,
/// this would wipe legitimate future offence reports on every subsequent
/// runtime upgrade too — not just the historical undecodable entries it was
/// written for.
pub struct ClearOffenceReports;

const CLEAR_OFFENCE_REPORTS_FROM_SPEC: u32 = 391;

impl OnRuntimeUpgrade for ClearOffenceReports {
    fn on_runtime_upgrade() -> Weight {
        if crate::System::last_runtime_upgrade_spec_version() > CLEAR_OFFENCE_REPORTS_FROM_SPEC {
            log::warn!("Skipping ClearOffenceReports: already applied");
            return <Runtime as frame_system::Config>::DbWeight::get().reads(1);
        }
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

/// F-002 — ClearLegacySudoKey
///
/// `pallet_sudo` was removed from `construct_runtime!` years ago, but the
/// `Sudo::Key` storage slot was never wiped and still holds the 2021 genesis
/// root key (confirmed live via RPC). Spec 392 re-adds `pallet_sudo` under the
/// same name with no migration touching that slot — without this fix, that
/// leftover value becomes live Root over mainnet the instant the upgrade
/// enacts, with no `set_key` call needed by anyone.
///
/// Guarded to run once, on the upgrade into spec 392, and never again.
pub struct ClearLegacySudoKey;

const CLEAR_LEGACY_SUDO_KEY_FROM_SPEC: u32 = 391; // Runs once, upgrading into spec 392

impl OnRuntimeUpgrade for ClearLegacySudoKey {
    fn on_runtime_upgrade() -> Weight {
        if crate::System::last_runtime_upgrade_spec_version() > CLEAR_LEGACY_SUDO_KEY_FROM_SPEC {
            log::warn!("Skipping ClearLegacySudoKey: already applied");
            return <Runtime as frame_system::Config>::DbWeight::get().reads(1);
        }

        // Clear everything under the Sudo pallet prefix (Key and the storage
        // version marker), so nothing remains under a pallet name that no longer exists.
        let prefix = sp_io::hashing::twox_128(b"Sudo");
        let result = frame_support::storage::unhashed::clear_prefix(&prefix, None, None);
        log::info!("🔑 Cleared legacy Sudo storage prefix (removed={})", result.backend);
        <Runtime as frame_system::Config>::DbWeight::get().writes(result.backend as u64 + 1)
    }

    #[cfg(feature = "try-runtime")]
    fn pre_upgrade() -> Result<sp_std::vec::Vec<u8>, sp_runtime::TryRuntimeError> {
        if crate::System::last_runtime_upgrade_spec_version() > CLEAR_LEGACY_SUDO_KEY_FROM_SPEC {
            return Ok(sp_std::vec::Vec::new());
        }
        let key = frame_support::storage::storage_prefix(b"Sudo", b"Key");
        let existed = sp_io::storage::exists(&key);
        log::info!("🔍 ClearLegacySudoKey pre_upgrade: Sudo::Key exists = {}", existed);
        Ok(sp_std::vec::Vec::new())
    }

    #[cfg(feature = "try-runtime")]
    fn post_upgrade(_state: sp_std::vec::Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
        use frame_support::ensure;
        let key = frame_support::storage::storage_prefix(b"Sudo", b"Key");
        ensure!(
            !sp_io::storage::exists(&key),
            "ClearLegacySudoKey: Sudo::Key still present after migration"
        );
        Ok(())
    }
}

parameter_types! {
    pub const OrderbookCommitteeStr: &'static str = "OrderbookCommittee";
}

/// F-029 — ClearOrderbookCommittee
///
/// `OrderbookCommittee` (pallet_collective Instance4) governed `OCEX`, which was
/// removed from `construct_runtime!`. The committee pallet stayed active with
/// nothing left to govern — an orphaned permission surface. Wipes all storage
/// under its prefix (members, proposals, votes) using the framework's own
/// `RemovePallet` migration.
pub type ClearOrderbookCommittee = RemovePallet<OrderbookCommitteeStr, RocksDbWeight>;

parameter_types! {
    pub const RandomnessCollectiveFlipStr: &'static str = "RandomnessCollectiveFlip";
}

/// ClearRandomnessCollectiveFlip
///
/// Mainnet still holds one `RandomnessCollectiveFlip::RandomMaterial` key (2,594 bytes) left by a
/// runtime older than 373; no runtime since has had the pallet. Wipes it with the framework's
/// `RemovePallet` migration. Idempotent: a second run finds nothing under the prefix.
pub type ClearRandomnessCollectiveFlip = RemovePallet<RandomnessCollectiveFlipStr, RocksDbWeight>;

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
/// beneficiary.  Without this migration, those 12 accounts (12 VestingSchedules entries —
/// confirmed on-chain; the 13th key cleared below is the pallet's own StorageVersion
/// marker, not an account) can never remove the lock (no `claim()` extrinsic exists after
/// pallet removal), so their vested tokens are permanently frozen.
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
        // F-030: one-shot, gated like the other spec-392 migrations
        if frame_system::Pallet::<T>::last_runtime_upgrade_spec_version() > 391 {
            log::warn!("Skipping ClearOrmlVestingLocks: already applied");
            return T::DbWeight::get().reads(1);
        }
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
                        } else {
                            // Never expected on mainnet (all 12 entries decode), but if it happens the
                            // lock stays in place and this line is the only record of which key it was.
                            log::warn!(
                                target: "runtime::migration",
                                "ClearOrmlVestingLocks: could not decode AccountId from key {:?}; lock left in place",
                                &key[..]
                            );
                        }
                    } else {
                        // Expected exactly once on mainnet: the pallet's own StorageVersion key (32 bytes).
                        log::warn!(
                            target: "runtime::migration",
                            "ClearOrmlVestingLocks: skipping non-schedule key {:?} ({} bytes)",
                            &key[..],
                            key.len()
                        );
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
        use frame_support::ensure;
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

// =============================================================================
// Locks left by the removed Rewards and PDEXMigration pallets
// =============================================================================

use crate::{AccountId, BlockNumber};
use frame_support::traits::{LockIdentifier, LockableCurrency};
use sp_std::collections::btree_set::BTreeSet;

/// Storage of the removed Rewards and PDEXMigration pallets, declared with the exact types of
/// mainnet spec 373 (Polkadex-Substrate/Polkadex @ a29298bb). The migration only reads it.
pub mod legacy_locks {
    use frame_support::{
        storage::types::{OptionQuery, ValueQuery},
        storage_alias, Blake2_128Concat,
    };
    use polkadex_primitives::{AccountId, Balance, BlockNumber};
    use sp_core::{Decode, Encode};

    /// `pallet_rewards::RewardInfo`: one reward cycle.
    // Mirrors the on-chain layout: every field must be present even where nothing reads it.
    #[allow(dead_code)]
    #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
    pub struct RewardInfo {
        pub start_block: BlockNumber,
        pub end_block: BlockNumber,
        pub initial_percentage: u32,
    }

    /// `pallet_rewards::RewardInfoForAccount`: one account's reward under one reward id.
    #[allow(dead_code)]
    #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
    pub struct RewardInfoForAccount {
        pub total_reward_amount: Balance,
        pub claim_amount: Balance,
        pub is_initial_rewards_claimed: bool,
        pub is_initialized: bool,
        pub lock_id: [u8; 8],
        pub last_block_rewards_claim: BlockNumber,
        pub initial_rewards_claimable: Balance,
        pub factor: Balance,
    }

    /// `Rewards::InitializeRewards`: reward id to reward cycle.
    #[storage_alias(verbatim)]
    pub type InitializeRewards =
        StorageMap<Rewards, Blake2_128Concat, u32, RewardInfo, OptionQuery>;

    /// `Rewards::Distributor`: (reward id, account) to the account's reward record.
    #[storage_alias(verbatim)]
    pub type Distributor = StorageDoubleMap<
        Rewards,
        Blake2_128Concat,
        u32,
        Blake2_128Concat,
        AccountId,
        RewardInfoForAccount,
        OptionQuery,
    >;

    /// `PDEXMigration::Operational`: the switch that enables `mint` and `unlock`.
    #[storage_alias(verbatim)]
    pub type Operational = StorageValue<PDEXMigration, bool, ValueQuery>;

    /// `PDEXMigration::LockedTokenHolders`: account to the block of its last migration mint.
    #[storage_alias(verbatim)]
    pub type LockedTokenHolders =
        StorageMap<PDEXMigration, Blake2_128Concat, AccountId, BlockNumber, OptionQuery>;
}

/// Lock id of the removed Rewards pallet (`REWARDS_LOCK_ID` in pallets/rewards).
pub const REWARDS_LOCK_ID: LockIdentifier = *b"REWARDID";
/// Lock id of the removed PDEXMigration pallet (`MIGRATION_LOCK` in pallets/pdex-migration).
pub const PDEX_MIGRATION_LOCK_ID: LockIdentifier = *b"pdexlock";
/// `LockPeriod` of PDEXMigration on mainnet spec 373: 201,600 blocks (28 days).
pub const PDEX_MIGRATION_LOCK_PERIOD: BlockNumber = 201_600;

const CLEAR_REWARDS_AND_MIGRATION_LOCKS_FROM_SPEC: u32 = 391; // Runs once, upgrading into spec 392

/// ClearRewardsAndMigrationLocks
///
/// Rewards and PDEXMigration were removed from construct_runtime without a cleanup migration, so
/// their balance locks stay on mainnet with no call left that can release them: `REWARDID` on
/// 732 accounts (319,287.14 PDEX) and `pdexlock` on 954 accounts (404,434.83 PDEX).
///
/// This removes each lock that the old pallet itself would release at the current block, by the
/// rules of mainnet spec 373 (Polkadex-Substrate/Polkadex @ a29298bb):
///
/// - `REWARDID`: `Rewards::claim(reward_id)` succeeds when `InitializeRewards(reward_id)` exists
///   and `Distributor(reward_id, who)` is initialised, and then removes the whole lock named in
///   that record. There is no block-number condition.
/// - `pdexlock`: `PDEXMigration::unlock()` succeeds when `Operational` is set and
///   `LockedTokenHolders(who) + 201,600 <= now`, and then removes the lock.
///
/// A lock whose rule is not met, or whose account has no record in the old storage, stays.
/// Each lock goes through `LockableCurrency::remove_lock`, which recomputes `frozen` from the
/// remaining locks and freezes. Only accounts that hold the lock are touched; the old pallets'
/// storage is read, never written. At mainnet block 13,204,429 all 1,686 locks meet their rule.
///
/// Guarded to run once, upgrading into spec 392, like the other one-shot migrations here.
pub struct ClearRewardsAndMigrationLocks;

/// The accounts whose lock the old pallet would release at a given block.
pub struct ReleasableLocks {
    /// Accounts whose `REWARDID` lock `Rewards::claim` would remove.
    pub rewards: BTreeSet<AccountId>,
    /// Accounts whose `pdexlock` lock `PDEXMigration::unlock` would remove.
    pub pdex_migration: BTreeSet<AccountId>,
    /// Storage reads spent finding them.
    pub reads: u64,
}

impl ClearRewardsAndMigrationLocks {
    fn has_lock(who: &AccountId, id: LockIdentifier) -> bool {
        pallet_balances::Locks::<Runtime>::get(who).iter().any(|lock| lock.id == id)
    }

    /// Applies the old unlock rules at block `now` to every record in the old storage.
    pub fn releasable(now: BlockNumber) -> ReleasableLocks {
        use legacy_locks::{Distributor, InitializeRewards, LockedTokenHolders, Operational};
        let mut reads: u64 = 0;

        // REWARDID: claim() succeeds for an initialised record under a registered reward id.
        let cycles: BTreeSet<u32> = InitializeRewards::iter_keys().collect();
        reads += cycles.len() as u64 + 1;
        let mut rewards = BTreeSet::new();
        for (reward_id, who, record) in Distributor::iter() {
            reads += 1;
            if record.is_initialized
                && record.lock_id == REWARDS_LOCK_ID
                && cycles.contains(&reward_id)
            {
                reads += 1;
                if Self::has_lock(&who, REWARDS_LOCK_ID) {
                    rewards.insert(who);
                }
            }
        }

        // pdexlock: unlock() succeeds while the pallet is operational, once the account's last
        // mint is at least LockPeriod blocks old.
        let mut pdex_migration = BTreeSet::new();
        reads += 1;
        if Operational::get() {
            for (who, minted_at) in LockedTokenHolders::iter() {
                reads += 1;
                if minted_at.saturating_add(PDEX_MIGRATION_LOCK_PERIOD) <= now {
                    reads += 1;
                    if Self::has_lock(&who, PDEX_MIGRATION_LOCK_ID) {
                        pdex_migration.insert(who);
                    }
                }
            }
        } else {
            log::warn!(
                target: "runtime::migration",
                "ClearRewardsAndMigrationLocks: PDEXMigration::Operational is false, so unlock() \
                 fails for everyone; every pdexlock lock stays"
            );
        }

        ReleasableLocks { rewards, pdex_migration, reads }
    }
}

impl OnRuntimeUpgrade for ClearRewardsAndMigrationLocks {
    fn on_runtime_upgrade() -> Weight {
        let db = <Runtime as frame_system::Config>::DbWeight::get();
        if crate::System::last_runtime_upgrade_spec_version() > CLEAR_REWARDS_AND_MIGRATION_LOCKS_FROM_SPEC {
            log::warn!("Skipping ClearRewardsAndMigrationLocks: already applied");
            return db.reads(1);
        }

        let found = Self::releasable(frame_system::Pallet::<Runtime>::block_number());
        for who in found.rewards.iter() {
            <pallet_balances::Pallet<Runtime> as LockableCurrency<AccountId>>::remove_lock(
                REWARDS_LOCK_ID,
                who,
            );
        }
        for who in found.pdex_migration.iter() {
            <pallet_balances::Pallet<Runtime> as LockableCurrency<AccountId>>::remove_lock(
                PDEX_MIGRATION_LOCK_ID,
                who,
            );
        }
        log::info!(
            target: "runtime::migration",
            "ClearRewardsAndMigrationLocks: removed {} REWARDID locks and {} pdexlock locks",
            found.rewards.len(),
            found.pdex_migration.len(),
        );

        // Each remove_lock reads Locks, Freezes and the account, and writes the account and Locks.
        let removed = (found.rewards.len() + found.pdex_migration.len()) as u64;
        db.reads_writes(1 + found.reads + 3 * removed, 2 * removed)
    }

    #[cfg(feature = "try-runtime")]
    fn pre_upgrade() -> Result<sp_std::vec::Vec<u8>, sp_runtime::TryRuntimeError> {
        if crate::System::last_runtime_upgrade_spec_version() > CLEAR_REWARDS_AND_MIGRATION_LOCKS_FROM_SPEC {
            return Ok(sp_std::vec::Vec::new());
        }
        let found = Self::releasable(frame_system::Pallet::<Runtime>::block_number());
        let scan = try_runtime_checks::LockScan::take();
        let snapshot = try_runtime_checks::LocksSnapshot::new(&found, &scan);
        log::info!(
            target: "runtime::migration",
            "ClearRewardsAndMigrationLocks pre_upgrade: REWARDID {} locks ({} planck), {} to remove; \
             pdexlock {} locks ({} planck), {} to remove",
            scan.rewards.len(),
            snapshot.rewards_total,
            snapshot.rewards_released.len(),
            scan.pdex_migration.len(),
            snapshot.pdex_migration_total,
            snapshot.pdex_migration_released.len(),
        );
        Ok(snapshot.encode())
    }

    #[cfg(feature = "try-runtime")]
    fn post_upgrade(state: sp_std::vec::Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
        use frame_support::ensure;
        use try_runtime_checks::{legacy_key_counts, LockScan, LocksSnapshot};

        if crate::System::last_runtime_upgrade_spec_version() > CLEAR_REWARDS_AND_MIGRATION_LOCKS_FROM_SPEC {
            return Ok(());
        }
        let pre = LocksSnapshot::decode(&mut &state[..])
            .map_err(|_| "ClearRewardsAndMigrationLocks: cannot decode the pre_upgrade state")?;
        let scan = LockScan::take();

        // The REWARDID and pdexlock locks left are exactly the ones whose rule was not met, with
        // their amounts unchanged.
        ensure!(
            scan.rewards == pre.rewards_kept,
            "ClearRewardsAndMigrationLocks: REWARDID locks left differ from the expected set"
        );
        ensure!(
            scan.pdex_migration == pre.pdex_migration_kept,
            "ClearRewardsAndMigrationLocks: pdexlock locks left differ from the expected set"
        );
        // Every lock with another id is unchanged.
        ensure!(
            scan.other_locks == pre.other_locks,
            "ClearRewardsAndMigrationLocks: a lock with another id changed"
        );
        // Accounts that lost a lock: free and reserved unchanged, frozen recomputed from what is
        // left.
        for (who, free, reserved) in pre.balances.iter() {
            let data = frame_system::Account::<Runtime>::get(who).data;
            ensure!(
                data.free == *free && data.reserved == *reserved,
                "ClearRewardsAndMigrationLocks: free or reserved balance changed"
            );
            let locked = pallet_balances::Locks::<Runtime>::get(who)
                .iter()
                .map(|lock| lock.amount)
                .max()
                .unwrap_or_default();
            let frozen = pallet_balances::Freezes::<Runtime>::get(who)
                .iter()
                .map(|freeze| freeze.amount)
                .max()
                .unwrap_or_default();
            ensure!(
                data.frozen == locked.max(frozen),
                "ClearRewardsAndMigrationLocks: frozen does not match the remaining locks"
            );
        }
        // The old pallets' storage and the total issuance are untouched.
        ensure!(
            legacy_key_counts() == pre.legacy_keys,
            "ClearRewardsAndMigrationLocks: Rewards or PDEXMigration storage changed"
        );
        ensure!(
            pallet_balances::TotalIssuance::<Runtime>::get() == pre.total_issuance,
            "ClearRewardsAndMigrationLocks: total issuance changed"
        );

        let kept = |locks: &[(AccountId, crate::Balance)]| locks.iter().map(|l| l.1).sum::<crate::Balance>();
        log::info!(
            target: "runtime::migration",
            "ClearRewardsAndMigrationLocks post_upgrade: removed {} REWARDID locks ({} planck) and \
             {} pdexlock locks ({} planck); {} REWARDID and {} pdexlock locks stay",
            pre.rewards_released.len(),
            pre.rewards_total - kept(&pre.rewards_kept),
            pre.pdex_migration_released.len(),
            pre.pdex_migration_total - kept(&pre.pdex_migration_kept),
            pre.rewards_kept.len(),
            pre.pdex_migration_kept.len(),
        );
        Ok(())
    }
}

/// State kept between pre_upgrade and post_upgrade of ClearRewardsAndMigrationLocks.
#[cfg(feature = "try-runtime")]
mod try_runtime_checks {
    use super::{ReleasableLocks, PDEX_MIGRATION_LOCK_ID, REWARDS_LOCK_ID};
    use crate::{AccountId, Balance, Runtime};
    use sp_core::{Decode, Encode};
    use sp_std::vec::Vec;

    /// One pass over Balances::Locks: every REWARDID and pdexlock lock, and a hash of all others.
    pub struct LockScan {
        pub rewards: Vec<(AccountId, Balance)>,
        pub pdex_migration: Vec<(AccountId, Balance)>,
        pub other_locks: [u8; 32],
    }

    impl LockScan {
        pub fn take() -> Self {
            let mut rewards: Vec<(AccountId, Balance)> = Vec::new();
            let mut pdex_migration: Vec<(AccountId, Balance)> = Vec::new();
            let mut others: Vec<u8> = Vec::new();
            for (who, locks) in pallet_balances::Locks::<Runtime>::iter() {
                let mut rest = Vec::new();
                for lock in locks.iter() {
                    if lock.id == REWARDS_LOCK_ID {
                        rewards.push((who.clone(), lock.amount));
                    } else if lock.id == PDEX_MIGRATION_LOCK_ID {
                        pdex_migration.push((who.clone(), lock.amount));
                    } else {
                        rest.push(lock.clone());
                    }
                }
                if !rest.is_empty() {
                    (who, rest).encode_to(&mut others);
                }
            }
            LockScan { rewards, pdex_migration, other_locks: sp_io::hashing::blake2_256(&others) }
        }
    }

    #[derive(Encode, Decode)]
    pub struct LocksSnapshot {
        pub rewards_released: Vec<AccountId>,
        pub pdex_migration_released: Vec<AccountId>,
        /// The locks expected to stay, in Balances::Locks order, with their amounts.
        pub rewards_kept: Vec<(AccountId, Balance)>,
        pub pdex_migration_kept: Vec<(AccountId, Balance)>,
        pub rewards_total: Balance,
        pub pdex_migration_total: Balance,
        pub other_locks: [u8; 32],
        /// (account, free, reserved) of every account that loses a lock.
        pub balances: Vec<(AccountId, Balance, Balance)>,
        /// Keys under the Rewards and PDEXMigration prefixes.
        pub legacy_keys: (u32, u32),
        pub total_issuance: Balance,
    }

    impl LocksSnapshot {
        pub fn new(found: &ReleasableLocks, scan: &LockScan) -> Self {
            let kept = |locks: &[(AccountId, Balance)], released: &sp_std::collections::btree_set::BTreeSet<AccountId>| {
                locks.iter().filter(|(who, _)| !released.contains(who)).cloned().collect::<Vec<_>>()
            };
            let total = |locks: &[(AccountId, Balance)]| locks.iter().map(|l| l.1).sum::<Balance>();
            let balances = found
                .rewards
                .union(&found.pdex_migration)
                .map(|who| {
                    let data = frame_system::Account::<Runtime>::get(who).data;
                    (who.clone(), data.free, data.reserved)
                })
                .collect();
            LocksSnapshot {
                rewards_released: found.rewards.iter().cloned().collect(),
                pdex_migration_released: found.pdex_migration.iter().cloned().collect(),
                rewards_kept: kept(&scan.rewards, &found.rewards),
                pdex_migration_kept: kept(&scan.pdex_migration, &found.pdex_migration),
                rewards_total: total(&scan.rewards),
                pdex_migration_total: total(&scan.pdex_migration),
                other_locks: scan.other_locks,
                balances,
                legacy_keys: legacy_key_counts(),
                total_issuance: pallet_balances::TotalIssuance::<Runtime>::get(),
            }
        }
    }

    fn count_keys(prefix: &[u8]) -> u32 {
        let mut count = 0u32;
        let mut key = prefix.to_vec();
        while let Some(next) = sp_io::storage::next_key(&key) {
            if !next.starts_with(prefix) {
                break;
            }
            count += 1;
            key = next;
        }
        count
    }

    /// Number of keys under the Rewards and under the PDEXMigration prefix.
    pub fn legacy_key_counts() -> (u32, u32) {
        (
            count_keys(&sp_io::hashing::twox_128(b"Rewards")),
            count_keys(&sp_io::hashing::twox_128(b"PDEXMigration")),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::legacy_locks::{
        Distributor, InitializeRewards, LockedTokenHolders, Operational, RewardInfo,
        RewardInfoForAccount,
    };
    use super::*;
    use crate::{Balance, Balances};
    use frame_support::traits::{Currency, WithdrawReasons};
    use sp_runtime::BuildStorage;

    const PDEX: Balance = crate::constants::currency::PDEX;
    /// The mainnet block the lock classification was taken at.
    const NOW: BlockNumber = 13_204_429;
    const STAKING_LOCK_ID: LockIdentifier = *b"staking ";

    fn new_test_ext() -> sp_io::TestExternalities {
        let mut ext: sp_io::TestExternalities = frame_system::GenesisConfig::<Runtime>::default()
            .build_storage()
            .unwrap()
            .into();
        ext.execute_with(|| frame_system::Pallet::<Runtime>::set_block_number(NOW));
        ext
    }

    fn account(n: u8) -> AccountId {
        AccountId::from([n; 32])
    }

    fn fund(who: &AccountId) {
        let _ = <Balances as Currency<AccountId>>::make_free_balance_be(who, 10_000 * PDEX);
    }

    fn lock(who: &AccountId, id: LockIdentifier, amount: Balance) {
        if frame_system::Account::<Runtime>::get(who).data.free == 0 {
            fund(who);
        }
        <Balances as LockableCurrency<AccountId>>::set_lock(id, who, amount, WithdrawReasons::TRANSFER);
    }

    fn lock_of(who: &AccountId, id: LockIdentifier) -> Option<Balance> {
        pallet_balances::Locks::<Runtime>::get(who).iter().find(|l| l.id == id).map(|l| l.amount)
    }

    fn frozen(who: &AccountId) -> Balance {
        frame_system::Account::<Runtime>::get(who).data.frozen
    }

    fn record(is_initialized: bool) -> RewardInfoForAccount {
        RewardInfoForAccount {
            total_reward_amount: 600 * PDEX,
            claim_amount: 100 * PDEX,
            is_initial_rewards_claimed: true,
            is_initialized,
            lock_id: REWARDS_LOCK_ID,
            last_block_rewards_claim: 6_271_048,
            initial_rewards_claimable: 150 * PDEX,
            factor: 6_370_340,
        }
    }

    /// Seeds the old storage and the locks the way mainnet holds them, with one account per case.
    ///  1: REWARDID, initialised record under the registered cycle            -> released
    ///  2: REWARDID, record not initialised                                   -> kept (not met)
    ///  3: REWARDID, no record                                                -> kept (no data)
    ///  4: REWARDID, initialised record under an unregistered reward id       -> kept (not met)
    ///  5: pdexlock, lock period ends exactly at NOW                          -> released
    ///  6: pdexlock, lock period ends one block after NOW                     -> kept (not met)
    ///  7: pdexlock, no record                                                -> kept (no data)
    ///  8: REWARDID and pdexlock, both released; a staking lock stays
    ///  9: initialised Rewards record but no lock (already claimed)           -> untouched
    /// 10: PDEXMigration record but no lock (already unlocked)                -> untouched
    fn seed() {
        InitializeRewards::insert(
            1,
            RewardInfo { start_block: 1_815_527, end_block: 6_653_927, initial_percentage: 25 },
        );
        Operational::put(true);

        lock(&account(1), REWARDS_LOCK_ID, 500 * PDEX);
        Distributor::insert(1, account(1), record(true));
        lock(&account(2), REWARDS_LOCK_ID, 500 * PDEX);
        Distributor::insert(1, account(2), record(false));
        lock(&account(3), REWARDS_LOCK_ID, 500 * PDEX);
        lock(&account(4), REWARDS_LOCK_ID, 500 * PDEX);
        Distributor::insert(2, account(4), record(true));

        lock(&account(5), PDEX_MIGRATION_LOCK_ID, 400 * PDEX);
        LockedTokenHolders::insert(account(5), NOW - PDEX_MIGRATION_LOCK_PERIOD);
        lock(&account(6), PDEX_MIGRATION_LOCK_ID, 400 * PDEX);
        LockedTokenHolders::insert(account(6), NOW - PDEX_MIGRATION_LOCK_PERIOD + 1);
        lock(&account(7), PDEX_MIGRATION_LOCK_ID, 400 * PDEX);

        lock(&account(8), REWARDS_LOCK_ID, 500 * PDEX);
        lock(&account(8), PDEX_MIGRATION_LOCK_ID, 300 * PDEX);
        lock(&account(8), STAKING_LOCK_ID, 200 * PDEX);
        Distributor::insert(1, account(8), record(true));
        LockedTokenHolders::insert(account(8), 624_538);

        fund(&account(9));
        Distributor::insert(1, account(9), record(true));
        fund(&account(10));
        LockedTokenHolders::insert(account(10), 624_538);
    }

    #[test]
    fn removes_only_the_locks_whose_old_unlock_rule_is_met() {
        new_test_ext().execute_with(|| {
            seed();
            let issuance = pallet_balances::TotalIssuance::<Runtime>::get();
            let account_9 = frame_system::Account::<Runtime>::get(account(9));
            let account_10 = frame_system::Account::<Runtime>::get(account(10));

            let found = ClearRewardsAndMigrationLocks::releasable(NOW);
            assert_eq!(found.rewards.into_iter().collect::<Vec<_>>(), vec![account(1), account(8)]);
            assert_eq!(
                found.pdex_migration.into_iter().collect::<Vec<_>>(),
                vec![account(5), account(8)]
            );

            let _ = ClearRewardsAndMigrationLocks::on_runtime_upgrade();

            // Released, and frozen recomputed from what is left.
            assert_eq!(lock_of(&account(1), REWARDS_LOCK_ID), None);
            assert_eq!(frozen(&account(1)), 0);
            assert_eq!(lock_of(&account(5), PDEX_MIGRATION_LOCK_ID), None);
            assert_eq!(frozen(&account(5)), 0);
            assert_eq!(lock_of(&account(8), REWARDS_LOCK_ID), None);
            assert_eq!(lock_of(&account(8), PDEX_MIGRATION_LOCK_ID), None);
            assert_eq!(lock_of(&account(8), STAKING_LOCK_ID), Some(200 * PDEX));
            assert_eq!(frozen(&account(8)), 200 * PDEX);

            // Kept: rule not met, or no record.
            for n in [2, 3, 4] {
                assert_eq!(lock_of(&account(n), REWARDS_LOCK_ID), Some(500 * PDEX));
                assert_eq!(frozen(&account(n)), 500 * PDEX);
            }
            for n in [6, 7] {
                assert_eq!(lock_of(&account(n), PDEX_MIGRATION_LOCK_ID), Some(400 * PDEX));
                assert_eq!(frozen(&account(n)), 400 * PDEX);
            }

            // Nothing else touched: balances, accounts without a lock, the old storage, issuance.
            for n in 1..=8 {
                let data = frame_system::Account::<Runtime>::get(account(n)).data;
                assert_eq!((data.free, data.reserved), (10_000 * PDEX, 0));
            }
            assert_eq!(frame_system::Account::<Runtime>::get(account(9)), account_9);
            assert_eq!(frame_system::Account::<Runtime>::get(account(10)), account_10);
            assert!(pallet_balances::Locks::<Runtime>::get(account(9)).is_empty());
            assert!(pallet_balances::Locks::<Runtime>::get(account(10)).is_empty());
            assert_eq!(Distributor::get(1, account(1)), Some(record(true)));
            assert_eq!(Distributor::iter().count(), 5);
            assert_eq!(LockedTokenHolders::get(account(5)), Some(NOW - PDEX_MIGRATION_LOCK_PERIOD));
            assert_eq!(LockedTokenHolders::iter().count(), 4);
            assert_eq!(pallet_balances::TotalIssuance::<Runtime>::get(), issuance);
        });
    }

    #[test]
    fn a_second_run_changes_nothing_and_the_guard_skips_after_392() {
        new_test_ext().execute_with(|| {
            seed();
            let _ = ClearRewardsAndMigrationLocks::on_runtime_upgrade();
            let root = sp_io::storage::root(sp_core::storage::StateVersion::V1);

            // Nothing is left to release, so running the migration again writes nothing.
            let found = ClearRewardsAndMigrationLocks::releasable(NOW);
            assert!(found.rewards.is_empty() && found.pdex_migration.is_empty());
            let _ = ClearRewardsAndMigrationLocks::on_runtime_upgrade();
            assert_eq!(sp_io::storage::root(sp_core::storage::StateVersion::V1), root);

            // Once the chain has upgraded into 392 the guard skips it, even for a lock whose
            // rule is met.
            frame_system::LastRuntimeUpgrade::<Runtime>::put(
                frame_system::LastRuntimeUpgradeInfo::from(crate::VERSION),
            );
            lock(&account(11), REWARDS_LOCK_ID, 500 * PDEX);
            Distributor::insert(1, account(11), record(true));
            let _ = ClearRewardsAndMigrationLocks::on_runtime_upgrade();
            assert_eq!(lock_of(&account(11), REWARDS_LOCK_ID), Some(500 * PDEX));
        });
    }

    #[test]
    fn pdexlock_stays_when_the_migration_pallet_is_not_operational() {
        new_test_ext().execute_with(|| {
            seed();
            Operational::put(false);
            let _ = ClearRewardsAndMigrationLocks::on_runtime_upgrade();
            assert_eq!(lock_of(&account(5), PDEX_MIGRATION_LOCK_ID), Some(400 * PDEX));
            assert_eq!(lock_of(&account(8), PDEX_MIGRATION_LOCK_ID), Some(300 * PDEX));
            // The REWARDID rule does not depend on it.
            assert_eq!(lock_of(&account(1), REWARDS_LOCK_ID), None);
            assert_eq!(lock_of(&account(8), REWARDS_LOCK_ID), None);
        });
    }

    /// Values read from mainnet at block 13,204,429 (spec 373) must decode through the aliases
    /// at the keys mainnet uses. The values are the raw mainnet bytes; the account in the keys is
    /// a test account.
    #[test]
    fn aliases_read_the_mainnet_layout() {
        fn unhex(s: &str) -> Vec<u8> {
            (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
        }
        new_test_ext().execute_with(|| {
            use sp_io::hashing::blake2_128;
            let who = account(7);
            let concat = |key: &[u8]| [&blake2_128(key)[..], key].concat();

            // twox128("Rewards") ++ twox128("Distributor") ++ (reward id 1) ++ (account)
            let distributor = [
                unhex("540a4f8754aa5298a3d6e9aa09e93f97ceb4876b50655e052e4e5e04aee50f9b"),
                concat(&1u32.encode()),
                concat(&who.encode()),
            ]
            .concat();
            assert_eq!(Distributor::hashed_key_for(1u32, &who), distributor);
            sp_io::storage::set(
                &distributor,
                &unhex(
                    "00208e7b602500000000000000000000a4b977982823000000000000000000000101524557415244\
                     494448b05f000088e31e58090000000000000000000024346100000000000000000000000000",
                ),
            );
            assert_eq!(
                Distributor::get(1u32, &who),
                Some(RewardInfoForAccount {
                    total_reward_amount: 41_096_320_000_000,
                    claim_amount: 38_657_263_647_140,
                    is_initial_rewards_claimed: true,
                    is_initialized: true,
                    lock_id: *b"REWARDID",
                    last_block_rewards_claim: 6_271_048,
                    initial_rewards_claimable: 10_274_080_000_000,
                    factor: 6_370_340,
                })
            );
            assert_eq!(
                Distributor::iter().map(|(id, acc, _)| (id, acc)).collect::<Vec<_>>(),
                vec![(1, who.clone())]
            );

            // twox128("Rewards") ++ twox128("InitializeRewards") ++ (reward id 1)
            let cycle = [
                unhex("540a4f8754aa5298a3d6e9aa09e93f977b3950f8fd6c2b89975e04f39790a2b9"),
                concat(&1u32.encode()),
            ]
            .concat();
            assert_eq!(InitializeRewards::hashed_key_for(1u32), cycle);
            sp_io::storage::set(&cycle, &unhex("e7b31b00e787650019000000"));
            assert_eq!(
                InitializeRewards::get(1u32),
                Some(RewardInfo { start_block: 1_815_527, end_block: 6_653_927, initial_percentage: 25 })
            );

            // twox128("PDEXMigration") ++ twox128("LockedTokenHolders") ++ (account)
            let holder = [
                unhex("4ef636f65fd8673a753a8276ac217285440e110669db20db8ae66c7379c53a7c"),
                concat(&who.encode()),
            ]
            .concat();
            assert_eq!(LockedTokenHolders::hashed_key_for(&who), holder);
            sp_io::storage::set(&holder, &unhex("9a870900"));
            assert_eq!(LockedTokenHolders::get(&who), Some(624_538));

            // twox128("PDEXMigration") ++ twox128("Operational")
            let operational = unhex("4ef636f65fd8673a753a8276ac21728529ddc5444fb1a19e6bb7daf22ffb6a95");
            assert_eq!(Operational::hashed_key().to_vec(), operational);
            assert!(!Operational::get());
            sp_io::storage::set(&operational, &unhex("01"));
            assert!(Operational::get());
        });
    }
}
