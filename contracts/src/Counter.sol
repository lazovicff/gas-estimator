// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

contract Counter {
    uint256 public offset = 42;
    uint256 public number;
    uint256 n = 10;
    uint256 f;

    function setNumber(uint256 newNumber) public {
        require(offset > newNumber);
        number = offset - newNumber;
    }

    function complex() public {
        require(n > 0);
        uint[] memory sequence = new uint[](n+1);
        for (uint i = 0; i <= n; i++) {
            if (i <= 1) {
                sequence[i] = i;
            } else {
                sequence[i] = sequence[i -1] + sequence[i -2];
            }
        }
        f = sequence[n];
    }
}
