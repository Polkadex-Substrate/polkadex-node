# Runtime 393: notes for reviewers

The plain-words version for holders is [runtime-393.md](runtime-393.md). This page is for code review.

## What changes

| Commit | Change |
|---|---|
| `vendor: pallet-democracy 45.0.0 and pallet-elections-phragmen 46.0.0, unchanged` | Copies of the crates.io releases, minus their `Cargo.lock`. `diff -r` against the published crates (sha256 `f1cd5208…` and `e4a54cc6…`, the checksums in `Cargo.lock`) shows nothing else. |
| `vendor, runtime: votes count PDEX held for staking` | Both pallets get `type VotingHold: Convert<AccountId, Balance>`. Democracy `try_vote` and `try_delegate` check the vote against `free_balance + VotingHold`; phragmen `vote` caps the stake at `free_balance + VotingHold`. The runtime sets `VotingHold = StakedBalance`, which returns `pallet_staking::asset::staked` (the staking hold). The upstream mocks set `VotingHold = ()`. `[patch.crates-io]` points the workspace at the copies. |
| `runtime: fees through FungibleAdapter, ...` | `OnChargeTransaction` moves from the deprecated `CurrencyAdapter<Balances, DealWithFees>` to `FungibleAdapter<Balances, ResolveTo<TreasuryAccount, Balances>>`. |
| `runtime: tests for votes and fees with staked PDEX` | `runtimes/mainnet/src/vote_tests.rs`, seven tests. |
| `runtime: spec_version 393` | `impl_version` 0; `transaction_version` stays 3, since no call changes. |
| `runtime: add the staked balance read to vote weights` | `DemocracyWeights` and `ElectionsWeights` wrap the upstream `SubstrateWeight`s and add `staked_balance_read()` to `vote_new`, `vote_existing`, `delegate`, `vote_equal`, `vote_more` and `vote_less`, the weights of the calls that look up `VotingHold`. That is one database read (`RocksDbWeight`, 25 µs) and 3,892 bytes of proof: the benchmark CLI's estimate (frame-benchmarking-cli 53.0.0, `MaxEncodedLen` mode) for a first read of `Balances::Holds` at its upstream size of 427 bytes, which is the item plus 5 map layers and 2 top-level layers of 15 × 33 bytes. Every other weight is the upstream one. |

## Why

- From runtime 392, pallet-staking 45.1.0 keeps the bond on a hold, and a hold is outside the free balance. Democracy and phragmen checked votes against `free_balance` only, so staked PDEX could not back a vote or a delegation, and it did not count in council votes.
- Fees went through legacy `Currency::withdraw` (pallet-balances 46.0.0, `impl_currency.rs`), which requires the free balance left after the fee to cover `frozen`. With the bond on a hold, any lock larger than the free balance blocks every fee. On mainnet that is (a) the old `staking ` lock, which `StakingLedger::update` leaves beside the new hold until `migrate_currency` removes it (`pallet/impls.rs`), and (b) vote locks set before 392. The fungible path (`impl_fungible.rs`, `reducible_balance`) applies `frozen` to free plus reserved, which is what the rest of pallet-balances does. Without this change, the vote fix itself would lock voters out of fees.
- Free plus the staking hold, and not `total_balance` as `pallet-conviction-voting` uses: in phragmen the balance caps the stake, so `total_balance` would also count every voter's own voting bond, a candidate's 100 PDEX candidacy bond and any other deposit. That would change council weights for accounts that do not stake. Counting deposits is a policy question for its own referendum.

## Points to check

- `FungibleAdapter` in pallet-transaction-payment 45.0.0 parks the withdrawn fee in `TxPaymentCredit` between withdrawal and settlement; this is the upstream design. Fees and tips are resolved into the Treasury account. A per-fee `Treasury::Deposit` event is no longer emitted; the `Balances::Deposit` to the Treasury account remains. For an account with nothing reserved, the fee check is the same as before.
- The 392 one-off migrations stay in `Migrations` and skip on this upgrade: each checks `last_runtime_upgrade_spec_version() > 391` or a storage version. No migration is added.
- Host functions (release procedure 4.1): 393 imports the same 44 host functions as 392. No node upgrade is needed.
- Weights: the six weights above carry a conservative allowance for the `VotingHold` read, not a benchmark of the runtime provider. The pallets' own benchmarks run with `VotingHold = ()`, so they cannot measure it.
- With `VotingHold = ()` the vendored pallets behave exactly as published.
- Outside the runtime, stashes that carry the old staking lock beside the hold are cleaned with `staking.migrate_currency`, which is permissionless and fee-free when it succeeds.

