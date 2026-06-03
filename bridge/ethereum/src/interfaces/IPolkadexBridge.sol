// SPDX-License-Identifier: GPL-3.0-or-later
pragma solidity ^0.8.22;

/// @title IPolkadexBridge
/// @notice Interface for the Polkadex ↔ Ethereum token bridge.
interface IPolkadexBridge {
    // ── Outbound: Ethereum → Polkadex ──────────────────────────────────────

    /// @notice Emitted when tokens are locked/burned for bridging to Polkadex.
    /// @param token             ERC-20 address on Ethereum.
    /// @param sender            Ethereum address of the depositor.
    /// @param polkadexRecipient 32-byte Polkadex AccountId (SS58-decoded public key).
    /// @param amount            Token amount in the token's native decimals.
    /// @param nonce             Monotonically increasing deposit identifier.
    event Deposit(
        address indexed token,
        address indexed sender,
        bytes32 indexed polkadexRecipient,
        uint256 amount,
        uint64 nonce
    );

    // ── Inbound: Polkadex → Ethereum ───────────────────────────────────────

    /// @notice Emitted when tokens are released/minted after a valid Polkadex withdrawal proof.
    /// @param nonce     Withdrawal nonce from the Polkadex bridge pallet.
    /// @param token     ERC-20 address on Ethereum.
    /// @param recipient Ethereum address that received the tokens.
    /// @param amount    Token amount released.
    event Withdrawal(
        uint64 indexed nonce,
        address indexed token,
        address indexed recipient,
        uint256 amount
    );

    /// @notice Message emitted by the Polkadex bridge pallet for each outgoing withdrawal.
    ///         The Merkle leaf is keccak256(abi.encodePacked("\x00", nonce, assetId, amount, recipient, polkadexSender)).
    struct WithdrawMessage {
        uint64  nonce;           // unique, monotonic; prevents replay
        uint32  assetId;         // Polkadex asset identifier (0 = native PDEX)
        uint256 amount;          // amount in the token's native decimals (18 for wPDEX)
        address recipient;       // Ethereum destination address
        bytes32 polkadexSender;  // 32-byte Polkadex AccountId of the initiator
    }

    /// @notice Lock (or burn) ERC-20 tokens and emit a deposit for the Polkadex relayer.
    /// @param token             ERC-20 to bridge. Must be whitelisted in TokenRegistry.
    /// @param amount            Amount to bridge.
    /// @param polkadexRecipient Polkadex AccountId that receives the tokens.
    function deposit(
        address token,
        uint256 amount,
        bytes32 polkadexRecipient
    ) external payable;

    /// @notice Claim tokens withdrawn from Polkadex by providing an inclusion proof.
    ///         The BEEFY relayer must have already committed the batch root via BeefyLightClient.
    /// @param message    The withdrawal details from the Polkadex bridge pallet.
    /// @param proof      Binary Merkle proof siblings.
    /// @param leafIndex  0-based index of this message in the batch.
    /// @param leafCount  Total leaves in the batch.
    function withdraw(
        WithdrawMessage calldata message,
        bytes32[] calldata proof,
        uint256 leafIndex,
        uint256 leafCount
    ) external;
}
