use std::collections::HashSet;

use crate::utils::{
    calculate_access_list_cost, calculate_calldata_cost, calculate_contract_creation_cost,
    estimate_execution_cost, estimate_storage_cost,
};
use alloy::providers::{Provider, ProviderBuilder};
use revm::primitives::Address;
use serde::{Deserialize, Serialize};

use super::Tx;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasEstimate {
    pub estimated_gas: u128,
    pub gas_price: u128,
    pub total_cost_wei: u128,
    pub breakdown: GasBreakdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasBreakdown {
    pub base_cost: u128,
    pub data_cost: u128,
    pub contract_creation_cost: u128,
    pub execution_cost: u128,
    pub access_list_cost: u128,
    pub storage_cost: u128,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NetworkGasInfo {
    pub current_gas_price: u128,
    pub base_fee_per_gas: Option<u64>,
    pub block_utilization: f64,
    pub latest_block_number: u64,
}

pub struct GasEstimator {
    rpc_url: String,
}

impl GasEstimator {
    pub async fn new(rpc_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            rpc_url: rpc_url.to_string(),
        })
    }

    /// Custom gas estimation implementation from scratch
    pub async fn estimate_gas(
        &self,
        tx_params: Tx,
    ) -> Result<GasEstimate, Box<dyn std::error::Error>> {
        let provider = ProviderBuilder::new().connect(&self.rpc_url).await.unwrap();

        // Calculate gas breakdown using our custom logic
        let breakdown = self.calculate_gas_breakdown(&tx_params).await?;

        // Sum up all gas costs
        let estimated_gas = breakdown.base_cost
            + breakdown.contract_creation_cost
            + breakdown.data_cost
            + breakdown.execution_cost
            + breakdown.access_list_cost
            + breakdown.storage_cost;

        // Get current gas price information
        let gas_price = provider.get_gas_price().await?;

        // Calculate total cost
        let total_cost_wei = estimated_gas * tx_params.gas_price.unwrap_or(gas_price);

        Ok(GasEstimate {
            estimated_gas,
            gas_price,
            total_cost_wei,
            breakdown,
        })
    }

    /// Calculate detailed gas breakdown using specialized estimators
    async fn calculate_gas_breakdown(
        &self,
        tx_params: &Tx,
    ) -> Result<GasBreakdown, Box<dyn std::error::Error>> {
        // Base transaction cost (21,000 gas for simple transfers)
        let base_cost = 21_000;

        let (access_list_cost, loaded_slots) = if tx_params.access_list.is_some() {
            calculate_access_list_cost(tx_params)
        } else {
            (0, HashSet::new())
        };

        let storage_cost = if tx_params.to.is_some() && tx_params.data.is_some() {
            estimate_storage_cost(tx_params.data.as_ref().unwrap(), loaded_slots)
        } else {
            0
        };

        let execution_cost = if tx_params.to.is_some() && tx_params.data.is_some() {
            estimate_execution_cost(tx_params.data.as_ref().unwrap())
        } else {
            0
        };

        // Calculate data cost (calldata)
        let data_cost = if tx_params.data.is_some() && self.is_contract(tx_params.to).await.unwrap()
        {
            calculate_calldata_cost(tx_params.data.as_ref().unwrap())
        } else {
            0
        };

        // Calculate contract creation cost
        let contract_creation_cost = if tx_params.to.is_none() {
            calculate_contract_creation_cost(tx_params.data.as_ref())
        } else {
            0
        };

        Ok(GasBreakdown {
            base_cost,
            data_cost,
            contract_creation_cost,
            execution_cost,
            access_list_cost,
            storage_cost,
        })
    }

    async fn is_contract(&self, to: Option<Address>) -> Result<bool, Box<dyn std::error::Error>> {
        if to.is_none() {
            return Ok(false);
        }
        let provider = ProviderBuilder::new().connect(&self.rpc_url).await.unwrap();
        let code = provider.get_code_at(to.unwrap()).await?;
        Ok(!code.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{address, U64};
    use alloy::sol;
    use alloy::sol_types::SolCall;
    use revm::primitives::{Bytes, U256};
    use Counter::setNumberCall;

    const ETH_RPC_URL: &str = "http://localhost:8545";
    const MNEMONIC: &str = "test test test test test test test test test test test junk";

    sol! {
        // SPDX-License-Identifier: MIT
        pragma solidity ^0.8.20;

        // solc contracts/src/Counter.sol --via-ir --optimize --bin
        #[sol(rpc, bytecode="60808060405234601957602a5f55610106908161001e8239f35b5f80fdfe608060405260043610156010575f80fd5b5f3560e01c80633fb5c1cb1460af5780638381f58a146094578063d09de08a14605e5763d5556544146040575f80fd5b34605a575f366003190112605a5760205f54604051908152f35b5f80fd5b34605a575f366003190112605a576001545f1981146080576001016001555f80f35b634e487b7160e01b5f52601160045260245ffd5b34605a575f366003190112605a576020600154604051908152f35b34605a576020366003190112605a575f54600435810180911160805760015500fea2646970667358221220e470db5efcff30a5d2bf2dfc5c01072c1364af37644d14ea4b2c86293086d86664736f6c634300081e0033")]
        contract Counter {
            uint256 public offset = 42;
            uint256 public number;

            function setNumber(uint256 newNumber) public {
                number = offset + newNumber;
            }

            function increment() public {
                number++;
            }
        }
    }

    // Test helper to create a basic transaction
    fn create_basic_tx() -> Tx {
        Tx {
            from: Some(address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266")),
            to: Some(address!("0x1234567890123456789012345678901234567890")),
            value: U256::from(1000000000000000000u64), // 1 ETH in wei
            data: None,
            nonce: Some(1),
            chain_id: Some(U64::from(1)),
            gas_limit: Some(21000),
            gas_price: Some(20000000000), // 20 gwei
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            access_list: None,
            transaction_type: Some(U64::from(0)),
        }
    }

    // Test helper to create a contract deployment transaction
    fn create_contract_deployment_tx() -> Tx {
        let bytecode = Bytes::from("60808060405234601957602a5f55610106908161001e8239f35b5f80fdfe608060405260043610156010575f80fd5b5f3560e01c80633fb5c1cb1460af5780638381f58a146094578063d09de08a14605e5763d5556544146040575f80fd5b34605a575f366003190112605a5760205f54604051908152f35b5f80fd5b34605a575f366003190112605a576001545f1981146080576001016001555f80f35b634e487b7160e01b5f52601160045260245ffd5b34605a575f366003190112605a576020600154604051908152f35b34605a576020366003190112605a575f54600435810180911160805760015500fea2646970667358221220e470db5efcff30a5d2bf2dfc5c01072c1364af37644d14ea4b2c86293086d86664736f6c634300081e0033");

        let tx = Tx {
            from: Some(address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266")),
            to: None, // Contract deployment
            value: U256::ZERO,
            data: Some(bytecode),
            nonce: Some(1),
            chain_id: Some(U64::from(1)),
            gas_limit: None,
            gas_price: Some(20000000000),
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            access_list: None,
            transaction_type: Some(U64::from(0)),
        };

        tx
    }

    // Test helper to create a contract call transaction
    fn create_contract_call_tx() -> Tx {
        // ERC20 transfer function call: setNumber(uint256 number)
        // number: 1000000000000000000
        let call_data = setNumberCall::new((U256::from(1000000000000000000u64),));

        Tx {
            from: Some(address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266")),
            to: Some(address!("0x1234567890123456789012345678901234567890")),
            value: U256::ZERO,
            data: Some(Bytes::from(call_data.abi_encode())),
            nonce: Some(1),
            chain_id: Some(U64::from(1)),
            gas_limit: None,
            gas_price: Some(20000000000),
            max_fee_per_gas: Some(30000000000),
            max_priority_fee_per_gas: Some(2000000000),
            access_list: None,
            transaction_type: Some(U64::from(2)), // EIP-1559
        }
    }

    #[tokio::test]
    async fn test_estimate_gas_simple_transfer() {
        let estimator = GasEstimator::new(&ETH_RPC_URL).await.unwrap();
        let tx = create_basic_tx();

        let result = estimator.estimate_gas(tx).await;
        assert!(result.is_ok());

        let estimate = result.unwrap();
        assert!(estimate.estimated_gas >= 21000);
        assert!(estimate.gas_price > 0);
    }

    #[tokio::test]
    #[ignore = "code: -32003, message: transaction already imported"]
    async fn test_estimate_gas_contract_deployment() {
        let estimator = GasEstimator::new(&ETH_RPC_URL).await.unwrap();
        let tx = create_contract_deployment_tx();

        let result = estimator.estimate_gas(tx).await;
        assert!(result.is_ok());

        let estimate = result.unwrap();
        assert!(estimate.estimated_gas >= 21000);
        assert!(estimate.gas_price > 0);
    }

    #[tokio::test]
    async fn test_estimate_gas_contract_call() {
        let estimator = GasEstimator::new(&ETH_RPC_URL).await.unwrap();
        let tx = create_contract_call_tx();

        let result = estimator.estimate_gas(tx).await;
        assert!(result.is_ok());

        let estimate = result.unwrap();
        assert!(estimate.estimated_gas >= 21000);
        assert!(estimate.gas_price > 0);
    }
}
