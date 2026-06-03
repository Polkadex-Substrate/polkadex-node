// SPDX-License-Identifier: GPL-3.0-or-later
pragma solidity ^0.8.22;

/// @notice Minimal WETH9 interface (same on Mainnet and all testnets).
interface IWETH {
    /// @notice Wrap ETH into WETH. Send ETH with msg.value.
    function deposit() external payable;

    /// @notice Unwrap `amount` WETH back to ETH.
    function withdraw(uint256 amount) external;

    function transfer(address to, uint256 value) external returns (bool);
    function approve(address spender, uint256 value) external returns (bool);
    function transferFrom(address from, address to, uint256 value) external returns (bool);
    function balanceOf(address owner) external view returns (uint256);
}
