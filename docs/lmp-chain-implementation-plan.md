# LMP Chain Implementation Plan — `polkadex-node`
**Date:** 2026-06-09 | **Branch target:** `testnet`  
**Based on:** lmp-sow-gap-analysis.md + lmp-implementation-log.md (orderbook M2–M4 complete)

---

## Current State

| What exists | Status |
|---|---|
| `claim_lmp_rewards` (Index 19) — direct, non-Merkle | EXISTS |
| `set_lmp_epoch_config` (Index 20), `start_new_epoch_lmp` (Index 23), `force_submit_snapshot` (Index 24) | EXISTS |
| `TraderMetrics`, `TotalScores`, `LMPConfig`, `LMPEpoch`, `LMPClaimBlk` storage | EXISTS |
| Hardcoded exponents y=0.15, z=5.0 (uptime), 0.85 (volume) in `validator.rs:compute_score` | EXISTS — needs tier-aware lookup |
| `max_spread` / `min_depth` fields in `LMPMarketConfig` — stored but never enforced | EXISTS — needs wiring |
| `StorageVersion` | MISSING — must introduce before first migration |
| `MarketTier` enum, tier field in `LMPMarketConfig`, DMM storage, FeesCollected, Merkle storage | MISSING |

**Unused call index gaps:** 9, 10, 11, 13, 22. Next available: 25.

---

## Constraints

- NEVER change existing call indices (19, 20, 21, 23, 24)
- NEVER change existing storage keys in-place — always migrate
- All amounts in `u128` smallest unit on-chain; `Decimal` only in offchain/engine layer
- `ensure!()` not `assert!()` — panics kill the parachain
- No `.unwrap()` in runtime code
- All new storage must be bounded (`BoundedVec`, `BoundedBTreeMap`)
- All new extrinsics must have benchmarks before runtime upgrade PR

---

## Phase Overview

| Phase | Scope | Unblocks (orderbook) | SOW items |
|---|---|---|---|
| **P1** | Primitives + type expansion | TR-02/TR-03, engine maker_volume | C-61, C-62, C-63 |
| **P2** | Tiering + enforcement | Engine tier-aware exponents, API `/lmp/pairs` | C-01–C-03, C-07, C-08, C-27 |
| **P3** | Fee split + reward pool | — | C-04, C-05, C-06 |
| **P4** | Volatility multiplier | Engine `TriggerVolatilityMultiplier` → chain (FEAT-110) | C-16, C-17, C-18, C-19 |
| **P5** | DMM system | Engine `DmmUptimeTracker` DMM list, API `/lmp/dmm` (FEAT-104) | C-09–C-15, C-31, C-32 |
| **P6** | Merkle snapshot + claim | `EpochAggregator` trigger, claim flow (FEAT-113) | C-20–C-23, C-33, C-34 |
| **P7** | Maker rebate + governance | — | C-24, C-25, C-26 |
| **P8** | `pallet-liquidity-mining` fixes | — | C-35–C-39 |
| **P9** | Offchain state verification | Engine end-to-end integration | C-29, C-30 |
| **P10** | Benchmarks + runtime wiring | Mainnet upgrade | C-28, C-67, C-68 |

---

## Phase 1 — Primitives & Type Expansion

**Files:** `primitives/orderbook/src/lmp.rs`, `primitives/orderbook/src/types.rs`

### P1-1: Add `MarketTier` enum (`primitives/orderbook/src/lmp.rs`)
```rust
#[derive(Encode, Decode, Clone, PartialEq, Eq, Debug, TypeInfo, Default,
         MaxEncodedLen, Copy, PartialOrd, Ord, Serialize, Deserialize, DecodeWithMemTracking)]
pub enum MarketTier { #[default] Tier3, Tier2, Tier1 }
```
Must derive `MaxEncodedLen` and `Copy` because `LMPMarketConfig` derives both and `tier` becomes a field.

### P1-2: Add `tier: MarketTier` to `LMPMarketConfig`
Append as last field so existing SCALE-encoded data decodes cleanly during migration:
```rust
pub struct LMPMarketConfig {
    // ... existing fields unchanged ...
    pub tier: MarketTier,   // new — defaults to Tier3 during migration
}
```

### P1-3: Add `DMMCommitment` struct (`primitives/orderbook/src/lmp.rs`)
```rust
#[derive(Encode, Decode, Clone, Debug, PartialEq, Eq, TypeInfo,
         MaxEncodedLen, Serialize, Deserialize, DecodeWithMemTracking)]
pub struct DMMCommitment<AccountId> {
    pub account: AccountId,
    pub max_spread: u128,       // bps, u128 on-chain
    pub min_depth: u128,        // base asset units
    pub committed_uptime: u8,   // 0–100
    pub stipend: u128,          // PDEX in smallest unit
}
```

### P1-4: Expand `UserActions::OneMinLMPReport` (`primitives/orderbook/src/types.rs`)
**Current:** `OneMinLMPReport(TradingPair, Decimal, BTreeMap<AccountId, Decimal>)`  
**New:**
```rust
OneMinLMPReport(
    TradingPair,
    Decimal,
    #[serde_as(as = "Vec<(_, _)>")] BTreeMap<AccountId, Decimal>,  // q_scores (existing)
    #[serde_as(as = "Vec<(_, _)>")] BTreeMap<AccountId, Decimal>,  // maker_volume (new)
    #[serde_as(as = "Vec<(_, _)>")] BTreeMap<AccountId, bool>,     // uptime_present (new)
),
```

> **Breaking SCALE change** — coordinate with chainfollower/tradesrelayer. Deploy chain first, then orderbook. The engine already populates these fields in `LMPOneMinuteReport`; this wires them through.

### P1 — Unit Tests

Tests live in `primitives/orderbook/src/lmp.rs` under `#[cfg(test)] mod tests {}`. No mock runtime needed — pure Rust.

