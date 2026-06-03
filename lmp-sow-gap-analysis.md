# LMP SOW v2 — Gap Analysis & Repo-Level Work Breakdown
**Date:** 2026-05-06 | **Author:** Engineering Review  
**Repos:** polkadex-node · orderbook · orderbook-fe · [new] eth-bridge-contract · [new] lmp-calibration

---

## 1. Executive Summary

The SOW v2 describes a 7-work-package LMP system. A partial LMP v1 already exists. The table below gives the honest state before any new work begins.

| Work Package | SOW Scope | Current Status |
|---|---|---|
| WP-01: pallet-ocex-lmp | Q-score, DMM, volatility, tiering, fee split | ~35% — core scoring exists; DMM / volatility / tiering absent |
| WP-02: Q-Score Engine | 60s sampling, epoch aggregation, anti-gaming, Merkle worker | ~20% — sampling loop only; aggregation / anti-gaming absent |
| WP-03: maxSpread Calibration | 4-condition ADF analysis, governance proposals | 0% — not started |
| WP-04: pallet-rewards | LMP epoch pool, DMM stipend, Merkle claims | 0% — existing rewards pallet is crowdloan vesting, unrelated |
| WP-05: Custom Bridge | THEA replacement, Eth contract, relayer sidecar | 0% — not started |
| WP-06: API & Data Services | 10 REST endpoints, 3 WS feeds, data pipeline | 0% — no LMP endpoints exist |
| WP-07: Frontend | LMP dashboard, bridge UI | 0% — not started |

---

## 2. Repo: `polkadex-node`

### 2.1 `pallets/ocex/src/` — Modify Existing

| # | Work Item | Status | File / Location | Notes |
|---|---|---|---|---|
| C-01 | Add `MarketTier` enum (Tier1/Tier2/Tier3) to `TradingPairConfig` | MISSING | `primitives/orderbook/src/lmp.rs` | Storage migration required on existing `TradingPairConfig` |
| C-02 | Add `set_pair_tier(pair, tier)` governance extrinsic | MISSING | `lib.rs` | New call index; assign next available |
| C-03 | Make `compute_trader_metrics()` look up y/z exponents by tier | MISSING | `validator.rs:864–894` | Currently hardcoded y=0.15, z=0.85 for all pairs |
| C-04 | Add `FeesCollected` storage: `(epoch, pair) → Balance` | MISSING | `lib.rs` | Needed for 25% taker fee → LMP pool split |
| C-05 | Populate `FeesCollected` on every taker fill inside trade execution | MISSING | `lib.rs` (trade processing path) | Must fire before epoch boundary |
| C-06 | Add 25% fee-split logic in `on_initialize` at epoch boundary | MISSING | `lib.rs on_initialize` | Transfer 25% of `FeesCollected` to reward pool; reset storage |
| C-07 | Enforce `max_spread` filter in Q-score eligibility | MISSING | `validator.rs` | Field exists in `LMPMarketConfig` but is never read during scoring |
| C-08 | Enforce `min_depth` filter in Q-score eligibility | MISSING | `validator.rs` | Same as above — stored, never enforced |
| C-09 | Add `DMMRegistry` storage: `(epoch, pair) → BoundedVec<DMMCommitment>` | MISSING | `lib.rs` | New bounded storage; needs `MaxDMMsPerPair` config const |
| C-10 | Add `DMMPerformance` storage: `(epoch, pair, account) → u8` (uptime %) | MISSING | `lib.rs` | Updated by engine via snapshot submission |
| C-11 | Add `register_dmm(epoch, pair, spread, depth, uptime)` extrinsic | MISSING | `lib.rs` | Callable before epoch start; validates commitment values |
| C-12 | Add `confirm_dmm_selection(epoch, pair, accounts)` governance extrinsic | MISSING | `lib.rs` | Governance/council selects winning DMM bids |
| C-13 | Add `submit_dmm_performance(epoch, pair, performance)` extrinsic | MISSING | `lib.rs` | Called at epoch end by engine operator; records actual uptime % |
| C-14 | Add `claim_dmm_stipend(epoch, pair)` extrinsic | MISSING | `lib.rs` | Verifies `DMMPerformance >= committed_uptime`; pays stipend |
| C-15 | Reserve DMM stipend in `on_initialize` at epoch start | MISSING | `lib.rs on_initialize` | Transfer stipend from treasury to pallet account |
| C-16 | Add `VolatilityTriggerCount` storage: `(epoch, pair, day) → u8` | MISSING | `lib.rs` | Capped at 6 per pair per day |
| C-17 | Add `trigger_volatility_multiplier(pair)` extrinsic | MISSING | `lib.rs` | Can be called by engine operator or council; enforces daily cap |
| C-18 | Apply 2× multiplier to Q-score contributions when trigger active | MISSING | `validator.rs` (snapshot processing) | Per-snapshot, not persistent across epoch |
| C-19 | Add `VolatilityMultiplierTriggered(pair, snapshot_id)` event | MISSING | `lib.rs` | For auditability |
| C-20 | Add Merkle root storage: `(epoch, pair) → H256` | MISSING | `lib.rs` | Submitted by engine operator after epoch aggregation |
| C-21 | Add `submit_lmp_snapshot(epoch, pair, q_scores, uptime, merkle_root)` extrinsic | MISSING | `lib.rs` | Threshold-voted by validators; replaces current `force_submit_snapshot` flow for LMP |
| C-22 | Add `claim_rewards(epoch, amount, merkle_proof)` extrinsic | MISSING | `lib.rs` | Verifies Merkle proof against stored root; transfers PDEX |
| C-23 | Add `RewardsClaimed` storage to prevent double-claim | MISSING | `lib.rs` | `(account, epoch) → Balance` |
| C-24 | Emit `DMMRegistered`, `DMMSelected`, `RewardClaimed`, `DMMStipendClaimed` events | MISSING | `lib.rs` | New events for all DMM and claim flows |
| C-25 | Add maker rebate logic (−0.01% to −0.03%) for Tier 1 fills within 5 bps of BBO | MISSING | `lib.rs` (trade execution path) | Paid per fill from taker fee pool; not per epoch |
| C-26 | Add market sunset / tier demotion extrinsics | MISSING | `lib.rs` | `suspend_lmp_rewards(pair)`, `demote_pair_tier(pair)` — governance-only |
| C-27 | Storage migration: add `tier` field to existing `TradingPairConfig` on-chain | MISSING | New `migrations/` module in pallet | Must use `try-runtime` before mainnet |
| C-28 | Benchmarks for all new extrinsics (C-02, C-11–C-14, C-17, C-21, C-22, C-26) | MISSING | `benchmarking.rs` | Required before runtime upgrade PR |

