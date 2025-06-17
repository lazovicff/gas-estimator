// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Counter} from "./Counter.sol";

contract Caller {
    bytes32 h;

    function precompile(uint256 numberToHash) public {
        (bool ok, bytes memory out) = address(0x02).staticcall(abi.encode(numberToHash));
        require(ok);
        h = abi.decode(out, (bytes32));
    }

    function call_counter(address counterAddress) public {
        bytes memory payload = abi.encodeWithSignature("setNumber(uint256)", 20);
        (bool success,) = address(counterAddress).call(payload);
        require(success);
    }
}
