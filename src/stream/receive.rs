use std::{
    fs::OpenOptions,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use crc_fast::{checksum, CrcAlgorithm};
use flate2::read::GzDecoder;
use log::{error, info, warn};

use crate::{
    cli::TRANSFER_PORT,
    connection::read_next_payload,
    file::utils::{read_file_block, write_file_block},
    stream::error::SendFileError,
    transport::{
        attach_headers, DataV1, ReceiverMessageV1, RequestV1, SenderMessageV1, TransferCompleteV1,
        VerifyBlockV1, MAX_MESSAGE_SIZE,
    },
};

const MAX_RETRIES: u32 = 5;
const INITIAL_RETRY_DELAY_MS: u64 = 500;

/// Starts receiving a file on the specified address.
///
/// This function binds to the given address and listens for incoming connections.
/// It handles the initial handshake and then spawns multiple threads to download
/// file blocks concurrently.
///
/// # Arguments
///
/// * `bind_addr` - The address and port to bind to (e.g., ("0.0.0.0", 7878)).
/// * `path` - The output path where the received file will be saved.
/// * `concurrency` - The number of concurrent connections to accept.
///
/// # Returns
///
/// A `Result` indicating success or a `SendFileError`.
pub fn receive_file(
    bind_addr: (&str, u16),
    path: &std::path::Path,
    concurrency: u16,
) -> Result<(), SendFileError> {
    info!(
        "Listening on {}:{} with concurrency {}",
        bind_addr.0, bind_addr.1, concurrency
    );

    let listener = TcpListener::bind(bind_addr)?;
    let (mut stream, sender_addr) = listener.accept()?;
    info!("Accepted connection from {}", sender_addr);

    let mut buffer = vec![0u8; MAX_MESSAGE_SIZE];
    let result = read_next_payload::<SenderMessageV1, _>(&mut stream, &mut buffer, 0)?;
    let handshake = match result.message {
        SenderMessageV1::Handshake(h) => h,
        _ => {
            return Err(SendFileError::UnexpectedMessage {
                received: format!("{:?}", result.message),
                expected: String::from("Handshake"),
            });
        }
    };

    let file_hash: [u8; 32] = handshake.file_hash.try_into().map_err(|_| {
        SendFileError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Invalid file hash length",
        ))
    })?;

    info!(
        "Received handshake: file={}, size={}, block_size={}, concurrency={}",
        handshake.file_name, handshake.total_size, handshake.block_size, handshake.concurrency
    );

    let final_path = determine_final_path(path, handshake.file_name);
    info!("Output file path: {:?}", final_path);

    let total_blocks = handshake.total_size.div_ceil(handshake.block_size as u64) as u32;

    let is_existing_file = final_path.exists();

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&final_path)?;

    file.set_len(handshake.total_size)?;

    let received_blocks: Vec<AtomicBool> =
        (0..total_blocks).map(|_| AtomicBool::new(false)).collect();

    let state = Arc::new(ReceiverState {
        file_hash,
        _total_size: handshake.total_size,
        block_size: handshake.block_size,
        _total_blocks: total_blocks,
        sender_addr,
        received_blocks,
        bytes_received: AtomicU64::new(0),
        file_path: final_path,
        is_existing_file,
    });

    let ranges = split_blocks_into_ranges(total_blocks, concurrency);

    thread::scope(|scope| {
        for range in ranges {
            let state = state.clone();
            scope.spawn(move || {
                if let Err(e) = run_connection(state, range.start, range.end) {
                    error!("Connection error in range {:?}: {}", range, e);
                }
            });
        }
    });

    let bytes_received = state.bytes_received.load(Ordering::SeqCst);
    info!(
        "Transfer complete: {} bytes received for file {:?}",
        bytes_received, state.file_path
    );

    Ok(())
}

