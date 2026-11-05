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

//! Unsigned ISMP calls are bounded and weighed by their messages; HFT refunds survive a contract
//! change and refuse amounts the destination cannot represent.

use super::*;
use frame_support::{assert_ok, dispatch::GetDispatchInfo};
use sp_core::U256;
use ismp::{
	messaging::{hash_request, ConsensusMessage, Message, Proof, RequestMessage},
	router::PostRequest,
};
use frame_support::{
	assert_noop,
	traits::{fungibles, Currency},
};
use pallet_hyper_fungible_token::types::{ChainConfig, SendParams, TokenRegistration, TokenUpdate};
use sp_runtime::{
	traits::{Dispatchable, ValidateUnsigned},
	transaction_validity::{InvalidTransaction, TransactionValidityError},
	DispatchResult,
};

const EXHAUSTED: TransactionValidityError =
	TransactionValidityError::Invalid(InvalidTransaction::ExhaustsResources);
const BAD_PROOF: TransactionValidityError =
	TransactionValidityError::Invalid(InvalidTransaction::BadProof);

const PDEX: Balance = crate::constants::currency::PDEX;
const EVM: StateMachine = StateMachine::Evm(1);

fn new_test_ext() -> sp_io::TestExternalities {
	frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap().into()
}

fn max_encoded_size() -> usize {
	<<Runtime as pallet_ismp::Config>::MaxEncodedSize as Get<u32>>::get() as usize
}

fn consensus(bytes: usize) -> Message {
	let consensus_proof = vec![0; bytes];
	let consensus_state_id = *b"ksma";
	Message::Consensus(ConsensusMessage { consensus_proof, consensus_state_id, signer: vec![] })
}

fn post(nonce: u64) -> PostRequest {
	PostRequest {
		source: StateMachine::Evm(1),
		dest: StateMachine::Substrate(*b"PDEX"),
		nonce,
		from: vec![1; 20],
		to: b"pall_hft".to_vec(),
		timeout_timestamp: 0,
		body: vec![],
	}
}

fn requests(n: u64) -> Message {
	requests_with_proof(n, 0)
}

fn requests_with_proof(n: u64, proof_bytes: usize) -> Message {
	let id = StateMachineId { state_id: StateMachine::Kusama(4009), consensus_state_id: *b"ksma" };
	let proof = Proof { height: StateMachineHeight { id, height: 1 }, proof: vec![0; proof_bytes] };
	Message::Request(RequestMessage { requests: (0..n).map(post).collect(), proof, signer: vec![] })
}

/// The declared weight of an unsigned call.
fn declared(messages: Vec<Message>) -> Weight {
	pallet_ismp::Call::<Runtime>::handle_unsigned { messages }.get_dispatch_info().call_weight
}

type Check = Result<(), TransactionValidityError>;

/// What the pool (`validate_unsigned`) and block import (`pre_dispatch`) say about a call.
fn checks(messages: Vec<Message>) -> (Check, Check) {
	let call = pallet_ismp::Call::<Runtime>::handle_unsigned { messages };
	let validate = Ismp::validate_unsigned(TransactionSource::External, &call).map(|_| ());
	(validate, Ismp::pre_dispatch(&call))
}

// Over a bound the pool and block import refuse the call before any message runs; within the
// bounds the messages run, and these fail their proofs instead.
#[test]
fn unsigned_ismp_calls_over_a_bound_are_refused() {
	new_test_ext().execute_with(|| {
		let max = max_encoded_size();
		let over = [vec![consensus(1); 9], vec![requests(65)], vec![requests(33), requests(32)]];
		for messages in over.into_iter().chain([vec![consensus(max)]]) {
			assert_eq!(checks(messages), (Err(EXHAUSTED), Err(EXHAUSTED)));
		}
		for messages in [vec![consensus(1); 8], vec![requests(64)], vec![consensus(max - 16)]] {
			assert_eq!(checks(messages), (Err(BAD_PROOF), Ok(())));
		}
	});
}

