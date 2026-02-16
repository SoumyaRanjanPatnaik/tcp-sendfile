use crate::{
    cli::TRANSFER_PORT,
    connection::read_next_payload,
    file::utils::read_file_block,
    stream::{error::SendFileError, utils::initialize_handshake},
    transport::{
        DataV1, ProgressV1, ReceiverErrorV1, ReceiverMessageV1, RequestV1, SenderErrorV1,
        SenderMessageV1, TransferCompleteV1, VerifyBlockV1, VerifyResponseV1, MAX_MESSAGE_SIZE,
    },
};
use crc_fast::{checksum, CrcAlgorithm};
use flate2::{write::GzEncoder, Compression};
use log::{error, info, warn};
use std::{
    fs::File,
    io::Write,
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
};

const POLL_SLEEP_MS: u64 = 500;

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

    let listener = TcpListener::bind(("0.0.0.0", TRANSFER_PORT))?;
    listener.set_nonblocking(true)?;
    info!("Sender listening on 0.0.0.0:{}", TRANSFER_PORT);

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
                    let success = handle_connection(stream, &file_hash, file_path, block_size);
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

fn handle_connection(
    mut stream: TcpStream,
    expected_hash: &[u8; 32],
    file_path: &Path,
    block_size: u32,
) -> bool {
    let mut buffer = vec![0u8; MAX_MESSAGE_SIZE];
    let mut filled_len = 0;

    let file_handler = match File::open(file_path) {
        Ok(f) => f,
        Err(e) => {
            error!("Failed to open file: {}", e);
            return false;
        }
    };

    let mut handler = ConnectionHandler {
        file: file_handler,
        expected_hash: *expected_hash,
        block_size,
        compression_enabled: None,
        write_buffer: vec![0u8; MAX_MESSAGE_SIZE],
        compressed_buffer: Vec::with_capacity(block_size as usize),
    };

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
                    ReceiverMessageV1::Request(req) => {
                        match handler.handle_data_request(&req, &mut stream) {
                            Ok(true) => {} // Continue
                            Ok(false) => return false,
                            Err(e) => {
                                error!("Request handling error: {}", e);
                                return false;
                            }
                        }
                    }
                    ReceiverMessageV1::Progress(prog) => {
                        if !handler.handle_progress(&prog) {
                            return false;
                        }
                    }
                    ReceiverMessageV1::TransferComplete(complete) => {
                        return handler.handle_transfer_complete(&complete);
                    }
                    ReceiverMessageV1::Error(err) => {
                        handler.handle_error(&err);
                        return false;
                    }
                    ReceiverMessageV1::VerifyBlock(verify) => {
                        match handler.handle_verify_block(&verify, &mut stream) {
                            Ok(true) => {}
                            Ok(false) => return false,
                            Err(e) => {
                                error!("Verify block handling error: {}", e);
                                return false;
                            }
                        }
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

pub struct ConnectionHandler {
    pub file: File,
    pub expected_hash: [u8; 32],
    pub block_size: u32,
    pub compression_enabled: Option<bool>,
    pub write_buffer: Vec<u8>,
    pub compressed_buffer: Vec<u8>,
}

impl ConnectionHandler {
    pub fn handle_data_request<W: Write>(
        &mut self,
        req: &RequestV1,
        writer: &mut W,
    ) -> Result<bool, SendFileError> {
        let RequestV1 { seq, file_hash } = req;

        if file_hash != &self.expected_hash {
            warn!("Received request for wrong file hash: {:?}", file_hash);
            return Ok(false);
        }
        info!("Received request for seq {}", seq);

        match read_file_block(&mut self.file, *seq, self.block_size) {
            Ok(data) => {
                let compressed_flag: bool;
                let final_data: &[u8];

                // Determine if we should attempt compression
                let attempt_compression = match self.compression_enabled {
                    Some(true) => true,
                    Some(false) => false,
                    None => true, // Probe on first request
                };

                if attempt_compression {
                    let mut compression_success = false;
                    self.compressed_buffer.clear();
                    {
                        let mut encoder =
                            GzEncoder::new(&mut self.compressed_buffer, Compression::default());

                        if encoder.write_all(&data).is_ok() && encoder.finish().is_ok() {
                            compression_success = true;
                        }
                    }

                    if compression_success {
                        let is_smaller = self.compressed_buffer.len() < data.len();

                        // If this is the first request (probe), set the sticky flag
                        if self.compression_enabled.is_none() {
                            self.compression_enabled = Some(is_smaller);
                        }

                        if is_smaller {
                            final_data = &self.compressed_buffer;
                            compressed_flag = true;
                        } else {
                            final_data = &data;
                            compressed_flag = false;
                        }
                    } else {
                        // Compression failed (e.g. IO error in encoder), fallback to raw
                        if self.compression_enabled.is_none() {
                            self.compression_enabled = Some(false);
                        }
                        final_data = &data;
                        compressed_flag = false;
                    }
                } else {
                    // Compression disabled
                    final_data = &data;
                    compressed_flag = false;
                }

                let checksum_val = checksum(CrcAlgorithm::Crc32IsoHdlc, final_data);

                let msg = SenderMessageV1::Data(DataV1 {
                    seq: *seq,
                    checksum: checksum_val as u32,
                    file_hash: &self.expected_hash,
                    compressed: compressed_flag,
                    data: final_data,
                });

                match msg.to_bytes(&mut self.write_buffer) {
                    Ok(payload) => {
                        let packet = crate::transport::attach_headers(payload);
                        if let Err(e) = writer.write_all(&packet) {
                            error!("Failed to write data to stream: {}", e);
                            return Ok(false);
                        }
                        if let Err(e) = writer.flush() {
                            error!("Failed to flush stream: {}", e);
                            return Ok(false);
                        }
                        Ok(true)
                    }
                    Err(e) => {
                        error!("Serialization error: {}", e);
                        Ok(false)
                    }
                }
            }
            Err(e) => {
                error!("Failed to read file block: {}", e);
                let error_msg = SenderMessageV1::Error(SenderErrorV1 {
                    code: 500,
                    message: format!("Read error: {}", e),
                });
                // Best effort to send error
                if let Ok(payload) = error_msg.to_bytes(&mut self.write_buffer) {
                    let packet = crate::transport::attach_headers(payload);
                    let _ = writer.write_all(&packet);
                }
                Ok(false)
            }
        }
    }

    pub fn handle_progress(&mut self, prog: &ProgressV1) -> bool {
        let ProgressV1 {
            file_hash,
            bytes_received,
        } = prog;

        if file_hash != &self.expected_hash {
            warn!(
                "Received progress response for wrong file hash: {:?}",
                file_hash
            );
            return false;
        }
        info!("Progress: {} bytes", bytes_received);
        true
    }

    pub fn handle_transfer_complete(&mut self, complete: &TransferCompleteV1) -> bool {
        let TransferCompleteV1 { file_hash } = complete;

        if file_hash != &self.expected_hash {
            warn!(
                "Received transfer complete for wrong file hash: {:?}",
                file_hash
            );
            return false;
        }
        info!("File transfer successful");
        true
    }

    pub fn handle_error(&mut self, err: &ReceiverErrorV1) {
        let ReceiverErrorV1 { code, message } = err;
        error!("Receiver error {}: {}", code, message);
    }

    pub fn handle_verify_block<W: Write>(
        &mut self,
        verify: &VerifyBlockV1,
        writer: &mut W,
    ) -> Result<bool, SendFileError> {
        let VerifyBlockV1 {
            file_hash,
            seq,
            checksum: receiver_checksum,
        } = verify;

        if file_hash != &self.expected_hash {
            warn!(
                "Received verify request for wrong file hash: {:?}",
                file_hash
            );
            return Ok(false);
        }
        info!("Received verify request for seq {}", seq);

        match read_file_block(&mut self.file, *seq, self.block_size) {
            Ok(data) => {
                let computed_checksum = checksum(CrcAlgorithm::Crc32IsoHdlc, &data) as u32;
                let valid = computed_checksum == *receiver_checksum;

                info!(
                    "Verify block seq {}: receiver={}, computed={}, valid={}",
                    seq, receiver_checksum, computed_checksum, valid
                );

                let msg = SenderMessageV1::VerifyResponse(VerifyResponseV1 {
                    file_hash: self.expected_hash,
                    seq: *seq,
                    valid,
                });

                match msg.to_bytes(&mut self.write_buffer) {
                    Ok(payload) => {
                        let packet = crate::transport::attach_headers(payload);
                        if let Err(e) = writer.write_all(&packet) {
                            error!("Failed to write verify response to stream: {}", e);
                            return Ok(false);
                        }
                        if let Err(e) = writer.flush() {
                            error!("Failed to flush stream: {}", e);
                            return Ok(false);
                        }
                        Ok(true)
                    }
                    Err(e) => {
                        error!("Serialization error: {}", e);
                        Ok(false)
                    }
                }
            }
            Err(e) => {
                error!("Failed to read file block for verify: {}", e);
                let msg = SenderMessageV1::VerifyResponse(VerifyResponseV1 {
                    file_hash: self.expected_hash,
                    seq: *seq,
                    valid: false,
                });
                if let Ok(payload) = msg.to_bytes(&mut self.write_buffer) {
                    let packet = crate::transport::attach_headers(payload);
                    let _ = writer.write_all(&packet);
                }
                Ok(false)
            }
        }
    }
}
