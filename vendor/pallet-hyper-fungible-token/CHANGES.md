# Changes to the published crate

This directory holds `pallet-hyper-fungible-token` 2512.0.0 from crates.io, by Polytope Labs under
the Apache License 2.0 (see `LICENSE`), built from `modules/pallets/hyper-fungible-token` of
polytope-labs/hyperbridge at 3979482. The Polkadex Authors changed it as follows; the changed files
say so in their headers, and the new code is marked "Polkadex".

- Decimal conversion both ways (a60c3c3): `convert_to_balance` and `convert_to_erc20` scale up as
  well as down, so a token may have fewer ERC-20 decimals than local ones. `register_token` and
  `update_token` no longer refuse that case; `ErcDecimalsBelowLocal` stays for the encoding only.
  Files: `src/impls.rs`, `src/lib.rs`.
- Bridge fee: a new `Config::BridgeFee` (trait `types::BridgeFee`). Before the lock or burn, `send`
  moves the fee for its asset and destination from the sender to the fee collector, in the asset
  sent (`NativeCurrency` for the native asset, `Assets` otherwise), and emits `BridgeFeePaid`. With
  no fee or no collector set, `send` fails with `BridgeFeeNotSet` and nothing moves. Timeout
  refunds return the amount only. Files: `src/lib.rs`, `src/types.rs`.
- `call_data` cap: `send` refuses `call_data` longer than `BridgeFee::max_call_data()` with
  `CallDataTooLong`. File: `src/lib.rs`.

The new event and errors are appended, so existing indices and `TokenSent` do not change.