### 2.2 `pallets/ocex/src/lmp.rs` — Offchain State (Modify)

| # | Work Item | Status | Notes |
|---|---|---|---|
| C-29 | Verify `update_maker_volume_by_main_account()` is called end-to-end from StateChanges | PARTIAL | Function exists; wiring through StateChanges → chain ingress needs integration test |
| C-30 | Verify `store_q_score_and_uptime()` uptime count is correctly inferred from non-zero samples | PARTIAL | Logic exists; never tested end-to-end with real epoch data |
| C-31 | Add offchain storage for DMM real-time uptime tracking per epoch | MISSING | Needed so `submit_dmm_performance` has data to report |

### 2.3 `pallets/ocex/rpc/` — Modify Existing

| # | Work Item | Status | Notes |
|---|---|---|---|
| C-32 | Add `get_dmm_status(epoch, pair)` RPC | MISSING | Returns DMMRegistry + DMMPerformance for a pair |
| C-33 | Add `get_merkle_proof(account, epoch, pair)` RPC | MISSING | Returns Merkle proof leaf data for frontend claim modal |
| C-34 | Add `get_volatility_trigger_count(pair, day)` RPC | MISSING | Returns daily trigger count for monitoring |

### 2.4 `pallets/liquidity-mining/src/` — Fix Existing TODOs

| # | Work Item | Status | File / Line | Notes |
|---|---|---|---|---|
| C-35 | Fix unsigned tx validation logic | TODO | `lib.rs:297` | Currently too permissive |
| C-36 | Replace global `SnapshotFlag` with per-pool flags | TODO | `lib.rs:429, 479` | Prevents cross-pool interference |
| C-37 | Parameterize weights by request count | TODO | `lib.rs:649` | Benchmark accuracy |
| C-38 | Emit events on forced pool closure | TODO | `lib.rs:744` | Frontend needs these events |
| C-39 | Resolve `base_freed`/`quote_freed` handling in `force_close_pool` callback | TODO | `callback.rs:185` | Currently a no-op |

