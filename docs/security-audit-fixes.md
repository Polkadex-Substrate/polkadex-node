# Security Audit — Fix Log

Tracking all changes applied from the 14 August 2026 security audit.  
Audit covered `polkadex-substrate/Polkadex` and `Polkadex-Substrate/matching-engine`.  
This document covers fixes applied to **this repo only**.

**Totals:** 65 findings in this repo · 49 fixed (as of last update) · 16 open  
See [`polkadex-audit-findings.md`](../polkadex-audit-findings.md) on the Desktop for the full findings table.

---

## Fixed

### C1 — Unauthenticated snapshot submission (empty authority set)
**Severity:** Critical  
**Location:** `pallets/ocex/src/lib.rs` — `validate_snapshot`  
**Fixed in spec:** 391  
**Date:** 2026-08-14

**Vulnerability:** `submit_snapshot` is an unsigned extrinsic. `validate_snapshot` fetched the authority set using the caller-supplied `snapshot_summary.validator_set_id` with no comparison against the active chain set. A non-existent ID returns an empty validator set via `ValueQuery`, collapsing the 51% threshold to zero. `0 > 0` is false, so zero signatures were accepted as valid.

**Changes made:**
- `pallets/ocex/src/lib.rs`: Pinned `validator_set_id` to `<ValidatorSetId<T>>::get()` — returns `Custom(14)` on mismatch
- `pallets/ocex/src/lib.rs`: Added empty authority set guard — returns `Custom(15)`
- `pallets/ocex/src/tests.rs`: Added regression test `exploit_fabricated_validator_set_id_is_rejected`
- `pallets/ocex/src/tests.rs`: Added regression test `empty_active_authority_set_is_rejected`

---

### C3 — Threshold floors to zero; signer index replay
**Severity:** Critical  
**Location:** `pallets/ocex/src/lib.rs` — `validate_snapshot`  
**Fixed in spec:** 391  
**Date:** 2026-08-14

**Vulnerability:** `Percent::from_percent(51) * n` truncates (51% × 3 = 1, meaning 33% suffices). A single compromised key could be submitted multiple times under different indices to meet the count. The comparison was `threshold > signatures.len()` (wrong direction) which also passed on zero.

**Changes made:**
- `pallets/ocex/src/lib.rs`: Replaced Percent-based floor with `core::cmp::max(threshold, 1)` and corrected comparison to `signatures.len() < required`
- `pallets/ocex/src/lib.rs`: Added `BTreeSet<u16>` to track seen signer indices — returns `Custom(16)` on duplicate
- `pallets/ocex/src/tests.rs`: Added regression test `zero_signatures_rejected_even_with_correct_set`
- `pallets/ocex/src/tests.rs`: Added regression test `duplicate_signer_index_is_rejected`
- `pallets/ocex/src/tests.rs`: Added regression test `genuine_majority_snapshot_still_validates`

---

### C8 — THEA outgoing quorum via retired or empty validator sets
**Severity:** Critical  
**Location:** `pallets/thea/src/lib.rs`, `primitives/thea/src/types.rs`  
**Fixed in spec:** 391  
**Date:** 2026-08-14

**Vulnerability:** Same root cause as C1/C3 applied to the bridge outgoing path. `submit_signed_outgoing_messages` accepted a caller-supplied `validator_set_id` without pinning it to the active set. `add_signature` silently merged signatures across different validator sets (bumping the stored ID and clearing prior signatures when a higher ID arrived). `threshold_reached` used `67% * max_len` which floors to zero for empty sets (`0 >= 0 = true`). Historical sets are never pruned, so a retired set with no stake at risk could finalise a withdrawal.

**Changes made:**

`pallets/thea/src/lib.rs`:
- Added `InvalidValidatorSetId` and `EmptyValidatorSet` error variants
- `validate_signed_outgoing_message`: Added pin to `<ValidatorSetId<T>>::get()` — returns `Custom(7)` on mismatch, `Custom(8)` on empty set
- `submit_signed_outgoing_messages` dispatch body: Added same pin + empty-set guard using `Error::InvalidValidatorSetId` / `Error::EmptyValidatorSet`
- `submit_signed_outgoing_messages`: Changed `auth_len` to use the already-validated active `authorities.len()` instead of re-deriving from `signed_msg.validator_set_id`
- `change_authorities`: Added `<Authorities<T>>::remove(new_id.saturating_sub(2))` to prune sets 2 epochs back on every rotation, keeping only current and previous

`primitives/thea/src/types.rs`:
- `add_signature`: Replaced cross-set merge logic with a strict equality check — signatures with a mismatched `validator_set_id` are now silently dropped with a log error instead of being accepted and resetting quorum
- `threshold_reached`: Replaced `Percent::from_percent(67) * max_len` with `if max_len == 0 { return false }` guard + ceiling integer arithmetic `(2 * max_len) / 3 + 1`
- Removed now-unused `use sp_runtime::Percent` import

---

### C9 — IngressMessages unbounded, never pruned, O(n²) re-encode; no minimum deposit
**Severity:** Critical  
**Location:** `pallets/ocex/src/lib.rs`, `pallets/ocex/src/lmp.rs`, `pallets/ocex/src/session.rs`  
**Fixed in spec:** 391  
**Date:** 2026-08-14

**Vulnerability:** `IngressMessages` was declared as an unbounded `Vec<...>`. Every extrinsic that queued a message called `.mutate().push()` — SCALE-decodes the whole vec, appends one item, SCALE-encodes the whole vec back. With N pushes per block this is O(n²) total encoding work. The map was never pruned, so entries accumulated forever. Any user could spam cheap deposits (funds immediately available for withdrawal) to bloat the queue toward OOM without losing money. No minimum deposit was enforced.

**Changes made:**

`pallets/ocex/src/lib.rs`:
- Added `OBIngressLimit: Get<u32>` and `MinimumDeposit: Get<u128>` to the `Config` trait
- Changed `IngressMessages` storage from `Vec<...>` to `BoundedVec<..., T::OBIngressLimit>`
- Added error variants `IngressQueueFull` and `DepositAmountTooLow`
- Converted all 8 in-file push sites from `mutate + push` to `try_mutate + try_push` — returns `IngressQueueFull` on overflow
- Restructured `close_trading_pair` and `open_trading_pair` to extract ingress push from inside nested `mutate` closure so the error can propagate
- Added `ensure!(amount >= T::MinimumDeposit::get(), DepositAmountTooLow)` in `do_deposit` before any funds are transferred (C9 + L6 partial)
- Added pruning in `submit_snapshot`: after every accepted snapshot, all `IngressMessages` entries for blocks ≤ `summary.last_processed_blk` are removed

`pallets/ocex/src/lmp.rs`:
- Converted 3 push sites to `try_push`; `add_liquidity` (returns `DispatchResult`) propagates the error; `remove_liquidity` and `force_close_pool` (return `()`) log and drop on overflow

`pallets/ocex/src/session.rs`:
- Converted 2 push sites (NewLMPEpoch + LMPConfig) to `try_push`; logs on overflow (called from `on_initialize`, cannot propagate)

`pallets/ocex/src/mock.rs`:
- Added `OBIngressLimit = 100` and `MinimumDeposit = 1` (1 planck, allows existing tests to pass)

`pallets/ocex/src/integration_tests.rs`:
- Changed 2 direct `IngressMessages::insert(_, vec![...])` calls to `BoundedVec::try_from(vec![...])` to match new type

`runtimes/mainnet/src/lib.rs`:
- Added `OBIngressLimit = 500` (500 ingress messages cap per block)
- Added `OcexMinimumDeposit = 1_000_000_000_000` (1 PDEX, chosen to prevent dust spam while allowing normal trading deposits)
- Wired both into `impl pallet_ocex_lmp::Config for Runtime`
- Added `migrations::PruneStaleIngressMessages` to the spec-391 `Migrations` tuple

`runtimes/mainnet/src/migrations.rs`:
- Added `PruneStaleIngressMessages` — reads the latest snapshot's `last_processed_blk`, then
  removes all `IngressMessages` entries with key ≤ that block number using key-only iteration
  (no value decode, so the Vec→BoundedVec type change cannot cause a decode failure during migration)
- Includes `pre_upgrade`/`post_upgrade` hooks for try-runtime verification

---

### C6 — LP callbacks wired to no-op `()`; pool_id ↔ market_maker key mismatch
**Severity:** Critical  
**Location:** `pallets/liquidity-mining/src/`, `runtimes/mainnet/src/`  
**Fixed in spec:** 391  
**Date:** 2026-08-17

**Vulnerability:** Two stacked bugs made liquidity-mining callbacks completely inoperative:

1. **No-op wire-up** — `runtimes/mainnet/src/lib.rs` had `type CrowdSourceLiqudityMining = ()`. The `()` implementation of `LiquidityMiningCrowdSourcePallet` is a blanket no-op; none of `add_liquidity_success`, `remove_liquidity_failed`, or `pool_force_close_success` ever ran on mainnet.

2. **Key mismatch** — Even with the correct type wired, OCEX calls all three callbacks with the *pool_id* (the derived PalletId sub-account) as the `pool: &T::AccountId` argument, but the LMP pallet's `Pools` storage is keyed by `(TradingPair, market_maker)`. A direct `Pools::get(market, pool)` would therefore always return `None`, causing share minting, refund logic, and force-close to silently fail.

**Impact:** On a live system, LPs depositing liquidity would never receive their LP-shares (minted by `add_liquidity_success`). Failed removals would never refund shares (`remove_liquidity_failed`). Force-closed pools would never be marked as closed on-chain (`pool_force_close_success`).

**RPC verification:** Confirmed via `state_getKeysPaged` against `https://so.polkadex.ee` that zero pool entries exist in `Pools` storage on mainnet — LMP was never used in production, so there are no stuck funds to recover.

**Changes made:**

`pallets/liquidity-mining/src/lib.rs`:
- Made `Pools` storage `pub` (was `pub(super)`) so the migration can iterate it
- Added `PoolIdIndex` storage: `Blake2_128Concat, AccountId → (TradingPair, AccountId)` — a reverse index from `pool_id` to `(market, market_maker)`, enabling O(1) lookup without a full table scan
- In `register_pool`: after building `config`, inserted into `PoolIdIndex` (`config.pool_id → (market, market_maker)`) before `Pools::insert`

`pallets/liquidity-mining/src/callback.rs`:
- `add_liquidity_success`: renamed parameter from `market_maker` to `pool_id`; added reverse-index lookup (`PoolIdIndex::get(pool_id)`) before `Pools::get`
- `remove_liquidity_failed`: added same reverse-index lookup before `Pools::get`
- `pool_force_close_success`: renamed parameter from `market_maker` to `pool_id`; added reverse-index lookup; updated `Pools::insert` to use the resolved `market_maker`

`pallets/liquidity-mining/src/tests.rs`:
- Updated 3 test call sites (`add_liquidity` helper, `test_add_liquidity_success_happy_path`, `test_force_close_pool_happy_path_and_error`) to compute `pool_id` via `create_pool_account` and pass `&pool_id` instead of `&market_maker` to the callbacks

`runtimes/mainnet/src/lib.rs`:
- Changed `type CrowdSourceLiqudityMining = ()` to `type CrowdSourceLiqudityMining = CrowdSourceLMP` in `impl pallet_ocex_lmp::Config for Runtime`
- Added `migrations::RebuildLmpPoolIdIndex` to the spec-391 `Migrations` tuple

`runtimes/mainnet/src/migrations.rs`:
- Added `RebuildLmpPoolIdIndex` migration: iterates all `Pools<Runtime>` entries, inserts corresponding `PoolIdIndex` entries, then removes any pre-existing stale entries. Idempotent. Includes `pre_upgrade`/`post_upgrade` hooks for try-runtime. (Migration is a no-op on mainnet since zero pools exist — included for correctness on any testnet instances that had pools registered before this fix.)

---

### C7 — Master BIP39 seed committed in repo — all session keys compromised
**Severity:** Critical  
**Location:** `session-keys/`, `nodes/mainnet/src/chain_spec_old.rs`  
**Fixed in spec:** N/A (operational + repo hygiene, no on-chain migration)  
**Date:** 2026-08-17

**Vulnerability:** Three BIP39 mnemonic seeds were committed to the repository in plaintext and are permanently visible in git history to anyone who ever cloned the repo:

| Seed | Keys derived from it |
|---|---|
| `***REMOVED***` | BABE (block production), GRANDPA (finality), Orderbook (snapshot signing), THEA (bridge signing) — validators 1–4 |
| `***REMOVED***` | BEEFY — validators 1–3 |
| `***REMOVED***` | Mixnet — validators 1–3 |

