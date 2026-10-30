# EVM token contracts

The token contracts on EVM chains for Polkadex transfers over Hyperbridge, built with Foundry.

- `src/PolkadexHyperFungibleToken.sol`: Polytope's `HyperFungibleTokenUpgradeable`. The token is its own
  bridge: it burns on send and mints on arrival.
- `src/PolkadexWrappedHyperFungibleToken.sol`: Polytope's `WrappedHyperFungibleTokenUpgradeable`. It locks an
  existing ERC-20 on send and releases it on arrival.
- `src/BridgeFees.sol`: the fee and size rules both share.
- `src/TestUSDT.sol`: a 6-decimal token the tests wrap.

## Changes from Polytope's contracts

The two copies keep Polytope's Apache-2.0 notice and add one line saying they were modified.

- A send pays the owner-set fee for its destination chain to the fee collector, in the token sent and on top
  of the amount, before the burn or lock. It reverts while that fee or the collector is unset; a zero fee is
  allowed.
- `to` is capped at 32 bytes and `data` at an owner-set length, which is 0 until set.
- The owner can switch the host contract with `setHost`; `onlyHost` reads it on every call.
- The wrapper pulls the amount plus the fee through the allowance and does not support WETH: `configure`
  refuses it.
- The new state comes after Polytope's storage, so an existing proxy of their contracts can upgrade in place.

`msg.value` still pays Hyperbridge's own relayer fee, and incoming transfers and timeout refunds are unchanged.

## Build and test

```sh
git submodule update --init contracts/evm/lib/openzeppelin-contracts \
  contracts/evm/lib/openzeppelin-contracts-upgradeable contracts/evm/lib/forge-std
cd contracts/evm
forge test
```

The submodules are pinned to OpenZeppelin Contracts 5.6.1, OpenZeppelin Contracts Upgradeable 5.6.1 and
forge-std 1.16.1. `lib/hyperbridge-core` and `lib/solidity-merkle-trees` hold the Polytope files these
contracts import, unmodified, with their Apache-2.0 licences: hyperbridge `sdk/packages/core` at the commit in
`lib/hyperbridge-core/COMMIT`, and `@polytope-labs/solidity-merkle-trees` 1.0.0.

## Staging script

`script/StageFees.s.sol` upgrades an existing token proxy and wrapper proxy to these contracts through their
ProxyAdmins, sets the fee for sends to Polkadex, the fee collector and the `data` cap, and checks that the
upstream state, the supply and the locked funds are unchanged. Run it as the ProxyAdmin owner. Without
`--broadcast` it only simulates:

```sh
HFT_PROXY=<token proxy> WHFT_PROXY=<wrapper proxy> \
  forge script script/StageFees.s.sol --rpc-url <rpc> --sender <owner>
```

| Variable | Meaning | Default |
|---|---|---|
| `HFT_PROXY` | the token proxy | required |
| `WHFT_PROXY` | the wrapper proxy | required |
| `FEE_COLLECTOR` | the account the fees go to | the ProxyAdmin owner |
| `HFT_FEE` | fee per send to Polkadex, in the token's units | `6e18` |
| `WHFT_FEE` | fee per send to Polkadex, in the wrapped token's units | `1e6` |
| `MAX_DATA_LENGTH` | the longest `data` a send may carry, in bytes | `0` |
