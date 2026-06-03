// This file is part of Polkadex.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Minimal Ethereum Merkle Patricia Trie (MPT) proof verifier and receipt/log parser.
//!
//! ## Receipt proof flow
//! 1. The receipt trie key for transaction `i` is `rlp_encode(i)`.
//! 2. The proof is a list of RLP-encoded trie nodes from the receipts root to the leaf.
//! 3. Each node is one of:
//!    - **Branch** (17 items): `[v0..v15, value]` — v_k is a 32-byte child hash or empty.
//!    - **Extension** (2 items): `[compact_path, next_hash]` — skips common prefix nibbles.
//!    - **Leaf** (2 items): `[compact_path, value]` — terminates the path.
//! 4. The compact path encoding uses the high nibble as a flag:
//!    `0` = extension even, `1` = extension odd, `2` = leaf even, `3` = leaf odd.
//!
//! ## Assumptions for v1
//! All proof nodes are ≥ 32 bytes (so they are hash-referenced, not inlined).
//! This holds for any non-trivial receipt trie (more than one transaction).

use crate::types::{DepositEvent, EthLog};
use sp_std::prelude::*;

// ── Error types ────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
pub enum RlpError {
    Empty,
    NotList,
    Truncated,
    InvalidLength,
    Overflow,
    InvalidStructure,
}

#[derive(Debug, PartialEq)]
pub enum ParseError {
    Rlp(RlpError),
    WrongContract,
    InvalidTopicCount,
    WrongEventSignature,
    AmountOverflow,
    InvalidData,
}

impl From<RlpError> for ParseError {
    fn from(e: RlpError) -> Self {
        ParseError::Rlp(e)
    }
}

// ── Minimal RLP decoder ────────────────────────────────────────────────────

/// Decode an RLP-encoded list, returning each item's *content* bytes.
/// For string items: the raw bytes (no prefix).
/// For list items: the full RLP bytes including the list prefix.
pub fn rlp_decode_list(data: &[u8]) -> Result<Vec<Vec<u8>>, RlpError> {
    let content = rlp_list_content(data)?;
    rlp_decode_items(content)
}

fn rlp_list_content(data: &[u8]) -> Result<&[u8], RlpError> {
    if data.is_empty() {
        return Err(RlpError::Empty);
    }
    let b = data[0];
    if b < 0xc0 {
        return Err(RlpError::NotList);
    }
    let (start, len) = if b <= 0xf7 {
        (1usize, (b - 0xc0) as usize)
    } else {
        let ll = (b - 0xf7) as usize;
        if data.len() < 1 + ll {
            return Err(RlpError::Truncated);
        }
        let len = decode_be_usize(&data[1..1 + ll])?;
        (1 + ll, len)
    };
    if data.len() < start + len {
        return Err(RlpError::Truncated);
    }
    Ok(&data[start..start + len])
}

fn rlp_decode_items(data: &[u8]) -> Result<Vec<Vec<u8>>, RlpError> {
    let mut items = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        let (item, consumed) = rlp_item_at(&data[offset..])?;
        items.push(item.to_vec());
        offset += consumed;
    }
    Ok(items)
}

/// Returns `(content_or_raw, total_bytes_consumed)`.
/// Strings return their content (payload only, no prefix).
/// Lists return their full raw RLP (payload + prefix) so the caller can recurse.
fn rlp_item_at(data: &[u8]) -> Result<(&[u8], usize), RlpError> {
    if data.is_empty() {
        return Err(RlpError::Empty);
    }
    let b = data[0];
    if b < 0x80 {
        // Single-byte literal
        Ok((&data[0..1], 1))
    } else if b < 0xb8 {
        // Short string
        let len = (b - 0x80) as usize;
        let end = 1 + len;
        if data.len() < end {
            return Err(RlpError::Truncated);
        }
        Ok((&data[1..end], end))
    } else if b < 0xc0 {
        // Long string
        let ll = (b - 0xb7) as usize;
        if data.len() < 1 + ll {
            return Err(RlpError::Truncated);
        }
        let len = decode_be_usize(&data[1..1 + ll])?;
        let end = 1 + ll + len;
        if data.len() < end {
            return Err(RlpError::Truncated);
        }
        Ok((&data[1 + ll..end], end))
    } else {
        // List — return whole raw RLP (including prefix) so caller can recurse
        let total = if b <= 0xf7 {
            1 + (b - 0xc0) as usize
        } else {
            let ll = (b - 0xf7) as usize;
            if data.len() < 1 + ll {
                return Err(RlpError::Truncated);
            }
            let len = decode_be_usize(&data[1..1 + ll])?;
            1 + ll + len
        };
        if data.len() < total {
            return Err(RlpError::Truncated);
        }
        Ok((&data[0..total], total))
    }
}

