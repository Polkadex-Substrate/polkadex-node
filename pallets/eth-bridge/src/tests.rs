// This file is part of Polkadex.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

use crate::{
    mock::*,
    pallet::{
        AuthorizedRelayer, BridgeContractAddress, EthHeaders, Error, OutgoingNonce,
        PendingWithdrawals, ProcessedDeposits, TokenRegistry,
    },
    types::{DepositProof, EthBlockHeader},
};
use frame_support::{assert_noop, assert_ok};
use sp_runtime::DispatchError;

// ── Constants ──────────────────────────────────────────────────────────────

const RELAYER: AccountId = 1;
const USER: AccountId = 2;
#[allow(dead_code)]
const ROOT: AccountId = 0;

/// Polkadex asset ID registered for WETH in tests.
const WETH_PDEX_ASSET_ID: u128 = 100;

/// The `PolkadexBridge` contract address used in tests.
const BRIDGE: [u8; 20] = [0xBE, 0xEF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01];

/// WETH contract address (matches what we put in the Deposit event topics).
const WETH: [u8; 20] = [0xAA, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01];

/// A Polkadex AccountId encoded as 8 little-endian bytes (u64=42) padded to 32 bytes.
/// `USER = 42` → little-endian `[42, 0, 0, 0, 0, 0, 0, 0, 0, 0, ...]`
fn polkadex_recipient_bytes(account: AccountId) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&account.to_le_bytes());
    bytes
}

// ── Helper: build a minimal receipt with one Deposit log ──────────────────

/// Build a fake RLP-encoded receipt containing a single Deposit event.
///
/// We construct the ABI-encoded log manually so we can test the parser
/// without needing a real Ethereum receipt from a node.
fn build_deposit_receipt(
    bridge: [u8; 20],
    token: [u8; 20],
    polkadex_recipient: [u8; 32],
    amount: u128,
    nonce: u64,
) -> Vec<u8> {
    // Compute Deposit event selector
    let selector = sp_io::hashing::keccak_256(b"Deposit(address,address,bytes32,uint256,uint64)");

    // topics[1]: token padded to 32 bytes (right-aligned address)
    let mut topic1 = [0u8; 32];
    topic1[12..].copy_from_slice(&token);

    // topics[2]: sender = a dummy address (right-aligned)
    let mut topic2 = [0u8; 32];
    topic2[12..].copy_from_slice(&[0xDE, 0xAD, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01]);

    // topics[3]: polkadexRecipient
    let topic3 = polkadex_recipient;

    // data: abi.encode(uint256 amount, uint64 nonce) = 64 bytes
    let mut data = [0u8; 64];
    let amount_bytes = amount.to_be_bytes();
    data[16..32].copy_from_slice(&amount_bytes);
    data[56..64].copy_from_slice(&nonce.to_be_bytes());

    // Build the log RLP: [address, [topics...], data]
    let log_rlp = rlp_encode_log(&bridge, &[selector, topic1, topic2, topic3], &data);

    // Build the receipt RLP: [status, cumGas, logsBloom, [log, ...]]
    // status = 0x01 (success), cumGas = 0x00, logsBloom = 256 zero bytes
    build_receipt_rlp(&log_rlp)
}

/// Minimal RLP encoding helpers for test use only.
fn rlp_encode_bytes(data: &[u8]) -> Vec<u8> {
    let len = data.len();
    if len == 1 && data[0] < 0x80 {
        return data.to_vec();
    }
    let mut out = Vec::new();
    if len < 56 {
        out.push(0x80 + len as u8);
    } else {
        let be = (len as u64).to_be_bytes();
        let skip = be.iter().position(|&b| b != 0).unwrap_or(7);
        let sig = &be[skip..];
        out.push(0xb7 + sig.len() as u8);
        out.extend_from_slice(sig);
    }
    out.extend_from_slice(data);
    out
}

fn rlp_encode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let payload: Vec<u8> = items.iter().flat_map(|i| i.clone()).collect();
    let len = payload.len();
    let mut out = Vec::new();
    if len < 56 {
        out.push(0xc0 + len as u8);
    } else {
        let be = (len as u64).to_be_bytes();
        let skip = be.iter().position(|&b| b != 0).unwrap_or(7);
        let sig = &be[skip..];
        out.push(0xf7 + sig.len() as u8);
        out.extend_from_slice(sig);
    }
    out.extend_from_slice(&payload);
    out
}

