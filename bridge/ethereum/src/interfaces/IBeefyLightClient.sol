// SPDX-License-Identifier: GPL-3.0-or-later
pragma solidity ^0.8.22;

/// @title IBeefyLightClient
/// @notice Interface for the Polkadex BEEFY light client on Ethereum.
///         The light client tracks Polkadex finality by verifying that 2/3+ of
///         the current BEEFY validator set has signed each new commitment.
///
/// Upgrade path to ZK:
///   Replace the validator-signature verification in BeefyLightClient with a
///   zkSNARK proof (e.g. via SP1 or Gnark) that proves the validator set signed
///   the commitment without revealing individual signatures on-chain.
interface IBeefyLightClient {
    /// @notice Latest Merkle root of outgoing bridge messages committed by validators.
    function latestMmrRoot() external view returns (bytes32);

    /// @notice Polkadex block number corresponding to `latestMMRRoot`.
    function latestBeefyBlock() external view returns (uint64);

    /// @notice Validator set ID used to produce `latestMMRRoot`.
    function currentValidatorSetId() external view returns (uint64);

    /// @notice Verify that `leaf` is included in `root` using a standard binary Merkle proof.
    /// @param root       Merkle root (from `latestMMRRoot`).
    /// @param leaf       Hashed leaf (use MerkleProof.hashLeaf to compute).
    /// @param proof      Sibling hashes from leaf to root.
    /// @param leafIndex  0-based position of the leaf.
    /// @param leafCount  Total number of leaves in the tree.
    function verifyMerkleLeaf(
        bytes32 root,
        bytes32 leaf,
        bytes32[] calldata proof,
        uint256 leafIndex,
        uint256 leafCount
    ) external pure returns (bool);
}
