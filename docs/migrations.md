# Runtime Migrations — Status & Mainnet Readiness

**Current testnet spec: 390**
**File:** `runtimes/mainnet/src/migrations.rs`
**Wired in:** `runtimes/mainnet/src/lib.rs` — `type Migrations = (...)`

---

## Summary

| Migration | Testnet status | Mainnet required | Safe to remove after first run |
|---|---|---|---|
| `InitOcexFeeConfig` | ✅ Applied | ✅ Yes | Yes |
| `UpgradeSessionKeys` | ✅ Applied (spec 379) | ✅ Yes (if mainnet < spec 379) | Yes, after first run |
| `StakingStorageVersionMigration` | ✅ Applied | ✅ Yes | Yes |
| `SessionStorageVersionMigration` | ✅ Applied | ✅ Yes | Yes |
| `GrandpaStorageVersionMigration` | ✅ Applied | ✅ Yes | Yes |
| `IdentityStorageVersionMigration` | ✅ Applied | ✅ Yes | Yes |
| `ChildBountiesStorageVersionMigration` | ✅ Applied | ✅ Yes | Yes |
| `StorageVersionMigration<*>` × 12 | ✅ Applied | ✅ Yes | Yes |
| `FixBalancesFrozen` | ✅ Applied | ✅ Yes | Yes |
| `FixCouncilPrime` | ✅ Applied | ✅ Yes | Yes |
| `ClearOffenceReports` | ✅ Applied | ✅ Yes | Yes |
| `TestnetOcexStateReset` | ✅ Applied (spec 387) | ❌ **Testnet only** | **Must remove before mainnet** |
| `ResetTestnetAssetSupply` | ✅ Applied (spec 389) | ❌ **Testnet only** | **Must remove before mainnet** |
| `pallet_alliance::migration::Migration` | ✅ Applied | ✅ Yes | Yes |
| `pallet_contracts::Migration` | ✅ Applied | ✅ Yes | Yes |
| `pallet_identity::migration::versioned::V0ToV1` | ✅ Applied | ✅ Yes | Yes |

---

## Detail per migration

### ✅ Keep for mainnet — run once, then remove

---

#### `InitOcexFeeConfig`
- **What it does:** Sets `OCEX::FeeDistributionConfig` to defaults (50% burn, 100-block auctions) if not already set.
- **Guard:** Checks `is_none()` before writing — safe to run on any chain, no-op if already configured.
- **Mainnet:** Must run on first upgrade to initialise the fee config.
- **After first run:** Remove from the tuple.

---

#### `UpgradeSessionKeys`
- **What it does:** Migrates session keys from the old format (grandpa + babe + im_online + authority_discovery + orderbook + **thea**) to the new format (same minus thea, plus **mixnet** + **beefy**). Validators are initialised with dummy mixnet/beefy keys and must call `author_rotateKeys` + `session_setKeys` post-upgrade.
- **Guard:** `last_runtime_upgrade_spec_version() <= 378`. No-op on any chain already past spec 378.
- **Mainnet:** Required if mainnet is upgrading from a spec ≤ 378 runtime. Check mainnet's current spec version before the upgrade.
- **After first run:** Remove from the tuple.

---

#### Storage version migrations (×17)

These all follow the same pattern: compare on-chain storage version with in-code version, bump if behind, no-op if current.

| Migration | Pallet |
|---|---|
| `StakingStorageVersionMigration` | `pallet_staking` |
| `SessionStorageVersionMigration` | `pallet_session` |
| `GrandpaStorageVersionMigration` | `pallet_grandpa` |
| `IdentityStorageVersionMigration` | `pallet_identity` |
| `ChildBountiesStorageVersionMigration` | `pallet_child_bounties` |
| `StorageVersionMigration<pallet_balances>` | Balances |
| `StorageVersionMigration<pallet_election_provider_multi_phase>` | EPM |
| `StorageVersionMigration<pallet_collective::Instance1>` | Council |
| `StorageVersionMigration<pallet_collective::Instance2>` | Technical Committee |
| `StorageVersionMigration<pallet_im_online>` | ImOnline |
| `StorageVersionMigration<pallet_offences>` | Offences |
| `StorageVersionMigration<pallet_session::historical>` | Session Historical |
| `StorageVersionMigration<pallet_scheduler>` | Scheduler |
| `StorageVersionMigration<pallet_multisig>` | Multisig |
| `StorageVersionMigration<pallet_bounties>` | Bounties |
| `StorageVersionMigration<pallet_democracy>` | Democracy |
| `StorageVersionMigration<pallet_preimage>` | Preimage |
| `StorageVersionMigration<pallet_assets::Instance1>` | Assets |

- **Mainnet:** All required to sync on-chain storage version markers with the upgraded in-code versions. Without these, pallets may refuse to run their own internal migrations.
- **After first run:** Remove from the tuple.

---

#### `FixBalancesFrozen`
- **What it does:** Iterates `pallet_balances::Locks` and ensures every account's `frozen` field is at least `max(all_locks)`. Fixes accounts that had `misc_frozen = 0` under the old v0 layout but held locks.
- **Guard:** None explicit — but writing only when `max_lock > info.data.frozen`, so it is harmless on repeated runs.
- **Mainnet:** Required. Mainnet may have accounts in the same state (old v0 balance layout before pallet-balances v1 landed).
- **After first run:** Remove from the tuple.