fn rlp_encode_log(address: &[u8; 20], topics: &[[u8; 32]], data: &[u8]) -> Vec<u8> {
    let addr_enc = rlp_encode_bytes(address);
    let topics_enc: Vec<Vec<u8>> = topics.iter().map(|t| rlp_encode_bytes(t)).collect();
    let topics_list = rlp_encode_list(&topics_enc);
    let data_enc = rlp_encode_bytes(data);
    rlp_encode_list(&[addr_enc, topics_list, data_enc])
}

fn build_receipt_rlp(log_rlp: &[u8]) -> Vec<u8> {
    let status = vec![0x01u8]; // success
    let cum_gas = vec![0x00u8];
    let logs_bloom = vec![0u8; 256];
    let logs_list = rlp_encode_list(&[log_rlp.to_vec()]);

    rlp_encode_list(&[
        rlp_encode_bytes(&status),
        rlp_encode_bytes(&cum_gas),
        rlp_encode_bytes(&logs_bloom),
        logs_list,
    ])
}

/// Build a fake single-leaf MPT proof for `tx_index = 0`.
///
/// For a trie with only one entry (tx_index=0), the entire trie is a single
/// leaf node with:
///   - compact-encoded path = `[0x20]` (leaf, even, empty path — key is fully consumed at root)
///
/// Wait — for a single-item trie the proof IS just that leaf.
/// Key = rlp_encode(0) = [0x80], nibbles = [8, 0].
/// The leaf encodes the remaining path nibbles (all of them) plus the value.
///
/// compact_encode([8, 0], is_leaf=true):
///   even nibble count → prefix = 0x20, then pack nibbles: [8, 0] → byte 0x80
///   encoded = [0x20, 0x80]
fn build_single_tx_mpt_proof(receipt_rlp: &[u8]) -> ([u8; 32], Vec<Vec<u8>>) {
    // Key nibbles for tx_index=0: rlp(0) = [0x80] → nibbles = [8, 0]
    let path_encoded = vec![0x20u8, 0x80]; // compact leaf, even, path=[8,0]
    let leaf_node = rlp_encode_list(&[rlp_encode_bytes(&path_encoded), receipt_rlp.to_vec()]);
    let root: [u8; 32] = sp_io::hashing::keccak_256(&leaf_node);
    (root, vec![leaf_node])
}

// ── Tests: admin calls ──────────────────────────────────────────────────────

#[test]
fn set_authorized_relayer_requires_root() {
    new_test_ext().execute_with(|| {
        assert_noop!(
            EthBridge::set_authorized_relayer(RuntimeOrigin::signed(USER), RELAYER),
            DispatchError::BadOrigin
        );
        assert_ok!(EthBridge::set_authorized_relayer(RuntimeOrigin::root(), RELAYER));
        assert_eq!(AuthorizedRelayer::<Test>::get(), Some(RELAYER));
    });
}

#[test]
fn set_bridge_contract_requires_root() {
    new_test_ext().execute_with(|| {
        assert_noop!(
            EthBridge::set_bridge_contract(RuntimeOrigin::signed(USER), BRIDGE),
            DispatchError::BadOrigin
        );
        assert_ok!(EthBridge::set_bridge_contract(RuntimeOrigin::root(), BRIDGE));
        assert_eq!(BridgeContractAddress::<Test>::get(), Some(BRIDGE));
    });
}

// ── Tests: header submission ────────────────────────────────────────────────

#[test]
fn submit_header_by_relayer_succeeds() {
    new_test_ext().execute_with(|| {
        assert_ok!(EthBridge::set_authorized_relayer(RuntimeOrigin::root(), RELAYER));

        let header = EthBlockHeader {
            block_number: 1000,
            block_hash: [0xAA; 32],
            receipts_root: [0xBB; 32],
            timestamp: 1_700_000_000,
        };
        assert_ok!(EthBridge::submit_eth_header(RuntimeOrigin::signed(RELAYER), header.clone()));
        assert_eq!(EthHeaders::<Test>::get(1000), Some(header));
    });
}

