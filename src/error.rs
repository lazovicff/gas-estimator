use alloy::transports::{RpcError, TransportError, TransportErrorKind};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Alloy Transport Error: {0}")]
    TransportError(TransportError),
    #[error("Alloy Rpc Error: {0}")]
    RpcError(RpcError<TransportErrorKind>),
}