struct ReceiverState {
    file_hash: [u8; 32],
    _total_size: u64,
    block_size: u32,
    _total_blocks: u32,
    sender_addr: SocketAddr,
    received_blocks: Vec<AtomicBool>,
    bytes_received: AtomicU64,
    file_path: PathBuf,
    is_existing_file: bool,
}

fn determine_final_path(output_path: &std::path::Path, file_name: &str) -> PathBuf {
    if output_path.is_dir() {
        output_path.join(file_name)
    } else {
        output_path.to_path_buf()
    }
}

fn split_blocks_into_ranges(total_blocks: u32, concurrency: u16) -> Vec<std::ops::Range<u32>> {
    let blocks_per_conn = total_blocks / concurrency as u32;
    let remainder = total_blocks % concurrency as u32;

    let mut ranges = Vec::new();
    let mut start = 0u32;

    for i in 0..concurrency {
        let extra = if (i as u32) < remainder { 1 } else { 0 };
        let end = (start + blocks_per_conn + extra).min(total_blocks);
        ranges.push(start..end);
        start = end;
    }

    ranges
}

fn run_connection(
    state: Arc<ReceiverState>,
    range_start: u32,
    range_end: u32,
) -> Result<(), SendFileError> {
    // Connect to the sender for this thread's assigned block range
    let mut stream = TcpStream::connect((state.sender_addr.ip(), TRANSFER_PORT))?;

    if state.is_existing_file {
        verify_existing_blocks(&mut stream, &state, range_start, range_end)?;
    } else {
        download_missing_blocks(&mut stream, &state, range_start, range_end)?;
    }

    if is_transfer_complete(&state) {
        send_transfer_complete(&mut stream, &state)?;
    }

    Ok(())
}

fn verify_existing_blocks(
    stream: &mut TcpStream,
    state: &ReceiverState,
    range_start: u32,
    range_end: u32,
) -> Result<(), SendFileError> {
    let mut buffer = vec![0u8; MAX_MESSAGE_SIZE];
    let mut filled_len = 0;
    let mut write_buffer = vec![0u8; MAX_MESSAGE_SIZE];

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&state.file_path)?;

    for seq in range_start..range_end {
        if state.received_blocks[seq as usize].load(Ordering::SeqCst) {
            continue;
        }

        let block_data = read_file_block(&mut file, seq, state.block_size)?;

        if block_data.is_empty() {
            continue;
        }

        let checksum_val = checksum(CrcAlgorithm::Crc32IsoHdlc, &block_data) as u32;

        let msg = ReceiverMessageV1::VerifyBlock(VerifyBlockV1 {
            file_hash: state.file_hash,
            seq,
            checksum: checksum_val,
        });

        send_message(stream, &msg, &mut write_buffer)?;

        let (valid, next_filled_len) = read_verify_response(stream, &mut buffer, filled_len, seq)?;

        filled_len = next_filled_len;

        if valid {
            state.received_blocks[seq as usize].store(true, Ordering::SeqCst);
            state
                .bytes_received
                .fetch_add(block_data.len() as u64, Ordering::SeqCst);
            info!("Block {} verified successfully", seq);
        } else {
            info!("Block {} verification failed, will re-download", seq);
        }
    }

    Ok(())
}

fn read_verify_response(
    stream: &mut TcpStream,
    buffer: &mut Vec<u8>,
    filled_len: usize,
    seq: u32,
) -> Result<(bool, usize), SendFileError> {
    let result = read_next_payload::<SenderMessageV1, _>(stream, buffer, filled_len)?;

    let (valid, next_idx, total_bytes_read) = match result.message {
        SenderMessageV1::VerifyResponse(resp) => {
            if resp.seq != seq {
                warn!(
                    "Verify response seq mismatch: expected {}, got {}",
                    seq, resp.seq
                );
                (false, result.next_payload_index, result.total_bytes_read)
            } else {
                (
                    resp.valid,
                    result.next_payload_index,
                    result.total_bytes_read,
                )
            }
        }
        SenderMessageV1::Error(err) => {
            error!("Sender error during verify: {} - {}", err.code, err.message);
            (false, result.next_payload_index, result.total_bytes_read)
        }
        _ => {
            warn!("Unexpected message during verify");
            (false, result.next_payload_index, result.total_bytes_read)
        }
    };

    let next_filled_len = if let Some(next_idx) = next_idx {
        let remaining_len = total_bytes_read - next_idx;
        buffer.copy_within(next_idx..total_bytes_read, 0);
        remaining_len
    } else {
        0
    };

    Ok((valid, next_filled_len))
}

