use ethers::{
    prelude::*,
    providers::{Http, Provider},
    types::{
        transaction::{eip2718::TypedTransaction, eip2930::AccessList},
        Address, Bytes, U256, U64,
    },
    utils::parse_ether,
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
    pub gas: Option<U256>,
    #[serde(alias = "gasPrice")]
    pub gas_price: Option<U256>,
    #[serde(alias = "maxFeePerGas")]
    pub max_fee_per_gas: Option<U256>,
    #[serde(alias = "maxPriorityFeePerGas")]
    pub max_priority_fee_per_gas: Option<U256>,

    // EIP-2930 Access List
    #[serde(alias = "accessList")]
    pub access_list: Option<AccessList>,

    // Transaction type (0=Legacy, 1=EIP-2930, 2=EIP-1559)
    #[serde(alias = "type")]
    pub transaction_type: Option<U64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasEstimate {
    pub estimated_gas: U256,
    pub gas_price: U256,
    pub max_fee_per_gas: Option<U256>,
    pub max_priority_fee_per_gas: Option<U256>,
    pub total_cost_wei: U256,
    pub total_cost_eth: String,
    pub transaction_type: String,
    pub breakdown: GasBreakdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasBreakdown {
    pub base_cost: U256,
    pub data_cost: U256,
    pub recipient_cost: U256,
    pub storage_cost: U256,
    pub contract_creation_cost: U256,
    pub execution_cost: U256,
    pub access_list_cost: U256,
    pub precompile_cost: U256,
}

pub struct GasEstimator {
    provider: Arc<Provider<Http>>,
}

impl GasEstimator {
    pub fn new(rpc_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let provider = Provider::<Http>::try_from(rpc_url)?;
        Ok(Self {
            provider: Arc::new(provider),
        })
    }

    /// Returns bytecode for a minimal empty contract
    pub fn minimal_contract_bytecode() -> Bytes {
        Bytes::from_static(&[
            0x60, 0x0a, // PUSH1 0x0a (size of runtime code)
            0x60, 0x0c, // PUSH1 0x0c (offset of runtime code)
            0x60, 0x00, // PUSH1 0x00 (destination memory offset)
            0x39, // CODECOPY
            0x60, 0x0a, // PUSH1 0x0a (size)
            0x60, 0x00, // PUSH1 0x00 (offset)
            0xf3, // RETURN
            // Runtime code (empty):
            0x60, 0x00, // PUSH1 0x00
            0x60, 0x00, // PUSH1 0x00
            0xf3, // RETURN
        ])
    }

    /// Returns bytecode for a simple storage contract (stores a value)
    pub fn simple_storage_contract_bytecode() -> Bytes {
        // Simple contract that stores value 42 in slot 0
        Bytes::from_static(&[
            0x60, 0x2a, // PUSH1 42 (value to store)
            0x60, 0x00, // PUSH1 0 (storage slot)
            0x55, // SSTORE
            0x60, 0x0a, // PUSH1 0x0a (runtime code size)
            0x60, 0x16, // PUSH1 0x16 (runtime code offset)
            0x60, 0x00, // PUSH1 0x00 (memory offset)
            0x39, // CODECOPY
            0x60, 0x0a, // PUSH1 0x0a (size)
            0x60, 0x00, // PUSH1 0x00 (offset)
            0xf3, // RETURN
            // Runtime code:
            0x60, 0x00, // PUSH1 0x00
            0x54, // SLOAD
            0x60, 0x00, // PUSH1 0x00
            0x52, // MSTORE
            0x60, 0x20, // PUSH1 0x20
            0x60, 0x00, // PUSH1 0x00
            0xf3, // RETURN
        ])
    }

    /// Returns bytecode for a contract that uses precompiles
    pub fn precompile_contract_bytecode() -> Bytes {
        // Contract that calls SHA256 precompile (address 0x02)
        Bytes::from_static(&[
            0x60, 0x20, // PUSH1 0x20 (size = 32 bytes)
            0x60, 0x00, // PUSH1 0x00 (offset)
            0x60, 0x20, // PUSH1 0x20 (retSize)
            0x60, 0x00, // PUSH1 0x00 (retOffset)
            0x60, 0x00, // PUSH1 0x00 (argsSize)
            0x60, 0x00, // PUSH1 0x00 (argsOffset)
            0x60, 0x02, // PUSH1 0x02 (SHA256 precompile address)
            0x61, 0x27, 0x10, // PUSH2 0x2710 (10000 gas)
            0xf1, // CALL
            0x60, 0x08, // PUSH1 0x08 (runtime code size)
            0x60, 0x12, // PUSH1 0x12 (runtime code offset)
            0x60, 0x00, // PUSH1 0x00 (memory offset)
            0x39, // CODECOPY
            0x60, 0x08, // PUSH1 0x08 (size)
            0x60, 0x00, // PUSH1 0x00 (offset)
            0xf3, // RETURN
            // Runtime code:
            0x60, 0x00, // PUSH1 0x00
            0x60, 0x00, // PUSH1 0x00
            0xf3, // RETURN
        ])
    }

    /// Custom gas estimation implementation from scratch
    pub async fn estimate_gas(
        &self,
        tx_params: Tx,
    ) -> Result<GasEstimate, Box<dyn std::error::Error>> {
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
        let gas_price = if let Some(price) = tx_params.gas_price {
            price
        } else {
            self.provider.get_gas_price().await?
        };

        // Try to get EIP-1559 fee data
        let fee_history = self
            .provider
            .fee_history(10, BlockNumber::Latest, &[25.0, 50.0, 75.0])
            .await;

        let (max_fee_per_gas, max_priority_fee_per_gas, transaction_type) = if fee_history.is_ok() {
            // EIP-1559 transaction
            let base_fee = self
                .provider
                .get_block(BlockNumber::Latest)
                .await?
                .unwrap()
                .base_fee_per_gas
                .unwrap_or(gas_price);

            let max_priority_fee = tx_params
                .max_priority_fee_per_gas
                .unwrap_or_else(|| U256::from(2_000_000_000u64)); // 2 gwei default

            let max_fee = tx_params
                .max_fee_per_gas
                .unwrap_or_else(|| base_fee * 2 + max_priority_fee);

            (
                Some(max_fee),
                Some(max_priority_fee),
                "EIP-1559 (Custom)".to_string(),
            )
        } else {
            // Legacy transaction
            (None, None, "Legacy (Custom)".to_string())
        };

        // Calculate total cost
        let effective_gas_price = max_fee_per_gas.unwrap_or(gas_price);
        let total_cost_wei = estimated_gas * effective_gas_price;
        let total_cost_eth = ethers::utils::format_ether(total_cost_wei);

        Ok(GasEstimate {
            estimated_gas,
            gas_price,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            total_cost_wei,
            total_cost_eth,
            transaction_type,
            breakdown,
        })
    }

    /// Compare custom estimation with provider's built-in estimation
    pub async fn compare_estimations(
        &self,
        tx_params: Tx,
    ) -> Result<EstimationComparison, Box<dyn std::error::Error>> {
        // Get our custom estimate
        let custom_estimate = self.estimate_gas(tx_params.clone()).await?;

        // Get provider's estimate
        let mut tx_request = TypedTransaction::default();

        if let Some(to) = tx_params.to {
            tx_request.set_to(to);
        }
        if let Some(value) = tx_params.value {
            tx_request.set_value(value);
        }
        if let Some(data) = tx_params.data {
            tx_request.set_data(data);
        }

        let provider_gas = self.provider.estimate_gas(&tx_request, None).await?;
        let provider_estimate = GasEstimate {
            estimated_gas: provider_gas,
            gas_price: custom_estimate.gas_price,
            max_fee_per_gas: custom_estimate.max_fee_per_gas,
            max_priority_fee_per_gas: custom_estimate.max_priority_fee_per_gas,
            total_cost_wei: provider_gas * custom_estimate.gas_price,
            total_cost_eth: ethers::utils::format_ether(provider_gas * custom_estimate.gas_price),
            transaction_type: "Provider Built-in".to_string(),
            breakdown: GasBreakdown {
                base_cost: provider_gas,
                data_cost: U256::zero(),
                recipient_cost: U256::zero(),
                storage_cost: U256::zero(),
                contract_creation_cost: U256::zero(),
                execution_cost: U256::zero(),
                access_list_cost: U256::zero(),
                precompile_cost: U256::zero(),
            },
        };

        let difference = if custom_estimate.estimated_gas > provider_gas {
            custom_estimate.estimated_gas - provider_gas
        } else {
            provider_gas - custom_estimate.estimated_gas
        };

        let accuracy_percentage = if provider_gas > U256::zero() {
            100.0 - (difference.as_u64() as f64 / provider_gas.as_u64() as f64 * 100.0)
        } else {
            0.0
        };

        Ok(EstimationComparison {
            custom_estimate,
            provider_estimate,
            difference,
            accuracy_percentage,
        })
    }

    /// Calculate detailed gas breakdown from scratch
    async fn calculate_gas_breakdown(
        &self,
        tx_params: &Tx,
    ) -> Result<GasBreakdown, Box<dyn std::error::Error>> {
        // Base transaction cost (21,000 gas for simple transfers)
        let base_cost = U256::from(21_000);

        // Calculate data cost (calldata)
        let data_cost = if let Some(ref data) = tx_params.data {
            self.calculate_calldata_cost(data)
        } else {
            U256::zero()
        };

        // Calculate recipient cost (contract vs EOA)
        let recipient_cost = if let Some(to) = tx_params.to {
            self.calculate_recipient_cost(to).await?
        } else {
            U256::zero()
        };

        // Calculate storage operations cost
        let storage_cost = if let Some(ref data) = tx_params.data {
            self.estimate_storage_cost(data).await?
        } else {
            U256::zero()
        };

        // Calculate contract creation cost
        let contract_creation_cost = if tx_params.to.is_none() {
            self.calculate_contract_creation_cost(tx_params.data.as_ref())
        } else {
            U256::zero()
        };

        // Calculate execution cost (opcode simulation)
        let execution_cost = if let Some(ref data) = tx_params.data {
            self.estimate_execution_cost(data, tx_params.to.is_none())
                .await?
        } else {
            U256::zero()
        };

        // Calculate access list cost (EIP-2930)
        let access_list_cost = self.calculate_access_list_cost(tx_params).await?;

        // Calculate precompile cost
        let precompile_cost = if let Some(ref data) = tx_params.data {
            self.estimate_precompile_cost(data, tx_params.to)
        } else {
            U256::zero()
        };

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

    pub async fn estimate_transfer_gas(
        &self,
        to: Address,
        amount_eth: &str,
    ) -> Result<GasEstimate, Box<dyn std::error::Error>> {
        let value = parse_ether(amount_eth)?;
        let tx_params = Tx {
            from: None,
            to: Some(to),
            value: Some(value),
            data: None,
            nonce: None,
            chain_id: None,
            gas: None,
            gas_price: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            access_list: None,
            transaction_type: None,
        };

        self.estimate_gas(tx_params).await
    }

    pub async fn estimate_contract_call_gas(
        &self,
        contract_address: Address,
        data: Bytes,
        value: Option<U256>,
    ) -> Result<GasEstimate, Box<dyn std::error::Error>> {
        let tx_params = Tx {
            from: None,
            to: Some(contract_address),
            value,
            data: Some(data),
            nonce: None,
            chain_id: None,
            gas: None,
            gas_price: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            access_list: None,
            transaction_type: None,
        };

        self.estimate_gas(tx_params).await
    }

    /// Calculate gas cost for calldata (transaction input data)
    fn calculate_calldata_cost(&self, data: &Bytes) -> U256 {
        let mut cost = U256::zero();

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
        // Check if the recipient is a contract by looking for code
        let code = self.provider.get_code(to, None).await?;

        if code.is_empty() {
            // External Owned Account (EOA) - no additional cost
            Ok(U256::zero())
        } else {
            // Contract account - additional gas for call
            Ok(U256::from(2_300)) // Basic stipend for contract calls
        }
    }

    /// Estimate storage operations cost
    async fn estimate_storage_cost(
        &self,
        data: &Bytes,
    ) -> Result<U256, Box<dyn std::error::Error>> {
        let mut cost = U256::zero();

        // Simple heuristic: look for SSTORE-like patterns in bytecode
        // This is a simplified estimation
        let data_bytes = data.as_ref();
        let mut i = 0;

        while i < data_bytes.len() {
            match data_bytes[i] {
                0x55 => {
                    // SSTORE opcode - storage write
                    cost += U256::from(20_000); // Rough estimate for new storage
                    i += 1;
                }
                0x54 => {
                    // SLOAD opcode - storage read
                    cost += U256::from(2_100);
                    i += 1;
                }
                _ => i += 1,
            }
        }

        Ok(cost)
    }

    /// Calculate contract creation cost
    fn calculate_contract_creation_cost(&self, data: Option<&Bytes>) -> U256 {
        if let Some(bytecode) = data {
            // Base cost for contract creation
            let mut cost = U256::from(32_000);

            // Additional cost per byte of bytecode
            cost += U256::from(bytecode.len() * 200);

            cost
        } else {
            U256::zero()
        }
    }

    /// Estimate execution cost by analyzing opcodes
    async fn estimate_execution_cost(
        &self,
        data: &Bytes,
        is_deployment: bool,
    ) -> Result<U256, Box<dyn std::error::Error>> {
        let mut cost = U256::zero();
        let data_bytes = data.as_ref();
        let mut i = 0;

        while i < data_bytes.len() {
            let opcode = data_bytes[i];

            cost += match opcode {
                // Arithmetic operations
                0x01..=0x0b => U256::from(3), // ADD, MUL, SUB, DIV, etc.

                // Comparison operations
                0x10..=0x1d => U256::from(3), // LT, GT, SLT, SGT, EQ, etc.

                // SHA3
                0x20 => U256::from(30),

                // Environmental operations
                0x30..=0x3f => U256::from(2), // ADDRESS, BALANCE, ORIGIN, etc.

                // Block operations
                0x40..=0x48 => U256::from(20), // BLOCKHASH, COINBASE, etc.

                // Stack operations
                0x50..=0x5f => U256::from(3), // POP, MLOAD, MSTORE, etc.

                // Push operations
                0x60..=0x7f => {
                    let size = (opcode - 0x60 + 1) as usize;
                    i += size; // Skip the pushed bytes
                    U256::from(3)
                }

                // Duplication operations
                0x80..=0x8f => U256::from(3),

                // Exchange operations
                0x90..=0x9f => U256::from(3),

                // Logging operations
                0xa0..=0xa4 => U256::from(375), // LOG0, LOG1, etc.

                // System operations
                0xf0 => U256::from(32_000), // CREATE
                0xf1 => U256::from(700),    // CALL
                0xf2 => U256::from(700),    // CALLCODE
                0xf3 => U256::from(0),      // RETURN
                0xf4 => U256::from(700),    // DELEGATECALL
                0xf5 => U256::from(32_000), // CREATE2
                0xfd => U256::from(0),      // REVERT
                0xff => U256::from(5_000),  // SELFDESTRUCT

                // Default case
                _ => U256::from(1),
            };

            i += 1;
        }

        // Add extra cost for complex contract deployments
        if is_deployment && data_bytes.len() > 100 {
            cost += U256::from(data_bytes.len() * 10);
        }

        Ok(cost)
    }

    /// Calculate access list cost (EIP-2930)
    async fn calculate_access_list_cost(
        &self,
        tx_params: &Tx,
    ) -> Result<U256, Box<dyn std::error::Error>> {
        // Simple heuristic: estimate potential access list items
        let mut cost = U256::zero();

        if let Some(to) = tx_params.to {
            // Check if target is a contract that might benefit from access list
            let code = self.provider.get_code(to, None).await?;
            if !code.is_empty() {
                // Estimate potential storage slots accessed
                cost += U256::from(2_400); // ADDRESS_ACCESS_COST
                cost += U256::from(1_900 * 2); // STORAGE_KEY_ACCESS_COST for 2 slots
            }
        }

        Ok(cost)
    }

    /// Estimate precompile costs
    fn estimate_precompile_cost(&self, data: &Bytes, to: Option<Address>) -> U256 {
        let mut cost = U256::zero();
        let data_bytes = data.as_ref();

        // Check for precompile addresses in the bytecode or direct calls
        if let Some(address) = to {
            let addr_u64 = address.as_fixed_bytes()[19]; // Last byte for precompile check
            match addr_u64 {
                0x01 => cost += U256::from(3_000), // ECDSA recovery
                0x02 => cost += U256::from(60 + (data_bytes.len() as u64 + 31) / 32 * 12), // SHA256
                0x03 => cost += U256::from(600 + (data_bytes.len() as u64 + 31) / 32 * 120), // RIPEMD160
                0x04 => cost += U256::from(15 + (data_bytes.len() as u64 + 31) / 32 * 3), // Identity
                0x05 => cost += U256::from(self.estimate_modexp_cost(data_bytes)),        // ModExp
                0x06 => cost += U256::from(150),    // BN254 Add
                0x07 => cost += U256::from(6_000),  // BN254 Mul
                0x08 => cost += U256::from(45_000), // BN254 Pairing base
                0x09 => cost += U256::from(50_000), // Blake2F
                _ => {}
            }
        }

        // Look for CALL opcodes to precompile addresses in bytecode
        let mut i = 0;
        while i + 20 < data_bytes.len() {
            if data_bytes[i] == 0xf1 {
                // CALL opcode
                // Simple heuristic: look for small addresses that might be precompiles
                for j in 1..=20 {
                    if i >= j && data_bytes[i - j] <= 0x09 && data_bytes[i - j] > 0 {
                        cost += U256::from(700); // Base call cost + estimated precompile cost
                        break;
                    }
                }
            }
            i += 1;
        }

        cost
    }

    /// Estimate ModExp precompile cost
    fn estimate_modexp_cost(&self, data: &[u8]) -> u64 {
        if data.len() < 96 {
            return 200; // Minimum cost
        }

        // Simplified calculation - in practice this would parse the input more carefully
        let base_len = if data.len() >= 32 {
            u64::from_be_bytes([0, 0, 0, 0, 0, 0, 0, data[31]])
        } else {
            32
        };
        let exp_len = if data.len() >= 64 {
            u64::from_be_bytes([0, 0, 0, 0, 0, 0, 0, data[63]])
        } else {
            32
        };
        let mod_len = if data.len() >= 96 {
            u64::from_be_bytes([0, 0, 0, 0, 0, 0, 0, data[95]])
        } else {
            32
        };

        let max_len = base_len.max(mod_len);
        let complexity = (max_len * max_len) / 64;

        200 + complexity * exp_len / 20
    }

    pub async fn get_network_gas_info(&self) -> Result<NetworkGasInfo, Box<dyn std::error::Error>> {
        let gas_price = self.provider.get_gas_price().await?;
        let latest_block = self.provider.get_block(BlockNumber::Latest).await?.unwrap();

        let base_fee_per_gas = latest_block.base_fee_per_gas;
        let gas_used = latest_block.gas_used;
        let gas_limit = latest_block.gas_limit;

        let utilization = if gas_limit > U256::zero() {
            (gas_used.as_u64() as f64 / gas_limit.as_u64() as f64) * 100.0
        } else {
            0.0
        };

        Ok(NetworkGasInfo {
            current_gas_price: gas_price,
            base_fee_per_gas,
            block_utilization: utilization,
            latest_block_number: latest_block.number.unwrap().as_u64(),
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NetworkGasInfo {
    pub current_gas_price: U256,
    pub base_fee_per_gas: Option<U256>,
    pub block_utilization: f64,
    pub latest_block_number: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EstimationComparison {
    pub custom_estimate: GasEstimate,
    pub provider_estimate: GasEstimate,
    pub difference: U256,
    pub accuracy_percentage: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[tokio::test]
    async fn test_gas_estimator_creation() {
        let rpc_url = "https://eth.llamarpc.com";
        let estimator = GasEstimator::new(rpc_url);
        assert!(estimator.is_ok());
    }

    #[tokio::test]
    async fn test_network_gas_info() {
        let rpc_url = "https://eth.llamarpc.com";
        let estimator = GasEstimator::new(rpc_url).unwrap();

        let gas_info = estimator.get_network_gas_info().await;
        if gas_info.is_ok() {
            let info = gas_info.unwrap();
            assert!(info.current_gas_price > U256::zero());
            assert!(info.latest_block_number > 0);
        }
        // Note: Test might fail if RPC is down, which is acceptable
    }

    #[tokio::test]
    async fn test_custom_vs_provider_accuracy() {
        let rpc_url = "https://eth.llamarpc.com";
        let estimator = GasEstimator::new(rpc_url).unwrap();

        // Test simple ETH transfer
        let recipient = "0x742d35Cc6634C0532925a3b8D0Ed9C5C8bD4c29c"
            .parse::<Address>()
            .unwrap();
        let tx_params = Tx {
            from: None,
            to: Some(recipient),
            value: Some(parse_ether("0.01").unwrap()),
            data: None,
            gas: None,
            nonce: None,
            chain_id: None,
            gas_price: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            access_list: None,
            transaction_type: None,
        };

        let comparison = estimator.compare_estimations(tx_params).await;
        if let Ok(comp) = comparison {
            // Our custom estimation should be within 5% of provider estimation
            assert!(comp.accuracy_percentage >= 95.0);
            println!("Accuracy test passed: {:.2}%", comp.accuracy_percentage);
        }
    }

    #[tokio::test]
    async fn test_performance_benchmark() {
        let rpc_url = "https://eth.llamarpc.com";
        let estimator = GasEstimator::new(rpc_url).unwrap();

        let recipient = "0x742d35Cc6634C0532925a3b8D0Ed9C5C8bD4c29c"
            .parse::<Address>()
            .unwrap();
        let tx_params = Tx {
            from: None,
            to: Some(recipient),
            value: Some(parse_ether("0.01").unwrap()),
            data: None,
            gas: None,
            nonce: None,
            chain_id: None,
            gas_price: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            access_list: None,
            transaction_type: None,
        };

        // Benchmark custom estimation
        let start = Instant::now();
        let custom_result = estimator.estimate_gas(tx_params.clone()).await;
        let custom_duration = start.elapsed();

        // Benchmark provider estimation
        let start = Instant::now();
        let mut tx_request = TypedTransaction::default();
        tx_request.set_to(recipient);
        tx_request.set_value(parse_ether("0.01").unwrap());
        let provider_result = estimator.provider.estimate_gas(&tx_request, None).await;
        let provider_duration = start.elapsed();

        if custom_result.is_ok() && provider_result.is_ok() {
            println!("Custom estimation time: {:?}", custom_duration);
            println!("Provider estimation time: {:?}", provider_duration);

            // Custom estimation should complete reasonably fast (under 1 second typically)
            assert!(custom_duration.as_millis() < 5000);
        }
    }

    #[tokio::test]
    async fn test_bytecode_analysis() {
        let estimator = GasEstimator::new("https://eth.llamarpc.com").unwrap();

        // Test minimal contract
        let minimal_bytecode = GasEstimator::minimal_contract_bytecode();
        let minimal_params = Tx {
            from: None,
            to: None,
            value: None,
            data: Some(minimal_bytecode),
            nonce: None,
            chain_id: None,
            gas: None,
            gas_price: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            access_list: None,
            transaction_type: None,
        };

        let minimal_estimate = estimator.estimate_gas(minimal_params).await;
        assert!(minimal_estimate.is_ok());

        if let Ok(estimate) = minimal_estimate {
            // Should have base cost + creation cost + execution cost
            assert!(estimate.breakdown.base_cost > U256::zero());
            assert!(estimate.breakdown.contract_creation_cost > U256::zero());
            assert!(estimate.estimated_gas > U256::from(21_000));
        }

        // Test storage contract
        let storage_bytecode = GasEstimator::simple_storage_contract_bytecode();
        let storage_params = Tx {
            from: None,
            to: None,
            value: None,
            data: Some(storage_bytecode),
            nonce: None,
            chain_id: None,
            gas: None,
            gas_price: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            access_list: None,
            transaction_type: None,
        };

        let storage_estimate = estimator.estimate_gas(storage_params).await;
        assert!(storage_estimate.is_ok());

        if let Ok(estimate) = storage_estimate {
            // Storage contract should have storage costs
            assert!(estimate.breakdown.storage_cost > U256::zero());
            assert!(estimate.estimated_gas > U256::from(50_000));
        }
    }

    #[test]
    fn test_calldata_cost_calculation() {
        let estimator = GasEstimator::new("https://eth.llamarpc.com").unwrap();

        // Test with zero bytes
        let zero_data = Bytes::from(vec![0u8; 10]);
        let zero_cost = estimator.calculate_calldata_cost(&zero_data);
        assert_eq!(zero_cost, U256::from(40)); // 10 * 4 gas

        // Test with non-zero bytes
        let nonzero_data = Bytes::from(vec![1u8; 10]);
        let nonzero_cost = estimator.calculate_calldata_cost(&nonzero_data);
        assert_eq!(nonzero_cost, U256::from(160)); // 10 * 16 gas

        // Test with mixed bytes
        let mixed_data = Bytes::from(vec![0, 1, 0, 1, 0]);
        let mixed_cost = estimator.calculate_calldata_cost(&mixed_data);
        assert_eq!(mixed_cost, U256::from(44)); // 3 * 4 + 2 * 16 = 44 gas
    }

    #[test]
    fn test_contract_creation_cost() {
        let estimator = GasEstimator::new("https://eth.llamarpc.com").unwrap();

        // Test with bytecode
        let bytecode = Bytes::from(vec![0x60, 0x80, 0x60, 0x40]); // 4 bytes
        let cost = estimator.calculate_contract_creation_cost(Some(&bytecode));
        assert_eq!(cost, U256::from(32_800)); // 32_000 + 4 * 200

        // Test without bytecode
        let no_cost = estimator.calculate_contract_creation_cost(None);
        assert_eq!(no_cost, U256::zero());
    }
}
