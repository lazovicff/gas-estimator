# Ethereum Gas Estimator

A comprehensive Rust-based tool for estimating gas costs for Ethereum transactions using **custom EVM simulation** powered by REVM. The tool provides accurate gas estimation for various transaction types including ETH transfers, contract calls, and contract deployments through a JSON-RPC server interface.

## Architecture

The gas estimator is built with a modular architecture:

- **`gas_estimator`**: Core EVM simulation logic using REVM
- **`rpc_server`**: JSON-RPC server implementation with CORS support
- **`tracer`**: Custom EVM tracer for detailed execution analysis
- **`utils`**: Utility functions for gas calculations and conversions
- **`error`**: Comprehensive error handling
- **`tests`**: Test suite comparing custom EVM estimation with live network provider results

## Features

- **EVM-based Gas Estimation**: Uses REVM to simulate transaction execution for precise gas calculations
- **JSON-RPC Server**: HTTP server with `estimate_gas` endpoint for easy integration
- **Detailed Gas Breakdown**: Cost breakdown by operation type (base, data, execution, storage, etc.)
- **Multiple Transaction Types**: Support for ETH transfers, contract calls, and deployments
- **EIP-1559 Support**: Handles both legacy and EIP-1559 transactions
- **Provider Comparison Testing**: Test suite comparing custom estimation with Alloy provider estimates
- **Precompile Support**: Estimates costs for precompile contract calls (SHA256, ECDSA, etc.)
- **Access List Estimation**: EIP-2930 access list cost calculation
- **Real-time Network Info**: Fetches current gas prices and network conditions
- **CORS Support**: Cross-origin requests enabled for web applications
- **Custom Tracer**: Built-in execution tracer for detailed transaction analysis

## Quick Start

### Prerequisites

- Rust 1.70 or higher
- Internet connection (for accessing Ethereum RPC endpoints)
- For testing: Local Ethereum node (Anvil) running on `http://localhost:8545`

### Installation

1. Clone the repository:
```bash
git clone <repository-url>
cd gas-estimator
```

2. Build the project:
```bash
cargo build --release
```

3. Run the JSON-RPC server:
```bash
cargo run
```

### Testing

Start a local Ethereum node and run tests:

```bash
# Terminal 1: Start Anvil
anvil

# Terminal 2: Run tests
cargo test

# Run comprehensive comparison test with output
cargo test test_all_gas_estimation_approaches -- --nocapture
```

## Usage

### JSON-RPC Server

#### Starting the Server

```bash
cargo run
```

The server starts on `http://127.0.0.1:3030` and displays:

```
Using Ethereum RPC: https://eth-mainnet.alchemyapi.io/v2/demo
Testing connection to Ethereum network...
    Connected to Ethereum network!
    Current Gas Price: 25.2 Gwei
    Latest Block: 18750000
    Base Fee: 24.8 Gwei
Starting JSON-RPC server on 127.0.0.1:3030
Gas Estimation JSON-RPC Server is running!
Address: http://127.0.0.1:3030
Endpoint: estimate_gas
```

#### API Endpoint

**Method**: `POST`
**URL**: `http://127.0.0.1:3030`
**Content-Type**: `application/json`

**Request Format**:
```json
{
  "jsonrpc": "2.0",
  "method": "estimate_gas",
  "params": [{
    "transaction": {
      "from": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
      "to": "0x742d35Cc6634C0532925a3b8D401B1C4029Ee7A7",
      "value": "1000000000000000000",
      "data": null,
      "nonce": 1,
      "chain_id": 1,
      "gas_limit": null,
      "gas_price": "20000000000",
      "max_fee_per_gas": null,
      "max_priority_fee_per_gas": null,
      "access_list": null,
      "transaction_type": 0
    },
    "rpc_url": null
  }],
  "id": 1
}
```

**Response Format**:
```json
{
  "jsonrpc": "2.0",
  "result": {
    "estimate": {
      "estimated_gas": 21000,
      "gas_price": 20000000000,
      "total_cost_wei": "420000000000000",
      "total_cost_eth": "0.00042",
      "breakdown": {
        "base_cost": 21000,
        "data_cost": 0,
        "contract_creation_cost": 0,
        "execution_cost": 0,
        "access_list_cost": 0
      }
    }
  },
  "id": 1
}
```

## License

[Add your license information here]
