# try-runtime Results: v373 → v392 (unfiltered) against mainnet, via local snapshot

**Date:** 2026-10-02
**Tool:** try-runtime-core 0.10.1
**Runtime under test:** `fix/spec-392-blockers` @ `247a29f`, which replaces the GRANDPA storage version bump with `pallet_grandpa::migrations::MigrateV4ToV5`, the migration that moves the authority list. This replaces the previous run at `9591a2f`, whose first pass ran without checks (see "Flags" below).
**Method:** snapshot-based, not a direct live run, to avoid the `RestartNeeded` connection drops hit when running `live` directly against a busy RPC endpoint.

**Step 1: build the runtime with try-runtime enabled:**
```bash
cargo build --release --locked -p node-polkadex-runtime --features try-runtime
```
`target/release/wbuild/node-polkadex-runtime/node_polkadex_runtime.compact.compressed.wasm`, copied as `runtime-392-247a29f-tryruntime.compact.compressed.wasm`: 1,415,445 bytes, sha256 `3ac819cbb4b5e880ceed4e1ff786c30b618a36ad187da46dc76fc661b354e9c7`.

**Step 2: create the snapshot** (the path goes before `--uri`, which takes several values):
```bash
try-runtime create-snapshot mainnet.snap --uri ws://127.0.0.1:9944
```
Taken from a mainnet full node; the public RPC endpoints were down at the time.

**Step 3: run the check against the local file:**
```bash
RUST_LOG=info,runtime=info try-runtime \
  --runtime runtime-392-247a29f-tryruntime.compact.compressed.wasm \
  on-runtime-upgrade \
  --blocktime 12000 \
  --checks all \
  --disable-mbm-checks \
  snap --path mainnet.snap
```
No `--pallet` filters: every pallet's state was scraped and checked.

**Block scraped:** 13,149,139, `0x79d03dc91d2a6f4833cbe5e19d82c17e4a61363b700abc10d87dfac885eefa16`
**Snapshot size:** 711,566,562 bytes (3,184,691 keys)
**Runtime versions:** Original `[Version: 373] [Code hash: 0xa598...e42b]` → New `[Version: 392] [Code hash: 0x699b...28af]`

### Flags

In try-runtime 0.10.1 the first pass, the only one that runs the migrations on unmigrated 373 state, gets the `--checks` value only when `--disable-mbm-checks` is set. Without it that pass runs with checks None, and the checks run only in the final multi-block migration pass, on state that is already migrated, where `VersionedMigration` skips GRANDPA's checks because the pallet is already at version 5. This runtime defines no multi-block migrations (`pallet_migrations` has `Migrations = ()` outside benchmarks), so the flag skips nothing. The previous run used the default flags.

---

## Result: ✅ Full success, exit code 0

### 1. Migrations on the unmigrated 373 state (`checks: All`)
- Session keys migration: 200 queued keys before and after, mixnet and beefy added, post-check passed ✅
- `ClearLegacySudoKey`: `Sudo::Key` present before, legacy prefix cleared ✅
- `ClearOrderbookCommittee`: 0 keys (was always empty) ✅
- Staking 0 → 16, Session 0 → 1 ✅
- **GRANDPA: `Pallet "Grandpa" VersionedMigration migrating storage version from 4 to 5`, inside this checked pass.** Its pre_upgrade (the old authority list is not empty and holds at most `MaxAuthorities` = 200) and post_upgrade (`Grandpa::Authorities` has the same length and `:grandpa_authorities` is gone) ran on version 4 state. They log nothing when they pass and stop the run when they fail ✅
- Storage version bumps: Identity, Balances, ElectionProviderMultiPhase, Council, TechnicalCommittee, ImOnline, Offences, Historical, Scheduler, Multisig, Bounties, Democracy, Preimage, Assets ✅
- `FixBalancesFrozen`: fixed 1 account with a stale lock ✅
- `FixCouncilPrime`: applied (reported as already applied on the second pass) ✅
- `ClearOffenceReports`: cleared 3,703 `Offences::Reports` entries ✅
- `ClearOrmlVestingLocks`: 13 keys found, **unlocked 12 accounts, ran 13 clear_prefix iterations**, post-check 13 cleared and 0 left ✅

### 2. Idempotency check (second pass)
Every one-shot migration skipped on the second pass; GRANDPA reports `migration 4->5 can be removed; on-chain is already at StorageVersion(5)`. try-runtime 0.10.1 runs this pass with checks None whatever `--checks` says.
```
Storage root before: 0x8a56b0a49d4671b9f6598856bf9d1140e0b96b92699752f73a7f77bfcafaa0d4
Storage root after:  0x8a56b0a49d4671b9f6598856bf9d1140e0b96b92699752f73a7f77bfcafaa0d4
✅ Migrations are idempotent
```

### 3. Weight / PoV check
```
PoV size (zstd-compressed compact proof): 2.5 MiB
Consumed ref_time: 1.0170546s (25.43% of max 4s)
✅ No weight safety issues detected.
```
The same snapshot gives the same 2.5 MiB with the default flags. PoV size limits parachains only; this chain is a solochain, and the figure that matters is the 25.43% of block time.

### 4. Full state decode and try-state
- `✅ Entire runtime state decodes without error. 18510844 bytes total.`
- `try-state` checks ran and passed for all 48 pallets that implement the hook: System, Utility, Babe, Timestamp, Authorship, Indices, Balances, TransactionPayment, ElectionProviderMultiPhase, Staking, Session, Council, TechnicalCommittee, Elections, TechnicalMembership, Grandpa, Treasury, ImOnline, AuthorityDiscovery, Offences, Historical, Identity, Recovery, Scheduler, Proxy, Multisig, Bounties, Democracy, Preimage, ChildBounties, Assets, AssetConversion, AssetConversionTxPayment, Statement, PoolAssets, SkipFeelessPayment, Alliance, AllianceMotion, DelegatedStaking, RandomnessCollectiveFlip, SafeMode, TxPause, MultiBlockMigrations, Beefy, Mmr, MmrLeaf, Mixnet, Society.
- Multi-block migration pass: skipped by `--disable-mbm-checks`; the runtime defines none.

**One pre-existing warning, unrelated to this PR (also seen in the earlier runs):**
```
💸 total issuance will cause T::CurrencyToVote to downscale -- report to maintainers.
```

---

## Comparison to the previous run (`9591a2f`, 2026-09-29)

| | Previous run | This run |
|---|---|---|
| Flags | `--checks all` | `--checks all --disable-mbm-checks` |
| Checks on the first (migrating) pass | None | All |
| GRANDPA | storage version bump only; the authority list was not moved | `MigrateV4ToV5` moves the list; its pre and post checks ran |
| Block | `0x92380f8c...a3f0` | 13,149,139 |
| Code hash (new) | `0x9839...cddf` | `0x699b...28af` |
| try-state pallets | 48 (the earlier table said 49; its own list has 48) | 48, same list |
| Offences::Reports cleared | 3,697 | 3,703 |
| Weight | 25.33% of the block | 25.43% of the block |
| Result | pass, but the migrating pass ran without checks | full pass |

---

## Raw log

Full raw output of this run: `docs/try-runtime-mainnet.log`.
