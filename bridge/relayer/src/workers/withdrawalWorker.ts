/**
 * Withdrawal worker — Polkadex → Ethereum
 *
 * Flow:
 *   1. Poll pallet-eth-bridge's PendingWithdrawals storage on Polkadex.
 *   2. When enough messages accumulate (WITHDRAWAL_BATCH_SIZE), build a Merkle batch.
 *   3. Sign the batch root with the relayer's Ethereum key (must be in the BEEFY validator set).
 *   4. Submit the SignedCommitment to BeefyLightClient.sol.
 *   5. Store the Merkle proofs so users can call PolkadexBridge.withdraw() on Ethereum.
 *
 * Security note:
 *   In this initial implementation the relayer is the sole BEEFY validator (1-of-1).
 *   For production, multiple validators must sign; deploy BeefyLightClient.sol with
 *   their addresses and collect signatures from each before submitting.
 */

import { ethers } from 'ethers'
import { config } from '../config'
import { getRelayerWallet } from '../ethereum/client'
import { getBeefyLightClientContract, getBeefyLightClientReadOnly } from '../ethereum/contracts'
import { getPendingWithdrawals, getOutgoingNonce } from '../polkadex/bridge'
import type { WithdrawalMessage, BeefyCommitment, MerkleTree, WithdrawalBatch } from '../types'
import { getApi } from '../polkadex/client'

// Track the highest nonce we have already submitted to Ethereum
let lastSubmittedNonce = -1n

export function startWithdrawalWorker(): void {
  console.log('[WithdrawalWorker] Starting — polling Polkadex PendingWithdrawals')
  poll().catch(err => console.error('[WithdrawalWorker] Fatal poll error:', err))
}

async function poll(): Promise<void> {
  while (true) {
    try {
      await processPendingWithdrawals()
    } catch (err) {
      console.error('[WithdrawalWorker] Error in poll cycle:', err)
    }
    await sleep(config.relayer.withdrawalPollMs)
  }
}

async function processPendingWithdrawals(): Promise<void> {
  const messages = await getPendingWithdrawals()

  // Only process messages with nonces we haven't submitted yet
  const newMessages = messages.filter(m => m.nonce > lastSubmittedNonce)

  if (newMessages.length === 0) return

  if (newMessages.length < config.relayer.withdrawalBatchSize) {
    console.log(
      `[WithdrawalWorker] ${newMessages.length} pending withdrawal(s), ` +
      `waiting for batch of ${config.relayer.withdrawalBatchSize}`,
    )
    return
  }

  console.log(`[WithdrawalWorker] Processing batch of ${newMessages.length} withdrawal(s)`)

  const api = await getApi()
  const currentBlock = (await api.rpc.chain.getBlock()).block.header.number.toNumber()

  const batch = buildBatch(newMessages, currentBlock)
  await submitBatch(batch)

  lastSubmittedNonce = newMessages[newMessages.length - 1].nonce
  console.log(`[WithdrawalWorker] ✓ Batch submitted. Latest nonce: ${lastSubmittedNonce}`)

  // Print Merkle proofs so users know how to call PolkadexBridge.withdraw()
  printWithdrawalInstructions(batch)
}

// ── Merkle batch building ──────────────────────────────────────────────────

function buildBatch(messages: WithdrawalMessage[], polkadexBlock: number): WithdrawalBatch {
  const leaves = messages.map(hashLeaf)
  const merkleTree = buildMerkleTree(leaves)
  return { messages, merkleTree, polkadexBlock }
}

/**
 * Compute the leaf hash for a withdrawal message.
 * Must match MerkleProof.hashLeaf() in bridge/ethereum/src/lib/MerkleProof.sol:
 *   keccak256(abi.encodePacked(bytes1(0x00), nonce, assetId, amount, recipient, polkadexSender))
 */
function hashLeaf(msg: WithdrawalMessage): string {
  return ethers.solidityPackedKeccak256(
    ['bytes1', 'uint64', 'uint32', 'uint256', 'address', 'bytes32'],
    [
      '0x00',
      msg.nonce,
      msg.ethAssetId,
      msg.amount,
      msg.ethRecipient,
      msg.polkadexSender,
    ],
  )
}

/**
 * Hash two adjacent nodes into a parent.
 * Must match MerkleProof._hashInternalNode():
 *   keccak256(abi.encodePacked(bytes1(0x01), left, right))
 */
function hashInternalNode(left: string, right: string): string {
  return ethers.solidityPackedKeccak256(
    ['bytes1', 'bytes32', 'bytes32'],
    ['0x01', left, right],
  )
}