fn download_missing_blocks(
    stream: &mut TcpStream,
    state: &ReceiverState,
    range_start: u32,
    range_end: u32,
) -> Result<(), SendFileError> {
    let mut current_seq = range_start;

    while current_seq < range_end {
        while current_seq < range_end
            && state.received_blocks[current_seq as usize].load(Ordering::SeqCst)
        {
            current_seq += 1;
        }

        if current_seq >= range_end {
            break;
        }

        let mut retry_count = 0u32;
        let mut retry_delay = INITIAL_RETRY_DELAY_MS;

        loop {
            let success = download_block_with_retry(stream, state, current_seq)?;

            if success {
                state.received_blocks[current_seq as usize].store(true, Ordering::SeqCst);
                current_seq += 1;
                break;
            }

            retry_count += 1;
            if retry_count >= MAX_RETRIES {
                error!(
                    "Max retries ({}) exceeded for block {}",
                    MAX_RETRIES, current_seq
                );
                return Err(SendFileError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("Max retries exceeded for block {}", current_seq),
                )));
            }

            retry_delay *= 2;
            thread::sleep(Duration::from_millis(retry_delay));
        }
    }

    Ok(())
}

fn download_block_with_retry(
    stream: &mut TcpStream,
    state: &ReceiverState,
    seq: u32,
) -> Result<bool, SendFileError> {
    let mut buffer = vec![0u8; MAX_MESSAGE_SIZE];
    let mut write_buffer = vec![0u8; MAX_MESSAGE_SIZE];

    let msg = ReceiverMessageV1::Request(RequestV1 {
        file_hash: state.file_hash,
        seq,
    });

    if let Err(e) = send_message(stream, &msg, &mut write_buffer) {
        warn!("Failed to send request for block {}: {}", seq, e);
        return Ok(false);
    }

    let result = match read_next_payload::<SenderMessageV1, _>(stream, &mut buffer, 0) {
        Ok(r) => r,
        Err(e) => {
            warn!("Failed to read response for block {}: {}", seq, e);
            return Ok(false);
        }
    };

    match result.message {
        SenderMessageV1::Data(data) => process_data_block(state, seq, data, &mut write_buffer),
        SenderMessageV1::Error(err) => {
            error!(
                "Sender error for block {}: {} - {}",
                seq, err.code, err.message
            );
            Ok(false)
        }
        _ => {
            warn!("Unexpected message type for block {}", seq);
            Ok(false)
        }
    }
}

fn process_data_block(
    state: &ReceiverState,
    seq: u32,
    data: DataV1,
    write_buffer: &mut [u8],
) -> Result<bool, SendFileError> {
    let computed_checksum = checksum(CrcAlgorithm::Crc32IsoHdlc, data.data) as u32;
    if computed_checksum != data.checksum {
        warn!(
            "Checksum mismatch for block {}: expected {}, got {}",
            seq, data.checksum, computed_checksum
        );
        return Ok(false);
    }

    let block_data = if data.compressed {
        match decompress_gzip(data.data) {
            Ok(d) => d,
            Err(e) => {
                warn!("Failed to decompress block {}: {}", seq, e);
                return Ok(false);
            }
        }
    } else {
        data.data.to_vec()
    };

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&state.file_path)?;

    if let Err(e) = write_file_block(&mut file, seq, state.block_size, &block_data) {
        warn!("Failed to write block {}: {}", seq, e);
        return Ok(false);
    }

    state
        .bytes_received
        .fetch_add(block_data.len() as u64, Ordering::SeqCst);

    let _ = write_buffer;
    Ok(true)
}

