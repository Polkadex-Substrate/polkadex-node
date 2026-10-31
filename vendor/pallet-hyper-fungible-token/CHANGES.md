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
