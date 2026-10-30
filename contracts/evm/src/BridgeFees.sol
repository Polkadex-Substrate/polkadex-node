// Copyright (C) 2026 The Polkadex Authors.
// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.17;

/**
 * @title BridgeFees
 * @notice Paid sends for the Polkadex token contracts. A send pays the owner-set fee for its destination chain, in
 * the token sent, to the fee collector before the burn or lock, and reverts while that fee or the collector is unset.
 * `to` is capped at 32 bytes and `data` at an owner-set length (0 until set, so none). Each token keeps one `Config`
 * after its upstream storage, so existing proxies can upgrade.
 */
library BridgeFees {
    /// @notice The fee per destination chain, the account the fees go to, and the cap on `data` in bytes
    struct Config {
        mapping(bytes => Fee) fees;
        address collector;
        uint256 maxDataLength;
    }

    /// @notice A fee in the token sent; `set` tells a zero fee from none
    struct Fee {
        bool set;
        uint256 amount;
    }

    /// @notice Thrown when the destination has no fee or no fee collector is set
    error FeeNotSet();

    /// @notice Thrown when `to` is longer than 32 bytes
    error RecipientTooLong();

    /// @notice Thrown when `data` is longer than the owner-set cap
    error DataTooLong();

    /// @notice Thrown when a fee collector or a host is the zero address
    error ZeroAddress();

    /// @notice A send paid its fee to the collector
    event FeePaid(address from, address collector, string dest, uint256 fee);

    /// @notice The fee for sends to a destination chain was set
    event FeeSet(string dest, uint256 fee);

    /// @notice The fee for a destination chain was removed; sends there revert
    event FeeRemoved(string dest);

    /// @notice The account the fees go to was set
    event FeeCollectorSet(address collector);

    /// @notice The cap on `data` was set
    event MaxDataLengthSet(uint256 maxDataLength);

    function setFee(Config storage self, bytes calldata dest, uint256 amount) internal {
        self.fees[dest] = Fee({set: true, amount: amount});
        emit FeeSet(string(dest), amount);
    }

    function removeFee(Config storage self, bytes calldata dest) internal {
        delete self.fees[dest];
        emit FeeRemoved(string(dest));
    }

    function setCollector(Config storage self, address collector) internal {
        if (collector == address(0)) revert ZeroAddress();
        self.collector = collector;
        emit FeeCollectorSet(collector);
    }

    function setMaxDataLength(Config storage self, uint256 maxDataLength) internal {
        self.maxDataLength = maxDataLength;
        emit MaxDataLengthSet(maxDataLength);
    }

    /// @notice Reverts unless `to` fits 32 bytes and `data` the cap
    function checkSize(Config storage self, bytes calldata to, bytes calldata data) internal view {
        if (to.length > 32) revert RecipientTooLong();
        if (data.length > self.maxDataLength) revert DataTooLong();
    }

    /// @notice The collector and the fee a send to `dest` owes, which the caller then moves; reverts while unset
    function charge(Config storage self, bytes calldata dest) internal returns (address collector, uint256 amount) {
        Fee storage fee = self.fees[dest];
        collector = self.collector;
        if (!fee.set || collector == address(0)) revert FeeNotSet();
        amount = fee.amount;
        emit FeePaid(msg.sender, collector, string(dest), amount);
    }
}