// The soak's consensus updates (6.6 to 23.7 KiB) and transfer batches fit four times over, and so
// does a Polkadot finality proof: about 400 precommits of 136 bytes plus ancestry.
#[test]
fn rehearsal_sized_calls_fit_the_bounds_with_margin() {
	new_test_ext().execute_with(|| {
		let soak = vec![consensus(24 * 1024), requests(1), consensus(7 * 1024)];
		assert_eq!(Ismp::check_bounds(&soak), Ok(()));
		assert!(soak.encoded_size() * 4 < max_encoded_size());
		let polkadot = vec![consensus(400 * 136 + 8 * 1024)];
		assert!(polkadot.encoded_size() * 4 < max_encoded_size());
	});
}

// A consensus proof is weighed at one signature per 136 bytes; a batch per request and proof byte,
// plus the runtime call a transfer carries into the HFT pallet. The heaviest calls the bounds allow
// still fit in one block.
#[test]
fn unsigned_ismp_weight_follows_the_messages() {
	type Weights = <Runtime as pallet_ismp::Config>::MessageWeightInfo;
	use pallet_ismp::weights::MessageWeightInfo;
	new_test_ext().execute_with(|| {
		let bytes = 24 * 1024;
		let signatures = bytes as u32 / 136 + 1;
		assert_eq!(declared(vec![consensus(bytes)]), Weights::handle_consensus(signatures));
		let small = declared(vec![consensus(1)]).ref_time();
		assert!(small < declared(vec![consensus(bytes)]).ref_time());
		// Only the proof's growth over an empty one is charged on top of the requests.
		let proof = Weights::handle_proof(100).saturating_sub(Weights::handle_proof(0));
		assert!(proof.any_gt(Weight::zero()));
		let expected = Weights::handle_requests(2).saturating_add(proof);
		assert_eq!(declared(vec![requests_with_proof(2, 100)]), expected);

		let remark = RuntimeCall::System(frame_system::Call::remark { remark: vec![0; 64] });
		let data = pallet_hyper_fungible_token::types::SubstrateCalldata {
			signature: None,
			runtime_call: remark.encode(),
		};
		let amount = U256::from(1u64);
		let body = crate::ismp_fixtures::transfer_body(&[1; 20], &[2; 32], amount, &data.encode());
		let Message::Request(mut carried) = requests(2) else { unreachable!() };
		carried.requests[0].body = body;
		let call_weight = remark.get_dispatch_info().call_weight;
		assert!(call_weight.any_gt(Weight::zero()));
		let expected = Weights::handle_requests(2).saturating_add(call_weight);
		assert_eq!(declared(vec![Message::Request(carried)]), expected);

		let normal = RuntimeBlockWeights::get().get(DispatchClass::Normal).max_extrinsic.unwrap();
		let max = max_encoded_size();
		assert!(declared(vec![consensus(max - 16)]).all_lte(normal));
		assert!(declared(vec![requests_with_proof(64, max - 64 * 128)]).all_lte(normal));
	});
}

// The benchmarks' two scenarios succeed here as they must there: a GRANDPA update finalised by five
// authorities, and a batch of three transfers in with unrelated proof nodes on top.
#[test]
fn ismp_benchmark_scenarios_succeed() {
	use crate::ismp_fixtures::*;
	use sp_core::{ed25519, Pair};
	new_test_ext().execute_with(|| {
		frame_system::Pallet::<Runtime>::set_block_number(1);
		let pair = |i: u8| ed25519::Pair::from_seed(&[i; 32]);
		let pairs: Vec<ed25519::Pair> = (1..=5u8).map(pair).collect();
		let keys: Vec<ed25519::Public> = pairs.iter().map(|pair| pair.public()).collect();
		assert_ok!(Ismp::create_consensus_client(RuntimeOrigin::root(), grandpa_client(&keys)));
		let sign = |key: &ed25519::Public, payload: &[u8]| {
			pairs.iter().find(|pair| pair.public() == *key).expect("one of the five").sign(payload)
		};
		assert_ok!(Ismp::handle_unsigned(RuntimeOrigin::none(), vec![grandpa_update(&keys, sign)]));
		let state = pallet_ismp::ConsensusStates::<Runtime>::get(STATE_ID).expect("stored");
		let state = grandpa_verifier_primitives::ConsensusState::decode(&mut &state[..]).unwrap();
		assert_eq!(state.latest_height, 1);

		let (message, commitment) = transfers_in(3, 2048, None);
		setup_transfers_in(3, commitment);
		assert_ok!(Ismp::handle_unsigned(RuntimeOrigin::none(), vec![message]));
		for i in 0..3 {
			assert_eq!(Balances::free_balance(AccountId::from(beneficiary(i))), TRANSFER);
		}
	});
}

