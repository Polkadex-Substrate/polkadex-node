# pallet-forced-exit

An escape hatch for orderbook custody: users can always recover their funds without the
settlement engine's cooperation.

## Why

Today a user's withdrawal can only be claimed once the engine embeds it in a snapshot. If the
engine stops publishing snapshots, funds stay in custody with no path out — custody liveness is
bound to engine liveness. This pallet breaks that dependency.

## How it works

| Stage | Mechanism |
|---|---|
| Ask | `request_withdrawal` records the request in runtime storage with a deadline, so being ignored becomes an on-chain fact rather than a support ticket. |
| Escalate | `trigger_settlement_freeze` — **permissionless**. Anyone may call it, presenting evidence that is objectively true on-chain: no finalized snapshot within `SnapshotLivenessTimeout`, or a request unserviced past `RequestServiceTimeout`. |
| Exit | `force_withdraw` — while frozen, pays out the caller's balance as committed in the last finalized snapshot (merkle inclusion proof) plus any deposits the chain witnessed after it. |
| Recover | `resume_settlement` — governance restarts the venue under a fresh snapshot. Bumping the exit epoch lapses prior claims. |

Freezing is permissionless; resuming is governed. Evacuation must never need permission;
recovery is a privileged, visible act.

## What stops the hatch being used to steal

The hatch never trusts the claimant's assertion of their balance, and never trusts the
operator's cooperation. It pays only what the validators last notarised.

- **Traded-away funds** — proofs are accepted only against the *current* finalized root, which
  already reflects the trade. Covered by `traded_away_balance_is_not_claimable`.
- **Funds locked in open orders** — not claimable while the venue is live; claimable after a
  freeze, when open orders are void because no engine exists to fill them.
- **Already-approved withdrawals** — a snapshot that approves a withdrawal has already reduced
  the user's balance in that same snapshot, so the normal claim path and the hatch draw on
  disjoint amounts.
- **Replay and inflation** — the leaf binds `(account, asset, free, in_orders)`, so another
  user's path does not verify and an inflated amount changes the leaf hash.
- **Double exit** — one claim per `(epoch, account, asset)`.

## Merkle format

Consensus-critical and specified in `merkle.rs` so third parties can rebuild proofs
independently:

- leaves sorted ascending by `scale(account, asset)`
- leaf hash `blake2_256(0x00 ++ scale(BalanceLeaf))`, node hash `blake2_256(0x01 ++ l ++ r)`
- a lone trailing node is **promoted**, never duplicated (duplication lets two distinct leaf
  sets share a root)
- domain-separation prefixes prevent an internal node being presented as a leaf

## Integration

The pallet is deliberately decoupled from the settlement pallet: neither has a Cargo dependency
on the other.

- The settlement pallet implements `traits::Custody` (release funds, report custody holdings).
- This pallet implements `traits::SettlementNotifier`; the settlement pallet calls
  `on_snapshot_finalized`, `on_requests_serviced`, `on_deposit`, `on_deposits_settled`, and
  must consult `is_frozen()` before accepting snapshots or deposits.

### Required of the settlement layer

1. **`balances_root` in every snapshot** — a merkle commitment over
   `(account, asset) → (free, in_orders)`. The current `state_hash` is unstructured and cannot
   serve: it verifies a whole book but proves nothing about an individual balance.
2. **Snapshots reported only after their dispute window** — `on_snapshot_finalized` must be
   called on finality, not acceptance.
3. **Public data availability** — proofs must be buildable without the operator. See the
   companion spec for the recommended approach (per-snapshot balance diffs in call data, with
   periodic full checkpoints), which keeps chain *state* at 32 bytes per snapshot while making
   the book reconstructible from chain history alone.

## Status

**Draft for review. Not production-ready.**

- Weights are hand-estimated placeholders; benchmarks must be generated before any runtime
  enables this pallet.
- Not yet wired into the mainnet runtime, and the settlement-side `Custody` implementation and
  notifier calls are not yet written.
- **Hard dependency:** forced exit is only as sound as the snapshot it trusts. It must not be
  enabled in a runtime whose settlement pallet still accepts unauthenticated snapshots, permits
  signer duplication below threshold, executes withdrawals without a dispute window, or allows
  the snapshot nonce to rewind. Those fixes land first.