| Test | What | How |
|---|---|---|
| `market_tier_default_is_tier3` | `MarketTier::default()` returns `Tier3` | `assert_eq!(MarketTier::default(), MarketTier::Tier3)` |
| `market_tier_scale_roundtrip` | All 3 variants survive SCALE encode/decode | `assert_eq!(decode(encode(Tier1)), Tier1)` for each variant |
| `market_tier_ordering` | `Tier1 > Tier2 > Tier3` | `assert!(MarketTier::Tier1 > MarketTier::Tier2)` etc. |
| `lmp_market_config_default_tier_is_tier3` | Default config has `Tier3` | `assert_eq!(LMPMarketConfig { ..Default::default(), tier: MarketTier::default() }.tier, MarketTier::Tier3)` |
| `lmp_market_config_scale_roundtrip_with_tier` | Struct survives SCALE with new `tier` field | Construct with `tier: Tier2`, encode, decode, assert field preserved |
| `dmm_commitment_scale_roundtrip` | `DMMCommitment` survives SCALE | Full struct with all fields; encode → decode → assert all fields equal |
| `dmm_commitment_serde_roundtrip` | `DMMCommitment` survives JSON | `serde_json::from_str(serde_json::to_string(&c).unwrap())` → assert equal |
| `dmm_commitment_uptime_boundary` | `committed_uptime` field accepts 0 and 100 | Construct with 0 and 100; verify no panic or encoding issues |
| `one_min_lmp_report_scale_roundtrip_with_new_fields` | Expanded variant survives SCALE | Construct with non-empty `maker_volume` and `uptime_present` maps; encode → decode → assert both maps preserved |
| `one_min_lmp_report_serde_roundtrip_with_new_fields` | Expanded variant survives JSON | Same struct; JSON round-trip; assert `uptime_present` booleans preserved |
| `one_min_lmp_report_empty_new_fields_roundtrip` | Empty maps in new fields survive | `maker_volume: BTreeMap::new()`, `uptime_present: BTreeMap::new()`; encode → decode |
| `lmp_epoch_config_verify_still_works` | Adding `tier` to `LMPMarketConfig` doesn't break `verify()` | Construct valid config with `tier` set; call `.verify()` → `true` |

---

## Phase 2 — Tiering + Enforcement + Storage Migration

**Files:** `pallets/ocex/src/lib.rs`, `pallets/ocex/src/validator.rs`, `pallets/ocex/src/migrations/`

### P2-1: Introduce `StorageVersion` (`lib.rs`)
Before any migration runs, add to the pallet:
```rust
const STORAGE_VERSION: StorageVersion = StorageVersion::new(1);
#[pallet::pallet]
#[pallet::storage_version(STORAGE_VERSION)]
pub struct Pallet<T>(_);
```
Version 0 = current (no version). Version 1 = after tier field migration.

### P2-2: `set_pair_tier` extrinsic — **Call Index 9**
```rust
#[pallet::call_index(9)]
#[pallet::weight(T::WeightInfo::set_pair_tier())]
pub fn set_pair_tier(origin: OriginFor<T>, pair: TradingPair, tier: MarketTier) -> DispatchResult
```
- `T::GovernanceOrigin::ensure_origin(origin)`
- Update `ExpectedLMPConfig` → `config[pair].tier = tier`
- Also update current epoch's `LMPConfig` if pair exists (governance can re-tier within epoch)
- Emit `MarketTierSet(pair, tier)` event

### P2-3: Tier-aware exponent lookup (`validator.rs:compute_score`)
Replace hardcoded exponents with a lookup by `LMPMarketConfig.tier`:
```rust
let (y, uptime_exp, z) = match config.tier {
    MarketTier::Tier1 => (0.15f64, 5.0f64, 0.85f64),
    MarketTier::Tier2 => (0.15f64, 5.0f64, 0.85f64),  // confirm with product
    MarketTier::Tier3 => (0.15f64, 5.0f64, 0.85f64),  // confirm with product
};
```
> **Open:** Product must confirm per-tier exponent values before this ships. Defaults match current hardcoded values to be safe.

### P2-4: Enforce `max_spread` and `min_depth` filters (`validator.rs`)
In `compute_score`, before computing Q-score, fetch current spread and depth from offchain state and apply:
```rust
// C-07: max_spread filter
if current_spread > config.max_spread { return Ok(Decimal::zero()); }
// C-08: min_depth filter
if current_depth < config.min_depth { return Ok(Decimal::zero()); }
```

### P2 — Unit Tests

Tests live in `pallets/ocex/src/tests.rs`. Use `new_test_ext().execute_with(|| { ... })` from `mock.rs`. The mock runtime already has `OCEX`, `Balances`, `Assets`, `Timestamp` wired up.

| Test | What | How |
|---|---|---|
| `set_pair_tier_requires_governance_origin` | Non-root/governance origin rejected | `assert_noop!(OCEX::set_pair_tier(RuntimeOrigin::signed(alice), pair, Tier1), BadOrigin)` |
| `set_pair_tier_stores_in_expected_lmp_config` | Tier written to `ExpectedLMPConfig` | Call `set_pair_tier` via `RuntimeOrigin::root()`; read `ExpectedLMPConfig`; assert `config[pair].tier == Tier1` |
| `set_pair_tier_updates_current_epoch_lmp_config` | Current epoch's `LMPConfig` also updated | Pre-populate `LMPConfig[0]` with pair; call `set_pair_tier`; assert tier updated in `LMPConfig[0]` |
| `set_pair_tier_emits_market_tier_set_event` | `MarketTierSet` event emitted | After call, `assert_last_event::<Test>(Event::MarketTierSet { pair, tier: Tier1 }.into())` |
| `set_pair_tier_for_unknown_pair_is_noop` | Pair not in config → no-op (or graceful error, confirm behaviour) | Call with a pair not in `ExpectedLMPConfig`; assert storage unchanged |
| `compute_score_returns_zero_when_spread_exceeds_max` | max_spread filter zeroes score | Set `LMPMarketConfig.max_spread` to 5 bps; inject offchain state with spread = 10 bps; call `compute_score`; assert `Ok(Decimal::zero())` |
| `compute_score_returns_zero_when_depth_below_min` | min_depth filter zeroes score | Set `min_depth` to 100; inject state with depth = 50; assert `Ok(Decimal::zero())` |
| `compute_score_passes_when_spread_and_depth_valid` | Filters pass → non-zero score returned | Spread within max, depth above min, positive maker_volume and uptime; assert score > 0 |
| `migration_v0_to_v1_sets_tier3_default` | Migration adds `tier: Tier3` to all existing `LMPMarketConfig` entries | Insert `LMPConfig` and `ExpectedLMPConfig` using old struct format (without tier); run migration; read back and assert `tier == Tier3` |
| `migration_v1_is_idempotent` | Running migration twice does not double-apply | Run migration; run again; assert `on_chain_storage_version() == 1` and storage unchanged |