### 2.5 `pallets/rewards/` — Decision: New Pallet or Extend OCEX

**Option A — New `pallets/lmp-rewards/`:**

| # | Work Item | Status | Notes |
|---|---|---|---|
| C-40 | Create new `pallet-lmp-rewards` crate | MISSING | New Cargo crate; wire into runtime |
| C-41 | `EpochRewardPool` storage: `epoch → (lmp_balance, dmm_stipend_balance)` | MISSING | |
| C-42 | `ClaimRecord` storage: `(account, epoch) → Balance` | MISSING | Prevents double-claim |
| C-43 | `fund_epoch_pool(epoch, lmp_amount, dmm_amount)` extrinsic | MISSING | Called by `pallet-ocex` `on_initialize` |
| C-44 | `claim(epoch, amount, proof)` extrinsic | MISSING | Verifies Merkle proof, pays PDEX |
| C-45 | `claim_dmm_stipend(epoch, pair)` extrinsic | MISSING | Checks DMMPerformance record, pays stipend |

**Option B — Extend `pallet-ocex` directly (recommended for M2):** Items C-40–C-45 collapse into C-22, C-14, and C-06 above.

### 2.6 NEW: `pallets/custom-bridge/` — Entire New Pallet

| # | Work Item | Status | Notes |
|---|---|---|---|
| C-46 | Create `pallet-custom-bridge` Cargo crate | MISSING | |
| C-47 | `PendingDeposits` storage: `nonce → DepositVote` | MISSING | |
| C-48 | `ProcessedNonces` storage: `BoundedBTreeSet<Nonce>` | MISSING | Replay protection |
| C-49 | `WithdrawalRequests` storage: `id → WithdrawalRequest` | MISSING | Pending withdrawals for relayers |
| C-50 | `WhitelistedAssets` storage: `asset_id → (source_chain, contract_address)` | MISSING | Governance-approved token list |
| C-51 | `ValidatorSet` storage | MISSING | Current bridge validator set |
| C-52 | `register_deposit_vote(nonce, from, to, asset, amount)` extrinsic | MISSING | Validator submits observation; minting at 2/3 threshold |
| C-53 | `initiate_withdrawal(asset, amount, destination)` extrinsic | MISSING | User-callable; emits event for relayers |
| C-54 | `whitelist_asset(asset_id, chain, contract)` governance extrinsic | MISSING | |
| C-55 | `update_validator_set(validators)` governance extrinsic | MISSING | |
| C-56 | Nonce replay protection: reject duplicate nonces | MISSING | |
| C-57 | Emergency pause extrinsic (governance multisig) | MISSING | |
| C-58 | Daily bridge volume cap per asset (circuit breaker) | MISSING | |
| C-59 | Events: `DepositVoteSubmitted`, `AssetMinted`, `WithdrawalInitiated`, `AssetBurned` | MISSING | |
| C-60 | Benchmarks for all bridge extrinsics | MISSING | |

### 2.7 `primitives/orderbook/src/` — Modify Existing Types

| # | Work Item | Status | Notes |
|---|---|---|---|
| C-61 | Add `MarketTier` enum to `lmp.rs` | MISSING | `Tier1 / Tier2 / Tier3` |
| C-62 | Add `tier: MarketTier` field to `LMPMarketConfig` | MISSING | Used in exponent lookup |
| C-63 | Add `DMMCommitment` struct `{account, max_spread, min_depth, committed_uptime, stipend}` | MISSING | Used in `DMMRegistry` storage |
| C-64 | Add `BridgeAsset` struct `{asset_id, source_chain, contract_address}` | MISSING | Used in bridge pallet |

### 2.8 `runtimes/mainnet/src/lib.rs` — Modify Runtime

| # | Work Item | Status | Notes |
|---|---|---|---|
| C-65 | Wire `pallet-custom-bridge` into runtime | MISSING | New pallet index; assign carefully |
| C-66 | Wire `pallet-lmp-rewards` into runtime (if Option A chosen) | MISSING | |
| C-67 | Add storage migration for `TradingPairConfig` tier field (C-27) | MISSING | Must be gated by `on_chain_storage_version()` check |
| C-68 | Runtime upgrade proposal + Wasm artefact | MISSING | Governance referendum required |