fn decode_be_usize(bytes: &[u8]) -> Result<usize, RlpError> {
    if bytes.is_empty() || bytes.len() > 8 {
        return Err(RlpError::InvalidLength);
    }
    let mut result = 0usize;
    for &byte in bytes {
        result = result.checked_shl(8).ok_or(RlpError::Overflow)? | byte as usize;
    }
    Ok(result)
}

// ── MPT helpers ────────────────────────────────────────────────────────────

/// RLP-encode a `u64` integer (used to compute the receipt trie key from tx index).
pub fn rlp_encode_uint(n: u64) -> Vec<u8> {
    if n == 0 {
        vec![0x80] // RLP empty string == integer zero
    } else if n < 0x80 {
        vec![n as u8]
    } else {
        let be = n.to_be_bytes();
        let skip = be.iter().position(|&b| b != 0).unwrap_or(7);
        let sig = &be[skip..];
        let mut out = Vec::with_capacity(1 + sig.len());
        out.push(0x80 + sig.len() as u8);
        out.extend_from_slice(sig);
        out
    }
}

/// Convert a byte slice into a nibble (half-byte) slice.
fn bytes_to_nibbles(bytes: &[u8]) -> Vec<u8> {
    let mut nibbles = Vec::with_capacity(bytes.len() * 2);
    for &b in bytes {
        nibbles.push(b >> 4);
        nibbles.push(b & 0x0f);
    }
    nibbles
}

/// Decode a compact-encoded path (HP encoding) used in leaf and extension nodes.
/// Returns `(nibbles, is_leaf)`.
///
/// High nibble of first byte encodes the type:
///   0 = extension, even nibble count
///   1 = extension, odd  nibble count (first nibble is low nibble of first byte)
///   2 = leaf,      even nibble count
///   3 = leaf,      odd  nibble count
fn decode_compact_nibbles(encoded: &[u8]) -> (Vec<u8>, bool) {
    if encoded.is_empty() {
        return (Vec::new(), false);
    }
    let flag = encoded[0] >> 4;
    let is_leaf = flag >= 2;
    let is_odd = flag & 1 == 1;

    let mut nibbles = Vec::new();
    if is_odd {
        nibbles.push(encoded[0] & 0x0f);
    }
    for &byte in &encoded[1..] {
        nibbles.push(byte >> 4);
        nibbles.push(byte & 0x0f);
    }
    (nibbles, is_leaf)
}

// ── MPT proof verifier ─────────────────────────────────────────────────────

/// Verify that `receipt_rlp` is the value stored at key `rlp_encode(tx_index)`
/// in the receipt trie whose root is `receipts_root`.
///
/// `proof_nodes` is a sequence of RLP-encoded trie nodes starting from the root.
pub fn verify_receipt_proof(
    receipts_root: [u8; 32],
    tx_index: u64,
    receipt_rlp: &[u8],
    proof_nodes: &[Vec<u8>],
) -> bool {
    let key = rlp_encode_uint(tx_index);
    let nibbles = bytes_to_nibbles(&key);
    verify_mpt_proof(receipts_root, &nibbles, receipt_rlp, proof_nodes)
}