### P2-5: Storage migration — add `tier` to `LMPMarketConfig` (`pallets/ocex/src/migrations/v1.rs`)
```rust
pub fn migrate<T: Config>() -> Weight {
    let on_chain_version = Pallet::<T>::on_chain_storage_version();
    if on_chain_version >= 1 { return Weight::zero(); }
    // Migrate LMPConfig (all epochs) + ExpectedLMPConfig:
    // For each BTreeMap<TradingPair, LMPMarketConfig> value, add tier: MarketTier::Tier3 (safe default)
    StorageVersion::new(1).put::<Pallet<T>>();
}
```
- Wire into `runtimes/mainnet/src/lib.rs` under `Executive` migrations list (C-67)
- Test with `scripts/try-runtime-on-runtime-upgrade.sh` before PR

---

## Phase 3 — Fee Split & Reward Pool

**Files:** `pallets/ocex/src/lib.rs`, `pallets/ocex/src/session.rs`

### P3-1: `FeesCollected` storage
```rust
#[pallet::storage]
pub type FeesCollected<T: Config> = StorageDoubleMap<
    _, Blake2_128Concat, u16,          // epoch
    Blake2_128Concat, TradingPair,     // pair
    BalanceOf<T>, ValueQuery,
>;
```

### P3-2: Populate `FeesCollected` on every taker fill (`lib.rs`, trade execution path)
In the trade processing path (wherever taker fees are collected), accumulate:
```rust
<FeesCollected<T>>::mutate(current_epoch, &pair, |acc| {
    *acc = acc.saturating_add(taker_fee);
});
```

### P3-3: Fee split in `on_initialize` at epoch boundary (`session.rs:start_new_epoch`)
At epoch boundary, after transitioning the epoch:
```rust
// Transfer 25% of each pair's collected fees to the pallet's reward account
for (pair, fees) in <FeesCollected<T>>::drain_prefix(current_epoch) {
    let lmp_cut = fees / 4;  // 25%
    T::Currency::transfer(&Self::account_id(), &Self::rewards_account_id(), lmp_cut, ...)?;
}
```
Reset is implicit via `drain_prefix`.

> **Note:** `T::GovernanceOrigin`-controlled `set_fee_distribution` (Index 21) already exists — check whether fee split percentage should come from `FeeDistributionConfig` or be a new config field. Prefer reusing the existing config.

### P3 — Unit Tests

| Test | What | How |
|---|---|---|
| `fees_collected_increments_on_taker_fill` | Each taker fill accumulates into `FeesCollected[epoch][pair]` | Execute a trade that produces a taker fee; read `FeesCollected`; assert > 0 |
| `fees_collected_independent_per_pair` | Fees for pair A do not affect pair B | Two pairs, fills on each; assert storage keyed independently |
| `epoch_boundary_transfers_25pct_to_reward_account` | 25% of collected fees moved to rewards account at epoch end | Fund `FeesCollected[0][pair] = 1_000_000`; trigger `start_new_epoch`; assert reward account balance increased by 250_000 |
| `fees_collected_cleared_after_epoch_boundary` | `FeesCollected` for old epoch drained after transition | After `start_new_epoch`, assert `FeesCollected[0][pair] == 0` |
| `no_fees_no_transfer` | Zero fees → no transfer, no panic | `FeesCollected[0][pair] == 0`; trigger epoch end; assert reward account balance unchanged |

---

## Phase 4 — Volatility Multiplier (FEAT-110)

**Files:** `pallets/ocex/src/lib.rs`

### P4-1: `VolatilityTriggerCount` storage
```rust
#[pallet::storage]
pub type VolatilityTriggerCount<T: Config> = StorageNMap<
    _,
    (NMapKey<Blake2_128Concat, u16>, NMapKey<Blake2_128Concat, TradingPair>, NMapKey<Blake2_128Concat, u32>),
    // (epoch, pair, day_index) → count
    u8, ValueQuery,
>;
```
`day_index = block_number / BLOCKS_PER_DAY`

### P4-2: `trigger_volatility_multiplier` extrinsic — **Call Index 25**
```rust
#[pallet::call_index(25)]
pub fn trigger_volatility_multiplier(origin: OriginFor<T>, pair: TradingPair) -> DispatchResult
```
- Allowed callers: `T::OrderbookOperatorOrigin` or `T::GovernanceOrigin`
- Get current `day_index`; read `VolatilityTriggerCount[epoch][pair][day]`
- `ensure!(count < 6, Error::<T>::DailyVolatilityCapReached)`
- Increment count
- Emit `VolatilityMultiplierTriggered(pair, snapshot_id)` event (C-19)

> **Note:** The multiplier itself (2×) is applied in the engine per-snapshot — the chain just records the trigger and emits the event. The engine's `handle_command(TriggerVolatilityMultiplier)` stub activates once this event is parseable by chainfollower.

### P4 — Unit Tests

