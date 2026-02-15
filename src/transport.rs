//! Transport layer for the custom file transfer protocol.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CURRENT_PROTOCOL_VERSION: u8 = 1;
pub const MAX_BLOCK_SIZE: u32 = 4 * 1024 * 1024; // 4 MB
pub const MAX_MESSAGE_SIZE: usize = MAX_BLOCK_SIZE as usize + 128; // Max block size plus some overhead for headers and metadata

pub const VERSION_HEADER_PREFIX_STR: &str = "Ver: ";
pub const LENGTH_HEADER_PREFIX_STR: &str = "Len: ";
pub const VERSION_HEADER_PRIFIX: &[u8] = VERSION_HEADER_PREFIX_STR.as_bytes();
pub const LENGTH_HEADER_PREFIX: &[u8] = LENGTH_HEADER_PREFIX_STR.as_bytes();

pub const MESSAGE_DELIMITER_STR: &str = "\r\n";
pub const MESSAGE_DELIMITER: &[u8] = MESSAGE_DELIMITER_STR.as_bytes();

#[derive(Error, Debug)]
pub enum TransportError {
    #[error("Serialization error: {0}")]
    Serialization(#[from] postcard::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub struct SerializedMessage {
    pub length: usize,
    pub payload: &'static [u8],
}

/// Messages sent from the Sender (the one sending the file) to the Receiver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SenderMessageV1<'a> {
    /// Initial handshake message sent from sender to receiver to establish transfer parameters.
    Handshake {
        /// SHA-256 hash of the file being transferred, used for integrity verification,
        /// and deduplication on the receiver side.
        file_hash: &'a [u8],

        /// Total size of the file in bytes, used for progress tracking and pre-allocation
        /// on the receiver side.
        total_size: u64,

        /// Number of concurrent connections for transferring the file
        concurrency: u16,

        /// Original file name, used for allocating the file on the receiver side and for display purposes.
        file_name: &'a str,

        /// Size of each data block in bytes, used for splitting the file into chunks and for progress tracking.
        block_size: u32,
    },

    /// A chunk of file data being sent from sender to receiver.
    Data {
        /// Sequence number of the chunk being sent, used for tracking which chunks have been sent and received.
        seq: u32,
        /// Checksum of the chunk data, used for integrity verification on the receiver side.
        checksum: u32,
        /// SHA-256 hash of the file this data belongs to.
        file_hash: &'a [u8],
        /// Actual chunk data being sent, with length specified in the Len header of the message.
        data: &'a [u8],
    },

    /// A request from sender to receiver to report the current progress of the transfer, including total bytes received so far.
    ProgressRequest {
        /// SHA-256 hash of the file being tracked.
        file_hash: &'a [u8],
    },

    /// An error message sent from the sender to indicate a problem.
    Error { code: u16, message: String },
}

impl<'a> SenderMessageV1<'a> {
    /// Serializes the message into a byte vector using postcard.
    pub fn to_bytes<'b>(&self, buffer: &'b mut [u8]) -> Result<&'b mut [u8], TransportError> {
        let message = postcard::to_slice(&self, buffer)?;
        Ok(message)
    }

    /// Deserializes a message from a byte slice using postcard.
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self, TransportError> {
        Ok(postcard::from_bytes(bytes)?)
    }
}

/// Messages sent from the Receiver (the one receiving the file) to the Sender.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReceiverMessageV1 {
    /// A request from receiver to sender to send a specific chunk of the file.
    Request {
        /// SHA-256 hash of the file being requested.
        file_hash: [u8; 32],
        /// Sequence number of the chunk being requested, used for tracking which chunks have been sent and received.
        /// In case of retransmissions, the receiver may request the same chunk multiple times until it is
        /// successfully received and verified.
        seq: u32,
    },

    /// A response from receiver to sender with the total bytes received so far.
    /// Used for progress tracking and retransmission decisions on the sender side.
    ProgressResponse {
        /// SHA-256 hash of the file being tracked.
        file_hash: [u8; 32],
        bytes_received: u64,
    },

    /// A signal from the receiver that the file has been successfully received and verified.
    /// This marks the completion of the transfer for this connection.
    TransferComplete {
        /// SHA-256 hash of the file that was completed.
        file_hash: [u8; 32],
    },

    /// An error message sent from the receiver to indicate a problem.
    Error { code: u16, message: String },
}

impl ReceiverMessageV1 {
    /// Serializes the message into a byte vector using postcard.
    pub fn to_bytes<'b>(&self, buffer: &'b mut [u8]) -> Result<&'b mut [u8], TransportError> {
        let message = postcard::to_slice(&self, buffer)?;
        Ok(message)
    }

    /// Deserializes a message from a byte slice using postcard.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TransportError> {
        Ok(postcard::from_bytes(bytes)?)
    }
}