// The benchmark's signed batch: each transfer carries an ECDSA-signed remark from its own
// recipient at nonce 0, and each delivery verifies it, dispatches it and bumps that nonce. Any
// mismatch with on_accept fails the batch here, before a benchmark run.
#[test]
fn a_signed_transfer_batch_is_delivered() {
	use crate::ismp_fixtures::*;
	use sp_core::{ecdsa, Pair};
	new_test_ext().execute_with(|| {
		frame_system::Pallet::<Runtime>::set_block_number(1);
		let pairs: Vec<ecdsa::Pair> = (1..=3u8).map(|i| ecdsa::Pair::from_seed(&[i; 32])).collect();
		let sign = |i: u32, msg: &[u8; 32]| pairs[i as usize].sign_prehashed(msg);
		let (message, commitment) = transfers_in(3, 2048, Some(&sign));
		setup_transfers_in(3, commitment);
		assert_ok!(Ismp::handle_unsigned(RuntimeOrigin::none(), vec![message]));
		// The escrow held four transfers' worth; three went out.
		assert_eq!(Balances::free_balance(HyperFungibleToken::pallet_account()), TRANSFER);
	});
}

fn account(n: u8) -> AccountId {
	AccountId::from([n; 32])
}

fn register(asset: u128, contract: [u8; 20], decimals: u8) {
	let chain = ChainConfig { token_contract: contract.into(), decimals };
	let native = asset == POLKADEX_NATIVE_ASSET_ID;
	let registration = TokenRegistration { local_id: asset, native, chains: [(EVM, chain)].into() };
	assert_ok!(HyperFungibleToken::register_token(RuntimeOrigin::root(), registration));
}

/// Sends of `asset` cost nothing and pay `collector`, the tests' sender, so the amounts below are
/// the transfers alone.
fn free_bridge(asset: u128, collector: &AccountId) {
	assert_ok!(IsmpParams::set_bridge_fee(RuntimeOrigin::root(), asset, EVM, Some(0)));
	assert_ok!(IsmpParams::set_fee_collector(RuntimeOrigin::root(), collector.clone()));
}

/// `send` through the runtime dispatch, which rolls a failed send back as a block would.
fn send(who: &AccountId, asset: u128, amount: Balance) -> DispatchResult {
	let recipient = vec![7u8; 20].try_into().unwrap();
	let params = SendParams {
		asset_id: asset,
		destination: EVM,
		recipient,
		amount,
		timeout: 0,
		relayer_fee: 0,
		call_data: None,
	};
	let call = RuntimeCall::HyperFungibleToken(pallet_hyper_fungible_token::Call::send { params });
	call.dispatch(RuntimeOrigin::signed(who.clone())).map(|_| ()).map_err(|e| e.error)
}

