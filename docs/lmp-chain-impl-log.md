# LMP Chain Implementation Log
**Branch:** `feat/lmp-chain`  
**Engineer:** Harsh Reddy  
**Phases covered:** P1, P2, P3, P4, P5, P6, P7, P8, P9

---

## Test Summary

| Crate | Tests | Result |
|---|---|---|
| `orderbook-primitives` | 13 (new: 8 in `lmp::tests`) | ✅ all pass |
| `pallet-ocex-lmp` after P1–P3 | 103 | ✅ all pass |
| `pallet-ocex-lmp` after P4–P6 | 123 (20 new) | ✅ all pass |
| `pallet-ocex-lmp` after P7–P9 | 132 (9 new) | ✅ all pass |
| `pallet-lmp` after P8 | 21 (4 new) | ✅ all pass |

Pre-existing ignored tests: 1 (`verify_withdrawal_request_signed_by_extension` — live node dependency, unchanged).

---

## Phase 1 — Primitives & Type Expansion

### `primitives/orderbook/src/lmp.rs` — Modified

#### `MarketTier` enum — new
```rust
pub enum MarketTier { #[default] Tier3, Tier2, Tier1 }
```
- Derives: all of `LMPMarketConfig`'s derives plus `Default`
- Declaration order `Tier3 < Tier2 < Tier1` gives correct derived `Ord` (Tier1 is highest)
- `Tier3` is `#[default]` — safe migration default for existing markets

#### `LMPMarketConfig` — modified
- Added `pub tier: MarketTier` as last field
- Added `Default` derive (all fields default safely: Decimal→0, MarketTier→Tier3)
- **Storage migration required** — handled by P2 `migrations/v1.rs`

#### `LMPMarketConfigWrapper` — modified
- Added `pub tier: MarketTier` field
- Used by `set_lmp_epoch_config` extrinsic to set tier during governance config updates

#### `DMMCommitment<AccountId>` — new struct
- All amounts `u128` (on-chain units)
- `committed_uptime: u8` (0–100); validated via `is_valid_uptime()`
- Derives `MaxEncodedLen` — required for use in `BoundedVec` (Phase 5)

#### Tests added (8 in `lmp::tests`)
`market_tier_default_is_tier3`, `market_tier_scale_roundtrip`, `market_tier_ordering`,
`lmp_market_config_default_tier_is_tier3`, `lmp_market_config_scale_roundtrip_with_tier`,
`dmm_commitment_scale_roundtrip`, `dmm_commitment_uptime_boundary`,
`lmp_epoch_config_verify_still_works_with_tier`

---

### `primitives/orderbook/src/types.rs` — Modified

#### `UserActions::OneMinLMPReport` — expanded
**Before:** `(TradingPair, Decimal, BTreeMap<AccountId, Decimal>)`  
**After:** `(TradingPair, Decimal, BTreeMap<AccountId, Decimal>, BTreeMap<AccountId, Decimal>, BTreeMap<AccountId, bool>)`

New fields: `maker_volume` and `uptime_present` — unblocks TR-02/TR-03 in orderbook repo.

> **Breaking SCALE change.** Chain deploys first; orderbook chainfollower/tradesrelayer updates second.

---

### `pallets/ocex/src/validator.rs` — Modified

Updated `OneMinLMPReport` match arm from 3 to 5 fields:
```rust
UserActions::OneMinLMPReport(_market, _total, _scores, _maker_volume, _uptime_present) => { ... }
```

---

## Phase 2 — Tiering + Enforcement + Storage Migration

### `pallets/ocex/src/lib.rs` — Modified

#### `StorageVersion` introduced
```rust
const STORAGE_VERSION: StorageVersion = StorageVersion::new(1);
#[pallet::storage_version(STORAGE_VERSION)]
pub struct Pallet<T>(_);
```
V0 = pre-LMP-chain (no version). V1 = after tier field migration.

#### `set_pair_tier` extrinsic — **Call Index 9**
- `GovernanceOrigin` only
- Updates `ExpectedLMPConfig` (next epoch) and current epoch's `LMPConfig` if pair present
- Emits `MarketTierSet { pair, tier }`

