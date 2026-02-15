use log::info;

use crate::{
    stream::{error::SendFileError, utils::initialize_handshake},
    transport::{ReceiverMessageV1, MAX_MESSAGE_SIZE},
};
use std::path::Path;

// TODO: Remove this once the function starts being used
#[allow(dead_code)]
/// Sends a file to the specified address using the custom file transfer protocol.
pub fn send_file(
    address: (&str, u16),
    file_path: &Path,
    block_size: u32,
    concurrency: u16,
) -> Result<(), SendFileError> {
    let mut transport_buffer = vec![0u8; MAX_MESSAGE_SIZE];
    let handshake_response = initialize_handshake(
        &mut transport_buffer,
        address,
        file_path,
        block_size,
        concurrency,
    )?;

    // Validate the handshake response and log the negotiated parameters
    if let ReceiverMessageV1::HandshakeAck {
        file_hash,
        total_size,
        concurrency,
        file_name,
        block_size,
    } = handshake_response
    {
        info!("Handshake successful with receiver:");
        info!("Negotiated File Name: {}", file_name);
        info!("Negotiated File Hash: {:x?}", file_hash);
        info!("Negotiated Total Size: {} bytes", total_size);
        info!("Negotiated Concurrency: {}", concurrency);
        info!("Negotiated Block Size: {} bytes", block_size);
    } else {
        panic!("Expected handshake response from receiver, but received a different message type");
    }
    Ok(())
}
