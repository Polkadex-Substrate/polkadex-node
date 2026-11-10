# Runtime 394: the bridge to Ethereum comes to Polkadex

Plan as of 9 Nov 2026.

## In short

- Runtime 394 adds Hyperbridge to Polkadex. It is the bridge Polkadex will use to move tokens to and from Ethereum. Other chains such as BSC can follow later, each by its own governance decision, once the bridge has run on Ethereum for a while.
- The vote enacts the runtime. It does not open the bridge by itself. Bridging opens step by step after that, and each step is listed below.
- Transfers are paid: a small bridge fee, set by governance, in the token you send. Fees pay for the bridge's bandwidth and the relayer.
- A transfer stays valid until it is delivered.
- Validators need no new node for 394. Only the relayer's RPC node runs node v7.1.0, which serves the bridge's RPC.
- Nothing changes for holders who do not bridge. Staking, voting, fees and balances work as they do on 393.

## What you get

- Send PDEX to Ethereum. Your PDEX is kept in escrow on Polkadex and the same amount is minted to you on Ethereum as an ERC-20 token. Send it back and the escrow releases.
- Bring tokens in from Ethereum, such as USDT. The token is locked on Ethereum and the same amount is minted to you on Polkadex as an asset. Send it back and the lock releases.
- Which tokens can cross is decided by governance, one token at a time. The first list is part of the opening steps below.
- Every transfer is checked by both sides with proofs of finality, not by a small group of signers like a multisig bridge. Polkadex checks Hyperbridge with Polkadot's own finality; Hyperbridge checks Polkadex with Polkadex's own finality. Details for reviewers are in runtime-394-review.md.

## What a transfer costs

- The bridge fee: a fixed amount per token and destination, set by governance, taken in the token you send, on top of the amount. It goes to a fee collector account that governance names. Until a fee and a collector are set, sends are refused, so nothing can leave by accident before the schedule exists. The schedule is published with the opening steps and is visible on chain.
- Your normal transaction fee on the chain you send from. On Ethereum that is your own gas.
- An optional relayer tip. You can leave it at zero.
- On the Ethereum side, our token contracts charge the same kind of fixed fee for sends to Polkadex, in the token sent.

## How long a transfer takes

From our testnet soak of 96 hours and Polytope's figures for their mainnet:

- Polkadex to Ethereum: usually 5 to 10 minutes.
- Ethereum to Polkadex: usually 20 to 30 minutes. Most of it is Ethereum finality itself; Hyperbridge sees a new Ethereum block about every 6 minutes, and that block is already about 13 minutes old.
- Slower deliveries happen when a relayer or an RPC is down. The longest we saw in the soak was 43 minutes, and every transfer arrived.

## Why a transfer waits instead of expiring

A bridge message can carry an expiry. For a plain token transfer an expiry adds nothing: if the message is not delivered in time, it has to be reversed with a second proof that someone must send, and the tokens are back where they started. So the Polkadex apps send transfers without an expiry, and a transfer stays valid until a relayer delivers it. A slow relayer means a slow transfer. The chain itself still accepts an expiry; a sender who sets one keeps the right to a refund once it passes.

Because a transfer without an expiry cannot be reversed, the chain refuses a transfer that could never be credited before any token moves: a recipient on Ethereum must be a 20-byte address, and a recipient on Polkadex must be a 32-byte account, and never the all-zero address. The contracts on the other side make the same check.

## What governance controls

- By referendum: which bridge Polkadex trusts (the consensus clients and the chains they prove), which tokens can cross and their contract addresses, and the account that collects fees.
- By the council, or by referendum: the fee for each token and destination, and the size cap for data a transfer may carry (none by default).
- Nothing about the bridge can be changed by a single account. A transfer-restricted proxy cannot bridge funds.
- On Ethereum, the token contracts sit behind upgradeable proxies. The proxies and their admin contracts are owned by a 3 of 5 Safe multisig held by the five current council members' Ethereum addresses. Ownership moves from the deploy key to the Safe right after deployment, and when a council seat changes, the Safe's owners change with it, 3 of 5 approving. That is how the trusted bridge contract can be switched later without moving anyone's tokens.

## What happens before bridging opens on mainnet

In this order, each announced in the main group when it is done:

1. Runtime 394 is enacted (this vote).
2. Hyperbridge's own governance adds Polkadex to its mainnet. Polkadex's governance creates the matching client for Polkadot's finality and points the bridge at Hyperbridge's mainnet.
3. The token contracts are deployed on Ethereum and handed to the council's Safe, and the first tokens are registered on Polkadex by referendum.
4. Fees and the fee collector are set, and bandwidth on Hyperbridge is bought. Relayers are set up with their RPC nodes.
5. Live transfers in both directions on mainnet with small amounts, checked end to end, before the bridge is announced as open.

BSC follows later by the same steps, once the bridge has run on Ethereum for a while.

## Check the runtime yourself

The release page on GitHub lists the compressed wasm with its sha256 and blake2 hashes, and the hash of the `set_code` call the referendum carries. The referendum page on the explorer shows the same `set_code` hash. To recompute from the downloaded file: `sha256sum <file>` and `b2sum -l 256 <file>` for the wasm, or `subwasm info <file>` for both the blake2 hash and the proposal hash.

- Wasm (1,463,893 bytes) blake2: `0x10eda81162251e928f52560e1a4779d59f043b5b8d31170dc7b3e354b5266097`
- Wasm sha256: `4812394858bf2a6343c5daa74e16a3eeeaca6396c64ab65529e8ca00898dc621`
- `set_code` hash: `0x783a5b869b9a366076a7d17ca9a3dd67e82f37f68e6803d2a4a07a236e8a8fb0`

## How to vote

The referendum link goes here when the vote opens. On explorer.polkadex.ee the referendum page has a Vote button. Since 393, staked PDEX counts in your vote. Keep at least 1 PDEX out of every vote, so fees stay payable.

## The steps

1. Code review by two team reviewers.
2. Release build and the try-runtime check on a copy of mainnet state.
3. The council proposes 394 and the technical committee fast-tracks it, with a 7 day vote and a 1 day delay.
4. The referendum runs, with the voting window posted in the main group in UTC.
5. 394 goes live one day after the vote ends.

We post the exact times in the main group before the vote opens.