/// The requests sent so far, read back from the offchain index as a relayer would.
fn sent(ext: &mut sp_io::TestExternalities) -> Vec<Request> {
	use pallet_ismp::offchain::{leaf_default_key, Leaf};
	let commitments: Vec<H256> = ext.execute_with(|| {
		let events = frame_system::Pallet::<Runtime>::events().into_iter();
		events
			.filter_map(|record| match record.event {
				RuntimeEvent::HyperFungibleToken(
					pallet_hyper_fungible_token::Event::TokenSent { commitment, .. },
				) => Some(commitment),
				_ => None,
			})
			.collect()
	});
	ext.persist_offchain_overlay();
	let db = ext.offchain_db();
	let read = |commitment: &H256| {
		let leaf = db.get(&leaf_default_key(*commitment)).expect("an indexed request");
		match Leaf::decode(&mut &leaf[..]).expect("a leaf") {
			Leaf::Request(request) => request,
			Leaf::GetResponse(_) => panic!("a request leaf"),
		}
	};
	commitments.iter().map(read).collect()
}

// First regression case from the #29 review: a refund follows the terms recorded at send, so after the contract
// changes a transfer still in flight refunds in full, and the old contract can no longer deliver.
#[test]
fn timeout_refunds_in_full_after_the_contract_changes() {
	let mut ext = new_test_ext();
	let sender = account(81);
	ext.execute_with(|| {
		frame_system::Pallet::<Runtime>::set_block_number(1);
		let _ = <Balances as Currency<AccountId>>::make_free_balance_be(&sender, 1_000 * PDEX);
		register(POLKADEX_NATIVE_ASSET_ID, [0xaa; 20], 18);
		free_bridge(POLKADEX_NATIVE_ASSET_ID, &sender);
		assert_ok!(send(&sender, POLKADEX_NATIVE_ASSET_ID, 100 * PDEX));
		let chain = ChainConfig { token_contract: [0xbb; 20].into(), decimals: 18 };
		let rotate = TokenUpdate {
			asset_id: POLKADEX_NATIVE_ASSET_ID,
			add_chains: [(EVM, chain)].into(),
			remove_chains: vec![],
		};
		assert_ok!(HyperFungibleToken::update_token(RuntimeOrigin::root(), rotate));
		use pallet_hyper_fungible_token::ContractToAsset;
		assert!(ContractToAsset::<Runtime>::get(EVM, [0xaa; 20].to_vec()).is_none());
	});
	let requests = sent(&mut ext);
	ext.execute_with(|| {
		assert_ok!(HyperFungibleToken::default().on_timeout(requests[0].clone()));
		assert_eq!(Balances::free_balance(&sender), 1_000 * PDEX);
	});
}

// Second regression case from the #29 review: with 6 local decimals and 4 on the destination, 101 units is refused before
// anything moves, and 100 is sent and refunded exactly.
#[test]
fn amounts_the_destination_cannot_represent_are_refused() {
	const USDT: u128 = 1_984;
	let mut ext = new_test_ext();
	let sender = account(82);
	ext.execute_with(|| {
		frame_system::Pallet::<Runtime>::set_block_number(1);
		let _ = <Balances as Currency<AccountId>>::make_free_balance_be(&sender, 10 * PDEX);
		let owner = sp_runtime::MultiAddress::Id(sender.clone());
		assert_ok!(Assets::force_create(RuntimeOrigin::root(), USDT.into(), owner, true, 1));
		let (name, symbol) = (b"Tether USD".to_vec(), b"USDT".to_vec());
		let root = RuntimeOrigin::root();
		assert_ok!(Assets::force_set_metadata(root, USDT.into(), name, symbol, 6, false));
		assert_ok!(<Assets as fungibles::Mutate<AccountId>>::mint_into(USDT, &sender, 1_000));
		register(USDT, [0xcc; 20], 4);
		free_bridge(USDT, &sender);
		let refused = pallet_hyper_fungible_token::Error::<Runtime>::AmountNotRepresentable;
		assert_noop!(send(&sender, USDT, 101), refused);
		assert_ok!(send(&sender, USDT, 100));
		assert_eq!(Assets::balance(USDT, &sender), 900);
	});
	let requests = sent(&mut ext);
	ext.execute_with(|| {
		assert_ok!(HyperFungibleToken::default().on_timeout(requests[0].clone()));
		assert_eq!(Assets::balance(USDT, &sender), 1_000);
	});
}

