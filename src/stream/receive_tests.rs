use crate::stream::error::SendFileError;
use crate::transport::{ReceiverMessageV1, RequestV1, TransferCompleteV1};
use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::PathBuf;

fn calculate_hash(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

fn create_temp_dir() -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "sendfile_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup_temp_dir(dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn gzip_compress(data: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

mod split_blocks_into_ranges_tests {
    fn call_fn(total_blocks: u32, concurrency: u16) -> Vec<std::ops::Range<u32>> {
        crate::stream::receive::split_blocks_into_ranges_for_test(total_blocks, concurrency)
    }

    #[test]
    fn single_block() {
        let ranges = call_fn(1, 1);
        assert_eq!(ranges, vec![0..1]);
    }

    #[test]
    fn equal_division() {
        let ranges = call_fn(10, 2);
        assert_eq!(ranges, vec![0..5, 5..10]);
    }

    #[test]
    fn with_remainder() {
        let ranges = call_fn(10, 3);
        assert_eq!(ranges.len(), 3);
        let covered: std::collections::HashSet<u32> =
            ranges.iter().flat_map(|r| r.clone()).collect();
        assert_eq!(covered.len(), 10);
    }

    #[test]
    fn empty_blocks() {
        let ranges = call_fn(0, 4);
        assert!(ranges.iter().all(|r| r.is_empty()));
    }
}

mod decompress_gzip_tests {
    use super::*;

    fn call_fn(data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
        crate::stream::receive::decompress_gzip_for_test(data)
    }

    #[test]
    fn valid_gzip() {
        let original = b"Hello, World!";
        let compressed = gzip_compress(original);
        let decompressed = call_fn(&compressed).unwrap();
        assert_eq!(decompressed.as_slice(), original);
    }

    #[test]
    fn invalid_gzip() {
        let result = call_fn(b"not gzip data");
        assert!(result.is_err());
    }

    #[test]
    fn empty_gzip() {
        let compressed = gzip_compress(b"");
        let decompressed = call_fn(&compressed).unwrap();
        assert!(decompressed.is_empty());
    }
}

mod determine_final_path_tests {
    use super::*;

    fn call_fn(output_path: &std::path::Path, file_name: &str) -> PathBuf {
        crate::stream::receive::determine_final_path_for_test(output_path, file_name)
    }

    #[test]
    fn directory_path() {
        let temp_dir = create_temp_dir();
        let result = call_fn(&temp_dir, "test.txt");
        assert_eq!(result, temp_dir.join("test.txt"));
        cleanup_temp_dir(&temp_dir);
    }

    #[test]
    fn file_path() {
        let temp_dir = create_temp_dir();
        let file_path = temp_dir.join("existing.txt");
        std::fs::write(&file_path, b"content").unwrap();
        let result = call_fn(&file_path, "ignored.txt");
        assert_eq!(result, file_path);
        cleanup_temp_dir(&temp_dir);
    }

    #[test]
    fn current_directory() {
        let result = call_fn(std::path::Path::new("."), "output.bin");
        assert!(result.ends_with("output.bin"));
    }
}

mod is_transfer_complete_tests {
    fn call_fn(received_blocks: &[bool], total_blocks: u32) -> bool {
        crate::stream::receive::is_transfer_complete_for_test(received_blocks, total_blocks)
    }

    #[test]
    fn all_blocks_received() {
        let received = vec![true; 10];
        assert!(call_fn(&received, 10));
    }

    #[test]
    fn partial_blocks() {
        let received = vec![true, true, false, true];
        assert!(!call_fn(&received, 4));
    }

    #[test]
    fn no_blocks_received() {
        let received = vec![false; 10];
        assert!(!call_fn(&received, 10));
    }
}

mod transfer_protocol_tests {
    use super::*;

    #[test]
    fn request_serde_roundtrip() {
        let file_hash = calculate_hash(b"test");
        let msg = ReceiverMessageV1::Request(RequestV1 { file_hash, seq: 42 });

        let mut buffer = vec![0u8; 1024];
        let serialized = msg.to_bytes(&mut buffer).unwrap();
        let decoded = ReceiverMessageV1::from_bytes(serialized).unwrap();

        assert_eq!(msg, decoded);
    }

    #[test]
    fn transfer_complete_serde_roundtrip() {
        let file_hash = calculate_hash(b"complete");
        let msg = ReceiverMessageV1::TransferComplete(TransferCompleteV1 { file_hash });

        let mut buffer = vec![0u8; 1024];
        let serialized = msg.to_bytes(&mut buffer).unwrap();
        let decoded = ReceiverMessageV1::from_bytes(serialized).unwrap();

        assert_eq!(msg, decoded);
    }
}

mod error_handling_tests {
    use super::*;

    #[test]
    fn io_error_conversion() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let send_error = SendFileError::from(io_error);
        assert!(matches!(send_error, SendFileError::Io(_)));
    }

    #[test]
    fn error_display() {
        let io_error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
        let send_error = SendFileError::from(io_error);
        assert!(format!("{}", send_error).contains("IO error"));
    }
}
