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
    initialize_handshake(
        &mut transport_buffer,
        address,
        file_path,
        block_size,
        concurrency,
    )
    .expect("Failed to initialize handshake");

    Ok(())
}
