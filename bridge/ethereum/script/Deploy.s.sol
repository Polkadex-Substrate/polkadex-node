// SPDX-License-Identifier: GPL-3.0-or-later
pragma solidity ^0.8.22;

import {Script, console2} from "forge-std/Script.sol";
import {BeefyLightClient} from "../src/BeefyLightClient.sol";
import {TokenRegistry}    from "../src/TokenRegistry.sol";
import {PolkadexBridge}   from "../src/PolkadexBridge.sol";

/// @notice Deployment script for the Polkadex ↔ Ethereum WETH bridge.
///
/// Usage (Sepolia):
///   forge script script/Deploy.s.sol \
///     --rpc-url $SEPOLIA_RPC_URL \
///     --broadcast \
///     --verify \
///     -vvvv
///
/// Required env vars (put in .env):
///   DEPLOYER_PRIVATE_KEY  — deployer wallet private key
///   ADMIN_ADDRESS         — multisig that will own all contracts
///   BEEFY_VALIDATORS      — comma-separated secp256k1 addresses of the initial BEEFY validator set
///   BEEFY_SET_ID          — initial validator set ID from the Polkadex runtime
///
/// WETH addresses (already hardcoded per network):
///   Sepolia mainnet  : 0xfFf9976782d46CC05630D1f6eBAb18b2324d6B14
///   Ethereum mainnet : 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2
contract DeployScript is Script {
    // ── Known WETH addresses ───────────────────────────────────────────────

    address constant WETH_SEPOLIA  = 0xfFf9976782d46CC05630D1f6eBAb18b2324d6B14;
    address constant WETH_MAINNET  = 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2;

    // Polkadex assetId assigned to bridged WETH (must match the Polkadex runtime config)
    uint32 constant WETH_ASSET_ID = 1;

    function run() external {
        uint256 deployerKey = vm.envUint("DEPLOYER_PRIVATE_KEY");
        address admin       = vm.envAddress("ADMIN_ADDRESS");
        uint64  setId       = uint64(vm.envUint("BEEFY_SET_ID"));
        address[] memory currentValidators = _parseValidators(vm.envString("BEEFY_VALIDATORS"));
        address[] memory nextValidators     = currentValidators;

        // Pick WETH based on chain ID
        address weth = block.chainid == 1 ? WETH_MAINNET : WETH_SEPOLIA;

        vm.startBroadcast(deployerKey);

        // 1. BEEFY light client
        BeefyLightClient lc = new BeefyLightClient(
            admin,
            currentValidators,
            setId,
            nextValidators,
            setId + 1
        );
        console2.log("BeefyLightClient:", address(lc));

        // 2. Token registry
        TokenRegistry registry = new TokenRegistry(admin);
        console2.log("TokenRegistry:   ", address(registry));

        // 3. Bridge (WETH-aware)
        PolkadexBridge bridge = new PolkadexBridge(admin, address(lc), address(registry), weth);
        console2.log("PolkadexBridge:  ", address(bridge));
        console2.log("WETH address:    ", weth);

        vm.stopBroadcast();

        // Post-deployment — call these from the admin multisig:
        console2.log("");
        console2.log("=== Post-deployment steps (admin multisig) ===");
        console2.log("registry.registerToken(WETH_ASSET_ID=1, WETH_ADDRESS, mintable=false)");
        console2.log("  -> WETH_ASSET_ID must match the assetId in the Polkadex runtime");
    }

    // ── Helpers ────────────────────────────────────────────────────────────

    function _parseValidators(string memory csv) internal pure returns (address[] memory addrs) {
        bytes memory b = bytes(csv);
        uint256 count = 1;
        for (uint256 i; i < b.length; ++i) if (b[i] == ",") ++count;

        addrs = new address[](count);
        uint256 start;
        uint256 idx;
        for (uint256 i; i <= b.length; ++i) {
            if (i == b.length || b[i] == ",") {
                bytes memory part = new bytes(i - start);
                for (uint256 j; j < part.length; ++j) part[j] = b[start + j];
                addrs[idx++] = _parseAddr(string(part));
                start = i + 1;
            }
        }
    }

    function _parseAddr(string memory s) internal pure returns (address) {
        bytes memory b = bytes(s);
        require(b.length == 42, "expected 0x-prefixed address");
        uint256 result;
        for (uint256 i = 2; i < 42; ++i) {
            result <<= 4;
            uint8 c = uint8(b[i]);
            if      (c >= 48 && c <= 57)  result |= c - 48;
            else if (c >= 65 && c <= 70)  result |= c - 55;
            else if (c >= 97 && c <= 102) result |= c - 87;
            else revert("invalid hex char");
        }
        // forge-lint: disable-next-line(unsafe-typecast)
        return address(uint160(result));
    }
}
