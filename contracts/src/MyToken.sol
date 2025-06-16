// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract MyToken {
    uint256 private constant _initialSupply = 100e12; // 100 trillion tokens
    mapping(address => uint256) balances;

    constructor() {
        balances[address] = _initialSupply;
    }

    function transfer(address recipient, uint256 amount) public returns (bool) {
        require(balances[msg.sender] >= amount);
        balances[msg.sender] -= amount;
        balances[recipient] -= amount;

        return true;
    }
}
