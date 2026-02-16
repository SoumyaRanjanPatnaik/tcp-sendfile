use crate::stream::send::ConnectionHandler;
use crate::transport::{ProgressV1, RequestV1, SenderMessageV1, TransferCompleteV1};
use blake3::Hasher;
use std::fs::File;
use std::io::{Cursor, Write};
use std::path::PathBuf;

fn create_temp_file(content: &[u8]) -> (File, PathBuf) {
    let mut dir = std::env::temp_dir();
    let filename = format!(
        "sendfile_test_{}.tmp",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    dir.push(filename);
    let mut file = File::create(&dir).unwrap();
    file.write_all(content).unwrap();
    file.sync_all().unwrap();
    let file = File::open(&dir).unwrap();
    (file, dir)
}

fn calculate_hash(data: &[u8]) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize().into()
}

// Helper to strip headers and deserialize
fn parse_message(bytes: &[u8]) -> SenderMessageV1<'_> {
    let delimiter = b"\r\n\r\n";
    if let Some(start) = bytes
        .windows(delimiter.len())
        .position(|window| window == delimiter)
    {
        let payload = &bytes[start + delimiter.len()..];
        SenderMessageV1::from_bytes(payload).expect("Failed to deserialize payload")
    } else {
        // Fallback or error
        SenderMessageV1::from_bytes(bytes).expect("Failed to deserialize raw bytes")
    }
}

