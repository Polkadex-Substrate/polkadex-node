// SPDX-License-Identifier: GPL-3.0-or-later
pragma solidity ^0.8.22;

import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {IBeefyLightClient} from "./interfaces/IBeefyLightClient.sol";
import {MerkleProof} from "./lib/MerkleProof.sol";

/// @title BeefyLightClient
/// @notice Trustless Polkadex finality tracker.
///
/// How it works
/// ────────────
/// 1. Polkadex runs the BEEFY gadget (secp256k1 keys — same curve as Ethereum).
/// 2. After each finalized block, BEEFY validators co-sign a Commitment that
///    contains the Merkle root of all outgoing bridge messages in that batch.
/// 3. A permissionless relayer submits the SignedCommitment here.
/// 4. This contract verifies that ≥ 2/3 of the known validator set signed the
///    Commitment and, if so, updates `latestMmrRoot`.
/// 5. PolkadexBridge then lets users prove their withdrawal is in that root.
///
/// ZK upgrade path
/// ───────────────
/// Replace `_verifySignatures` with a call to a ZK verifier (e.g. SP1 / Gnark)
/// that proves "the validator set signed this commitment" without enumerating
/// every signature on-chain.  The rest of the contract is unchanged.
///
/// Validator set rotation
/// ──────────────────────
/// When Polkadex rotates its validator set the new set's addresses are included
/// in the commitment's `nextValidators` fields.  The owner calls
/// `queueNextValidatorSet` then the next accepted commitment auto-activates it.
contract BeefyLightClient is IBeefyLightClient, Ownable {
    // ── Types ──────────────────────────────────────────────────────────────

    struct ValidatorSet {
        uint64    id;
        address[] validators; // secp256k1 Ethereum addresses derived from BEEFY keys
    }

    /// @notice Data that BEEFY validators sign.
    ///         On the Polkadex side this struct is ABI-encoded and keccak256'd.
    struct Commitment {
        bytes32 messagesRoot;       // Merkle root of outgoing bridge messages
        uint64  blockNumber;        // Polkadex block number
        uint64  validatorSetId;     // ID of the signing validator set
        bytes32 nextValidatorsHash; // keccak256 of next validator addresses (0 = no rotation)
        uint64  nextValidatorsLen;  // length of the next validator set (0 = no rotation)
    }

    struct SignedCommitment {
        Commitment commitment;
        bytes[]   signatures; // one entry per validator slot; empty bytes = absent
    }

    // ── Constants ──────────────────────────────────────────────────────────

    uint256 private constant THRESHOLD_NUMERATOR   = 2;
    uint256 private constant THRESHOLD_DENOMINATOR = 3;
    uint256 public  constant MAX_VALIDATORS        = 256;

    // ── State ──────────────────────────────────────────────────────────────

    ValidatorSet private _current;
    ValidatorSet private _next;

    // forge-lint: disable-next-line(mixed-case-variable)
    bytes32 public override latestMmrRoot;
    uint64  public override latestBeefyBlock;
    uint64  public override currentValidatorSetId;

    // ── Events ─────────────────────────────────────────────────────────────

    event CommitmentSubmitted(bytes32 indexed messagesRoot, uint64 blockNumber, uint64 validatorSetId);
    event ValidatorSetActivated(uint64 indexed id, uint256 length);
    event NextValidatorSetQueued(uint64 indexed id, uint256 length);

    // ── Errors ─────────────────────────────────────────────────────────────

    error StaleCommitment(uint64 submitted, uint64 latest);
    error UnknownValidatorSet(uint64 id);
    error SignaturesLengthMismatch(uint256 got, uint256 expected);
    error InvalidSignature(uint256 validatorIndex);
    error ThresholdNotMet(uint256 signed, uint256 required);
    error TooManyValidators(uint256 count);

    // ── Constructor ────────────────────────────────────────────────────────

    /// @param initialOwner       Admin address (can rotate next validator set).
    /// @param _currentValidators ECDSA addresses of the current BEEFY validator set.
    /// @param currentSetId       Identifier of the current validator set.
    /// @param _nextValidators    Addresses for the queued next set (may equal current).
    /// @param nextSetId          Identifier of the next validator set.
    constructor(
        address   initialOwner,
        address[] memory _currentValidators,
        uint64    currentSetId,
        address[] memory _nextValidators,
        uint64    nextSetId
    ) Ownable(initialOwner) {
        if (_currentValidators.length > MAX_VALIDATORS) revert TooManyValidators(_currentValidators.length);
        if (_nextValidators.length    > MAX_VALIDATORS) revert TooManyValidators(_nextValidators.length);

        _current = ValidatorSet({ id: currentSetId, validators: _currentValidators });
        _next    = ValidatorSet({ id: nextSetId,    validators: _nextValidators });
        currentValidatorSetId = currentSetId;

        emit ValidatorSetActivated(currentSetId, _currentValidators.length);
    }

    // ── Relayer entry point ────────────────────────────────────────────────

    /// @notice Submit a new BEEFY commitment signed by ≥ 2/3 of the validator set.
    ///         Callable by anyone — the signature check is the authorization.
    function submitCommitment(SignedCommitment calldata sc) external {
        Commitment calldata c = sc.commitment;

        if (c.blockNumber <= latestBeefyBlock) {
            revert StaleCommitment(c.blockNumber, latestBeefyBlock);
        }

        // Pick the matching validator set (current or next)
        ValidatorSet storage vset;
        if (c.validatorSetId == _current.id) {
            vset = _current;
        } else if (c.validatorSetId == _next.id) {
            vset = _next;
        } else {
            revert UnknownValidatorSet(c.validatorSetId);
        }

        _verifySignatures(sc, vset);

        latestMmrRoot         = c.messagesRoot;
        latestBeefyBlock      = c.blockNumber;
        currentValidatorSetId = c.validatorSetId;

        // Activate next set when a commitment from that set is first accepted
        if (c.validatorSetId == _next.id && _next.id != _current.id) {
            _current = _next;
            emit ValidatorSetActivated(_current.id, _current.validators.length);
        }

        emit CommitmentSubmitted(c.messagesRoot, c.blockNumber, c.validatorSetId);
    }

    // ── Validator set management (owner-gated; can be made trustless via ZK) ──

    /// @notice Queue a new validator set that will become active once a commitment
    ///         signed by it is accepted.
    function queueNextValidatorSet(
        uint64    id,
        address[] calldata validators
    ) external onlyOwner {
        if (validators.length > MAX_VALIDATORS) revert TooManyValidators(validators.length);
        _next = ValidatorSet({ id: id, validators: validators });
        emit NextValidatorSetQueued(id, validators.length);
    }

    // ── IBeefyLightClient ─────────────────────────────────────────────────

    /// @inheritdoc IBeefyLightClient
    function verifyMerkleLeaf(
        bytes32 root,
        bytes32 leaf,
        bytes32[] calldata proof,
        uint256 leafIndex,
        uint256 leafCount
    ) external pure override returns (bool) {
        return MerkleProof.verify(root, leaf, proof, leafIndex, leafCount);
    }

    // ── View helpers ───────────────────────────────────────────────────────

    function currentValidators() external view returns (address[] memory) {
        return _current.validators;
    }

    function nextValidators() external view returns (uint64 id, address[] memory validators) {
        return (_next.id, _next.validators);
    }

    /// @notice Compute the digest validators sign for a given commitment.
    ///         Exposed so the off-chain relayer and tests can reproduce it exactly.
    function commitmentDigest(Commitment calldata c) public pure returns (bytes32) {
        // forge-lint: disable-next-line(asm-keccak256)
        return keccak256(abi.encode(
            c.messagesRoot,
            c.blockNumber,
            c.validatorSetId,
            c.nextValidatorsHash,
            c.nextValidatorsLen
        ));
    }

    // ── Internal ───────────────────────────────────────────────────────────

    function _verifySignatures(
        SignedCommitment calldata sc,
        ValidatorSet storage vset
    ) internal view {
        uint256 n = vset.validators.length;
        if (sc.signatures.length != n) revert SignaturesLengthMismatch(sc.signatures.length, n);

        bytes32 digest = commitmentDigest(sc.commitment);
        uint256 signed;

        for (uint256 i; i < n; ++i) {
            if (sc.signatures[i].length == 0) continue; // validator did not sign this round

            address recovered = _ecrecover(digest, sc.signatures[i]);
            if (recovered != vset.validators[i]) revert InvalidSignature(i);
            ++signed;
        }

        // Ceiling division: require ⌈2n/3⌉ signatures
        uint256 required = (n * THRESHOLD_NUMERATOR + THRESHOLD_DENOMINATOR - 1) / THRESHOLD_DENOMINATOR;
        if (signed < required) revert ThresholdNotMet(signed, required);
    }

    function _ecrecover(bytes32 digest, bytes calldata sig) internal pure returns (address) {
        if (sig.length != 65) revert InvalidSignature(type(uint256).max);
        bytes32 r = bytes32(sig[0:32]);
        bytes32 s = bytes32(sig[32:64]);
        uint8   v = uint8(sig[64]);
        if (v < 27) v += 27;
        address addr = ecrecover(digest, v, r, s);
        if (addr == address(0)) revert InvalidSignature(type(uint256).max);
        return addr;
    }
}
