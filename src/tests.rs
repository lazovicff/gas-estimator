use crate::gas_estimator::{GasEstimator, Tx};
use alloy::{
    primitives::{address, U256, U64},
    providers::{Provider, ProviderBuilder},
    signers::local::{coins_bip39::English, MnemonicBuilder},
    sol,
    sol_types::SolCall,
};
use revm::primitives::Bytes;
use std::str::FromStr;

const ETH_RPC_URL: &str = "http://localhost:8545";
const MNEMONIC: &str = "test test test test test test test test test test test junk";

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
async fn test_all_gas_estimation_approaches() {
    println!("\n=== Testing All Gas Estimation Approaches ===\n");

    // Setup wallet and provider for contract operations
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

    // Deploy contract once for testing contract calls
    let contract = Counter::deploy(&provider).await.unwrap();
    let counter_contract_address = *contract.address();

    let contract = Caller::deploy(&provider).await.unwrap();
    let caller_contract_address = *contract.address();

    // Initialize estimators
    let evm_estimator = GasEstimator::new(ETH_RPC_URL).await.unwrap();

    // Test Case 1: Simple Transfer
    let simple_transfer_tx = Tx {
        from: Some(wallet.address()),
        to: Some(address!("0x1234567890123456789012345678901234567890")),
        value: U256::from(1u64), // 1 ETH
        data: None,
        nonce: Some(1),
        chain_id: Some(U64::from(31337)), // Anvil default chain ID
        gas_limit: Some(21000),
        gas_price: Some(20000000000), // 20 gwei
        max_fee_per_gas: None,
        max_priority_fee_per_gas: None,
        access_list: None,
        transaction_type: Some(U64::from(0)),
    };

    // Test EVM-based estimation for simple transfer
    let evm_transfer_result = evm_estimator.estimate_gas(simple_transfer_tx.clone()).await;
    let evm_transfer_estimate = evm_transfer_result.unwrap();

    // Test Alloy provider estimation for simple transfer
    let alloy_transfer_tx = alloy::rpc::types::TransactionRequest::default()
        .from(wallet.address())
        .to(address!("0x1234567890123456789012345678901234567890"))
        .value(U256::from(1u64));

    let alloy_transfer_result = provider.estimate_gas(alloy_transfer_tx).await;
    let alloy_transfer_estimate = alloy_transfer_result.unwrap();

    // Test Case 2: Contract Deployment
    let deployment_bytecode = Bytes::from_str("0x60808060405234601957602a5f55610106908161001e8239f35b5f80fdfe608060405260043610156010575f80fd5b5f3560e01c80633fb5c1cb1460af5780638381f58a146094578063d09de08a14605e5763d5556544146040575f80fd5b34605a575f366003190112605a5760205f54604051908152f35b5f80fd5b34605a575f366003190112605a576001545f1981146080576001016001555f80f35b634e487b7160e01b5f52601160045260245ffd5b34605a575f366003190112605a576020600154604051908152f35b34605a576020366003190112605a575f54600435810180911160805760015500fea2646970667358221220e470db5efcff30a5d2bf2dfc5c01072c1364af37644d14ea4b2c86293086d86664736f6c634300081e0033").unwrap();

    let contract_deployment_tx = Tx {
        from: Some(wallet.address()),
        to: None, // Contract deployment
        value: U256::ZERO,
        data: Some(deployment_bytecode.clone()),
        nonce: Some(2),
        chain_id: Some(U64::from(31337)),
        gas_limit: None,
        gas_price: Some(20000000000),
        max_fee_per_gas: None,
        max_priority_fee_per_gas: None,
        access_list: None,
        transaction_type: Some(U64::from(0)),
    };

    // Test EVM-based estimation for contract deployment
    let evm_deploy_result = evm_estimator
        .estimate_gas(contract_deployment_tx.clone())
        .await;
    let evm_deploy_estimate = evm_deploy_result.unwrap();

    // Test Alloy provider estimation for contract deployment
    let alloy_deploy_tx = alloy::rpc::types::TransactionRequest::default()
        .from(wallet.address())
        .input(deployment_bytecode.into());

    let alloy_deploy_result = provider.estimate_gas(alloy_deploy_tx).await;
    let alloy_deploy_estimate = alloy_deploy_result.unwrap();

    // Test Case 3: Contract Call
    // Call 1 ------------------------------------------------------
    let call_data = Counter::setNumberCall::new((U256::from(20),));
    let encoded_call_data = Bytes::from(call_data.abi_encode());

    let contract_call_tx = Tx {
        from: Some(wallet.address()),
        to: Some(counter_contract_address),
        value: U256::ZERO,
        data: Some(encoded_call_data.clone()),
        nonce: Some(3),
        chain_id: Some(U64::from(31337)),
        gas_limit: None,
        gas_price: Some(20000000000),
        max_fee_per_gas: Some(30000000000),
        max_priority_fee_per_gas: Some(2000000000),
        access_list: None,
        transaction_type: Some(U64::from(2)), // EIP-1559
    };

    // Test EVM-based estimation for contract call
    let evm_call_result = evm_estimator.estimate_gas(contract_call_tx.clone()).await;
    let evm_call_estimate_1 = evm_call_result.unwrap();

    // Test Alloy provider estimation for contract call
    let alloy_call_tx = alloy::rpc::types::TransactionRequest::default()
        .from(wallet.address())
        .to(counter_contract_address)
        .input(encoded_call_data.into());

    let alloy_call_result = provider.estimate_gas(alloy_call_tx).await;
    let alloy_call_estimate_1 = alloy_call_result.unwrap();

    // Call 2 ------------------------------------------------------
    let call_data = Counter::complexCall::new(());
    let encoded_call_data = Bytes::from(call_data.abi_encode());

    let contract_call_tx = Tx {
        from: Some(wallet.address()),
        to: Some(counter_contract_address),
        value: U256::ZERO,
        data: Some(encoded_call_data.clone()),
        nonce: Some(1),
        chain_id: Some(U64::from(1)),
        gas_limit: None,
        gas_price: Some(20000000000),
        max_fee_per_gas: Some(30000000000),
        max_priority_fee_per_gas: Some(2000000000),
        access_list: None,
        transaction_type: Some(U64::from(2)), // EIP-1559
    };

    // Test EVM-based estimation for contract call
    let evm_call_result = evm_estimator.estimate_gas(contract_call_tx.clone()).await;
    let evm_call_estimate_2 = evm_call_result.unwrap();

    // Test Alloy provider estimation for contract call
    let alloy_call_tx = alloy::rpc::types::TransactionRequest::default()
        .from(wallet.address())
        .to(counter_contract_address)
        .input(encoded_call_data.into());

    let alloy_call_result = provider.estimate_gas(alloy_call_tx).await;
    let alloy_call_estimate_2 = alloy_call_result.unwrap();

    // Call 3 ------------------------------------------------------
    let call_data = Caller::precompileCall::new((U256::from(123456),));
    let encoded_call_data = Bytes::from(call_data.abi_encode());

    let contract_call_tx = Tx {
        from: Some(wallet.address()),
        to: Some(caller_contract_address),
        value: U256::ZERO,
        data: Some(encoded_call_data.clone()),
        nonce: Some(1),
        chain_id: Some(U64::from(1)),
        gas_limit: None,
        gas_price: Some(20000000000),
        max_fee_per_gas: Some(30000000000),
        max_priority_fee_per_gas: Some(2000000000),
        access_list: None,
        transaction_type: Some(U64::from(2)), // EIP-1559
    };

    // Test EVM-based estimation for contract call
    let evm_call_result = evm_estimator.estimate_gas(contract_call_tx.clone()).await;
    let evm_call_estimate_3 = evm_call_result.unwrap();

    // Test Alloy provider estimation for contract call
    let alloy_call_tx = alloy::rpc::types::TransactionRequest::default()
        .from(wallet.address())
        .to(caller_contract_address)
        .input(encoded_call_data.into());

    let alloy_call_result = provider.estimate_gas(alloy_call_tx).await;
    let alloy_call_estimate_3 = alloy_call_result.unwrap();

    // Call 4 ------------------------------------------------------
    let call_data = Caller::call_counterCall::new((counter_contract_address,));
    let encoded_call_data = Bytes::from(call_data.abi_encode());

    let contract_call_tx = Tx {
        from: Some(wallet.address()),
        to: Some(caller_contract_address),
        value: U256::ZERO,
        data: Some(encoded_call_data.clone()),
        nonce: Some(1),
        chain_id: Some(U64::from(1)),
        gas_limit: None,
        gas_price: Some(20000000000),
        max_fee_per_gas: Some(30000000000),
        max_priority_fee_per_gas: Some(2000000000),
        access_list: None,
        transaction_type: Some(U64::from(2)), // EIP-1559
    };

    // Test EVM-based estimation for contract call
    let evm_call_result = evm_estimator.estimate_gas(contract_call_tx.clone()).await;
    let evm_call_estimate_4 = evm_call_result.unwrap();

    // Test Alloy provider estimation for contract call
    let alloy_call_tx = alloy::rpc::types::TransactionRequest::default()
        .from(wallet.address())
        .to(caller_contract_address)
        .input(encoded_call_data.into());

    let alloy_call_result = provider.estimate_gas(alloy_call_tx).await;
    let alloy_call_estimate_4 = alloy_call_result.unwrap();

    // Summary and comparison
    println!("\nSummary Comparison:");
    println!("Simple Transfer:");
    println!("  - EVM-based: {} gas", evm_transfer_estimate.estimated_gas);
    println!("  - Alloy Provider: {} gas", alloy_transfer_estimate);

    println!("Contract Deployment:");
    println!("  - EVM-based: {} gas", evm_deploy_estimate.estimated_gas);
    println!("  - Alloy Provider: {} gas", alloy_deploy_estimate);

    println!("Contract Call 1:");
    println!("  - EVM-based: {} gas", evm_call_estimate_1.estimated_gas);
    println!("  - Alloy Provider: {} gas", alloy_call_estimate_1);

    println!("Contract Call 2:");
    println!("  - EVM-based: {} gas", evm_call_estimate_2.estimated_gas);
    println!("  - Alloy Provider: {} gas", alloy_call_estimate_2);

    println!("Contract Call 3:");
    println!("  - EVM-based: {} gas", evm_call_estimate_3.estimated_gas);
    println!("  - Alloy Provider: {} gas", alloy_call_estimate_3);

    println!("Contract Call 4:");
    println!("  - EVM-based: {} gas", evm_call_estimate_4.estimated_gas);
    println!("  - Alloy Provider: {} gas", alloy_call_estimate_4);

    println!("\nAll gas estimation approaches tested successfully!");
}