---

#### `FixCouncilPrime`
- **What it does:** Clears `pallet_collective::Prime` for the Council if the stored prime is not in the members list. Pre-existing state inconsistency carried over from the mainnet fork.
- **Guard:** Reads members list, only kills prime if it's not a member.
- **Mainnet:** Required — this inconsistency originated from the mainnet state itself.
- **After first run:** Remove from the tuple.

---

#### `ClearOffenceReports`
- **What it does:** Raw-prefix-clears all `Offences::Reports` entries. These entries encode `IdentificationTuple` (from `pallet_session::historical`) whose type changed between spec versions — existing entries cannot be decoded with the new runtime types.
- **Guard:** None — raw clear. Second run is a no-op (nothing left to clear).
- **Mainnet:** Required — mainnet carries the same pre-upgrade offence reports.
- **After first run:** Remove from the tuple.

---

#### `pallet_alliance::migration::Migration<Runtime>`
- **What it does:** Standard Alliance pallet upstream migration. Idempotent — checks storage version internally.
- **Mainnet:** Required.
- **After first run:** Remove from the tuple.

---

#### `pallet_contracts::Migration<Runtime>`
- **What it does:** Standard Contracts pallet upstream migration chain. Idempotent — driven by storage version.
- **Mainnet:** Required.
- **After first run:** Remove from the tuple.

---

#### `pallet_identity::migration::versioned::V0ToV1<Runtime, IDENTITY_MIGRATION_KEY_LIMIT>`
- **What it does:** Migrates Identity pallet storage from v0 to v1 layout. Processes up to `IDENTITY_MIGRATION_KEY_LIMIT` entries.
- **Mainnet:** Required — mainnet identity data is in v0 format.
- **After first run:** Remove from the tuple.

---

## ❌ Testnet-only — must remove before mainnet upgrade

---

### `TestnetOcexStateReset`
- **Introduced:** spec 387
- **What it does:** Wipes all OCEX on-chain state (Snapshots, IngressMessages, Accounts, Proxies, TotalAssets, Withdrawals, OnChainEvents, PriceOracle, LMP state, SnapshotNonce, LMPEpoch) and all `Assets::Account` + `Assets::Approvals` entries via raw `clear_storage_prefix`.
- **Why testnet-only:** This was needed because the testnet accumulated stale OCEX state under the wrong decimal configuration (pre-spec 386). Mainnet never had this state — all mainnet assets were correctly configured at 12dp from the start.
- **Mainnet behaviour if left in:** The code comment claims it is a no-op on mainnet, but this relies on mainnet having no entries in those storage maps — which is not guaranteed. Wiping `Assets::Account` on mainnet would burn every account's token balance. **Catastrophic if triggered.**
- **Action: Remove before building the mainnet runtime.**

---

### `ResetTestnetAssetSupply`
- **Introduced:** spec 389
- **What it does:** For asset IDs 3–10, resets `supply`, `accounts`, and `approvals` counters in `Assets::Asset` to zero if the asset has zero actual holders on-chain. Fixes phantom counters left by `TestnetOcexStateReset` (which cleared `Assets::Account` without going through pallet-assets burn flow, so counters were never decremented).
- **Why testnet-only:** The root cause (phantom counters from the spec 387 wipe) cannot exist on mainnet since `TestnetOcexStateReset` never ran there. The guard (`actual_holders == 0`) would be false for every mainnet asset with real holders, making it a functional no-op. However, leaving dead testnet-specific logic in the mainnet runtime is bad practice.
- **Mainnet behaviour if left in:** No-op (guard protects it) — but still, remove.
- **Action: Remove before building the mainnet runtime.**

---

## Pending migrations (not yet written — needed before mainnet upgrade)

These items have been identified as necessary but are not yet in `migrations.rs`:

### `deliver_failed = true` in tesseract config
- Not a runtime migration — an operational change on the relayer server (`74.50.85.34:/root/hyperbridge-relayer/config.toml`).
- Must be set before mainnet to prevent transient ISMP delivery failures from permanently skipping commitments.

### MaxConsumers increase
- **Done on testnet:** spec 390, `type MaxConsumers = ConstU32<64>` in `lib.rs` (was 16).
- **Mainnet:** Verify whether mainnet has the same `MaxConsumers = 16` limit and whether any mainnet stash accounts are approaching the old limit. If mainnet never ran the `TestnetOcexStateReset` migration, its consumer counts are accurate and the limit may not need changing — but raising it is still a safety improvement.

---

## Checklist before next mainnet runtime upgrade

- [ ] Remove `TestnetOcexStateReset` from `type Migrations`
- [ ] Remove `ResetTestnetAssetSupply` from `type Migrations`
- [ ] Confirm mainnet current spec version — verify `UpgradeSessionKeys` guard applies
- [ ] Check mainnet MaxConsumers — raise to 64 if still at 16
- [ ] Set `deliver_failed = true` in tesseract config on relayer server
- [ ] Build runtime with `--features on-chain-release-build`
- [ ] Test full migration run on silo before touching mainnet
- [ ] After mainnet upgrade: remove all one-time migrations from the tuple for the following spec bump
