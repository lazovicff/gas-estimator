# Ethereum Gas Estimator 🔥

A comprehensive Rust-based tool for estimating gas costs for Ethereum transactions. This tool provides accurate gas estimation for various transaction types including ETH transfers, contract calls, and contract deployments using **custom-built estimation logic** implemented from scratch.

**NEW: JSON-RPC Server** - Now includes a JSON-RPC server that exposes gas estimation functionality over HTTP!

## Features

- **JSON-RPC Server**: HTTP server with `estimate_gas` endpoint for easy integration
- **Custom Gas Estimation**: Built-from-scratch gas estimation logic with detailed breakdown
- **Real-time Gas Estimation**: Get current gas prices and estimates from the Ethereum network
- **Multiple Transaction Types**: Support for ETH transfers, contract calls, and deployments
- **EIP-1559 Support**: Handles both legacy and EIP-1559 (London hard fork) transactions
- **Detailed Gas Breakdown**: Shows cost breakdown by operation type (data, storage, execution, etc.)
- **Provider Comparison**: Compare custom estimation with provider's built-in estimation
- **Precompile Support**: Estimates costs for precompile contract calls (SHA256, ECDSA, etc.)
- **Access List Estimation**: EIP-2930 access list cost calculation
- **Network Information**: Fetches current network conditions including block utilization
- **Flexible API**: Easy-to-use API for integrating gas estimation into your applications
- **CORS Support**: Cross-origin requests enabled for web applications

## Quick Start

### Prerequisites

- Rust 1.70 or higher
- Internet connection (for accessing Ethereum RPC endpoints)

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
cargo run --bin server
```

4. Or run the demo examples:
```bash
cargo run
```

## Usage

### JSON-RPC Server

The easiest way to use the gas estimator is through the JSON-RPC server:

#### Starting the Server

```bash
cargo run --bin server
```

The server will start on `http://127.0.0.1:3030` by default and display:

```
   Starting Gas Estimation JSON-RPC Server...
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

#### Making Requests

The server exposes a single JSON-RPC endpoint: `estimate_gas`

**Method**: `POST`
**URL**: `http://127.0.0.1:3030`
**Content-Type**: `application/json`

**Request Format**:
```json
{
  "jsonrpc": "2.0",
  "method": "estimate_gas",
  "params": {
    "transaction": {
      "from": null,
      "to": "0x742d35Cc6634C0532925a3b8D401B1C4029Ee7A7",
      "value": "1000000000000000000",
      "data": null,
      "nonce": null,
      "chainId": 1,
      "gas": null,
      "gasPrice": null,
      "maxFeePerGas": null,
      "maxPriorityFeePerGas": null,
      "accessList": null,
      "type": null
    },
    "rpc_url": null
  },
  "id": 1
}
```
