/**
 * Polkadex bridge helpers — wraps the pallet-eth-bridge extrinsics and storage queries.
 */

import { getApi, getRelayerPair } from './client'
import type { EthBlockHeader, DepositProof, WithdrawalMessage } from '../types'

// ── Extrinsic submission helpers ───────────────────────────────────────────

function waitForInBlock(tx: ReturnType<typeof import('@polkadot/api').ApiPromise.prototype.tx.ethBridge.submitEthHeader>): Promise<string> {
  return new Promise((resolve, reject) => {
    tx.signAndSend(getRelayerPair(), ({ status, dispatchError }) => {
      if (dispatchError) {
        const err = dispatchError.isModule
          ? dispatchError.asModule.toString()
          : dispatchError.toString()
        reject(new Error(`Dispatch error: ${err}`))
        return
      }
      if (status.isInBlock) {
        resolve(status.asInBlock.toHex())
      }
    }).catch(reject)
  })
}

/** Submit a finalised Ethereum block header so the pallet can verify deposit proofs against it. */
export async function submitEthHeader(header: EthBlockHeader): Promise<string> {
  const api = await getApi()

  const blockHash = await waitForInBlock(
    api.tx.ethBridge.submitEthHeader({
      block_number:  header.blockNumber,
      block_hash:    header.blockHash,
      receipts_root: header.receiptsRoot,
      timestamp:     header.timestamp,
    }) as any,
  )

  console.log(`[Polkadex] EthHeader #${header.blockNumber} submitted — in block ${blockHash}`)
  return blockHash
}

/** Submit an MPT receipt proof to credit bridged WETH to the Polkadex recipient. */
export async function submitDepositProof(proof: DepositProof): Promise<string> {
  const api = await getApi()

  const blockHash = await waitForInBlock(
    api.tx.ethBridge.submitDepositProof({
      block_number:  proof.blockNumber,
      tx_index:      proof.txIndex,
      receipt_rlp:   Array.from(proof.receiptRlp),
      mpt_proof:     proof.mptProof.map(n => Array.from(n)),
      log_index:     proof.logIndex,
      deposit_nonce: proof.depositNonce.toString(),
    }) as any,
  )

  console.log(`[Polkadex] DepositProof nonce=${proof.depositNonce} submitted — in block ${blockHash}`)
  return blockHash
}

// ── Storage queries ────────────────────────────────────────────────────────

/** Read all pending withdrawal messages queued in pallet-eth-bridge storage. */
export async function getPendingWithdrawals(): Promise<WithdrawalMessage[]> {
  const api = await getApi()
  const entries = await api.query.ethBridge.pendingWithdrawals.entries()

  return entries.map(([_key, value]) => {
    const msg = value.toJSON() as any
    return {
      nonce:          BigInt(msg.nonce),
      ethAssetId:     Number(msg.eth_asset_id),
      amount:         BigInt(msg.amount),
      ethRecipient:   msg.eth_recipient,
      polkadexSender: msg.polkadex_sender,
    }
  }).sort((a, b) => Number(a.nonce - b.nonce))
}

/** Check whether a specific deposit nonce has already been processed on-chain. */
export async function isDepositProcessed(nonce: bigint): Promise<boolean> {
  const api = await getApi()
  const result = await api.query.ethBridge.processedDeposits(nonce.toString())
  return result.toJSON() as boolean
}

/** Return the current outgoing nonce (total withdrawals initiated). */
export async function getOutgoingNonce(): Promise<bigint> {
  const api = await getApi()
  const nonce = await api.query.ethBridge.outgoingNonce()
  return BigInt(nonce.toString())
}
