use ethers::{
    providers::{Http, Middleware, Provider},
    types::{Address, Bytes, U256},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};

use crate::gas_estimator::Tx;

/// Simple CALL gas estimator for predicting contract call costs
///
/// This module provides gas estimation for external contract calls with
/// special handling for well-known function patterns like ERC20 operations.
#[derive(Debug, Clone)]
pub struct CallEstimator {
    /// Provider for blockchain queries
    provider: Arc<Provider<Http>>,
    /// Cache of known function gas costs
    function_gas_cache: HashMap<[u8; 4], FunctionGasCost>,
    /// Base call costs
    base_call_cost: u64,
}

/// Gas cost breakdown for a function call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionGasCost {
    /// Base execution cost
    pub execution_gas: u64,
    /// Storage operations cost
    pub storage_gas: u64,
    /// Memory operations cost
    pub memory_gas: u64,
    /// Typical total gas used
    pub total_gas: u64,
    /// Function name for debugging
    pub function_name: String,
}

/// Result of call gas estimation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallGasEstimate {
    /// Target contract address
    pub target: Address,
    /// Function selector called
    pub selector: Option<[u8; 4]>,
    /// Estimated gas for the call
    pub estimated_gas: U256,
    /// Base call overhead
    pub call_overhead: U256,
    /// Function execution cost
    pub function_cost: U256,
    /// Whether target is a contract
    pub is_contract: bool,
    /// Function name if recognized
    pub function_name: Option<String>,
}

impl CallEstimator {
    /// Create a new CallEstimator instance
    pub fn new(provider: Arc<Provider<Http>>) -> Self {
        let mut estimator = Self {
            provider,
            function_gas_cache: HashMap::new(),
            base_call_cost: 700, // Base CALL opcode cost
        };

        // Initialize well-known function gas costs
        estimator.initialize_known_functions();
        estimator
    }

    /// Main entrypoint: estimate gas cost for a transaction's call
    pub async fn estimate_transaction_call_gas(
        &self,
        tx: &Tx,
    ) -> Result<CallGasEstimate, Box<dyn std::error::Error>> {
        // Extract target address - if None, this is a contract creation
        let target = match tx.to {
            Some(addr) => addr,
            None => {
                // Contract creation - return creation estimate
                return Ok(CallGasEstimate {
                    target: Address::zero(),
                    selector: None,
                    estimated_gas: U256::from(200000), // Typical contract creation cost
                    call_overhead: U256::from(32000),  // CREATE opcode base cost
                    function_cost: U256::from(168000), // Estimated init code execution
                    is_contract: false,
                    function_name: Some("contract_creation".to_string()),
                });
            }
        };

        // Extract call data
        let empty_bytes = Bytes::new();
        let call_data = tx.data.as_ref().unwrap_or(&empty_bytes);

        // Extract value
        let value = tx.value;

        self.estimate_call_gas(target, call_data, value).await
    }

    /// Estimate gas cost for a contract call (internal method)
    async fn estimate_call_gas(
        &self,
        target: Address,
        call_data: &Bytes,
        value: Option<U256>,
    ) -> Result<CallGasEstimate, Box<dyn std::error::Error>> {
        // Check if target is a contract
        let code = self.provider.get_code(target, None).await?;
        let is_contract = !code.is_empty();

        let mut estimate = CallGasEstimate {
            target,
            selector: None,
            estimated_gas: U256::zero(),
            call_overhead: U256::from(self.base_call_cost),
            function_cost: U256::zero(),
            is_contract,
            function_name: None,
        };

        if !is_contract {
            // Simple transfer to EOA
            estimate.estimated_gas = U256::from(21000);
            if value.unwrap_or(U256::zero()) > U256::zero() {
                estimate.call_overhead += U256::from(9000); // Value transfer cost
            }
            return Ok(estimate);
        }

        // Extract function selector if call data is long enough
        if call_data.len() >= 4 {
            let mut selector = [0u8; 4];
            selector.copy_from_slice(&call_data[0..4]);
            estimate.selector = Some(selector);

            // Check if we have cached gas cost for this function
            if let Some(function_cost) = self.function_gas_cache.get(&selector) {
                estimate.function_cost = U256::from(function_cost.total_gas);
                estimate.function_name = Some(function_cost.function_name.clone());
            } else {
                // Estimate based on call data complexity
                estimate.function_cost = self.estimate_unknown_function_gas(call_data);
            }
        } else {
            // Fallback function call
            estimate.function_cost = U256::from(2300);
            estimate.function_name = Some("fallback".to_string());
        }

        // Add value transfer cost if applicable
        if value.unwrap_or(U256::zero()) > U256::zero() {
            estimate.call_overhead += U256::from(9000);
        }

        // Calculate total estimated gas
        estimate.estimated_gas = estimate.call_overhead + estimate.function_cost;

        Ok(estimate)
    }