---

## 3. Repo: `orderbook`

### 3.1 `engine/src/lmp.rs` — Modify Existing

| # | Work Item | Status | File | Notes |
|---|---|---|---|---|
| E-01 | Add uptime flag to `LMPOneMinuteReport` per account (present/absent boolean) | MISSING | `engine/src/lmp.rs` | Currently only score sent; uptime inferred on chain |
| E-02 | Add per-account `maker_volume` field to `LMPOneMinuteReport` | MISSING | `engine/src/lmp.rs` | Chain has storage for it but engine never populates it |
| E-03 | Implement volatility trigger detection (30-min price range, depth drop check) | MISSING | `engine/src/lmp.rs` | Fires when range > 3% Tier1 / 5% Tier2-3, or depth < 50% 30d avg |
| E-04 | Apply 2× multiplier to snapshot Q-scores when volatility trigger active | MISSING | `engine/src/lmp.rs` | Multiplier per snapshot; call `trigger_volatility_multiplier` on chain |
| E-05 | Implement `StopLMP` command handler (currently empty match arm) | MISSING | `engine/src/lmp.rs` | Should cleanly stop the sampling loop |
| E-06 | Add wash-trade filter: exclude maker volume where > 40% matched against own proxy accounts | MISSING | `engine/src/lmp.rs` | Anti-gaming: filter before populating makerVolume |
| E-07 | Add uptime spike detection: flag accounts absent 4+ consecutive hours then reappearing | MISSING | `engine/src/lmp.rs` | Flag for manual review; do not auto-exclude |
| E-08 | Add DMM real-time uptime tracking: count qualifying snapshots per DMM per epoch | MISSING | `engine/src/lmp.rs` | Input data for `submit_dmm_performance` at epoch end |
| E-09 | Add tier-awareness to snapshot: read market tier from LMPConfig, tag report | MISSING | `engine/src/lmp.rs` | Engine sends raw scores; chain needs tier info to select exponents |

### 3.2 `engine/src/engine.rs` — Modify Existing

| # | Work Item | Status | File | Notes |
|---|---|---|---|---|
| E-10 | Add `EngineMessage::SubmitDMMPerformance` handler | MISSING | `engine/src/engine.rs` | Triggered at epoch end to call `submit_dmm_performance` on chain |
| E-11 | Wire `maker_volume` from trade fills into `LMPOneMinuteReport` accumulation | PARTIAL | `engine/src/engine.rs` | `update_lmp_storage_from_trade()` exists on chain side; verify engine sends this via StateChanges |
| E-12 | End-to-end integration test: place order → fill → verify maker_volume in StateChanges → verify chain storage updated | MISSING | `engine/src/tests/` | Currently unverified |

### 3.3 NEW: `engine/src/epoch_aggregator.rs` (or `aggregators/lmp/`)

| # | Work Item | Status | Notes |
|---|---|---|---|
| E-13 | Build epoch aggregation worker: runs at epoch boundary, reads all per-minute reports from S3 | MISSING | New file/module |
| E-14 | Apply QFinal formula: `(depth_score)^y × (uptime_count)^5 × (maker_volume)^z` with tier-correct exponents | MISSING | |
| E-15 | Normalise scores per market: `maker_share = maker_QFinal / SUM(all_QFinal)` | MISSING | |
| E-16 | Compute reward per maker: `maker_share × epoch_reward_pool_for_pair` | MISSING | |
| E-17 | Build Merkle tree over `(account, epoch, pair, reward_amount)` leaves | MISSING | Use `sp_trie` or `rs-merkle` crate |
| E-18 | Publish leaf data to IPFS (CID generation) | MISSING | Required by SOW; consider S3 as fallback if IPFS adds complexity |
| E-19 | Submit `submit_lmp_snapshot(epoch, pair, q_scores, uptime, merkle_root)` to chain | MISSING | RPC call to chain node |
| E-20 | Persist final epoch scores + Merkle proofs to PostgreSQL permanently | MISSING | For API serving and claim proof generation |

### 3.4 `primitives/src/types.rs` — Modify Existing

