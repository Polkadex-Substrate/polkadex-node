# Local patches to ismp-grandpa 2512.1.0

Unpacked from the crates.io tarball `ismp-grandpa-2512.1.0.crate` (sha256
`ba73dd0e6e958a88ee0142514bff3985431560636d13db00a9357ae5dcea59d1`, the checksum in the root
Cargo.lock; published from polytope-labs/hyperbridge `3979482228`, `modules/ismp/clients/grandpa`).
The tarball's own `Cargo.lock` is left out: cargo ignores the lockfile of a dependency.

## 1. Bind the proof envelope to the tracked state machine

Backport of polytope-labs/hyperbridge #1069, merge commit `2bb34ae8ac` (28 Jul 2026), first
released in ismp-grandpa 2606.0.0, which needs polkadot-sdk 2606.

Without it a relay-chain finality proof wrapped as `StandaloneChain` passes the signature checks
(a parachain tracker's authority set is the relay's GRANDPA set) and records the relay's global
state root under the tracked parachain (our coprocessor), which enables a false timeout and a
double refund.

`src/consensus.rs`: `verify_consensus` rejects a message whose envelope does not match the class
of the tracked state machine before any arm runs; `envelope_matches_state_machine` and its tests
are upstream's, verbatim. Upstream returns the new grandpa-verifier variant
`ConsensusMessageStateMachineMismatch`; here `envelope_mismatch` builds the same
`ismp::Error::Custom` text inside the client, so grandpa-verifier stays the crates.io 2512.1.0.
Three tests for the coprocessor case (`Kusama(4009)`) follow upstream's.

Drop this copy when the runtime moves to ismp-grandpa 2606.0.0 or later.
