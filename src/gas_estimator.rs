use crate::{
    tracer::Tracer,
    utils::{calculate_calldata_cost, calculate_contract_creation_cost},
};
use alloy::{
    eips::BlockId,
    primitives::U64,
    providers::{Provider, ProviderBuilder},
};
use revm::{
    context::{transaction::AccessList, tx::TxEnvBuilder},
    database::{CacheDB, EmptyDB},
    inspector::InspectEvm,
    primitives::{keccak256, Address, Bytes, TxKind, U256},
    state::{AccountInfo, Bytecode},
    Context, MainBuilder, MainContext,
};
use serde::{Deserialize, Serialize};

pub const BLOCK_GAS_LIMIT: u64 = 30_000_000; // or 36,000,000

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
        // Base transaction cost (21,000 gas for simple transfers)
        let base_cost = 21_000;

        let execution_cost = if tx_params.to.is_some() && tx_params.data.is_some() {
            self.simulate_call(&tx_params).await?
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

    /// Check if an address is a precompile address
    /// Ethereum precompiles are at addresses 0x01 through 0x09 (and potentially higher)
    pub fn is_precompile(address: Address) -> bool {
        let addr_u64 = address.as_slice()[19];
        // Check if address is in the range 0x01 to 0x09 (standard Ethereum precompiles)
        // Can be extended to include more precompiles as needed
        addr_u64 >= 1 && addr_u64 <= 9
    }

    pub async fn simulate_call(&self, tx_params: &Tx) -> Result<u128, Box<dyn std::error::Error>> {
        let provider = ProviderBuilder::new().connect(&self.rpc_url).await.unwrap();
        let current_gas_price = provider.get_gas_price().await?;
        let mut tracer = Tracer::new();

        let mut cache_db = CacheDB::new(EmptyDB::default());

        // Get actual balance from the provider
        let caller = tx_params.from.unwrap();
        self.add_balance_to_db(&mut cache_db, caller).await;

        // Get contract code from provider and add it to cache
        let contract_address = tx_params.to.unwrap();
        self.add_code_to_db(&mut cache_db, contract_address).await;

        let account = cache_db.load_account(caller).unwrap();
        let tx_evm = TxEnvBuilder::new()
            .caller(caller)
            .kind(TxKind::Call(tx_params.to.unwrap()))
            .data(tx_params.data.clone().unwrap())
            .value(tx_params.value)
            .gas_price(tx_params.gas_price.unwrap_or(current_gas_price))
            .gas_limit(tx_params.gas_limit.unwrap_or(BLOCK_GAS_LIMIT))
            .nonce(account.info.nonce)
            .access_list(
                tx_params
                    .access_list
                    .clone()
                    .unwrap_or(AccessList::default()),
            )
            .build()
            .unwrap();

        let mut latest_gas_costs = 0;
        let mut max_gas_costs = 0;
        while {
            let mut evm = Context::mainnet()
                .with_db(cache_db.clone())
                .build_mainnet_with_inspector(&mut tracer);
            // Execute transaction without writing to the DB
            let gas_costs = match evm.inspect_tx(tx_evm.clone()) {
                Ok(result) => {
                    println!("result: {:?}", result);

                    let tracer_after_call = evm.inspector.clone();
                    println!("tracer: {:?}", tracer_after_call);

                    result.gas_used() as u128
                }
                Err(e) => {
                    println!("EVM execution error: {:?}", e);
                    // Return a default gas cost for contract calls
                    30_000
                }
            };
            latest_gas_costs = gas_costs;
            if gas_costs > max_gas_costs {
                max_gas_costs = gas_costs;
            }

            tracer.has_new_accesses()
        } {
            for contract_address in &tracer.contract_addresses {
                self.add_code_to_db(&mut cache_db, *contract_address).await;
            }
            for (contract_address, storage_slot) in &tracer.storage_accesses {
                self.populate_storage_slot(&mut cache_db, *contract_address, *storage_slot)
                    .await;
            }
            tracer.reset_state();
        }
        Ok(latest_gas_costs)
    }

    pub async fn add_balance_to_db(&self, cache_db: &mut CacheDB<EmptyDB>, caller: Address) {
        let provider = ProviderBuilder::new().connect(&self.rpc_url).await.unwrap();
        let balance = provider.get_balance(caller).await.unwrap_or_else(|_| {
            // Fallback to a reasonable amount if balance fetch fails
            U256::from(10u128.pow(18) * 1000) // 1000 ETH
        });
        let nonce = provider.get_transaction_count(caller).await.unwrap_or(0);

        cache_db.insert_account_info(
            caller,
            AccountInfo {
                balance,
                nonce,
                code_hash: revm::primitives::KECCAK_EMPTY,
                code: None,
            },
        );
    }
    pub async fn add_code_to_db(&self, cache_db: &mut CacheDB<EmptyDB>, contract_address: Address) {
        let provider = ProviderBuilder::new().connect(&self.rpc_url).await.unwrap();
        let contract_code = provider
            .get_code_at(contract_address)
            .await
            .unwrap_or_default();
        if !Self::is_precompile(contract_address) {
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
        }
    }

    pub async fn populate_storage_slot(
        &self,
        cache_db: &mut CacheDB<EmptyDB>,
        contract_address: Address,
        storage_slot: U256,
    ) {
        let provider = ProviderBuilder::new().connect(&self.rpc_url).await.unwrap();
        let storage_val = provider
            .get_storage_at(contract_address, storage_slot)
            .await
            .unwrap();
        cache_db
            .insert_account_storage(contract_address, storage_slot, storage_val)
            .unwrap();
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
    use Caller::{call_counterCall, precompileCall};
    use Counter::{complexCall, setNumberCall};

    const ETH_RPC_URL: &str = "http://localhost:8545";
    const MNEMONIC: &str = "test test test test test test test test test test test junk";

    const COUNTER_BYTECODE: &str = "60808060405234601957602a5f556102a8908161001e8239f35b5f80fdfe60806040526004361015610011575f80fd5b5f3560e01c80633fb5c1cb1461020c5780638381f58a146101ef578063a49e0ab1146100655763d555654414610045575f80fd5b34610061575f3660031901126100615760205f54604051908152f35b5f80fd5b34610061575f366003190112610061576002545f6002558061018f575b505f5b600a81111561013b575f5b600a81111561009b57005b600181116100d657806100cc816100b46100d194610246565b90919082549060031b91821b915f19901b1916179055565b610238565b610090565b5f198101818111610127576100ea90610246565b90549060031b1c9060011981018181116101275761010790610246565b90549060031b1c8201809211610127576100cc6100d1926100b483610246565b634e487b7160e01b5f52601160045260245ffd5b600254906801000000000000000082101561017b576101638260016101769401600255610246565b8154905f199060031b1b19169055610238565b610085565b634e487b7160e01b5f52604160045260245ffd5b60025f527f405787fa12a823e0f2b7631cc41b3ba8828b3321ca811111fa75cd3aa3bb5ace017f405787fa12a823e0f2b7631cc41b3ba8828b3321ca811111fa75cd3aa3bb5ace5b8181106101e45750610082565b5f81556001016101d7565b34610061575f366003190112610061576020600154604051908152f35b34610061576020366003190112610061575f546004358082111561006157810390811161012757600155005b5f1981146101275760010190565b60025481101561025e5760025f5260205f2001905f90565b634e487b7160e01b5f52603260045260245ffdfea2646970667358221220ab2acc29b1df7998556b96668ce5211aeaeb96da04317a1c3def538659a7dffc64736f6c634300081e0033";
    sol!(
        #[allow(missing_docs)]
        #[sol(rpc)]
        Counter,
        "./contracts/out/Counter.sol/Counter.json"
    );

    sol!(
        #[allow(missing_docs)]
        #[sol(rpc)]
        Caller,
        "./contracts/out/Caller.sol/Caller.json"
    );

    #[tokio::test]
    async fn test_estimate_gas_simple_transfer() {
        let estimator = GasEstimator::new(&ETH_RPC_URL).await.unwrap();
        let tx = Tx {
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
        };

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
        let bytecode = Bytes::from(COUNTER_BYTECODE);
        Counter::deploy(&provider).await.unwrap();

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

        let result = estimator.estimate_gas(tx).await;
        assert!(result.is_ok());

        let estimate = result.unwrap();
        assert!(estimate.estimated_gas >= 21000);
        assert!(estimate.gas_price > 0);
    }

    #[tokio::test]
    async fn test_estimate_gas_contract_call_1() {
        let estimator = GasEstimator::new(&ETH_RPC_URL).await.unwrap();
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
        let contract = Counter::deploy(&provider).await.unwrap();
        let contract_address = contract.address();
        let call_data = setNumberCall::new((U256::from(20),));

        let tx = Tx {
            from: Some(wallet.address()),
            to: Some(*contract_address),
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
        };

        let result = estimator.estimate_gas(tx).await;
        assert!(result.is_ok());

        let estimate = result.unwrap();
        assert!(estimate.estimated_gas >= 21000);
        assert!(estimate.gas_price > 0);
    }

    #[tokio::test]
    async fn test_estimate_gas_contract_call_2() {
        let estimator = GasEstimator::new(&ETH_RPC_URL).await.unwrap();
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
        let contract = Counter::deploy(&provider).await.unwrap();
        let contract_address = contract.address();

        let call_data = complexCall::new(());

        let tx = Tx {
            from: Some(wallet.address()),
            to: Some(*contract_address),
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
        };

        let result = estimator.estimate_gas(tx).await;
        assert!(result.is_ok());

        let estimate = result.unwrap();
        assert!(estimate.estimated_gas >= 21000);
        assert!(estimate.gas_price > 0);
    }

    #[tokio::test]
    async fn test_estimate_gas_contract_call_3() {
        let estimator = GasEstimator::new(&ETH_RPC_URL).await.unwrap();
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
        let contract = Caller::deploy(&provider).await.unwrap();
        let contract_address = contract.address();
        // calling precompile function
        let call_data = precompileCall::new((U256::from(123456),));

        let tx = Tx {
            from: Some(wallet.address()),
            to: Some(*contract_address),
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
        };

        let result = estimator.estimate_gas(tx).await;
        assert!(result.is_ok());

        let estimate = result.unwrap();
        assert!(estimate.estimated_gas >= 21000);
        assert!(estimate.gas_price > 0);
    }

    #[tokio::test]
    async fn test_estimate_gas_contract_call_4() {
        let estimator = GasEstimator::new(&ETH_RPC_URL).await.unwrap();
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
        let contract = Caller::deploy(&provider).await.unwrap();
        let contract_address = contract.address();

        let counter_contract = Counter::deploy(&provider).await.unwrap();
        let counter_contract_address = counter_contract.address();

        // calling precompile function
        let call_data = call_counterCall::new((*counter_contract_address,));

        let tx = Tx {
            from: Some(wallet.address()),
            to: Some(*contract_address),
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
        };

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
