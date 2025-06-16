use revm::primitives::{Address, Bytes, FixedBytes, U256};
use std::collections::HashSet;

use crate::estimators::Tx;

/// Calculate gas cost for calldata (transaction input data)
pub fn calculate_calldata_cost(data: &Bytes) -> u128 {
    let mut cost = 0;

    for byte in data.iter() {
        if *byte == 0 {
            // Zero bytes cost 4 gas each
            cost += 4;
        } else {
            // Non-zero bytes cost 16 gas each
            cost += 16;
        }
    }

    cost
}

/// Estimate storage operations cost with cold/warm slot tracking
pub fn estimate_storage_cost(data: &Bytes, initial_warm_slots: HashSet<FixedBytes<32>>) -> u128 {
    let mut cost = 0;
    let mut warm_slots: HashSet<FixedBytes<32>> = initial_warm_slots;
    let data_bytes = data.as_ref();
    let mut i = 0;

    while i < data_bytes.len() {
        match data_bytes[i] {
            0x55 => {
                // SSTORE opcode - storage write
                // Try to extract the storage slot from the previous PUSH operations
                let slot = extract_storage_slot(data_bytes, i);

                if warm_slots.contains(&slot) {
                    // Warm storage slot - cheaper write
                    cost += 100; // WARM_STORAGE_READ_COST
                } else {
                    // Cold storage slot - expensive first access
                    cost += 2_100; // COLD_SLOAD_COST
                    warm_slots.insert(slot);

                    // Additional cost for setting new storage (vs modifying existing)
                    // In practice, this would require checking if slot is zero
                    cost += 20_000; // SSTORE_SET_COST (new storage)
                }
                i += 1;
            }
            0x54 => {
                // SLOAD opcode - storage read
                let slot = extract_storage_slot(data_bytes, i);

                if warm_slots.contains(&slot) {
                    cost += 100; // WARM_STORAGE_READ_COST
                } else {
                    cost += 2_100; // COLD_SLOAD_COST
                    warm_slots.insert(slot);
                }
                i += 1;
            }
            _ => i += 1,
        }
    }

    cost
}

/// Extract storage slot from bytecode (simplified heuristic)
fn extract_storage_slot(data: &[u8], sstore_pos: usize) -> FixedBytes<32> {
    // Look backwards for PUSH instructions to find the storage slot
    // This is a simplified approach - real implementation would need stack simulation
    let mut slot = FixedBytes::<32>::ZERO;
    let start = if sstore_pos >= 34 { sstore_pos - 34 } else { 0 };

    for i in start..sstore_pos {
        if data[i] >= 0x60 && data[i] <= 0x7f {
            // PUSH1 to PUSH32
            let push_size = (data[i] - 0x60 + 1) as usize;
            if i + push_size < sstore_pos {
                // Extract up to 32 bytes as slot identifier
                let end = (i + push_size + 1).min(data.len());
                if end > i + 1 {
                    let bytes_to_take = (end - i - 1).min(32);
                    let mut slot_bytes = [0u8; 32];
                    let start_offset = 32 - bytes_to_take;
                    for j in 0..bytes_to_take {
                        if i + 1 + j < data.len() {
                            slot_bytes[start_offset + j] = data[i + 1 + j];
                        }
                    }
                    slot = FixedBytes::<32>::from_slice(&slot_bytes);
                }
            }
        }
    }

    slot
}

/// Calculate contract creation cost
pub fn calculate_contract_creation_cost(data: Option<&Bytes>) -> u128 {
    if let Some(bytecode) = data {
        // Base cost for contract creation
        let mut cost = 32_000;
        // Additional cost per byte of bytecode
        cost += bytecode.len() as u128 * 200;
        cost
    } else {
        0
    }
}

