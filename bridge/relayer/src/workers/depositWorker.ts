/**
 * Deposit worker — Ethereum → Polkadex
 *
 * Flow:
 *   1. Subscribe to Deposit events on PolkadexBridge.sol (Sepolia).
 *   2. Wait ETH_CONFIRMATION_BLOCKS for finality.
 *   3. Build the MPT inclusion proof for the receipt.
 *   4. Submit EthBlockHeader to pallet-eth-bridge (so the pallet has receiptsRoot).
 *   5. Submit DepositProof to pallet-eth-bridge (triggers WETH mint to recipient).
 */

import { ethers } from 'ethers'
import { config } from '../config'
import { getHttpProvider } from '../ethereum/client'
import { getBridgeWatchContract } from '../ethereum/contracts'
import { buildReceiptProof } from '../ethereum/receiptProof'
import { submitEthHeader, submitDepositProof, isDepositProcessed } from '../polkadex/bridge'

// Track which deposit nonces we have already submitted to avoid double-processing
// across restarts (the pallet's ProcessedDeposits storage is the final guard).
const processedNonces = new Set<string>()

export function startDepositWorker(): void {
  console.log('[DepositWorker] Starting — watching PolkadexBridge.sol for Deposit events')

  const bridge = getBridgeWatchContract()

  bridge.on('Deposit', async (
    token:             string,
    sender:            string,
    polkadexRecipient: string,
    amount:            bigint,
    nonce:             bigint,
    event:             ethers.EventLog,
  ) => {
    const nonceStr = nonce.toString()

    try {
      console.log(`[DepositWorker] Deposit event detected — nonce=${nonceStr} amount=${amount} token=${token}`)

      // Skip if we've already handled this nonce in this process
      if (processedNonces.has(nonceStr)) {
        console.log(`[DepositWorker] Nonce ${nonceStr} already queued, skipping`)
        return
      }

      // Skip if the pallet has already processed this nonce (e.g. from a previous run)
      if (await isDepositProcessed(nonce)) {
        console.log(`[DepositWorker] Nonce ${nonceStr} already processed on Polkadex, skipping`)
        processedNonces.add(nonceStr)
        return
      }

      processedNonces.add(nonceStr)

      // Wait for enough confirmations before building the proof
      await waitForConfirmations(event.blockNumber)

      // Build the MPT receipt proof
      const provider = getHttpProvider()
      const { header, proof } = await buildReceiptProof(
        provider,
        event.transactionHash,
        event.index,   // log index within the receipt
        nonce,
      )

      console.log(`[DepositWorker] Proof built for block #${header.blockNumber}, tx index ${proof.txIndex}`)

      // Submit block header first (pallet needs receiptsRoot before verifying proof)
      await submitEthHeader(header)

      // Submit the deposit proof (pallet verifies MPT, parses event, mints WETH)
      await submitDepositProof(proof)

      console.log(`[DepositWorker] ✓ Deposit nonce=${nonceStr} processed on Polkadex`)

    } catch (err) {
      console.error(`[DepositWorker] Error processing deposit nonce=${nonceStr}:`, err)
      // Remove from processed so it can be retried on next event or restart
      processedNonces.delete(nonceStr)
    }
  })

  bridge.on('error', (err: Error) => {
    console.error('[DepositWorker] Contract event error:', err)
  })
}

async function waitForConfirmations(eventBlock: number): Promise<void> {
  const provider = getHttpProvider()
  const needed = config.ethereum.confirmationBlocks

  // Poll until enough blocks have passed
  while (true) {
    const current = await provider.getBlockNumber()
    const confirmations = current - eventBlock
    if (confirmations >= needed) break

    const remaining = needed - confirmations
    console.log(`[DepositWorker] Waiting for ${remaining} more confirmation(s) (${confirmations}/${needed})`)
    await sleep(12_000) // ~one Ethereum block
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms))
}
