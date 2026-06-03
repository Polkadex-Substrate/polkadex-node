// SPDX-License-Identifier: GPL-3.0-or-later
pragma solidity ^0.8.22;

import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

/// @title TokenRegistry
/// @notice Maintains a whitelist of (Polkadex assetId ↔ Ethereum ERC-20) pairs
///         and whether each pair uses a lock/unlock or mint/burn model.
///
/// Lock/unlock — token originates on Ethereum (e.g. USDC, WETH):
///   Deposit  → bridge holds the ERC-20.
///   Withdraw → bridge releases the ERC-20.
///
/// Mint/burn  — token originates on Polkadex (e.g. PDEX → wPDEX):
///   Deposit  → bridge burns the bridge-issued ERC-20 (user is returning it).
///   Withdraw → bridge mints the bridge-issued ERC-20 to the recipient.
contract TokenRegistry is Ownable {
    // ── Storage ────────────────────────────────────────────────────────────

    /// Polkadex assetId → Ethereum token address
    mapping(uint32 => address) public assetToToken;

    /// Ethereum token address → Polkadex assetId
    mapping(address => uint32) public tokenToAsset;

    /// Whether the bridge mints/burns this token (true) or locks/unlocks it (false)
    mapping(address => bool) public isMintable;

    /// Whether a given Ethereum token is registered at all
    mapping(address => bool) public isRegistered;

    // ── Events ─────────────────────────────────────────────────────────────

    event TokenRegistered(uint32 indexed assetId, address indexed token, bool mintable);
    event TokenDeregistered(uint32 indexed assetId, address indexed token);

    // ── Errors ─────────────────────────────────────────────────────────────

    error TokenAlreadyRegistered(address token);
    error AssetAlreadyRegistered(uint32 assetId);
    error TokenNotRegistered(address token);
    error ZeroAddress();

    // ── Constructor ────────────────────────────────────────────────────────

    constructor(address initialOwner) Ownable(initialOwner) {}

    // ── Owner actions ──────────────────────────────────────────────────────

    /// @notice Register an (assetId, token) pair.
    /// @param assetId  Polkadex asset identifier. 0 is conventionally native PDEX.
    /// @param token    ERC-20 contract address on Ethereum.
    /// @param mintable True  → WrappedPDEX-style: bridge mints on withdraw, burns on deposit.
    ///                False → USDC-style: bridge locks on deposit, unlocks on withdraw.
    function registerToken(
        uint32  assetId,
        address token,
        bool    mintable
    ) external onlyOwner {
        if (token == address(0))           revert ZeroAddress();
        if (isRegistered[token])           revert TokenAlreadyRegistered(token);
        if (assetToToken[assetId] != address(0)) revert AssetAlreadyRegistered(assetId);

        assetToToken[assetId] = token;
        tokenToAsset[token]   = assetId;
        isRegistered[token]   = true;
        isMintable[token]     = mintable;

        emit TokenRegistered(assetId, token, mintable);
    }

    /// @notice Remove a token from the whitelist.
    ///         Any in-flight deposits or withdrawals for this token should be
    ///         completed before calling this.
    function deregisterToken(uint32 assetId) external onlyOwner {
        address token = assetToToken[assetId];
        if (!isRegistered[token]) revert TokenNotRegistered(token);

        delete assetToToken[assetId];
        delete tokenToAsset[token];
        delete isRegistered[token];
        delete isMintable[token];

        emit TokenDeregistered(assetId, token);
    }
}