use crate::ismp_bounds::{consensus_within_cap, MAX_VOTE_ANCESTRIES};
use ismp_grandpa::messages::{ConsensusMessage as GrandpaMessage, RelayChainMessage, SubstrateHeader};

/// A GRANDPA consensus proof whose justification carries `ancestries` vote-ancestry headers.
fn grandpa_proof_with_ancestries(ancestries: usize) -> Vec<u8> {
	use finality_grandpa::Commit;
	use grandpa_verifier_primitives::{justification::GrandpaJustification, FinalityProof};
	use parity_scale_codec::Encode;
	use sp_runtime::traits::Header as _;
	let header = |n: u32| SubstrateHeader::new(n, H256::zero(), H256::zero(), H256::zero(), Default::default());
	let commit = Commit { target_hash: H256::zero(), target_number: 1, precommits: vec![] };
	let justification = GrandpaJustification::<SubstrateHeader> {
		round: 1,
		commit,
		votes_ancestries: (0..ancestries as u32).map(header).collect(),
	};
	let finality_proof =
		FinalityProof { block: H256::zero(), justification: justification.encode(), unknown_headers: vec![] };
	let relay = RelayChainMessage { finality_proof, parachain_headers: Default::default() };
	GrandpaMessage::Polkadot(relay).encode()
}

// One over the cap is refused; the cap itself is accepted. The DoS is the per-precommit ancestry
// walk, so the vote-ancestry count is what must be bounded.
#[test]
fn consensus_over_the_ancestry_cap_is_refused() {
	assert!(!consensus_within_cap(&grandpa_proof_with_ancestries(MAX_VOTE_ANCESTRIES + 1)));
	assert!(consensus_within_cap(&grandpa_proof_with_ancestries(MAX_VOTE_ANCESTRIES)));
}

// A Polkadot-sized proof (about 560 headers of 327 bytes plus a 54 KiB justification) fits the
// 256 KiB encoded bound with room, so the cap does not reject real finality. unknown_headers stay
// bounded only by that size and are priced by byte.
#[test]
fn a_polkadot_sized_consensus_proof_fits_the_bounds() {
	new_test_ext().execute_with(|| {
		let proof = vec![0u8; 560 * 327 + 54 * 1024];
		let message = ConsensusMessage {
			consensus_proof: proof,
			consensus_state_id: *b"ksma",
			signer: vec![],
		};
		assert!(Message::Consensus(message).encoded_size() < max_encoded_size());
		assert_eq!(Ismp::check_bounds(&[consensus(560 * 327 + 54 * 1024)]), Ok(()));
	});
}

// A transfer carries an untrusted runtime call; it is decoded with the extrinsic depth limit, so a
// call nested past the limit is dropped (zero weight) instead of recursing without bound.
#[test]
fn a_call_nested_past_the_depth_limit_is_dropped() {
	use pallet_hyper_fungible_token::types::SubstrateCalldata;
	let body = |depth: u32| {
		let mut call = RuntimeCall::System(frame_system::Call::remark { remark: vec![] });
		for _ in 0..depth {
			call = RuntimeCall::Utility(pallet_utility::Call::batch { calls: vec![call] });
		}
		let data = SubstrateCalldata { signature: None, runtime_call: call.encode() };
		crate::ismp_fixtures::transfer_body(&[1; 20], &[2; 32], U256::from(1u64), &data.encode())
	};
	new_test_ext().execute_with(|| {
		assert_eq!(HyperFungibleToken::runtime_call_weight(&body(400)), Weight::zero());
		assert!(HyperFungibleToken::runtime_call_weight(&body(2)).any_gt(Weight::zero()));
	});
}

