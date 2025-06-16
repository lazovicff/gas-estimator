use revm::primitives::Bytes;

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