| # | Work Item | Status | Notes |
|---|---|---|---|
| E-21 | Add `maker_volume` field to `LMPOneMinuteReport` | MISSING | Breaking change — all consumers of this struct need updating |
| E-22 | Add `uptime_present: bool` field to `LMPOneMinuteReport` per-account entry | MISSING | |
| E-23 | Add `LMPEpochFinalReport` struct for epoch aggregation output | MISSING | `{epoch, pair, merkle_root, per_account_rewards, tier}` |
| E-24 | Add `EngineMessage::SubmitDMMPerformance(epoch, pair, BTreeMap<AccountId, u8>)` variant | MISSING | |
| E-25 | Add `LMPCommand::TriggerVolatilityMultiplier(TradingPair)` variant | MISSING | Sent from engine to chain on trigger detection |

### 3.5 `tradesrelayer/src/service.rs` — Verify / Modify

| # | Work Item | Status | Notes |
|---|---|---|---|
| E-26 | Verify `OneMinLMPReport` extraction from `ob_message.action` is correct end-to-end | PARTIAL | Code path exists; never integration-tested with real LMP data |
| E-27 | Add `LMPEpochFinalReport` extraction from `UserActionBatch` at epoch boundary | MISSING | Epoch aggregation worker output must also flow through relayer |
| E-28 | Ensure `maker_volume` field (once added) flows correctly through StateChanges → UserActionBatch | MISSING | Depends on E-21 |

### 3.6 `server/src/` — Add New Handlers

The existing server is Rust + actix-web + async-graphql. All LMP API work goes here (no separate Node.js service recommended).

**New REST module: `server/src/rest/lmp.rs`**

| # | Work Item | Status | Endpoint |
|---|---|---|---|
| E-29 | Epochs list handler | MISSING | `GET /lmp/epochs` |
| E-30 | Epoch detail handler | MISSING | `GET /lmp/epochs/{epoch}` |
| E-31 | Leaderboard handler (Redis-cached) | MISSING | `GET /lmp/epochs/{epoch}/leaderboard` |
| E-32 | Account Q-score breakdown handler (real-time from chain RPC) | MISSING | `GET /lmp/accounts/{address}/qscore` |
| E-33 | Claimable rewards handler (Merkle proof generation) | MISSING | `GET /lmp/accounts/{address}/rewards/claimable` |
| E-34 | Pairs list with tier/maxSpread/minDepth/DMM | MISSING | `GET /lmp/pairs` |
| E-35 | Pair calibration report handler | MISSING | `GET /lmp/pairs/{pair}/calibration` |
| E-36 | Active DMM assignments handler | MISSING | `GET /lmp/dmm` |

**New REST module: `server/src/rest/bridge.rs`**

| # | Work Item | Status | Endpoint |
|---|---|---|---|
| E-37 | Supported bridge assets handler | MISSING | `GET /bridge/supported-assets` |
| E-38 | Deposit status handler (validator vote count) | MISSING | `GET /bridge/deposits/{txHash}` |

**New WebSocket module: `server/src/ws/lmp.rs`**

| # | Work Item | Status | Channel |
|---|---|---|---|
| E-39 | Live per-snapshot Q-score feed (includes volatility multiplier status) | MISSING | `ws://.../lmp/live` |
| E-40 | Personalised Q-score breakdown feed | MISSING | `ws://.../lmp/accounts/{address}` |
| E-41 | DMM real-time uptime feed per pair | MISSING | `ws://.../lmp/dmm/{pair}` |

**Data pipeline (new infra + code):**

| # | Work Item | Status | Notes |
|---|---|---|---|
| E-42 | PostgreSQL schema for raw LMP snapshots (minute-level) | MISSING | `server/src/db/lmp_snapshots.rs` |
| E-43 | Redis cache layer: 30s TTL for live scores, 5min TTL for epoch aggregates | MISSING | `server/src/cache/lmp.rs` |
| E-44 | Aggregation service: reads raw snapshots, produces epoch-level summaries for API | MISSING | Can be a background task within server or separate worker |

### 3.7 `cloud/backend-ts/` — CDK Infrastructure