#[test]
fn submit_header_by_non_relayer_fails() {
    new_test_ext().execute_with(|| {
        assert_ok!(EthBridge::set_authorized_relayer(RuntimeOrigin::root(), RELAYER));

        let header = EthBlockHeader {
            block_number: 2000,
            block_hash: [0; 32],
            receipts_root: [0; 32],
            timestamp: 0,
        };
        assert_noop!(
            EthBridge::submit_eth_header(RuntimeOrigin::signed(USER), header),
            Error::<Test>::NotAuthorizedRelayer
        );
    });
}

#[test]
fn submit_header_without_relayer_set_fails() {
    new_test_ext().execute_with(|| {
        let header = EthBlockHeader {
            block_number: 1,
            block_hash: [0; 32],
            receipts_root: [0; 32],
            timestamp: 0,
        };
        assert_noop!(
            EthBridge::submit_eth_header(RuntimeOrigin::signed(RELAYER), header),
            Error::<Test>::NoRelayerSet
        );
    });
}

// ── Tests: deposit proof submission ────────────────────────────────────────

fn setup_bridge() {
    assert_ok!(EthBridge::set_authorized_relayer(RuntimeOrigin::root(), RELAYER));
    assert_ok!(EthBridge::set_bridge_contract(RuntimeOrigin::root(), BRIDGE));
    // Register WETH so deposit proof tests can reach the mint step
    assert_ok!(EthBridge::register_token(RuntimeOrigin::root(), WETH, WETH_PDEX_ASSET_ID, 1, 18));
}

#[test]
fn submit_deposit_proof_success() {
    new_test_ext().execute_with(|| {
        setup_bridge();

        let recipient_bytes = polkadex_recipient_bytes(USER);
        // 2 WETH in Ethereum 18-decimal units
        let eth_amount = 2_000_000_000_000_000_000u128;
        // Expected credited amount in Polkadex 12-decimal units (eth_to_native: 2e18 / 1e6 = 2e12)
        let pdex_amount = 2_000_000_000_000u128;
        let nonce = 1u64;

        let receipt_rlp = build_deposit_receipt(BRIDGE, WETH, recipient_bytes, eth_amount, nonce);
        let (receipts_root, mpt_proof) = build_single_tx_mpt_proof(&receipt_rlp);

        let header = EthBlockHeader {
            block_number: 5000,
            block_hash: [0x11; 32],
            receipts_root,
            timestamp: 1_700_000_000,
        };
        assert_ok!(EthBridge::submit_eth_header(RuntimeOrigin::signed(RELAYER), header));

        let proof = DepositProof {
            block_number: 5000,
            tx_index: 0,
            receipt_rlp,
            mpt_proof,
            log_index: 0,
            deposit_nonce: nonce,
        };
        assert_ok!(EthBridge::submit_deposit_proof(RuntimeOrigin::signed(USER), proof));

        assert!(ProcessedDeposits::<Test>::get(nonce));

        // Mint uses polkadex_asset_id (not eth_token) and amount in Polkadex 12-decimal units
        let mints = minted_tokens();
        assert_eq!(mints.len(), 1);
        assert_eq!(mints[0], (WETH_PDEX_ASSET_ID, USER, pdex_amount));
    });
}

#[test]
fn submit_deposit_proof_replay_fails() {
    new_test_ext().execute_with(|| {
        setup_bridge();

        let recipient_bytes = polkadex_recipient_bytes(USER);
        let receipt_rlp = build_deposit_receipt(BRIDGE, WETH, recipient_bytes, 1_000u128, 1);
        let (receipts_root, mpt_proof) = build_single_tx_mpt_proof(&receipt_rlp);

        let header = EthBlockHeader {
            block_number: 5001,
            block_hash: [0x22; 32],
            receipts_root,
            timestamp: 0,
        };
        assert_ok!(EthBridge::submit_eth_header(RuntimeOrigin::signed(RELAYER), header));

        let proof = DepositProof {
            block_number: 5001,
            tx_index: 0,
            receipt_rlp: receipt_rlp.clone(),
            mpt_proof: mpt_proof.clone(),
            log_index: 0,
            deposit_nonce: 1,
        };
        assert_ok!(EthBridge::submit_deposit_proof(RuntimeOrigin::signed(USER), proof.clone()));

        // Second submission → replay
        assert_noop!(
            EthBridge::submit_deposit_proof(RuntimeOrigin::signed(USER), proof),
            Error::<Test>::DepositAlreadyProcessed
        );
    });
}

