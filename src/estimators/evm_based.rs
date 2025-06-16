use crate::{
    estimators::BLOCK_GAS_LIMIT,
    utils::{calculate_calldata_cost, calculate_contract_creation_cost},
};
use alloy::{
    eips::BlockId,
    providers::{Provider, ProviderBuilder},
};
use revm::{
    context::{transaction::AccessList, tx::TxEnvBuilder},
    database::{CacheDB, EmptyDB},
    primitives::{keccak256, Address, TxKind, U256},
    state::{AccountInfo, Bytecode},
    Context, ExecuteEvm, MainBuilder, MainContext,
};
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
        let estimated_gas =
            breakdown.base_cost + breakdown.contract_creation_cost + breakdown.execution_cost;

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
        let provider = ProviderBuilder::new().connect(&self.rpc_url).await.unwrap();
        let current_gas_price = provider.get_gas_price().await?;
        // Base transaction cost (21,000 gas for simple transfers)
        let base_cost = 21_000;

        let execution_cost = if tx_params.to.is_some() && tx_params.data.is_some() {
            let mut cache_db = CacheDB::new(EmptyDB::default());

            // Get actual balance from the provider
            let caller = tx_params.from.unwrap();
            let balance = provider.get_balance(caller).await.unwrap_or_else(|_| {
                // Fallback to a reasonable amount if balance fetch fails
                U256::from(10u128.pow(18) * 1000) // 1000 ETH
            });
            let nonce = provider.get_transaction_count(caller).await.unwrap_or(0);

            cache_db.insert_account_info(
                caller,
                AccountInfo {
                    balance,
                    nonce: tx_params.nonce.unwrap_or(nonce),
                    code_hash: revm::primitives::KECCAK_EMPTY,
                    code: None,
                },
            );

            // Get contract code from provider and add it to cache
            let contract_address = tx_params.to.unwrap();
            let contract_code = provider
                .get_code_at(contract_address)
                .await
                .unwrap_or_default();
            assert!(!contract_code.is_empty());
            cache_db.insert_account_info(
                contract_address,
                AccountInfo {
                    balance: U256::ZERO,
                    nonce: 0,
                    code_hash: keccak256(&contract_code),
                    code: Some(Bytecode::new_raw(contract_code)),
                },
            );

            // Initialise storage
            // Only accounts for primitive storage variables, excluding mappings and arrays and structs
            for i in 0..256 {
                let storage_val = provider
                    .get_storage_at(contract_address, U256::from(i))
                    .await
                    .unwrap();
                cache_db
                    .insert_account_storage(contract_address, U256::from(i), storage_val)
                    .unwrap();
            }

            let mut evm = Context::mainnet().with_db(cache_db).build_mainnet();
            let tx_evm = TxEnvBuilder::new()
                .caller(caller)
                .kind(TxKind::Call(tx_params.to.unwrap()))
                .data(tx_params.data.clone().unwrap())
                .value(tx_params.value)
                .gas_price(tx_params.gas_price.unwrap_or(current_gas_price))
                .gas_limit(tx_params.gas_limit.unwrap_or(BLOCK_GAS_LIMIT))
                .nonce(tx_params.nonce.unwrap_or(1))
                .access_list(
                    tx_params
                        .access_list
                        .clone()
                        .unwrap_or(AccessList::default()),
                )
                .build()
                .unwrap();

            // Execute transaction without writing to the DB
            match evm.transact_finalize(tx_evm) {
                Ok(result) => {
                    println!("result: {:?}", result);
                    result.result.gas_used() as u128 + 2_300 // Basic stipend for contract calls
                }
                Err(e) => {
                    println!("EVM execution error: {:?}", e);
                    // Return a default gas cost for contract calls
                    30_000
                }
            }
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

    pub async fn get_network_gas_info(&self) -> Result<NetworkGasInfo, Box<dyn std::error::Error>> {
        let provider = ProviderBuilder::new().connect(&self.rpc_url).await.unwrap();
        let gas_price = provider.get_gas_price().await?;
        let latest_block = provider.get_block(BlockId::latest()).await?.unwrap();

        let base_fee_per_gas = latest_block.header.base_fee_per_gas;
        let gas_used = latest_block.header.gas_used;
        let gas_limit = latest_block.header.gas_limit;

        let utilization = if gas_limit > 0 {
            (gas_used as f64 / gas_limit as f64) * 100.0
        } else {
            0.0
        };

        Ok(NetworkGasInfo {
            current_gas_price: gas_price,
            base_fee_per_gas,
            block_utilization: utilization,
            latest_block_number: latest_block.header.number,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{address, U64};
    use alloy::signers::local::coins_bip39::English;
    use alloy::signers::local::MnemonicBuilder;
    use alloy::sol;
    use alloy::sol_types::SolCall;
    use revm::primitives::Bytes;
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
    async fn create_contract_deployment_tx() -> (Tx, Address) {
        // Deploy the contract bytecode using provider
        let wallet = MnemonicBuilder::<English>::default()
            .phrase(MNEMONIC)
            .index(0)
            .unwrap()
            .build()
            .unwrap();
        let provider = ProviderBuilder::new()
            .wallet(wallet.clone())
            .connect(ETH_RPC_URL)
            .await
            .unwrap();
        let bytecode = Bytes::from("60808060405234601957602a5f55610106908161001e8239f35b5f80fdfe608060405260043610156010575f80fd5b5f3560e01c80633fb5c1cb1460af5780638381f58a146094578063d09de08a14605e5763d5556544146040575f80fd5b34605a575f366003190112605a5760205f54604051908152f35b5f80fd5b34605a575f366003190112605a576001545f1981146080576001016001555f80f35b634e487b7160e01b5f52601160045260245ffd5b34605a575f366003190112605a576020600154604051908152f35b34605a576020366003190112605a575f54600435810180911160805760015500fea2646970667358221220e470db5efcff30a5d2bf2dfc5c01072c1364af37644d14ea4b2c86293086d86664736f6c634300081e0033");
        let contract = Counter::deploy(&provider).await.unwrap();
        let contract_address = contract.address();

        let tx = Tx {
            from: Some(wallet.address()),
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

        (tx, *contract_address)
    }

    // Test helper to create a contract call transaction
    fn create_contract_call_tx(contract_address: Address) -> Tx {
        // ERC20 transfer function call: setNumber(uint256 number)
        // number: 1000000000000000000
        let call_data = setNumberCall::new((U256::from(1000000000000000000u64),));

        let wallet = MnemonicBuilder::<English>::default()
            .phrase(MNEMONIC)
            .index(0)
            .unwrap()
            .build()
            .unwrap();

        Tx {
            from: Some(wallet.address()),
            to: Some(contract_address),
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
        let (tx, _contract_address) = create_contract_deployment_tx().await;

        let result = estimator.estimate_gas(tx).await;
        assert!(result.is_ok());

        let estimate = result.unwrap();
        assert!(estimate.estimated_gas >= 21000);
        assert!(estimate.gas_price > 0);
    }

    #[tokio::test]
    async fn test_estimate_gas_contract_call() {
        let estimator = GasEstimator::new(&ETH_RPC_URL).await.unwrap();
        let (_, contract_address) = create_contract_deployment_tx().await;
        let tx = create_contract_call_tx(contract_address);

        let result = estimator.estimate_gas(tx).await;
        assert!(result.is_ok());

        let estimate = result.unwrap();
        assert!(estimate.estimated_gas >= 21000);
        assert!(estimate.gas_price > 0);
    }

    #[tokio::test]
    async fn test_get_network_gas_info() {
        let estimator = GasEstimator::new(&ETH_RPC_URL).await.unwrap();

        let result = estimator.get_network_gas_info().await;
        assert!(result.is_ok());

        let info = result.unwrap();
        assert!(info.current_gas_price > 0);
        assert!(info.block_utilization >= 0.0 && info.block_utilization <= 100.0);
    }
}