| # | Work Item | Status | Notes |
|---|---|---|---|
| E-45 | PostgreSQL (RDS or Aurora Serverless) stack for LMP snapshot storage | MISSING | `lib/lmp-db-stack.ts` |
| E-46 | ElastiCache Redis cluster for API caching | MISSING | `lib/lmp-cache-stack.ts` |
| E-47 | ECS Fargate task for LMP epoch aggregation worker (E-13–E-20) | MISSING | `lib/lmp-aggregator-stack.ts` |
| E-48 | ECS Fargate task for bridge relayer sidecar | MISSING | `lib/bridge-relayer-stack.ts` |
| E-49 | ECS Fargate task for maxSpread calibration service (scheduled monthly) | MISSING | `lib/calibration-stack.ts` |
| E-50 | S3 bucket + lifecycle policy for calibration reports | MISSING | |
| E-51 | ALB routing rules: add `/lmp/*` and `/bridge/*` to existing rule set | MISSING | `lib/alb-stack.ts` |
| E-52 | Secrets Manager entries for bridge Ethereum RPC endpoint and signing key | MISSING | Never hardcode; use SSM/Secrets Manager |

---

## 4. Repo: `orderbook-fe`

All components listed below are 0% / not started.

### 4.1 LMP Dashboard — `src/components/lmp/`

| # | Work Item | Notes |
|---|---|---|
| F-01 | `EpochOverviewPanel` component | Time remaining, estimated PDEX reward, Q-score rank, total pool |
| F-02 | `QScoreGauges` component (3 animated gauges: Depth/Spread, Uptime, makerVolume) | Real-time via WebSocket `lmp/accounts/{address}` feed |
| F-03 | `MarketTierSelector` component | Filters view by Tier 1/2/3; shows maxSpread + minDepth per pair |
| F-04 | `LMPLeaderboard` component (top 20 + self-rank highlight) | Polls `GET /lmp/epochs/{epoch}/leaderboard` |
| F-05 | `VolatilityMultiplierBadge` component | Live badge on pairs with active 2× trigger |
| F-06 | `DMMPanel` component | Active DMM assignments, committed spreads, live uptime % |
| F-07 | `LMPHistoryTab` component | Past epochs: Q-score achieved, rewards earned/claimed, stipends |
| F-08 | `ClaimModal` component | Fetches Merkle proof from `GET /lmp/accounts/{address}/rewards/claimable`; submits `claim_rewards` extrinsic via Polkadot.js |
| F-09 | WebSocket hook: `useLMPLive()` | Subscribes to `lmp/live` feed; provides snapshot data to dashboard |
| F-10 | WebSocket hook: `useAccountQScore(address)` | Subscribes to `lmp/accounts/{address}` feed |

### 4.2 Bridge UI — `src/components/bridge/`

| # | Work Item | Notes |
|---|---|---|
| F-11 | `BridgeDepositFlow` component | Chain select → token → amount → ERC-20 approve → `lock()` → confirmation with estimated credit time |
| F-12 | `BridgeWithdrawFlow` component | Token → destination address → `initiate_withdrawal()` → validator vote progress bar → completion |
| F-13 | `BridgeStatus` component | Live validator signature count for pending deposits; polls `GET /bridge/deposits/{txHash}` |
| F-14 | `BridgeTransactionHistory` component | Deposit + withdrawal history with status (pending / confirmed / failed) |
| F-15 | Ethereum wallet connection (MetaMask / WalletConnect) for ERC-20 approve + lock | Needs `wagmi` or `ethers.js` integration |
| F-16 | Confirmation modal with fee breakdown before any on-chain call | Required by SOW UX rule — never auto-submit |

### 4.3 Shared / API Layer — `src/lib/`

| # | Work Item | Notes |
|---|---|---|
| F-17 | `lmpApi.ts` — typed API client for all `/lmp/*` REST endpoints | |
| F-18 | `bridgeApi.ts` — typed API client for all `/bridge/*` REST endpoints | |
| F-19 | `usePolkadotExtrinsic()` hook — wraps Polkadot.js extrinsic submission with loading/error state | May already exist; check for reuse |

---

## 5. New Repo: `eth-bridge-contract` (Hardhat or Foundry)

All items 0% / not started.