#[test]
fn test_handle_data_request_compression_probe_positive() {
    let data = vec![0u8; 1024]; // Highly compressible
    let hash = calculate_hash(&data);
    let (file, path) = create_temp_file(&data);

    let mut handler = ConnectionHandler {
        file,
        expected_hash: hash,
        block_size: 1024,
        compression_enabled: None,
        write_buffer: vec![0u8; 2048],
        compressed_buffer: vec![0u8; 2048],
    };

    let req = RequestV1 {
        file_hash: hash,
        seq: 0,
    };
    let mut cursor = Cursor::new(Vec::new());

    let result = handler.handle_data_request(&req, &mut cursor);
    assert!(
        result.is_ok(),
        "handle_data_request should succeed: {:?}",
        result.err()
    );

    // Check compression enabled
    assert_eq!(handler.compression_enabled, Some(true));

    // Verify message
    let written = cursor.into_inner();
    let msg = parse_message(&written);

    match msg {
        SenderMessageV1::Data(d) => {
            assert!(d.compressed, "Data should be compressed");
            assert!(d.data.len() < 1024, "Compressed data should be smaller");
            assert_eq!(d.seq, 0);
        }
        _ => panic!("Expected Data message"),
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn test_handle_data_request_compression_probe_negative() {
    // Generate random data (incompressible)
    let mut data = Vec::with_capacity(1024);
    // Use a complex pattern to ensure incompressibility
    let mut state: u64 = 0xCAFEBABE;
    for _ in 0..1024 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        data.push((state >> 33) as u8);
    }

    let hash = calculate_hash(&data);
    let (file, path) = create_temp_file(&data);

    let mut handler = ConnectionHandler {
        file,
        expected_hash: hash,
        block_size: 1024,
        compression_enabled: None,
        write_buffer: vec![0u8; 2048],
        compressed_buffer: vec![0u8; 2048],
    };

    let req = RequestV1 {
        file_hash: hash,
        seq: 0,
    };
    let mut cursor = Cursor::new(Vec::new());

    let result = handler.handle_data_request(&req, &mut cursor);
    assert!(
        result.is_ok(),
        "handle_data_request should succeed: {:?}",
        result.err()
    );

    assert_eq!(handler.compression_enabled, Some(false));

    let written = cursor.into_inner();
    let msg = parse_message(&written);

    match msg {
        SenderMessageV1::Data(d) => {
            assert!(!d.compressed, "Data should not be compressed");
            assert_eq!(d.data, data.as_slice());
        }
        _ => panic!("Expected Data message"),
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn test_handle_data_request_honors_compression_disabled() {
    let data = vec![0u8; 1024]; // Compressible
    let hash = calculate_hash(&data);
    let (file, path) = create_temp_file(&data);

    let mut handler = ConnectionHandler {
        file,
        expected_hash: hash,
        block_size: 1024,
        compression_enabled: Some(false), // Explicitly disabled
        write_buffer: vec![0u8; 2048],
        compressed_buffer: vec![0u8; 2048],
    };

    let req = RequestV1 {
        file_hash: hash,
        seq: 0,
    };
    let mut cursor = Cursor::new(Vec::new());

    let _ = handler
        .handle_data_request(&req, &mut cursor)
        .expect("handle_data_request failed");

    let written = cursor.into_inner();
    let msg = parse_message(&written);

    match msg {
        SenderMessageV1::Data(d) => {
            assert!(!d.compressed, "Should not compress when disabled");
            assert_eq!(d.data, data.as_slice());
        }
        _ => panic!("Expected Data message"),
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn test_handle_data_request_mismatched_hash() {
    let data = b"some data";
    let hash = calculate_hash(data);
    let (file, path) = create_temp_file(data);

    let mut handler = ConnectionHandler {
        file,
        expected_hash: hash,
        block_size: 1024,
        compression_enabled: None,
        write_buffer: vec![0u8; 2048],
        compressed_buffer: vec![0u8; 2048],
    };

    let wrong_hash = [0u8; 32];
    let req = RequestV1 {
        file_hash: wrong_hash,
        seq: 0,
    };
    let mut cursor = Cursor::new(Vec::new());

    let result = handler.handle_data_request(&req, &mut cursor);

    assert!(result.is_err(), "Should return error for mismatched hash");
    assert!(cursor.into_inner().is_empty(), "Should not write anything");

    let _ = std::fs::remove_file(path);
}

#[test]
fn test_handle_data_request_eof_handling() {
    let data = vec![0u8; 100];
    let hash = calculate_hash(&data);
    let (file, path) = create_temp_file(&data);

    let mut handler = ConnectionHandler {
        file,
        expected_hash: hash,
        block_size: 1024,
        compression_enabled: None,
        write_buffer: vec![0u8; 2048],
        compressed_buffer: vec![0u8; 2048],
    };

    // Request seq 1 (offset 1024), which is beyond EOF (100 bytes)
    let req = RequestV1 {
        file_hash: hash,
        seq: 1,
    };
    let mut cursor = Cursor::new(Vec::new());

    let result = handler.handle_data_request(&req, &mut cursor);
    assert!(
        result.is_ok(),
        "handle_data_request should succeed: {:?}",
        result.err()
    );

    let written = cursor.into_inner();
    let msg = parse_message(&written);

    match msg {
        SenderMessageV1::Data(d) => {
            assert!(d.data.is_empty(), "Should return empty data for EOF");
        }
        _ => panic!("Expected Data message"),
    }

    let _ = std::fs::remove_file(path);
}

#[test]
fn test_handle_progress_valid_hash() {
    let data = b"test";
    let hash = calculate_hash(data);
    let (file, path) = create_temp_file(data);

    let mut handler = ConnectionHandler {
        file,
        expected_hash: hash,
        block_size: 1024,
        compression_enabled: None,
        write_buffer: vec![],
        compressed_buffer: vec![],
    };

    let prog = ProgressV1 {
        file_hash: hash,
        bytes_received: 10,
    };
    assert!(handler.handle_progress(&prog).is_ok());

    let _ = std::fs::remove_file(path);
}

#[test]
fn test_handle_progress_invalid_hash() {
    let data = b"test";
    let hash = calculate_hash(data);
    let (file, path) = create_temp_file(data);

    let mut handler = ConnectionHandler {
        file,
        expected_hash: hash,
        block_size: 1024,
        compression_enabled: None,
        write_buffer: vec![],
        compressed_buffer: vec![],
    };

    let wrong_hash = [1u8; 32];
    let prog = ProgressV1 {
        file_hash: wrong_hash,
        bytes_received: 10,
    };
    assert!(handler.handle_progress(&prog).is_err());

    let _ = std::fs::remove_file(path);
}

#[test]
fn test_handle_transfer_complete_success() {
    let data = b"test";
    let hash = calculate_hash(data);
    let (file, path) = create_temp_file(data);

    let mut handler = ConnectionHandler {
        file,
        expected_hash: hash,
        block_size: 1024,
        compression_enabled: None,
        write_buffer: vec![],
        compressed_buffer: vec![],
    };

    let complete = TransferCompleteV1 { file_hash: hash };
    assert!(handler.handle_transfer_complete(&complete).is_ok());

    let _ = std::fs::remove_file(path);
}