fn verify_mpt_proof(
    root: [u8; 32],
    key_nibbles: &[u8],
    expected_value: &[u8],
    proof_nodes: &[Vec<u8>],
) -> bool {
    if proof_nodes.is_empty() {
        // Empty trie: only valid if root is the known empty-trie hash
        // 0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421
        let empty_root: [u8; 32] = [
            0x56, 0xe8, 0x1f, 0x17, 0x1b, 0xcc, 0x55, 0xa6, 0xff, 0x83, 0x45, 0xe6, 0x92, 0xc0,
            0xf8, 0x6e, 0x5b, 0x48, 0xe0, 0x1b, 0x99, 0x6c, 0xad, 0xc0, 0x01, 0x62, 0x2f, 0xb5,
            0xe3, 0x63, 0xb4, 0x21,
        ];
        return root == empty_root && expected_value.is_empty();
    }

    let mut expected_hash = root;
    let mut nibble_idx = 0usize;

    for (node_pos, node_rlp) in proof_nodes.iter().enumerate() {
        // Each proof node must hash to the expected hash
        let node_hash = sp_io::hashing::keccak_256(node_rlp);
        if node_hash != expected_hash {
            return false;
        }

        let items = match rlp_decode_list(node_rlp) {
            Ok(v) => v,
            Err(_) => return false,
        };

        let is_last_node = node_pos == proof_nodes.len() - 1;

        match items.len() {
            17 => {
                // Branch node: items[0..15] are child references, items[16] is the value
                if nibble_idx == key_nibbles.len() {
                    // We've consumed all key nibbles — value is at items[16]
                    return is_last_node && items[16] == expected_value;
                }
                let nibble = key_nibbles[nibble_idx] as usize;
                let child = &items[nibble];
                if child.len() == 32 {
                    expected_hash.copy_from_slice(child);
                } else if child.is_empty() {
                    return false; // Path does not exist
                } else {
                    // Inline node (< 32 bytes): only valid as the last node
                    return is_last_node && child == expected_value;
                }
                nibble_idx += 1;
            },
            2 => {
                // Leaf or extension node
                let (path_nibbles, is_leaf) = decode_compact_nibbles(&items[0]);
                let remaining = &key_nibbles[nibble_idx..];

                if is_leaf {
                    // Path nibbles must exactly equal the remaining key nibbles
                    return is_last_node
                        && path_nibbles.as_slice() == remaining
                        && items[1] == expected_value;
                } else {
                    // Extension: path nibbles must be a prefix of remaining key
                    if remaining.len() < path_nibbles.len() {
                        return false;
                    }
                    if &remaining[..path_nibbles.len()] != path_nibbles.as_slice() {
                        return false;
                    }
                    nibble_idx += path_nibbles.len();
                    // items[1] is the hash of the next node (must be 32 bytes)
                    if items[1].len() != 32 {
                        return false;
                    }
                    expected_hash.copy_from_slice(&items[1]);
                }
            },
            _ => return false,
        }
    }

    false
}

// ── Receipt parser ─────────────────────────────────────────────────────────

/// Parse all event logs from a raw Ethereum receipt.
///
/// Handles both legacy receipts and EIP-2718 typed receipts (type 1, 2, 3).
/// The receipt tuple is `[status, cumulative_gas_used, logs_bloom, logs]`.
pub fn parse_receipt_logs(receipt_rlp: &[u8]) -> Result<Vec<EthLog>, RlpError> {
    // Typed receipts start with a single byte < 0x7f (the type byte)
    let rlp_data = if !receipt_rlp.is_empty() && receipt_rlp[0] < 0x80 {
        &receipt_rlp[1..]
    } else {
        receipt_rlp
    };

    let fields = rlp_decode_list(rlp_data)?;
    // Expected: [status, cumGasUsed, logsBloom, logs]
    if fields.len() < 4 {
        return Err(RlpError::InvalidStructure);
    }

    // fields[3] is the raw RLP of the logs list
    let log_entries = rlp_decode_list(&fields[3])?;
    let mut logs = Vec::new();
    for log_rlp in log_entries {
        logs.push(parse_eth_log(&log_rlp)?);
    }
    Ok(logs)
}

fn parse_eth_log(log_rlp: &[u8]) -> Result<EthLog, RlpError> {
    // Each log: [address, topics, data]
    let fields = rlp_decode_list(log_rlp)?;
    if fields.len() != 3 {
        return Err(RlpError::InvalidStructure);
    }

    if fields[0].len() != 20 {
        return Err(RlpError::InvalidStructure);
    }
    let mut address = [0u8; 20];
    address.copy_from_slice(&fields[0]);

    // fields[1] is the raw RLP of the topics list
    let topic_entries = rlp_decode_list(&fields[1])?;
    let mut topics: Vec<[u8; 32]> = Vec::new();
    for topic in topic_entries {
        if topic.len() != 32 {
            return Err(RlpError::InvalidStructure);
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&topic);
        topics.push(arr);
    }

    Ok(EthLog { address, topics, data: fields[2].clone() })
}

// ── Deposit event parser ───────────────────────────────────────────────────

/// Keccak256 of the `Deposit` event ABI signature.
/// Computed at runtime to avoid a compile-time dependency.
///
/// Signature: `Deposit(address,address,bytes32,uint256,uint64)`
fn deposit_event_selector() -> [u8; 32] {
    sp_io::hashing::keccak_256(b"Deposit(address,address,bytes32,uint256,uint64)")
}