function buildMerkleTree(leaves: string[]): MerkleTree {
  if (leaves.length === 0) throw new Error('Cannot build Merkle tree with no leaves')

  let level = [...leaves]
  const tree: string[][] = [level]

  while (level.length > 1) {
    const next: string[] = []
    for (let i = 0; i < level.length; i += 2) {
      if (i + 1 < level.length) {
        next.push(hashInternalNode(level[i], level[i + 1]))
      } else {
        next.push(level[i]) // odd node promoted unchanged (no duplication)
      }
    }
    level = next
    tree.push(level)
  }

  return { root: level[0], leaves, tree }
}

function getMerkleProof(tree: MerkleTree, leafIndex: number): string[] {
  const proof: string[] = []
  let index = leafIndex

  for (let level = 0; level < tree.tree.length - 1; level++) {
    const row = tree.tree[level]
    const siblingIdx = index % 2 === 0 ? index + 1 : index - 1
    if (siblingIdx < row.length) {
      proof.push(row[siblingIdx])
    }
    index = Math.floor(index / 2)
  }

  return proof
}

// ── BEEFY commitment signing and submission ────────────────────────────────

async function submitBatch(batch: WithdrawalBatch): Promise<void> {
  const commitment: BeefyCommitment = {
    messagesRoot:       batch.merkleTree.root,
    blockNumber:        BigInt(batch.polkadexBlock),
    validatorSetId:     config.ethereum.validatorSetId,
    nextValidatorsHash: ethers.ZeroHash,
    nextValidatorsLen:  0n,
  }

  // Compute the digest the validators must sign.
  // This matches BeefyLightClient.commitmentDigest() exactly.
  const lc = getBeefyLightClientReadOnly()
  const digest: string = await lc.commitmentDigest({
    messagesRoot:       commitment.messagesRoot,
    blockNumber:        commitment.blockNumber,
    validatorSetId:     commitment.validatorSetId,
    nextValidatorsHash: commitment.nextValidatorsHash,
    nextValidatorsLen:  commitment.nextValidatorsLen,
  })

  // Sign the raw digest (no eth_sign prefix — matches _ecrecover in Solidity)
  const wallet = getRelayerWallet()
  const signingKey = new ethers.SigningKey(wallet.privateKey)
  const rawSig = signingKey.sign(digest)

  // Encode as [r (32 bytes) | s (32 bytes) | v (1 byte)] = 65 bytes
  const encodedSig = ethers.concat([rawSig.r, rawSig.s, ethers.toBeHex(rawSig.v, 1)])

  console.log(`[WithdrawalWorker] Submitting commitment — root=${commitment.messagesRoot} block=${commitment.blockNumber}`)

  const lc_write = getBeefyLightClientContract()
  const tx: ethers.TransactionResponse = await lc_write.submitCommitment({
    commitment: {
      messagesRoot:       commitment.messagesRoot,
      blockNumber:        commitment.blockNumber,
      validatorSetId:     commitment.validatorSetId,
      nextValidatorsHash: commitment.nextValidatorsHash,
      nextValidatorsLen:  commitment.nextValidatorsLen,
    },
    signatures: [encodedSig],
  })

  const receipt = await tx.wait()
  console.log(`[WithdrawalWorker] Commitment confirmed — Ethereum tx: ${receipt?.hash}`)
}

// ── User instructions ──────────────────────────────────────────────────────

function printWithdrawalInstructions(batch: WithdrawalBatch): void {
  console.log('\n[WithdrawalWorker] ── Merkle proofs for users to claim on Ethereum ──')

  batch.messages.forEach((msg, i) => {
    const proof = getMerkleProof(batch.merkleTree, i)
    console.log(`  Withdrawal nonce=${msg.nonce}:`)
    console.log(`    recipient:  ${msg.ethRecipient}`)
    console.log(`    amount:     ${msg.amount} (Ethereum decimals)`)
    console.log(`    leafIndex:  ${i}`)
    console.log(`    leafCount:  ${batch.messages.length}`)
    console.log(`    proof:      [${proof.join(', ')}]`)
    console.log(`    → call PolkadexBridge.withdraw({`)
    console.log(`        nonce: ${msg.nonce},`)
    console.log(`        assetId: ${msg.ethAssetId},`)
    console.log(`        amount: ${msg.amount},`)
    console.log(`        recipient: "${msg.ethRecipient}",`)
    console.log(`        polkadexSender: "${msg.polkadexSender}"`)
    console.log(`      }, proof, ${i}, ${batch.messages.length})`)
  })
  console.log('[WithdrawalWorker] ────────────────────────────────────────────────────\n')
}

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms))
}