| Test | What | How |
|---|---|---|
| `trigger_volatility_requires_operator_or_governance` | Random signed origin rejected | `assert_noop!(OCEX::trigger_volatility_multiplier(RuntimeOrigin::signed(alice), pair), BadOrigin)` |
| `trigger_volatility_increments_trigger_count` | Count in `VolatilityTriggerCount` goes from 0 to 1 | Call via operator origin; read `VolatilityTriggerCount[epoch][pair][day]`; assert == 1 |
| `trigger_volatility_capped_at_6_per_day` | 7th call on same day fails | Call 6 times (assert_ok each); 7th call: `assert_noop!(..., Error::<Test>::DailyVolatilityCapReached)` |
| `trigger_volatility_resets_across_days` | Counter per day, not per epoch | Call 6 times on day 0; advance block to day 1; first call on day 1 succeeds |
| `trigger_volatility_sets_volatility_active` | `VolatilityActive[pair]` becomes `true` | Call trigger; read storage; assert `true` |
| `trigger_volatility_emits_event` | `VolatilityMultiplierTriggered` event emitted | `assert_last_event::<Test>(Event::VolatilityMultiplierTriggered { pair, epoch }.into())` |
| `volatility_active_cleared_at_epoch_boundary` | `VolatilityActive` reset to `false` when new epoch starts | Set `true`; trigger `start_new_epoch`; assert `VolatilityActive[pair] == false` |

### P4-3: `VolatilityActive` storage (for API `/lmp/accounts/{addr}/qscore` volatility flag)
```rust
#[pallet::storage]
pub type VolatilityActive<T: Config> = StorageMap<
    _, Blake2_128Concat, TradingPair, bool, ValueQuery,
>;
```
Set `true` on trigger, clear at epoch boundary.

---

## Phase 5 — DMM System (FEAT-104)

**Files:** `pallets/ocex/src/lib.rs`, `pallets/ocex/rpc/`, `pallets/ocex/src/lmp.rs`

### P5-1: Config constants
```rust
#[pallet::constant]
type MaxDMMsPerPair: Get<u32>;  // set to 10 in runtime
```

### P5-2: `DMMRegistry` storage
```rust
#[pallet::storage]
pub type DMMRegistry<T: Config> = StorageDoubleMap<
    _, Blake2_128Concat, u16,       // epoch
    Blake2_128Concat, TradingPair,
    BoundedVec<DMMCommitment<T::AccountId>, T::MaxDMMsPerPair>,
    ValueQuery,
>;
```

### P5-3: `DMMPerformance` storage
```rust
#[pallet::storage]
pub type DMMPerformance<T: Config> = StorageNMap<
    _,
    (NMapKey<Blake2_128Concat, u16>, NMapKey<Blake2_128Concat, TradingPair>, NMapKey<Blake2_128Concat, T::AccountId>),
    u8, OptionQuery,  // uptime percentage 0–100
>;
```

### P5-4: `register_dmm` extrinsic — **Call Index 10**
```rust
#[pallet::call_index(10)]
pub fn register_dmm(origin, epoch: u16, pair: TradingPair, max_spread: u128, min_depth: u128, committed_uptime: u8, stipend: u128) -> DispatchResult
```
- `ensure_signed`; only callable before epoch start (current epoch < target epoch)
- Validate: `committed_uptime <= 100`, pair exists in LMPConfig
- Append `DMMCommitment` to `DMMRegistry[epoch][pair]` (bounded)
- Emit `DMMRegistered(epoch, pair, account)` event

### P5-5: `confirm_dmm_selection` extrinsic — **Call Index 11**
```rust
#[pallet::call_index(11)]
pub fn confirm_dmm_selection(origin, epoch: u16, pair: TradingPair, accounts: BoundedVec<T::AccountId, T::MaxDMMsPerPair>) -> DispatchResult
```
- `T::GovernanceOrigin`
- Filter `DMMRegistry[epoch][pair]` to only retain entries whose account is in `accounts`
- Emit `DMMSelected(epoch, pair, accounts)` event

### P5-6: `submit_dmm_performance` extrinsic — **Call Index 13**
```rust
#[pallet::call_index(13)]
pub fn submit_dmm_performance(origin, epoch: u16, pair: TradingPair, performance: BoundedVec<(T::AccountId, u8), T::MaxDMMsPerPair>) -> DispatchResult
```
- `T::OrderbookOperatorOrigin`
- Called at epoch end by engine operator
- Writes each `(account, uptime_pct)` to `DMMPerformance[epoch][pair][account]`

### P5-7: `claim_dmm_stipend` extrinsic — **Call Index 22**
```rust
#[pallet::call_index(22)]
pub fn claim_dmm_stipend(origin, epoch: u16, pair: TradingPair) -> DispatchResult
```
- `ensure_signed`
- Read `DMMPerformance[epoch][pair][account]`; read `DMMRegistry[epoch][pair]` to find commitment
- `ensure!(actual_uptime >= committed_uptime, Error::<T>::DMMUptimeNotMet)`
- Transfer `stipend` from pallet account to caller
- Emit `DMMStipendClaimed(epoch, pair, account)` event

### P5-8: Reserve DMM stipend in `on_initialize` at epoch start (`session.rs`)
At each new epoch start:
```rust
for commitment in DMMRegistry::get(new_epoch, pair) {
    T::Currency::transfer(&T::TreasuryAccount::get(), &Self::account_id(), commitment.stipend, ...)?;
}
```

### P5 — Unit Tests

