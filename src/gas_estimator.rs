use crate::utils::calculate_calldata_cost;
use alloy::{
    eips::BlockId,
    providers::{Provider, ProviderBuilder},
};
use revm::{
    context::{transaction::AccessList, tx::TxEnvBuilder},
    database::{CacheDB, EmptyDB},
    primitives::{alloy_primitives::U64, Address, Bytes, TxKind, U256},
    state::AccountInfo,
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

            // Add balance to the caller account to avoid LackOfFundForMaxFee error
            let caller = tx_params.from.unwrap();
            let balance = U256::from(10u128.pow(18) * 1000); // 1000 ETH
            cache_db.insert_account_info(
                caller,
                AccountInfo {
                    balance,
                    nonce: tx_params.nonce.unwrap_or(0),
                    code_hash: revm::primitives::KECCAK_EMPTY,
                    code: None,
                },
            );

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
        println!("execution_cost: {}", execution_cost);

        // Calculate data cost (calldata)
        let data_cost = if tx_params.data.is_some() && self.is_contract(tx_params.to).await.unwrap()
        {
            calculate_calldata_cost(tx_params.data.as_ref().unwrap())
        } else {
            0
        };

        // Calculate contract creation cost
        let contract_creation_cost = if tx_params.to.is_none() {
            let mut cache_db = CacheDB::new(EmptyDB::default());

            // Add balance to the caller account to avoid LackOfFundForMaxFee error
            let caller = tx_params.from.unwrap();
            let balance = U256::from(10u128.pow(18) * 1000); // 1000 ETH
            cache_db.insert_account_info(
                caller,
                AccountInfo {
                    balance,
                    nonce: tx_params.nonce.unwrap_or(0),
                    code_hash: revm::primitives::KECCAK_EMPTY,
                    code: None,
                },
            );

            let mut evm = Context::mainnet().with_db(cache_db).build_mainnet();
            let tx_evm = TxEnvBuilder::new()
                .caller(caller)
                .kind(TxKind::Create) // Creating a contract
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
                    result.result.gas_used() as u128
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
//     // sol! {
//     //         function getReserves() external view returns (uint112 reserve0, uint112 reserve1, uint32 blockTimestampLast);
//     //     }

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{address, bytes};

    // Test helper to create a basic transaction
    fn create_basic_tx() -> Tx {
        Tx {
            from: Some(address!("0x742d35Cc6634C0532925a3b8D401B1C4029Ee7A7")),
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
        // ERC20 contract bytecode (simplified version)
        let bytecode = bytes!("608060405234801561001057600080fd5b50336000806101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff1602179055506000809054906101000a900473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16600073ffffffffffffffffffffffffffffffffffffffff167f8be0079c531659141344cd1fd0a4f28419497f9722a3daafe3b4186f6b6457e060405160405180910390a360405161185f38038061185f8339818101604052810190610107919061023e565b80600081905550610116610137565b600081905550610134336000543360405180602001604052806000815250610163565b50565b60003073ffffffffffffffffffffffffffffffffffffffff163190508091505090565b600081600260008673ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff168152602001908152602001600020600082825461019b9190610308565b925050819055508373ffffffffffffffffffffffffffffffffffffffff168573ffffffffffffffffffffffffffffffffffffffff167fddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef856040516101ff919061033e565b60405180910390a35050505050565b600080fd5b6000819050919050565b61022681610213565b811461023157600080fd5b50565b6000815190506102438161021d565b92915050565b60006020828403121561025f5761025e61020e565b5b600061026d84828501610234565b91505092915050565b7f4e487b7100000000000000000000000000000000000000000000000000000000600052601160045260246000fd5b60006102b082610213565b91506102bb83610213565b9250827fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff038211156102f0576102ef610276565b5b828201905092915050565b60006103068261021356fe");

        Tx {
            from: Some(address!("0x742d35Cc6634C0532925a3b8D401B1C4029Ee7A7")),
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
        }
    }

    // Test helper to create a contract call transaction
    fn create_contract_call_tx() -> Tx {
        // ERC20 transfer function call: transfer(address to, uint256 amount)
        // Function selector: 0xa9059cbb
        // to: 0x742d35Cc6634C0532925a3b8D401B1C4029Ee7A7 (padded to 32 bytes)
        // amount: 1000000000000000000 (1 token with 18 decimals, padded to 32 bytes)
        let call_data = bytes!("a9059cbb000000000000000000000000742d35cc6634c0532925a3b8d401b1c4029ee7a70000000000000000000000000000000000000000000000000de0b6b3a7640000");

        Tx {
            from: Some(address!("0x742d35Cc6634C0532925a3b8D401B1C4029Ee7A7")),
            to: Some(address!("0xA0b86a33E6441D0ade1CBC8D62F78D6f4a8e5c5F")), // Mock ERC20 contract address
            value: U256::ZERO,
            data: Some(call_data),
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

    // Integration-style tests that would work with a real RPC (commented out for unit testing)
    #[tokio::test]
    async fn test_estimate_gas_simple_transfer() {
        let rpc_url = std::env::var("ETH_RPC_URL").unwrap();
        let estimator = GasEstimator::new(&rpc_url).await.unwrap();
        let tx = create_basic_tx();

        let result = estimator.estimate_gas(tx).await;
        assert!(result.is_ok());

        let estimate = result.unwrap();
        assert!(estimate.estimated_gas >= 21000);
        assert!(estimate.gas_price > 0);
    }

    #[tokio::test]
    async fn test_estimate_gas_contract_deployment() {
        let rpc_url = std::env::var("ETH_RPC_URL").unwrap();
        let estimator = GasEstimator::new(&rpc_url).await.unwrap();
        let tx = create_contract_deployment_tx();

        let result = estimator.estimate_gas(tx).await;
        assert!(result.is_ok());

        let estimate = result.unwrap();
        assert!(estimate.estimated_gas >= 21000);
        assert!(estimate.gas_price > 0);
    }

    #[tokio::test]
    async fn test_estimate_gas_contract_call() {
        let rpc_url = std::env::var("ETH_RPC_URL").unwrap();
        let estimator = GasEstimator::new(&rpc_url).await.unwrap();
        let tx = create_contract_call_tx();

        let result = estimator.estimate_gas(tx).await;
        assert!(result.is_ok());

        let estimate = result.unwrap();
        assert!(estimate.estimated_gas >= 21000);
        assert!(estimate.gas_price > 0);
    }

    #[tokio::test]
    async fn test_get_network_gas_info() {
        let rpc_url = std::env::var("ETH_RPC_URL").unwrap();
        let estimator = GasEstimator::new(&rpc_url).await.unwrap();

        let result = estimator.get_network_gas_info().await;
        assert!(result.is_ok());

        let info = result.unwrap();
        assert!(info.current_gas_price > 0);
        assert!(info.block_utilization >= 0.0 && info.block_utilization <= 100.0);
    }
}