#[test]
fn submit_deposit_proof_no_header_fails() {
    new_test_ext().execute_with(|| {
        setup_bridge();

        let receipt_rlp = build_deposit_receipt(BRIDGE, WETH, [0u8; 32], 1_000u128, 42);
        let proof = DepositProof {
            block_number: 9999, // no header for this block
            tx_index: 0,
            receipt_rlp,
            mpt_proof: vec![],
            log_index: 0,
            deposit_nonce: 42,
        };
        assert_noop!(
            EthBridge::submit_deposit_proof(RuntimeOrigin::signed(USER), proof),
            Error::<Test>::HeaderNotFound
        );
    });
}

#[test]
fn submit_deposit_proof_wrong_root_fails() {
    new_test_ext().execute_with(|| {
        setup_bridge();

        let receipt_rlp = build_deposit_receipt(BRIDGE, WETH, [0u8; 32], 1_000u128, 7);
        let (_, mpt_proof) = build_single_tx_mpt_proof(&receipt_rlp);

        // Store a header with a wrong receipts_root
        let header = EthBlockHeader {
            block_number: 6000,
            block_hash: [0x33; 32],
            receipts_root: [0xFFu8; 32], // deliberately wrong
            timestamp: 0,
        };
        assert_ok!(EthBridge::submit_eth_header(RuntimeOrigin::signed(RELAYER), header));

        let proof = DepositProof {
            block_number: 6000,
            tx_index: 0,
            receipt_rlp,
            mpt_proof,
            log_index: 0,
            deposit_nonce: 7,
        };
        assert_noop!(
            EthBridge::submit_deposit_proof(RuntimeOrigin::signed(USER), proof),
            Error::<Test>::InvalidMptProof
        );
    });
}

#[test]
fn submit_deposit_proof_no_bridge_contract_fails() {
    new_test_ext().execute_with(|| {
        // Set relayer but NOT the bridge contract address
        assert_ok!(EthBridge::set_authorized_relayer(RuntimeOrigin::root(), RELAYER));

        let receipt_rlp = build_deposit_receipt(BRIDGE, WETH, [0u8; 32], 1_000u128, 8);
        let (receipts_root, mpt_proof) = build_single_tx_mpt_proof(&receipt_rlp);

        let header = EthBlockHeader {
            block_number: 7000,
            block_hash: [0x44; 32],
            receipts_root,
            timestamp: 0,
        };
        assert_ok!(EthBridge::submit_eth_header(RuntimeOrigin::signed(RELAYER), header));

        let proof = DepositProof {
            block_number: 7000,
            tx_index: 0,
            receipt_rlp,
            mpt_proof,
            log_index: 0,
            deposit_nonce: 8,
        };
        assert_noop!(
            EthBridge::submit_deposit_proof(RuntimeOrigin::signed(USER), proof),
            Error::<Test>::BridgeContractNotSet
        );
    });
}

// ── Tests: token registration ───────────────────────────────────────────────

#[test]
fn register_token_requires_root() {
    new_test_ext().execute_with(|| {
        assert_noop!(
            EthBridge::register_token(RuntimeOrigin::signed(USER), WETH, WETH_PDEX_ASSET_ID, 1, 18),
            DispatchError::BadOrigin
        );
        assert_ok!(EthBridge::register_token(RuntimeOrigin::root(), WETH, WETH_PDEX_ASSET_ID, 1, 18));
        let cfg = TokenRegistry::<Test>::get(WETH).unwrap();
        assert_eq!(cfg.polkadex_asset_id, WETH_PDEX_ASSET_ID);
        assert_eq!(cfg.eth_asset_id, 1);
        assert_eq!(cfg.decimals, 18);
    });
}

// ── Tests: initiate withdrawal ──────────────────────────────────────────────

const ETH_RECIPIENT: [u8; 20] = [0xCA, 0xFE, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01];

fn setup_withdrawal() {
    assert_ok!(EthBridge::register_token(RuntimeOrigin::root(), WETH, WETH_PDEX_ASSET_ID, 1, 18));
    // Give USER a mock balance of 5 WETH in Polkadex 12-decimal units (5 * 1e12)
    set_balance(WETH_PDEX_ASSET_ID, USER, 5_000_000_000_000u128);
}

