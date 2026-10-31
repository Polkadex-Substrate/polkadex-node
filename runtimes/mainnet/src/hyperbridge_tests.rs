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
	messaging::{ConsensusMessage, Message, Proof, RequestMessage},
	router::PostRequest,
};
use frame_support::traits::Currency;
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
		assert_eq!(declared(vec![requests_with_proof(2, 100)]), Weights::handle_requests(2, 100));

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
		let expected = Weights::handle_requests(2, 0).saturating_add(call_weight);
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

		let (message, commitment) = transfers_in(3, 2048);
		setup_transfers_in(3, commitment);
		assert_ok!(Ismp::handle_unsigned(RuntimeOrigin::none(), vec![message]));
		for i in 0..3 {
			assert_eq!(Balances::free_balance(AccountId::from(beneficiary(i))), TRANSFER);
		}
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

// First regression case from the #29 review: a refund follows the route recorded at registration, so after the contract
// changes a transfer still in flight refunds in full, and the old contract can no longer deliver.
#[test]
fn timeout_refunds_in_full_after_the_contract_changes() {
	let mut ext = new_test_ext();
	let sender = account(81);
	ext.execute_with(|| {
		frame_system::Pallet::<Runtime>::set_block_number(1);
		let _ = <Balances as Currency<AccountId>>::make_free_balance_be(&sender, 1_000 * PDEX);
		register(POLKADEX_NATIVE_ASSET_ID, [0xaa; 20], 18);
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