    /// Estimate gas for multiple calls (useful for multicall patterns)
    pub async fn estimate_multicall_gas(
        &self,
        calls: &[(Address, Bytes, Option<U256>)],
    ) -> Result<Vec<CallGasEstimate>, Box<dyn std::error::Error>> {
        let mut estimates = Vec::new();

        for (target, call_data, value) in calls {
            let estimate = self.estimate_call_gas(*target, call_data, *value).await?;
            estimates.push(estimate);
        }

        Ok(estimates)
    }

    /// Get total gas for multiple calls
    pub async fn estimate_total_multicall_gas(
        &self,
        calls: &[(Address, Bytes, Option<U256>)],
    ) -> Result<U256, Box<dyn std::error::Error>> {
        let estimates = self.estimate_multicall_gas(calls).await?;
        let total_gas = estimates
            .iter()
            .fold(U256::zero(), |acc, est| acc + est.estimated_gas);
        Ok(total_gas)
    }

    /// Initialize gas costs for well-known functions
    fn initialize_known_functions(&mut self) {
        // ERC20 Standard Functions

        // transfer(address,uint256) - 0xa9059cbb
        self.function_gas_cache.insert(
            [0xa9, 0x05, 0x9c, 0xbb],
            FunctionGasCost {
                execution_gas: 5000,
                storage_gas: 15000, // Two SSTORE operations (sender/receiver balance)
                memory_gas: 300,
                total_gas: 65000, // Typical ERC20 transfer cost
                function_name: "transfer(address,uint256)".to_string(),
            },
        );

        // approve(address,uint256) - 0x095ea7b3
        self.function_gas_cache.insert(
            [0x09, 0x5e, 0xa7, 0xb3],
            FunctionGasCost {
                execution_gas: 3000,
                storage_gas: 20000, // SSTORE for allowance
                memory_gas: 200,
                total_gas: 46000, // Typical ERC20 approve cost
                function_name: "approve(address,uint256)".to_string(),
            },
        );

        // transferFrom(address,address,uint256) - 0x23b872dd
        self.function_gas_cache.insert(
            [0x23, 0xb8, 0x72, 0xdd],
            FunctionGasCost {
                execution_gas: 7000,
                storage_gas: 25000, // Multiple SSTORE operations
                memory_gas: 400,
                total_gas: 85000, // Typical ERC20 transferFrom cost
                function_name: "transferFrom(address,address,uint256)".to_string(),
            },
        );

        // balanceOf(address) - 0x70a08231
        self.function_gas_cache.insert(
            [0x70, 0xa0, 0x82, 0x31],
            FunctionGasCost {
                execution_gas: 800,
                storage_gas: 2100, // SLOAD for balance
                memory_gas: 100,
                total_gas: 3500, // Typical ERC20 balanceOf cost
                function_name: "balanceOf(address)".to_string(),
            },
        );

        // allowance(address,address) - 0xdd62ed3e
        self.function_gas_cache.insert(
            [0xdd, 0x62, 0xed, 0x3e],
            FunctionGasCost {
                execution_gas: 800,
                storage_gas: 2100, // SLOAD for allowance
                memory_gas: 100,
                total_gas: 3500, // Typical ERC20 allowance cost
                function_name: "allowance(address,address)".to_string(),
            },
        );

        // Uniswap V2 Functions

        // swapExactTokensForTokens - 0x38ed1739
        self.function_gas_cache.insert(
            [0x38, 0xed, 0x17, 0x39],
            FunctionGasCost {
                execution_gas: 15000,
                storage_gas: 35000,
                memory_gas: 1000,
                total_gas: 150000, // Typical Uniswap swap cost
                function_name: "swapExactTokensForTokens".to_string(),
            },
        );

        // addLiquidity - 0xe8e33700
        self.function_gas_cache.insert(
            [0xe8, 0xe3, 0x37, 0x00],
            FunctionGasCost {
                execution_gas: 20000,
                storage_gas: 50000,
                memory_gas: 1500,
                total_gas: 200000, // Typical add liquidity cost
                function_name: "addLiquidity".to_string(),
            },
        );

        // ERC721 Functions

        // transferFrom(address,address,uint256) - Same selector as ERC20 but different gas
        // This is handled by context, but we'll use a higher estimate for NFTs

        // safeTransferFrom(address,address,uint256) - 0x42842e0e
        self.function_gas_cache.insert(
            [0x42, 0x84, 0x2e, 0x0e],
            FunctionGasCost {
                execution_gas: 10000,
                storage_gas: 25000,
                memory_gas: 2000,  // More memory for safe transfer checks
                total_gas: 120000, // Typical ERC721 transfer cost
                function_name: "safeTransferFrom(address,address,uint256)".to_string(),
            },
        );

        // mint functions (common pattern) - varies by implementation
        // We'll use a generic mint selector: 0x40c10f19 - mint(address,uint256)
        self.function_gas_cache.insert(
            [0x40, 0xc1, 0x0f, 0x19],
            FunctionGasCost {
                execution_gas: 15000,
                storage_gas: 40000, // New token creation
                memory_gas: 1000,
                total_gas: 180000, // Typical mint cost
                function_name: "mint(address,uint256)".to_string(),
            },
        );
    }

