use std::{io::Write, net::TcpStream, path::Path};

use log::{debug, info};
use thiserror::Error;

use crate::{
    file::FileMetadata,
    transport::{self, TransportError, TransportMessageV1, MAX_MESSAGE_SIZE},
};

#[derive(Error, Debug)]
pub enum ConnectionError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Transport error: {0}")]
    Transport(#[from] TransportError),
    #[error("Invalid address format: {0}")]
    InvalidAddress(#[from] std::net::AddrParseError),
}

pub fn send_file(
    address: (&str, u16),
    file_path: &Path,
    block_size: u32,
    concurrency: u16,
) -> Result<(), ConnectionError> {
    debug!("Calculating file metadata for {:?}", file_path);

    let file_metadata = FileMetadata::from_file(file_path)?;
    info!("File name: {}", file_metadata.name());
    info!("File size: {} bytes", file_metadata.size());
    info!("File SHA-256 hash: {:x?}", file_metadata.hash());

    let handshake_message = TransportMessageV1::Handshake {
        file_name: file_metadata.name(),
        file_hash: &file_metadata.hash(),
        total_size: file_metadata.size(),
        concurrency,
        block_size,
    };

    let mut transport_buffer = vec![0u8; MAX_MESSAGE_SIZE];
    let payload_bytes = handshake_message.to_bytes(&mut transport_buffer)?;
    let handshake_message = transport::attach_headers(&payload_bytes);

    debug!(
        "Serialized handshake message: {} bytes",
        handshake_message.len()
    );

    info!(
        "Connecting to reciever at {}",
        format!("{}:{}", address.0, address.1)
    );
    let mut stream = TcpStream::connect(address)?;

    info!("Connected to server, Initiating: {:?}", file_path);
    stream.write_all(&handshake_message)?;
    stream.flush()?; // Ensure the message is sent immediately

    info!("Handshake message sent, waiting for acknowledgment...");
    Ok(())
}
