// Copyright (C) 2026 The Polkadex Authors.
// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.17;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";

/// @notice Test stablecoin for the bridge rehearsal: 6 decimals, minted only by its owner.
contract TestUSDT is ERC20, Ownable {
    constructor(address owner_) ERC20("Test USDT", "tUSDT") Ownable(owner_) {}

    function decimals() public pure override returns (uint8) {
        return 6;
    }

    function mint(address to, uint256 amount) external onlyOwner {
        _mint(to, amount);
    }
}
