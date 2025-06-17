use revm::context::ContextTr;
use revm::inspector::JournalExt;
use revm::interpreter::interpreter_types::Jumps;
use revm::interpreter::{CallInputs, CallOutcome, Interpreter};
use revm::{
    inspector::Inspector,
    primitives::{Address, U256},
    Database,
};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct StorageAccess {
    pub address: Address,
    pub slot: U256,
}

#[derive(Debug, Clone)]
pub struct Tracer {
    contract_addresses: HashSet<Address>,
    storage_accesses: Vec<StorageAccess>,
    current_address: Option<Address>,
}

impl Tracer {
    pub fn new() -> Self {
        Self {
            contract_addresses: HashSet::new(),
            storage_accesses: Vec::new(),
            current_address: None,
        }
    }

    pub fn get_contract_addresses(&self) -> &HashSet<Address> {
        &self.contract_addresses
    }

    pub fn get_storage_accesses(&self) -> &[StorageAccess] {
        &self.storage_accesses
    }

    pub fn get_unique_slots(&self) -> HashSet<(Address, U256)> {
        self.storage_accesses
            .iter()
            .map(|access| (access.address, access.slot))
            .collect()
    }

    pub fn record_storage_access(&mut self, slot: U256) {
        if let Some(address) = self.current_address {
            let access = StorageAccess { address, slot };
            self.storage_accesses.push(access);
        }
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
    fn call(&mut self, _context: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        self.contract_addresses.insert(inputs.target_address);
        self.current_address = Some(inputs.target_address);
        None
    }

    fn call_end(&mut self, _context: &mut CTX, _inputs: &CallInputs, _outcome: &mut CallOutcome) {
        self.current_address = None;
    }

    fn step(&mut self, interpreter: &mut Interpreter, _context: &mut CTX) {
        // Get the current opcode from the bytecode
        if let Some(opcode) = interpreter
            .bytecode
            .bytecode()
            .get(interpreter.bytecode.pc())
        {
            match *opcode {
                // SLOAD - Load from storage (opcode 0x54)
                0x54 => {
                    if let Ok(slot) = interpreter.stack.peek(0) {
                        self.record_storage_access(slot);
                    }
                }
                _ => {}
            }
        }
    }
}