pub fn attach_headers(payload: &[u8]) -> Box<[u8]> {
    let mut message = Vec::with_capacity(64 + payload.len());
    message.extend_from_slice(
        format!(
            //Ver: [PROTOCOL_VERSION]\r\n
            "{VERSION_HEADER_PREFIX_STR}: {}{MESSAGE_DELIMITER_STR}",
            CURRENT_PROTOCOL_VERSION
        )
        .as_bytes(),
    );
    message.extend_from_slice(
        format!(
            //Len: [length of payload]\r\n
            "{LENGTH_HEADER_PREFIX_STR}: {}{MESSAGE_DELIMITER_STR}",
            payload.len()
        )
        .as_bytes(),
    );
    // Headers are separated from the payload by an additional delimiter
    message.extend_from_slice(MESSAGE_DELIMITER);

    // Append the actual message payload after the headers
    message.extend_from_slice(payload);
    message.into_boxed_slice()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handshake_serde() {
        let msg = SenderMessageV1::Handshake {
            file_hash: &[0xAA; 32],
            total_size: 1024 * 1024,
            concurrency: 8,
            file_name: "test_file.txt",
            block_size: MAX_BLOCK_SIZE,
        };

        let mut buffer = [0u8; 1024]; // Large enough buffer for serialization
        let serialized = msg.to_bytes(&mut buffer).expect("Failed to serialize");
        let decoded = SenderMessageV1::from_bytes(&serialized).expect("Failed to deserialize");

        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_transfer_complete_serde() {
        let msg = ReceiverMessageV1::TransferComplete {
            file_hash: [0xBB; 32],
        };
        let mut buffer = [0u8; 1024];
        let serialized = msg.to_bytes(&mut buffer).expect("Failed to serialize");
        let decoded = ReceiverMessageV1::from_bytes(&serialized).expect("Failed to deserialize");

        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_data_serde() {
        let data_payload = [1, 2, 3, 4, 5];
        let msg = SenderMessageV1::Data {
            seq: 10,
            checksum: 0xDEADBEEF,
            file_hash: &[0xAA; 32],
            data: &data_payload,
        };

        let mut buffer = [0u8; 1024]; // Large enough buffer for serialization
        let serialized = msg.to_bytes(&mut buffer).expect("Failed to serialize");
        let decoded = SenderMessageV1::from_bytes(&serialized).expect("Failed to deserialize");

        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_error_serde() {
        let msg = SenderMessageV1::Error {
            code: 404,
            message: "File not found".to_string(),
        };

        let mut buffer = [0u8; 1024]; // Large enough buffer for serialization
        let serialized = msg.to_bytes(&mut buffer).expect("Failed to serialize");
        let decoded = SenderMessageV1::from_bytes(&serialized).expect("Failed to deserialize");

        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_progress_request_serde() {
        let msg = SenderMessageV1::ProgressRequest {
            file_hash: &[0xCC; 32],
        };

        let mut buffer = [0u8; 1024]; // Large enough buffer for serialization
        let serialized = msg.to_bytes(&mut buffer).expect("Failed to serialize");
        let decoded = SenderMessageV1::from_bytes(&serialized).expect("Failed to deserialize");

        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_progress_response_serde() {
        let msg = ReceiverMessageV1::ProgressResponse {
            file_hash: [0xDD; 32],
            bytes_received: 512 * 1024,
        };
        let mut buffer = [0u8; 1024]; // Large enough buffer for serialization
        let serialized = msg.to_bytes(&mut buffer).expect("Failed to serialize");
        let decoded = ReceiverMessageV1::from_bytes(&serialized).expect("Failed to deserialize ");

        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_request_serde() {
        let msg = ReceiverMessageV1::Request {
            file_hash: [0xEE; 32],
            seq: 42,
        };
        let mut buffer = [0u8; 1024]; // Large enough buffer for
                                      // serialization
        let serialized = msg.to_bytes(&mut buffer).expect("Failed to serialize");
        let decoded = ReceiverMessageV1::from_bytes(&serialized).expect("Failed to deserialize");

        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_large_data_serde() {
        let data_payload = vec![0xFF; MAX_BLOCK_SIZE as usize];
        let msg = SenderMessageV1::Data {
            seq: 100,
            checksum: 0xBEEFDEAD,
            file_hash: &[0xFF; 32],
            data: &data_payload,
        };

        let mut buffer = vec![0u8; (MAX_BLOCK_SIZE + 512) as usize]; // Large enough buffer for serialization
        let serialized = msg.to_bytes(&mut buffer).expect("Failed to serialize");
        let decoded = SenderMessageV1::from_bytes(&serialized).expect("Failed to deserialize");

        assert_eq!(msg, decoded);
    }
}