#### `set_lmp_epoch_config` — updated
`LMPMarketConfig` struct literal now includes `tier: market_config.tier` from the wrapper.

#### New error: `MarketConfigNotFound`

#### New event: `MarketTierSet { pair: TradingPair, tier: MarketTier }`

#### `get_trader_metrics_inner` — updated
Reads `LMPMarketConfig` from `LMPConfig[epoch]` storage and passes it to `compute_score`. Falls back to `LMPMarketConfig::default()` if config is missing (Tier3 exponents, safe).

---

### `pallets/ocex/src/validator.rs` — Modified

#### `compute_score` — signature changed
**Before:** `compute_score(state, main, pair, epoch) -> Result<Decimal>`  
**After:** `compute_score(state, main, pair, epoch, market_config: &LMPMarketConfig) -> Result<Decimal>`

#### `compute_all_scores` — updated
Fetches `market_config = config.config.get(&pair)` and passes to `compute_score`.

#### Tier-aware exponents (P2-3)
```rust
let (depth_exp, uptime_exp, volume_exp) = match market_config.tier {
    MarketTier::Tier1 => (0.15, 5.0, 0.85),
    MarketTier::Tier2 => (0.15, 5.0, 0.85),
    MarketTier::Tier3 => (0.15, 5.0, 0.85),
};
```
Values identical pending product confirmation of per-tier parameters. Structure is in place.

---

### `pallets/ocex/src/migrations/` — New directory

#### `mod.rs` — declares `pub mod v1`

#### `v1.rs` — V0→V1 migration
- Decodes each `LMPConfig` epoch entry using `OldLMPMarketConfig` (without `tier`)
- Reconstructs as new `LMPMarketConfig` with `tier: Tier3`
- Does same for `ExpectedLMPConfig`
- Sets `StorageVersion::new(1)` after completion
- Idempotent: returns early if `on_chain_storage_version >= 1`
- Includes `pre_upgrade` / `post_upgrade` hooks for `try-runtime`
- Returns accurate `Weight` via `T::DbWeight::get().reads_writes(reads, writes)`

**Runtime wiring needed** (P10): add to `Executive` migrations in `runtimes/mainnet/src/lib.rs`:
```rust
type Migrations = (pallet_ocex_lmp::migrations::v1::Migration<Runtime>,);
```

---

### Tests added (5 in `pallets/ocex/src/tests.rs`)
`set_pair_tier_requires_governance_origin`,
`set_pair_tier_stores_in_expected_lmp_config`,
`set_pair_tier_updates_current_epoch_lmp_config`,
`set_pair_tier_emits_market_tier_set_event`,
`set_pair_tier_fails_when_no_expected_lmp_config`

---

## Phase 3 — Fee Split & Reward Pool

### `pallets/ocex/src/lib.rs` — Modified

#### `FeesCollected` storage — new
```rust
pub(super) type FeesCollected<T: Config> = StorageDoubleMap<
    _, Blake2_128Concat, u16, Blake2_128Concat, TradingPair, BalanceOf<T>, ValueQuery,
>;
```

#### `update_lmp_scores` — updated
After inserting `TotalScores`, accumulates `total_fees_paid` (converted from Decimal to BalanceOf via `× UNIT_BALANCE`) into `FeesCollected[finalizing_epoch][pair]`.

---

### `pallets/ocex/src/session.rs` — Modified

#### `start_new_epoch` — updated
Calls `Self::distribute_lmp_fee_split(current_epoch)` before epoch counter increments.

#### `distribute_lmp_fee_split` — new function
- Drains `FeesCollected[epoch]` for the expiring epoch
- Computes `lmp_cut = fees / 4` (25%)
- Transfers from pallet account → `LMPRewardsPalletId` account
- Uses `KeepAlive` — never kills source account
- Failures log a warning and continue (parachain safety — no panic)

---

### Tests added (3 in `pallets/ocex/src/tests.rs`)
`fees_collected_populated_by_update_lmp_scores`,
`fees_collected_independent_per_pair`,
`fees_collected_cleared_after_epoch_boundary`

---

## Phase 4 — Volatility Multiplier (FEAT-110)

### `pallets/ocex/src/lib.rs` — Modified

#### Constant
```rust
pub(crate) const BLOCKS_PER_DAY: u32 = 7200;
```