/// Estimate execution cost by analyzing opcodes
pub fn estimate_execution_cost(data: &Bytes) -> u128 {
    let mut cost = 0;
    let data_bytes = data.as_ref();
    let mut i = 0;

    let mut current_registers = Vec::new();
    while i < data_bytes.len() {
        let opcode = data_bytes[i];

        cost += match opcode {
            // Arithmetic operations
            0x01..=0x0b => 3, // ADD, MUL, SUB, DIV, etc.
            // Comparison operations
            0x10..=0x1d => 3, // LT, GT, SLT, SGT, EQ, etc.
            // SHA3
            0x20 => 30,
            // Environmental operations
            0x30..=0x3f => 2, // ADDRESS, BALANCE, ORIGIN, etc.
            // Block operations
            0x40..=0x48 => 20, // BLOCKHASH, COINBASE, etc.
            // Stack operations
            0x50..=0x5f => 3, // POP, MLOAD, MSTORE, etc.
            // Push operations
            0x60..=0x7f => {
                let size = (opcode - 0x60 + 1) as usize;

                let mut bytes = Vec::new();
                for byte_i in i..(i + size) {
                    bytes.push(data_bytes[byte_i]);
                }
                current_registers = bytes;
                i += size; // Skip the pushed bytes
                3
            }
            // Duplication operations
            0x80..=0x8f => 3,
            // Exchange operations
            0x90..=0x9f => 3,
            // Logging operations
            0xa0..=0xa4 => 375, // LOG0, LOG1, etc.
            // CALL-like operations
            0xf1 => 700, // CALL
            0xf2 => 700, // CALLCODE
            0xf4 => 700, // DELEGATECALL
            0xfa => 700, // STATICCALL

            // Other system operations
            0xf0 => 32_000, // CREATE
            0xf3 => 0,      // RETURN
            0xf5 => 32_000, // CREATE2
            0xfd => 0,      // REVERT
            0xff => 5_000,  // SELFDESTRUCT
            // Default case
            _ => 1,
        };

        i += 1;
    }

    cost
}

/// Estimate precompile costs
pub fn estimate_precompile_cost(data: &Bytes, to: Option<Address>) -> U256 {
    let mut cost = U256::ZERO;
    let data_bytes = data.as_ref();

    // Check for precompile addresses in the bytecode or direct calls
    if let Some(address) = to {
        let addr_u64 = address.as_slice()[19]; // Last byte for precompile check
        match addr_u64 {
            0x01 => cost += U256::from(3_000), // ECDSA recovery
            0x02 => cost += U256::from(60 + (data_bytes.len() as u64 + 31) / 32 * 12), // SHA256
            0x03 => cost += U256::from(600 + (data_bytes.len() as u64 + 31) / 32 * 120), // RIPEMD160
            0x04 => cost += U256::from(15 + (data_bytes.len() as u64 + 31) / 32 * 3),    // Identity
            0x05 => cost += U256::from(estimate_modexp_cost(data_bytes)),                // ModExp
            0x06 => cost += U256::from(150),    // BN254 Add
            0x07 => cost += U256::from(6_000),  // BN254 Mul
            0x08 => cost += U256::from(45_000), // BN254 Pairing base
            0x09 => cost += U256::from(50_000), // Blake2F
            _ => {}
        }
    }

    // Look for CALL opcodes to precompile addresses in bytecode
    let mut i = 0;
    while i + 20 < data_bytes.len() {
        if data_bytes[i] == 0xf1 {
            // CALL opcode
            // Simple heuristic: look for small addresses that might be precompiles
            for j in 1..=20 {
                if i >= j && data_bytes[i - j] <= 0x09 && data_bytes[i - j] > 0 {
                    cost += U256::from(700); // Base call cost + estimated precompile cost
                    break;
                }
            }
        }
        i += 1;
    }

    cost
}

/// Estimate ModExp precompile cost
pub fn estimate_modexp_cost(data: &[u8]) -> u64 {
    if data.len() < 96 {
        return 200; // Minimum cost
    }

    // Simplified calculation - in practice this would parse the input more carefully
    let base_len = if data.len() >= 32 {
        u64::from_be_bytes([0, 0, 0, 0, 0, 0, 0, data[31]])
    } else {
        32
    };
    let exp_len = if data.len() >= 64 {
        u64::from_be_bytes([0, 0, 0, 0, 0, 0, 0, data[63]])
    } else {
        32
    };
    let mod_len = if data.len() >= 96 {
        u64::from_be_bytes([0, 0, 0, 0, 0, 0, 0, data[95]])
    } else {
        32
    };

    let max_len = base_len.max(mod_len);
    let complexity = (max_len * max_len) / 64;

    200 + complexity * exp_len / 20
}

/// Calculate access list cost (EIP-2930)
pub fn calculate_access_list_cost(tx_params: &Tx) -> (u128, HashSet<FixedBytes<32>>) {
    // Simple heuristic: estimate potential access list items
    let mut cost = 0;
    let mut loaded_slots = HashSet::new();

    assert!(tx_params.access_list.is_some());
    let access_list = tx_params.access_list.clone().unwrap();
    let mut filtered_list = Vec::new();
    for access_item in access_list.0 {
        if access_item.address == tx_params.to.unwrap() {
            filtered_list.push(access_item);
        }
    }

    for access_item in filtered_list {
        for storage_key in access_item.storage_keys {
            cost += 2_100;
            loaded_slots.insert(storage_key);
        }
    }
    cost += 2400; // 1 address access cost;

    (cost, loaded_slots)
}
