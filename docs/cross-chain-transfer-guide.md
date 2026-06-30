# Cross-Chain Transfer Guide — Polkadex Testnet

This guide explains how to get testnet tokens and transfer them between **Polkadex** and **Sepolia (Ethereum testnet)**.

---

## What You Can Do

- Get free testnet tokens directly from the **Polkadex Exchange UI**
- Transfer tokens **from Sepolia to Polkadex**
- Transfer tokens **from Polkadex back to Sepolia**

The same set of tokens is available on both chains: **PDEX, WETH, WBTC, USDC, USDT, LINK, UNI, AAVE, wstETH**.

---

## Recommended Wallet - Enkrypt

We recommend using **[Enkrypt](https://www.enkrypt.com/)** as your wallet for testing.

Enkrypt is a multi-chain browser extension wallet that supports both **Ethereum (Sepolia)** and **Polkadex** in a single extension - so you don't need to juggle two separate wallets.

**Setup:**

1. Install the [Enkrypt extension](https://www.enkrypt.com/) for your browser
2. Create a new wallet or import an existing one
3. Switch to the **Sepolia** network for your Ethereum address
4. Switch to the **Polkadex** network for your Polkadex address

Both addresses are accessible from the same Enkrypt interface.

> MetaMask (Ethereum only) and the Polkadot.js extension (Polkadex only) also work if you prefer to use them separately.

---

## Step 1 — Get Testnet Tokens from the Faucet

Open the **Polkadex Exchange UI** and navigate to the Faucet section.

You can request tokens for both chains from there:

- **Polkadex tokens** - enter your Polkadex wallet address to receive PDEX and other assets directly on-chain
- **Sepolia tokens** - enter your Ethereum wallet address to receive ERC20 tokens on Sepolia

> Each token has a **daily limit of 1 drip per address**. Wait 24 hours before requesting again.

---

## Step 2 — Cross-Chain Transfers

Once you have tokens, you can bridge them in either direction through the Exchange UI.

### Sepolia → Polkadex

1. Connect your Ethereum wallet (MetaMask or Enkrypt) to the Exchange UI
2. Select the token and amount you want to transfer
3. Enter your Polkadex wallet address as the destination
4. Confirm the transactions in your wallet - **your wallet will prompt you twice** (once to approve the token, once to send)
5. Wait a few minutes for the transfer to arrive on Polkadex

> **Note:** Transfers from Sepolia require a small amount of **Sepolia ETH for gas fees**. Get free Sepolia ETH from [sepoliafaucet.com](https://cloud.google.com/application/web3/faucet/ethereum/sepolia).

### Polkadex → Sepolia

1. Connect your Polkadex wallet (Enkrypt or Polkadot.js extension) to the Exchange UI
2. Select the token and amount
3. Enter your Ethereum wallet address as the destination
4. Sign the transaction
5. Wait a few minutes for the transfer to arrive on Sepolia

> Transfers from Polkadex **do not require ETH** - only a small amount of PDEX for the Polkadex network fee.

---

## Sepolia Token Contracts

To see your Sepolia token balances in MetaMask or Enkrypt, import the following contract addresses:

| Token  | Contract Address                             |
| ------ | -------------------------------------------- |
| USDC   | `0xb177b85d589B806E9e82C02e5b92180a4B4d90bb` |
| USDT   | `0x086d2f4CCD29D6CbD921EF0aa09EC20F67f7d69D` |
| WBTC   | `0xf32CCA1B10C65553690F9F72Afe8df13CC33A406` |
| LINK   | `0xEfa898bCb94Cc119F4687F47dc77E68f5F097197` |
| UNI    | `0x491497cf6ec0D498A0586Af9679F0F5dA94e4e24` |
| AAVE   | `0x8D7392d6e955a87B41383037826157011700B2c8` |
| wstETH | `0xcF47f5C69aE7bEee74C12d37fe5842dA64e4f9aa` |

**How to import a token in MetaMask / Enkrypt:**

1. Open wallet extension and switch to the **Sepolia** network
2. Scroll down and click **Import tokens**
3. Paste the contract address - the token symbol and decimals will fill in automatically
4. Click **Add custom token**

---

## Transfer Times

Transfers typically complete around **20 minutes** after the transaction is confirmed. Its dependent on the ethereum sepolia chain and the cross chain testnet.