An attacker with these seeds can derive every private key, sign arbitrary blocks, finality votes, snapshots, and bridge messages for all affected validators.

**Code changes made:**

`nodes/mainnet/src/chain_spec_old.rs`:
- Removed the hardcoded `seed` literal from `udon_testnet_config_genesis`
- Replaced it with three `std::env::var("TESTNET_SEED_1/2/3")` reads that panic with a descriptive security message if unset — the function cannot be called without explicitly supplying seeds at runtime
- Updated the `for` loop from a range over an integer to `seeds.iter().enumerate()` to match the new structure

`session-keys/` directory:
- Removed all 20 key files (`babe1-4`, `gran1-4`, `ob1-3`, `thea1-3`, `beefy1-3`, `mixnet1-3`) from git tracking via `git rm --cached`
- Directory is already in `.gitignore` — will not be re-added

**⚠️ Operational work still required — validators must rotate keys:**

> Cleaning the repo prevents future exposure but does NOT invalidate the already-leaked seeds. Every validator must rotate their session keys on the live chain.

For each validator node (repeat for all 4 validators):

**Step 1 — Generate new keys inside the node keystore**
```bash
# Call author_rotateKeys on the running node — generates fresh keys from local entropy, no mnemonic
curl -H "Content-Type: application/json" -d '{"id":1,"jsonrpc":"2.0","method":"author_rotateKeys","params":[]}' http://localhost:9944
# Save the returned 0x... hex blob — that is your new session key set
```

**Step 2 — Submit set_keys on-chain**  
Each validator's controller account calls:
```
session::set_keys(keys: <0x hex from step 1>, proof: 0x)
```
via Polkadot.js Apps → Extrinsics → session → setKeys.

**Step 3 — Wait for activation**  
New keys become active at the next session boundary (≈ 1 era on mainnet). Verify with:  
`session::nextKeys(validatorAccountId)` — should return the new pubkeys.

**Step 4 — Confirm and purge old keystores**  
On each validator node, verify the old BABE/GRANDPA/OB/THEA/BEEFY/mixnet pubkeys derived from the committed seeds are no longer present in the node's keystore directory. Remove any stale keystore files that correspond to the old pubkeys.

**Step 5 — THEA bridge authority set**  
The new THEA ECDSA pubkey must be registered with the bridge authority set. Depending on how THEA's validator set rotation is managed, this may require a governance call or a direct `change_authorities` dispatch from the governance origin.

---

### H4 — UserActionBatch.signature never verified against operator public key
**Severity:** High  
**Location:** `pallets/ocex/src/validator.rs`, `primitives/orderbook/src/types.rs`  
**Fixed in spec:** 391  
**Date:** 2026-08-17  
**Migration required:** No — pure off-chain worker logic change.

**Vulnerability:** The OCW (`run_on_chain_validation`) fetches `UserActionBatch` from the aggregator and passes it directly to `process_batch` without ever checking `batch.signature`. The `UserActionBatch` struct carries an ECDSA operator signature field and a `sign_data()` method — the infrastructure to verify was present but the verification call was simply missing. `OrderbookOperatorPublicKey` is registered on-chain by governance but was never consulted by the validator. Any attacker able to serve a crafted response at the aggregator endpoint (MITM, compromised aggregator) could inject arbitrary trades, withdrawals, and block-import events with no signature check to reject them.

**Changes made:**

`primitives/orderbook/src/types.rs`:
- Added `verify(&self, public_key: &sp_core::ecdsa::Public) -> bool` to the `UserActionBatch` impl — uses `signature.recover_prehashed(&self.sign_data())` and compares the recovered key, mirroring the existing `ObMessage::verify` pattern

`pallets/ocex/src/validator.rs`:
- Imported `OrderbookOperatorPublicKey` from `crate::pallet`
- Added `verify_batch_signature(batch: &UserActionBatch<T::AccountId>) -> bool` helper — reads the registered operator key from storage and calls `batch.verify`. Returns `false` (with a log error) if no key is registered
- Called `verify_batch_signature` at both batch load sites — the back-fill sync loop (`for nonce in last_processed_nonce..next_nonce`) and the main processing path (`next_nonce`) — before any call to `process_batch`. Returns `Err("Invalid batch signature")` on failure.

`pallets/ocex/src/integration_tests.rs`:
- Added `test_operator_pair()` helper (deterministic `//test-operator-h4` ECDSA key) and `register_test_operator()` helper
- Updated `push_trade_user_actions` and `push_trade_user_actions_with_fee` to sign the batch with `test_operator_pair().sign_prehashed(&batch.sign_data())`
- Added `register_test_operator()` call at the start of both integration tests that exercise `run_on_chain_validation`

---

### R4-A — `claim_withdraw` re-inserts empty vec; repeated no-op calls allowed
**Severity:** High  
**Location:** `pallets/ocex/src/lib.rs` — `claim_withdraw`  
**Fixed in spec:** 391  
**Date:** 2026-08-17  
**Migration required:** No — storage layout unchanged; existing ghost entries self-correct on next `claim_withdraw` call (they get cleaned up at that point).

**Vulnerability:** `do_withdraw` returns `(failed_withdrawals, processed_withdrawals)`. The original code unconditionally called `btree_map.insert(account, failed_withdrawals)` regardless of whether `failed_withdrawals` was empty. When all withdrawals in a batch succeeded, an empty `Vec` was re-inserted under the account key. The snapshot's `Withdrawals` entry therefore still existed in storage, `contains_key(snapshot_id)` remained `true`, and the account key was still present in the map. Any signed caller could then call `claim_withdraw` again for the same `(snapshot_id, account)` pair: `do_withdraw` would be called with the empty vec (a pure no-op), and the empty vec would be re-inserted again — this cycle could repeat indefinitely.

The no-op loop doesn't move funds (since `do_withdraw` with an empty input produces no transfers), but it wastes block space and is a clear logic error.

**Changes made:**

`pallets/ocex/src/lib.rs`:
- In `claim_withdraw`: changed `btree_map.insert(account, failed_withdrawals)` to a conditional `if !failed_withdrawals.is_empty()` — only re-inserts when there are genuine retryable failures
- After the `mutate` closure: added `if Withdrawals::get(snapshot_id).is_empty() { Withdrawals::remove(snapshot_id) }` — frees the storage entry entirely when no accounts' withdrawals remain, ensuring `contains_key` returns `false` and all subsequent calls correctly return `InvalidWithdrawalIndex`

`pallets/ocex/src/tests.rs`:
- Added `test_claim_withdraw_no_double_claim_after_all_succeed` — submits a snapshot without funding the custodian (so `do_withdraw` fails and the withdrawal goes into `Withdrawals`), then funds the custodian, calls `claim_withdraw` successfully, and asserts the second call returns `InvalidWithdrawalIndex`

---

### R2-H1 — `process_egress_msg` routes funds to caller-chosen account; egress unbounded
**Severity:** High  
**Location:** `pallets/ocex/src/lib.rs`, `primitives/orderbook/src/traits.rs`, `pallets/liquidity-mining/src/callback.rs`  
**Fixed in spec:** 391  
**Date:** 2026-08-17  
**Migration required:** No — `MaxEgressMessages` is a new Config constant (no storage changes).

**Vulnerability:** Two stacked issues in `process_egress_msg`:

1. **Caller-chosen destination**: `EgressMessages::RemoveLiquidityResult` and `EgressMessages::PoolForceClosed` both include a `pool: AccountId` field that is embedded in the `SnapshotSummary` submitted by the caller. Before the fix, these handlers transferred assets from the OCEX pallet account directly to `pool` without verifying it is a registered LMP pool sub-account. An attacker who can produce a validly-signed snapshot (requires compromising 51%+ of validator keys — a realistic concern while C7 key rotation is still pending) could set `pool` to any account and redirect funds from the pallet.

2. **Unbounded loop**: `process_egress_msg` iterated over all `msgs` with no cap. A crafted snapshot with thousands of egress messages causes unbounded block execution.

**Changes made:**

`primitives/orderbook/src/traits.rs`:
- Added `fn is_valid_pool_id(pool_id: &AccountId) -> bool` to `LiquidityMiningCrowdSourcePallet` — returns whether `pool_id` is a registered pool sub-account
- `()` impl returns `false` (when LMP is disabled, no pool IDs are valid)

`pallets/liquidity-mining/src/callback.rs`:
- Implemented `is_valid_pool_id` for the LMP pallet: `PoolIdIndex::contains_key(pool_id)`

`pallets/ocex/src/lib.rs`:
- Added `MaxEgressMessages: Get<u32>` to `Config`
- Added error variants `TooManyEgressMessages` and `InvalidEgressPoolId`
- At start of `process_egress_msg`: `ensure!(msgs.len() <= T::MaxEgressMessages::get(), TooManyEgressMessages)`
- In `RemoveLiquidityResult`: added `ensure!(T::CrowdSourceLiqudityMining::is_valid_pool_id(pool), InvalidEgressPoolId)` before `transfer_asset` calls
- In `PoolForceClosed`: same pool-id validation before `transfer_asset` calls

`pallets/ocex/src/mock.rs` and `pallets/liquidity-mining/src/mock.rs`:
- Added `MaxEgressMessages = 1000` to `parameter_types!` and wired into `impl Config`

`runtimes/mainnet/src/lib.rs`:
- Added `MaxEgressMessages: u32 = 1000` constant and wired into `impl pallet_ocex_lmp::Config`

`pallets/ocex/src/tests.rs`:
- Updated `test_process_remove_liquidity_result`: inserts into `PoolIdIndex` for the test pool account so the new validation passes

---

### H7 — OutgoingMessages slot overwrite — bridged tokens permanently lost
**Severity:** High  
**Location:** `pallets/thea/src/lib.rs` — `execute_withdrawals`, `update_outgoing_nonce`  
**Fixed in spec:** 391  
**Date:** 2026-08-17  
**Migration required:** No — pure logic guard; no storage layout change.

**Vulnerability:** `execute_withdrawals` calls `generate_payload` which reads `OutgoingNonce` from storage and adds 1 to get the next slot, then unconditionally writes both `OutgoingNonce` and `OutgoingMessages[network][nonce]`. The governance extrinsic `update_outgoing_nonce` accepted any nonce value with no lower-bound check.

**Attack/failure scenario:**
1. Withdrawals create messages at nonces N, N+1, … N+K, all burning user tokens and writing to `OutgoingMessages`.
2. Governance calls `update_outgoing_nonce(network, N-1)` to "reset" a stuck counter.
3. The next `execute_withdrawals` (e.g. from xcm-helper) reads the reset nonce → computes N → writes a NEW message into `OutgoingMessages[network][N]`, overwriting the original.
4. In-flight signatures from validators for the **old** message at slot N are stored in `SignedOutgoingMessages[network][N]`. `add_signature` compares `self.message == incoming_message` — they now differ → every new signature for the new message is silently dropped.
5. The slot can never reach finalization threshold. Tokens burned for BOTH the old and new withdrawal are permanently lost.

The audit labeled this "ReadyWithdrawals::insert overwrites existing entry" (using a conceptual name for `OutgoingMessages`).

**Changes made:**

`pallets/thea/src/lib.rs`:
- Added error variant `OutgoingMessageSlotOccupied`: returned by `execute_withdrawals` if the target nonce slot is already occupied — prevents silent overwrite
- Added error variant `OutgoingNonceBelowFinalized`: returned by `update_outgoing_nonce` if the new nonce is below the current `OutgoingNonce` — prevents the counter from being rolled back below the highest message already created
- `update_outgoing_nonce`: reads `<OutgoingNonce<T>>::get(network)` and `ensure!(nonce >= current, OutgoingNonceBelowFinalized)` before writing — provides defence-in-depth against counter rollback
- `execute_withdrawals` (in `impl TheaOutgoingExecutor`): added `ensure!(!OutgoingMessages::contains_key(network, payload.nonce), OutgoingMessageSlotOccupied)` before both the nonce update and the message insert — hard safety net regardless of nonce counter state

`pallets/thea/src/tests.rs`:
- Added `test_h7_execute_withdrawals_sequential_nonces_not_overwritten`: sends two governance messages, verifies each gets a distinct nonce and neither slot is overwritten
- Added `test_h7_update_outgoing_nonce_rejects_backwards_roll`: advances nonce to 10, verifies that `update_outgoing_nonce(9, …)` and `update_outgoing_nonce(0, …)` both return `OutgoingNonceBelowFinalized`; advancing (≥ current) still succeeds
- Added `test_h7_execute_withdrawals_refuses_to_overwrite_occupied_slot`: manually places a message at nonce 1 and resets the counter (direct storage write to simulate emergency recovery), then verifies `send_thea_message` returns `OutgoingMessageSlotOccupied` and the original message is untouched

