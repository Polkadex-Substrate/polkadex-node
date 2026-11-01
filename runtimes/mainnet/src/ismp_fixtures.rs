// This file is part of Polkadex.
//
// Copyright (C) 2026 The Polkadex Authors.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

//! ISMP messages built as relayers deliver them, for the runtime tests and the `ismp_messages`
//! benchmarks.

use crate::{
	constants::currency::PDEX, Balance, Balances, HyperFungibleToken, Ismp, PolkadexAssetId,
	Runtime, RuntimeOrigin,
};
use finality_grandpa::{Commit, Message as Vote, Precommit, SignedPrecommit};
use frame_support::{assert_ok, traits::Currency};
use grandpa_verifier_primitives::{
	justification::GrandpaJustification, ConsensusState, FinalityProof,
};
use ismp::{
	consensus::{StateCommitment, StateMachineHeight, StateMachineId},
	host::StateMachine,
	messaging::{
		hash_request, ConsensusMessage, CreateConsensusState, Message, Proof, RequestMessage,
		StateCommitmentHeight,
	},
	router::{PostRequest, Request},
};
use ismp_grandpa::{
	consensus::GRANDPA_CONSENSUS_ID,
	messages::{ConsensusMessage as GrandpaMessage, RelayChainMessage, SubstrateHeader},
};
use pallet_hyper_fungible_token::types::{ChainConfig, TokenRegistration};
use parity_scale_codec::Encode;
use sp_consensus_grandpa::{AuthorityId, AuthoritySignature};
use sp_core::{ed25519, H160, H256, U256};
use sp_runtime::traits::{BlakeTwo256, Header as _};
use sp_std::{vec, vec::Vec};

/// The coprocessor whose proofs carry requests here, and the consensus state that tracks it.
pub const COPROCESSOR: StateMachine = StateMachine::Kusama(4009);
pub const STATE_ID: [u8; 4] = *b"ksma";
/// The token contract on the source chain whose transfers in release PDEX from escrow.
pub const SOURCE: StateMachine = StateMachine::Evm(1);
pub const CONTRACT: [u8; 20] = [0xaa; 20];
/// Each transfer in: 10 PDEX, which has 18 decimals on the EVM side.
pub const TRANSFER: Balance = 10 * PDEX;

/// A 32-byte ABI word holding `n`.
fn word(n: usize) -> Vec<u8> {
	let mut word = vec![0u8; 32];
	word[24..].copy_from_slice(&(n as u64).to_be_bytes());
	word
}

/// ABI `bytes`: the length, then the data padded to a whole word.
fn abi_bytes(data: &[u8]) -> Vec<u8> {
	let mut out = word(data.len());
	out.extend_from_slice(data);
	out.resize(out.len() + (32 - data.len() % 32) % 32, 0);
	out
}

/// The body of an HFT transfer as the EVM token contracts encode it:
/// `abi.encode(Message(bytes from, bytes to, uint256 amount, bytes data))`.
pub fn transfer_body(from: &[u8], to: &[u8], amount: U256, data: &[u8]) -> Vec<u8> {
	let (from, to, data) = (abi_bytes(from), abi_bytes(to), abi_bytes(data));
	let head = 4 * 32; // the struct's four head words; the offsets below count from its start
	let mut body = word(32); // offset of the struct, a dynamic tuple
	body.extend(word(head));
	body.extend(word(head + from.len()));
	body.extend(amount.to_big_endian());
	body.extend(word(head + from.len() + to.len()));
	body.extend(from);
	body.extend(to);
	body.extend(data);
	body
}

/// A consensus client for a relay chain finalised by `authorities`, at block 0.
pub fn grandpa_client(authorities: &[ed25519::Public]) -> CreateConsensusState {
	let state = ConsensusState {
		current_authorities: authorities.iter().map(|key| (AuthorityId::from(*key), 1)).collect(),
		current_set_id: 0,
		latest_height: 0,
		latest_hash: H256::repeat_byte(1),
		slot_duration: 6000,
		state_machine: COPROCESSOR,
	};
	CreateConsensusState {
		consensus_state: state.encode(),
		consensus_client_id: GRANDPA_CONSENSUS_ID,
		consensus_state_id: STATE_ID,
		unbonding_period: 7 * 24 * 60 * 60,
		challenge_periods: [(COPROCESSOR, 0)].into(),
		state_machine_commitments: vec![],
	}
}

