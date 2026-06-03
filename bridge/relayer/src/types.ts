// ── Shared bridge types ────────────────────────────────────────────────────
//
// These mirror the Rust structs in pallets/eth-bridge/src/types.rs and the
// Solidity structs in bridge/ethereum/src/interfaces/IPolkadexBridge.sol.
// Keep them in sync when either side changes.

export interface EthBlockHeader {
  blockNumber: number
  blockHash:   string  // 0x-prefixed 32-byte hex
  receiptsRoot: string // 0x-prefixed 32-byte hex
  timestamp:   number
}

export interface DepositProof {
  blockNumber:  number
  txIndex:      number
  receiptRlp:   Uint8Array
  mptProof:     Uint8Array[]
  logIndex:     number
  depositNonce: bigint
}

// Matches `WithdrawalMessage` in pallet-eth-bridge/src/types.rs
export interface WithdrawalMessage {
  nonce:          bigint
  ethAssetId:     number
  amount:         bigint  // in Ethereum decimals (18 for WETH)
  ethRecipient:   string  // 20-byte Ethereum address (0x-prefixed)
  polkadexSender: string  // 32-byte hex (SCALE-encoded Polkadex AccountId)
}

// Matches `BeefyLightClient.Commitment` struct in Solidity
export interface BeefyCommitment {
  messagesRoot:       string  // bytes32 — Merkle root of WithdrawalMessages
  blockNumber:        bigint  // Polkadex block number
  validatorSetId:     bigint
  nextValidatorsHash: string  // bytes32, ZeroHash if no rotation
  nextValidatorsLen:  bigint  // 0 if no rotation
}

export interface MerkleTree {
  root:   string
  leaves: string[]
  tree:   string[][]
}

export interface WithdrawalBatch {
  messages:    WithdrawalMessage[]
  merkleTree:  MerkleTree
  polkadexBlock: number
}
