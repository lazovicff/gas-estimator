// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Test, console} from "forge-std/Test.sol";
import {ERC20} from "../src/ERC20.sol";

contract ERC20Test is Test {
    ERC20 public token;
    address public owner;
    address public alice;
    address public bob;

    function setUp() public {
        owner = address(this);
        alice = address(0x1);
        bob = address(0x2);

        token = new ERC20();
    }

    function test_InitialBalance() public {
        // Owner should have 10 billion tokens initially
        assertEq(token.balances(owner), 10000000000);
    }

    function test_Transfer() public {
        uint256 transferAmount = 1000;
        uint256 initialOwnerBalance = token.balances(owner);
        uint256 initialAliceBalance = token.balances(alice);

        // Transfer tokens from owner to alice
        token.transfer(alice, transferAmount);

        // Check balances after transfer
        assertEq(token.balances(owner), initialOwnerBalance - transferAmount);
        assertEq(token.balances(alice), initialAliceBalance + transferAmount);
    }
}
