# Runtime Migrations — Status & Mainnet Readiness

**Current testnet spec: 390**
**File:** `runtimes/mainnet/src/migrations.rs`
**Wired in:** `runtimes/mainnet/src/lib.rs` — `type Migrations = (...)`

---

## Summary

| Migration | Testnet status | Mainnet required | Safe to remove after first run |
|---|---|---|---|
| `UpgradeSessionKeys` | ✅ Applied (spec 379) | ✅ Yes (if mainnet < spec 379) | Yes, after first run |
| `ClearLegacySudoKey` | New in 392; ran in try-runtime and on the mainnet-state soak | ✅ Yes | Yes |
| `ClearOrderbookCommittee` | New in 392; ran in try-runtime and on the mainnet-state soak | ✅ Yes (the prefix is already empty) | Yes |
| `ClearRandomnessCollectiveFlip` | New in 392; ran in try-runtime and on the mainnet-state soak | ✅ Yes | Yes |
| `StakingStorageVersionMigration` | ✅ Applied | ✅ Yes | Yes |
| `SessionStorageVersionMigration` | ✅ Applied | ✅ Yes | Yes |
| `pallet_grandpa::migrations::MigrateV4ToV5` | No-op, testnet is already at v5 | ✅ Yes, mainnet is at v4 | Yes |
| `IdentityStorageVersionMigration` | ✅ Applied | ✅ Yes | Yes |
| `ChildBountiesStorageVersionMigration` | ✅ Applied | ✅ Yes | Yes |
| `StorageVersionMigration<*>` × 13 | ✅ Applied | ✅ Yes | Yes |
| `FixBalancesFrozen` | ✅ Applied | ✅ Yes | Yes |
| `FixCouncilPrime` | ✅ Applied | ✅ Yes | Yes |
| `ClearOffenceReports` | ✅ Applied | ✅ Yes | Yes |
| `ClearOrmlVestingLocks` | New in 392; ran in try-runtime and on the mainnet-state soak | ✅ Yes | Yes |
| `ClearRewardsAndMigrationLocks` | New in 392; ran in try-runtime and on the mainnet-state soak | ✅ Yes | Yes |
| `pallet_treasury::migration::cleanup_proposals::Migration` | New in 392; ran in try-runtime and on the mainnet-state soak | ✅ Yes | Yes |

This list follows `type Migrations` in `runtimes/mainnet/src/lib.rs`, in the order the runtime runs them.

---

## Detail per migration

### ✅ Keep for mainnet — run once, then remove

---

#### `UpgradeSessionKeys`
- **What it does:** Migrates session keys from the old format (grandpa + babe + im_online + authority_discovery + orderbook + **thea**) to the new format (same minus thea, plus **mixnet** + **beefy**). Validators are initialised with dummy mixnet/beefy keys and must call `author_rotateKeys` + `session_setKeys` post-upgrade.
- **Guard:** `last_runtime_upgrade_spec_version() <= 378`. No-op on any chain already past spec 378.
- **Mainnet:** Required if mainnet is upgrading from a spec ≤ 378 runtime. Check mainnet's current spec version before the upgrade.
- **After first run:** Remove from the tuple.

---

#### `ClearLegacySudoKey`
- **What it does:** F-002. `pallet_sudo` was removed from `construct_runtime!` long ago, but its storage was never wiped: `Sudo::Key` still holds the 2021 genesis root key. Clears everything under the `Sudo` prefix, the key and the storage version marker, so nothing is left under a pallet name the runtime does not have.
- **Guard:** `last_runtime_upgrade_spec_version() <= 391`: runs once, upgrading into 392.
- **Mainnet:** Required. try-runtime at block 13,204,927 found `Sudo::Key` and removed 2 keys.
- **After first run:** Remove from the tuple.

---

#### `ClearOrderbookCommittee`
- **What it does:** F-029. `RemovePallet` for the `OrderbookCommittee` prefix. That committee (`pallet_collective` Instance4) governed OCEX, which was removed from `construct_runtime!`; this wipes anything left under its prefix (members, proposals, votes).
- **Guard:** None needed. A second run finds nothing under the prefix.
- **Mainnet:** In the tuple as a guarantee. try-runtime at block 13,204,927 found no keys under the prefix, so on mainnet it removes nothing.
- **After first run:** Remove from the tuple.

