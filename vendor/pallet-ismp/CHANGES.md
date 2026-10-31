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