| Test | What | How |
|---|---|---|
| `register_dmm_stores_commitment` | Commitment appears in `DMMRegistry[epoch][pair]` | Call `register_dmm` for a future epoch; read storage; assert len == 1, fields match |
| `register_dmm_fails_if_epoch_already_started` | Can't register DMM for current or past epoch | Call with `epoch == current_epoch`; `assert_noop!(..., Error::<Test>::EpochAlreadyStarted)` |
| `register_dmm_fails_when_uptime_exceeds_100` | `committed_uptime > 100` rejected | `assert_noop!(register_dmm(..., committed_uptime: 101, ...), Error::<Test>::InvalidUptimeCommitment)` |
| `register_dmm_emits_dmm_registered_event` | `DMMRegistered` event emitted | `assert_last_event::<Test>(Event::DMMRegistered { epoch, pair, account }.into())` |
| `register_dmm_bounded_storage` | Can't exceed `MaxDMMsPerPair` commitments | Register `MaxDMMsPerPair` DMMs (assert_ok each); one more: `assert_noop!(..., Error::<Test>::TooManyDMMs)` |
| `confirm_dmm_selection_requires_governance` | Signed origin rejected | `assert_noop!(OCEX::confirm_dmm_selection(RuntimeOrigin::signed(alice), ...), BadOrigin)` |
| `confirm_dmm_selection_filters_registry` | Only listed accounts remain in registry | Register 3 DMMs; confirm with 2 of them; read `DMMRegistry`; assert len == 2, correct accounts |
| `confirm_dmm_selection_emits_event` | `DMMSelected` event emitted | `assert_last_event::<Test>(Event::DMMSelected { epoch, pair, accounts }.into())` |
| `submit_dmm_performance_requires_operator` | Signed non-operator rejected | `assert_noop!(..., BadOrigin)` |
| `submit_dmm_performance_stores_uptime` | `DMMPerformance[epoch][pair][account]` written correctly | Call with vec of (account, 85); read storage; assert == Some(85) |
| `claim_dmm_stipend_fails_if_uptime_not_met` | Actual < committed → rejected | Set `committed_uptime = 90`, `DMMPerformance = 80`; `assert_noop!(..., Error::<Test>::DMMUptimeNotMet)` |
| `claim_dmm_stipend_pays_when_uptime_met` | Actual >= committed → stipend transferred | Set `committed_uptime = 80`, `DMMPerformance = 85`; assert_ok; assert caller balance increased by stipend |
| `claim_dmm_stipend_emits_event` | `DMMStipendClaimed` event emitted | After successful claim: `assert_last_event::<Test>(Event::DMMStipendClaimed { ... }.into())` |
| `dmm_stipend_reserved_at_epoch_start` | Treasury → pallet transfer happens in `on_initialize` | Fund treasury; trigger epoch start; assert pallet account balance increased by sum of all registered stipends |

### P5-9: `get_dmm_status` RPC (`pallets/ocex/rpc/`)
```
get_dmm_status(epoch: u16, pair: TradingPair) -> DmmStatusResult {
    registry: Vec<DMMCommitment>,
    performance: Vec<(AccountId, u8)>,
}
```

### P5-10: Offchain DMM uptime tracking (`pallets/ocex/src/lmp.rs` — C-31)
Add offchain storage key `dmm_uptime_{epoch}_{pair}_{account}` updated each snapshot cycle. This is the data source for `submit_dmm_performance`. The engine's `DmmUptimeTracker` (already implemented) feeds this; confirm the offchain storage write path.

---

## Phase 6 — Merkle Snapshot & Claim (FEAT-113)

**Files:** `pallets/ocex/src/lib.rs`, `pallets/ocex/rpc/`

### P6-1: `LMPMerkleRoot` storage
```rust
#[pallet::storage]
pub type LMPMerkleRoot<T: Config> = StorageDoubleMap<
    _, Blake2_128Concat, u16,       // epoch
    Blake2_128Concat, TradingPair,
    H256, OptionQuery,
>;
```

### P6-2: `RewardsClaimed` storage (double-claim guard)
```rust
#[pallet::storage]
pub type MerkleRewardsClaimed<T: Config> = StorageDoubleMap<
    _, Blake2_128Concat, T::AccountId,
    Blake2_128Concat, u16,          // epoch
    BalanceOf<T>, ValueQuery,
>;
```
Use a different name from the existing claim tracking in `TraderMetrics` to avoid collision.

### P6-3: `submit_lmp_snapshot` extrinsic — **Call Index 26**
```rust
#[pallet::call_index(26)]
pub fn submit_lmp_snapshot(
    origin, epoch: u16, pair: TradingPair,
    merkle_root: H256,
) -> DispatchResult
```
- `T::OrderbookOperatorOrigin` (threshold-voted by validators — same pattern as `submit_snapshot` / `force_submit_snapshot`)
- Write `LMPMerkleRoot[epoch][pair] = merkle_root`
- Set `LMPClaimBlk[epoch]` = `current_block + claim_safety_period` (reuse existing storage)
- Emit `LMPMerkleRootSubmitted(epoch, pair, merkle_root)` event

> **Note:** Leaf format must match `EpochAggregator` in orderbook: `Blake2b256(account_bytes ++ epoch_le_bytes ++ reward_str_bytes)`. Coordinate with engine team before deploying.

### P6-4: `claim_rewards` (Merkle-based) extrinsic — **Call Index 27**
```rust
#[pallet::call_index(27)]
pub fn claim_rewards_merkle(
    origin,
    epoch: u16,
    pair: TradingPair,
    amount: BalanceOf<T>,
    proof: BoundedVec<H256, ConstU32<32>>,
) -> DispatchResult
```
- `ensure_signed`
- Verify claim safety period: `ensure!(current_block >= LMPClaimBlk[epoch])`
- `ensure!(MerkleRewardsClaimed[account][epoch] == 0, Error::<T>::RewardAlreadyClaimed)`
- Compute leaf: `Blake2b256(account ++ epoch ++ amount)`
- Verify Merkle proof against `LMPMerkleRoot[epoch][pair]`
- Transfer `amount` from pallet rewards account to `account`
- Set `MerkleRewardsClaimed[account][epoch] = amount`
- Emit `RewardClaimed(account, epoch, pair, amount)` event

> The existing `claim_lmp_rewards` (Index 19) remains untouched for the current epoch's direct claim. This Merkle version activates for epochs after the aggregator goes live.

### P6 — Unit Tests

For Merkle proof tests, compute expected leaf hashes inline using `sp_io::hashing::blake2_256`.