    /// Estimate gas for unknown functions based on call data complexity
    fn estimate_unknown_function_gas(&self, call_data: &Bytes) -> U256 {
        // Base cost for unknown function
        let base_cost = 5000u64;

        // Add cost based on call data size (parameter complexity)
        let data_complexity = call_data.len() / 32; // Number of 32-byte parameters
        let complexity_cost = data_complexity as u64 * 1000;

        // Add heuristic costs based on call data patterns
        let pattern_cost = self.analyze_call_data_patterns(call_data);

        U256::from(base_cost + complexity_cost + pattern_cost)
    }

    /// Analyze call data for patterns that might indicate higher gas usage
    fn analyze_call_data_patterns(&self, call_data: &Bytes) -> u64 {
        let mut pattern_cost = 0u64;

        // Large call data suggests complex operations
        if call_data.len() > 200 {
            pattern_cost += 10000;
        } else if call_data.len() > 100 {
            pattern_cost += 5000;
        }

        // Look for patterns that suggest array operations or loops
        let data = call_data.as_ref();
        let mut repeated_patterns = 0;

        // Simple heuristic: count repeated 32-byte patterns (might indicate arrays)
        if data.len() >= 64 {
            for i in 0..(data.len() - 64) {
                let chunk1 = &data[i..i + 32];
                let chunk2 = &data[i + 32..i + 64];

                // If we find similar patterns, it might be array-like data
                if chunk1[0..4] == chunk2[0..4] {
                    repeated_patterns += 1;
                }
            }
        }

        // Add cost for potential array operations
        if repeated_patterns > 3 {
            pattern_cost += repeated_patterns as u64 * 2000;
        }

        pattern_cost
    }

    /// Check if a function selector is known
    pub fn is_known_function(&self, selector: &[u8; 4]) -> bool {
        self.function_gas_cache.contains_key(selector)
    }

    /// Get function name if known
    pub fn get_function_name(&self, selector: &[u8; 4]) -> Option<String> {
        self.function_gas_cache
            .get(selector)
            .map(|cost| cost.function_name.clone())
    }

