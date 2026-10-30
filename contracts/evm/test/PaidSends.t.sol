// Copyright (C) 2026 The Polkadex Authors.
// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.17;

import {Test} from "forge-std/Test.sol";
import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IERC20Errors} from "@openzeppelin/contracts/interfaces/draft-IERC6093.sol";
import {OwnableUpgradeable} from "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import {
    TransparentUpgradeableProxy,
    ITransparentUpgradeableProxy
} from "@openzeppelin/contracts/proxy/transparent/TransparentUpgradeableProxy.sol";
import {ProxyAdmin} from "@openzeppelin/contracts/proxy/transparent/ProxyAdmin.sol";
import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";
import {HyperFungibleTokenUpgradeable as UpstreamHFT} from "@hyperbridge/core/apps/HyperFungibleTokenUpgradeable.sol";
import {
    WrappedHyperFungibleTokenUpgradeable as UpstreamWHFT
} from "@hyperbridge/core/apps/WrappedHyperFungibleTokenUpgradeable.sol";
import {HyperApp} from "@hyperbridge/core/apps/HyperApp.sol";
import {DispatchPost} from "@hyperbridge/core/interfaces/IDispatcher.sol";
import {IncomingPostRequest, PostRequestTimeout} from "@hyperbridge/core/interfaces/IApp.sol";
import {PostRequest} from "@hyperbridge/core/libraries/Message.sol";
import {StateMachine} from "@hyperbridge/core/libraries/StateMachine.sol";
import {BridgeFees} from "../src/BridgeFees.sol";
import {PolkadexHyperFungibleToken as HFT} from "../src/PolkadexHyperFungibleToken.sol";
import {PolkadexWrappedHyperFungibleToken as WHFT} from "../src/PolkadexWrappedHyperFungibleToken.sol";
import {TestUSDT} from "../src/TestUSDT.sol";

/// The host side of a send: records each dispatch and the native value paid with it.
contract MockHost {
    address public immutable feeToken;
    uint256 public dispatched;
    uint256 public lastValue;
    bytes public lastBody;

    constructor(address feeToken_) {
        feeToken = feeToken_;
    }

    function dispatch(DispatchPost memory request) external payable returns (bytes32) {
        dispatched += 1;
        lastValue = msg.value;
        lastBody = request.body;
        return keccak256(abi.encode(request.body, dispatched));
    }
}

contract MockFeeToken is ERC20 {
    constructor() ERC20("Fee token", "FEE") {}
}

/// What both Polkadex token contracts share.
interface IPaidSends {
    function send(HFT.SendParams calldata params) external payable;
    function setFee(bytes calldata dest, uint256 amount) external;
    function removeFee(bytes calldata dest) external;
    function setFeeCollector(address collector) external;
    function setMaxDataLength(uint256 maxDataLength) external;
    function setHost(address newHost) external;
    function feeOf(bytes calldata dest) external view returns (bool set, uint256 amount);
    function feeCollector() external view returns (address);
    function maxDataLength() external view returns (uint256);
    function host() external view returns (address);
    function addChain(bytes calldata chainId, bytes calldata moduleId) external;
    function onAccept(IncomingPostRequest calldata incoming) external;
    function onPostRequestTimeout(PostRequestTimeout calldata incoming) external;
}