| Test | What | How |
|---|---|---|
| `submit_lmp_snapshot_requires_operator` | Signed non-operator rejected | `assert_noop!(OCEX::submit_lmp_snapshot(RuntimeOrigin::signed(alice), ...), BadOrigin)` |
| `submit_lmp_snapshot_stores_merkle_root` | `LMPMerkleRoot[epoch][pair]` written | Call via operator; read storage; assert == `Some(root)` |
| `submit_lmp_snapshot_sets_claim_block` | `LMPClaimBlk[epoch]` set to `current_block + claim_safety_period` | Submit at block 100, `claim_safety_period = 50400`; assert `LMPClaimBlk[epoch] == 50500` |
| `submit_lmp_snapshot_emits_event` | `LMPMerkleRootSubmitted` event emitted | `assert_last_event::<Test>(Event::LMPMerkleRootSubmitted { epoch, pair, root }.into())` |
| `claim_rewards_merkle_fails_before_safety_period` | Claim before `LMPClaimBlk` rejected | Submit snapshot at block 1; try claim at block 1; `assert_noop!(..., Error::<Test>::RewardsNotReady)` |
| `claim_rewards_merkle_single_leaf_valid_proof` | Single-leaf tree: empty proof, root = leaf hash | Build leaf for alice; set root = leaf_hash; claim with `proof = vec![]`; assert_ok; assert balance increased |
| `claim_rewards_merkle_two_leaf_valid_proof` | Both accounts can claim with correct sibling proof | Build 2-leaf tree; claim alice with bob's hash as proof; claim bob with alice's hash; both assert_ok |
| `claim_rewards_merkle_fails_with_wrong_proof` | Tampered proof rejected | Flip one byte in proof hash; `assert_noop!(..., Error::<Test>::InvalidMerkleProof)` |
| `claim_rewards_merkle_fails_with_wrong_amount` | Correct proof, wrong amount rejected | Claim with `amount + 1`; assert_noop — leaf hash won't match |
| `claim_rewards_merkle_prevents_double_claim` | Second claim on same (account, epoch) rejected | First claim: assert_ok; second claim: `assert_noop!(..., Error::<Test>::RewardAlreadyClaimed)` |
| `claim_rewards_merkle_fails_for_unknown_epoch` | No root stored → rejected | `assert_noop!(..., Error::<Test>::LMPConfigNotFound)` (or appropriate error) |

### P6-5: `get_merkle_proof` RPC (`pallets/ocex/rpc/`)
```
get_merkle_proof(account: AccountId, epoch: u16, pair: TradingPair) -> Option<MerkleProofData>
```
Reads `LMPMerkleRoot` + queries PostgreSQL (via server API) for the proof path. Chain RPC returns the root; proof path comes from the orderbook server's `GET /lmp/accounts/{address}/rewards/claimable`.

### P6-6: `get_volatility_trigger_count` RPC (`pallets/ocex/rpc/`)
```
get_volatility_trigger_count(pair: TradingPair, day: u32) -> u8
```
Reads `VolatilityTriggerCount[current_epoch][pair][day]`.

---

## Phase 7 — Maker Rebate + Governance Extrinsics + Events

**Files:** `pallets/ocex/src/lib.rs`

### P7-1: Maker rebate logic — **Call Index 28** (indirectly, applied per-fill)
For Tier 1 pairs, on each fill where maker order is within 5 bps of BBO:
- Compute rebate: `0.01%` to `0.03%` of fill notional (exact rate from `LMPMarketConfig.tier`)
- Pay from taker fee pool immediately (before epoch boundary)
- Emit `MakerRebatePaid(maker, pair, rebate_amount)` event

> This fires per-fill, not per-epoch. Needs careful review of the trade execution path to avoid affecting gas per-block predictably.

### P7-2: `suspend_lmp_rewards` extrinsic — **Call Index 28**
```rust
#[pallet::call_index(28)]
pub fn suspend_lmp_rewards(origin, pair: TradingPair) -> DispatchResult
```
- `T::GovernanceOrigin`
- Mark pair in `ExpectedLMPConfig` and current `LMPConfig` as suspended (add `suspended: bool` to `LMPMarketConfig`, or remove from the config map)

### P7-3: `demote_pair_tier` extrinsic — **Call Index 29**
```rust
#[pallet::call_index(29)]
pub fn demote_pair_tier(origin, pair: TradingPair, new_tier: MarketTier) -> DispatchResult
```
- `T::GovernanceOrigin`
- Same as `set_pair_tier` but semantically a demotion; both can share implementation, or `set_pair_tier` can cover this

### P7-4: All missing events (C-24)
Add to `#[pallet::event]`:
- `DMMRegistered { epoch: u16, pair: TradingPair, account: T::AccountId }`
- `DMMSelected { epoch: u16, pair: TradingPair, accounts: Vec<T::AccountId> }`
- `DMMStipendClaimed { epoch: u16, pair: TradingPair, account: T::AccountId, amount: BalanceOf<T> }`
- `RewardClaimed { account: T::AccountId, epoch: u16, pair: TradingPair, amount: BalanceOf<T> }`
- `VolatilityMultiplierTriggered { pair: TradingPair, epoch: u16 }`
- `MarketTierSet { pair: TradingPair, tier: MarketTier }`
- `LMPMerkleRootSubmitted { epoch: u16, pair: TradingPair, root: H256 }`
- `MakerRebatePaid { maker: T::AccountId, pair: TradingPair, amount: BalanceOf<T> }`

### P7 — Unit Tests

| Test | What | How |
|---|---|---|
| `suspend_lmp_rewards_requires_governance` | Signed origin rejected | `assert_noop!(OCEX::suspend_lmp_rewards(RuntimeOrigin::signed(alice), pair), BadOrigin)` |
| `suspend_lmp_rewards_removes_pair_from_config` | Pair suspended in both `LMPConfig` and `ExpectedLMPConfig` | Pre-populate; call; assert pair either removed or flagged suspended |
| `demote_pair_tier_requires_governance` | Signed origin rejected | `assert_noop!(OCEX::demote_pair_tier(RuntimeOrigin::signed(alice), pair, Tier2), BadOrigin)` |
| `demote_pair_tier_updates_tier` | Tier downgraded in `ExpectedLMPConfig` | Pre-set `Tier1`; call demote to `Tier2`; assert `ExpectedLMPConfig.config[pair].tier == Tier2` |
| `maker_rebate_paid_for_tier1_fill_within_5bps` | Rebate emitted on qualifying Tier1 fill | Execute a Tier1 fill where maker price is within 5 bps of BBO; assert `MakerRebatePaid` event |
| `maker_rebate_not_paid_for_tier2_fill` | No rebate for non-Tier1 fill | Same fill on Tier2 pair; assert no `MakerRebatePaid` event |
| `maker_rebate_not_paid_when_spread_exceeds_5bps` | Fill outside 5 bps BBO → no rebate | Tier1 pair, maker price 10 bps from BBO; assert no rebate event |