/// Creates asset `id` with `decimals` local decimals, owned by `owner`.
fn asset(id: u128, decimals: u8, owner: &AccountId) {
	let owner = sp_runtime::MultiAddress::Id(owner.clone());
	assert_ok!(Assets::force_create(RuntimeOrigin::root(), id.into(), owner, true, 1));
	metadata(id, decimals);
}

/// Sets asset `id`'s local decimals.
fn metadata(id: u128, decimals: u8) {
	let (name, symbol) = (b"Token".to_vec(), b"TKN".to_vec());
	let root = RuntimeOrigin::root();
	assert_ok!(Assets::force_set_metadata(root, id.into(), name, symbol, decimals, false));
}

/// Points `asset` on the EVM chain at `contract` with `decimals` remote decimals.
fn update(asset: u128, contract: [u8; 20], decimals: u8) {
	let chain = ChainConfig { token_contract: contract.into(), decimals };
	let add_chains = [(EVM, chain)].into();
	let update = TokenUpdate { asset_id: asset, add_chains, remove_chains: vec![] };
	assert_ok!(HyperFungibleToken::update_token(RuntimeOrigin::root(), update));
}

/// The refund terms recorded for a sent request.
fn terms(request: &Request) -> Option<(u128, Balance)> {
	pallet_hyper_fungible_token::RefundTerms::<Runtime>::get(hash_request::<Ismp>(request))
}

// A precision update on the same contract changes the route, not a pending refund: sent at 18
// remote decimals and updated to 19, the transfer refunds the 100 PDEX sent, and its terms go.
#[test]
fn a_precision_update_keeps_the_refund_and_clears_its_terms() {
	let mut ext = new_test_ext();
	let sender = account(83);
	ext.execute_with(|| {
		frame_system::Pallet::<Runtime>::set_block_number(1);
		let _ = <Balances as Currency<AccountId>>::make_free_balance_be(&sender, 1_000 * PDEX);
		register(POLKADEX_NATIVE_ASSET_ID, [0xaa; 20], 18);
		free_bridge(POLKADEX_NATIVE_ASSET_ID, &sender);
		assert_ok!(send(&sender, POLKADEX_NATIVE_ASSET_ID, 100 * PDEX));
		update(POLKADEX_NATIVE_ASSET_ID, [0xaa; 20], 19);
	});
	let requests = sent(&mut ext);
	ext.execute_with(|| {
		assert_eq!(terms(&requests[0]), Some((POLKADEX_NATIVE_ASSET_ID, 100 * PDEX)));
		assert_ok!(HyperFungibleToken::default().on_timeout(requests[0].clone()));
		assert_eq!(Balances::free_balance(&sender), 1_000 * PDEX);
		assert_eq!(terms(&requests[0]), None);
	});
}

// A contract reused for another asset leaves a pending refund in the asset that was sent.
#[test]
fn a_reused_contract_refunds_the_asset_that_was_sent() {
	const OTHER: u128 = 1_985;
	let mut ext = new_test_ext();
	let sender = account(84);
	ext.execute_with(|| {
		frame_system::Pallet::<Runtime>::set_block_number(1);
		let _ = <Balances as Currency<AccountId>>::make_free_balance_be(&sender, 1_000 * PDEX);
		register(POLKADEX_NATIVE_ASSET_ID, [0xaa; 20], 18);
		free_bridge(POLKADEX_NATIVE_ASSET_ID, &sender);
		assert_ok!(send(&sender, POLKADEX_NATIVE_ASSET_ID, 100 * PDEX));
		let remove_chains = vec![EVM];
		let drop =
			TokenUpdate { asset_id: POLKADEX_NATIVE_ASSET_ID, add_chains: Default::default(), remove_chains };
		assert_ok!(HyperFungibleToken::update_token(RuntimeOrigin::root(), drop));
		asset(OTHER, 18, &sender);
		register(OTHER, [0xaa; 20], 18);
	});
	let requests = sent(&mut ext);
	ext.execute_with(|| {
		assert_ok!(HyperFungibleToken::default().on_timeout(requests[0].clone()));
		assert_eq!(Balances::free_balance(&sender), 1_000 * PDEX);
		assert_eq!(Assets::balance(OTHER, &sender), 0);
	});
}