#[test]
fn initiate_withdrawal_success() {
    new_test_ext().execute_with(|| {
        setup_withdrawal();

        // User passes amount in Polkadex 12-decimal units (1 WETH = 1e12 in Polkadex)
        let pdex_amount = 1_000_000_000_000u128;
        // Withdrawal message stores amount in Ethereum 18-decimal units (native_to_eth: 1e12 * 1e6 = 1e18)
        let eth_amount  = 1_000_000_000_000_000_000u128;

        assert_ok!(EthBridge::initiate_withdrawal(
            RuntimeOrigin::signed(USER),
            WETH,
            pdex_amount,
            ETH_RECIPIENT,
        ));

        assert_eq!(OutgoingNonce::<Test>::get(), 1);

        let msg = PendingWithdrawals::<Test>::get(0).unwrap();
        assert_eq!(msg.nonce, 0);
        assert_eq!(msg.eth_asset_id, 1);
        // WithdrawalMessage stores ETH-decimal amount for the Ethereum contract
        assert_eq!(msg.amount, eth_amount);
        assert_eq!(msg.eth_recipient, ETH_RECIPIENT);

        // BridgeAssets::burn is called with (polkadex_asset_id, user, polkadex_units)
        let burns = burned_tokens();
        assert_eq!(burns.len(), 1);
        assert_eq!(burns[0], (WETH_PDEX_ASSET_ID, USER, pdex_amount));
    });
}

#[test]
fn initiate_withdrawal_unregistered_token_fails() {
    new_test_ext().execute_with(|| {
        let random_token = [0xFFu8; 20];
        // No register_token call → should fail immediately
        assert_noop!(
            EthBridge::initiate_withdrawal(RuntimeOrigin::signed(USER), random_token, 1_000, ETH_RECIPIENT),
            Error::<Test>::TokenNotRegistered
        );
    });
}

#[test]
fn initiate_withdrawal_insufficient_balance_fails() {
    new_test_ext().execute_with(|| {
        assert_ok!(EthBridge::register_token(RuntimeOrigin::root(), WETH, WETH_PDEX_ASSET_ID, 1, 18));
        // No set_balance call → mock burn will fail

        assert_noop!(
            EthBridge::initiate_withdrawal(
                RuntimeOrigin::signed(USER),
                WETH,
                1_000_000_000_000u128, // 1 WETH in Polkadex 12-dec units
                ETH_RECIPIENT,
            ),
            Error::<Test>::InsufficientBalance
        );
    });
}

#[test]
fn initiate_withdrawal_zero_amount_fails() {
    new_test_ext().execute_with(|| {
        setup_withdrawal();
        assert_noop!(
            EthBridge::initiate_withdrawal(RuntimeOrigin::signed(USER), WETH, 0, ETH_RECIPIENT),
            Error::<Test>::ZeroWithdrawalAmount
        );
    });
}

#[test]
fn multiple_withdrawals_get_sequential_nonces() {
    new_test_ext().execute_with(|| {
        setup_withdrawal(); // seeds 5e12 Polkadex units for USER
        let pdex_amount = 500_000_000_000u128; // 0.5 WETH in Polkadex 12-dec units

        assert_ok!(EthBridge::initiate_withdrawal(RuntimeOrigin::signed(USER), WETH, pdex_amount, ETH_RECIPIENT));
        assert_ok!(EthBridge::initiate_withdrawal(RuntimeOrigin::signed(USER), WETH, pdex_amount, ETH_RECIPIENT));

        assert_eq!(OutgoingNonce::<Test>::get(), 2);
        assert_eq!(PendingWithdrawals::<Test>::get(0).unwrap().nonce, 0);
        assert_eq!(PendingWithdrawals::<Test>::get(1).unwrap().nonce, 1);
        // Each message stores ETH 18-dec amount: native_to_eth(5e11) = 5e17
        assert_eq!(PendingWithdrawals::<Test>::get(0).unwrap().amount, 500_000_000_000_000_000u128);
        assert_eq!(burned_tokens().len(), 2);
    });
}