    /// Add custom function gas cost
    pub fn add_custom_function(&mut self, selector: [u8; 4], cost: FunctionGasCost) {
        self.function_gas_cache.insert(selector, cost);
    }

    /// Get all known function selectors
    pub fn get_known_selectors(&self) -> Vec<[u8; 4]> {
        self.function_gas_cache.keys().cloned().collect()
    }
}

/// Helper function to extract function selector from call data
pub fn extract_selector(call_data: &Bytes) -> Option<[u8; 4]> {
    if call_data.len() >= 4 {
        let mut selector = [0u8; 4];
        selector.copy_from_slice(&call_data[0..4]);
        Some(selector)
    } else {
        None
    }
}

/// Helper function to create call data from selector and parameters
pub fn create_call_data(selector: [u8; 4], parameters: &[u8]) -> Bytes {
    let mut call_data = Vec::with_capacity(4 + parameters.len());
    call_data.extend_from_slice(&selector);
    call_data.extend_from_slice(parameters);
    Bytes::from(call_data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::providers::{Http, Provider};

    #[tokio::test]
    async fn test_call_estimator_creation() {
        let provider = Arc::new(
            Provider::<Http>::try_from("https://eth-mainnet.alchemyapi.io/v2/demo").unwrap(),
        );
        let estimator = CallEstimator::new(provider);

        // Check that known functions are loaded
        assert!(estimator.is_known_function(&[0xa9, 0x05, 0x9c, 0xbb])); // transfer
        assert!(estimator.is_known_function(&[0x70, 0xa0, 0x82, 0x31])); // balanceOf
    }

    #[test]
    fn test_selector_extraction() {
        let call_data = Bytes::from(vec![0xa9, 0x05, 0x9c, 0xbb, 0x00, 0x00, 0x00, 0x00]);
        let selector = extract_selector(&call_data);
        assert_eq!(selector, Some([0xa9, 0x05, 0x9c, 0xbb]));
    }

    #[test]
    fn test_call_data_creation() {
        let selector = [0xa9, 0x05, 0x9c, 0xbb];
        let params = vec![0x00; 64]; // 64 bytes of parameters
        let call_data = create_call_data(selector, &params);

        assert_eq!(call_data.len(), 68); // 4 + 64
        assert_eq!(&call_data[0..4], &selector);
    }

    #[test]
    fn test_function_name_lookup() {
        let provider = Arc::new(
            Provider::<Http>::try_from("https://eth-mainnet.alchemyapi.io/v2/demo").unwrap(),
        );
        let estimator = CallEstimator::new(provider);

        let name = estimator.get_function_name(&[0xa9, 0x05, 0x9c, 0xbb]);
        assert_eq!(name, Some("transfer(address,uint256)".to_string()));
    }

    #[tokio::test]
    async fn test_unknown_function_estimation() {
        let provider = Arc::new(
            Provider::<Http>::try_from("https://eth-mainnet.alchemyapi.io/v2/demo").unwrap(),
        );
        let estimator = CallEstimator::new(provider);

        // Create call data with unknown selector
        let mut unknown_call_data_vec = vec![0x12, 0x34, 0x56, 0x78];
        unknown_call_data_vec.extend(vec![0x00; 96]); // Add 96 more bytes to total 100
        let unknown_call_data = Bytes::from(unknown_call_data_vec);
        let gas_estimate = estimator.estimate_unknown_function_gas(&unknown_call_data);

        assert!(gas_estimate > U256::from(5000)); // Should be more than base cost
    }

    #[test]
    fn test_call_data_pattern_analysis() {
        let provider = Arc::new(
            Provider::<Http>::try_from("https://eth-mainnet.alchemyapi.io/v2/demo").unwrap(),
        );
        let estimator = CallEstimator::new(provider);

        // Large call data should incur additional cost
        let large_call_data = Bytes::from(vec![0x00; 300]);
        let pattern_cost = estimator.analyze_call_data_patterns(&large_call_data);

        assert!(pattern_cost > 0);
    }
}
