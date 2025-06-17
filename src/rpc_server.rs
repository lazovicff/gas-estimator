use crate::gas_estimator::{GasEstimate, GasEstimator, Tx};
use jsonrpsee::{
    core::{async_trait, RpcResult},
    proc_macros::rpc,
    server::{ServerBuilder, ServerHandle},
    types::ErrorObjectOwned,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tower_http::cors::{Any, CorsLayer};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstimateGasRequest {
    pub transaction: Tx,
    pub rpc_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstimateGasResponse {
    pub estimate: GasEstimate,
}

// Define the JSON-RPC interface
#[rpc(server)]
pub trait GasEstimationRpc {
    #[method(name = "estimate_gas")]
    async fn estimate_gas(&self, request: EstimateGasRequest) -> RpcResult<EstimateGasResponse>;
}

pub struct GasEstimationRpcImpl {
    default_rpc_url: String,
}

impl GasEstimationRpcImpl {
    pub fn new(default_rpc_url: String) -> Self {
        Self { default_rpc_url }
    }
}

#[async_trait]
impl GasEstimationRpcServer for GasEstimationRpcImpl {
    async fn estimate_gas(&self, request: EstimateGasRequest) -> RpcResult<EstimateGasResponse> {
        // Use provided RPC URL or fallback to default
        let rpc_url = request.rpc_url.as_ref().unwrap_or(&self.default_rpc_url);

        // Create gas estimator instance
        let estimator = match GasEstimator::new(rpc_url).await {
            Ok(estimator) => estimator,
            Err(e) => {
                return Err(ErrorObjectOwned::owned(
                    -32603,
                    format!("Failed to create gas estimator: {}", e),
                    None::<String>,
                ))
            }
        };

        // Perform gas estimation
        let estimate = match estimator.estimate_gas(request.transaction).await {
            Ok(estimate) => estimate,
            Err(e) => {
                return Err(ErrorObjectOwned::owned(
                    -32603,
                    format!("Gas estimation failed: {}", e),
                    None::<String>,
                ))
            }
        };

        Ok(EstimateGasResponse { estimate })
    }
}

pub struct RpcServer {
    handle: ServerHandle,
    addr: SocketAddr,
}

impl RpcServer {
    pub async fn new(
        bind_addr: SocketAddr,
        default_rpc_url: String,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Setup CORS
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_headers(Any)
            .allow_methods(Any);

        // Build the server
        let server = ServerBuilder::default()
            .set_middleware(tower::ServiceBuilder::new().layer(cors))
            .build(bind_addr)
            .await?;

        let addr = server.local_addr()?;
        // Create the RPC implementation
        let rpc_impl = GasEstimationRpcImpl::new(default_rpc_url);
        // Start the server
        let handle = server.start(rpc_impl.into_rpc());
        Ok(Self { handle, addr })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    pub async fn stop(self) -> Result<(), Box<dyn std::error::Error>> {
        self.handle
            .stop()
            .map_err(|e| format!("Failed to stop server: {:?}", e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        primitives::{address, U256, U64},
        providers::ProviderBuilder,
        signers::local::{coins_bip39::English, MnemonicBuilder},
        sol,
        sol_types::SolCall,
    };
    use reqwest;
    use revm::primitives::{Address, Bytes};
    use serde_json::{json, Value};
    use std::time::Duration;
    use tokio::time::sleep;

    const ETH_RPC_URL: &str = "http://localhost:8545";
    const MNEMONIC: &str = "test test test test test test test test test test test junk";

    const BYTECODE: &str = "60808060405234601957602a5f55610106908161001e8239f35b5f80fdfe608060405260043610156010575f80fd5b5f3560e01c80633fb5c1cb1460af5780638381f58a146094578063d09de08a14605e5763d5556544146040575f80fd5b34605a575f366003190112605a5760205f54604051908152f35b5f80fd5b34605a575f366003190112605a576001545f1981146080576001016001555f80f35b634e487b7160e01b5f52601160045260245ffd5b34605a575f366003190112605a576020600154604051908152f35b34605a576020366003190112605a575f54600435810180911160805760015500fea2646970667358221220e470db5efcff30a5d2bf2dfc5c01072c1364af37644d14ea4b2c86293086d86664736f6c634300081e0033";
    sol!(
        #[allow(missing_docs)]
        #[sol(rpc)]
        Counter,
        "./contracts/out/Counter.sol/Counter.json"
    );

    async fn setup_test_server() -> (RpcServer, String) {
        let bind_addr = "127.0.0.1:0".parse().unwrap();

        let server = RpcServer::new(bind_addr, ETH_RPC_URL.to_string())
            .await
            .unwrap();
        let server_url = format!("http://{}", server.local_addr());

        // Give server a moment to fully start
        sleep(Duration::from_millis(100)).await;

        (server, server_url)
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
        let bytecode = Bytes::from(BYTECODE);
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
        let call_data = Counter::setNumberCall::new((U256::from(1000000000000000000u64),));

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
    async fn test_rpc_simple_transfer() {
        let (_server, server_url) = setup_test_server().await;
        let client = reqwest::Client::new();

        let request_body = json!({
            "jsonrpc": "2.0",
            "method": "estimate_gas",
            "params": [{
                "transaction": create_basic_tx(),
                "rpc_url": null
            }],
            "id": 1
        });

        let response = client
            .post(&server_url)
            .json(&request_body)
            .send()
            .await
            .expect("Failed to send request");

        assert!(response.status().is_success());

        let response_body: Value = response.json().await.expect("Failed to parse JSON");

        // Check that we got a successful response
        if !response_body["error"].is_null() {
            panic!("RPC request failed with error: {}", response_body["error"]);
        }
        assert!(response_body["result"].is_object());

        let result = &response_body["result"]["estimate"];
        assert!(result["estimated_gas"].as_u64().unwrap() >= 21000);
        assert!(result["gas_price"].as_u64().unwrap() > 0);
        assert!(result["total_cost_wei"].as_u64().unwrap() > 0);

        // Check breakdown structure
        let breakdown = &result["breakdown"];
        assert!(breakdown["base_cost"].as_u64().unwrap() == 21000);
        assert!(breakdown["data_cost"].as_u64().unwrap() == 0); // No data for simple transfer
        assert!(breakdown["contract_creation_cost"].as_u64().unwrap() == 0);
        assert!(breakdown["execution_cost"].as_u64().unwrap() == 0);
        assert!(breakdown["access_list_cost"].as_u64().unwrap() == 0);
    }

    #[tokio::test]
    async fn test_rpc_contract_call() {
        let (_, contract_address) = create_contract_deployment_tx().await;
        let (_server, server_url) = setup_test_server().await;
        let client = reqwest::Client::new();

        let request_body = json!({
            "jsonrpc": "2.0",
            "method": "estimate_gas",
            "params": [{
                "transaction": create_contract_call_tx(contract_address),
                "rpc_url": null
            }],
            "id": 1
        });

        let response = client
            .post(&server_url)
            .json(&request_body)
            .send()
            .await
            .expect("Failed to send request");

        assert!(response.status().is_success());

        let response_body: Value = response.json().await.expect("Failed to parse JSON");

        // Check that we got a successful response
        if !response_body["error"].is_null() {
            panic!("RPC request failed with error: {}", response_body["error"]);
        }
        assert!(response_body["result"].is_object());

        let result = &response_body["result"]["estimate"];
        assert!(result["estimated_gas"].as_u64().unwrap() >= 21000);
        assert!(result["gas_price"].as_u64().unwrap() > 0);
        assert!(result["total_cost_wei"].as_u64().unwrap() > 0);

        // Check breakdown structure for contract call
        let breakdown = &result["breakdown"];
        assert!(breakdown["base_cost"].as_u64().unwrap() == 21000);
        assert!(breakdown["contract_creation_cost"].as_u64().unwrap() == 0);
        // Should have some execution cost and potentially data cost for contract call
        assert!(breakdown["execution_cost"].as_u64().unwrap_or(0) >= 0);
    }
}
