use crate::utils::{calculate_calldata_cost, calculate_contract_creation_cost};
use alloy::{
    eips::BlockId,
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
};
use revm::{
    context::{transaction::AccessList, tx::TxEnvBuilder},
    database::{CacheDB, EmptyDB},
    primitives::{alloy_primitives::U64, keccak256, Address, Bytes, TxKind, U256},
    state::{AccountInfo, Bytecode},
    Context, ExecuteEvm, MainBuilder, MainContext,
};
use serde::{Deserialize, Serialize};

const BLOCK_GAS_LIMIT: u64 = 30_000_000; // or 36,000,000

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tx {
    // Standard transaction fields
    pub from: Option<Address>,
    pub to: Option<Address>,
    pub value: U256,
    #[serde(alias = "input")]
    pub data: Option<Bytes>,
    pub nonce: Option<u64>,
    #[serde(alias = "chainId")]
    pub chain_id: Option<U64>,

    // Gas fields - using standard names
    pub gas_limit: Option<u64>,
    #[serde(alias = "gasPrice")]
    pub gas_price: Option<u128>,
    #[serde(alias = "maxFeePerGas")]
    pub max_fee_per_gas: Option<u128>,
    #[serde(alias = "maxPriorityFeePerGas")]
    pub max_priority_fee_per_gas: Option<u128>,

    // EIP-2930 Access List
    #[serde(alias = "accessList")]
    pub access_list: Option<AccessList>,

    // Transaction type (0=Legacy, 1=EIP-2930, 2=EIP-1559)
    #[serde(alias = "type")]
    pub transaction_type: Option<U64>,
}

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
            + breakdown.execution_cost
            + breakdown.access_list_cost;

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

            for i in 0..32 {
                let storage_val = provider
                    .get_storage_at(contract_address, U256::from(i))
                    .await
                    .unwrap();
                println!("{} {}", U256::from(i), storage_val);
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

        // Calculate access list cost (EIP-2930)
        let access_list_cost = self.calculate_access_list_cost(tx_params).await?;

        Ok(GasBreakdown {
            base_cost,
            data_cost,
            contract_creation_cost,
            execution_cost,
            access_list_cost,
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

    /// Calculate access list cost (EIP-2930)
    async fn calculate_access_list_cost(
        &self,
        tx_params: &Tx,
    ) -> Result<u128, Box<dyn std::error::Error>> {
        let provider = ProviderBuilder::new().connect(&self.rpc_url).await.unwrap();
        // Simple heuristic: estimate potential access list items
        let mut cost = 0;

        if let Some(to) = tx_params.to {
            // Check if target is a contract that might benefit from access list
            let code = provider.get_code_at(to).await?;
            if !code.is_empty() {
                // Estimate potential storage slots accessed
                cost += 2_400; // ADDRESS_ACCESS_COST
                cost += 1_900 * 2; // STORAGE_KEY_ACCESS_COST for 2 slots
            }
        }

        Ok(cost)
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
    use alloy::primitives::{address, bytes};
    use alloy::signers::local::coins_bip39::English;
    use alloy::signers::local::MnemonicBuilder;
    use alloy::sol;
    use alloy::sol_types::SolCall;

    const ETH_RPC_URL: &str = "http://localhost:8545";
    const MNEMONIC: &str = "test test test test test test test test test test test junk";

    sol! {
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
        let bytecode = MyToken::deploy(&provider).await.unwrap();

        let res = deployment_tx.watch().await.unwrap();
        // Get the deployed contract address from the transaction receipt
        let receipt = provider
            .get_transaction_receipt(res)
            .await
            .unwrap()
            .unwrap();
        let contract_address = receipt.contract_address.unwrap();

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

        (tx, contract_address)
    }

    // Test helper to create a contract call transaction
    fn create_contract_call_tx(contract_address: Address) -> Tx {
        // ERC20 transfer function call: transfer(address to, uint256 amount)
        // Function selector: 0xa9059cbb
        // to: 0x742d35Cc6634C0532925a3b8D401B1C4029Ee7A7 (padded to 32 bytes)
        // amount: 1000000000000000000 (1 token with 18 decimals, padded to 32 bytes)
        let call_data = transferCall::new((
            address!("0x742d35Cc6634C0532925a3b8D401B1C4029Ee7A7"),
            U256::from(1000000000000000000u64),
        ));

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
