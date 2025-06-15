use alloy::{
    eips::BlockId,
    primitives::BlockNumber,
    providers::{Provider, ProviderBuilder},
};
// use ethers::{
//     prelude::*,
//     providers::{Http, Provider},
//     types::{transaction::eip2930::AccessList, Address, Bytes, U256, U64},
// };
use revm::{
    context::{transaction::AccessList, TxEnv},
    database::{CacheDB, EmptyDB},
    primitives::{alloy_primitives::U64, Address, Bytes, TxKind, U256},
    Context, ExecuteEvm, MainBuilder, MainContext,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tx {
    // Standard transaction fields
    pub from: Option<Address>,
    pub to: Option<Address>,
    pub value: Option<U256>,
    #[serde(alias = "input")]
    pub data: Option<Bytes>,
    pub nonce: Option<U256>,
    #[serde(alias = "chainId")]
    pub chain_id: Option<U64>,

    // Gas fields - using standard names
    #[serde(alias = "gas_limit")]
    pub gas: Option<u64>,
    #[serde(alias = "gasPrice")]
    pub gas_price: Option<u64>,
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
    pub max_fee_per_gas: Option<u128>,
    pub max_priority_fee_per_gas: Option<u128>,
    pub total_cost_wei: u128,
    pub total_cost_eth: String,
    pub transaction_type: String,
    pub breakdown: GasBreakdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasBreakdown {
    pub base_cost: u128,
    pub data_cost: u128,
    pub recipient_cost: u128,
    pub storage_cost: u128,
    pub contract_creation_cost: u128,
    pub execution_cost: u128,
    pub access_list_cost: u128,
    pub precompile_cost: u128,
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
        let provider = ProviderBuilder::new().connect(rpc_url).await?;

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
            + breakdown.data_cost
            + breakdown.recipient_cost
            + breakdown.storage_cost
            + breakdown.contract_creation_cost
            + breakdown.execution_cost
            + breakdown.access_list_cost;

        // Get current gas price information
        let gas_price = provider.get_gas_price().await?;

        let (max_fee_per_gas, max_priority_fee_per_gas, transaction_type) = {
            // EIP-1559 transaction
            let block = provider.get_block(BlockId::latest()).await?.unwrap();
            let base_fee = block.header.base_fee_per_gas.unwrap();

            let max_priority_fee = tx_params
                .max_priority_fee_per_gas
                .unwrap_or_else(|| 2_000_000_000); // 2 gwei default

            let max_fee = tx_params
                .max_fee_per_gas
                .unwrap_or_else(|| base_fee as u128 * 2 + max_priority_fee);

            (
                Some(max_fee),
                Some(max_priority_fee),
                "EIP-1559 (Custom)".to_string(),
            )
        };

        // Calculate total cost
        let effective_gas_price = max_fee_per_gas.unwrap_or(gas_price);
        let total_cost_wei = estimated_gas * effective_gas_price;

        Ok(GasEstimate {
            estimated_gas,
            gas_price,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            total_cost_wei,
            total_cost_eth: "".to_string(),
            transaction_type,
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

        let is_contract_call =
            self.is_contract(tx_params.to).await.unwrap() && tx_params.data.is_some();
        let is_contract_creation = tx_params.to.is_none();
        let is_eoa_call = tx_params.data.is_none();

        let contract_exe_cost = if is_contract_call {
            let mut cache_db = CacheDB::new(EmptyDB::default());
            let mut evm = Context::mainnet().with_db(cache_db).build_mainnet();
            // Execute transaction without writing to the DB
            let result = evm
                .transact_finalize(TxEnv {
                    // fill in missing bits of env struct
                    // change that to whatever caller you want to be
                    caller: tx_params.from.unwrap(),
                    // account you want to transact with
                    kind: TxKind::Call(tx_params.to.unwrap()),
                    // calldata formed via abigen
                    data: tx_params.data.clone().unwrap(),
                    // transaction value in wei
                    value: U256::from(0),
                    ..Default::default()
                })
                .unwrap();
            result.result.gas_used();
        };

        // Calculate data cost (calldata)
        let data_cost = if let Some(ref data) = tx_params.data {
            // self.calculate_calldata_cost(data)
            0
        } else {
            0
        };

        // Calculate recipient cost (contract vs EOA)
        let recipient_cost = if let Some(to) = tx_params.to {
            // self.calculate_recipient_cost(to).await?
            0
        } else {
            0
        };

        // Calculate contract creation cost
        let contract_creation_cost = if tx_params.to.is_none() {
            // self.calculate_contract_creation_cost(tx_params.data.as_ref())
            0
        } else {
            0
        };

        // ---------------------------------------- Not needed

        let storage_cost = if let Some(ref data) = tx_params.data {
            // self.estimate_storage_cost(data)
            0
        } else {
            0
        };

        let execution_cost = if let Some(data) = tx_params.data.clone() {
            // self.estimate_execution_cost(&data)
            0
        } else {
            0
        };

        // Calculate precompile cost
        let precompile_cost = if let Some(ref data) = tx_params.data {
            // self.estimate_precompile_cost(data, tx_params.to)
            0
        } else {
            0
        };
        // ----------------------------------------

        // Calculate access list cost (EIP-2930)
        let access_list_cost = 0;
        // self.calculate_access_list_cost(tx_params).await?;

        Ok(GasBreakdown {
            base_cost,
            data_cost,
            recipient_cost,
            storage_cost,
            contract_creation_cost,
            execution_cost,
            access_list_cost,
            precompile_cost,
        })
    }

    /// Calculate gas cost for calldata (transaction input data)
    fn calculate_calldata_cost(&self, data: &Bytes) -> U256 {
        let mut cost = U256::ZERO;

        for byte in data.iter() {
            if *byte == 0 {
                // Zero bytes cost 4 gas each
                cost += U256::from(4);
            } else {
                // Non-zero bytes cost 16 gas each
                cost += U256::from(16);
            }
        }

        cost
    }

    /// Calculate cost based on recipient (contract vs EOA)
    async fn calculate_recipient_cost(
        &self,
        to: Address,
    ) -> Result<U256, Box<dyn std::error::Error>> {
        let provider = ProviderBuilder::new().connect(&self.rpc_url).await.unwrap();
        // Check if the recipient is a contract by looking for code
        let code = provider.get_code_at(to).await?;

        if code.is_empty() {
            // External Owned Account (EOA) - no additional cost
            Ok(U256::ZERO)
        } else {
            // Contract account - additional gas for call
            Ok(U256::from(2_300)) // Basic stipend for contract calls
        }
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
    ) -> Result<U256, Box<dyn std::error::Error>> {
        let provider = ProviderBuilder::new().connect(&self.rpc_url).await.unwrap();
        // Simple heuristic: estimate potential access list items
        let mut cost = U256::ZERO;

        if let Some(to) = tx_params.to {
            // Check if target is a contract that might benefit from access list
            let code = provider.get_code_at(to).await?;
            if !code.is_empty() {
                // Estimate potential storage slots accessed
                cost += U256::from(2_400); // ADDRESS_ACCESS_COST
                cost += U256::from(1_900 * 2); // STORAGE_KEY_ACCESS_COST for 2 slots
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

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use ethers::types::transaction::eip2718::TypedTransaction;

//     #[derive(Debug, Serialize, Deserialize)]
//     pub struct EstimationComparison {
//         pub custom_estimate: GasEstimate,
//         pub provider_estimate: GasEstimate,
//         pub difference: U256,
//         pub accuracy_percentage: f64,
//     }

//     struct GasEstimatorHelper {
//         provider: Arc<Provider<Http>>,
//     }

//     impl GasEstimatorHelper {
//         pub fn new(rpc_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
//             let provider = Provider::<Http>::try_from(rpc_url)?;
//             Ok(Self {
//                 provider: Arc::new(provider),
//             })
//         }

//         /// Compare custom estimation with provider's built-in estimation
//         pub async fn get_provider_gas(&self, tx_params: Tx) -> U256 {
//             // Get provider's estimate
//             let mut tx_request = TypedTransaction::default();

//             if let Some(to) = tx_params.to {
//                 tx_request.set_to(to);
//             }
//             if let Some(value) = tx_params.value {
//                 tx_request.set_value(value);
//             }
//             if let Some(data) = tx_params.data {
//                 tx_request.set_data(data);
//             }

//             let provider_gas = self.provider.estimate_gas(&tx_request, None).await.unwrap();
//             provider_gas
//         }
//     }

//     // sol! {
//     //         function getReserves() external view returns (uint112 reserve0, uint112 reserve1, uint32 blockTimestampLast);
//     //     }
// }
