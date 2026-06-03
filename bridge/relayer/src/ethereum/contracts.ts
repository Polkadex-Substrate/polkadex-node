import { ethers } from 'ethers'
import { config } from '../config'
import { getWsProvider, getHttpProvider, getRelayerWallet } from './client'

// ── Minimal ABIs (only the functions/events we need) ──────────────────────

const POLKADEX_BRIDGE_ABI = [
  // Event emitted on every deposit
  'event Deposit(address indexed token, address indexed sender, bytes32 indexed polkadexRecipient, uint256 amount, uint64 nonce)',
  // Read the current deposit nonce
  'function depositNonce() view returns (uint64)',
]

const BEEFY_LIGHT_CLIENT_ABI = [
  // Submit a signed BEEFY commitment to update the messages root
  `function submitCommitment(tuple(
    tuple(bytes32 messagesRoot, uint64 blockNumber, uint64 validatorSetId, bytes32 nextValidatorsHash, uint64 nextValidatorsLen) commitment,
    bytes[] signatures
  ) signedCommitment)`,
  // Compute the digest validators must sign (exposed for off-chain use)
  `function commitmentDigest(tuple(
    bytes32 messagesRoot, uint64 blockNumber, uint64 validatorSetId,
    bytes32 nextValidatorsHash, uint64 nextValidatorsLen
  ) c) pure returns (bytes32)`,
  // Read the latest committed messages root
  'function latestMmrRoot() view returns (bytes32)',
  'function latestBeefyBlock() view returns (uint64)',
  // Read the current validator set
  'function currentValidators() view returns (address[])',
]

// ── Contract instances ─────────────────────────────────────────────────────

// Read-only instance subscribed via WebSocket (for event watching)
export function getBridgeWatchContract(): ethers.Contract {
  return new ethers.Contract(
    config.ethereum.polkadexBridge,
    POLKADEX_BRIDGE_ABI,
    getWsProvider(),
  )
}

// Write instance connected to the relayer wallet (for BeefyLightClient updates)
export function getBeefyLightClientContract(): ethers.Contract {
  return new ethers.Contract(
    config.ethereum.beefyLightClient,
    BEEFY_LIGHT_CLIENT_ABI,
    getRelayerWallet(),
  )
}

// Read-only BeefyLightClient for view calls
export function getBeefyLightClientReadOnly(): ethers.Contract {
  return new ethers.Contract(
    config.ethereum.beefyLightClient,
    BEEFY_LIGHT_CLIENT_ABI,
    getHttpProvider(),
  )
}
