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
    use alloy::primitives::{address, bytes, U256, U64};
    use reqwest;
    use serde_json::{json, Value};
    use std::time::Duration;
    use tokio::time::sleep;

    async fn setup_test_server() -> (RpcServer, String) {
        let bind_addr = "127.0.0.1:0".parse().unwrap();
        let default_rpc_url = std::env::var("ETH_RPC_URL")
            .unwrap_or_else(|_| "https://eth-mainnet.alchemyapi.io/v2/demo".to_string());

        let server = RpcServer::new(bind_addr, default_rpc_url).await.unwrap();
        let server_url = format!("http://{}", server.local_addr());

        // Give server a moment to fully start
        sleep(Duration::from_millis(100)).await;

        (server, server_url)
    }

    fn create_simple_transfer_tx() -> Tx {
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

    #[tokio::test]
    async fn test_rpc_simple_transfer() {
        // Skip test if no ETH_RPC_URL is set
        if std::env::var("ETH_RPC_URL").is_err() {
            println!("Skipping test: ETH_RPC_URL not set");
            return;
        }

        let (_server, server_url) = setup_test_server().await;
        let client = reqwest::Client::new();

        let request_body = json!({
            "jsonrpc": "2.0",
            "method": "estimate_gas",
            "params": [{
                "transaction": create_simple_transfer_tx(),
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
    async fn test_rpc_contract_deployment() {
        // Skip test if no ETH_RPC_URL is set
        if std::env::var("ETH_RPC_URL").is_err() {
            println!("Skipping test: ETH_RPC_URL not set");
            return;
        }

        let (_server, server_url) = setup_test_server().await;
        let client = reqwest::Client::new();

        let request_body = json!({
            "jsonrpc": "2.0",
            "method": "estimate_gas",
            "params": [{
                "transaction": create_contract_deployment_tx(),
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

        // Check breakdown structure for contract deployment
        let breakdown = &result["breakdown"];
        assert!(breakdown["base_cost"].as_u64().unwrap() == 21000);
        assert!(breakdown["contract_creation_cost"].as_u64().unwrap() > 0); // Should have creation cost
        assert!(breakdown["data_cost"].as_u64().unwrap() == 0);
        assert!(breakdown["execution_cost"].as_u64().unwrap() == 0);
        assert!(breakdown["access_list_cost"].as_u64().unwrap() == 0);
    }

    #[tokio::test]
    async fn test_rpc_contract_call() {
        // Skip test if no ETH_RPC_URL is set
        if std::env::var("ETH_RPC_URL").is_err() {
            println!("Skipping test: ETH_RPC_URL not set");
            return;
        }

        let (_server, server_url) = setup_test_server().await;
        let client = reqwest::Client::new();

        let request_body = json!({
            "jsonrpc": "2.0",
            "method": "estimate_gas",
            "params": [{
                "transaction": create_contract_call_tx(),
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
        assert!(breakdown["access_list_cost"].as_u64().unwrap() == 0);
    }
}