#### `VolatilityTriggerCount` storage — new
`StorageNMap<(epoch: u16, pair: TradingPair, day_index: u32), u8>` — tracks daily trigger count per pair. Capped at 6/day.

#### `VolatilityActive` storage — new
`StorageMap<TradingPair, bool>` — `true` while multiplier is active; cleared at epoch boundary.

#### `trigger_volatility_multiplier` extrinsic — **Call Index 25**
- Accepts `GovernanceOrigin` OR `EnclaveOrigin` (engine operator)
- Computes `day_index = current_block / BLOCKS_PER_DAY`
- Enforces `count < 6` per (epoch, pair, day); emits `VolatilityMultiplierTriggered`

#### New error: `DailyVolatilityCapReached`
#### New event: `VolatilityMultiplierTriggered { pair, epoch }`

### `pallets/ocex/src/session.rs` — Modified

`start_new_epoch`: clears all `VolatilityActive` flags via `VolatilityActive::<T>::clear(u32::MAX, None)`.

### Tests added (6)
`trigger_volatility_requires_governance_or_enclave_origin`,
`trigger_volatility_increments_count`,
`trigger_volatility_capped_at_6_per_day`,
`trigger_volatility_sets_volatility_active`,
`trigger_volatility_emits_event`,
`volatility_active_cleared_at_epoch_boundary`

---

## Phase 5 — DMM System (FEAT-104)

### `pallets/ocex/src/lib.rs` — Modified

#### Config addition
```rust
#[pallet::constant]
type MaxDMMsPerPair: Get<u32>;
```

#### `DMMRegistry` storage — new
`StorageDoubleMap<(epoch: u16, pair: TradingPair), BoundedVec<DMMCommitment<AccountId>, MaxDMMsPerPair>>` — confirmed DMM commitments per epoch/pair.

#### `DMMPerformance` storage — new
`StorageNMap<(epoch: u16, pair: TradingPair, account: AccountId), u8>` — actual uptime % recorded at epoch end.

#### `register_dmm` extrinsic — **Call Index 10**
- `ensure_signed`; only for future epochs (`epoch > current_epoch`)
- Validates `committed_uptime <= 100`; appends to `DMMRegistry` (bounded)

#### `confirm_dmm_selection` extrinsic — **Call Index 11**
- `GovernanceOrigin`; filters `DMMRegistry` to only retain selected accounts

#### `submit_dmm_performance` extrinsic — **Call Index 13**
- `EnclaveOrigin` (engine operator); writes `(account, uptime_pct)` to `DMMPerformance`

#### `claim_dmm_stipend` extrinsic — **Call Index 22**
- `ensure_signed`; checks `DMMPerformance >= committed_uptime`
- Transfers stipend from `LMPRewardsPalletId` account to caller

#### New errors: `EpochAlreadyStarted`, `InvalidUptimeCommitment`, `TooManyDMMs`, `DMMUptimeNotMet`, `DMMCommitmentNotFound`, `DMMPerformanceNotFound`
#### New events: `DMMRegistered`, `DMMSelected`, `DMMStipendClaimed`

### `pallets/ocex/src/session.rs` — Modified

`start_new_epoch`: calls `reserve_dmm_stipends(new_epoch)` — iterates `DMMRegistry` for the incoming epoch, sums all stipends per pair, transfers total from `LMPRewardsPalletId` → pallet account. Failures are logged (non-panicking).

### `pallets/ocex/src/rpc.rs` — Modified

`get_dmm_status(epoch, pair)` — returns `(Vec<DMMCommitment>, Vec<(AccountId, u8)>)` by reading `DMMRegistry` and `DMMPerformance` storage.

### `pallets/ocex/src/mock.rs` — Modified

Added `MaxDMMsPerPair = 10` to `parameter_types!` and wired into `Config for Test`.

### Tests added (7)
`register_dmm_stores_commitment`,
`register_dmm_fails_for_current_epoch`,
`register_dmm_fails_when_uptime_exceeds_100`,
`confirm_dmm_selection_requires_governance`,
`confirm_dmm_selection_filters_registry`,
`submit_dmm_performance_stores_uptime`,
`claim_dmm_stipend_fails_if_uptime_not_met`

