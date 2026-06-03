// SPDX-License-Identifier: GPL-3.0-or-later
pragma solidity ^0.8.22;

import {Test}           from "forge-std/Test.sol";
import {ERC20}          from "@openzeppelin/contracts/token/ERC20/ERC20.sol";

import {BeefyLightClient} from "../src/BeefyLightClient.sol";
import {PolkadexBridge}   from "../src/PolkadexBridge.sol";
import {IPolkadexBridge}  from "../src/interfaces/IPolkadexBridge.sol";
import {TokenRegistry}    from "../src/TokenRegistry.sol";
import {MerkleProof}      from "../src/lib/MerkleProof.sol";

// ── Mock WETH9 ─────────────────────────────────────────────────────────────
// Real WETH wraps ETH. This mock does the same so tests behave realistically.

contract MockWETH is ERC20 {
    constructor() ERC20("Wrapped Ether", "WETH") {}

    /// @notice Wrap ETH → WETH (mirrors real WETH9.deposit)
    function deposit() external payable {
        _mint(msg.sender, msg.value);
    }

    /// @notice Unwrap WETH → ETH (mirrors real WETH9.withdraw)
    function withdraw(uint256 amount) external {
        _burn(msg.sender, amount);
        (bool ok,) = msg.sender.call{value: amount}("");
        require(ok, "ETH transfer failed");
    }

    receive() external payable { this.deposit(); }
}

// ── Test suite ─────────────────────────────────────────────────────────────