---

## Phase 8 — `pallet-liquidity-mining` Fixes

**File:** `pallets/liquidity-mining/src/lib.rs`, `pallets/liquidity-mining/src/callback.rs`

| Item | Location | Fix |
|---|---|---|
| C-35: Unsigned tx validation too permissive | `lib.rs:297` | Add proper checks: validate epoch, nonce, account match |
| C-36: Global `SnapshotFlag` → per-pool | `lib.rs:429, 479` | Replace `StorageValue<bool>` with `StorageMap<PoolId, bool>` |
| C-37: Parameterize weights by request count | `lib.rs:649` | Pass `requests.len()` to weight calculation |
| C-38: Emit events on forced pool closure | `lib.rs:744` | Add `PoolForceClosed { pool_id }` event and emit it |
| C-39: `base_freed`/`quote_freed` in `force_close_pool` | `callback.rs:185` | Return freed amounts to the pool initiator account |

> C-36 (per-pool SnapshotFlag) requires a storage migration if the existing flag is on-chain. Check `on_chain_storage_version()` for this pallet too.

### P8 — Unit Tests

Tests live in `pallets/liquidity-mining/src/tests.rs` (or alongside existing tests in `lib.rs`).

| Test | What | How |
|---|---|---|
| `unsigned_tx_validation_rejects_wrong_epoch` | Unsigned tx with mismatched epoch rejected | Construct unsigned tx with epoch mismatch; call `validate_unsigned`; assert `InvalidTransaction` |
| `unsigned_tx_validation_rejects_wrong_nonce` | Wrong nonce rejected | Same pattern with bad nonce |
| `snapshot_flag_independent_per_pool` | Setting flag for pool A does not affect pool B | Set `SnapshotFlag[pool_a] = true`; assert `SnapshotFlag[pool_b] == false` |
| `snapshot_flag_cleared_per_pool` | Clearing pool A flag does not clear pool B | Set both; clear A; assert B still set |
| `force_close_pool_emits_pool_force_closed_event` | `PoolForceClosed` event emitted on forced closure | Call `force_close_pool`; `assert_last_event::<Test>(Event::PoolForceClosed { pool_id }.into())` |
| `force_close_pool_returns_freed_amounts_to_initiator` | `base_freed` and `quote_freed` returned to pool initiator | Fund pool; force close; assert initiator account balance increased by freed amounts |

---

## Phase 9 — Offchain State Verification

**File:** `pallets/ocex/src/lmp.rs`, integration tests

### P9-1: Verify `update_maker_volume_by_main_account` end-to-end (C-29)
Write an integration test:
- Place an order from `main_account`
- Execute a fill (trade)
- Verify `StateChanges` contains the maker volume update
- Verify the chain ingress parser stores it in offchain storage at the expected key

### P9-2: Verify `store_q_score_and_uptime` uptime count (C-30)
Write a test with a simulated epoch of N snapshots, M of which have non-zero scores for an account. Assert uptime count = M.

### P9 — Integration Tests

Tests live in `pallets/ocex/src/integration_tests.rs`. Use `new_test_ext()` with `ext.persist_offchain_overlay()` and `register_offchain_ext(&mut ext)`. Pattern mirrors `test_run_on_chain_validation_trades_happy_path`.

| Test | What | How |
|---|---|---|
| `maker_volume_stored_in_offchain_state_after_trade` | `update_maker_volume_by_main_account` called end-to-end | Place limit order; execute fill via `push_trade_user_actions`; call `run_on_chain_validation`; read offchain storage at `get_maker_volume_by_main_account_key(epoch, pair, &main)`; assert > 0 |
| `maker_volume_correctly_attributed_to_main_account` | Volume attributed to main, not proxy | Fill between proxy accounts; assert maker volume stored under main account key, not proxy key |
| `uptime_count_increments_per_non_zero_score_snapshot` | `store_q_score_and_uptime` counts only snapshots with non-zero score | Push N `OneMinLMPReport` actions where M have non-zero scores; run validation; read uptime at `get_q_score_uptime_by_main_account(epoch, pair, &main)`; assert uptime == M |
| `uptime_count_not_incremented_for_zero_score_snapshot` | Zero-score snapshots don't count toward uptime | Push one report with score = 0 for account; assert uptime remains 0 |

---

## Phase 10 — Benchmarks + Runtime Wiring

### P10-1: Benchmarks (`pallets/ocex/src/benchmarking.rs`) — C-28
Benchmark every new extrinsic (worst-case inputs):
- `set_pair_tier` — `MaxDMMsPerPair` pairs
- `register_dmm` — BoundedVec at max capacity
- `confirm_dmm_selection` — max accounts
- `submit_dmm_performance` — max entries
- `claim_dmm_stipend` — straightforward
- `trigger_volatility_multiplier` — `VolatilityTriggerCount` at max (5)
- `submit_lmp_snapshot` — single entry (root is fixed size)
- `claim_rewards_merkle` — proof depth 32 (max)
- `suspend_lmp_rewards`, `demote_pair_tier` — straightforward

Run: `scripts/benchmark.sh` → regenerate `weights.rs`

### P10-2: Runtime wiring (`runtimes/mainnet/src/lib.rs`) — C-65, C-67, C-68
1. Add V0→V1 migration to `Executive` migrations list:
   ```rust
   type Migrations = (pallet_ocex::migrations::v1::Migration<Runtime>,);
   ```
2. Set `MaxDMMsPerPair = 10` in `impl pallet_ocex::Config for Runtime`
3. Prepare Wasm artifact + governance referendum

