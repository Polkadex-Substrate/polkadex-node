# Changes to the published crate

This directory holds `pallet-hyper-fungible-token` 2512.0.0 from crates.io, by Polytope Labs under
the Apache License 2.0 (see `LICENSE`), built from `modules/pallets/hyper-fungible-token` of
polytope-labs/hyperbridge at 3979482. The Polkadex Authors changed it as follows; the changed files
say so in their headers, and the new code is marked "Polkadex".

- Decimal conversion both ways (a60c3c3): `convert_to_balance` and `convert_to_erc20` scale up as
  well as down, so a token may have fewer ERC-20 decimals than local ones. `register_token` and
  `update_token` no longer refuse that case; `ErcDecimalsBelowLocal` stays for the encoding only.
  Files: `src/impls.rs`, `src/lib.rs`.
- Callback weight: `runtime_call_weight` returns the declared weight of the runtime call an
  arriving transfer carries, which `on_accept` dispatches unmetered, so the runtime can charge it
  to the unsigned ISMP call. File: `src/module.rs`.
- Refund routes: `RefundRoutes` keeps the local asset and ERC-20 decimals of every (chain,
  contract) that `register_token` or `update_token` adds, and nothing removes it. `on_timeout`
  reads it instead of `ContractToAsset` and `Precisions`, so a transfer in flight refunds in full
  after its contract changes, while `on_accept` still trusts only the live maps. Files:
  `src/lib.rs`, `src/module.rs`.
- Exact amounts: `send` refuses, with `AmountNotRepresentable` and before anything moves, an
  amount that is not a whole multiple of 10^(local - remote) decimals, since `convert_to_erc20`
  floored it and the timeout refunded the rounded amount. `local_decimals` replaces the three
  copies of the decimals lookup. Files: `src/impls.rs`, `src/lib.rs`, `src/module.rs`.
- Depth-limited call decoding: the untrusted `runtime_call` an arriving transfer carries is decoded
  with `decode_with_depth_limit(MAX_EXTRINSIC_DEPTH)` in both `runtime_call_weight` and `on_accept`,
  so a nested call cannot recurse without bound. File: `src/module.rs`.
- Refund terms: `send` records the asset and local amount it debited in `RefundTerms`, keyed by the
  request commitment, and `on_timeout` takes them and refunds exactly that, so a precision update,
  a contract reused for another asset or a local metadata change cannot alter a pending refund.
  Transfers sent before the upgrade have no terms and refund by `RefundRoutes` as before. Files:
  `src/lib.rs`, `src/module.rs`.
- Bridge fee: a new `Config::BridgeFee` (trait `types::BridgeFee`). Before the lock or burn, `send`
  moves the fee for its asset and destination from the sender to the fee collector, in the asset
  sent (`NativeCurrency` for the native asset, `Assets` otherwise), and emits `BridgeFeePaid`; a
  zero fee or the collector's own send moves nothing and emits nothing. With no fee or no collector
  set, `send` fails with `BridgeFeeNotSet` and nothing moves. Timeout refunds return the amount
  only. Files: `src/lib.rs`, `src/types.rs`.
- `call_data` cap: `send` refuses `call_data` longer than `BridgeFee::max_call_data()` with
  `CallDataTooLong`. File: `src/lib.rs`.

The new event and errors are appended, so existing indices and `TokenSent` do not change.
