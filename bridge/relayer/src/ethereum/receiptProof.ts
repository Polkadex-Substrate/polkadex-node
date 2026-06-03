/**
 * Builds a Merkle Patricia Trie (MPT) inclusion proof for a transaction receipt.
 *
 * The receipt trie in an Ethereum block is keyed by RLP(tx_index) and stores
 * RLP-encoded receipts (or type-prefixed for EIP-2718 transactions).
 *
 * The proof is a list of RLP-encoded trie nodes from the receipts root down to
 * the receipt leaf. This matches the format expected by pallet-eth-bridge's
 * mpt::verify_receipt_proof().
 */

import { Trie } from '@ethereumjs/trie'
import { RLP } from '@ethereumjs/rlp'
import { ethers } from 'ethers'
import type { DepositProof, EthBlockHeader } from '../types'

// ── Receipt encoding ───────────────────────────────────────────────────────

function encodeLog(log: ethers.Log): Uint8Array {
  const address = ethers.getBytes(log.address)
  const topics  = log.topics.map(t => ethers.getBytes(t))
  const data    = ethers.getBytes(log.data)
  return RLP.encode([address, topics, data])
}

function encodeReceipt(receipt: ethers.TransactionReceipt): Uint8Array {
  const status   = receipt.status === 1 ? new Uint8Array([0x01]) : new Uint8Array([0x80])
  const cumGas   = bigintToMinimalBytes(receipt.cumulativeGasUsed)
  const bloom    = ethers.getBytes(receipt.logsBloom)
  const logs     = receipt.logs.map(encodeLog)

  const rlp = RLP.encode([status, cumGas, bloom, logs])

  // EIP-2718: prepend transaction type byte for non-legacy transactions
  if (receipt.type !== 0) {
    const typed = new Uint8Array(1 + rlp.length)
    typed[0] = receipt.type
    typed.set(rlp, 1)
    return typed
  }
  return rlp
}

function bigintToMinimalBytes(value: bigint): Uint8Array {
  if (value === 0n) return new Uint8Array([0])
  const hex = value.toString(16).padStart(2, '0')
  const padded = hex.length % 2 === 0 ? hex : '0' + hex
  return ethers.getBytes('0x' + padded)
}

// ── Proof builder ──────────────────────────────────────────────────────────

export interface ReceiptProofResult {
  header:      EthBlockHeader
  proof:       DepositProof
}

export async function buildReceiptProof(
  provider: ethers.JsonRpcProvider,
  txHash:   string,
  logIndex: number,
  depositNonce: bigint,
): Promise<ReceiptProofResult> {
  const receipt = await provider.getTransactionReceipt(txHash)
  if (!receipt) throw new Error(`Receipt not found for tx: ${txHash}`)

  const block = await provider.getBlock(receipt.blockHash, /* prefetchTxs */ true)
  if (!block) throw new Error(`Block not found: ${receipt.blockHash}`)

  // Fetch all receipts in the block to reconstruct the receipt trie
  const allReceipts = await Promise.all(
    block.prefetchedTransactions.map(tx => provider.getTransactionReceipt(tx.hash)),
  )

  // Build the receipt trie
  const trie = new Trie({ useKeyHashing: false })

  for (const r of allReceipts) {
    if (!r) continue
    const key   = RLP.encode(r.index)          // RLP(tx_index)
    const value = encodeReceipt(r)
    await trie.put(Buffer.from(key), Buffer.from(value))
  }

  // Verify the computed root matches the block header
  const computedRoot = '0x' + Buffer.from(trie.root()).toString('hex')
  if (computedRoot.toLowerCase() !== block.receiptsRoot?.toLowerCase()) {
    throw new Error(
      `Receipt trie root mismatch. Computed: ${computedRoot}, Block: ${block.receiptsRoot}`,
    )
  }

  // Generate inclusion proof for our specific receipt
  const key       = RLP.encode(receipt.index)
  const proofNodes = await trie.createProof(Buffer.from(key))
  const receiptRlp = encodeReceipt(receipt)

  const header: EthBlockHeader = {
    blockNumber:  block.number,
    blockHash:    block.hash ?? '',
    receiptsRoot: block.receiptsRoot ?? '',
    timestamp:    block.timestamp,
  }

  const proof: DepositProof = {
    blockNumber:  block.number,
    txIndex:      receipt.index,
    receiptRlp,
    mptProof:     proofNodes.map(node => new Uint8Array(node)),
    logIndex,
    depositNonce,
  }

  return { header, proof }
}
