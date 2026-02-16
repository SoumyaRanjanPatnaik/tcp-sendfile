use thiserror::Error;

use crate::{connection::StreamReadError, transport::TransportError};

/// Errors that can occur during file transfer (sending or receiving).
#[derive(Error, Debug)]
pub enum SendFileError {
    /// An I/O error occurred.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// A transport layer error occurred (serialization/deserialization).
    #[error("Transport error: {0}")]
    Transport(#[from] TransportError),
    /// The provided address was invalid.
    #[error("Invalid address format: {0}")]
    InvalidAddress(#[from] std::net::AddrParseError),
    /// Error reading from the TCP stream.
    #[error("Error when trying to read from TCP stream: {0}")]
    Stream(#[from] StreamReadError),
    /// Received an unexpected message type.
    #[error("Unexpected message received: {received}, expected: {expected}")]
    UnexpectedMessage { received: String, expected: String },
}
