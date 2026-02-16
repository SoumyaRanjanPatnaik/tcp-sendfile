//! Transport layer for the custom file transfer protocol.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The current version of the file transfer protocol.
pub const CURRENT_PROTOCOL_VERSION: u8 = 1;
/// The maximum size of a file block (4 MB).
pub const MAX_BLOCK_SIZE: u32 = 4 * 1024 * 1024; // 4 MB
/// The maximum size of a message, including overhead for headers and metadata.
pub const MAX_MESSAGE_SIZE: usize = MAX_BLOCK_SIZE as usize + 128; // Max block size plus some overhead for headers and metadata

/// The string prefix for the version header.
pub const VERSION_HEADER_PREFIX_STR: &str = "Ver: ";
/// The string prefix for the length header.
pub const LENGTH_HEADER_PREFIX_STR: &str = "Len: ";
/// The byte slice prefix for the version header.
pub const VERSION_HEADER_PRIFIX: &[u8] = VERSION_HEADER_PREFIX_STR.as_bytes();
/// The byte slice prefix for the length header.
pub const LENGTH_HEADER_PREFIX: &[u8] = LENGTH_HEADER_PREFIX_STR.as_bytes();

/// The string delimiter for messages (CRLF).
pub const MESSAGE_DELIMITER_STR: &str = "\r\n";
/// The byte slice delimiter for messages (CRLF).
pub const MESSAGE_DELIMITER: &[u8] = MESSAGE_DELIMITER_STR.as_bytes();

/// Errors that can occur in the transport layer.
#[derive(Error, Debug)]
pub enum TransportError {
    /// Serialization or deserialization failed.
    #[error("Serialization error: {0}")]
    Serialization(#[from] postcard::Error),
    /// An I/O error occurred.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// A serialized message ready to be sent.
pub struct SerializedMessage {
    /// The length of the payload.
    pub length: usize,
    /// The message payload.
    pub payload: &'static [u8],
}

/// Handshake message sent by the sender to initiate a transfer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandshakeV1<'a> {
    /// BLAKE3 hash of the file being transferred, used for integrity verification,
    /// and deduplication on the receiver side.
    pub file_hash: &'a [u8],

    /// Total size of the file in bytes, used for progress tracking and pre-allocation
    /// on the receiver side.
    pub total_size: u64,

    /// Number of concurrent connections for transferring the file
    pub concurrency: u16,

    /// Original file name, used for allocating the file on the receiver side and for display purposes.
    pub file_name: &'a str,

    /// Size of each data block in bytes, used for splitting the file into chunks and for progress tracking.
    pub block_size: u32,
}

/// Data chunk message sent by the sender.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataV1<'a> {
    /// Sequence number of the chunk being sent, used for tracking which chunks have been sent and received.
    pub seq: u32,
    /// Checksum of the chunk data, used for integrity verification on the receiver side.
    pub checksum: u32,
    /// BLAKE3 hash of the file this data belongs to.
    pub file_hash: &'a [u8],
    /// Whether the data is compressed using gzip.
    pub compressed: bool,
    /// Actual chunk data being sent, with length specified in the Len header of the message.
    pub data: &'a [u8],
}

/// Error message sent by the sender.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SenderErrorV1 {
    /// Error code.
    pub code: u16,
    /// Error message.
    pub message: String,
}

/// Messages sent from the Sender (the one sending the file) to the Receiver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SenderMessageV1<'a> {
    /// Initial handshake message sent from sender to receiver to establish transfer parameters.
    Handshake(#[serde(borrow)] HandshakeV1<'a>),

    /// A chunk of file data being sent from sender to receiver.
    Data(#[serde(borrow)] DataV1<'a>),

    /// An error message sent from the sender to indicate a problem.
    Error(SenderErrorV1),

    /// A response to a VerifyBlock request, indicating if the block checksum matches.
    VerifyResponse(VerifyResponseV1),
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

/// Request message sent by the receiver to request a data chunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestV1 {
    /// BLAKE3 hash of the file being requested.
    pub file_hash: [u8; 32],
    /// Sequence number of the chunk being requested, used for tracking which chunks have been sent and received.
    /// In case of retransmissions, the receiver may request the same chunk multiple times until it is
    /// successfully received and verified.
    pub seq: u32,
}

/// Progress update message sent by the receiver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressV1 {
    /// BLAKE3 hash of the file being tracked.
    pub file_hash: [u8; 32],
    pub bytes_received: u64,
}

/// Message sent by the receiver when the transfer is complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferCompleteV1 {
    /// BLAKE3 hash of the file that was completed.
    pub file_hash: [u8; 32],
}

/// Error message sent by the receiver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiverErrorV1 {
    /// Error code.
    pub code: u16,
    /// Error message.
    pub message: String,
}

/// Request to verify a block's checksum, sent by the receiver (e.g. during resume).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyBlockV1 {
    /// BLAKE3 hash of the file.
    pub file_hash: [u8; 32],
    /// Sequence number of the block to verify.
    pub seq: u32,
    /// Checksum calculated by the receiver.
    pub checksum: u32,
}

/// Response to a VerifyBlock request, sent by the sender.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyResponseV1 {
    /// BLAKE3 hash of the file.
    pub file_hash: [u8; 32],
    /// Sequence number of the block.
    pub seq: u32,
    /// Whether the checksum matched.
    pub valid: bool,
}