/// The update that finalises block 1 above that client, with a precommit from each authority.
pub fn grandpa_update(
	authorities: &[ed25519::Public],
	sign: impl Fn(&ed25519::Public, &[u8]) -> ed25519::Signature,
) -> Message {
	let parent = H256::repeat_byte(1);
	let header = SubstrateHeader::new(1, H256::zero(), H256::zero(), parent, Default::default());
	let precommit = Precommit { target_hash: header.hash(), target_number: 1 };
	let (round, set_id) = (1u64, 0u64);
	let payload = (Vote::Precommit(precommit.clone()), round, set_id).encode();
	let signed = |key: &ed25519::Public| SignedPrecommit {
		precommit: precommit.clone(),
		signature: AuthoritySignature::from(sign(key, &payload)),
		id: AuthorityId::from(*key),
	};
	let precommits = authorities.iter().map(signed).collect();
	let commit = Commit { target_hash: header.hash(), target_number: 1, precommits };
	let votes_ancestries = vec![];
	let justification = GrandpaJustification::<SubstrateHeader> { round, commit, votes_ancestries };
	let justification = justification.encode();
	let block = header.hash();
	let finality_proof = FinalityProof { block, justification, unknown_headers: vec![header] };
	let relay = RelayChainMessage { finality_proof, parachain_headers: Default::default() };
	let consensus_proof = GrandpaMessage::Polkadot(relay).encode();
	let consensus_state_id = STATE_ID;
	Message::Consensus(ConsensusMessage { consensus_proof, consensus_state_id, signer: vec![] })
}

/// The beneficiary of transfer `i`.
pub fn beneficiary(i: u32) -> [u8; 32] {
	let mut account = [0x77; 32];
	account[..4].copy_from_slice(&i.to_le_bytes());
	account
}

/// The keystore key type the benchmark's ECDSA signers use.
pub const ECDSA_KEYS: sp_core::crypto::KeyTypeId = sp_core::crypto::KeyTypeId(*b"isme");

/// Signs a prehashed message with transfer `i`'s own ECDSA key: the keystore in the benchmark, an
/// `ecdsa::Pair` in the native test.
pub type EcdsaSigner<'a> = &'a dyn Fn(u32, &[u8; 32]) -> sp_core::ecdsa::Signature;

/// Transfer `i`'s ECDSA-signed empty `System::remark` and the recipient that authorises it, as
/// `on_accept`'s heaviest path expects: the recipient is the signer's Ethereum address
/// (`EvmToSubstrate` = `()`), signing `keccak(prefix || len || (nonce, call))` at nonce 0. Each
/// transfer has its own signer, since a signed delivery bumps its recipient's nonce.
fn signed_remark(i: u32, sign: EcdsaSigner) -> ([u8; 32], Vec<u8>) {
	use pallet_hyper_fungible_token::types::SubstrateCalldata;
	let call = crate::RuntimeCall::System(frame_system::Call::remark { remark: vec![] }).encode();
	let nonce = <<Runtime as frame_system::Config>::Nonce>::default();
	let payload = (nonce, call.clone()).encode();
	let prefix = alloc::format!("\x19Ethereum Signed Message:\n{}", payload.len());
	let msg = sp_io::hashing::keccak_256(&[prefix.into_bytes(), payload].concat());
	let sig = sign(i, &msg);
	let recovered = sp_io::crypto::secp256k1_ecdsa_recover(&sig.0, &msg).ok().expect("recoverable");
	let mut recipient = [0u8; 32];
	recipient[12..].copy_from_slice(&sp_io::hashing::keccak_256(&recovered)[12..]);
	let signature = Some(sp_runtime::MultiSignature::Ecdsa(sig).encode());
	(recipient, SubstrateCalldata { signature, runtime_call: call }.encode())
}

