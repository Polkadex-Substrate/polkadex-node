// This file is part of Polkadex.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

use parity_scale_codec::{Decode, DecodeWithMemTracking, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use sp_std::vec::Vec;

/// A finalised Ethereum block header, trimmed to the fields needed for bridge verification.
/// The relayer submits this; governance can rotate the relayer if it misbehaves.
#[derive(Clone, Encode, Decode, DecodeWithMemTracking, TypeInfo, Debug, PartialEq, MaxEncodedLen)]
pub struct EthBlockHeader {
    /// Ethereum block number (height).
    pub block_number: u64,
    /// keccak256 block hash — used for human-readable identification and logging.
    pub block_hash: [u8; 32],
    /// `receiptsRoot` from the Ethereum block header.
    /// All deposit proofs are verified against this root.
    pub receipts_root: [u8; 32],
    /// Unix timestamp of the block (for ordering / TTL checks in future versions).
    pub timestamp: u64,
}

/// Everything needed to prove that a specific `Deposit` event occurred on Ethereum.
///
/// Flow:
///   1. User calls `deposit()` on `PolkadexBridge.sol` on Ethereum.
///   2. Relayer submits the block header via `submit_eth_header`.
///   3. Anyone constructs this proof from the block's receipt trie and calls
///      `submit_deposit_proof` to credit WETH on Polkadex.
#[derive(Clone, Encode, Decode, DecodeWithMemTracking, TypeInfo, Debug, PartialEq)]
pub struct DepositProof {
    /// Block number in which the deposit transaction was included.
    pub block_number: u64,
    /// Transaction index within the block (used as the MPT key).
    pub tx_index: u64,
    /// RLP-encoded transaction receipt.
    /// For legacy receipts: `RLP([status, cumGas, logsBloom, logs])`.
    /// For typed (EIP-2718): `type_byte || RLP([status, cumGas, logsBloom, logs])`.
    pub receipt_rlp: Vec<u8>,
    /// Merkle Patricia Trie proof: ordered list of RLP-encoded trie nodes
    /// from the receipts root down to the receipt leaf.
    pub mpt_proof: Vec<Vec<u8>>,
    /// Index of the `Deposit` log within the receipt's logs array.
    pub log_index: u32,
    /// Deposit nonce from the `Deposit` event — used as the replay-prevention key.
    pub deposit_nonce: u64,
}

/// Runtime configuration for a bridged token pair.
/// Stored in `TokenRegistry`, keyed by the Ethereum ERC-20 address.
#[derive(Clone, Encode, Decode, DecodeWithMemTracking, TypeInfo, Debug, PartialEq, MaxEncodedLen)]
pub struct TokenConfig {
    /// Polkadex asset ID used to mint/burn the bridged token via `pallet-assets`.
    pub polkadex_asset_id: u128,
    /// Ethereum asset ID (uint32) stored in the `WithdrawMessage` leaf on Ethereum.
    /// Must match the `assetId` registered in `TokenRegistry.sol`.
    pub eth_asset_id: u32,
    /// Number of decimal places the token uses on Ethereum (e.g. 18 for WETH, 6 for USDC).
    /// Polkadex always stores balances with 12 decimal places, so amounts are converted
    /// at deposit (ETH→Polkadex) and withdrawal (Polkadex→ETH) time.
    pub decimals: u8,
}

impl TokenConfig {
    /// Convert an amount from Ethereum decimals to Polkadex native decimals (12).
    ///
    /// Example (WETH, 18 decimals):
    ///   `1_000_000_000_000_000_000` (1 ETH, 18 dec) → `1_000_000_000_000` (1 WETH, 12 dec)
    pub fn eth_to_native(&self, amount: u128) -> u128 {
        let diff = 12i16 - self.decimals as i16;
        match diff.cmp(&0) {
            core::cmp::Ordering::Less =>
                amount.saturating_div(10u128.pow((-diff) as u32)),
            core::cmp::Ordering::Equal => amount,
            core::cmp::Ordering::Greater =>
                amount.saturating_mul(10u128.pow(diff as u32)),
        }
    }

    /// Convert an amount from Polkadex native decimals (12) to Ethereum decimals.
    ///
    /// Example (WETH, 18 decimals):
    ///   `1_000_000_000_000` (1 WETH, 12 dec) → `1_000_000_000_000_000_000` (1 ETH, 18 dec)
    pub fn native_to_eth(&self, amount: u128) -> u128 {
        let diff = 12i16 - self.decimals as i16;
        match diff.cmp(&0) {
            core::cmp::Ordering::Less =>
                amount.saturating_mul(10u128.pow((-diff) as u32)),
            core::cmp::Ordering::Equal => amount,
            core::cmp::Ordering::Greater =>
                amount.saturating_div(10u128.pow(diff as u32)),
        }
    }
}

/// An outgoing withdrawal queued for inclusion in the next BEEFY batch.
///
/// Fields mirror `PolkadexBridge.sol → WithdrawMessage`:
/// ```solidity
/// struct WithdrawMessage {
///     uint64  nonce;
///     uint32  assetId;
///     uint256 amount;
///     address recipient;
///     bytes32 polkadexSender;
/// }
/// ```
/// The Ethereum contract computes the Merkle leaf as:
/// `keccak256(abi.encodePacked(0x00, nonce, assetId, amount, recipient, polkadexSender))`
#[derive(Clone, Encode, Decode, DecodeWithMemTracking, TypeInfo, Debug, PartialEq, MaxEncodedLen)]
pub struct WithdrawalMessage {
    /// Unique, monotonically increasing nonce. Ethereum uses this as the replay-prevention key.
    pub nonce: u64,
    /// Ethereum asset ID — looked up by the bridge contract to determine which ERC-20 to release.
    pub eth_asset_id: u32,
    /// Amount in the token's Ethereum decimals (18 for WETH).
    pub amount: u128,
    /// Ethereum address that will receive the released tokens.
    pub eth_recipient: [u8; 20],
    /// SCALE-encoded Polkadex AccountId of the initiator, padded to 32 bytes.
    pub polkadex_sender: [u8; 32],
}

/// A parsed Ethereum event log.
#[derive(Clone, Debug, PartialEq)]
pub struct EthLog {
    /// Address of the contract that emitted this log.
    pub address: [u8; 20],
    /// Indexed event topics. `topics[0]` is always the event signature hash.
    pub topics: Vec<[u8; 32]>,
    /// ABI-encoded non-indexed event parameters.
    pub data: Vec<u8>,
}

/// Decoded `Deposit` event from `PolkadexBridge.sol`:
/// ```solidity
/// event Deposit(
///     address indexed token,
///     address indexed sender,
///     bytes32 indexed polkadexRecipient,
///     uint256 amount,
///     uint64  nonce
/// );
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct DepositEvent {
    /// ERC-20 token address on Ethereum (WETH for the initial deployment).
    pub token: [u8; 20],
    /// Ethereum address that initiated the deposit.
    pub sender: [u8; 20],
    /// Polkadex AccountId encoded as a 32-byte array (raw SS58-decoded public key).
    pub polkadex_recipient: [u8; 32],
    /// Token amount in the token's native Ethereum decimals (18 for WETH).
    pub amount: u128,
    /// Monotonically increasing deposit nonce from the bridge contract.
    pub nonce: u64,
}
