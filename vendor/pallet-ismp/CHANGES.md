# Changes to the published crate

This directory holds `pallet-ismp` 2512.2.0 from crates.io, by Polytope Labs under the Apache
License 2.0 (see `LICENSE`), built from `modules/pallets/ismp` of polytope-labs/hyperbridge at
3979482. The crate's own `Cargo.lock` is left out; the workspace lock applies. The Polkadex Authors
changed it as follows; the changed files say so in their headers, and the new code is marked
"Polkadex".

- Bounds on `handle_unsigned`: `Config::MaxMessages`, `MaxRequests` (requests, responses and
  timeouts over all messages) and `MaxEncodedSize`. `check_bounds` refuses a call over any of them
  with `ExhaustsResources`, first in `validate_unsigned`, before the messages execute, and in
  `pre_dispatch`, which runs at block import. File: `src/lib.rs`.
- Weights of `handle_unsigned` from its messages, replacing the flat 300M with no proof size:
  `Config::MessageWeightInfo` prices a consensus or fraud proof at one signature per 136 bytes
  (`PRECOMMIT_BYTES`) and a request, response or timeout batch per request and proof byte, and
  `Config::WeightProvider` (upstream's unused trait) adds each module callback's declared weight.
  Files: `src/lib.rs`, `src/weights.rs`.
- Sizing of `MaxEncodedSize` (a runtime value, 256 KiB): it fits a Polkadot GRANDPA finality proof
  (up to ~629 headers of 327 B plus the justification), so the relayer's grandpa prover must stay
  under it, `max_block_range` 560 for Polkadot (Nexus) and 700 for Paseo (Gargantua). The per-call
  GRANDPA vote-ancestry cap the DoS bound needs is enforced in the runtime consensus-client wrapper
  (`runtimes/mainnet/src/ismp_bounds.rs`), not here, since this crate does not decode the GRANDPA
  message format.
