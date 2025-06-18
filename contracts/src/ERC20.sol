// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

contract ERC20 {
    mapping(address => uint256) public balances;

    constructor() {
        // 10 bilion to the caller
        balances[msg.sender] = 10000000000;
    }

    function transfer(address recipient, uint256 amount) public {
        require(balances[msg.sender] >= amount);
        balances[msg.sender] -= amount;
        balances[recipient] += amount;
    }
}