## How it was tested

- **Runtime tests:** 32 pass (24 existing, 8 new). The new ones cover a stash fully bonded on the hold by `migrate_currency` voting and delegating its whole bond; a staker's council stake (free plus staked, less the voting bond) and the election that uses it; vote locks and holds together (transfers and fees from what the vote leaves free, unbond and withdraw under the lock, unlock after the conviction period); accounts without stake voting exactly as before, with deposits not counting; votes cast before the upgrade keeping their weight through the upgrade and `migrate_currency`; `migrate_currency` clearing an old staking lock left beside the hold without trimming the bond; and the vote weights, each the upstream weight plus one `Balances::Holds` read, with a proof allowance that covers this runtime's `Balances::Holds` item.
- **Upstream suites:** 134 tests (democracy 63, elections-phragmen 71) pass in polkadot-sdk at `ac2b0e3fd86a23cc33b2f4d6f46ac9d0ecb05115`, the commit in the crates' `.cargo_vcs_info.json`. That tree's sources equal the published crates, and our changed sources replace them for the run.
- **CI checks** on a clean build box: the pin check. At `4ae9c34`, the head before the weight commit, also `cargo check -p polkadex-node` and the node checks with `try-runtime` and with `runtime-benchmarks`; the weight commit changes runtime code only.
- **try-runtime:** try-runtime-cli 0.10.1, `on-runtime-upgrade --checks all --disable-mbm-checks`, on a mainnet snapshot at block 13,354,649 taken from an archive node. Exit 0, no errors, no panics, the 392 migrations skip, the whole state decodes, and the try-state checks of every pallet pass.
- **Mainnet fork (chopsticks), 15 of 15 checks.**
  - On 392, a real staker's referendum vote and delegation with its staked PDEX fail with `InsufficientFunds`, and its council stake counts only the free balance. A stash whose pre-392 vote lock exceeds its free balance cannot pay a fee. A referendum is started by external majority and fast track.
  - The 393 `set_code` preimage is noted with the hash below.
  - The upgrade is dispatched by Root through the scheduler. The fork's own executor cannot read runtime versions for `set_code` at all, which shows with the live 392 code as well, so the fork enacts with `set_code_without_checks`.
  - On 393, every stored democracy vote, council vote and referendum is unchanged. The same staker votes, delegates and votes in the council election with free plus staked PDEX. The stuck stash pays a fee. A fee is charged once, refunded down to the actual fee and received by the Treasury, with nothing left parked.
- **Reproducible build:** the build setup below rebuilds the live 392 wasm byte for byte from commit `df09c11` (blake2_256 `0xccaf34613316e742db4cb844a55c5750148139f0aa181d84f986d12fbe9c0273`).

## Release artifacts

Built on Ubuntu 26.04 with Rust 1.91.0 (from `rust-toolchain.toml`), in `/root/polkadex-node`, with `cargo build --release --locked -p node-polkadex-runtime --features on-chain-release-build` and the workspace `Cargo.lock`.

| | Bytes | Hash |
|---|---|---|
| `node_polkadex_runtime.compact.compressed.wasm` | 1,271,710 | sha256 `66e8dd005d6020510694253a65aba5e17a6082bb6fa6833f766ffc1476ff1fe2`<br>blake2_256 `0x9937e36a13254d40b3fe1bcee5da0e6b803c4ab818094233fd519b1f76d1c7b7` |
| `System.set_code` preimage (call `0x0002`, the compact length, the code) | 1,271,716 | blake2_256 `0xe463c230ff613a96570c88e7230e8f8b20973b8deb2b090641195af5f8fb3510` |

The runtime reports spec `node` 393, impl 0, transaction version 3. The same method gives the preimage of the 392 referendum from the live 392 wasm.