/// The fee, size, setter, host and refund cases, run against each contract behind a proxy.
abstract contract PaidSendsTest is Test {
    event HostSet(address newHost);

    uint256 internal constant FEE = 3e6;
    bytes internal polkadex = StateMachine.substrate(bytes4("PDEX"));
    bytes internal pallet = abi.encodePacked(bytes8("pall_hft"));
    address internal owner = makeAddr("owner");
    address internal user = makeAddr("user");
    address internal collector = makeAddr("collector");
    MockFeeToken internal feeToken = new MockFeeToken();
    MockHost internal host = new MockHost(address(feeToken));
    IPaidSends internal token;

    /// The contract under test behind a TransparentUpgradeableProxy owned by `owner`, on `host`.
    function deploy() internal virtual returns (IPaidSends);

    /// The token a send moves: the token itself, or the wrapped one.
    function sent() internal view virtual returns (IERC20);

    /// Gives `who` `amount` of the sent token, approved to the contract under test.
    function fund(address who, uint256 amount) internal virtual;

    /// What a send takes the amount into: the token supply (burnt) or the wrapper's balance (locked).
    function custody() internal view virtual returns (uint256);

    /// The amount taken into custody since `before`.
    function custodyDelta(uint256 before) internal view virtual returns (uint256);

    /// Lets the contract under test pay out `amount` on an incoming transfer.
    function stock(uint256 amount) internal virtual;

    /// The revert of a send that runs short: the balance (burn) or the allowance (lock).
    function shortError(uint256 have, uint256 need) internal view virtual returns (bytes memory);

    function setUp() public {
        token = deploy();
        vm.prank(owner);
        token.addChain(polkadex, pallet);
    }

    function params(uint256 amount) internal view returns (HFT.SendParams memory) {
        bytes memory to = abi.encodePacked(bytes32(uint256(7)));
        return HFT.SendParams({dest: polkadex, to: to, amount: amount, timeout: 0, relayerFee: 0, data: ""});
    }

    function setFees() internal {
        vm.startPrank(owner);
        token.setFee(polkadex, FEE);
        token.setFeeCollector(collector);
        vm.stopPrank();
    }

    /// A transfer in from Polkadex for `to`.
    function incoming(address to, uint256 amount) internal view returns (IncomingPostRequest memory) {
        bytes memory from = abi.encodePacked(bytes32(uint256(1)));
        bytes memory body = abi.encode(HFT.Message({from: from, to: abi.encodePacked(to), amount: amount, data: ""}));
        PostRequest memory request = PostRequest({
            source: polkadex,
            dest: StateMachine.evm(block.chainid),
            nonce: 0,
            from: pallet,
            to: abi.encodePacked(address(token)),
            timeoutTimestamp: 0,
            body: body
        });
        return IncomingPostRequest({request: request, relayer: address(0)});
    }

    function send(address from, HFT.SendParams memory p) internal {
        vm.prank(from);
        token.send(p);
    }

    function expectRevertedSend(bytes4 selector, HFT.SendParams memory p) internal {
        vm.expectRevert(selector);
        send(user, p);
    }

    // The fee goes to the collector on top of the amount, in the token sent, and the message carries the amount.
    function test_SendPaysTheFeeOnTop() public {
        setFees();
        fund(user, 100e6 + FEE);
        uint256 before = custody();
        bytes memory dest = polkadex;
        vm.expectEmit(address(token));
        emit BridgeFees.FeePaid(user, collector, string(dest), FEE);
        send(user, params(100e6));
        assertEq(sent().balanceOf(user), 0);
        assertEq(sent().balanceOf(collector), FEE);
        assertEq(custodyDelta(before), 100e6);
        HFT.Message memory message = abi.decode(host.lastBody(), (HFT.Message));
        assertEq(message.amount, 100e6);
    }

    // Fail closed: no fee for the destination, or no collector, and the send reverts. A zero fee is a set fee.
    function test_RevertWhen_FeeOrCollectorUnset() public {
        fund(user, 10e6);
        (bool set,) = token.feeOf(polkadex);
        assertFalse(set);
        expectRevertedSend(BridgeFees.FeeNotSet.selector, params(1e6));
        vm.prank(owner);
        token.setFee(polkadex, FEE);
        expectRevertedSend(BridgeFees.FeeNotSet.selector, params(1e6));
        vm.startPrank(owner);
        token.setFeeCollector(collector);
        token.removeFee(polkadex);
        vm.stopPrank();
        expectRevertedSend(BridgeFees.FeeNotSet.selector, params(1e6));
        vm.prank(owner);
        token.setFee(polkadex, 0);
        send(user, params(1e6));
        assertEq(sent().balanceOf(user), 9e6);
        assertEq(sent().balanceOf(collector), 0);
    }

    // `to` is capped at 32 bytes and `data` at the owner-set length, 0 until set.
    function test_SizeCaps() public {
        setFees();
        fund(user, 3 * (1e6 + FEE));
        HFT.SendParams memory p = params(1e6);
        p.to = new bytes(33);
        expectRevertedSend(BridgeFees.RecipientTooLong.selector, p);
        p.to = new bytes(32);
        send(user, p);
        p.data = hex"01";
        expectRevertedSend(BridgeFees.DataTooLong.selector, p);
        vm.prank(owner);
        token.setMaxDataLength(64);
        p.data = new bytes(65);
        expectRevertedSend(BridgeFees.DataTooLong.selector, p);
        p.data = new bytes(64);
        send(user, p);
        assertEq(sent().balanceOf(collector), 2 * FEE);
    }

    // Every new setter is owner-only and emits its event; zero addresses are refused.
    function test_SettersAreOwnerOnly() public {
        bytes memory notOwner = abi.encodeWithSelector(OwnableUpgradeable.OwnableUnauthorizedAccount.selector, user);
        vm.startPrank(user);
        vm.expectRevert(notOwner);
        token.setFee(polkadex, FEE);
        vm.expectRevert(notOwner);
        token.removeFee(polkadex);
        vm.expectRevert(notOwner);
        token.setFeeCollector(collector);
        vm.expectRevert(notOwner);
        token.setMaxDataLength(64);
        vm.expectRevert(notOwner);
        token.setHost(address(1));
        vm.stopPrank();

        bytes memory dest = polkadex;
        vm.startPrank(owner);
        vm.expectEmit(address(token));
        emit BridgeFees.FeeSet(string(dest), FEE);
        token.setFee(polkadex, FEE);
        vm.expectEmit(address(token));
        emit BridgeFees.FeeRemoved(string(dest));
        token.removeFee(polkadex);
        vm.expectEmit(address(token));
        emit BridgeFees.FeeCollectorSet(collector);
        token.setFeeCollector(collector);
        vm.expectEmit(address(token));
        emit BridgeFees.MaxDataLengthSet(64);
        token.setMaxDataLength(64);
        vm.expectRevert(BridgeFees.ZeroAddress.selector);
        token.setFeeCollector(address(0));
        vm.expectRevert(BridgeFees.ZeroAddress.selector);
        token.setHost(address(0));
        vm.stopPrank();
        assertEq(token.feeCollector(), collector);
        assertEq(token.maxDataLength(), 64);
    }

    // setHost moves onAccept to the new host: the old one is refused, the new one delivers.
    function test_SetHostMovesOnAccept() public {
        MockHost next = new MockHost(address(feeToken));
        stock(5e6);
        IncomingPostRequest memory transfer = incoming(user, 5e6);
        vm.expectEmit(address(token));
        emit HostSet(address(next));
        vm.prank(owner);
        token.setHost(address(next));
        assertEq(token.host(), address(next));
        vm.expectRevert(HyperApp.UnauthorizedCall.selector);
        vm.prank(address(host));
        token.onAccept(transfer);
        vm.prank(address(next));
        token.onAccept(transfer);
        assertEq(sent().balanceOf(user), 5e6);
    }

    // One unit short of the amount plus the fee and nothing moves, the fee included.
    function test_RevertWhen_AmountPlusFeeShort() public {
        setFees();
        fund(user, 100e6 + FEE - 1);
        vm.expectRevert(shortError(100e6 - 1, 100e6));
        send(user, params(100e6));
        assertEq(sent().balanceOf(user), 100e6 + FEE - 1);
        assertEq(sent().balanceOf(collector), 0);
    }

    // A timed-out send refunds the amount alone; the fee stays with the collector.
    function test_TimeoutRefundsTheAmountOnly() public {
        setFees();
        fund(user, 100e6 + FEE);
        send(user, params(100e6));
        PostRequest memory request = PostRequest({
            source: StateMachine.evm(block.chainid),
            dest: polkadex,
            nonce: 0,
            from: abi.encodePacked(address(token)),
            to: pallet,
            timeoutTimestamp: 1,
            body: host.lastBody()
        });
        vm.prank(address(host));
        token.onPostRequestTimeout(PostRequestTimeout({request: request, relayer: address(0)}));
        assertEq(sent().balanceOf(user), 100e6);
        assertEq(sent().balanceOf(collector), FEE);
    }

    // msg.value still pays Hyperbridge in native tokens; the bridge fee is paid in the token as well.
    function test_NativeValuePaysHyperbridgeOnly() public {
        setFees();
        fund(user, 100e6 + FEE);
        vm.deal(user, 1 ether);
        vm.prank(user);
        token.send{value: 0.01 ether}(params(100e6));
        assertEq(host.lastValue(), 0.01 ether);
        assertEq(sent().balanceOf(collector), FEE);
        assertEq(sent().balanceOf(user), 0);
    }

    /// Upgrades `proxy` to `implementation` through its ProxyAdmin, as the owner.
    function upgrade(address proxy, address implementation) internal {
        ProxyAdmin admin = ProxyAdmin(address(uint160(uint256(vm.load(proxy, ERC1967Utils.ADMIN_SLOT)))));
        vm.prank(owner);
        admin.upgradeAndCall(ITransparentUpgradeableProxy(proxy), implementation, "");
    }

    /// The appended state after setFees: the fee map at `base`, the collector at base + 1, the cap at base + 2.
    function assertBridgeFeeSlots(address proxy, uint256 base) internal view {
        bytes memory dest = polkadex;
        bytes32 entry = keccak256(abi.encodePacked(dest, base));
        assertEq(vm.load(proxy, entry), bytes32(uint256(1))); // Fee.set
        assertEq(vm.load(proxy, bytes32(uint256(entry) + 1)), bytes32(FEE)); // Fee.amount
        assertEq(vm.load(proxy, bytes32(base + 1)), bytes32(uint256(uint160(collector))));
        assertEq(vm.load(proxy, bytes32(base + 2)), bytes32(0)); // no data allowed yet
    }

    function slots(address proxy, uint256 count) internal view returns (bytes32[] memory values) {
        values = new bytes32[](count);
        for (uint256 i = 0; i < count; i++) {
            values[i] = vm.load(proxy, bytes32(i));
        }
    }

    /// After an upgrade, sends fail closed until the owner sets the fee and the collector, then pay it.
    function assertFailsClosedThenPays(address proxy, uint256 balance) internal {
        token = IPaidSends(proxy);
        expectRevertedSend(BridgeFees.FeeNotSet.selector, params(100e6));
        setFees();
        send(user, params(100e6));
        assertEq(sent().balanceOf(user), balance - 100e6 - FEE);
        assertEq(sent().balanceOf(collector), FEE);
    }
}

