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
use ismp::{
	messaging::{ConsensusMessage, Message, Proof, RequestMessage},
	router::PostRequest,
};
use sp_runtime::{
	traits::ValidateUnsigned,
	transaction_validity::{InvalidTransaction, TransactionValidityError},
};

const EXHAUSTED: TransactionValidityError =
	TransactionValidityError::Invalid(InvalidTransaction::ExhaustsResources);
const BAD_PROOF: TransactionValidityError =
	TransactionValidityError::Invalid(InvalidTransaction::BadProof);

fn new_test_ext() -> sp_io::TestExternalities {
	frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap().into()
}

fn max_encoded_size() -> usize {
	<<Runtime as pallet_ismp::Config>::MaxEncodedSize as Get<u32>>::get() as usize
}

fn consensus(bytes: usize) -> Message {
	let consensus_proof = vec![0; bytes];
	Message::Consensus(ConsensusMessage { consensus_proof, consensus_state_id: *b"ksma", signer: vec![] })
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
	let id = StateMachineId { state_id: StateMachine::Kusama(4009), consensus_state_id: *b"ksma" };
	let proof = Proof { height: StateMachineHeight { id, height: 1 }, proof: vec![] };
	Message::Request(RequestMessage { requests: (0..n).map(post).collect(), proof, signer: vec![] })
}

/// What the pool (`validate_unsigned`) and block import (`pre_dispatch`) say about a call.
fn checks(messages: Vec<Message>) -> (Result<(), TransactionValidityError>, Result<(), TransactionValidityError>) {
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
