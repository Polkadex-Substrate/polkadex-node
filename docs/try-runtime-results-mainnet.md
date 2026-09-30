# try-runtime Results: v373 → v392 (unfiltered) against mainnet, via local snapshot

**Date:** 2026-09-29
**Tool:** try-runtime-core 0.10.1
**Runtime under test:** `fix/spec-392-blockers` @ `9591a2f` (includes the A01/A02/A04/A07 independent-audit fixes — Recovery blocked for NonTransfer, BEEFY MaxAuthorities aligned, Contracts/Revive removed entirely)
**Method:** snapshot-based, not a direct live run — per visiondream3's suggestion, to avoid the `RestartNeeded` connection drops hit when running `live` directly against a busy RPC endpoint under sustained load.

**Step 1 — create the snapshot:**
```bash
try-runtime create-snapshot mainnet.snap --uri wss://so.polkadex.ee
```
**Step 2 — run the check against the local file:**
```bash
RUST_LOG=info try-runtime \
  --runtime target/release/wbuild/node-polkadex-runtime/node_polkadex_runtime.wasm \
  on-runtime-upgrade \
  --blocktime 12000 \
  --checks all \
  snap --path mainnet.snap
```
No `--pallet` filters — every pallet's state was scraped and checked.

**Block scraped:** `0x92380f8c9ea3c36df43e37a57356af567256e42934b8f99978b791e3d13fa3f0` (finalized head at snapshot time)
**Snapshot size:** 711,273,640 bytes (3,184,386 keys)
**Runtime versions:** Original `[Version: 373] [Code hash: 0xa598...e42b]` → New `[Version: 392] [Code hash: 0x9839...cddf]` (new code hash vs. the previous run — reflects the Contracts/Revive removal shrinking the WASM)

---

## Result: ✅ Full success, no errors

Same four phases as before; all completed cleanly with exit code 0, and markedly faster than the live run since state loads from disk instead of over RPC (each phase reloads the snapshot in ~2-3 seconds).

### 1. Migration dry-run (`checks: None`)
All migrations executed against the snapshot:
- Session keys migration — added mixnet + beefy ✅
- `ClearLegacySudoKey` — cleared legacy `Sudo::Key` prefix ✅
- `ClearOrderbookCommittee` — 0 keys (was always empty) ✅
- Storage version bumps: Staking, Session, GRANDPA, Identity, Balances, ElectionProviderMultiPhase, Council, TechnicalCommittee, ImOnline, Offences, Historical, Scheduler, Multisig, Bounties, Democracy, Preimage, Assets — all ✅
- `FixBalancesFrozen` — fixed 1 account with a stale lock ✅
- `ClearOffenceReports` — cleared 3,697 `Offences::Reports` entries ✅
- `ClearOrmlVestingLocks` — **unlocked 12 accounts, ran 13 clear_prefix iterations** ✅ (matches the confirmed 12 vesting schedules + 1 StorageVersion marker breakdown)

### 2. Idempotency check (`checks: All`, re-run)
Every one-shot migration correctly skipped on the second pass (all firing the `warn!` skip logs).
```
Storage root before: 0xbb946d8b9bac86e1e1a177bac8d7c3b51d82029504135cf56d38c76e0a4211c8
Storage root after:  0xbb946d8b9bac86e1e1a177bac8d7c3b51d82029504135cf56d38c76e0a4211c8
✅ Migrations are idempotent
```

### 3. Weight / PoV check
```
PoV size (zstd-compressed compact proof): 1.1 MiB
Consumed ref_time: 1.0133796s (25.33% of max 4s)
✅ No weight safety issues detected.
```
Consistent with the previous run — OCEX/ISMP/LMP/THEA remain disconnected, so `PruneStaleIngressMessages` stays a no-op.

### 4. Multi-block migrations + full state decode + try-state (`checks: All`)
- `MBM finished after 1 blocks` ✅
- `✅ Entire runtime state decodes without error. 18,550,857 bytes total.`
- `try-state` checks ran and passed for every pallet that implements the hook: System, Utility, Babe, Timestamp, Authorship, Indices, Balances, TransactionPayment, ElectionProviderMultiPhase, Staking, Session, Council, TechnicalCommittee, Elections, TechnicalMembership, Grandpa, Treasury, ImOnline, AuthorityDiscovery, Offences, Historical, Identity, Recovery, Scheduler, Proxy, Multisig, Bounties, Democracy, Preimage, ChildBounties, Assets, AssetConversion, AssetConversionTxPayment, Statement, PoolAssets, SkipFeelessPayment, Alliance, AllianceMotion, DelegatedStaking, RandomnessCollectiveFlip, SafeMode, TxPause, MultiBlockMigrations, Beefy, Mmr, MmrLeaf, Mixnet, Society.
- Note: `Contracts` and `Revive` no longer appear in this list — both pallets were removed from the runtime on 2026-09-28 (independent audit findings A02/A07). Everything else matches the previous run's pallet list exactly.

**One pre-existing warning, unrelated to this PR (also seen in the previous run):**
```
💸 total issuance will cause T::CurrencyToVote to downscale -- report to maintainers.
```

---

## Comparison to the previous mainnet run (2026-09-24, live)

| | Previous run (live) | This run (snapshot) |
|---|---|---|
| Method | Direct `live --uri` | `create-snapshot` then `snap --path` |
| Code hash (new) | `0x20d8...8cec` | `0x9839...cddf` (Contracts/Revive removed) |
| try-state pallet count | 51 (incl. Contracts, Revive) | 49 (Contracts/Revive gone) |
| Offences::Reports cleared | 3,660 | 3,697 (more offences recorded live since) |
| Weight/PoV | 25.25% of budget | 25.33% of budget |
| Result | Full pass | Full pass |

No regressions — same clean result, now reflecting the current PR head after the Contracts/Revive removal.

---

## Raw log

Full raw output saved alongside this file at `docs/try-runtime-mainnet.log`.