contract PolkadexHyperFungibleTokenTest is PaidSendsTest {
    function deploy() internal override returns (IPaidSends) {
        bytes memory init = abi.encodeCall(HFT.initialize, ("Polkadex", "PDEX", owner));
        address proxy = address(new TransparentUpgradeableProxy(address(new HFT()), owner, init));
        vm.prank(owner);
        HFT(proxy).configure(HFT.ConfigOptions({host: address(host), dispatcher: address(0)}));
        return IPaidSends(proxy);
    }

    function sent() internal view override returns (IERC20) {
        return IERC20(address(token));
    }

    function fund(address who, uint256 amount) internal override {
        vm.prank(address(host));
        token.onAccept(incoming(who, amount));
    }

    function custody() internal view override returns (uint256) {
        return sent().totalSupply();
    }

    function custodyDelta(uint256 before) internal view override returns (uint256) {
        return before - custody();
    }

    function stock(uint256) internal override {}

    function shortError(uint256 have, uint256 need) internal view override returns (bytes memory) {
        return abi.encodeWithSelector(IERC20Errors.ERC20InsufficientBalance.selector, user, have, need);
    }

    // An existing upstream proxy upgrades in place: balances, settings and the upstream slots stay.
    function test_UpgradeKeepsBalancesAndSettings() public {
        bytes memory init = abi.encodeCall(UpstreamHFT.initialize, ("Polkadex", "PDEX", owner));
        address proxy = address(new TransparentUpgradeableProxy(address(new UpstreamHFT()), owner, init));
        vm.startPrank(owner);
        UpstreamHFT(proxy).configure(UpstreamHFT.ConfigOptions({host: address(host), dispatcher: address(0xd15)}));
        UpstreamHFT(proxy).addChain(polkadex, pallet);
        vm.stopPrank();
        vm.prank(address(host));
        UpstreamHFT(proxy).onAccept(incoming(user, 500e6));
        bytes32[] memory before = slots(proxy, 3);
        assertEq(before[0], bytes32(uint256(uint160(address(host)))));
        assertEq(before[1], bytes32(uint256(uint160(address(0xd15)))));

        upgrade(proxy, address(new HFT()));

        HFT upgraded = HFT(proxy);
        assertEq(upgraded.balanceOf(user), 500e6);
        assertEq(upgraded.totalSupply(), 500e6);
        assertEq(upgraded.name(), "Polkadex");
        assertEq(upgraded.symbol(), "PDEX");
        assertEq(upgraded.owner(), owner);
        assertEq(upgraded.host(), address(host));
        assertEq(upgraded.dispatcher(), address(0xd15));
        assertEq(upgraded.supportedChain(polkadex), pallet);
        assertFailsClosedThenPays(proxy, 500e6);
        assertEq(slots(proxy, 3), before);
        assertBridgeFeeSlots(proxy, 3);
    }
}

