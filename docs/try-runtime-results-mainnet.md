# try-runtime Results: v373 → v392 (unfiltered) against mainnet, via local snapshot

**Date:** 2026-10-10
**Tool:** try-runtime-core 0.10.1
**Runtime under test:** `fix/392-template-defaults` @ `df09c11`, the PR 23 head. Against the previous run at `247a29f` it adds: council elections fill 5 seats; native fees and tips go 100% to the Treasury; Treasury spends by Root or 3/5 of the Council up to 50,000 PDEX; the Alliance, AllianceMotion, Society and RandomnessCollectiveFlip pallets removed; PoolAssets creation closed to users; and three migrations, `ClearRandomnessCollectiveFlip`, `ClearRewardsAndMigrationLocks` and the SDK's treasury `cleanup_proposals`.
**Method:** snapshot-based, not a direct live run, to avoid the `RestartNeeded` connection drops hit when running `live` directly against a busy RPC endpoint. All steps ran in one `scripts/build-and-check.sh` run on a fresh build machine.

**Step 1: build the runtime with try-runtime enabled** (and the release runtime, for the hashes below):
```bash
cargo build --release --locked -p node-polkadex-runtime --features try-runtime
cargo build --release --locked -p node-polkadex-runtime --features on-chain-release-build
```
try-runtime build `node_polkadex_runtime-392-tryruntime.compact.compressed.wasm`: 1,362,204 bytes, sha256 `b4b09ca54baa88b3c496c6b5c8bb0f0fc54d65f056a17a5a13b2549d8afdf8ec`, blake2_256 `0x741b396c7e263028d54f6fb10a7120ccdee2ee37d137b8e6a1479217828edd1f`.

Release build `node_polkadex_runtime-392.compact.compressed.wasm`, the one to upload: 1,268,867 bytes, sha256 `eda7b6a5ec43e82f6c93be9ba519dfcf94048447f505aa78fb91b50972c6c768`, blake2_256 `0xccaf34613316e742db4cb844a55c5750148139f0aa181d84f986d12fbe9c0273`. The blake2_256 is the code hash the chain reports after enactment.

**Step 2: create the snapshot** (the path goes before `--uri`, which takes several values):
```bash
try-runtime create-snapshot mainnet.snap --uri <mainnet RPC endpoint>
```
Run separately, in parallel with the build, as the script's `SNAPSHOT_FILE` option describes. The snapshot command's own output was not kept with the results; the block number below is the one the runtime prints in the run's log.

**Step 3: run the check against the local file:**
```bash
RUST_LOG=info,runtime=info try-runtime \
  --runtime node_polkadex_runtime-392-tryruntime.compact.compressed.wasm \
  on-runtime-upgrade \
  --blocktime 12000 \
  --checks all \
  --disable-mbm-checks \
  snap --path mainnet.snap
```
No `--pallet` filters: every pallet's state was scraped and checked.

