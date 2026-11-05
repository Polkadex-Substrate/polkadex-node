// Copyright (C) 2026 The Polkadex Authors.
// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.17;

/**
 * @title BridgeFees
 * @notice Paid sends for the Polkadex token contracts. A send pays the owner-set fee for its destination chain, in
 * the token sent, to the fee collector before the burn or lock, and reverts while that fee or the collector is unset.
 * Each destination also has a unit, 10^(decimals here - decimals there): an amount that is not a multiple of it
 * cannot arrive exactly, so the send reverts.
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

    /// @notice A fee in the token sent; `set` tells a zero fee from none. Amounts sent must be multiples of `unit`.
    struct Fee {
        bool set;
        uint256 amount;
        uint256 unit;
    }

    /// @notice Thrown when the destination has no fee or no fee collector is set
    error FeeNotSet();

    /// @notice Thrown when `to` is longer than 32 bytes
    error RecipientTooLong();

    /// @notice Thrown when `data` is longer than the owner-set cap
    error DataTooLong();

    /// @notice Thrown when a fee collector or a host is the zero address
    error ZeroAddress();

    /// @notice Thrown when the amount is not a multiple of the destination's unit, or the unit is 0
    error AmountNotRepresentable();

    /// @notice A send paid its fee to the collector
    event FeePaid(address from, address collector, string dest, uint256 fee);

    /// @notice The fee and amount unit for sends to a destination chain were set
    event FeeSet(string dest, uint256 fee, uint256 unit);

    /// @notice The fee for a destination chain was removed; sends there revert
    event FeeRemoved(string dest);

    /// @notice The account the fees go to was set
    event FeeCollectorSet(address collector);

    /// @notice The cap on `data` was set
    event MaxDataLengthSet(uint256 maxDataLength);

    function setFee(Config storage self, bytes calldata dest, uint256 amount, uint256 unit) internal {
        self.fees[dest] = Fee({set: true, amount: amount, unit: unit});
        emit FeeSet(string(dest), amount, unit);
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

    /// @notice The collector and the fee a send of `sent` to `dest` owes, which the caller then moves; reverts while
    /// unset, or when `sent` is not a multiple of the destination's unit (a unit of 0 fails closed)
    function charge(Config storage self, bytes calldata dest, uint256 sent)
        internal
        returns (address collector, uint256 amount)
    {
        Fee storage fee = self.fees[dest];
        collector = self.collector;
        if (!fee.set || collector == address(0)) revert FeeNotSet();
        if (fee.unit == 0 || sent % fee.unit != 0) revert AmountNotRepresentable();
        amount = fee.amount;
        // Match the pallet: a zero fee moves nothing, so it emits nothing.
        if (amount != 0) emit FeePaid(msg.sender, collector, string(dest), amount);
    }
}