---

### H8 — Batch deposit atomicity — one failed deposit reverts entire incoming message
**Severity:** High  
**Location:** `pallets/thea/src/lib.rs` — `on_initialize`; `primitives/thea/src/lib.rs` — `TheaIncomingExecutor`  
**Fixed in spec:** 391  
**Date:** 2026-08-18  
**Migration required:** No — pure logic change; no storage layout change.

**Vulnerability:** `on_initialize` calls `T::Executor::execute_deposits(network, data)` and ignores the return value (`execute_deposits` returned `()`). A naive executor implementation that wraps the entire SCALE-decoded deposit vector in a single `#[transactional]` block (or uses `?` inside one transaction) would cause one deposit failure to revert **all** earlier deposits in the same bridge message. Tokens on the source chain have already been burned; if the bridge message nonce advances (which it did unconditionally), the deposits are permanently unrecoverable. Additionally, any error inside the executor was silently swallowed — the nonce advanced and the relayer was refunded regardless.

This finding is latent in the current codebase (THEA executor is wired to the no-op `()` impl) but would become a live vulnerability when Hyperbridge integration wires in a real executor.

**Attack / failure scenario:**
1. A bridge message containing 10 deposits arrives; 9 are valid but 1 targets an account that doesn't exist.
2. The executor wraps all 10 inside a single `#[transactional]` block.
3. Deposit 8 (for example) panics or returns `Err` → the entire transaction rolls back.
4. `on_initialize` returns normally, advances `IncomingNonce`, refunds the relayer.
5. 9 users' tokens are permanently lost — burned on the source chain, never credited on Polkadex.

**Changes made:**

`primitives/thea/src/lib.rs`:
- Changed `fn execute_deposits(network, deposits) -> ()` to `fn execute_deposits(network, deposits) -> DispatchResult` — closes R3-H14 simultaneously; callers can now observe executor failure
- Updated `()` stub to return `Ok(())`
- Added comprehensive doc-comment on `TheaIncomingExecutor` requiring per-deposit isolation: implementations **must** process each deposit inside its own storage layer (e.g. `with_storage_layer`) so one failure does not revert others

`pallets/thea/src/lib.rs`:
- Imported `frame_support::storage::transactional::with_transaction` and `sp_runtime::TransactionOutcome`
- Added `DepositExecutionFailed(Network, u64, DispatchError)` event variant — emitted when the executor returns `Err`; includes the network, nonce, and error for operator diagnosis
- `on_initialize`: wrapped `execute_deposits` call inside `with_transaction(|| { ... TransactionOutcome::Commit/Rollback ... })` — if the executor fails (returns `Err` or panics via unwinding), all partial storage changes from the executor are rolled back atomically
- Nonce is advanced unconditionally **after** the transaction block — the message was already `take`n from `IncomingMessagesQueue`; not advancing would permanently stall the bridge for the network
- On error: emits `DepositExecutionFailed` event and still releases the relayer stake (they submitted a cryptographically valid message; executor failure is not their fault)
- Added detailed inline comments explaining the invariant

`pallets/thea/src/tests.rs`:
- Added `test_h8_on_initialize_emits_payload_processed_event_and_archives_message`: verifies success path — nonce advances, message dequeued, message archived in `IncomingMessages`, `TheaPayloadProcessed` event emitted, relayer stake released
- Added `test_h8_on_initialize_queue_always_cleared_regardless_of_stake`: verifies `IncomingMessagesQueue` is unconditionally drained and nonce advances even with zero stake
- Added `test_h8_on_initialize_sequential_messages_all_processed`: queues two messages at nonces 1 and 2, runs two `on_initialize` calls, verifies both are processed independently without the transactional wrapper causing interference

**Note on partial coverage:** The Err branch of the `deposit_result` match can only be exercised by a real executor that returns `Err`. The `()` executor always returns `Ok(())`. Full branch coverage requires a mock failing executor and should be added when Hyperbridge's token-gateway is integrated.

---

### R3-H14 — execute_deposits returns () — failures swallowed, nonce advances, relayer refunded
**Severity:** High  
**Location:** `primitives/thea/src/lib.rs` — `TheaIncomingExecutor`; `pallets/thea/src/lib.rs` — `on_initialize`  
**Fixed in spec:** 391  
**Date:** 2026-08-18  
**Fixed via:** H8 (same PR — trait return-type change was the primary mechanical fix for both findings)

**Vulnerability:** The `TheaIncomingExecutor::execute_deposits` trait method returned `()`. Because no error could propagate out, `on_initialize` had no way to detect executor failure. The nonce was advanced and the relayer's stake was released unconditionally — even if zero deposits were credited. This made deposit-message failures completely invisible on-chain.

**What the H8 fix delivers for R3-H14:**
1. **"Returns ()"** → trait now returns `DispatchResult`; the `()` stub returns `Ok(())`; failures are propagatable
2. **"Failures swallowed"** → `on_initialize` now matches on the result; `Err` emits `DepositExecutionFailed(Network, u64, DispatchError)` so the chain operator can detect, diagnose, and alert on failures
3. **"Nonce advances"** → nonce still advances on failure by design: the message is already `take`n from `IncomingMessagesQueue`; withholding the nonce would permanently stall the bridge for that network. This is a deliberate liveness trade-off, now made explicit via the event
4. **"Relayer refunded"** → relayer stake is still released on failure by design: the relayer submitted a cryptographically-valid signed message — executor failure (e.g. un-registered asset, insufficient balance) is not their fault. This is also made explicit via the event

No additional code changes required — the full fix is in the H8 commit.

---

### H1 — submit_incoming_message gated on single allowlisted test relayer
**Severity:** High  
**Location:** `pallets/thea/src/lib.rs` — `submit_incoming_message`, `AllowListTestingRelayers`  
**Fixed in spec:** 391  
**Date:** 2026-08-18  
**Migration required:** No (storage item retained; clearing migration deferred to next breaking spec upgrade).

**Vulnerability:** `AllowListTestingRelayers` stored one `AccountId` per network. `submit_incoming_message` read that entry and rejected any signer that didn't match exactly. This was a temporary testing guard that was never removed for production. Single point of failure: if the allowlisted account goes offline, is lost, or is compromised, no incoming bridge messages can be submitted for that network — permanently stalling deposits. There is also no economic competition; the single relayer has no incentive to be timely or honest beyond the stake amount.

**Attack / failure scenario:**
1. Governance sets `AllowListTestingRelayers[Ethereum] = Alice`.
2. Alice's node goes offline, or her private key is lost.
3. All ETH deposits into Polkadex halt for as long as the allowlist is not updated via governance.
4. Governance must notice the stall, propose a transaction, wait for council voting, and execute — potentially days of bridge downtime.

**Changes made:**

`pallets/thea/src/lib.rs`:
- Removed the three-line `AllowListTestingRelayers` check from `submit_incoming_message` (the `expected_signer` lookup, `ok_or(NoRelayersFound)`, and the `ensure!` call)
- Added `// SECURITY (H1)` comment explaining the open-relayer model: any signed account may now relay provided they lock `min_stake`; fisherman slashing replaces the allowlist as the honesty incentive
- `AllowListTestingRelayers` storage map: comment updated to DEPRECATED; nothing reads it post-fix
- `add_relayer_origin_for_network` extrinsic (call index 9): comment updated to DEPRECATED; retained at its original call index to avoid breaking any pending governance proposals that encode call index 9
- `NoRelayersFound` and `NotAnAllowlistedRelayer` error variants: retained at original enum positions (5 and 6) to preserve error-index encoding for existing clients; both are now unreachable by any extrinsic

`pallets/thea/src/tests.rs`:
- Added `test_h1_any_account_with_sufficient_stake_can_relay`: an account with NO entry in `AllowListTestingRelayers` submits a message and succeeds — before H1 this would have returned `NotAnAllowlistedRelayer`
- Added `test_h1_multiple_unlisted_relayers_compete_highest_stake_wins`: two un-allowlisted accounts both submit for the same nonce; the higher-staked relayer takes the slot and the lower-staked relayer's stake is released
- Added `test_h1_insufficient_stake_still_rejected`: confirms the stake guard remains intact post-fix

**Future cleanup (next breaking spec upgrade):**
- Remove `AllowListTestingRelayers` storage definition (add a clearing migration)
- Remove `add_relayer_origin_for_network` extrinsic
- Remove `NoRelayersFound` and `NotAnAllowlistedRelayer` error variants

---

### R2-H2 — Session rotation leaves outgoing message permanently unsignable
**Severity:** High  
**Location:** `pallets/thea/src/lib.rs` — `change_authorities`  
**Fixed in spec:** 391  
**Date:** 2026-08-18  
**Migration required:** No — pure logic change in `change_authorities`.

**Vulnerability:** The C8 fix (cross-set signature merging rejected by `add_signature`) combined with `submit_signed_outgoing_messages` pinning to the active set creates a permanent signing deadlock on rotation. When `ValidatorSetId` advances from N to N+1: the retiring set (id=N) can no longer submit signatures (the active set pin rejects them); the new set (id=N+1) can't add signatures either because `add_signature` drops any sig where `self.validator_set_id != validator_set_id` (stored id=N, submitted id=N+1). Any outgoing message that had not reached threshold before the rotation is permanently unsignable.

**Changes made:**

`pallets/thea/src/lib.rs` — `change_authorities`:
- After advancing `ValidatorSetId` to `new_id`, loops over all active networks and for each unfinalized nonce (from `SignedOutgoingNonce+1` to `OutgoingNonce`), updates any existing `SignedOutgoingMessages` entry: sets `validator_set_id = new_id` and clears all accumulated signatures
- The new active set can now sign those messages from scratch without hitting the `add_signature` cross-set guard
- If no signatures were accumulated yet (entry is `None`), the new set creates a fresh entry when they first sign — no action needed

