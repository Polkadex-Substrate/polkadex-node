// SPDX-License-Identifier: GPL-3.0-or-later
pragma solidity ^0.8.22;

import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {Ownable}         from "@openzeppelin/contracts/access/Ownable.sol";
import {IERC20}          from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20}       from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";

import {IBeefyLightClient} from "./interfaces/IBeefyLightClient.sol";
import {IPolkadexBridge}   from "./interfaces/IPolkadexBridge.sol";
import {IWETH}             from "./interfaces/IWETH.sol";
import {TokenRegistry}     from "./TokenRegistry.sol";
import {MerkleProof}       from "./lib/MerkleProof.sol";

/// @title PolkadexBridge
/// @notice Lock/unlock bridge between Polkadex and Ethereum, starting with WETH.
///
/// ┌──────────────────────────────────────────────────────────────────────────┐
/// │  Ethereum → Polkadex  (deposit)                                          │
/// │                                                                          │
/// │  Option A: depositEth(polkadexRecipient)                                 │
/// │    → Send plain ETH. Bridge wraps it to WETH and locks it.              │
/// │                                                                          │
/// │  Option B: deposit(WETH_ADDRESS, amount, polkadexRecipient)              │
/// │    → Send WETH directly. Bridge locks it (requires prior approval).      │
/// │                                                                          │
/// │  In both cases the bridge emits a Deposit event. The relayer picks it   │
/// │  up and credits the recipient's Polkadex account with bridged WETH.     │
/// │                                                                          │
/// │  Polkadex → Ethereum  (withdraw)                                         │
/// │                                                                          │
/// │  1. User initiates withdrawal on Polkadex bridge pallet.                │
/// │  2. BEEFY validators commit the batch Merkle root via BeefyLightClient. │
/// │  3. Anyone calls withdraw() here with a Merkle inclusion proof.          │
/// │  4. Bridge releases WETH to the recipient.                               │
/// └──────────────────────────────────────────────────────────────────────────┘
contract PolkadexBridge is IPolkadexBridge, ReentrancyGuard, Ownable {
    using SafeERC20 for IERC20;

    // ── State ──────────────────────────────────────────────────────────────

    IBeefyLightClient public immutable LIGHT_CLIENT;
    TokenRegistry     public immutable REGISTRY;
    IWETH             public immutable WETH;

    uint64 public depositNonce;
    mapping(uint64 => bool) public processedWithdrawals;
    bool public paused;

    // ── Errors ─────────────────────────────────────────────────────────────

    error BridgePaused();
    error TokenNotSupported(address token);
    error AssetNotSupported(uint32 assetId);
    error ZeroAmount();
    error AlreadyProcessed(uint64 nonce);
    error NoRootCommitted();
    error InvalidProof();
    error ETHTransferFailed();

    // ── Modifier ───────────────────────────────────────────────────────────

    modifier whenNotPaused() {
        _whenNotPaused();
        _;
    }

    function _whenNotPaused() internal view {
        if (paused) revert BridgePaused();
    }

    // ── Constructor ────────────────────────────────────────────────────────

    /// @param initialOwner Admin address (pause, rescue).
    /// @param _lightClient Deployed BeefyLightClient.
    /// @param _registry    Deployed TokenRegistry.
    /// @param _weth        WETH contract address on this network.
    ///                     Sepolia: 0xfFf9976782d46CC05630D1f6eBAb18b2324d6B14
    constructor(
        address initialOwner,
        address _lightClient,
        address _registry,
        address _weth
    ) Ownable(initialOwner) {
        LIGHT_CLIENT = IBeefyLightClient(_lightClient);
        REGISTRY     = TokenRegistry(_registry);
        WETH         = IWETH(_weth);
    }

    // ── Outbound: Ethereum → Polkadex ─────────────────────────────────────

    /// @notice Bridge plain ETH to Polkadex.
    ///         The bridge wraps your ETH to WETH and locks it here.
    /// @param polkadexRecipient 32-byte Polkadex AccountId (your Polkadex public key).
    function depositEth(bytes32 polkadexRecipient)
        external payable whenNotPaused nonReentrant
    {
        if (msg.value == 0) revert ZeroAmount();
        if (!REGISTRY.isRegistered(address(WETH))) revert TokenNotSupported(address(WETH));

        // Wrap ETH → WETH and hold it in this contract
        WETH.deposit{value: msg.value}();

        uint64 nonce = ++depositNonce;
        emit Deposit(address(WETH), msg.sender, polkadexRecipient, msg.value, nonce);
    }

    /// @inheritdoc IPolkadexBridge
    /// @notice Bridge WETH (or any other whitelisted ERC-20) to Polkadex.
    ///         Requires prior ERC-20 approval for `amount`.
    function deposit(
        address token,
        uint256 amount,
        bytes32 polkadexRecipient
    ) external payable override whenNotPaused nonReentrant {
        if (amount == 0) revert ZeroAmount();
        if (!REGISTRY.isRegistered(token)) revert TokenNotSupported(token);

        // All currently supported tokens are lock/unlock (WETH, future ERC-20s).
        // Mintable tokens (bridge-issued) are not in scope for this deployment.
        IERC20(token).safeTransferFrom(msg.sender, address(this), amount);

        uint64 nonce = ++depositNonce;
        emit Deposit(token, msg.sender, polkadexRecipient, amount, nonce);
    }

    // ── Inbound: Polkadex → Ethereum ──────────────────────────────────────

    /// @inheritdoc IPolkadexBridge
    /// @notice Claim WETH withdrawn from Polkadex.
    ///         The BEEFY relayer must have already committed the batch root via BeefyLightClient.
    ///
    /// @param message   The withdrawal details from the Polkadex bridge pallet.
    /// @param proof     Merkle siblings from the leaf up to the batch root.
    /// @param leafIndex 0-based index of this message in the batch.
    /// @param leafCount Total leaves in the batch.
    function withdraw(
        WithdrawMessage calldata message,
        bytes32[]       calldata proof,
        uint256 leafIndex,
        uint256 leafCount
    ) external override whenNotPaused nonReentrant {
        if (processedWithdrawals[message.nonce]) revert AlreadyProcessed(message.nonce);

        bytes32 root = LIGHT_CLIENT.latestMmrRoot();
        if (root == bytes32(0)) revert NoRootCommitted();

        bytes32 leaf = MerkleProof.hashLeaf(
            message.nonce,
            message.assetId,
            message.amount,
            message.recipient,
            message.polkadexSender
        );
        if (!LIGHT_CLIENT.verifyMerkleLeaf(root, leaf, proof, leafIndex, leafCount)) {
            revert InvalidProof();
        }

        processedWithdrawals[message.nonce] = true;

        address token = REGISTRY.assetToToken(message.assetId);
        if (!REGISTRY.isRegistered(token)) revert AssetNotSupported(message.assetId);

        // Release locked ERC-20 (WETH) to the recipient
        IERC20(token).safeTransfer(message.recipient, message.amount);

        emit Withdrawal(message.nonce, token, message.recipient, message.amount);
    }

    // ── Admin ──────────────────────────────────────────────────────────────

    function setPaused(bool _paused) external onlyOwner {
        paused = _paused;
    }

    /// @notice Emergency rescue. Only callable while paused.
    function rescueTokens(address token, address to, uint256 amount) external onlyOwner {
        require(paused, "PolkadexBridge: must be paused");
        IERC20(token).safeTransfer(to, amount);
    }

    /// @notice Receive ETH only from the WETH contract (during unwrap).
    receive() external payable {
        require(msg.sender == address(WETH), "only WETH");
    }
}
