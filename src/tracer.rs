use revm::bytecode::opcode;
use revm::context::ContextTr;
use revm::inspector::JournalExt;
use revm::interpreter::interpreter_types::Jumps;
use revm::interpreter::{CallInputs, CallOutcome, Interpreter};
use revm::primitives::B256;
use revm::{
    inspector::Inspector,
    primitives::{Address, U256},
};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct Tracer {
    pub contract_addresses: HashSet<Address>,
    pub storage_accesses: HashMap<Address, U256>,
    address_stack: Vec<Address>,
    // Keep track of historical accesses
    storage_access_archive: HashMap<Address, U256>,
    contract_addresses_archive: HashSet<Address>,
}

impl Tracer {
    pub fn new() -> Self {
        Self {
            contract_addresses: HashSet::new(),
            storage_accesses: HashMap::new(),
            address_stack: Vec::new(),

            storage_access_archive: HashMap::new(),
            contract_addresses_archive: HashSet::new(),
        }
    }

    pub fn has_new_accesses(&self) -> bool {
        self.contract_addresses.len() > 0 || self.storage_accesses.len() > 0
    }

    pub fn reset_state(&mut self) {
        self.storage_access_archive
            .extend(self.storage_accesses.iter());
        self.contract_addresses_archive
            .extend(self.contract_addresses.iter());

        self.storage_accesses.clear();
        self.contract_addresses.clear();
    }
}

impl Default for Tracer {
    fn default() -> Self {
        Self::new()
    }
}

impl<CTX> Inspector<CTX> for Tracer
where
    CTX: ContextTr<Journal: JournalExt>,
{
    // IMPROVEMENT:
    // Since CTX is available on every call
    // We can dinamically fetch account code and balance to populate DB
    fn call(&mut self, _context: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        if !self
            .contract_addresses_archive
            .contains(&inputs.target_address)
        {
            self.contract_addresses.insert(inputs.target_address);
        }
        self.address_stack.push(inputs.target_address);
        None
    }

    fn call_end(&mut self, _context: &mut CTX, _inputs: &CallInputs, _outcome: &mut CallOutcome) {
        self.address_stack.pop();
    }

    // IMPROVEMENT:
    // Since CTX is available on every call
    // We can dinamically fetch storage variables and populate the DB
    fn step(&mut self, interpreter: &mut Interpreter, _context: &mut CTX) {
        // Get the current opcode from the bytecode
        match interpreter.bytecode.opcode() {
            // SLOAD - Load from storage (opcode 0x54)
            opcode::SLOAD => {
                if let Ok(slot) = interpreter.stack.peek(0) {
                    if let Some(address) = self.address_stack.last() {
                        if !self.storage_access_archive.contains_key(address) {
                            self.storage_accesses.insert(*address, slot);
                        }
                    }
                }
            }
            opcode::DELEGATECALL | opcode::CALL | opcode::STATICCALL | opcode::CALLCODE => {
                let slot = interpreter.stack.peek(1).unwrap();
                let addr = Address::from_word(B256::from(slot.to_be_bytes()));
                println!("Calling address: {:?}", addr);
            }
            _ => {}
        }
    }
}
