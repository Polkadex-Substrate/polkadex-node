# Runtime 394: notes for reviewers

The plain-words version for holders is [runtime-394.md](runtime-394.md). This page is for code review, as of 9 Nov 2026.

## What changes

| Where | Change |
|---|---|
| `pallet-ismp` 2512.2.0, vendored (`vendor/pallet-ismp`) | The crates.io release byte for byte (`CHANGES.md` lists our edits). Added: `Config::MaxMessages` 8, `MaxRequests` 64, `MaxEncodedSize` 256 KiB, checked by `check_bounds` in `validate_unsigned` and `pre_dispatch`, so the bounds hold in the pool and at block import; `handle_unsigned` is weighed from its arguments through `MessageWeightInfo` (a consensus proof at one signature per 136 bytes, a batch per request plus per proof byte) and `WeightProvider` (the module callback's declared weight, including the runtime call a transfer carries). The upstream flat 300M weight with zero proof size is gone. |
| `ismp-grandpa` 2512.2.0 | Has the fix from hyperbridge #1069: a relay chain proof wrapped as StandaloneChain is rejected. Wrapped by the runtime's `ismp_bounds::BoundedGrandpa`, which refuses a justification with more than 16 vote ancestries before verification, for consensus updates and both halves of a fraud proof. |
| `pallet-hyper-fungible-token` 2512.0.0, vendored | Decimal conversion both ways; `convert_to_erc20` refuses amounts the destination cannot represent (`AmountNotRepresentable`); `RefundRoutes` keeps every (chain, contract) ever registered and `RefundTerms` records what each send debited, so a timeout refunds exactly that after a contract change; the runtime call an incoming transfer carries is decoded with `MAX_EXTRINSIC_DEPTH`; `send` pays the bridge fee from `Config::BridgeFee` before the lock or burn and fails closed; `call_data` is capped by `MaxCallData`; a recipient the destination cannot credit is refused (20 bytes for EVM, 32 otherwise); refund terms are stored only when the timeout is non-zero. Weights by path and `call_data` length from the runtime's benchmarks. |
| `pallets/ismp-params` (new) | Root sets the coprocessor (Hyperbridge's state machine), so one wasm serves testnet and mainnet. Root or more than half of the council sets the bridge fee per (asset, destination) and `MaxCallData` (default 0); Root sets the fee collector, which must exist. |
| Runtime | Ismp at 68, IsmpGrandpa at 69, HyperFungibleToken at 72, IsmpParams at 73; 70 and 71 stay unused. Every trust decision is Root: consensus clients (`AdminOrigin`), the state machine whitelist (`RootOrigin`), the coprocessor, token registration (`CreateOrigin`). The NonTransfer proxy filter excludes `HyperFungibleToken::send` and `Ismp::fund_message`. Weights: `weights/ismp_messages.rs`, `weights/pallet_hyper_fungible_token.rs`, `weights/pallet_ismp.rs`, `weights/ismp_grandpa.rs`, all benchmarked on this runtime. `spec_version` 394, `impl_version` 0, `transaction_version` 3 (no existing call or extension changes its encoding; new errors and events are appended). |
| Node | `nodes/mainnet/src/node_rpc.rs` serves the ISMP RPC (`ismp_*`). Released as v7.1.0 (#39, which also merges mainnet's v7.0.2 into testnet), tagged on the merge; relayer RPC nodes need it, with offchain indexing on; validators do not. |
| `contracts/evm` | Our copies of Polytope's upgradeable token and wrapper contracts: a fee per destination in the token sent, an amount unit so sends arrive exactly, size caps on `to` and `data`, an owner-set host, the recipient length check, no WETH path. New state sits after the upstream storage, so existing proxies upgrade in place. Forge tests: 28 of 28. |

## Why

- Unsigned ISMP delivery is free by design, so a relayer must not be able to make the chain do more work than it charges. Bounds plus argument-based weights make the cost of any accepted call known before it executes. The remaining pre-signature work in GRANDPA verification is the per-precommit ancestry walk, which the 16-ancestry cap bounds; live Paseo and Polkadot justifications carry zero.
- Refunds must not depend on the live trust maps: a contract rotation by governance deleted the route a pending transfer needed. Per-send terms fix it exactly.
- Our apps send with timeout 0 because an expiry adds nothing for a plain token transfer: nothing to reverse, nothing to submit. The runtime does not force it; a send with a timeout keeps refund terms. The recipient check closes the one way a transfer without an expiry could be stuck for good.
- Fees fail closed so no transfer can leave before governance has set the schedule.

## Points to check

- `MaxEncodedSize` 256 KiB fits a Polkadot finality proof of about 629 headers of 327 bytes plus a 54 KiB justification. The relayer's grandpa prover must run with `max_block_range` 560 on Nexus and 700 on the Paseo testnet; a larger range builds catch-up proofs the chain refuses.
- The ISMP batch weight is benchmarked per request at a minimal proof and per proof byte at one request, charged as growth over an empty proof, with ECDSA-signed `call_data` on every transfer (the heaviest `on_accept` path). The worst allowed batch is charged 129 ms against 112.6 ms measured.
- Mainnet has no bridge storage, so no migration runs; the 392 and 393 one-off migrations skip, and the try-runtime run below shows the upgrade is idempotent.
- Host functions: the 394 wasm imports 51, 8 more than the live 393 runtime's 43. All 51 are present in the v7.0.1 and v7.0.2 release binaries, and in staging a v6.2.0 validator authored and finalized blocks on 394. Validators need no new node; the relayer's RPC node needs v7.1.0 for the ISMP RPC.
- A sender who sets a timeout gets refund terms and can be refunded by a timeout message through `pallet-ismp` (Hyperbridge's SDK and app do this once their indexer lists Polkadex).
- EVM: `BridgeFees.Fee` lives in a mapping slot, so adding `unit` did not shift storage; the proxies' upstream slots are asserted unchanged in the forge upgrade tests.
- Launch is Ethereum only. The Ethereum proxies and their ProxyAdmins are owned by a 3 of 5 Safe multisig of the five current council members' Ethereum addresses; ownership moves from the deploy key right after deployment, and a council seat change means a 3 of 5 owner swap. Other chains are added later by their own token registration.

## How it was tested

- Runtime tests: 66 of 66 on ce620eb, the #34 plus #35 code (bounds, weights, benchmark scenarios, refunds after a contract change, exact amounts, bridge fee, recipient checks, refund terms only with a timeout). CI green on #29 (run 41836135166), #34 (run 42177837923) and #35 (run 42351504301); #39: 42464211661.
- Benchmarks on a clean build box with steps 50 and repeat 20: `ismp_messages` (consensus by signatures, requests, proof bytes) and the token pallet (send on both paths with `call_data` to 64 KiB, registration and update to 32 chains).
- 96 hour soak on a mainnet-state copy against Hyperbridge's testnet and Sepolia: 156 transfers, 77 from Polkadex to Sepolia and 79 from Sepolia to Polkadex, every one delivered; medians about 6 minutes out and about 23 minutes back; max 43 minutes; relayer tesseract 2.4.19; a log-independent reconciler found nothing undelivered.
- Staging on soak3: governance upgrade to 394 (preimage, council motion, technical committee fast track, referendum, enactment), 12 of 12 post-upgrade checks; relayer at `max_block_range` 700; fees, collector and refund routes set by governance; Sepolia proxies upgraded in place; live tests passed: paid transfers both ways for PDEX and tUSDT, sends without a fee and sends with `call_data` refused, forged and over-bound ISMP messages rejected; reconciler: 70 transfers, 34 out to Sepolia and 36 in, every one delivered, 0 pending.
- try-runtime-cli 0.10.1, `on-runtime-upgrade --checks all --disable-mbm-checks`, on a mainnet snapshot at block 13,403,624 (`0x97d309a1f2f9f5eec5df205cd27e5d210ec812fb6c733daa87651239290e4233`, spec 393), run twice: exit 0, no errors or panics; the whole state decodes (18,602,574 bytes); 48 pallets pass try-state; the migrations are idempotent. The try-runtime build's code hash is `0x21eedbb354101c2fe9b4efca12909102165a1991cb5114e385519516f0e1d411`; it differs from the release wasm because it carries the check code.
- Reproducible build: the same setup rebuilt the live 393 wasm from v7.0.1 byte for byte (blake2_256 `0x9937e36a13254d40b3fe1bcee5da0e6b803c4ab818094233fd519b1f76d1c7b7`, mainnet's code). The 394 wasm rebuilt from the 7.1.0 tree on 8 Nov is byte for byte the 7 Nov build from ce620eb.

## Release artifacts

Built with rustc 1.91.0 (f8297e351, from `rust-toolchain.toml`) and libprotoc 25.3, `cargo build --release --locked -p node-polkadex-runtime --features on-chain-release-build` as in `release.yml`, with the workspace `Cargo.lock`, from the v7.1.0 tree.

| | Bytes | Hash |
|---|---|---|
| `node_polkadex_runtime.compact.compressed.wasm` | 1,465,316 | sha256 `1a35874ae8d93ca27279062206bff8faee03a4099071c5bc6d362c5a7c0a59bb`<br>blake2_256 `0xa3a3856adeb50aa7c148771c437ea2bd011ed663a521486b8cfdf2ed85bb30f8` |
| `System.set_code` preimage (call `0x0002`, the compact length, the code) | 1,465,322 | blake2_256 `0x0b85e150ee8d6348542bb0e9decbd347dabac2ed53dc52c2175164e35850dc6c` |

The runtime reports spec `node` 394, impl 0, transaction version 3.
