use crate::{
    connection::read_next_payload,
    file::FileMetadata,
    stream::error::SendFileError,
    transport::{self, TransportMessageV1},
};
use log::{debug, info};
use std::{io::Write, net::TcpStream, path::Path};

/// Initializes a file handshake with the specified address and file path,
/// sending the necessary metadata to the receiver.
pub fn initialize_handshake<'a>(
    transport_buffer: &'a mut [u8],
    address: (&str, u16),
    file_path: &Path,
    block_size: u32,
    concurrency: u16,
) -> Result<TransportMessageV1<'a>, SendFileError> {
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

    let payload_bytes = handshake_message.to_bytes(transport_buffer)?;
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

    let handshake_response = read_next_payload(&mut stream, transport_buffer, 0)?;

    Ok(handshake_response.message)
}
