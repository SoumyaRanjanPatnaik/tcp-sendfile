use std::{net::TcpStream, path::Path};

use log::info;
use thiserror::Error;

use crate::transport::TransportError;

#[derive(Error, Debug)]
pub enum ConnectionError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Transport error: {0}")]
    Transport(#[from] TransportError),
}

pub fn send_file(addr: &str, file_path: &Path) -> Result<(), ConnectionError> {
    // Placeholder for sending a file over the connection
    info!("Connecting to server at {}", addr);
    let _stream = TcpStream::connect(addr)?;
    info!("Connected to server, sending file: {:?}", file_path);
    Ok(())
}
