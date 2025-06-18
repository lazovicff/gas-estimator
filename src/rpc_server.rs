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
