use crate::{
    error::Error,
    tracer::Tracer,
    utils::{calculate_calldata_cost, calculate_contract_creation_cost},
};
use alloy::{
    eips::BlockId,
    network::Ethereum,
    primitives::U64,
    providers::{Provider, RootProvider},
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
    provider: RootProvider,
}

impl GasEstimator {
    pub fn new(rpc_url: &str) -> Self {
        let provider = RootProvider::<Ethereum>::new_http(rpc_url.parse().unwrap());
        Self { provider }
    }

    /// Custom gas estimation implementation from scratch
    pub async fn estimate_gas(&self, tx_params: Tx) -> Result<GasEstimate, Error> {
        // Calculate gas breakdown using our custom logic
        let breakdown = self.calculate_gas_breakdown(&tx_params).await?;

        // Sum up all gas costs
        let estimated_gas =
            breakdown.base_cost + breakdown.contract_creation_cost + breakdown.execution_cost;

        // Get current gas price information
        let gas_price = self
            .provider
            .get_gas_price()
            .await
            .map_err(Error::RpcError)?;

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
    async fn calculate_gas_breakdown(&self, tx_params: &Tx) -> Result<GasBreakdown, Error> {
        // Base transaction cost
        // IMPROVEMENT
        // Use provider to fetch base fee per gas to determain base fee
        let base_cost = if let Some(chain_id) = tx_params.chain_id {
            // Anvil has 0 base cost if calling a contract
            if chain_id == U64::from(31337) && self.is_contract(tx_params.to).await? {
                0
            } else {
                21000
            }
        } else {
            21000
        };

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
        // IMPROVEMENT:
        // Use EVM for more precise predictions
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

    async fn is_contract(&self, to: Option<Address>) -> Result<bool, Error> {
        if to.is_none() {
            return Ok(false);
        }
        let code = self
            .provider
            .get_code_at(to.unwrap())
            .await
            .map_err(Error::RpcError)?;
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

    pub async fn simulate_call(&self, tx_params: &Tx) -> Result<u128, Error> {
        let current_gas_price = self
            .provider
            .get_gas_price()
            .await
            .map_err(Error::RpcError)?;
        let mut tracer = Tracer::new();

        let mut cache_db = CacheDB::new(EmptyDB::default());

        // Get actual balance from the provider
        let caller = tx_params.from.unwrap();
        self.add_balance_to_db(&mut cache_db, caller).await?;

        // Get contract code from provider and add it to cache
        let contract_address = tx_params.to.unwrap();
        self.add_code_to_db(&mut cache_db, contract_address).await?;

        let account = cache_db.load_account(caller).unwrap();
        // IMPROVEMENT
        // Add:
        // - gas_priority_fee
        // - max_fee_per_gas
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
                self.add_code_to_db(&mut cache_db, *contract_address)
                    .await?;
            }
            for (contract_address, storage_slot) in &tracer.storage_accesses {
                self.populate_storage_slot(&mut cache_db, *contract_address, *storage_slot)
                    .await?;
            }
            tracer.reset_state();
        }
        Ok(latest_gas_costs)
    }

    pub async fn add_balance_to_db(
        &self,
        cache_db: &mut CacheDB<EmptyDB>,
        caller: Address,
    ) -> Result<(), Error> {
        let balance = self.provider.get_balance(caller).await.unwrap_or_else(|_| {
            // Fallback to a reasonable amount if balance fetch fails
            U256::from(10u128.pow(18) * 1000) // 1000 ETH
        });
        let nonce = self
            .provider
            .get_transaction_count(caller)
            .await
            .unwrap_or(0);

        cache_db.insert_account_info(
            caller,
            AccountInfo {
                balance,
                nonce,
                code_hash: revm::primitives::KECCAK_EMPTY,
                code: None,
            },
        );

        Ok(())
    }
    pub async fn add_code_to_db(
        &self,
        cache_db: &mut CacheDB<EmptyDB>,
        contract_address: Address,
    ) -> Result<(), Error> {
        let contract_code = self
            .provider
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

        Ok(())
    }

    pub async fn populate_storage_slot(
        &self,
        cache_db: &mut CacheDB<EmptyDB>,
        contract_address: Address,
        storage_slot: U256,
    ) -> Result<(), Error> {
        let storage_val = self
            .provider
            .get_storage_at(contract_address, storage_slot)
            .await
            .map_err(Error::RpcError)?;

        cache_db
            .insert_account_storage(contract_address, storage_slot, storage_val)
            .unwrap();

        Ok(())
    }

    pub async fn get_network_gas_info(&self) -> Result<NetworkGasInfo, Error> {
        let gas_price = self
            .provider
            .get_gas_price()
            .await
            .map_err(Error::RpcError)?;
        let latest_block = self
            .provider
            .get_block(BlockId::latest())
            .await
            .map_err(Error::RpcError)?
            .unwrap();

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
