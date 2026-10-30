// Copyright (C) 2026 The Polkadex Authors.
// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.17;

import {Script, console} from "forge-std/Script.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {ProxyAdmin} from "@openzeppelin/contracts/proxy/transparent/ProxyAdmin.sol";
import {ITransparentUpgradeableProxy} from "@openzeppelin/contracts/proxy/transparent/TransparentUpgradeableProxy.sol";
import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";
import {StateMachine} from "@hyperbridge/core/libraries/StateMachine.sol";
import {PolkadexHyperFungibleToken as HFT} from "../src/PolkadexHyperFungibleToken.sol";
import {PolkadexWrappedHyperFungibleToken as WHFT} from "../src/PolkadexWrappedHyperFungibleToken.sol";

/// @notice Paid sends on existing token proxies: each proxy upgrades in place to our implementation through its
/// ProxyAdmin, then gets its fee for sends to Polkadex, the fee collector and the `data` cap. It checks that the
/// upstream slots, owner, host, Polkadex peer, token supply and locked funds survive. Run as the ProxyAdmin owner;
/// without --broadcast it is a dry run:
///     HFT_PROXY=0x... WHFT_PROXY=0x... forge script script/StageFees.s.sol --rpc-url <rpc> --sender <owner>
/// Env: HFT_PROXY and WHFT_PROXY (required), FEE_COLLECTOR (default the owner), HFT_FEE (default 6e18),
/// WHFT_FEE (default 1e6), MAX_DATA_LENGTH (default 0).
contract StageFees is Script {
    function run() external {
        address hft = vm.envAddress("HFT_PROXY");
        address payable whft = payable(vm.envAddress("WHFT_PROXY"));
        bytes memory polkadex = StateMachine.substrate(bytes4("PDEX"));
        address owner = admin(hft).owner();
        require(owner == msg.sender && admin(whft).owner() == owner, "run as the ProxyAdmin owner");
        address collector = vm.envOr("FEE_COLLECTOR", owner);
        uint256 hftFee = vm.envOr("HFT_FEE", uint256(6e18));
        uint256 whftFee = vm.envOr("WHFT_FEE", uint256(1e6));
        uint256 maxDataLength = vm.envOr("MAX_DATA_LENGTH", uint256(0));
        IERC20 wrapped = IERC20(WHFT(whft).underlying());
        bytes32 hftKept = kept(hft, polkadex, IERC20(hft).totalSupply());
        bytes32 whftKept = kept(whft, polkadex, wrapped.balanceOf(whft));

        vm.startBroadcast();
        upgrade(hft, address(new HFT()));
        upgrade(whft, address(new WHFT()));
        HFT(hft).setFee(polkadex, hftFee);
        HFT(hft).setFeeCollector(collector);
        HFT(hft).setMaxDataLength(maxDataLength);
        WHFT(whft).setFee(polkadex, whftFee);
        WHFT(whft).setFeeCollector(collector);
        WHFT(whft).setMaxDataLength(maxDataLength);
        vm.stopBroadcast();

        require(kept(hft, polkadex, IERC20(hft).totalSupply()) == hftKept, "token state changed");
        require(kept(whft, polkadex, wrapped.balanceOf(whft)) == whftKept, "wrapper state changed");
        report("token", hft, polkadex, IERC20(hft).totalSupply());
        report("wrapper", whft, polkadex, wrapped.balanceOf(whft));
    }

    function admin(address proxy) internal view returns (ProxyAdmin) {
        return ProxyAdmin(address(uint160(uint256(vm.load(proxy, ERC1967Utils.ADMIN_SLOT)))));
    }

    function upgrade(address proxy, address implementation) internal {
        admin(proxy).upgradeAndCall(ITransparentUpgradeableProxy(proxy), implementation, "");
    }

    /// What the upgrade must keep: the upstream slots 0-3, owner, host, Polkadex peer, and the supply or locked funds.
    function kept(address proxy, bytes memory polkadex, uint256 held) internal view returns (bytes32) {
        bytes32[4] memory upstream;
        for (uint256 i = 0; i < 4; i++) {
            upstream[i] = vm.load(proxy, bytes32(i));
        }
        HFT token = HFT(proxy); // owner, host and supportedChain have the same selectors on both contracts
        return keccak256(abi.encode(upstream, token.owner(), token.host(), token.supportedChain(polkadex), held));
    }

    function report(string memory name, address proxy, bytes memory polkadex, uint256 held) internal view {
        (bool set, uint256 fee) = HFT(proxy).feeOf(polkadex);
        address implementation = address(uint160(uint256(vm.load(proxy, ERC1967Utils.IMPLEMENTATION_SLOT))));
        console.log(name, proxy);
        console.log("  implementation", implementation);
        console.log("  fee to Polkadex set", set, fee);
        console.log("  collector", HFT(proxy).feeCollector());
        console.log("  max data length", HFT(proxy).maxDataLength());
        console.log("  supply or locked, unchanged", held);
    }
}