---

#### `ClearRandomnessCollectiveFlip`
- **What it does:** `RemovePallet` for the `RandomnessCollectiveFlip` prefix. Mainnet still holds one `RandomnessCollectiveFlip::RandomMaterial` key (2,594 bytes) left by a runtime older than 373; no runtime since has had the pallet, and 392 does not either.
- **Guard:** None needed. A second run finds nothing under the prefix.
- **Mainnet:** Required. try-runtime at block 13,204,927 removed the one key and found none left.
- **After first run:** Remove from the tuple.

---

#### Storage version migrations (×17)

These all follow the same pattern: compare on-chain storage version with in-code version, bump if behind, no-op if current.

| Migration | Pallet |
|---|---|
| `StakingStorageVersionMigration` | `pallet_staking` |
| `SessionStorageVersionMigration` | `pallet_session` |
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

`pallet_grandpa::migrations::MigrateV4ToV5` is not a bump. It moves the authority list from the well-known key `:grandpa_authorities` into `Grandpa::Authorities`, removes the old key, and only runs at on-chain version 4. Mainnet is at v4, so it runs there.

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

#### `ClearOrmlVestingLocks<Runtime>`
- **What it does:** F-030. OrmlVesting was removed from `construct_runtime!` without a cleanup migration, so its `ormlvest` lock stays on 12 accounts with no call left that can remove it. Removes that lock from each of them and wipes the orphaned OrmlVesting storage: 12 `VestingSchedules` entries and the storage version marker.
- **Guard:** `last_runtime_upgrade_spec_version() <= 391`: runs once, upgrading into 392.
- **Mainnet:** Required. try-runtime at block 13,204,927 found 13 keys, unlocked 12 accounts and left none.
- **After first run:** Remove from the tuple.

---

#### `ClearRewardsAndMigrationLocks`
- **What it does:** Rewards and PDEXMigration were removed from `construct_runtime!` without a cleanup migration, so their locks stay with no call left that can release them: `REWARDID` on 732 accounts (319,287.14 PDEX) and `pdexlock` on 954 accounts (404,434.83 PDEX). It removes each lock that the old pallet would release at the current block under mainnet spec 373's rules: `REWARDID` when `Rewards::claim` would succeed (no block condition), `pdexlock` when `PDEXMigration::Operational` is set and `LockedTokenHolders(who) + 201,600 <= now`. Any other lock stays. An account that holds such a lock without the consumer ref Balances expects (one reaped account on mainnet) gets the ref first, so the removal does not underflow.
- **Guard:** `last_runtime_upgrade_spec_version() <= 391`: runs once, upgrading into 392.
- **Mainnet:** Required. try-runtime at block 13,204,927 removed all 732 `REWARDID` and all 954 `pdexlock` locks and added the one missing consumer ref. On a fork of mainnet state, block numbers restart at 0, so the `pdexlock` rule is not met there and those locks stay; that is a fork artifact.
- **After first run:** Remove from the tuple.

---

#### `pallet_treasury::migration::cleanup_proposals::Migration<Runtime, (), BalanceUnreserveWeight>`
- **What it does:** The SDK's cleanup (polkadot-sdk PR #5892). Releases the bonds of legacy treasury proposals that were never approved and removes the proposals. Approved proposals are left alone. On mainnet: proposals #62, #69, #72 and #81, 14,893.14 PDEX of bonds.
- **Guard:** None needed. A second run finds nothing to release.
- **Mainnet:** Required. try-runtime released the 4 bonds and left proposals and approvals at (0, 0).
- **After first run:** Remove from the tuple.

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

- [x] Remove `TestnetOcexStateReset` from `type Migrations`
- [x] Remove `ResetTestnetAssetSupply` from `type Migrations`
- [ ] Confirm mainnet current spec version — verify `UpgradeSessionKeys` guard applies
- [ ] Check mainnet MaxConsumers — raise to 64 if still at 16
- [ ] Set `deliver_failed = true` in tesseract config on relayer server
- [ ] Build runtime with `--features on-chain-release-build`
- [ ] Test full migration run on silo before touching mainnet
- [ ] After mainnet upgrade: remove all one-time migrations from the tuple for the following spec bump