contract PolkadexBridgeTest is Test {
    BeefyLightClient lc;
    TokenRegistry    registry;
    PolkadexBridge   bridge;
    MockWETH         weth;

    address admin = makeAddr("admin");
    address user  = makeAddr("user");

    // BEEFY validator setup (5 validators)
    uint256[] pks;
    address[] validators;
    uint64 constant SET_ID = 1;

    // WETH is assetId = 1 on Polkadex (0 is reserved for native PDEX)
    uint32 constant WETH_ASSET_ID = 1;

    // Polkadex AccountId of the test user (pretend SS58-decoded public key)
    bytes32 constant POLKADEX_USER = bytes32(uint256(0xBEEFCAFE));

    // ── Setup ──────────────────────────────────────────────────────────────

    function setUp() public {
        // 5 BEEFY validators
        for (uint256 i; i < 5; ++i) {
            (address addr, uint256 pk) = makeAddrAndKey(string(abi.encode(i)));
            validators.push(addr);
            pks.push(pk);
        }

        // Deploy contracts
        address[] memory empty = new address[](0);
        lc       = new BeefyLightClient(admin, validators, SET_ID, empty, SET_ID + 1);
        registry = new TokenRegistry(admin);
        weth     = new MockWETH();
        bridge   = new PolkadexBridge(admin, address(lc), address(registry), address(weth));

        // Register WETH as a lock/unlock token
        vm.prank(admin);
        registry.registerToken(WETH_ASSET_ID, address(weth), false);

        // Give user 10 ETH to play with
        vm.deal(user, 10 ether);

        // Commit an initial batch root (contains two withdrawals for tests)
        _commitRoot(_buildBatchRoot());
    }

    // ── Helpers ────────────────────────────────────────────────────────────

    bytes32 leaf0; // nonce=1, 1 WETH → user
    bytes32 leaf1; // nonce=2, 0.5 WETH → user

    function _buildBatchRoot() internal returns (bytes32 root) {
        leaf0 = MerkleProof.hashLeaf(1, WETH_ASSET_ID, 1 ether,    user, POLKADEX_USER);
        leaf1 = MerkleProof.hashLeaf(2, WETH_ASSET_ID, 0.5 ether,  user, POLKADEX_USER);
        root  = keccak256(abi.encodePacked(bytes1(0x01), leaf0, leaf1));
    }

    function _commitRoot(bytes32 root) internal {
        BeefyLightClient.Commitment memory c = BeefyLightClient.Commitment({
            messagesRoot:       root,
            blockNumber:        1000,
            validatorSetId:     SET_ID,
            nextValidatorsHash: bytes32(0),
            nextValidatorsLen:  0
        });
        bytes32 digest = lc.commitmentDigest(c);
        bytes[] memory sigs = new bytes[](5);
        for (uint256 i; i < 5; ++i) {
            (uint8 v, bytes32 r, bytes32 s) = vm.sign(pks[i], digest);
            sigs[i] = abi.encodePacked(r, s, v);
        }
        lc.submitCommitment(BeefyLightClient.SignedCommitment({commitment: c, signatures: sigs}));
    }

    // ── Deposit: ETH path ──────────────────────────────────────────────────

    function test_depositEth_wraps_and_locks() public {
        vm.prank(user);
        bridge.depositEth{value: 2 ether}(POLKADEX_USER);

        // Bridge holds WETH (not ETH)
        assertEq(weth.balanceOf(address(bridge)), 2 ether, "bridge should hold 2 WETH");
        assertEq(address(bridge).balance,         0,       "bridge should hold no ETH");
        assertEq(bridge.depositNonce(),            1,       "nonce bumped");
    }

    function test_depositEth_emitsDeposit() public {
        vm.expectEmit(true, true, true, true, address(bridge));
        emit IPolkadexBridge.Deposit(address(weth), user, POLKADEX_USER, 1 ether, 1);

        vm.prank(user);
        bridge.depositEth{value: 1 ether}(POLKADEX_USER);
    }

    function test_depositEth_zero_reverts() public {
        vm.prank(user);
        vm.expectRevert(PolkadexBridge.ZeroAmount.selector);
        bridge.depositEth{value: 0}(POLKADEX_USER);
    }

    // ── Deposit: WETH ERC-20 path ──────────────────────────────────────────

    function test_deposit_weth_erc20_locks() public {
        // User wraps ETH → WETH themselves, then deposits
        vm.startPrank(user);
        weth.deposit{value: 3 ether}();
        weth.approve(address(bridge), 3 ether);
        bridge.deposit(address(weth), 3 ether, POLKADEX_USER);
        vm.stopPrank();

        assertEq(weth.balanceOf(address(bridge)), 3 ether);
        assertEq(bridge.depositNonce(), 1);
    }

    function test_deposit_unregisteredToken_reverts() public {
        address randomToken = makeAddr("random");
        vm.prank(user);
        vm.expectRevert(abi.encodeWithSelector(PolkadexBridge.TokenNotSupported.selector, randomToken));
        bridge.deposit(randomToken, 1 ether, POLKADEX_USER);
    }

    // ── Withdraw: Polkadex → Ethereum ──────────────────────────────────────

    function test_withdraw_releases_weth() public {
        // Bridge must hold WETH before it can release it (simulates prior deposits)
        deal(address(weth), address(bridge), 5 ether);

        bytes32[] memory proof = new bytes32[](1);
        proof[0] = leaf1; // sibling of leaf0

        IPolkadexBridge.WithdrawMessage memory msg_ = IPolkadexBridge.WithdrawMessage({
            nonce:          1,
            assetId:        WETH_ASSET_ID,
            amount:         1 ether,
            recipient:      user,
            polkadexSender: POLKADEX_USER
        });

        uint256 balBefore = weth.balanceOf(user);
        bridge.withdraw(msg_, proof, 0, 2);

        assertEq(weth.balanceOf(user), balBefore + 1 ether, "user receives 1 WETH");
        assertTrue(bridge.processedWithdrawals(1),           "nonce marked processed");
    }

    function test_withdraw_emitsWithdrawal() public {
        deal(address(weth), address(bridge), 5 ether);

        bytes32[] memory proof = new bytes32[](1);
        proof[0] = leaf1;

        IPolkadexBridge.WithdrawMessage memory msg_ = IPolkadexBridge.WithdrawMessage({
            nonce: 1, assetId: WETH_ASSET_ID, amount: 1 ether,
            recipient: user, polkadexSender: POLKADEX_USER
        });

        vm.expectEmit(true, true, true, true, address(bridge));
        emit IPolkadexBridge.Withdrawal(1, address(weth), user, 1 ether);
        bridge.withdraw(msg_, proof, 0, 2);
    }

    function test_withdraw_replay_reverts() public {
        deal(address(weth), address(bridge), 5 ether);

        bytes32[] memory proof = new bytes32[](1);
        proof[0] = leaf1;
        IPolkadexBridge.WithdrawMessage memory msg_ = IPolkadexBridge.WithdrawMessage({
            nonce: 1, assetId: WETH_ASSET_ID, amount: 1 ether,
            recipient: user, polkadexSender: POLKADEX_USER
        });

        bridge.withdraw(msg_, proof, 0, 2);

        vm.expectRevert(abi.encodeWithSelector(PolkadexBridge.AlreadyProcessed.selector, uint64(1)));
        bridge.withdraw(msg_, proof, 0, 2);
    }

    function test_withdraw_badProof_reverts() public {
        deal(address(weth), address(bridge), 5 ether);

        bytes32[] memory badProof = new bytes32[](1);
        badProof[0] = keccak256("wrong-sibling");

        IPolkadexBridge.WithdrawMessage memory msg_ = IPolkadexBridge.WithdrawMessage({
            nonce: 1, assetId: WETH_ASSET_ID, amount: 1 ether,
            recipient: user, polkadexSender: POLKADEX_USER
        });

        vm.expectRevert(PolkadexBridge.InvalidProof.selector);
        bridge.withdraw(msg_, badProof, 0, 2);
    }

    function test_withdraw_tampered_amount_reverts() public {
        deal(address(weth), address(bridge), 5 ether);

        bytes32[] memory proof = new bytes32[](1);
        proof[0] = leaf1;

        // Claim 999 WETH but the proof is for 1 WETH → leaf hash mismatch
        IPolkadexBridge.WithdrawMessage memory msg_ = IPolkadexBridge.WithdrawMessage({
            nonce: 1, assetId: WETH_ASSET_ID, amount: 999 ether,
            recipient: user, polkadexSender: POLKADEX_USER
        });

        vm.expectRevert(PolkadexBridge.InvalidProof.selector);
        bridge.withdraw(msg_, proof, 0, 2);
    }

    // ── Second leaf (nonce=2, 0.5 WETH) ───────────────────────────────────

    function test_withdraw_second_leaf() public {
        deal(address(weth), address(bridge), 5 ether);

        bytes32[] memory proof = new bytes32[](1);
        proof[0] = leaf0; // sibling of leaf1

        IPolkadexBridge.WithdrawMessage memory msg_ = IPolkadexBridge.WithdrawMessage({
            nonce: 2, assetId: WETH_ASSET_ID, amount: 0.5 ether,
            recipient: user, polkadexSender: POLKADEX_USER
        });

        uint256 balBefore = weth.balanceOf(user);
        bridge.withdraw(msg_, proof, 1, 2);
        assertEq(weth.balanceOf(user), balBefore + 0.5 ether);
    }

    // ── Pause ──────────────────────────────────────────────────────────────

    function test_paused_depositEth_reverts() public {
        vm.prank(admin);
        bridge.setPaused(true);

        vm.prank(user);
        vm.expectRevert(PolkadexBridge.BridgePaused.selector);
        bridge.depositEth{value: 1 ether}(POLKADEX_USER);
    }

    function test_paused_withdraw_reverts() public {
        vm.prank(admin);
        bridge.setPaused(true);

        IPolkadexBridge.WithdrawMessage memory msg_;
        bytes32[] memory proof;
        vm.expectRevert(PolkadexBridge.BridgePaused.selector);
        bridge.withdraw(msg_, proof, 0, 1);
    }
}