`pallets/thea/src/tests.rs`:
- Added `test_r2_h2_rotation_resets_pending_signatures_to_new_set_id`: installs set A, triggers session S0→B, inserts partial signatures (set B's id) at nonce 1, triggers session S1→C, asserts `SignedOutgoingMessages[n][1].validator_set_id = 2` and `signatures.is_empty()`

---

### R2-H3 — ValidatorsRotated generated even when ScheduledRotateValidators was skipped
**Severity:** High  
**Location:** `pallets/thea/src/lib.rs` — `change_authorities`  
**Fixed in spec:** 391  
**Date:** 2026-08-18  
**Migration required:** No — pure logic change.

**Vulnerability:** `change_authorities` contains two independent `if` blocks. Block 1 (`incoming ≠ queued`) generates `ScheduledRotateValidators` ("here is the next validator set") per network; on payload-generation failure it emits an error event and `continue`s, skipping the write for that network. Block 2 (`incoming ≠ outgoing`) always generates `ValidatorsRotated` ("activate the scheduled set") for ALL networks regardless of whether block 1 succeeded. A destination chain that missed block 1's notification receives "activate the next set" without knowing what that set is, leaving the bridge in an undefined validator state.

**Changes made:**

`pallets/thea/src/lib.rs` — `change_authorities`:
- Added `block1_ran` bool (true when `incoming ≠ queued`) and `scheduled_networks: BTreeSet<Network>` to track which networks received their `ScheduledRotateValidators` payload
- In block 1, added `scheduled_networks.insert(*network)` after a successful payload write
- In block 2, skips `ValidatorsRotated` for any network where `block1_ran && !scheduled_networks.contains(&network)`: logs error and emits `UnableToGenerateValidatorSet` event — exactly the same observable signal used when block 1 fails
- When `block1_ran` is false (`incoming == queued` — a scheduled change from a previous session is being activated), all networks are processed normally

`pallets/thea/src/tests.rs`:
- Added `test_r2_h3_validators_rotated_matches_scheduled_rotate_per_network`: drives two sessions (S0: B incoming, C queued → produces nonces 1+2; S1: C incoming, C queued → produces nonce 3) and verifies each has the correct `PayloadType` (`ScheduledRotateValidators` at 1, `ValidatorsRotated` at 2 and 3)

---

### R2-H4 — fork_period accepts 0; executed messages removed so report returns MessageNotFound
**Severity:** High  
**Location:** `pallets/thea/src/lib.rs` — `add_thea_network`, `report_misbehaviour`  
**Fixed in spec:** 391  
**Date:** 2026-08-19  
**Migration required:** No — pure logic change.

**Vulnerability:** Two related defects:

1. `add_thea_network` accepted `fork_period = 0` (or 1). With `fork_period N` and a message submitted at block B, `execute_at = B + N`.  `on_initialize` of block `B + N` runs **before** any extrinsics of that block, so fishermen can only react in blocks `B+1 … B+N-1` (N − 1 blocks). With N = 0 or N = 1 the window is zero — fishermen physically cannot submit a challenge before the message is executed.

2. Once `on_initialize` executes a message it calls `IncomingMessagesQueue::take` (removing it from the queue) and stores the bare `Message` in `IncomingMessages` (the archive). `report_misbehaviour` called `IncomingMessagesQueue::take` for the same nonce and returned the generic `MessageNotFound` error when the message was already executed, hiding the true cause. The relayer's stake was released at execution time, leaving no automated slashing path.

**Changes made:**

`pallets/thea/src/lib.rs`:
- Added two new error variants: `ForkPeriodTooShort` (index 9) and `MessageAlreadyExecuted` (index 10), both with explanatory SECURITY comments
- `add_thea_network`: Added `ensure!(fork_period >= 2, Error::<T>::ForkPeriodTooShort)` before inserting the network config, with an inline comment explaining the minimum-2 timing rationale and the recommendation to use ≥ 20 blocks in production
- `report_misbehaviour`: After a queue miss, added a check of `IncomingMessages::contains_key(network, nonce)`. If present → returns `MessageAlreadyExecuted` (informing the caller that the window expired). If absent → returns `MessageNotFound` (nonce was never submitted). In both error cases the `#[transactional]` wrapper rolls back the fisherman's stake hold, so the fisherman is never charged on a failed report.

`pallets/thea/src/tests.rs`:
- Added `test_r2_h4_add_thea_network_rejects_fork_period_below_minimum`: verifies fork_period 0 and 1 return `ForkPeriodTooShort`, and fork_period 2+ is accepted
- Added `test_r2_h4_report_misbehaviour_returns_already_executed_when_in_archive`: places a message directly in `IncomingMessages`, verifies `MessageAlreadyExecuted` is returned and fisherman balance is unchanged
- Added `test_r2_h4_report_misbehaviour_returns_not_found_for_truly_unknown_nonce`: verifies absent nonce still returns `MessageNotFound` with no stake loss

**Design note — no automated clawback for already-executed messages:** Because `IncomingMessages` stores only the bare `Message` (no relayer account or stake amount), and the relayer's stake hold is released in `on_initialize` upon successful execution, there is no in-protocol mechanism to automatically slash a relayer after their message has been executed. This is a known design limitation. The correct mitigation is the fork_period minimum: if the window is always ≥ 2 blocks, honest fishermen can always intervene *before* execution. Off-chain remediation (governance slash via other means) remains available if a fraudulent message slips through under exceptional circumstances.

---

### R3-H12 — LMP market weightage never applied; epoch budget over-issued
**Severity:** High  
**Location:** `pallets/ocex/src/lib.rs` — `calculate_lmp_rewards`, `get_lmp_rewards`; `primitives/orderbook/src/lmp.rs` — `LMPEpochConfig::verify`  
**Fixed in spec:** N/A (pallet disconnected; fix ships when OCEX re-enabled in spec 393+)  
**Date:** 2026-09-08  
**Migration required:** No — pure logic fix; no storage layout change.

**Vulnerability:**  
`LMPEpochConfig` holds a `config: BTreeMap<TradingPair, LMPMarketConfig>` where each market carries a `weightage: Decimal` (e.g. 0.5 for ETH/PDEX, 0.5 for BTC/PDEX). The `verify()` check enforces that all weightages sum to exactly 1.0. However, `weightage` was never applied in the reward calculation functions — making it write-only.

**Impact:**  
`calculate_lmp_rewards` (on-chain claim path) and `get_lmp_rewards` (RPC preview path) both computed rewards as:

```
mm_rewards = total_liquidity_mining_rewards × (user_score / market_total_score)
```

Without applying `market_weightage`. Since `Σ(user_score / market_total_score) = 1.0` for each market independently, total payouts across N markets = `total_liquidity_mining_rewards × N`. With 3 active markets, the epoch budget would be over-issued by 3×. Depending on the LMP rewards pot balance, this either drains the pot faster than intended (for the first claimants) or fails with insufficient balance errors for later claimants once the pot is drained.

Additionally, `verify()` accepted `total_liquidity_mining_rewards = 0`, allowing governance to misconfigure an epoch with a zero budget — it would silently accept the transaction and distribute zero rewards to all participants.

**Changes made:**

`pallets/ocex/src/lib.rs`:
- `calculate_lmp_rewards`: Added `market_weightage = config.config.get(&market).map(|c| c.weightage).unwrap_or_default()`; applied it as a multiplier before `market_making_portion` and `trading_rewards_portion`. Total payout across all markets now correctly equals the epoch budget.
- `get_lmp_rewards` (RPC path): Same fix applied so the preview shown to users matches what they will actually receive on-chain.

`primitives/orderbook/src/lmp.rs`:
- `verify()`: Added `total_liquidity_mining_rewards <= 0` guard — returns `false` so governance submission is rejected. `total_trading_rewards < 0` is also rejected (negative values are nonsensical); zero is allowed for epochs that have no trading rewards component.

---

### R3-H13 — close_auction non-transactional; place_bid commented out
**Severity:** High  
**Location:** `pallets/ocex/src/lib.rs` — `close_auction`, `place_bid`  
**Fixed in spec:** N/A (pallet disconnected; fix ships when OCEX re-enabled in spec 393+)  
**Date:** 2026-09-08  
**Migration required:** No — pure logic change; no storage layout change.

**Vulnerability:**  
`close_auction` performs a sequence of operations: iterates fee-asset transfers from the auction pot to the highest bidder, then calls `NativeCurrency::settle` to burn the native token amount. The function had no `#[transactional]` attribute.

**Impact:**  
If any operation in the sequence fails midway — for example `NativeCurrency::settle` returns `Err` after several fee-asset transfers have already completed — those earlier transfers are already committed to storage. The bidder receives assets for free (the native payment never burned) and the auction pallet's internal state is left inconsistent. Without the transactional savepoint, there is no rollback path.

`place_bid` is separately fully commented out at call index 22 pending frontend readiness. It also performs a multi-step operation (`reserve` new bidder → `unreserve` old bidder → `Auction::put`) with the same non-transactional risk: if `Auction::put` or `unreserve` fail after `reserve` has committed, the new bidder's funds remain locked with no auction entry pointing at them.

**Changes made:**

`pallets/ocex/src/lib.rs`:
- Added `#[transactional]` attribute to `close_auction` — wraps the entire function in a storage-layer savepoint so any failure rolls back all partial fee-asset transfers atomically
- Added detailed security comment above the commented-out `place_bid` block documenting the transactional requirement and the `#[transactional]` line that must be included when the extrinsic is re-enabled. Added `// #[transactional]` inside the commented block so it ships with the correct attribute when uncommented

---

### R3-H6 — LMP callbacks look up Pools by pool_id but storage is keyed by market_maker
**Severity:** High  
**Location:** `pallets/liquidity-mining/src/callback.rs` — `add_liquidity_success`, `remove_liquidity_failed`, `pool_force_close_success`  
**Fixed in spec:** 391  
**Date:** 2026-08-17  
**Fixed via:** C6 (same commit — the PoolIdIndex reverse map introduced for C6 directly resolves this finding)

**Vulnerability:** OCEX calls all three LMP callbacks with the *pool_id* (the derived PalletId sub-account) as the `pool` parameter. But the LMP `Pools` storage map is keyed by `(TradingPair, market_maker)`. A direct `Pools::get(market, pool_id)` would always return `None`, causing all three callbacks to silently fail.

**Impact:** Share minting, share refund on failed removal, and pool-force-close state update all silently no-op'd whenever the pool_id was passed — which is every call.

**Changes made (as part of C6):**

`pallets/liquidity-mining/src/callback.rs`:
- All three callbacks now perform a reverse-index lookup via `PoolIdIndex::get(pool_id)` to resolve `market_maker`, then look up `Pools::get(market, market_maker)` with the correct key

---

### R3-H7 — remove_liquidity_failed re-scales already-planck shares — mints 10¹²× shares
**Severity:** High  
**Location:** `pallets/liquidity-mining/src/callback.rs` — `remove_liquidity_failed`  
**Fixed in spec:** N/A (pallet disconnected; fix ships when OCEX re-enabled in spec 393+)  
**Date:** 2026-09-08  
**Migration required:** No — pure logic fix; no storage layout change.

**Vulnerability:**  
`remove_liquidity` queues `(lp, burned_amt, total)` in `WithdrawalRequests` where both values are `BalanceOf<T>` — planck units. OCEX's `lmp.rs::remove_liquidity` converts these to `Decimal` via `Decimal::from(burned.saturated_into::<u128>())` **without dividing by UNIT_BALANCE**, so the matching engine receives and stores shares in planck units (e.g., `10_500_000_000_000` not `10.5`). The matching engine echoes these planck values back in `RemoveLiquidityFailed`.

**Impact:**  
The `remove_liquidity_failed` callback then computed:
```
shares_burned = total_shares × burn_frac × UNIT_BALANCE
             = planck_total × frac × 10¹²
```
The extra `× UNIT_BALANCE` inflated the minted-back share count by 10¹². An LP holding 10.5 shares (10_500_000_000_000 planck) after a failed removal would receive 10_500_000_000_000_000_000_000_000 planck shares back — effectively infinite supply inflation of the share token.

**Changes made:**

`pallets/liquidity-mining/src/callback.rs`:
- Removed the `.saturating_mul(Decimal::from(UNIT_BALANCE))` from the `shares_burned` conversion — casts the Decimal directly to u128 (already in planck) before calling `mint_into`

---

### R3-H8 — force_close_pool passes market_maker to OCEX instead of pool_id
**Severity:** High  
**Location:** `pallets/liquidity-mining/src/lib.rs` — `force_close_pool`  
**Fixed in spec:** N/A (pallet disconnected; fix ships when OCEX re-enabled in spec 393+)  
**Date:** 2026-09-08  
**Migration required:** No — pure logic fix; no storage layout change.

**Vulnerability:**  
`force_close_pool` called `T::OCEX::force_close_pool(market, market_maker)` with the market maker's personal account. The matching engine expects the `pool_id` (the derived PalletId sub-account). It stores `ForceClosePool(config, market_maker)` in `IngressMessages`.

**Impact:**  
When OCEX processes the egress `PoolForceClosed` message, it validates `is_valid_pool_id(pool)` which checks `PoolIdIndex::contains_key(pool)`. The market_maker's personal account is never in `PoolIdIndex` — only pool sub-accounts are. The validation fails and the force-close egress is rejected, leaving the pool stuck in limbo even after the governance root called `force_close_pool`. The pool remains open to new activity while governance believes it is closed.

**Changes made:**

`pallets/liquidity-mining/src/lib.rs`:
- `force_close_pool`: Look up `pool_config` via `Pools::get(market, market_maker)` and pass `pool_config.pool_id` to `T::OCEX::force_close_pool` — removed the now-redundant `Pools::contains_key` guard (the `get` returns an error if not found)

---

### R3-H11 — claim_force_closed_pool_funds reads market.base for both asset legs
**Severity:** High  
**Location:** `pallets/liquidity-mining/src/lib.rs` — `claim_force_closed_pool_funds`  
**Fixed in spec:** N/A (pallet disconnected; fix ships when OCEX re-enabled in spec 393+)  
**Date:** 2026-09-08  
**Migration required:** No — pure logic fix; no storage layout change.

**Vulnerability:**  
`claim_force_closed_pool_funds` read both `base_balance` and `quote_balance` using `market.base.asset_id()`. The `quote_balance` line erroneously used `market.base` instead of `market.quote`.

**Impact:**  
`quote_amt_to_claim` was computed from the base asset's pool balance, not the quote asset. The function then transferred `quote_amt_to_claim` (based on base) from the **quote** asset of the pool to the LP — the LP receives the wrong amount from the quote leg (typically a massive overpay or shortfall depending on the base/quote price ratio), and the correct quote balance is never drawn down, leaving quote funds permanently stuck in the pool sub-account.

**Changes made:**

`pallets/liquidity-mining/src/lib.rs`:
- `claim_force_closed_pool_funds`: Changed the `quote_balance` `reducible_balance` call from `market.base.asset_id()` to `market.quote.asset_id()`

---

### R3-H3 — Aggregator HTTP response uncapped
**Severity:** High  
**Location:** `pallets/ocex/src/aggregator.rs` — `send_request`  
**Fixed in spec:** N/A (node binary fix, no on-chain migration)  
**Date:** 2026-09-08  
**Migration required:** No

**Vulnerability:**  
`send_request` called `response.body().collect::<Vec<u8>>()` with no size limit. `ResponseBody` is an `Iterator<Item = u8>` — `.collect()` reads every byte the aggregator returns into a heap-allocated `Vec` before any processing happens.

**Impact:**  
A compromised or malicious aggregator endpoint (MITM, DNS hijack, or a rogue operator) could return an arbitrarily large HTTP body — gigabytes if needed. The validator's OCW would attempt to allocate that entire response into memory before checking any content, leading to an out-of-memory crash of the validator process. Since the OCW runs inside the node process (not a separate sandbox), an OOM kill takes the entire validator offline. All three aggregator endpoints (`/snapshots`, `/latest_checkpoint`, `/submit_snapshot`) called `send_request` and were equally affected.

**Changes made:**

`pallets/ocex/src/aggregator.rs`:
- Added `const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024` (10 MB) — generous for any legitimate snapshot or checkpoint, which are typically < 1 MB
- Replaced `.collect::<Vec<u8>>()` with `.take(MAX_RESPONSE_BYTES + 1).collect()` — reads at most one byte beyond the cap so truncation is detectable
- Added an explicit size check: if `body.len() > MAX_RESPONSE_BYTES`, logs an error naming the endpoint and returns `Err("aggregator response too large")` — prevents silent partial-response processing
- All three aggregator call sites (`get_user_action_batch`, `get_checkpoint`, `load_signed_summary_and_send`) benefit automatically as they all go through `send_request`

---

### R3-H2 — OCW mutex retry backoff missing; Dev RPC always exposed
**Severity:** High  
**Location:** `pallets/ocex/src/rpc.rs`, `nodes/mainnet/src/node_rpc.rs`  
**Fixed in spec:** N/A (node binary fix, no on-chain migration)  
**Date:** 2026-09-08  
**Migration required:** No

**Vulnerability 1 — Mutex retry backoff missing:**  
`acquire_offchain_lock` logs "retrying after 1 sec" but had no `sleep_until` between the 3 retry iterations. All 3 retries ran back-to-back in microseconds.

**Impact:** If the OCW lock was held by a running worker (normal during active block processing), all 3 retries would see `WORKER_STATUS = true` and fail in under a millisecond — identical to having no retries at all. The calling worker returned `Err` immediately, skipped snapshot processing for that block, and the orderbook OCW silently fell behind. On a busy validator node where the OCW regularly ran longer than one block time, this caused systematic snapshot processing gaps that could desync the off-chain state from the on-chain snapshot.

**Vulnerability 2 — Dev RPC registered unconditionally:**  
`sc_rpc::Dev` is explicitly labelled "All methods are unsafe" in the Substrate source. It was registered unconditionally in `create_full` on every node including production validators. Any caller who could reach port 9944 had access to Dev API methods regardless of whether `--unsafe-rpc-methods` was set.

**Impact:** The Dev API exposes `dev_getBlockStats` which reveals internal block execution details (extrinsic weights, storage proof sizes, DB reads/writes per block). On a validator with port 9944 publicly reachable, an attacker could use this to profile the validator's block execution timing and resource usage — information useful for targeted DoS (crafting extrinsics that maximise validator load) or for inferring validator hardware and configuration. The Polkadex README already warns operators to firewall port 9944 after key submission (`ufw deny 9944`), but nothing prevented a misconfigured validator from leaving it open and unknowingly exposing the Dev API.

**Changes made:**

`pallets/ocex/src/rpc.rs`:
- Added `sp_io::offchain::sleep_until(timestamp + 1s)` between each retry iteration in `acquire_offchain_lock` so the log message matches the actual behaviour and the lock can be released by the running worker between attempts

`nodes/mainnet/src/node_rpc.rs`:
- Moved `use sc_rpc::dev::{Dev, DevApiServer}` behind `#[cfg(feature = "dev-rpc")]`
- Moved `io.merge(Dev::new(client.clone()).into_rpc())?` behind `#[cfg(feature = "dev-rpc")]`

`nodes/mainnet/Cargo.toml`:
- Added `dev-rpc = []` feature with a security comment — never enable on production validator nodes

---

## One-shot migrations — must be removed before spec 392

The following migrations are in `runtimes/mainnet/src/migrations.rs` and wired into the `Migrations` tuple in `runtimes/mainnet/src/lib.rs`. They are idempotent but execute on **every** runtime upgrade. They must be **removed from the tuple before the next spec version bump** (≥ 392):

| Migration | Added for | Safe to remove after |
|---|---|---|
| `PruneStaleIngressMessages` | C9 | After spec-391 upgrade has run on mainnet |
| `RebuildLmpPoolIdIndex` | C6 | After spec-391 upgrade has run on mainnet |

To remove them: delete the two entries from the `type Migrations = (...)` tuple in `runtimes/mainnet/src/lib.rs` and delete the corresponding structs from `migrations.rs`.

---

### R2-H5 — on_initialize runs unbounded deposit batch; xcm-helper drains unbounded queue
**Severity:** High  
**Location:** `pallets/xcm-helper/src/lib.rs`  
**Fixed in spec:** 391  
**Date:** 2026-08-19  
**Migration required:** No — pure logic change.

**Vulnerability:** Two related issues in xcm-helper:

1. **Trait signature mismatch:** `xcm_helper::Pallet<T>` implemented `TheaIncomingExecutor::execute_deposits` with return type `()`, but the trait was updated (H8 / R3-H14 fix) to require `DispatchResult`. This caused a trait-mismatch compile error whenever xcm-helper was compiled against the updated primitives.

2. **Unbounded drain:** `handle_new_pending_withdrawals` used `while let Some(withdrawal) = withdrawals.pop()` which drains the entire `PendingWithdrawals[n]` Vec in one hook invocation. The weight returned was hardcoded at `MAXIMUM_BLOCK_WEIGHT / 4` regardless of queue depth. A relayer could inject a large deposit batch into `execute_deposits`, schedule thousands of withdrawals for the same future block, and stall that block's execution.

3. **Silent decode failures:** `execute_deposits` used `Vec::<Withdraw>::decode(...).unwrap_or_default()` — a malformed payload silently produced an empty list, advancing the nonce and releasing the relayer's stake with no indication of failure.

**Changes made:**

`pallets/xcm-helper/src/lib.rs`:
- Added `MaxWithdrawalsPerBlock: Get<u32>` to `Config` trait (with `#[pallet::constant]`)
- `TheaIncomingExecutor::execute_deposits`: Changed return type to `DispatchResult`; replaced `unwrap_or_default()` with explicit `map_err` returning a descriptive `DispatchError::Other`; added batch-size guard — rejects any batch with `len() > MaxWithdrawalsPerBlock`
- `handle_new_pending_withdrawals`: Changed from an unbounded `mutate`-based drain to a bounded `take`-then-split approach. After `take`-ing all withdrawals for block `n`, items beyond `MaxWithdrawalsPerBlock` are re-queued to block `n+1` with a `log::warn!`. Overflow is never silently dropped.
- `on_initialize`: Added explanatory comment; TODO noting that the weight should be made proportional to items actually processed

`pallets/xcm-helper/src/mock.rs`:
- Added `MaxWithdrawalsPerBlock = 100` `parameter_types!` entry
- Added `type MaxWithdrawalsPerBlock = MaxWithdrawalsPerBlock;` to the test Config impl

**Notes:**
- xcm-helper has pre-existing compilation failures due to XCM API version drift (MultiAsset / MultiLocation paths changed between staging-xcm versions) and pallet_balances Config API changes. These are separate issues not introduced by this fix — xcm-helper was already excluded from the workspace (`pallets/xcm-helper` not in workspace members).
- The THEA `on_initialize` batch-weight issue (part 1 of R2-H5) is a separate concern: THEA processes one message per network per block, and the `WeightInfo::on_initialize(active_networks.len())` weight doesn't account for the payload size within each message. Since THEA is currently commented out of the mainnet runtime, this is documented as a TODO in the THEA `on_initialize` code.

---

### R3-H9 — Global asset registry; weakest network mints any bridged asset
**Severity:** High  
**Location:** `pallets/thea-executor/src/lib.rs` (remote repo: `Polkadex-Substrate/Polkadex`)  
**Date:** 2026-08-19

**Vulnerability:** `Metadata` was a `StorageMap<AssetId, AssetMetadata>` keyed only by `AssetId`.  Any active bridge network could submit a deposit claiming any `AssetId` already registered, causing `execute_deposit` to mint tokens even though the depositing network is not the canonical issuer of that asset.  A compromised low-security chain (e.g., a testnet-grade parachain) could mint mainnet Ethereum-bridged tokens.

**Changes made (remote repo `pallets/thea-executor/src/lib.rs`):**
- Changed `Metadata` from `StorageMap<AssetId>` to `StorageDoubleMap<Network, AssetId>` with a security comment block
- Added `STORAGE_VERSION` constant (bumped to 1) and `#[pallet::storage_version]` attribute
- `update_asset_metadata`: added `network: Network` as second parameter; callers now register each `(network, asset_id)` pair independently; updated doc comment
- `create_parachain_asset`: registers metadata under `PARACHAIN_NETWORK`
- `do_deposit` → `execute_deposit`: threaded `network` through so deposit metadata is looked up as `Metadata[network][asset_id]`
- `execute_deposit`: added `network: Network` parameter
- `do_withdraw`: scoped both asset and fee-asset metadata lookups to `network`
- `claim_deposit`: added `network: Network` parameter; passed to `execute_deposit`
- `TheaBenchmarkHelper::set_metadata`: registers under `ETHEREUM_NETWORK`
- Added `migrations::MigrateMetadataToDoubleMap` — copies all v0 `Metadata[asset_id]` entries to `Metadata[ETHEREUM_NETWORK][asset_id]`, bumps storage version to 1; includes `try-runtime` pre/post upgrade checks
- `benchmarking.rs`: updated `Metadata::insert` and `claim_deposit` call to use network keys
- `tests.rs`: updated all 15 tests to pass `network` to `update_asset_metadata`, `execute_deposit`, `claim_deposit`; updated `Metadata` storage reads/writes; all 15 tests pass

**Commit:** `5c334ada` (remote repo, branch `mainnet-release`)

---

### R4-C — crowdloan verifier: reader exhausted, wrong columns, always prints success
**Severity:** High  
**Location:** `misc/crowdloan-verifier/src/main.rs`  
**Fixed in spec:** N/A (off-chain tool)  
**Date:** 2026-09-09  
**Migration required:** No — off-chain script only.

**Vulnerability:**  
The verification branch (`!args.convert`) had three stacked bugs that rendered it completely non-functional:

1. **Reader exhausted**: The first `for result in rdr.records()` loop (building the dedup map) consumed the entire CSV reader. `csv::Reader` is an iterator — once exhausted, subsequent iterations yield nothing. The second `for result in rdr.records()` loop at line 103 silently iterated zero rows.

2. **Wrong column indices**: The second loop read `record.get(1)` as `total_rewards`, `record.get(2)` as `cliff_amt`, `record.get(3)` as `claim_per_blk`, and `record.get(4)` as `dot_contributed`. The actual CSV layout is `col0=accountId, col1=DOTs contributed, col2=Total PDEX, col3=Initial cliff, col4=Factor` — so the second loop would have read DOT amounts as PDEX totals, etc.

3. **Always printed success**: Because the second loop never ran (bug 1), execution fell through to `println!("Excel and Source code account lists match, All good!")` unconditionally — regardless of any mismatches between the CSV and the HASHMAP.

**Impact:**  
The verifier tool gave false confidence that the hardcoded `HASHMAP` in `crowdloan_rewardees.rs` correctly matched the source CSV. Any discrepancy (wrong amounts, missing accounts, transposed entries) would be silently reported as success. This tool was the only automated check between the source spreadsheet and the on-chain reward allocations for 3,631 crowdloan contributors.

**Changes made:**

`misc/crowdloan-verifier/src/main.rs`:
- Removed the second `rdr.records()` loop entirely — the reader cannot be rewound
- After the single-pass dedup map is built (using the correct columns 2, 3, 4), verify each map entry against HASHMAP directly using the already-collected values
- Added `found_error` flag — errors are now all printed (not just the first), and the process exits with code 1 at the end if any mismatch was found
- Success message (`All good!`) is now only printed when `found_error` is false

---

### R4-B — crowdloan HASHMAP accessible for any reward_id; full re-pay on second cycle
**Severity:** High  
**Location:** `pallets/rewards/src/lib.rs` — `do_initialize_claim_rewards`  
**Fixed in spec:** N/A (no migration; fix active on next node deploy)  
**Date:** 2026-09-09  
**Migration required:** No — pure logic fix; no storage layout change.

**Vulnerability:**  
`do_initialize_claim_rewards` looked up the caller in `crowdloan_rewardees::HASHMAP` regardless of the `reward_id` argument. The `Distributor` storage is a double map keyed by `(reward_id, AccountId)`, so the existing guard (`contains_key(reward_id, user)`) only prevents double-initialization for the **same** `reward_id`. It does not prevent a contributor from initializing under a different `reward_id`.

**Impact:**  
Whenever governance calls `create_reward_cycle` with a new `reward_id` (e.g., `2` for a second parachain auction or a different rewards programme), all 3,631 accounts in the crowdloan HASHMAP can call `initialize_claim_rewards(2)`. Each receives the same `total_rewards_in_pdex`, `initial_rewards_claimable`, and `factor` as the first crowdloan — the pallet transfers that amount from its account and sets a new lock. With ~2M PDEX total across contributors, a second cycle would fully drain the rewards pot and mint a second round of crowdloan allocations to all 3,631 participants.

**Changes made:**

`pallets/rewards/src/lib.rs`:
- Added `const CROWDLOAN_REWARD_ID: u32 = 1` with a security comment explaining the guard
- Added `Error::NotCrowdloanRewardId` variant to the Error enum
- Added `ensure!(reward_id == CROWDLOAN_REWARD_ID, Error::<T>::NotCrowdloanRewardId)` at the top of `do_initialize_claim_rewards` — rejects any call that supplies a `reward_id` other than the first crowdloan cycle before reaching the HASHMAP lookup

---

### H3 — do_claim removes entire vesting lock on first claim — 96-week vest bypassed
**Severity:** High  
**Location:** `pallets/rewards/src/lib.rs` — `do_claim`  
**Fixed in spec:** N/A (no migration needed; fix active on next node deploy)  
**Date:** 2026-09-09  
**Migration required:** No — pure logic fix; no storage layout change.

**Vulnerability:**  
`do_initialize_claim_rewards` transfers the user's full `total_reward_amount` to their account and places a `LockableCurrency` lock on the entire amount (`WithdrawReasons::TRANSFER`). The intent is a 96-week linear vest: each `do_claim` call should progressively reduce the lock by the vested portion (`factor × blocks_elapsed`), making only the vested portion transferable.

`do_claim` had two stacked bugs:
1. `_reward_info` — the `InitializeRewards` entry containing `end_block` was fetched and immediately discarded (underscore prefix, never used).
2. Instead of computing `factor × unclaimed_blocks`, it computed `total_reward_amount - claim_amount` — the entire remaining allocation — and called `remove_lock` (not `set_lock`) to remove the entire lock in one shot.

**Impact:**  
On the very first call to `do_claim` after initialization, the user's full vesting lock was removed unconditionally. All `total_reward_amount` tokens became immediately transferable regardless of where in the 96-week schedule the user was. A user who initialized at block 1 and claimed at block 2 received the same outcome as a user who waited 4,838,400 blocks — zero vesting was enforced. The crowdloan reward schedule for all 3,631 contributors was effectively a no-op; everyone could claim 100% of their allocation immediately after initialization.

**Changes made:**

`pallets/rewards/src/lib.rs` — `do_claim`:
- Renamed `_reward_info` to `reward_info` so `end_block` is accessible
- Replaced the `total - claimed` calculation with the correct pro-rated formula: `factor × min(current_block, end_block) - last_claimed_block`
- Added a cap at `total - claim_amount` to handle rounding on the final claim
- Replaced unconditional `remove_lock` with: `set_lock(lock_id, user, total - new_claim_amount)` when unvested tokens remain; `remove_lock` only once `claim_amount >= total_reward_amount`
- `is_initial_rewards_claimed` is now set to `true` in the same branch where `initial_rewards_claimable` is added, rather than unconditionally on every call

---

### H5 — initiate_withdrawal panics when num_requests exceeds queue length
**Severity:** High  
**Location:** `pallets/liquidity-mining/src/lib.rs` — `initiate_withdrawal`  
**Fixed in spec:** N/A (pallet disconnected; fix ships when LMP re-enabled in spec 393+)  
**Date:** 2026-09-09  
**Migration required:** No — pure logic fix; no storage layout change.

**Vulnerability:**  
`initiate_withdrawal` takes a caller-supplied `num_requests: u16` and uses it to slice the withdrawal queue:
```rust
requests = requests[num_requests..].to_vec();
```
Rust slice indexing panics with an out-of-bounds error if `num_requests > requests.len()`. The `.iter().take(num_requests)` above it is safe (take clamps to available items), but the slice is not.

**Impact:**  
Any signed account that is a registered market maker can call `initiate_withdrawal` with a `num_requests` larger than the current queue — for example, `u16::MAX` (65535). This triggers an unrecoverable panic in the WASM runtime for that block. In Substrate's no_std WASM environment, an uncaught panic aborts block execution, which can stall block production on the affected node and, if triggered on multiple validators simultaneously, halt the chain for the duration it takes operators to notice and redeploy.

**Changes made:**

`pallets/liquidity-mining/src/lib.rs`:
- Added `let num_requests = num_requests.min(requests.len());` before the slice and the iteration loop — clamps the effective request count to the actual queue length so both the `take()` and the slice remain in bounds

---

### H6 — dev_mode attribute live on production pallet; unbounded storage
**Severity:** High  
**Location:** `pallets/liquidity-mining/src/lib.rs`  
**Fixed in spec:** N/A (pallet disconnected; fix ships when LMP re-enabled in spec 393+)  
**Date:** 2026-09-09  
**Migration required:** No — compile-time fix only.

**Vulnerability:**  
The pallet was declared with `#[frame_support::pallet(dev_mode)]`. In FRAME, `dev_mode` relaxes multiple production safety checks simultaneously: it allows omitting explicit call indices, allows shorthand weight expressions, and — most critically — implies `without_storage_info`, which suppresses the `MaxEncodedLen` requirement on all storage value types. Three storage maps held unbounded types:

- `AddLiquidityRecords`: `Vec<(BlockNumber, Balance)>` — no size cap  
- `WithdrawalRequests`: `Vec<(AccountId, Balance, Balance)>` — no size cap  
- `LiquidityProviders` / `MMInfo`: `BTreeMap<AccountId, (MMScore, MMClaimFlag)>` — no size cap  

`dev_mode` was silently hiding all three as compile-time non-errors.

**Impact:**  
`dev_mode` masks the unbounded storage issues so they never surface as compiler errors. Without size caps, any of these maps can grow without bound — a malicious or buggy caller can bloat storage until decoding them in a single extrinsic exceeds the block weight limit, causing that extrinsic to always fail (effectively bricking operations that touch large entries). Additionally having `dev_mode` on a production pallet is a latent risk: future FRAME versions may change what it relaxes, and new code paths might inadvertently rely on the relaxed behaviour.

**Changes made:**

`pallets/liquidity-mining/src/lib.rs`:
- Replaced `#[frame_support::pallet(dev_mode)]` with `#[frame_support::pallet]` — `dev_mode` is gone
- Added `#[pallet::without_storage_info]` on the `Pallet<T>` struct as an explicit, narrow opt-out of the `MaxEncodedLen` requirement only. Unlike `dev_mode`, this does not relax call-index, weight, or any other production check
- Added a detailed TODO comment listing the three storage types that must be converted to `BoundedVec` / `BoundedBTreeMap` before the pallet is re-enabled at spec 393+

### H2 — Third approver's beneficiary/amount used regardless of earlier approvals
**Severity:** High  
**Location:** `pallets/pdex-migration/src/lib.rs` — `BurnTxDetails`, `process_migration`  
**Fixed in spec:** N/A (pdex-migration pallet)  
**Date:** 2026-09-10

**Vulnerability:**  
`BurnTxDetails` only stored `approvals` (count) and `approvers` (list of relayer accounts). It did not record the `beneficiary` or `amount` submitted by the first two approvers. When the third (final) approver called `mint(beneficiary, amount, eth_tx)`, the code used that relayer's own parameters unconditionally to mint and lock tokens — never comparing them against what the first two relayers agreed on.

An attacker who controls one relayer key could wait for two honest relayers to submit approvals for a legitimate burn, then be the third approver and supply a different `beneficiary` (redirecting the tokens to an attacker-controlled account) or a different `amount` (minting more tokens than were burned on Ethereum).

**Impact:**  
A single malicious relayer acting as the third approver can redirect newly minted PDEX to an arbitrary account or inflate the minted amount, bypassing the intended 2-of-3 consensus entirely. This is a complete break of the multi-relayer trust model and could drain all `MintableTokens`.

**Changes made:**

`pallets/pdex-migration/src/lib.rs`:
- Added `beneficiary: Option<AccountId>` and `amount: Option<Balance>` fields to `BurnTxDetails`, plus a new type parameter `Balance`; updated all generic bounds, `Default` impl, `EthTxns` storage type, and `process_migration` signature accordingly
- `process_migration`: On first approval (`None, None`), stores the caller's `beneficiary` and `amount` in the struct. On subsequent approvals, enforces `beneficiary == stored_beneficiary && amount == stored_amount` before accepting the approval — returns new `ApprovalParamsMismatch` error if they differ
- When all three approvals are collected, reads `beneficiary` and `amount` from the struct (not the current caller's parameters) to perform the mint and lock — so even a misbehaving third relayer that somehow passed the check cannot influence the final target
- Added `ApprovalParamsMismatch` error variant

### H9 — XCM deposit_asset accepts any token — whitelist check never enforced
**Severity:** High  
**Location:** `pallets/xcm-helper/src/lib.rs` — `deposit_asset`  
**Fixed in spec:** N/A (xcm-helper pallet)  
**Date:** 2026-09-10

**Vulnerability:**  
The pallet has a `WhitelistedTokens` storage and a `check_whitelisted_token` function intended to gate which foreign assets may enter Polkadex via XCM. However, `deposit_asset` — the XCM executor entry point that handles every incoming cross-chain deposit — never called `check_whitelisted_token`. Any parachain could push arbitrary tokens into the system by crafting an XCM deposit message.

**Impact:**  
Without the whitelist gate, any foreign chain or relay-chain parachain can inject unrecognised tokens into Polkadex accounts via XCM. This includes spam tokens that pollute balances, crafted asset IDs that could shadow legitimate assets registered in the `ParachainAssets` map, and any future asset whose price can be manipulated before governance has time to assess it. The `WhitelistedTokens` extrinsic and storage existed entirely dead — the check was defined but never wired into the deposit path.

**Changes made:**

`pallets/xcm-helper/src/lib.rs`:
- In `deposit_asset`, added a `check_whitelisted_token(asset_id)` guard immediately after `asset_id = Self::generate_asset_id_for_parachain(*id)` — before either the sibling-parachain or standard deposit branch. Returns `XcmError::AssetNotFound` and logs the rejected asset ID if the token is not whitelisted

### M4 — payload.action domain separator never checked — cross-call signature replay
**Severity:** Medium  
**Location:** `pallets/rewards/src/lib.rs` — `validate_unsigned_claim`, `validate_unsigned_initialize_claim_rewards`  
**Date:** 2026-09-10

**Vulnerability:** `ExchangePayload` has an `action: ExchangePayloadAction` field (`Initialize` or `Claim`) included in the signed message. Both validator functions verified the cryptographic signature but never checked that `payload.action` matched the call's intended action. A signature over an `Initialize` payload could be replayed as a `Claim` call and vice versa.

**Impact:** An attacker holding a valid `Initialize` signature (obtained from a user or observed on-chain) could replay it as `unsigned_claim`, calling `do_claim` on the user's account. Similarly, a `Claim` signature could trigger `do_initialize_claim_rewards` a second time, potentially overwriting the user's reward state.

**Changes made:**
`pallets/rewards/src/lib.rs`:
- `validate_unsigned_claim`: added `ExchangePayloadAction::Claim` check as the first gate, returning `InvalidTransaction::Custom(0)` on mismatch
- `validate_unsigned_initialize_claim_rewards`: added `ExchangePayloadAction::Initialize` check, same error code

---

### M5 — AssetId(0) aliasing: From<u128>(0) maps to Polkadex; TryFrom<String>("0") maps to Asset(0)
**Severity:** Medium  
**Location:** `primitives/polkadex/src/assets.rs` — `TryFrom<String> for AssetId`  
**Date:** 2026-09-10

**Vulnerability:** `From<u128>(0)` returned `AssetId::Polkadex` (correct) but `TryFrom<String>("0")` called `AssetId::Asset(0)` directly (incorrect). Code that stored or compared an `AssetId` obtained via one path against the other would silently diverge on the native token.

**Changes made:**
`primitives/polkadex/src/assets.rs`:
- `TryFrom<String>`: replaced `Ok(AssetId::Asset(id))` with `Ok(AssetId::from(id))` so the string and u128 paths are always consistent

---

### M6 — Cross-chain decimal truncation floors to zero; small deposits credited as zero
**Severity:** Medium  
**Location:** `primitives/thea/src/types.rs` — `AssetMetadata::convert_to_native_decimals`  
**Date:** 2026-09-10

**Vulnerability:** When converting from a higher-precision foreign chain (e.g., 18dp Ethereum → 12dp Polkadex), the function divided by an integer power of 10. Any amount smaller than the divisor (e.g., less than 10^6 wei for 18dp→12dp) would floor to exactly 0. The old return type `u128` gave callers no way to distinguish "zero input" from "amount too small to represent" — so a deposit of a non-zero foreign amount could result in zero PDEX credited while the source-chain funds were already burned.

**Changes made:**
`primitives/thea/src/types.rs`:
- `convert_to_native_decimals` return type changed from `u128` to `Option<u128>` — returns `None` when result is zero but input was non-zero (precision underflow)
- `Deposit::amount_in_native_decimals` updated to propagate `Option<u128>`
- Tests updated to use `Some(value)` and a new assertion confirms `999_999 wei (18dp) → None`

---

### M7 — Order Ord not a total order — equal (price, timestamp) compares Less in both directions
**Severity:** Medium  
**Location:** `primitives/orderbook/src/types.rs` — `impl Ord for Order`  
**Date:** 2026-09-10

**Vulnerability:** When `price` was equal and `timestamp` was also equal, the `else` branch always returned `Ordering::Less`. This meant `cmp(a, b) == Less` AND `cmp(b, a) == Less` — violating the antisymmetry requirement of `Ord`. Any `BTreeMap` or `BinaryHeap` keyed by `Order` could exhibit undefined behaviour under equal-priority entries.

**Changes made:**
`primitives/orderbook/src/types.rs`:
- Replaced the `if self.timestamp < other.timestamp { Greater } else { Less }` pattern on both Bid and Sell sides with a full three-way match: `Less → Greater`, `Equal → Equal`, `Greater → Less`

---

### M8 — sub_balance fails open on withdrawal — over-withdrawal silently capped, masking insolvency
**Severity:** Medium  
**Location:** `pallets/ocex/src/settlement.rs` — `sub_balance`  
**Date:** 2026-09-10

**Vulnerability:** For withdrawal operations (`is_withdrawal = true`), if the requested amount exceeded the available balance by any magnitude, the function silently capped the withdrawal to the available amount and logged a warning. There was no limit on the acceptable deviation, so a real insolvency (trie holding less than users are owed) would be absorbed and hidden.

**Changes made:**
`pallets/ocex/src/settlement.rs`:
- Added a `rounding_tolerance = Decimal::new(1, 9)` (1e-9 units) cap on acceptable deviation
- Deviations within tolerance: existing behaviour (cap + warn log)
- Deviations above tolerance: return `Err("NotEnoughBalance: withdrawal exceeds available balance beyond rounding tolerance")` with a SECURITY-tagged error log

---

### M9 — Trie read-after-remove stale — get() ignores keys_to_remove; deleted keys appear to exist
**Severity:** Medium  
**Location:** `pallets/ocex/src/storage.rs` — `OffchainState::get`  
**Date:** 2026-09-10

**Vulnerability:** `OffchainState::remove(key)` added the key to `keys_to_remove` and cleared it from the write cache. But `get(key)` only checked the cache, not `keys_to_remove` — a miss in the cache caused a fall-through to `self.trie.get(key)`, which returned the stale pre-deletion trie value until the next `commit()`. Logically-deleted accounts appeared to still exist with their old balances.

**Changes made:**
`pallets/ocex/src/storage.rs`:
- Added `if self.keys_to_remove.contains(key) { return Ok(None); }` at the top of `OffchainState::get`

---

### M10 — add_balance skips rounding on first credit — leaves sub-9dp dust in trie
**Severity:** Medium  
**Location:** `pallets/ocex/src/settlement.rs` — `add_balance`  
**Date:** 2026-09-10

**Vulnerability:** `.and_modify(|total| *total = Order::rounding_off(...))` correctly rounded on subsequent credits, but `.or_insert(balance)` stored the raw unrounded value on the first credit for a given asset. Trade calculations produce 9dp-rounded results in subsequent operations, but the first entry stored raw precision, creating a lasting inconsistency in the trie.

**Changes made:**
`pallets/ocex/src/settlement.rs`:
- Replaced `.or_insert(balance)` with `.or_insert_with(|| Order::rounding_off(balance))`

---

### M13 — Unlock gated on Operational flag — bridge pause freezes all completed migrations
**Severity:** Medium  
**Location:** `pallets/pdex-migration/src/lib.rs` — `unlock`  
**Date:** 2026-09-10

**Vulnerability:** `unlock` was gated on `Self::operational()`. Governance pausing the bridge would prevent users who had already completed migration from unlocking their tokens after the 28-day lock period expired, even though their migration was done and the tokens already belonged to them.

**Changes made:**
`pallets/pdex-migration/src/lib.rs`:
- Removed the `if Self::operational() { ... } else { NotOperational }` check from `unlock`
- `process_unlock` still enforces the 28-day lock period via `LockedTokenHolders`

---

### M16 — Fee fractions unbounded — >1 makes fee exceed trade value; <0 drains the pot
**Severity:** Medium  
**Location:** `primitives/polkadex/src/fees.rs` — `FeeConfig`, `pallets/ocex/src/settlement.rs` — `process_trade`  
**Date:** 2026-09-10

**Vulnerability:** `FeeConfig.maker_fraction` and `FeeConfig.taker_fraction` are `Decimal` values with no validation. A fraction > 1 causes `taker_credit.saturating_mul(fraction)` to exceed the trade value — the surplus is silently lost. A negative fraction would credit the user extra, draining the fee pot.

**Changes made:**
`primitives/polkadex/src/fees.rs`:
- Added `FeeConfig::validate() -> Result<(), &'static str>` checking both fractions are in `[0, 1]`

`pallets/ocex/src/settlement.rs`:
- Added `maker_fees.validate()?` and `taker_fees.validate()?` at the start of `process_trade` before any balance operations

---

### M15 — All calls use weight zero — WeightInfo trait never declared in lib.rs
**Severity:** Medium  
**Location:** `pallets/pdex-migration/src/lib.rs`, `pallets/pdex-migration/src/weights.rs`  
**Date:** 2026-09-11

**Vulnerability:** `weights.rs` contains full benchmarked weight data (benchmarked 2024-03-05), but the `WeightInfo` trait was never declared in `lib.rs`. The five dispatchable calls (`set_migration_operational_status`, `set_relayer_status`, `mint`, `unlock`, `remove_minted_tokens`) all used `#[pallet::weight(Weight::default())]` — zero weight. This meant all migration transactions were free and exempt from block-weight limits, making them usable for block-stuffing attacks during active migration.

**Impact:** During an active ERC-20 → native PDEX migration window, an attacker could spam `mint` or `unlock` calls with zero cost, consuming block space without paying fees or contributing to block weight limits.

**Changes made:**  
`pallets/pdex-migration/src/lib.rs`:
- Declared `pub trait WeightInfo` at crate root with 5 method signatures using `frame_support::weights::Weight`
- Implemented `WeightInfo for ()` (zero-weight fallback used in tests)
- Added `pub mod weights;` to expose the benchmarked `SubstrateWeight<T>` implementation
- Added `type WeightInfo: crate::WeightInfo` to `Config` trait
- Wired all 5 calls: `#[pallet::weight(<T as Config>::WeightInfo::...)]`
- Added `use crate::WeightInfo as _;` inside `pub mod pallet` so trait methods are in scope

`pallets/pdex-migration/src/mock.rs`:
- Added `type WeightInfo = ();` to `impl pdex_migration::Config for Test`

---

### M1 — Threshold truncation: Percent::mul_floor makes 51% * 3 = 1 (33%)
**Severity:** Medium  
**Location:** `pallets/ocex/src/lib.rs` — `validate_snapshot`  
**Date:** 2026-09-11  
**Thea:** covered by C8 fix (already uses `(2*n)/3 + 1` ceiling)

**Vulnerability:** `validate_snapshot` computed the signature threshold as `Percent::from_percent(51) * authorities.len()`. `Percent` uses floor multiplication (`mul_floor`): for a 3-validator set, `51% * 3 = floor(1.53) = 1`, so only one signature out of three was required — a 33% threshold masquerading as 51%. For a 5-validator set, two signatures sufficed (40%).

**Impact:** A single compromised validator key could forge a settlement approval in any set of 3 or fewer authorities. With the `max(threshold, 1)` guard from C3, the issue was visible only for non-zero sets — but small validator sets (3–6 members) are common in early mainnet phases.

**Changes made:**  
`pallets/ocex/src/lib.rs`:
- Replaced `Percent::from_percent(51) * authorities.len()` with integer ceiling arithmetic: `(51 * authorities.len() + 99) / 100`
- This gives `ceil(51n/100)`: n=3 → 2 (67%), n=5 → 3 (60%), n=100 → 51 (51%)
- Retained `core::cmp::max(required, 1)` as defence-in-depth guard

---

### M14 — Two construct_runtime! blocks — verified stale, no code change needed
**Severity:** Medium  
**Location:** `runtimes/mainnet/src/lib.rs`  
**Date:** 2026-09-11  
**Resolution:** Verified stale — no fix required

**Finding:** The audit noted two `construct_runtime!` macro invocations in the mainnet runtime.

**Verification:** All `construct_runtime!` blocks in `runtimes/mainnet/src/lib.rs` are commented out (lines 2681, 2749, 2805). The active runtime is defined with `#[frame_support::runtime]` at line 2461. No duplicate runtime definitions exist in the active code — this finding no longer applies.

---

### M11 — Council votes survive membership changes; thresholds recalculated from new size
**Severity:** Medium  
**Location:** `runtimes/mainnet/src/lib.rs` — `pallet_collective::Config<CouncilCollective>`  
**Date:** 2026-09-11  
**Resolution:** Mitigated by existing runtime config — no new code change required

**Finding:** In FRAME's `pallet_collective`, when elections produce a new council via `ChangeMembers`, existing open proposals retain votes from removed members. The quorum threshold is recalculated from the new (potentially smaller) membership, but stale "aye" votes from ex-members still count — a proposal voted through by ex-members before their removal can still execute.

**Mitigations already in place:**
- `DisapproveOrigin = EnsureRoot<Self::AccountId>` — root/sudo can disapprove any open proposal at any time
- `KillOrigin = EnsureRoot<Self::AccountId>` — root/sudo can kill any open proposal
- `MotionDuration = 7 * DAYS` — all proposals expire within 7 days, bounding the exposure window

**Operational procedure:** After any election that changes council membership, the operations team should review open proposals via `pallet_collective::Proposals`. Any proposal with votes from removed members that would no longer meet threshold should be `disapprove_proposal`'d via sudo before executing. The 7-day window provides sufficient time for this review.

---

### M12 — Council cannot be bootstrapped — delete_transaction permanently unreachable
**Severity:** Medium  
**Location:** `runtimes/mainnet/src/lib.rs` — `pallet_collective::Config<CouncilCollective>` / `pallet_elections_phragmen::Config`  
**Date:** 2026-09-11  
**Resolution:** Verified already handled — no new code change required

**Finding:** The audit noted that "delete_transaction" was permanently unreachable and the council could not be bootstrapped from an empty state.

**Verification:**
1. `"delete_transaction"` does not exist in the codebase — the auditor's label most likely refers to `pallet_collective::disapprove_proposal`, which IS reachable via `DisapproveOrigin = EnsureRoot`.
2. Bootstrap is already handled: `pallet_collective::Config<CouncilCollective>` sets `SetMembersOrigin = EnsureRoot<Self::AccountId>` — root/sudo can call `Council::set_members` directly to initialize any member set without going through elections.
3. The genesis config preset seeds initial council members via `ElectionsConfig { members: ... }`, providing a valid starting state from day one.

No code change is needed. Both the bootstrap path (root sets members) and proposal recovery path (root disapproves) are reachable.

---

### M2 — PriceOracle: no outlier rejection, one tick moves average ≈50%
**Severity:** Medium  
**Location:** `pallets/ocex/src/lib.rs` — `EgressMessages::PriceOracle` handler  
**Date:** 2026-09-11

**Vulnerability:** The on-chain price oracle used by the LMP (Liquidity Mining Program) stored cumulative average prices computed from each operator snapshot. No bounds check was applied to incoming prices: a single malicious or erroneous snapshot could report an arbitrary price, which would dominate the average when the tick count was low (e.g., for a new market with only 1 prior tick, the new price carries 50% weight).

**Impact:** A validator submitting a manipulated price during a low-tick-count period for an active LMP market could skew average prices used for reward calculations, potentially over-rewarding or under-rewarding specific trading pairs/pools.

**Changes made:**  
`pallets/ocex/src/lib.rs`:
- Added `PRICE_ORACLE_MAX_DEVIATION_PCT = 50` constant
- Added outlier rejection in the PriceOracle update loop: if the incoming price deviates more than ±50% from the current cumulative TWAP, the update is skipped with a `warn!` log. First-tick entries (no prior history) are accepted unconditionally so new markets can establish a baseline.

---

### M3 — No signature domain separation in SnapshotSummary — replay on forks
**Severity:** Medium  
**Location:** `pallets/ocex/src/lib.rs` — `validate_snapshot`, `pallets/ocex/src/validator.rs` — OCW signing  
**Date:** 2026-09-11  
**Scope:** Snapshot path fixed; `ExchangePayload` path requires exchange backend coordination

**Vulnerability:** `SnapshotSummary` was signed as `key.sign(&summary.encode())` and verified as `auth.verify(&summary.encode(), sig)` — the raw SCALE bytes of the struct with no protocol prefix, chain ID, or type tag. A valid snapshot signature could be replayed on a chain fork, a testnet, or any other Polkadex deployment that uses the same validator key set.

**Impact:** An attacker with access to a mainnet snapshot and signatures (e.g., obtained from an aggregator) could replay them on a testnet or fork with identical validator keys to settle withdrawals or advance state. With the custody pool drained in the attack environment, the signatures could later be presented in governance claims.

**Changes made:**  
`pallets/ocex/src/lib.rs`:
- Added `SNAPSHOT_SIGNING_PREFIX: &[u8] = b"polkadex::ocex::snapshot::v1:"` constant
- Added `Pallet::<T>::snapshot_signing_payload(summary)` helper that prepends the domain prefix to `summary.encode()`
- Updated `validate_snapshot`: signature is now verified against `snapshot_signing_payload(summary)` instead of bare `summary.encode()`

`pallets/ocex/src/validator.rs`:
- Updated OCW signing: `key.sign(&signing_payload)` where `signing_payload = snapshot_signing_payload(&summary)` — both sides now use the same domain-prefixed payload

**Remaining (requires exchange backend change):** `ExchangePayload` in `pallets/rewards` is signed by the off-chain exchange backend using `serde_json::to_vec(payload)` with no chain ID or type tag. Adding domain separation there requires coordinating a breaking change with the matching engine.

---

### L1 — unwrap()/expect() in off-chain and RPC paths
**Severity:** Low
**Location:** `pallets/ocex/src/validator.rs`, `pallets/ocex/src/rpc.rs`
**Date:** 2026-09-16

**Finding:** Three `unwrap()` calls in non-test production code paths that would crash the offchain worker or RPC handler:
- `validator.rs` `store_q_scores()` and `compute_trader_metrics()`: `Decode::decode(&mut &main.encode()[..]).unwrap()`
- `validator.rs` `compute_score()`: `Decimal::from_f64(0.0025).unwrap()`
- `rpc.rs` `calculate_inventory_deviation()`: `Decode::decode(...).unwrap()`

**Changes made:**
- Replaced `Decode::decode(...).unwrap()` with `.map_err(|_| "Failed to decode main AccountId")?` in both OCW paths
- Replaced `Decimal::from_f64(0.0025).unwrap()` with infallible `Decimal::new(25, 4)`
- Replaced `Decode::decode(...).unwrap()` in RPC path with `.map_err(|_| Error::<T>::FailedToDecodeAccount)?`
- Added `FailedToDecodeAccount` error variant to `Error<T>` enum

---

### L2 — Decimal::pow on attacker-influenced volumes can panic the offchain worker
**Severity:** Low
**Location:** `pallets/ocex/src/validator.rs` — `compute_score()`
**Date:** 2026-09-16

**Finding:** `Decimal::pow(f64)` converts internally to `f64`. A negative base with a fractional exponent (e.g. `(-1.0_f64).powf(0.15)`) produces `NaN`, which cannot convert back to `Decimal` and panics the OCW. `maker_volume` comes from attacker-influenced offchain state; `q_score` from computed metrics — either could theoretically be negative.

**Changes made:**
`pallets/ocex/src/validator.rs`:
- Added `let q_score = q_score.max(Decimal::zero())` and `let maker_volume = maker_volume.max(Decimal::zero())` before the power formula
- Negative volume/score semantically contributes nothing to the reward score, so clamping to zero is correct

---

### L3 — TotalAssets never decremented and never read by any guard
**Severity:** Low
**Location:** `pallets/ocex/src/lib.rs` — `on_idle_withdrawal_processor()`
**Date:** 2026-09-16

**Finding:** `TotalAssets` was incremented on every deposit in `do_deposit()` but never decremented on withdrawal, causing the stored value to drift upward indefinitely. The storage item was also never read by any guard or invariant check, making it misleading.

**Changes made:**
`pallets/ocex/src/lib.rs`:
- Added `<TotalAssets<T>>::mutate(withdrawal.asset, |total| { *total = total.saturating_sub(withdrawal.amount); })` after a successful transfer in `on_idle_withdrawal_processor()`

---

### L4 — Wrong length constants and empty key set accepted in bls-primitives
**Severity:** Low
**Location:** `primitives/bls/src/lib.rs`
**Date:** 2026-09-16

**Finding:** Three bugs in the deprecated `bls-primitives` module:
1. `TryFrom<&[u8]> for Signature` checked `len != 196` but `Signature` is `[u8; 48]`. Any 196-byte input passed the guard then panicked on `try_into::<[u8; 48]>()`
2. `ByteArray::LEN` was `96` instead of `48` (G1 compressed, min_sig scheme)
3. `verify()` accepted an empty `public_keys` slice — aggregated key collapses to `G2::identity()`, allowing an all-zeros signature to verify

**Changes made:**
`primitives/bls/src/lib.rs`:
- Fixed `TryFrom` length check from `196` to `48`
- Fixed `ByteArray::LEN` from `96` to `48`
- Added early `return false` for empty `public_keys` in `verify()`

**Not fixed:** Rogue-key / proof-of-possession redesign — module is explicitly marked deprecated (host function only, not for production use). Redesign is out of scope for a deprecated module.

---

### L5 — new_random_id is block-number derived — same-block deposits produce colliding IDs
**Severity:** Low
**Location:** `pallets/ocex/src/lib.rs` — `new_random_id()`
**Date:** 2026-09-16

**Finding:** `new_random_id` generated an H160 ID by hashing only the block number via `blake2_128(block_number.encode())`. Every deposit extrinsic in the same block produced the same ID, causing collisions in the ingress message queue.

**Changes made:**
`pallets/ocex/src/lib.rs`:
- Added `extrinsic_index()` to the hash input: `blake2_128(&(current_blk, extrinsic_index).encode())`
- `extrinsic_index()` is unique per extrinsic within a block — no new storage required

---

### L6 — set_fee_distribution / validate_snapshot accept zero-fee withdrawals
**Severity:** Low
**Location:** `primitives/polkadex/src/auction.rs`, `pallets/ocex/src/lib.rs`
**Date:** 2026-09-16

**Finding:** Two gaps:
1. `set_fee_distribution` (call_index 21) stored `FeeDistribution` with no bounds validation — `burn_ration` could exceed 100 and `auction_duration` could be 0, permanently stalling the auction timing.
2. `validate_snapshot` accepted snapshots where withdrawal `fees <= 0`, letting the off-chain engine submit zero-fee withdrawals. With no floor enforced on-chain, any withdrawal queue entry could have `fees = 0`, enabling free spam withdrawals.

The deposit side of L6 was already fixed by C9: `ensure!(amount >= T::MinimumDeposit::get(), DepositAmountTooLow)` in `do_deposit`.

**Changes made:**
`primitives/polkadex/src/auction.rs`:
- Added `FeeDistribution::validate()`: rejects `burn_ration > 100` and `auction_duration <= 0`

`pallets/ocex/src/lib.rs`:
- Called `fee_distribution.validate().map_err(DispatchError::Other)?` in `set_fee_distribution` before storing
- Added loop in `validate_snapshot` (before auth checks) that returns `InvalidTransaction::Custom(17)` for any withdrawal with `fees <= Decimal::ZERO`

---

### L7 — AssetMetadata decimal no upper bound — decimal ≥ 51 overflows pow
**Severity:** Low
**Location:** `primitives/thea/src/types.rs` — `AssetMetadata::new()`
**Date:** 2026-09-17

**Finding:** `AssetMetadata::new()` rejected `decimal < 1` but allowed any value up to 255. `convert_to_native_decimals` calls `10u128.pow(decimal - 12)` for foreign assets with more decimals than native. `10u128.pow(39)` (decimal = 51) exceeds `u128::MAX ≈ 3.4×10^38`, causing overflow — panic in debug builds, silent wrap in release.

**Changes made:**
`primitives/thea/src/types.rs`:
- Added `|| decimal > 50` to the `new()` guard — returns `None` for out-of-range decimals

---

### L8 — ProxyType::Any wraps OCEX and THEA calls
**Severity:** Low
**Location:** `runtimes/mainnet/src/lib.rs` — `InstanceFilter<RuntimeCall> for ProxyType`
**Date:** 2026-09-17

**Finding:** `ProxyType::Any => true` and `ProxyType::NonTransfer` both allow OCEX and THEA calls through. A proxy key with `Any` permission can sign `submit_snapshot` and `submit_signed_outgoing_messages` on behalf of a validator, turning a compromised proxy into a full custody/bridge signing capability.

**Changes made:**
`runtimes/mainnet/src/lib.rs`:
- Added `SECURITY (L8)` comment on `ProxyType::Any` documenting that `OcexPallet` and `Thea` calls must be explicitly excluded when those pallets are re-enabled. Runtime call variants do not exist while the pallets are dormant, so the active filter change is deferred to the re-enable PR.

**Deferred:** Full filter fix (`| RuntimeCall::OcexPallet(..) | RuntimeCall::Thea(..)`) must be added to `NonTransfer` and excluded from `Any` in the same PR that re-enables OCEX/THEA.

---

### L9 — Market orders have no slippage bound
**Severity:** Low
**Location:** `primitives/orderbook/src/types.rs` — `Order::verify_config`
**Date:** 2026-09-17

**Finding:** The market order ASK branch in `verify_config` only checked `qty_step_size`, skipping the `min_volume`/`max_volume` bounds that the BID branch enforces. No `worst_price` field exists on `Order`, so a market order can sweep the book to any fill price with no user-set bound.

**Changes made:**
`primitives/orderbook/src/types.rs`:
- Added `qty > Decimal::ZERO`, `qty >= config.min_volume`, and `qty <= config.max_volume` to the ASK market order branch — now symmetric with the BID branch
- Added `SECURITY (L9)` comment noting that a `worst_price` slippage field should be added to `Order` when OCEX is re-enabled

**Deferred:** Adding a `worst_price` field to `Order` and enforcing it in the off-chain engine requires coordination with the exchange backend. Deferred to OCEX re-enable.

---

## Open — Pending

| ID | Severity | Location | Finding |
|---|---|---|---|
| C7 | 🔴 Critical | nodes/, session-keys/ | Master BIP39 seed committed in repo — rotate all session keys |
| H4 | 🟠 High | pallets/ocex | UserActionBatch.signature never verified |
| R2-H1 | 🟠 High | pallets/ocex | process_egress_msg routes funds to caller-chosen account |
| R4-A | 🟠 High | pallets/ocex | claim_withdraw benchmarked wrong; empty key re-inserted |
| R3-H4 | 🟠 High | CI config | Fork PRs run as root on IAM-bearing runner |
| R3-H5 | 🟠 High | Cargo.toml | WASM builder on mutable fork branch; no rev pin |
| M3 (partial) | 🟡 Medium | pallets/rewards | ExchangePayload domain sep — requires exchange backend coordination |
| L10–L14 | ⚪ Low | various | See full findings table |