---

## Phase 6 — Merkle Snapshot & Claim (FEAT-113)

### `pallets/ocex/src/lib.rs` — Modified

#### `LMPMerkleRoot` storage — new
`StorageDoubleMap<(epoch: u16, pair: TradingPair), H256>` — Merkle root submitted by the engine operator.

#### `MerkleRewardsClaimed` storage — new
`StorageDoubleMap<(account: AccountId, epoch: u16), BalanceOf<T>>` — double-claim guard.

#### `submit_lmp_snapshot` extrinsic — **Call Index 26**
- `EnclaveOrigin`; stores `LMPMerkleRoot[epoch][pair]`
- Sets `LMPClaimBlk[epoch]` = `current_block + claim_safety_period` (reuses existing safety period from `LMPConfig[epoch]`)

#### `claim_rewards_merkle` extrinsic — **Call Index 27**
- `ensure_signed`; checks safety period, double-claim guard, Merkle proof
- Leaf = `Blake2b256(account_bytes ++ epoch_le_u16 ++ amount_le_u128)` — matches `EpochAggregator` in orderbook
- Transfers from `LMPRewardsPalletId` to caller; marks claimed

#### `build_merkle_leaf` helper
Constructs the leaf hash deterministically for a given `(account, epoch, amount)`.

#### `verify_merkle_proof` helper
Walks proof path with canonical sibling ordering (smaller hash on left) — matches the orderbook's `EpochAggregator` tree construction.

#### New errors: `MerkleRootNotFound`, `InvalidMerkleProof`, `MerkleRewardAlreadyClaimed`
#### New events: `LMPMerkleRootSubmitted`, `MerkleRewardClaimed`

### `pallets/ocex/src/rpc.rs` — Modified

- `get_lmp_merkle_root(epoch, pair)` — returns stored `Option<H256>`
- `get_volatility_trigger_count(pair, day)` — reads `VolatilityTriggerCount[current_epoch][pair][day]`
- `current_day_index()` — returns `current_block / BLOCKS_PER_DAY` (helper for clients)

### Tests added (7)
`submit_lmp_snapshot_requires_enclave_origin`,
`submit_lmp_snapshot_stores_root`,
`submit_lmp_snapshot_emits_event`,
`claim_rewards_merkle_single_leaf_valid_proof`,
`claim_rewards_merkle_fails_with_wrong_proof`,
`claim_rewards_merkle_fails_before_safety_period`,
`verify_merkle_proof_two_leaf_tree`

---

---

## Phase 7 — Maker Rebate + Governance Extrinsics

### `pallets/ocex/src/lib.rs` — Modified

#### `SuspendedLMPPairs` storage — new
`StorageMap<TradingPair, bool>` — pairs suspended by governance; checked in `compute_all_scores` to skip suspended pairs silently.