/// Extract and decode the `Deposit` event from a parsed log.
///
/// Expected log structure:
/// - `topics[0]` = `keccak256("Deposit(address,address,bytes32,uint256,uint64)")`
/// - `topics[1]` = `token` (20-byte address, right-aligned in 32 bytes)
/// - `topics[2]` = `sender` (20-byte address, right-aligned)
/// - `topics[3]` = `polkadexRecipient` (bytes32 — the Polkadex AccountId)
/// - `data`      = `abi.encode(uint256 amount, uint64 nonce)` = 64 bytes
pub fn parse_deposit_event(
    log: &EthLog,
    bridge_contract: [u8; 20],
) -> Result<DepositEvent, ParseError> {
    if log.address != bridge_contract {
        return Err(ParseError::WrongContract);
    }
    if log.topics.len() != 4 {
        return Err(ParseError::InvalidTopicCount);
    }
    if log.topics[0] != deposit_event_selector() {
        return Err(ParseError::WrongEventSignature);
    }

    // topics[1]: token address — right-aligned (bytes 12..32 are the address)
    let mut token = [0u8; 20];
    token.copy_from_slice(&log.topics[1][12..]);

    // topics[2]: sender address — right-aligned
    let mut sender = [0u8; 20];
    sender.copy_from_slice(&log.topics[2][12..]);

    // topics[3]: polkadexRecipient — full 32 bytes
    let polkadex_recipient = log.topics[3];

    // data: abi.encode(uint256 amount, uint64 nonce) = 64 bytes
    if log.data.len() != 64 {
        return Err(ParseError::InvalidData);
    }

    // amount is uint256 — reject if it overflows u128 (high 16 bytes must be zero)
    for &b in &log.data[0..16] {
        if b != 0 {
            return Err(ParseError::AmountOverflow);
        }
    }
    let mut amount_bytes = [0u8; 16];
    amount_bytes.copy_from_slice(&log.data[16..32]);
    let amount = u128::from_be_bytes(amount_bytes);

    // nonce is uint64 — encoded as uint256 (left-padded), so last 8 bytes hold the value
    let mut nonce_bytes = [0u8; 8];
    nonce_bytes.copy_from_slice(&log.data[56..64]);
    let nonce = u64::from_be_bytes(nonce_bytes);

    Ok(DepositEvent { token, sender, polkadex_recipient, amount, nonce })
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rlp_encode_uint_edge_cases() {
        assert_eq!(rlp_encode_uint(0), vec![0x80]);
        assert_eq!(rlp_encode_uint(1), vec![0x01]);
        assert_eq!(rlp_encode_uint(127), vec![0x7f]);
        assert_eq!(rlp_encode_uint(128), vec![0x81, 0x80]);
        assert_eq!(rlp_encode_uint(256), vec![0x82, 0x01, 0x00]);
    }

    #[test]
    fn nibble_conversion() {
        assert_eq!(bytes_to_nibbles(&[0xab, 0xcd]), vec![0xa, 0xb, 0xc, 0xd]);
        assert_eq!(bytes_to_nibbles(&[0x00]), vec![0x0, 0x0]);
    }

    #[test]
    fn compact_nibble_decode_leaf_even() {
        // Flag 0x20 = leaf, even — path bytes follow directly
        let (nibbles, is_leaf) = decode_compact_nibbles(&[0x20, 0x15, 0x27]);
        assert!(is_leaf);
        assert_eq!(nibbles, vec![1, 5, 2, 7]);
    }

    #[test]
    fn compact_nibble_decode_leaf_odd() {
        // Flag 0x31 = leaf, odd — first nibble is 1
        let (nibbles, is_leaf) = decode_compact_nibbles(&[0x31, 0x23]);
        assert!(is_leaf);
        assert_eq!(nibbles, vec![1, 2, 3]);
    }

    #[test]
    fn compact_nibble_decode_extension_even() {
        let (nibbles, is_leaf) = decode_compact_nibbles(&[0x00, 0xab]);
        assert!(!is_leaf);
        assert_eq!(nibbles, vec![0xa, 0xb]);
    }

    #[test]
    fn rlp_decode_list_simple() {
        // RLP of ["cat", "dog"]: c8 83 63 61 74 83 64 6f 67
        let rlp = [0xc8u8, 0x83, 0x63, 0x61, 0x74, 0x83, 0x64, 0x6f, 0x67];
        let items = rlp_decode_list(&rlp).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], b"cat");
        assert_eq!(items[1], b"dog");
    }
}
