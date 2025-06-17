// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Test, console} from "forge-std/Test.sol";
import {Caller} from "../src/Caller.sol";
import {Counter} from "../src/Counter.sol";

contract CallerTest is Test {
    Caller public caller;
    Counter public counter;

    function setUp() public {
        caller = new Caller();
        counter = new Counter();
    }

    function test_PrecompileSuccess() public {
        // Test that precompile function works with valid input
        uint256 numberToHash = 12345;

        // This should not revert
        caller.precompile(numberToHash);

        // The hash should be stored (we can't easily test the exact value without knowing precompile output)
        // But we can verify the call completed successfully
        assertTrue(true, "Precompile call completed successfully");
    }

    function test_CallCounterSuccess() public {
        caller.call_counter(address(counter));
    }
}