**Block scraped:** 13,204,927
**Snapshot size:** 713,068,407 bytes
**Runtime versions:** Original `[Version: 373] [Code hash: 0xa598...e42b]` → New `[Version: 392] [Code hash: 0x741b...dd1f]` (the try-runtime build; the release build's code hash is `0xccaf...0273`)

### Flags

In try-runtime 0.10.1 the first pass, the only one that runs the migrations on unmigrated 373 state, gets the `--checks` value only when `--disable-mbm-checks` is set. Without it that pass runs with checks None, and the checks run only in the final multi-block migration pass, on state that is already migrated, where `VersionedMigration` skips GRANDPA's checks because the pallet is already at version 5. This runtime defines no multi-block migrations (`pallet_migrations` has `Migrations = ()` outside benchmarks), so the flag skips nothing. Same flags as the previous run.

---

## Result: ✅ Full success, exit code 0

### 1. Migrations on the unmigrated 373 state (`checks: All`)
- Session keys migration: 200 queued keys before and after, mixnet and beefy added, post-check passed ✅
- `ClearLegacySudoKey`: `Sudo::Key` present before, legacy prefix cleared (2 keys) ✅
- `ClearOrderbookCommittee`: 0 keys (was always empty) ✅
- `ClearRandomnessCollectiveFlip`: 1 key found and removed, none left after ✅
- Staking 0 → 16, Session 0 → 1 ✅
- **GRANDPA: `Pallet "Grandpa" VersionedMigration migrating storage version from 4 to 5`, inside this checked pass.** Its pre_upgrade (the old authority list is not empty and holds at most `MaxAuthorities` = 200) and post_upgrade (`Grandpa::Authorities` has the same length and `:grandpa_authorities` is gone) ran on version 4 state. They log nothing when they pass and stop the run when they fail ✅
- Storage version bumps: Identity, Balances, ElectionProviderMultiPhase, Council, TechnicalCommittee, ImOnline, Offences, Historical, Scheduler, Multisig, Bounties, Democracy, Preimage, Assets ✅
- `FixBalancesFrozen`: fixed 1 account with a stale lock ✅
- `FixCouncilPrime`: applied (reported as already applied on the second pass) ✅
- `ClearOffenceReports`: cleared 3,729 `Offences::Reports` entries ✅
- `ClearOrmlVestingLocks`: 13 keys found, **unlocked 12 accounts, ran 13 clear_prefix iterations**, post-check 13 cleared and 0 left ✅
- `ClearRewardsAndMigrationLocks`: **removed all 732 `REWARDID` locks (319,287.14 PDEX) and all 954 `pdexlock` locks (404,434.83 PDEX)**; one reaped account (no providers) held a lock without a consumer ref and got one before its lock was removed; post-check 0 `REWARDID` and 0 `pdexlock` locks left ✅
- Treasury `cleanup_proposals`: 4 legacy proposals, **4 bonds released (681.8 + 2,803.125 + 8,171.05 + 3,237.1645 = 14,893.1395 PDEX)**, proposals and approvals count (4, 0) → (0, 0) ✅

### 2. Idempotency check (second pass)
Every one-shot migration skipped on the second pass; GRANDPA reports `migration 4->5 can be removed; on-chain is already at StorageVersion(5)`, and the treasury cleanup releases 0 bonds. try-runtime 0.10.1 runs this pass with checks None whatever `--checks` says. The pass logs `Removed 1 RandomnessCollectiveFlip keys`: the count includes the key still present in the snapshot underneath the first pass's changes. The storage root is the same before and after, so nothing was removed twice.
```
Storage root before: 0xd51d4c291b2fe06a4b4b3d7c9fa1dacc4bb1fef5d4965d227a96e56777f1675e
Storage root after:  0xd51d4c291b2fe06a4b4b3d7c9fa1dacc4bb1fef5d4965d227a96e56777f1675e
✅ Migrations are idempotent
```

### 3. Weight / PoV check
```
PoV size (zstd-compressed compact proof): 2.5 MiB
Consumed ref_time: 1.6208916s (40.52% of max 4s)
✅ No weight safety issues detected.
```
Up from 25.43% in the previous run, with the migrations added since. PoV size limits parachains only; this chain is a solochain, and the figure that matters is the 40.52% of block time.

### 4. Full state decode and try-state
- `✅ Entire runtime state decodes without error. 19351676 bytes total.`
- `try-state` checks ran and passed for all 44 pallets that implement the hook: System, Utility, Babe, Timestamp, Authorship, Indices, Balances, TransactionPayment, ElectionProviderMultiPhase, Staking, Session, Council, TechnicalCommittee, Elections, TechnicalMembership, Grandpa, Treasury, ImOnline, AuthorityDiscovery, Offences, Historical, Identity, Recovery, Scheduler, Proxy, Multisig, Bounties, Democracy, Preimage, ChildBounties, Assets, AssetConversion, AssetConversionTxPayment, Statement, PoolAssets, SkipFeelessPayment, DelegatedStaking, SafeMode, TxPause, MultiBlockMigrations, Beefy, Mmr, MmrLeaf, Mixnet.
- Multi-block migration pass: skipped by `--disable-mbm-checks`; the runtime defines none.

**One pre-existing warning, unrelated to this PR (also seen in the earlier runs):**
```
💸 total issuance will cause T::CurrencyToVote to downscale -- report to maintainers.
```

---

## Comparison to the previous run (`247a29f`, 2026-10-02)

| | Previous run | This run |
|---|---|---|
| Flags | `--checks all --disable-mbm-checks` | same |
| Block | 13,149,139 | 13,204,927 |
| Code hash (new, try-runtime build) | `0x699b...28af` | `0x741b...dd1f` |
| Pallets | with Alliance, AllianceMotion, Society, RandomnessCollectiveFlip | those four removed |
| Migrations added | | `ClearRandomnessCollectiveFlip`, `ClearRewardsAndMigrationLocks`, treasury `cleanup_proposals` |
| try-state pallets | 48 | 44 |
| Offences::Reports cleared | 3,703 | 3,729 |
| State decoded | 18,510,844 bytes | 19,351,676 bytes |
| Weight | 25.43% of the block | 40.52% of the block |
| Result | full pass | full pass |

---

## Raw log

Full raw output of this run: `docs/try-runtime-mainnet.log`.
