mod error;
mod gas_estimator;
mod rpc_server;
mod tests;
mod tracer;
mod utils;

use gas_estimator::GasEstimator;
use rpc_server::RpcServer;
use std::net::SocketAddr;
use tokio::signal;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Default configuration
    let bind_addr: SocketAddr = "127.0.0.1:3030".parse()?;
    let default_rpc_url = std::env::var("ETH_RPC_URL")
        .unwrap_or_else(|_| "https://eth-mainnet.alchemyapi.io/v2/demo".to_string());

    println!("Using Ethereum RPC: {}", default_rpc_url);

    // Test connection to the RPC endpoint
    println!("Testing connection to Ethereum network...");
    match GasEstimator::new(&default_rpc_url).await {
        Ok(estimator) => match estimator.get_network_gas_info().await {
            Ok(network_info) => {
                println!("    Connected to Ethereum network!");
                println!(
                    "    Current Gas Price: {} Gwei",
                    network_info.current_gas_price
                );
                println!("    Latest Block: {}", network_info.latest_block_number);
                if let Some(base_fee) = network_info.base_fee_per_gas {
                    println!("    Base Fee: {} Gwei", base_fee);
                }
            }
            Err(e) => {
                println!("    Warning: Could not fetch network info: {}", e);
                println!("    Server will still start, but gas estimation may be limited");
            }
        },
        Err(e) => {
            eprintln!("    ❌ Failed to connect to Ethereum RPC: {}", e);
            eprintln!("    Please check your RPC URL and try again");
            return Err(e);
        }
    }

    // Start the RPC server
    println!("Starting JSON-RPC server on {}", bind_addr);
    let server = RpcServer::new(bind_addr, default_rpc_url).await?;
    let actual_addr = server.local_addr();

    println!("Gas Estimation JSON-RPC Server is running!");
    println!("Address: http://{}", actual_addr);
    println!("Endpoint: estimate_gas");
    println!();
    println!("Example request:");
    println!(
        r#"{{
  "jsonrpc": "2.0",
  "method": "estimate_gas",
  "params": {{
    "transaction": {{
      "to": "0x742d35Cc6634C0532925a3b8D401B1C4029Ee7A7",
      "value": "1000000000000000000",
      "data": null
    }},
    "rpc_url": null
  }},
  "id": 1
}}"#
    );
    println!();
    println!("You can also provide a custom RPC URL in the request");
    println!("Press Ctrl+C to stop the server");

    // Wait for shutdown signal
    signal::ctrl_c().await?;
    println!("\nShutting down server...");

    // Stop the server gracefully
    server.stop().await?;
    println!("Server stopped successfully");

    Ok(())
}
