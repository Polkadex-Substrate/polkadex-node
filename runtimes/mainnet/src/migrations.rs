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
// use hex; // Not available in runtime

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

/// Migration to re-key `pallet_assets` storage maps from u32 AssetId to u128 AssetId.
///
/// All storage maps that use AssetId as a Blake2_128Concat key need re-keying because
/// the encoded key bytes change from 4 bytes (u32) to 16 bytes (u128).
///
/// Storage layout with Blake2_128Concat:
///   key = twox_128(pallet) ++ twox_128(storage) ++ blake2_128(encoded_id) ++ encoded_id
///
/// Old key lengths (u32 = 4 bytes):
///   Asset/Metadata/Reserves : 32 + 16 + 4       = 52  bytes
///   Account (double map)    : 32 + 16 + 4 + 48  = 100 bytes  (48 = blake2_128+AccountId)
///   Approvals (N-map)       : 32 + 16 + 4 + 96  = 148 bytes  (96 = 2x (blake2_128+AccountId))
///
/// New key lengths (u128 = 16 bytes, +12 each):  64 / 112 / 160 bytes
pub struct AssetsStorageMigration;

const ASSETS_MIGRATION_FROM_SPEC: u32 = 380;

impl OnRuntimeUpgrade for AssetsStorageMigration {
    fn on_runtime_upgrade() -> Weight {
        if crate::System::last_runtime_upgrade_spec_version() != ASSETS_MIGRATION_FROM_SPEC {
            log::info!("Skipping AssetsStorageMigration: not upgrading from spec {}", ASSETS_MIGRATION_FROM_SPEC);
            return <crate::Runtime as frame_system::Config>::DbWeight::get().reads(1);
        }

        let mut total_reads = 1u64;
        let mut total_writes = 0u64;

        for pallet_name in &[b"Assets" as &[u8], b"PoolAssets"] {
            // Simple maps: old key = 52 bytes (32 prefix + 16 hash + 4 u32)
            for storage_name in &[b"Asset" as &[u8], b"Metadata", b"Reserves"] {
                let (r, w) = rekey_assets_map(pallet_name, storage_name, 52);
                total_reads += r;
                total_writes += w;
            }
            // Account double map: old key = 100 bytes
            let (r, w) = rekey_assets_map(pallet_name, b"Account", 100);
            total_reads += r;
            total_writes += w;

            // Approvals N-map: old key = 148 bytes
            let (r, w) = rekey_assets_map(pallet_name, b"Approvals", 148);
            total_reads += r;
            total_writes += w;
        }

        log::info!(
            "🔧 AssetsStorageMigration complete: {} reads, {} writes",
            total_reads, total_writes
        );
        <crate::Runtime as frame_system::Config>::DbWeight::get()
            .reads_writes(total_reads, total_writes)
    }

    #[cfg(feature = "try-runtime")]
    fn pre_upgrade() -> Result<sp_std::vec::Vec<u8>, sp_runtime::TryRuntimeError> {
        use sp_io::hashing::twox_128;

        let mut total = 0u32;
        for pallet_name in &[b"Assets" as &[u8], b"PoolAssets"] {
            for storage_name in &[b"Asset" as &[u8], b"Metadata", b"Reserves", b"Account", b"Approvals"] {
                let prefix: sp_std::vec::Vec<u8> = [
                    twox_128(pallet_name).as_ref(),
                    twox_128(storage_name).as_ref(),
                ].concat();
                let mut key = prefix.clone();
                loop {
                    match sp_io::storage::next_key(&key) {
                        Some(next) if next.starts_with(&prefix) => { total += 1; key = next; }
                        _ => break,
                    }
                }
            }
        }
        log::info!("🔍 AssetsStorageMigration pre-upgrade: {} total entries", total);
        Ok(total.encode())
    }

    #[cfg(feature = "try-runtime")]
    fn post_upgrade(state: sp_std::vec::Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
        use frame_support::ensure;
        use sp_io::hashing::twox_128;

        let pre_count: u32 = Decode::decode(&mut &state[..])
            .map_err(|_| "Failed to decode pre-upgrade state")?;

        let mut post_count = 0u32;
        for pallet_name in &[b"Assets" as &[u8], b"PoolAssets"] {
            for storage_name in &[b"Asset" as &[u8], b"Metadata", b"Reserves", b"Account", b"Approvals"] {
                let prefix: sp_std::vec::Vec<u8> = [
                    twox_128(pallet_name).as_ref(),
                    twox_128(storage_name).as_ref(),
                ].concat();
                let mut key = prefix.clone();
                loop {
                    match sp_io::storage::next_key(&key) {
                        Some(next) if next.starts_with(&prefix) => { post_count += 1; key = next; }
                        _ => break,
                    }
                }
            }
        }

        ensure!(
            pre_count == post_count,
            "AssetsStorageMigration: entry count changed after migration"
        );
        log::info!("✅ AssetsStorageMigration post-upgrade: {} entries all re-keyed", post_count);
        Ok(())
    }
}

