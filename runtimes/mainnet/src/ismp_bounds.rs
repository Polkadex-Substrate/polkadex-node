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

//! Caps the GRANDPA vote ancestries an unsigned consensus update may carry. `validate_commit`
//! walks `votes_ancestries` once per precommit before any signature check, so an uncapped list
//! turns a bounded-size proof into O(precommits x ancestries) work. The cap keeps that walk
//! negligible next to the per-signature cost the weight already charges. `unknown_headers` is left
//! to `MaxEncodedSize`: it costs one hash pass and two single walks, which the by-byte weight
//! over-charges. Live Paseo and Polkadot justifications carried zero ancestries.

use crate::Runtime;
use parity_scale_codec::Decode;
use grandpa_verifier_primitives::{justification::GrandpaJustification, FinalityProof};
use ismp::{
	consensus::{
		ConsensusClient, ConsensusClientId, ConsensusStateId, StateMachineClient,
		VerifiedCommitments,
	},
	error::Error,
	host::{IsmpHost, StateMachine},
};
use ismp_grandpa::{
	consensus::GrandpaConsensusClient,
	messages::{ConsensusMessage, SubstrateHeader},
};
use sp_std::{boxed::Box, vec::Vec};

/// The most vote ancestries one GRANDPA justification may carry; 16 leaves margin over the zero
/// seen on live Paseo and Polkadot while keeping the per-precommit ancestry walk negligible.
pub const MAX_VOTE_ANCESTRIES: usize = 16;

/// Whether a SCALE-encoded justification's `votes_ancestries` are within the cap (unreadable ones
/// pass here and fail later in verification).
fn justification_within_cap(justification: &[u8]) -> bool {
	GrandpaJustification::<SubstrateHeader>::decode(&mut &justification[..])
		.map_or(true, |j| j.votes_ancestries.len() <= MAX_VOTE_ANCESTRIES)
}

/// Whether a GRANDPA consensus proof's justification is within the ancestry cap.
pub fn consensus_within_cap(proof: &[u8]) -> bool {
	match ConsensusMessage::decode(&mut &proof[..]) {
		Ok(ConsensusMessage::StandaloneChain(m)) => justification_within_cap(&m.finality_proof.justification),
		Ok(ConsensusMessage::Polkadot(m)) | Ok(ConsensusMessage::Relaychain(m)) =>
			justification_within_cap(&m.finality_proof.justification),
		Err(_) => true,
	}
}

/// Whether a GRANDPA fraud-proof half's justification is within the ancestry cap.
pub fn fraud_within_cap(proof: &[u8]) -> bool {
	FinalityProof::<SubstrateHeader>::decode(&mut &proof[..])
		.map_or(true, |p| justification_within_cap(&p.justification))
}

const OVER_CAP: &str = "grandpa vote ancestries over the cap";

/// `GrandpaConsensusClient` that refuses a justification whose `votes_ancestries` exceed the cap.
#[derive(Default)]
pub struct BoundedGrandpa(GrandpaConsensusClient<Runtime>);

impl ConsensusClient for BoundedGrandpa {
	fn verify_consensus(
		&self,
		host: &dyn IsmpHost,
		consensus_state_id: ConsensusStateId,
		trusted_consensus_state: Vec<u8>,
		proof: Vec<u8>,
	) -> Result<(Vec<u8>, VerifiedCommitments), Error> {
		// Capped before the inner client walks the ancestry in `validate_commit`.
		if !consensus_within_cap(&proof) {
			return Err(Error::Custom(OVER_CAP.into()));
		}
		self.0.verify_consensus(host, consensus_state_id, trusted_consensus_state, proof)
	}

	fn verify_fraud_proof(
		&self,
		host: &dyn IsmpHost,
		trusted_consensus_state: Vec<u8>,
		proof_1: Vec<u8>,
		proof_2: Vec<u8>,
	) -> Result<(), Error> {
		// Both halves of a fraud proof carry a justification; cap each.
		if !fraud_within_cap(&proof_1) || !fraud_within_cap(&proof_2) {
			return Err(Error::Custom(OVER_CAP.into()));
		}
		self.0.verify_fraud_proof(host, trusted_consensus_state, proof_1, proof_2)
	}

	fn consensus_client_id(&self) -> ConsensusClientId {
		self.0.consensus_client_id()
	}

	fn state_machine(&self, id: StateMachine) -> Result<Box<dyn StateMachineClient>, Error> {
		self.0.state_machine(id)
	}
}