#[test]
fn deposit_then_withdrawal_roundtrip() {
    new_test_ext().execute_with(|| {
        setup_bridge();
        assert_ok!(EthBridge::register_token(RuntimeOrigin::root(), WETH, WETH_PDEX_ASSET_ID, 1, 18));

        // Step 1: Ethereum → Polkadex deposit (2 WETH emitted in 18-decimal ETH units)
        let recipient_bytes = polkadex_recipient_bytes(USER);
        let eth_deposit_amount  = 2_000_000_000_000_000_000u128; // 2 WETH, 18-dec
        let pdex_credited_amount = 2_000_000_000_000u128;         // 2 WETH, 12-dec

        let receipt_rlp = build_deposit_receipt(BRIDGE, WETH, recipient_bytes, eth_deposit_amount, 99);
        let (receipts_root, mpt_proof) = build_single_tx_mpt_proof(&receipt_rlp);

        assert_ok!(EthBridge::submit_eth_header(RuntimeOrigin::signed(RELAYER), EthBlockHeader {
            block_number: 8000, block_hash: [0x55; 32], receipts_root, timestamp: 0,
        }));
        assert_ok!(EthBridge::submit_deposit_proof(
            RuntimeOrigin::signed(USER),
            DepositProof { block_number: 8000, tx_index: 0, receipt_rlp, mpt_proof, log_index: 0, deposit_nonce: 99 }
        ));

        // Mint used polkadex_asset_id and Polkadex 12-dec amount
        assert_eq!(minted_tokens()[0], (WETH_PDEX_ASSET_ID, USER, pdex_credited_amount));

        // Step 2: Polkadex → Ethereum withdrawal
        // User passes Polkadex 12-dec amount; pallet converts to 18-dec for the message
        assert_ok!(EthBridge::initiate_withdrawal(
            RuntimeOrigin::signed(USER),
            WETH,
            pdex_credited_amount,
            ETH_RECIPIENT,
        ));

        let msg = PendingWithdrawals::<Test>::get(0).unwrap();
        assert_eq!(msg.amount, eth_deposit_amount); // 18-dec for the Ethereum contract
        assert_eq!(msg.eth_recipient, ETH_RECIPIENT);
        // Burn used polkadex_asset_id and Polkadex 12-dec amount
        assert_eq!(burned_tokens()[0], (WETH_PDEX_ASSET_ID, USER, pdex_credited_amount));
    });
}

// ── Decimal conversion unit tests ───────────────────────────────────────────

#[test]
fn decimal_conversion_weth_18_decimals() {
    use crate::types::TokenConfig;
    let weth = TokenConfig { polkadex_asset_id: 1, eth_asset_id: 1, decimals: 18 };

    // 1 WETH (18-dec) → 1 WETH (12-dec)
    assert_eq!(weth.eth_to_native(1_000_000_000_000_000_000u128), 1_000_000_000_000u128);
    // 1 WETH (12-dec) → 1 WETH (18-dec)
    assert_eq!(weth.native_to_eth(1_000_000_000_000u128), 1_000_000_000_000_000_000u128);
    // Roundtrip: start from 18-dec, convert both ways
    let eth_amt = 3_500_000_000_000_000_000u128; // 3.5 WETH
    assert_eq!(weth.native_to_eth(weth.eth_to_native(eth_amt)), eth_amt);
}

#[test]
fn decimal_conversion_usdc_6_decimals() {
    use crate::types::TokenConfig;
    let usdc = TokenConfig { polkadex_asset_id: 2, eth_asset_id: 2, decimals: 6 };

    // 1 USDC (6-dec) → 1 USDC (12-dec)
    assert_eq!(usdc.eth_to_native(1_000_000u128), 1_000_000_000_000u128);
    // 1 USDC (12-dec) → 1 USDC (6-dec)
    assert_eq!(usdc.native_to_eth(1_000_000_000_000u128), 1_000_000u128);
}

#[test]
fn decimal_conversion_same_decimals() {
    use crate::types::TokenConfig;
    let token = TokenConfig { polkadex_asset_id: 3, eth_asset_id: 3, decimals: 12 };
    assert_eq!(token.eth_to_native(5_000_000_000_000u128), 5_000_000_000_000u128);
    assert_eq!(token.native_to_eth(5_000_000_000_000u128), 5_000_000_000_000u128);
}