/// Re-keys a single pallet_assets storage map from u32 to u128 AssetId.
///
/// Identifies old keys by their expected `old_key_len`, extracts the u32 from
/// bytes [48..52], and writes the value at the new u128 key position.
fn rekey_assets_map(pallet_name: &[u8], storage_name: &[u8], old_key_len: usize) -> (u64, u64) {
    use sp_io::hashing::{twox_128, blake2_128};
    use sp_runtime::codec::Encode;

    let prefix: sp_std::vec::Vec<u8> = [
        twox_128(pallet_name).as_ref(),
        twox_128(storage_name).as_ref(),
    ].concat();

    // AssetId sits at [48..52] in old keys (32 prefix + 16 blake2_128 hash)
    const ASSET_ID_START: usize = 48;
    const OLD_ASSET_ID_END: usize = 52; // 48 + 4 (u32)

    let mut reads = 0u64;
    let mut writes = 0u64;

    // Collect old-format keys first to avoid iterator invalidation
    let mut old_keys: sp_std::vec::Vec<sp_std::vec::Vec<u8>> = sp_std::vec::Vec::new();
    let mut cursor = prefix.clone();
    loop {
        reads += 1;
        match sp_io::storage::next_key(&cursor) {
            Some(next) if next.starts_with(&prefix) => {
                if next.len() == old_key_len {
                    old_keys.push(next.clone());
                }
                cursor = next;
            }
            _ => break,
        }
    }

    for old_key in old_keys {
        reads += 1;
        if let Some(value) = sp_io::storage::get(&old_key) {
            let u32_bytes: [u8; 4] = match old_key[ASSET_ID_START..OLD_ASSET_ID_END].try_into() {
                Ok(b) => b,
                Err(_) => continue,
            };
            let new_id: u128 = u32::from_le_bytes(u32_bytes) as u128;
            let new_id_encoded = new_id.encode();
            let new_hash = blake2_128(&new_id_encoded);

            // Everything after the old u32 AssetId stays the same (AccountId parts for double/N maps)
            let rest = &old_key[OLD_ASSET_ID_END..];

            let mut new_key = prefix.clone();
            new_key.extend_from_slice(&new_hash);
            new_key.extend_from_slice(&new_id_encoded);
            new_key.extend_from_slice(rest);

            sp_io::storage::set(&new_key, &value);
            sp_io::storage::clear(&old_key);
            writes += 2;
        }
    }

    (reads, writes)
}

/// Migration to convert `TokenGateway::LocalAssets` values from `u32` to `u128`.
///
/// This is required because `pallet_assets::Config<Instance1>::AssetId` changed from `u32` to
/// `u128` in the new runtime, while the on-chain entries were stored as 4-byte SCALE-LE u32.
pub struct TokenGatewayLocalAssetsMigration;

impl OnRuntimeUpgrade for TokenGatewayLocalAssetsMigration {
    fn on_runtime_upgrade() -> Weight {
        use sp_io::hashing::twox_128;
        use sp_runtime::codec::Encode;

        let prefix: sp_std::vec::Vec<u8> = [twox_128(b"TokenGateway").as_ref(), twox_128(b"LocalAssets").as_ref()].concat();

        let mut count = 0u32;
        let mut key = prefix.clone();

        loop {
            match sp_io::storage::next_key(&key) {
                Some(next) if next.starts_with(&prefix) => {
                    if let Some(raw_val) = sp_io::storage::get(&next) {
                        if raw_val.len() == 4 {
                            // Old encoding: u32 little-endian (SCALE)
                            let old = u32::from_le_bytes(raw_val[..4].try_into().unwrap_or([0u8; 4]));
                            let new: u128 = old as u128;
                            sp_io::storage::set(&next, &new.encode());
                            count += 1;
                        }
                    }
                    key = next;
                }
                _ => break,
            }
        }

        log::info!("🔧 Migrated {} TokenGateway::LocalAssets entries (u32 → u128)", count);

        <crate::Runtime as frame_system::Config>::DbWeight::get().reads_writes(count as u64 + 1, count as u64)
    }

    #[cfg(feature = "try-runtime")]
    fn pre_upgrade() -> Result<sp_std::vec::Vec<u8>, sp_runtime::TryRuntimeError> {
        use sp_io::hashing::twox_128;
        use sp_runtime::codec::Encode;

        let prefix: sp_std::vec::Vec<u8> = [twox_128(b"TokenGateway").as_ref(), twox_128(b"LocalAssets").as_ref()].concat();
        let mut count = 0u32;
        let mut key = prefix.clone();
        loop {
            match sp_io::storage::next_key(&key) {
                Some(next) if next.starts_with(&prefix) => { count += 1; key = next; }
                _ => break,
            }
        }
        log::info!("🔍 TokenGateway::LocalAssets pre-upgrade: {} entries", count);
        Ok(count.encode())
    }

    #[cfg(feature = "try-runtime")]
    fn post_upgrade(state: sp_std::vec::Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
        use frame_support::ensure;
        use sp_io::hashing::twox_128;

        let pre_count: u32 = Decode::decode(&mut &state[..])
            .map_err(|_| "Failed to decode pre-upgrade state")?;

        let prefix: sp_std::vec::Vec<u8> = [twox_128(b"TokenGateway").as_ref(), twox_128(b"LocalAssets").as_ref()].concat();
        let mut post_count = 0u32;
        let mut key = prefix.clone();
        loop {
            match sp_io::storage::next_key(&key) {
                Some(next) if next.starts_with(&prefix) => {
                    if let Some(raw_val) = sp_io::storage::get(&next) {
                        ensure!(raw_val.len() == 16, "TokenGateway::LocalAssets value is not u128 after migration");
                    }
                    post_count += 1;
                    key = next;
                }
                _ => break,
            }
        }

        ensure!(pre_count == post_count, "TokenGateway::LocalAssets entry count changed after migration");
        log::info!("✅ TokenGateway::LocalAssets post-upgrade: {} entries all u128", post_count);
        Ok(())
    }
}