fn decompress_gzip(data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let mut decoder = GzDecoder::new(data);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)?;
    Ok(decompressed)
}

fn send_message<W: Write>(
    stream: &mut W,
    msg: &ReceiverMessageV1,
    buffer: &mut [u8],
) -> Result<(), SendFileError> {
    let payload = msg.to_bytes(buffer)?;
    let packet = attach_headers(payload);
    stream.write_all(&packet)?;
    stream.flush()?;
    Ok(())
}

fn is_transfer_complete(state: &ReceiverState) -> bool {
    state
        .received_blocks
        .iter()
        .all(|b| b.load(Ordering::SeqCst))
}

fn send_transfer_complete(
    stream: &mut TcpStream,
    state: &ReceiverState,
) -> Result<(), SendFileError> {
    let mut buffer = vec![0u8; MAX_MESSAGE_SIZE];

    let msg = ReceiverMessageV1::TransferComplete(TransferCompleteV1 {
        file_hash: state.file_hash,
    });

    send_message(stream, &msg, &mut buffer)?;

    info!("Sent TransferComplete for file {:?}", state.file_path);
    Ok(())
}

#[cfg(test)]
pub fn split_blocks_into_ranges_for_test(
    total_blocks: u32,
    concurrency: u16,
) -> Vec<std::ops::Range<u32>> {
    split_blocks_into_ranges(total_blocks, concurrency)
}

#[cfg(test)]
pub fn decompress_gzip_for_test(data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    decompress_gzip(data)
}

#[cfg(test)]
pub fn determine_final_path_for_test(output_path: &std::path::Path, file_name: &str) -> PathBuf {
    determine_final_path(output_path, file_name)
}

#[cfg(test)]
pub fn is_transfer_complete_for_test(received_blocks: &[bool], total_blocks: u32) -> bool {
    received_blocks
        .iter()
        .take(total_blocks as usize)
        .all(|&b| b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::sync::atomic::AtomicU64;

    #[test]
    fn test_process_data_block_checksum_logic() {
        // Setup
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join("test_checksum_fix.txt");
        let _ = std::fs::remove_file(&file_path); // Cleanup

        // Create and pre-allocate file
        {
            let file = std::fs::File::create(&file_path).unwrap();
            file.set_len(1024).unwrap();
        }

        let state = ReceiverState {
            file_hash: [0u8; 32],
            _total_size: 100,
            block_size: 1024,
            _total_blocks: 1,
            sender_addr: "127.0.0.1:0".parse().unwrap(),
            received_blocks: vec![AtomicBool::new(false)],
            bytes_received: AtomicU64::new(0),
            file_path: file_path.clone(),
            is_existing_file: false,
        };

        // Create compressed data
        let original_data = b"Hello, World!";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original_data).unwrap();
        let compressed_data = encoder.finish().unwrap();

        // Calculate checksum on COMPRESSED data (as per sender logic)
        let checksum_val = checksum(CrcAlgorithm::Crc32IsoHdlc, &compressed_data) as u32;

        let data = DataV1 {
            seq: 0,
            checksum: checksum_val,
            file_hash: &[0u8; 32],
            compressed: true,
            data: &compressed_data,
        };

        let mut write_buffer = vec![0u8; 1024];

        // Execute
        let result = process_data_block(&state, 0, data, &mut write_buffer);

        // Verify
        assert!(
            result.is_ok(),
            "process_data_block failed: {:?}",
            result.err()
        );
        assert!(result.unwrap());

        // Cleanup
        let _ = std::fs::remove_file(file_path);
    }
}
