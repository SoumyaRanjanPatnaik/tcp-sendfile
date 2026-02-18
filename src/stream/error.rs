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
    /// File hash mismatch.
    #[error("Block hash mismatch: expected {:?}, got {:?}", expected, received)]
    BlockHashMismatch {
        expected: [u8; 32],
        received: Vec<u8>,
    },
    /// Checksum mismatch for a data block.
    #[error("Checksum mismatch for block {seq}: expected {expected}, got {computed}")]
    ChecksumMismatch {
        seq: u32,
        expected: u32,
        computed: u32,
    },
    /// Invalid request received.
    #[error("Invalid request: {0}")]
    InvalidRequest(String),
    /// Connection failed.
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error(
        "Integrity check failed. Expected hash: {:?}, received hash: {:?}",
        expected,
        received
    )]
    IntegrityCheckFailed {
        expected: [u8; 32],
        received: [u8; 32],
    },
}