| # | Work Item | Notes |
|---|---|---|
| B-01 | `PolkadexBridge.sol` — `lock(token, amount, polkadexRecipient)` function | Locks ERC-20; emits `DepositLocked(nonce, from, token, amount, recipient)` |
| B-02 | `release(token, amount, recipient, nonce, signatures[])` function | Verifies 2/3 validator sigs; releases locked funds; replay-safe via nonce |
| B-03 | Asset whitelist mapping (`whitelistedTokens`) | Only governance-approved ERC-20s accepted |
| B-04 | Daily bridge volume cap per asset (circuit breaker) | Revert if `dailyVolume[token][day] + amount > cap[token]` |
| B-05 | Emergency pause (OpenZeppelin `Pausable`) | Controlled by Polkadex governance multisig |
| B-06 | Validator set management (add/remove) | Mirrors on-chain validator set; updated via governance |
| B-07 | Comprehensive Hardhat/Foundry test suite | Lock, release, replay protection, pause, cap enforcement |
| B-08 | Deployment scripts for Ethereum Sepolia testnet | |
| B-09 | External smart contract audit | Must complete before mainnet; ~4 weeks; schedule auditor at M1 |

---

## 6. New Service: `lmp-calibration/` (Python)

Lives in `orderbook/lmp-calibration/` or as a standalone repo. All items 0% / not started.

| # | Work Item | Notes |
|---|---|---|
| P-01 | Data ingestion: pull 30-day minute-level orderbook snapshots from Polkadex API | Uses `GET /lmp/pairs` and snapshot history |
| P-02 | Data ingestion: pull 30-day trade data (side, size, price, timestamp) | Uses existing trades API |
| P-03 | High-impact event detection: on-chain governance outcomes, large PDEX unlocks, 30-min price moves > 5% | |
| P-04 | Per-minute orderbook reconstruction from trade data | `OrderBook.reconstruct(trades_minute)` — BUY→Ask, SELL→Bid |
| P-05 | 95th percentile per-minute volume calculation (per pair, per side) | |
| P-06 | Condition 1 check: `MinTickSize > Price × Spread_bps` | Hard floor; governance cannot override downward |
| P-07 | Condition 2 check: `mean_depth_at_spread >= 95th_pctile_daily_volume_per_side_per_minute` | Over 30-day window |
| P-08 | Condition 3 check: ADF stationarity test on depth time-series (p < 0.05) | Use `statsmodels.tsa.stattools.adfuller` |
| P-09 | Condition 4 check: depth at chosen spread >= estimated volume during high-impact events | |
| P-10 | Candidate spread sweep: `MinTickSize_bps` to 200 bps in 5 bps increments; test all 4 conditions | |
| P-11 | Select tightest spread passing all 4 conditions | |
| P-12 | Governance proposal JSON generator: `{pair, current_spread, recommended_spread, evidence}` | Max 10 bps change per cycle |
| P-13 | Monthly scheduled trigger (cron / AWS EventBridge) | CDK stack at E-49 |
| P-14 | Output: signed JSON + PDF report for Polkadex governance forum | |

---

## 7. New Service: `bridge-relayer/` (Rust sidecar)

Lives in `orderbook/bridge-relayer/` or alongside validator node. All items 0% / not started.

| # | Work Item | Notes |
|---|---|---|
| R-01 | Ethereum event listener: subscribe to `DepositLocked` events via JSON-RPC WebSocket | Uses `ethers-rs` |
| R-02 | Submit `register_deposit_vote` to Polkadex chain on each observed deposit | Uses `subxt` |
| R-03 | Polkadex event listener: subscribe to `WithdrawalInitiated` events | |
| R-04 | Co-sign Ethereum `release()` transaction on observed withdrawal | Aggregate signatures off-chain; submit when threshold reached |
| R-05 | Nonce deduplication: skip already-processed deposit nonces | |
| R-06 | Retry logic with exponential backoff on Ethereum or Polkadex RPC failures | |
| R-07 | Config: Ethereum RPC URL, Polkadex WS URL, signing key (from Secrets Manager) | Never hardcode |
| R-08 | Docker image + ECS Fargate task definition | CDK stack at E-48 |

---

## 8. Work Item Count Summary by Repo

| Repo / Service | Modify Existing | New Items | Total |
|---|---|---|---|
| `polkadex-node` (chain) | 16 | 52 | **68** |
| `orderbook` (engine + server + infra) | 14 | 30 | **44** |
| `orderbook-fe` (frontend) | 0 | 19 | **19** |
| `eth-bridge-contract` (new) | 0 | 9 | **9** |
| `lmp-calibration` (new Python service) | 0 | 14 | **14** |
| `bridge-relayer` (new Rust sidecar) | 0 | 8 | **8** |
| **Total** | **30** | **132** | **162** |

---

## 9. Cross-Repo Coordination Points