// A local decimals change between two sends (6 to 8) leaves each refund exact.
#[test]
fn a_local_decimals_change_keeps_each_refund_exact() {
	const TOKEN: u128 = 1_986;
	let mut ext = new_test_ext();
	let sender = account(85);
	ext.execute_with(|| {
		frame_system::Pallet::<Runtime>::set_block_number(1);
		let _ = <Balances as Currency<AccountId>>::make_free_balance_be(&sender, 10 * PDEX);
		asset(TOKEN, 6, &sender);
		assert_ok!(<Assets as fungibles::Mutate<AccountId>>::mint_into(TOKEN, &sender, 1_000_000));
		register(TOKEN, [0xcc; 20], 6);
		free_bridge(TOKEN, &sender);
		assert_ok!(send(&sender, TOKEN, 100));
		metadata(TOKEN, 8);
		assert_ok!(send(&sender, TOKEN, 10_000));
		assert_eq!(Assets::balance(TOKEN, &sender), 1_000_000 - 10_100);
	});
	let requests = sent(&mut ext);
	ext.execute_with(|| {
		for request in &requests {
			assert_ok!(HyperFungibleToken::default().on_timeout(request.clone()));
		}
		assert_eq!(Assets::balance(TOKEN, &sender), 1_000_000);
	});
}

// A transfer sent before the upgrade has no terms and refunds by the route recorded at
// registration.
#[test]
fn a_transfer_without_terms_refunds_by_the_route() {
	let mut ext = new_test_ext();
	let sender = account(86);
	ext.execute_with(|| {
		frame_system::Pallet::<Runtime>::set_block_number(1);
		let _ = <Balances as Currency<AccountId>>::make_free_balance_be(&sender, 1_000 * PDEX);
		register(POLKADEX_NATIVE_ASSET_ID, [0xaa; 20], 18);
		free_bridge(POLKADEX_NATIVE_ASSET_ID, &sender);
		assert_ok!(send(&sender, POLKADEX_NATIVE_ASSET_ID, 100 * PDEX));
	});
	let requests = sent(&mut ext);
	ext.execute_with(|| {
		let commitment = hash_request::<Ismp>(&requests[0]);
		pallet_hyper_fungible_token::RefundTerms::<Runtime>::remove(commitment);
		assert_ok!(HyperFungibleToken::default().on_timeout(requests[0].clone()));
		assert_eq!(Balances::free_balance(&sender), 1_000 * PDEX);
	});
}

// A send is weighed for its path and its call_data length, an update for the chains it adds and
// the chains it removes.
#[test]
fn hft_calls_are_weighed_by_path_length_and_chains() {
	type Weights = <Runtime as pallet_hyper_fungible_token::Config>::WeightInfo;
	use pallet_hyper_fungible_token::{Call, WeightInfo};
	let declared = |call: Call<Runtime>| call.get_dispatch_info().call_weight;
	let send = |asset_id: u128| {
		let params = SendParams {
			asset_id,
			destination: EVM,
			recipient: vec![7u8; 20].try_into().unwrap(),
			amount: 1,
			timeout: 0,
			relayer_fee: 0,
			call_data: Some(vec![0; 100]),
		};
		Call::send { params }
	};
	assert_eq!(declared(send(POLKADEX_NATIVE_ASSET_ID)), Weights::send_native(100));
	assert_eq!(declared(send(1_987)), Weights::send_asset(100));
	let chain = ChainConfig { token_contract: [0xaa; 20].into(), decimals: 18 };
	let remove_chains = vec![StateMachine::Evm(2), StateMachine::Evm(3)];
	let update = TokenUpdate { asset_id: 1_987, add_chains: [(EVM, chain)].into(), remove_chains };
	assert_eq!(declared(Call::update_token { update }), Weights::update_token(3));
}
