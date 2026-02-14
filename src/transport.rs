//! Transport layer for the custom file transfer protocol.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_BLOCK_SIZE: u32 = 4 * 1024 * 1024; // 4 MB

#[derive(Error, Debug)]
pub enum TransportError {
    #[error("Serialization error: {0}")]
    Serialization(#[from] postcard::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Represents a message in the custom file transfer protocol.
///
/// Message format as sent on the wire:
/// ```text
/// Ver: [PROTOCOL_VERSION]\r\n
/// Len: [length of payload]\r\n
/// \r\n
/// [serialized TransportMessage with length specified in Len header]
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportMessageV1<'a> {
    /// Initial handshake message sent from sender to receiver to establish transfer parameters.
    /// Reciever responds with another handshake message to confirm parameters and start the transfer.
    Handshake {
        /// SHA-256 hash of the file being transferred, used for integrity verification,
        /// and deduplication on the receiver side.
        file_hash: [u8; 32],

        /// Total size of the file in bytes, used for progress tracking and pre-allocation
        /// on the receiver side.
        total_size: u64,

        /// Number of concurrent connections for transferring the file
        concurrency: u16,

        /// Original file name, used for allocating the file on the receiver side and for display purposes.
        file_name: String,

        /// Size of each data block in bytes, used for splitting the file into chunks and for progress tracking.
        block_size: u32,
    },

    /// A request from receiver to sender to send a specific chunk of the file.
    Request {
        /// Sequence number of the chunk being requested, used for tracking which chunks have been sent and received.
        /// In case of retransmissions, the receiver may request the same chunk multiple times until it is
        /// successfully received and verified.
        seq: u32,
    },

    /// A chunk of file data being sent from sender to receiver.
    Data {
        /// Sequence number of the chunk being sent, used for tracking which chunks have been sent and received.
        seq: u32,
        /// Checksum of the chunk data, used for integrity verification on the receiver side.
        checksum: u32,
        /// Actual chunk data being sent, with length specified in the Len header of the message.
        data: &'a [u8],
    },

    /// A requet from sender to receiver to report the current progress of the transfer, including total bytes received so far.
    ProgressRequest,

    /// A response from receiver to sender with the total bytes received so far.
    /// Used for progress tracking and retransmission decisions on the sender side.
    ProgressResponse { bytes_received: u64 },

    /// An error message sent from either side to indicate a problem with the transfer,
    /// such as an invalid request, checksum failure, or other issues.
    Error { code: u16, message: String },
}

impl<'a> TransportMessageV1<'a> {
    /// Serializes the message into a byte vector using postcard.
    pub fn to_bytes(&self) -> Result<Vec<u8>, TransportError> {
        Ok(postcard::to_allocvec(self)?)
    }

    /// Deserializes a message from a byte slice using postcard.
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self, TransportError> {
        Ok(postcard::from_bytes(bytes)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handshake_serde() {
        let msg = TransportMessageV1::Handshake {
            file_hash: [0xAA; 32],
            total_size: 1024 * 1024,
            concurrency: 8,
            file_name: "test_file.txt".to_string(),
            block_size: MAX_BLOCK_SIZE,
        };

        let serialized = msg.to_bytes().expect("Failed to serialize");
        let decoded = TransportMessageV1::from_bytes(&serialized).expect("Failed to deserialize");

        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_data_serde() {
        let data_payload = vec![1, 2, 3, 4, 5];
        let msg = TransportMessageV1::Data {
            seq: 10,
            checksum: 0xDEADBEEF,
            data: &data_payload,
        };

        let serialized = msg.to_bytes().expect("Failed to serialize");
        let decoded = TransportMessageV1::from_bytes(&serialized).expect("Failed to deserialize");

        assert_eq!(msg, decoded);
    }
}
