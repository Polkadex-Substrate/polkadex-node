// SPDX-License-Identifier: GPL-3.0-or-later
pragma solidity ^0.8.22;

import {Test} from "forge-std/Test.sol";
import {BeefyLightClient} from "../src/BeefyLightClient.sol";
import {MerkleProof}      from "../src/lib/MerkleProof.sol";

contract BeefyLightClientTest is Test {
    BeefyLightClient lc;

    // 5 validator keys (indices 0–4)
    uint256[] pks;
    address[] validators;

    uint64 constant INITIAL_SET_ID = 1;
    uint64 constant NEXT_SET_ID    = 2;

    function setUp() public {
        for (uint256 i; i < 5; ++i) {
            (address addr, uint256 pk) = makeAddrAndKey(string(abi.encode(i)));
            validators.push(addr);
            pks.push(pk);
        }

        address[] memory nextValidators = new address[](0);
        lc = new BeefyLightClient(
            address(this),
            validators,
            INITIAL_SET_ID,
            nextValidators,
            NEXT_SET_ID
        );
    }

    // ── Helpers ────────────────────────────────────────────────────────────

    function _makeCommitment(bytes32 root, uint64 blockNum)
        internal pure returns (BeefyLightClient.Commitment memory)
    {
        return BeefyLightClient.Commitment({
            messagesRoot:      root,
            blockNumber:       blockNum,
            validatorSetId:    INITIAL_SET_ID,
            nextValidatorsHash: bytes32(0),
            nextValidatorsLen:  0
        });
    }

    /// Sign the commitment with the given key subset (indices).
    function _sign(
        BeefyLightClient.Commitment memory c,
        uint256[] memory signerIndices
    ) internal view returns (bytes[] memory sigs) {
        bytes32 digest = lc.commitmentDigest(c);
        sigs = new bytes[](validators.length);
        for (uint256 k; k < signerIndices.length; ++k) {
            uint256 idx = signerIndices[k];
            (uint8 v, bytes32 r, bytes32 s) = vm.sign(pks[idx], digest);
            sigs[idx] = abi.encodePacked(r, s, v);
        }
    }

    function _allSignerIndices() internal view returns (uint256[] memory indices) {
        indices = new uint256[](validators.length);
        for (uint256 i; i < validators.length; ++i) indices[i] = i;
    }

    // ── Tests ──────────────────────────────────────────────────────────────

    function test_submitCommitment_allSign() public {
        bytes32 root = keccak256("batch-1");
        BeefyLightClient.Commitment memory c = _makeCommitment(root, 100);
        bytes[] memory sigs = _sign(c, _allSignerIndices());

        lc.submitCommitment(BeefyLightClient.SignedCommitment({commitment: c, signatures: sigs}));

        assertEq(lc.latestMmrRoot(),    root);
        assertEq(lc.latestBeefyBlock(), 100);
    }

    function test_submitCommitment_thresholdExact() public {
        // 5 validators → threshold = ⌈10/3⌉ = 4
        bytes32 root = keccak256("batch-2");
        BeefyLightClient.Commitment memory c = _makeCommitment(root, 200);

        uint256[] memory indices = new uint256[](4);
        indices[0] = 0; indices[1] = 1; indices[2] = 2; indices[3] = 3;
        bytes[] memory sigs = _sign(c, indices);

        lc.submitCommitment(BeefyLightClient.SignedCommitment({commitment: c, signatures: sigs}));
        assertEq(lc.latestMmrRoot(), root);
    }

    function test_submitCommitment_belowThreshold_reverts() public {
        BeefyLightClient.Commitment memory c = _makeCommitment(keccak256("batch-3"), 300);

        // Only 3 out of 5 sign → below threshold of 4
        uint256[] memory indices = new uint256[](3);
        indices[0] = 0; indices[1] = 1; indices[2] = 2;
        bytes[] memory sigs = _sign(c, indices);

        vm.expectRevert(abi.encodeWithSelector(BeefyLightClient.ThresholdNotMet.selector, 3, 4));
        lc.submitCommitment(BeefyLightClient.SignedCommitment({commitment: c, signatures: sigs}));
    }

    function test_submitCommitment_stale_reverts() public {
        bytes32 root = keccak256("batch-4");
        BeefyLightClient.Commitment memory c = _makeCommitment(root, 400);
        bytes[] memory sigs = _sign(c, _allSignerIndices());
        lc.submitCommitment(BeefyLightClient.SignedCommitment({commitment: c, signatures: sigs}));

        // Re-submit the same commitment → stale
        vm.expectRevert(abi.encodeWithSelector(BeefyLightClient.StaleCommitment.selector, 400, 400));
        lc.submitCommitment(BeefyLightClient.SignedCommitment({commitment: c, signatures: sigs}));
    }

    function test_submitCommitment_wrongSig_reverts() public {
        BeefyLightClient.Commitment memory c = _makeCommitment(keccak256("batch-5"), 500);
        bytes[] memory sigs = _sign(c, _allSignerIndices());

        // Corrupt the first signature
        sigs[0][0] = ~sigs[0][0];

        vm.expectRevert(abi.encodeWithSelector(BeefyLightClient.InvalidSignature.selector, 0));
        lc.submitCommitment(BeefyLightClient.SignedCommitment({commitment: c, signatures: sigs}));
    }

    function test_validatorSetRotation() public {
        // Queue a new validator set
        (address newAddr, uint256 newPk) = makeAddrAndKey("new-validator");
        address[] memory newSet = new address[](1);
        newSet[0] = newAddr;
        lc.queueNextValidatorSet(NEXT_SET_ID, newSet);

        // Submit a commitment signed by the current (old) set referencing the new set id
        // First: current set commits a regular block
        BeefyLightClient.Commitment memory c = _makeCommitment(keccak256("batch-6"), 600);
        bytes[] memory sigs = _sign(c, _allSignerIndices());
        lc.submitCommitment(BeefyLightClient.SignedCommitment({commitment: c, signatures: sigs}));

        // Now create a commitment signed by the *next* validator set
        BeefyLightClient.Commitment memory nextC = BeefyLightClient.Commitment({
            messagesRoot:       keccak256("batch-7"),
            blockNumber:        700,
            validatorSetId:     NEXT_SET_ID,
            nextValidatorsHash: bytes32(0),
            nextValidatorsLen:  0
        });
        bytes32 digest = lc.commitmentDigest(nextC);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(newPk, digest);
        bytes[] memory nextSigs = new bytes[](1);
        nextSigs[0] = abi.encodePacked(r, s, v);

        lc.submitCommitment(BeefyLightClient.SignedCommitment({commitment: nextC, signatures: nextSigs}));

        assertEq(lc.currentValidatorSetId(), NEXT_SET_ID);
        assertEq(lc.latestBeefyBlock(), 700);
    }

    // ── Merkle verification ────────────────────────────────────────────────

    function test_verifyMerkleLeaf_singleLeaf() public view {
        // forge-lint: disable-next-line(unsafe-typecast)
        bytes32 leaf = MerkleProof.hashLeaf(1, 0, 1e12, address(0xBEEF), bytes32("sender"));
        bytes32[] memory proof = new bytes32[](0);

        bool ok = lc.verifyMerkleLeaf(leaf, leaf, proof, 0, 1);
        assertTrue(ok);
    }

    function test_verifyMerkleLeaf_twoLeaves() public view {
        // forge-lint: disable-next-line(unsafe-typecast)
        bytes32 leaf0 = MerkleProof.hashLeaf(1, 0, 1e12, address(0xBEEF), bytes32("s0"));
        // forge-lint: disable-next-line(unsafe-typecast)
        bytes32 leaf1 = MerkleProof.hashLeaf(2, 0, 2e12, address(0xDEAD), bytes32("s1"));

        // root = hash(leaf0, leaf1) using internal node prefix
        bytes32 root = keccak256(abi.encodePacked(bytes1(0x01), leaf0, leaf1));

        bytes32[] memory proof0 = new bytes32[](1);
        proof0[0] = leaf1;

        bytes32[] memory proof1 = new bytes32[](1);
        proof1[0] = leaf0;

        assertTrue(lc.verifyMerkleLeaf(root, leaf0, proof0, 0, 2));
        assertTrue(lc.verifyMerkleLeaf(root, leaf1, proof1, 1, 2));
        assertFalse(lc.verifyMerkleLeaf(root, leaf0, proof1, 0, 2)); // wrong proof
    }
}