#### `suspend_lmp_rewards` extrinsic — **Call Index 28**
- `GovernanceOrigin`; sets `SuspendedLMPPairs[pair] = true`
- Removes pair from `ExpectedLMPConfig` (so next epoch won't include it)
- Emits `LMPRewardsSuspended { pair }`

#### `demote_pair_tier` extrinsic — **Call Index 29**
- `GovernanceOrigin`; validates `new_tier < current_tier` (rejects promotions)
- Updates `ExpectedLMPConfig` and current epoch `LMPConfig`
- Emits `MarketTierDemoted { pair, new_tier }`

#### New error: `InvalidTierDemotion`
#### New events: `LMPRewardsSuspended`, `MarketTierDemoted`, `MakerRebatePaid` (infrastructure)

### `pallets/ocex/src/validator.rs` — Modified
`compute_all_scores` skips pairs where `SuspendedLMPPairs[pair] == true`.

### Tests added (7)
`suspend_lmp_rewards_requires_governance`, `suspend_lmp_rewards_sets_suspended_flag`,
`suspend_lmp_rewards_removes_pair_from_expected_config`, `suspend_lmp_rewards_emits_event`,
`demote_pair_tier_requires_governance`, `demote_pair_tier_updates_expected_config`,
`demote_pair_tier_rejects_invalid_demotion`

---

## Phase 8 — `pallet-liquidity-mining` Fixes (C-35 to C-39)

### `pallets/liquidity-mining/src/lib.rs` — Modified

#### C-35: `validate_unsigned` tightened
- Rejects when no `SnapshotFlag` active
- Rejects `External` AND `InBlock` sources (local only)
- Uses `(b"lmp_snapshot", snapshot_blk).encode()` as `and_provides` tag — deduplicates per snapshot
- Removed TODOs; `propagate(false)` so tx is not gossiped

#### C-36: `PoolSnapshotFlag` per-pool storage added
`StorageDoubleMap<(TradingPair, AccountId), BlockNumberFor<T>>` — per-pool in-progress flag.
`add_liquidity` and `remove_liquidity` now check `PoolSnapshotFlag[market][market_maker]` instead of the global `SnapshotFlag`. Allows other pools to operate concurrently.

#### C-37: `initiate_withdrawal` weight parameterized
`#[pallet::weight(10000 + num_requests * 5000)]` — scales with actual request count.

#### C-38: `LiquidityFundsClaimed` event + emitted in `claim_force_closed_pool_funds`
New event fields: `market, pool, lp, base_amount, quote_amount`. Removes the TODO comment.

#### C-39: `callback.rs` FIXME resolved
Removed `//FIXME: What are we doing with base_freed and quote_freed?`. Documented that amounts remain in the pool account for LPs to claim via `claim_force_closed_pool_funds`. No behaviour change needed.

### `pallets/liquidity-mining/src/mock.rs` — Modified
Added `type MaxDMMsPerPair = ConstU32<10>` to `impl ocex::Config for Test`.

### Tests added (4 in `pallets/liquidity-mining/src/tests.rs`)
`unsigned_tx_validation_rejects_when_no_snapshot_active`,
`unsigned_tx_validation_rejects_external_source`,
`pool_snapshot_flag_independent_per_pool`,
`pool_snapshot_flag_cleared_per_pool`

---

## Phase 9 — Offchain State Verification + LMP Wiring

### `pallets/ocex/src/settlement.rs` — Modified (P9-1)
Uncommented `update_lmp_storage_from_trade` call (was a commented-out TODO). Renamed `_maker_fees` / `_taker_fees` local bindings to `maker_fees_amount` / `taker_fees_amount` to avoid confusion with the `FeeConfig` parameters. Every trade now writes maker volume, trade volume, and fees paid to offchain trie.

### `pallets/ocex/src/validator.rs` — Modified (P9-2)
Uncommented `store_q_scores` call inside `OneMinLMPReport` match arm. Engine-submitted Q-scores now flow through to offchain storage during `run_on_chain_validation`.

### `pallets/ocex/src/integration_tests.rs` — Modified
- Added `register_offchain_ext` helper (inline, matches definition in `tests.rs`)
- Added `push_lmp_report_user_actions` helper — builds a `UserActionBatch` with `OneMinLMPReport` actions
- Added `NewLMPEpoch` ingress injection before LMP report tests (required to initialize LMP config in offchain state)

### Integration tests added (2)
`maker_volume_stored_in_offchain_state_after_trade` — verifies `update_maker_volume_by_main_account` is called end-to-end after a trade fill; asserts offchain storage key has `> 0` volume.

`uptime_count_increments_per_non_zero_score_snapshot` — pushes a batch with 3 `OneMinLMPReport` actions; verifies `get_q_score_and_uptime` returns `uptime_count = 3`.

---

## Files Modified (P4–P6)

| File | Change |
|---|---|
| `pallets/ocex/src/lib.rs` | `MaxDMMsPerPair` config, `BLOCKS_PER_DAY` const, 5 new storage items, 7 new extrinsics (indices 10,11,13,22,25,26,27), 10 new errors, 7 new events, 2 Merkle helpers |
| `pallets/ocex/src/session.rs` | `VolatilityActive` clear + `reserve_dmm_stipends` at epoch start |
| `pallets/ocex/src/rpc.rs` | `get_dmm_status`, `get_lmp_merkle_root`, `get_volatility_trigger_count`, `current_day_index` helpers |
| `pallets/ocex/src/mock.rs` | `MaxDMMsPerPair = 10` parameter + Config wire |
| `pallets/ocex/src/tests.rs` | 20 new tests for P4/P5/P6 |

---

## Files Modified

| File | Change |
|---|---|
| `primitives/orderbook/src/lmp.rs` | `MarketTier`, `DMMCommitment`, `tier` in `LMPMarketConfig` + `LMPMarketConfigWrapper`, tests |
| `primitives/orderbook/src/types.rs` | `OneMinLMPReport` variant expanded (+2 fields) |
| `pallets/ocex/src/lib.rs` | `StorageVersion`, `set_pair_tier` (index 9), `FeesCollected` storage, `update_lmp_scores` update, `get_trader_metrics_inner` update, new error/event, `migrations` module declaration |
| `pallets/ocex/src/validator.rs` | `compute_score` signature + tier-aware exponents, `compute_all_scores` passes `market_config`, `OneMinLMPReport` match arm |
| `pallets/ocex/src/session.rs` | `distribute_lmp_fee_split`, fee split call in `start_new_epoch` |
| `pallets/ocex/src/tests.rs` | Fixed existing `LMPMarketConfigWrapper` literals (added `tier`), 8 new tests |
| `pallets/ocex/src/integration_tests.rs` | Fixed `LMPMarketConfigWrapper` literal (added `tier`) |

## Files Created

| File | Purpose |
|---|---|
| `pallets/ocex/src/migrations/mod.rs` | Module declaration for migrations |
| `pallets/ocex/src/migrations/v1.rs` | V0→V1 storage migration |
| `docs/lmp-chain-impl-log.md` | This file |

---

## Files Modified (P7–P9)

| File | Change |
|---|---|
| `pallets/ocex/src/lib.rs` | `SuspendedLMPPairs` storage, `suspend_lmp_rewards` (28), `demote_pair_tier` (29), 3 new events, 1 new error |
| `pallets/ocex/src/validator.rs` | Skip suspended pairs in `compute_all_scores`; uncomment `store_q_scores` wiring |
| `pallets/ocex/src/settlement.rs` | Uncomment `update_lmp_storage_from_trade`; rename local fee variables |
| `pallets/ocex/src/tests.rs` | 7 new P7 tests |
| `pallets/ocex/src/integration_tests.rs` | `register_offchain_ext` helper, `push_lmp_report_user_actions` helper, 2 P9 integration tests |
| `pallets/liquidity-mining/src/lib.rs` | C-35 validate_unsigned, C-36 PoolSnapshotFlag, C-37 weight, C-38 event, C-39 FIXME resolved |
| `pallets/liquidity-mining/src/mock.rs` | `MaxDMMsPerPair = ConstU32<10>` |
| `pallets/liquidity-mining/src/tests.rs` | 4 new P8 tests; fix `LMPMarketConfigWrapper` struct literal |
| `pallets/liquidity-mining/src/callback.rs` | Remove FIXME, document freed-amounts behaviour |

---

## Pending Before Runtime Upgrade (P10)

- Wire migration into `runtimes/mainnet/src/lib.rs` `Executive` migrations tuple
- Add `MaxDMMsPerPair = 10` to `impl pallet_ocex_lmp::Config for Runtime` in runtime
- Run `scripts/try-runtime-on-runtime-upgrade.sh` against a mainnet snapshot
- Confirm per-tier exponent values with product (currently all tiers use same values)
- Benchmarks for all new extrinsics (currently using `T::DbWeight::get().reads_writes(...)` estimates)

## Cross-Repo Activation (orderbook)

| Chain change | Orderbook item unblocked |
|---|---|
| `OneMinLMPReport` expanded (P1) | TR-02: `maker_volume` + `uptime_present` flow to chain ingress |
| `tier` in `LMPEpochConfig` (P2) | `GET /lmp/pairs` returns real tier data |
| `VolatilityActive` storage (P4) | `/qscore` endpoint shows volatility flag |
| `DMMRegistry` storage (P5) | `DmmUptimeTracker.update_dmm_list()` activates in engine |
| `submit_lmp_snapshot` extrinsic (P6) | `EpochAggregator` trigger in `engine.rs` activates |
| `claim_rewards_merkle` extrinsic (P6) | `GET /lmp/accounts/{addr}/rewards/claimable` returns real proofs |