### P10-3: try-runtime validation
```bash
./scripts/try-runtime-on-runtime-upgrade.sh
```
Must pass before governance referendum is submitted.

---

## Call Index Assignment Summary

| Index | Extrinsic | Phase |
|---|---|---|
| 9 | `set_pair_tier` | P2 |
| 10 | `register_dmm` | P5 |
| 11 | `confirm_dmm_selection` | P5 |
| 13 | `submit_dmm_performance` | P5 |
| 22 | `claim_dmm_stipend` | P5 |
| 25 | `trigger_volatility_multiplier` | P4 |
| 26 | `submit_lmp_snapshot` | P6 |
| 27 | `claim_rewards_merkle` | P6 |
| 28 | `suspend_lmp_rewards` | P7 |
| 29 | `demote_pair_tier` | P7 |

---

## Cross-Repo Sequencing (Critical)

| Chain ships | Then orderbook can activate |
|---|---|
| P1-4 (`UserActions::OneMinLMPReport` field expansion) | TR-02: `maker_volume` flows to chain ingress |
| P2 (tier field + `LMPEpochConfig` RPC) | `GET /lmp/pairs` returns real data |
| P4 (`VolatilityActive` storage) | `/qscore` endpoint shows volatility flag |
| P5 (`DMMRegistry` storage) | `DmmUptimeTracker.update_dmm_list()` activates |
| P6 (`submit_lmp_snapshot` extrinsic) | `EpochAggregator` trigger in `engine.rs` activates |
| P6 (Merkle claim) | `GET /lmp/accounts/{addr}/rewards/claimable` returns real proofs |

---

## Open Decisions (Resolve Before P2)

| # | Decision | Recommendation |
|---|---|---|
| D-05 | Which pairs are Tier 1/2/3? | Must be agreed before writing migration default tier |
| D-01 | New `pallet-lmp-rewards` vs extend pallet-ocex | Extend pallet-ocex (Option B) — confirmed above |
| D-02 | Direct vs Merkle claim rollout | Phased: existing `claim_lmp_rewards` (Index 19) for current epoch; Merkle (Index 27) for M3+ epochs |
| P2-3 | Per-tier exponent values | Confirm with product; defaults = current hardcoded values |
| P7-1 | Maker rebate rate per tier | Confirm: 0.01% Tier3, 0.02% Tier2, 0.03% Tier1? |

---

## Test Count Summary

| Phase | Test file | Tests |
|---|---|---|
| P1 — Primitives | `primitives/orderbook/src/lmp.rs` (inline `#[cfg(test)]`) | 12 |
| P2 — Tiering | `pallets/ocex/src/tests.rs` | 10 |
| P3 — Fee split | `pallets/ocex/src/tests.rs` | 5 |
| P4 — Volatility | `pallets/ocex/src/tests.rs` | 7 |
| P5 — DMM system | `pallets/ocex/src/tests.rs` | 14 |
| P6 — Merkle claim | `pallets/ocex/src/tests.rs` | 11 |
| P7 — Rebate + governance | `pallets/ocex/src/tests.rs` | 7 |
| P8 — LMP pallet fixes | `pallets/liquidity-mining/src/tests.rs` | 6 |
| P9 — Offchain integration | `pallets/ocex/src/integration_tests.rs` | 4 |
| **Total** | | **76** |

Run all pallet tests: `cargo test -p pallet-ocex`  
Run primitive tests: `cargo test -p orderbook-primitives`  
Run LMP pallet tests: `cargo test -p pallet-lmp`

---

## Items Explicitly Out of Scope

### Bridge (C-46–C-64 bridge items) — CONFIRMED DELIVERED by Hyperbridge

The custom bridge work package is fully superseded. The chain already ships:

| Pallet | Index | Delivers |
|---|---|---|
| `pallet_ismp` | 68 | ISMP protocol layer; `AdminOrigin = EnsureRootOrHalfCouncil`; Coprocessor = Kusama 4009 |
| `ismp_grandpa` | 69 | Grandpa consensus client for cross-chain finality |
| `pallet_hyper_fungible_token` | 72 | Token lock/mint/burn across EVM ↔ Polkadex; replaces C-46–C-60 entirely |

Asset whitelisting (C-50 equivalent) is owned by `pallets/xcm-helper/`:
- `WhitelistedTokens` storage — governs which assets are bridgeable
- `whitelist_token` / `remove_whitelisted_token` extrinsics — governance-controlled
- `ParachainAssets` storage — maps `polkadex_primitives::AssetId` ↔ XCM `AssetId`

**C-64 (`BridgeAsset` primitive struct) — NOT NEEDED.** `xcm-helper` already owns asset registration. Do not add this struct to `primitives/orderbook/`.

**Do not modify any of the above pallets** as part of LMP chain work. They are production infrastructure.

### Bridge relayer (R-01–R-08) — CANCELLED
Hyperbridge's own permissionless relayer network replaces the custom Rust sidecar.

### Ethereum contract (B-01–B-09) — CANCELLED
Hyperbridge provides the EVM-side contracts. No `eth-bridge-contract` repo needed.

### Orderbook bridge server endpoints — SPEC UPDATED (not chain work)
The two bridge API stubs (E-37, E-38) in the orderbook server are now unblocked. Their specs changed:
- `GET /bridge/supported-assets` → query `xcm-helper::WhitelistedTokens` via chain RPC (not custom storage)
- `GET /bridge/deposits/{txHash}` → use `ismp_queryRequests` / `ismp_queryEvents` RPC; replace `vote_count`/validator model with ISMP request lifecycle (`Pending → Processing → Complete / Timeout`)
- `bridge_transactions` DB table → replace `vote_count`/`votes` columns with ISMP request commitment + state machine height + ISMP status enum

This is tracked in the orderbook repo, not here.

### Other permanent exclusions
- `pallet-thea` — deprecated, replaced by Hyperbridge; do not touch
- `pallet-pdex-migration` — one-time migration already executed on mainnet; do not touch
