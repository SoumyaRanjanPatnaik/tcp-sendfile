use thiserror::Error;

use crate::{connection::StreamReadError, transport::TransportError};

#[derive(Error, Debug)]
pub enum SendFileError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Transport error: {0}")]
    Transport(#[from] TransportError),
    #[error("Invalid address format: {0}")]
    InvalidAddress(#[from] std::net::AddrParseError),
    #[error("Error when trying to read from TCP stream: {0}")]
    Stream(#[from] StreamReadError),
    #[error("Unexpected message received: {received}, expected: {expected}")]
    UnexpectedMessage { received: String, expected: String },
}