/// `n` transfers of 10 PDEX into the HFT pallet from `SOURCE`, proved through the coprocessor, with
/// `extra` bytes of unrelated proof nodes on top. With a signer, each transfer carries its own
/// ECDSA-signed `System::remark`, so `on_accept` runs its signature-verify and dispatch path.
/// Returns the message and the state commitment its proof checks against.
pub fn transfers_in(
	n: u32,
	extra: u32,
	sign: Option<EcdsaSigner>,
) -> (Message, StateCommitmentHeight) {
	let amount = U256::from(TRANSFER) * U256::from(1_000_000u64); // 12 local decimals to 18
	let post = |i: u32| {
		let (to, data) = match sign {
			Some(sign) => signed_remark(i, sign),
			None => (beneficiary(i), Vec::new()),
		};
		PostRequest {
			source: SOURCE,
			dest: StateMachine::Substrate(*b"PDEX"),
			nonce: i.into(),
			from: CONTRACT.to_vec(),
			to: pallet_hyper_fungible_token::PALLET_ID.to_bytes(),
			timeout_timestamp: 0,
			body: transfer_body(&[0x11; 20], &to, amount, &data),
		}
	};
	let requests: Vec<PostRequest> = (0..n).map(post).collect();
	let key = |post: &PostRequest| {
		let commitment = hash_request::<Ismp>(&Request::Post(post.clone()));
		pallet_ismp::child_trie::RequestCommitments::<Runtime>::storage_key(commitment)
	};
	let keys: Vec<Vec<u8>> = requests.iter().map(key).collect();
	let (state_root, mut nodes) = trie(&keys);
	// Distinct 5-byte nodes: `into_memory_db` hashes each, so tiny nodes are the worst case per byte.
	let unrelated = |i: u32| [i.to_le_bytes().to_vec(), vec![0]].concat();
	nodes.extend((0..extra / 5).map(unrelated));
	// substrate-state-machine's StateMachineProof { hasher: Blake2, storage_proof }
	let proof = (1u8, nodes).encode();
	let id = StateMachineId { state_id: COPROCESSOR, consensus_state_id: STATE_ID };
	let proof = Proof { height: StateMachineHeight { id, height: 1 }, proof };
	let message = Message::Request(RequestMessage { requests, proof, signer: vec![] });
	let commitment = StateCommitment { timestamp: 1, overlay_root: None, state_root };
	(message, StateCommitmentHeight { commitment, height: 1 })
}

/// A V0 trie holding `keys`, and all its nodes, which prove every key.
fn trie(keys: &[Vec<u8>]) -> (H256, Vec<Vec<u8>>) {
	use sp_trie::{LayoutV0, MemoryDB, TrieDBMutBuilder, TrieMut};
	// default() exists only for std's hasher builder; new(&[0u8]) is the same empty database.
	let mut db = MemoryDB::<BlakeTwo256>::new(&[0u8]);
	let mut root = H256::zero();
	{
		let mut trie =
			TrieDBMutBuilder::<LayoutV0<BlakeTwo256>>::new(&mut db, &mut root).build();
		for key in keys {
			trie.insert(key, &[1]).expect("an in-memory trie takes any key");
		}
	}
	let live = db.drain().into_values().filter(|(_, references)| *references > 0);
	(root, live.map(|(node, _)| node).collect())
}

/// What `n` transfers in need: the coprocessor and a consensus client with `commitment`, the
/// registered contract, and an escrow that holds the transfers.
pub fn setup_transfers_in(n: u32, commitment: StateCommitmentHeight) {
	pallet_ismp_params::Coprocessor::<Runtime>::put(COPROCESSOR);
	ismp_grandpa::SupportedStateMachines::<Runtime>::insert(COPROCESSOR, 6000);
	let mut client = grandpa_client(&[]);
	let id = StateMachineId { state_id: COPROCESSOR, consensus_state_id: STATE_ID };
	client.state_machine_commitments = vec![(id, commitment)];
	assert_ok!(Ismp::create_consensus_client(RuntimeOrigin::root(), client));
	let chain = ChainConfig { token_contract: H160::from(CONTRACT), decimals: 18 };
	let local_id = PolkadexAssetId::get();
	let chains = [(SOURCE, chain)].into();
	let registration = TokenRegistration { local_id, native: true, chains };
	assert_ok!(HyperFungibleToken::register_token(RuntimeOrigin::root(), registration));
	let escrow = HyperFungibleToken::pallet_account();
	let _ = <Balances as Currency<_>>::make_free_balance_be(&escrow, TRANSFER * (n as Balance + 1));
}