/// Messages sent from the Receiver (the one receiving the file) to the Sender.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReceiverMessageV1 {
    /// A request from receiver to sender to send a specific chunk of the file.
    Request(RequestV1),

    /// A response from receiver to sender with the total bytes received so far.
    /// Used for progress tracking and retransmission decisions on the sender side.
    Progress(ProgressV1),

    /// A signal from the receiver that the file has been successfully received and verified.
    /// This marks the completion of the transfer for this connection.
    TransferComplete(TransferCompleteV1),

    /// An error message sent from the receiver to indicate a problem.
    Error(ReceiverErrorV1),

    /// A request to verify an existing block during resume.
    VerifyBlock(VerifyBlockV1),
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

/// Attaches the protocol headers (Version and Length) to the payload.
///
/// This function constructs a new byte buffer containing the headers followed by the payload.
///
/// # Arguments
///
/// * `payload` - The serialized message payload.
///
/// # Returns
///
/// A `Box<[u8]>` containing the full message with headers.
pub fn attach_headers(payload: &[u8]) -> Box<[u8]> {
    let mut message = Vec::with_capacity(64 + payload.len());
    message.extend_from_slice(
        format!(
            //Ver: [PROTOCOL_VERSION]\r\n
            "{VERSION_HEADER_PREFIX_STR}{}{MESSAGE_DELIMITER_STR}",
            CURRENT_PROTOCOL_VERSION
        )
        .as_bytes(),
    );
    message.extend_from_slice(
        format!(
            //Len: [length of payload]\r\n
            "{LENGTH_HEADER_PREFIX_STR}{}{MESSAGE_DELIMITER_STR}",
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
        let msg = SenderMessageV1::Handshake(HandshakeV1 {
            file_hash: &[0xAA; 32],
            total_size: 1024 * 1024,
            concurrency: 8,
            file_name: "test_file.txt",
            block_size: MAX_BLOCK_SIZE,
        });

        let mut buffer = [0u8; 1024]; // Large enough buffer for serialization
        let serialized = msg.to_bytes(&mut buffer).expect("Failed to serialize");
        let decoded = SenderMessageV1::from_bytes(&serialized).expect("Failed to deserialize");

        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_transfer_complete_serde() {
        let msg = ReceiverMessageV1::TransferComplete(TransferCompleteV1 {
            file_hash: [0xBB; 32],
        });
        let mut buffer = [0u8; 1024];
        let serialized = msg.to_bytes(&mut buffer).expect("Failed to serialize");
        let decoded = ReceiverMessageV1::from_bytes(&serialized).expect("Failed to deserialize");

        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_data_serde() {
        let data_payload = [1, 2, 3, 4, 5];
        let msg = SenderMessageV1::Data(DataV1 {
            seq: 10,
            checksum: 0xDEADBEEF,
            file_hash: &[0xAA; 32],
            compressed: false,
            data: &data_payload,
        });

        let mut buffer = [0u8; 1024]; // Large enough buffer for serialization
        let serialized = msg.to_bytes(&mut buffer).expect("Failed to serialize");
        let decoded = SenderMessageV1::from_bytes(&serialized).expect("Failed to deserialize");

        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_error_serde() {
        let msg = SenderMessageV1::Error(SenderErrorV1 {
            code: 404,
            message: "File not found".to_string(),
        });

        let mut buffer = [0u8; 1024]; // Large enough buffer for serialization
        let serialized = msg.to_bytes(&mut buffer).expect("Failed to serialize");
        let decoded = SenderMessageV1::from_bytes(&serialized).expect("Failed to deserialize");

        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_progress_serde() {
        let msg = ReceiverMessageV1::Progress(ProgressV1 {
            file_hash: [0xDD; 32],
            bytes_received: 512 * 1024,
        });
        let mut buffer = [0u8; 1024]; // Large enough buffer for serialization
        let serialized = msg.to_bytes(&mut buffer).expect("Failed to serialize");
        let decoded = ReceiverMessageV1::from_bytes(&serialized).expect("Failed to deserialize ");

        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_request_serde() {
        let msg = ReceiverMessageV1::Request(RequestV1 {
            file_hash: [0xEE; 32],
            seq: 42,
        });
        let mut buffer = [0u8; 1024]; // Large enough buffer for
                                      // serialization
        let serialized = msg.to_bytes(&mut buffer).expect("Failed to serialize");
        let decoded = ReceiverMessageV1::from_bytes(&serialized).expect("Failed to deserialize");

        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_large_data_serde() {
        let data_payload = vec![0xFF; MAX_BLOCK_SIZE as usize];
        let msg = SenderMessageV1::Data(DataV1 {
            seq: 100,
            checksum: 0xBEEFDEAD,
            file_hash: &[0xFF; 32],
            compressed: false,
            data: &data_payload,
        });

        let mut buffer = vec![0u8; (MAX_BLOCK_SIZE + 512) as usize]; // Large enough buffer for serialization
        let serialized = msg.to_bytes(&mut buffer).expect("Failed to serialize");
        let decoded = SenderMessageV1::from_bytes(&serialized).expect("Failed to deserialize");

        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_verify_block_serde() {
        let msg = ReceiverMessageV1::VerifyBlock(VerifyBlockV1 {
            file_hash: [0xCC; 32],
            seq: 123,
            checksum: 0xDEADBEEF,
        });
        let mut buffer = [0u8; 1024];
        let serialized = msg.to_bytes(&mut buffer).expect("Failed to serialize");
        let decoded = ReceiverMessageV1::from_bytes(&serialized).expect("Failed to deserialize");

        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_verify_response_serde() {
        let msg = SenderMessageV1::VerifyResponse(VerifyResponseV1 {
            file_hash: [0xCC; 32],
            seq: 123,
            valid: true,
        });
        let mut buffer = [0u8; 1024];
        let serialized = msg.to_bytes(&mut buffer).expect("Failed to serialize");
        let decoded = SenderMessageV1::from_bytes(&serialized).expect("Failed to deserialize");

        assert_eq!(msg, decoded);
    }
}
