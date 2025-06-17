// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

contract Counter {
    uint256 public offset = 42;
    uint256 public number;
    uint256[] sequence;

    function setNumber(uint256 newNumber) public {
        require(offset > newNumber);
        number = offset - newNumber;
    }

    function complex() public {
        uint256 n = 10;
        // Clear and resize the array to accommodate n+1 elements (0 to n inclusive)
        delete sequence;
        for (uint i = 0; i <= n; i++) {
            sequence.push(0); // Initialize with zeros
        }

        for (uint i = 0; i <= n; i++) {
            if (i <= 1) {
                sequence[i] = i;
            } else {
                sequence[i] = sequence[i - 1] + sequence[i - 2];
            }
        }
    }
}