These items require simultaneous changes in multiple repos. Sequence matters.

| Coordination Point | Repos | Risk if Misaligned |
|---|---|---|
| `LMPOneMinuteReport` struct changes (E-21, E-22) | `orderbook/primitives` → `engine` → `tradesrelayer` → `polkadex-node` (ingress parsing) | Deserialization failure on chain; stid gap |
| `IngressMessages::LMPConfig` tier field (C-61) | `polkadex-node/primitives` → `orderbook/chainfollower` → `engine` | Engine runs with wrong exponents if not updated atomically |
| `submit_lmp_snapshot` new extrinsic (C-21) | `polkadex-node` (define) → `orderbook/aggregators/lmp` (call) | Epoch scores not submitted on-chain |
| Merkle proof in `claim_rewards` (C-22) | `polkadex-node` (verify) → `orderbook/server` (serve proofs) → `orderbook-fe` (fetch + submit) | Claims broken if any layer mismatches |
| Bridge: `initiate_withdrawal` event → relayer → Ethereum release | `polkadex-node` (emit event) → `bridge-relayer` (observe) → `eth-bridge-contract` (execute) | Withdrawals stuck permanently |
| `TradingPairConfig` storage migration (C-27, C-67) | `polkadex-node` only — but runtime upgrade required | Parachain halts if migration not run with `try-runtime` first |

---

## 10. Milestone Assignment

| Milestone | Repos | Key Items |
|---|---|---|
| **M1 — Design** | All | Architecture decisions (Section 11 of original doc); select bridge auditor; agree tier classification; finalise Merkle claim approach |
| **M2 — Core Chain + Engine** | `polkadex-node`, `orderbook/engine` | C-01 to C-08, C-61–C-63 (tiering + enforcement); E-01, E-02, E-09–E-12 (maker volume + uptime); storage migration C-27; C-29/C-30 integration tests |
| **M3 — Bridge + Aggregation + API** | All repos | C-09–C-60 (DMM, volatility, bridge pallet); B-01–B-08 (Eth contract); R-01–R-08 (relayer); E-13–E-20 (epoch worker + Merkle); E-29–E-44 (API + data pipeline); P-01–P-14 (calibration); audit begins |
| **M4 — Frontend + Testnet + Audit Close** | `orderbook-fe`, `polkadex-node`, `orderbook` | F-01–F-19 (all frontend); C-65–C-68 (runtime upgrade); E-45–E-52 (CDK infra); audit findings remediated; full testnet cycle |
| **M5 — Mainnet** | `polkadex-node`, `orderbook`, `orderbook-fe` | Runtime upgrade governance referendum; bridge validator onboarding; LMP activated Tier 3 first |
| **M5+ — Calibration** | `lmp-calibration` | P-01–P-14 first run on real mainnet data; governance proposals submitted |

---

## 11. Open Decisions (Must Resolve at M1)

| # | Decision | Options | Recommendation |
|---|---|---|---|
| D-01 | Reward accounting: separate `pallet-lmp-rewards` vs extend `pallet-ocex` | New pallet = cleaner separation; extend = smaller runtime upgrade | Extend `pallet-ocex` for M2; extract to separate pallet post-launch |
| D-02 | Claim mechanism rollout | Merkle from day 1 vs phased (direct M2 → Merkle M3) | Phased: reduces M2 scope; Merkle path well-tested before claim window opens |
| D-03 | API server language | Add REST to existing Rust server vs new Node.js service | Rust — avoid 2-service operational overhead |
| D-04 | `pallet-liquidity-mining` scope | Fix TODOs within this SOW vs defer | Fix C-35–C-39 in M2; they're small and block prod stability |
| D-05 | Tier classification | Which pairs are Tier 1/2/3? | Must be agreed before C-01 migration is written |
| D-06 | IPFS vs S3 for Merkle leaf data | IPFS = decentralised; S3 = simpler ops | S3 for v1; IPFS post-launch if decentralisation required |
| D-07 | Bridge auditor | Who? When engaged? | Select at M1 kick-off; contract code ready end of M3 |

---

## 12. Items That Must NOT Be Touched

- `pallet-thea` (deprecated)
- `pallet-pdex-migration` (already executed on mainnet)
- Existing pallet call indices — never change after mainnet deployment
- Existing storage layouts — always migrate, never rename in-place
