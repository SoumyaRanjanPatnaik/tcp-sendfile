use crate::{
    connection::read_next_payload,
    stream::{error::SendFileError, utils::initialize_handshake},
    transport::{ReceiverMessageV1, MAX_MESSAGE_SIZE},
};
use log::{error, info, warn};
use std::{
    io,
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
};

const SEND_LISTEN_PORT: u16 = 7890;
const POLL_SLEEP_MS: u64 = 500;

// TODO: Remove this once the function starts being used
#[allow(dead_code)]
/// Sends a file to the specified address using the custom file transfer protocol.
pub fn send_file(
    address: (&str, u16),
    file_path: &Path,
    block_size: u32,
) -> Result<(), SendFileError> {
    let available_parallelism = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let cap = (available_parallelism * 4).max(8).min(u16::MAX as usize); // Ensure at least 8 connections, max u16

    let mut transport_buffer = vec![0u8; MAX_MESSAGE_SIZE];
    let file_hash = initialize_handshake(
        &mut transport_buffer,
        address,
        file_path,
        block_size,
        cap as u16,
    )
    .expect("Failed to initialize handshake");

    let listener = TcpListener::bind(("0.0.0.0", SEND_LISTEN_PORT))?;
    listener.set_nonblocking(true)?;
    info!("Sender listening on 0.0.0.0:{}", SEND_LISTEN_PORT);

    let active_connections = Arc::new(AtomicUsize::new(0));
    let transfer_complete = Arc::new(AtomicBool::new(false));

    thread::scope(|scope| loop {
        if transfer_complete.load(Ordering::Relaxed) {
            break;
        }

        match listener.accept() {
            Ok((stream, addr)) => {
                info!("Accepted connection from {}", addr);
                if active_connections.load(Ordering::Relaxed) >= cap {
                    warn!("Max connections reached, dropping incoming connection");
                    continue;
                }

                let active_connections = active_connections.clone();
                let transfer_complete = transfer_complete.clone();

                active_connections.fetch_add(1, Ordering::SeqCst);
                scope.spawn(move || {
                    let success = handle_connection(stream, &file_hash);
                    active_connections.fetch_sub(1, Ordering::SeqCst);
                    if success {
                        transfer_complete.store(true, Ordering::SeqCst);
                    }
                });
            }
            Err(e) => {
                error!("Listener accept error: {}", e);
                thread::sleep(std::time::Duration::from_millis(POLL_SLEEP_MS));
            }
        }
    });

    Ok(())
}

fn handle_connection(mut stream: TcpStream, expected_hash: &[u8; 32]) -> bool {
    let mut buffer = vec![0u8; MAX_MESSAGE_SIZE];
    let mut filled_len = 0;

    loop {
        match read_next_payload::<ReceiverMessageV1, _>(&mut stream, &mut buffer, filled_len) {
            Ok(result) => {
                let message = result.message;

                // Handle buffer management for next iteration
                if let Some(next_idx) = result.next_payload_index {
                    let remaining_len = result.total_bytes_read - next_idx;
                    buffer.copy_within(next_idx..result.total_bytes_read, 0);
                    filled_len = remaining_len;
                } else {
                    filled_len = 0;
                }

                match message {
                    ReceiverMessageV1::Request { seq, file_hash } => {
                        if file_hash != *expected_hash {
                            warn!("Received request for wrong file hash: {:?}", file_hash);
                            return false;
                        }
                        info!("Received request for seq {}", seq);
                        // TODO: Send actual file chunk
                    }
                    ReceiverMessageV1::ProgressResponse {
                        bytes_received,
                        file_hash,
                    } => {
                        if file_hash != *expected_hash {
                            warn!(
                                "Received progress response for wrong file hash: {:?}",
                                file_hash
                            );
                            return false;
                        }
                        info!("Progress: {} bytes", bytes_received);
                    }
                    ReceiverMessageV1::TransferComplete { file_hash } => {
                        if file_hash != *expected_hash {
                            warn!(
                                "Received transfer complete for wrong file hash: {:?}",
                                file_hash
                            );
                            return false;
                        }
                        info!("File transfer successful");
                        return true;
                    }
                    ReceiverMessageV1::Error { code, message } => {
                        warn!("Receiver error {}: {}", code, message);
                        return false;
                    }
                }
            }
            Err(e) => {
                warn!("Connection error: {}", e);
                return false;
            }
        }
    }
}
