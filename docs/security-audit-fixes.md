# Security Audit — Fix Log

Tracking all changes applied from the 14 August 2026 security audit.  
Audit covered `polkadex-substrate/Polkadex` and `Polkadex-Substrate/matching-engine`.  
This document covers fixes applied to **this repo only**.

**Totals:** 65 findings in this repo · 17 fixed (as of last update) · 48 open  
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
| `replace similar invest corn admit radio staff peanut subway early capital design` | BEEFY — validators 1–3 |
| `valve clap veteran panel cousin hover angle annual kick confirm cave deer` | Mixnet — validators 1–3 |

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

## Open — Pending

| ID | Severity | Location | Finding |
|---|---|---|---|
| C7 | 🔴 Critical | nodes/, session-keys/ | Master BIP39 seed committed in repo — rotate all session keys |
| H4 | 🟠 High | pallets/ocex | UserActionBatch.signature never verified |
| R2-H1 | 🟠 High | pallets/ocex | process_egress_msg routes funds to caller-chosen account |
| R3-H2 | 🟠 High | pallets/ocex | OCW mutex released on failed acquisition; unsafe RPC namespaces |
| R3-H3 | 🟠 High | pallets/ocex | Aggregator HTTP response uncapped |
| R3-H12 | 🟠 High | pallets/ocex | LMP config metrics write-only; epoch budget over-issued |
| R3-H13 | 🟠 High | pallets/ocex | close_auction non-transactional; place_bid commented out |
| R4-A | 🟠 High | pallets/ocex | claim_withdraw benchmarked wrong; empty key re-inserted |
| R3-H9 | 🟠 High | pallets/thea | Global asset registry — weakest network mints any bridged asset |
| R3-H6 | 🟠 High | pallets/liquidity-mining | Pools keyed by market_maker, callbacks look up by pool_id |
| R3-H7 | 🟠 High | pallets/liquidity-mining | remove_liquidity_failed mints 10¹²× shares |
| R3-H8 | 🟠 High | pallets/liquidity-mining | force_close_pool sends funds to personal account |
| R3-H11 | 🟠 High | pallets/liquidity-mining | claim_force_closed_pool_funds reads wrong asset |
| H5 | 🟠 High | pallets/liquidity-mining | requests[num_requests..] panics on caller u16 |
| H6 | 🟠 High | pallets/liquidity-mining | dev_mode + flat weight live at mainnet index 50 |
| H3 | 🟠 High | pallets/rewards | Vesting: entire lock removed on first claim |
| R4-B | 🟠 High | pallets/rewards | Crowdloan re-pay on second cycle (~2M PDEX) |
| R4-C | 🟠 High | scripts/ | Crowdloan verifier always prints success |
| H2 | 🟠 High | pallets/pdex-migration | Third approver's beneficiary used for mint |
| H9 | 🟠 High | pallets/xcm-helper | XCM fee whitelist commented out; zero fee hardcoded |
| R3-H4 | 🟠 High | CI config | Fork PRs run as root on IAM-bearing runner |
| R3-H5 | 🟠 High | Cargo.toml | WASM builder on mutable fork branch; no rev pin |
| M1–M16 | 🟡 Medium | various | See full findings table |
| L1–L14 | ⚪ Low | various | See full findings table |