contract PolkadexWrappedHyperFungibleTokenTest is PaidSendsTest {
    TestUSDT internal usdt = new TestUSDT(address(this));

    function deploy() internal override returns (IPaidSends) {
        address proxy = address(
            new TransparentUpgradeableProxy(address(new WHFT()), owner, abi.encodeCall(WHFT.initialize, (owner)))
        );
        configure(proxy, address(usdt), false);
        return IPaidSends(proxy);
    }

    function configure(address proxy, address underlying, bool isWeth) internal {
        vm.prank(owner);
        WHFT(payable(proxy)).configure(
            WHFT.WrappedConfigOptions({host: address(host), dispatcher: address(0), underlying: underlying, isWeth: isWeth})
        );
    }

    function sent() internal view override returns (IERC20) {
        return IERC20(address(usdt));
    }

    function fund(address who, uint256 amount) internal override {
        usdt.mint(who, amount);
        vm.prank(who);
        usdt.approve(address(token), amount);
    }

    function custody() internal view override returns (uint256) {
        return usdt.balanceOf(address(token));
    }

    function custodyDelta(uint256 before) internal view override returns (uint256) {
        return custody() - before;
    }

    function stock(uint256 amount) internal override {
        usdt.mint(address(token), amount);
    }

    function shortError(uint256 have, uint256 need) internal view override returns (bytes memory) {
        return abi.encodeWithSelector(IERC20Errors.ERC20InsufficientAllowance.selector, address(token), have, need);
    }

    // The allowance must cover the amount plus the fee: the fee is pulled first, then the amount.
    function test_AllowanceMustCoverAmountPlusFee() public {
        setFees();
        usdt.mint(user, 100e6 + FEE);
        vm.prank(user);
        usdt.approve(address(token), 100e6);
        bytes4 short = IERC20Errors.ERC20InsufficientAllowance.selector;
        vm.expectRevert(abi.encodeWithSelector(short, address(token), 100e6 - FEE, 100e6));
        send(user, params(100e6));
        vm.prank(user);
        usdt.approve(address(token), 100e6 + FEE);
        send(user, params(100e6));
        assertEq(usdt.balanceOf(collector), FEE);
        assertEq(usdt.balanceOf(address(token)), 100e6);
    }

    // configure refuses WETH: send does not wrap, so ETH sent as the amount would only pay the host's fee swap.
    function test_RevertWhen_ConfiguredForWeth() public {
        vm.expectRevert(WHFT.WethNotSupported.selector);
        configure(address(token), address(usdt), true);
        assertFalse(WHFT(payable(address(token))).isWeth());
    }

    // An existing upstream wrapper upgrades in place: locked funds, settings and the upstream slots stay.
    function test_UpgradeKeepsLockedFundsAndSettings() public {
        bytes memory init = abi.encodeCall(UpstreamWHFT.initialize, (owner));
        address proxy = address(new TransparentUpgradeableProxy(address(new UpstreamWHFT()), owner, init));
        vm.startPrank(owner);
        UpstreamWHFT(payable(proxy)).configure(
            UpstreamWHFT.WrappedConfigOptions({
                host: address(host),
                dispatcher: address(0xd15),
                underlying: address(usdt),
                isWeth: false
            })
        );
        UpstreamWHFT(payable(proxy)).addChain(polkadex, pallet);
        vm.stopPrank();
        usdt.mint(user, 500e6);
        vm.prank(user);
        usdt.approve(proxy, 200e6);
        vm.prank(user);
        IPaidSends(proxy).send(params(200e6)); // the upstream send locks 200 with no fee
        bytes32[] memory before = slots(proxy, 4);
        assertEq(before[2], bytes32(uint256(uint160(address(usdt))))); // _underlying, _isWeth false

        upgrade(proxy, address(new WHFT()));

        WHFT upgraded = WHFT(payable(proxy));
        assertEq(usdt.balanceOf(proxy), 200e6);
        assertEq(upgraded.owner(), owner);
        assertEq(upgraded.host(), address(host));
        assertEq(upgraded.dispatcher(), address(0xd15));
        assertEq(upgraded.underlying(), address(usdt));
        assertFalse(upgraded.isWeth());
        assertEq(upgraded.supportedChain(polkadex), pallet);
        token = IPaidSends(proxy);
        vm.prank(address(host));
        upgraded.onAccept(incoming(user, 50e6)); // the locked funds still pay out
        assertEq(usdt.balanceOf(user), 350e6);
        vm.prank(user);
        usdt.approve(proxy, 100e6 + FEE);
        assertFailsClosedThenPays(proxy, 350e6);
        assertEq(slots(proxy, 4), before);
        assertBridgeFeeSlots(proxy, 4);
    }
}
