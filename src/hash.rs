use crate::transport::MAX_BLOCK_SIZE;
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::fs::File;
use std::io::{BufReader, Read};

enum EndOfFile {
    Reached,
    NotReached,
}

pub fn get_file_sha256_hash(file_path: &std::path::Path) -> Result<[u8; 32], std::io::Error> {
    let file = File::open(file_path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();

    // Use a thread-local buffer to avoid reallocating the buffer on each call
    thread_local! {
        static BUFFER: RefCell<Vec<u8>> = RefCell::new(vec![0u8; MAX_BLOCK_SIZE as usize]);
    }

    loop {
        let result = BUFFER.with(|buffer_cell| {
            let mut buffer = buffer_cell.borrow_mut();
            let bytes_read = reader.read(&mut buffer)?;

            if bytes_read == 0 {
                return Ok(EndOfFile::Reached);
            }

            hasher.update(&buffer[..bytes_read]);
            Ok(EndOfFile::NotReached)
        });

        match result {
            Ok(EndOfFile::Reached) => break,
            Ok(EndOfFile::NotReached) => continue,
            Err(e) => return Err(e),
        }
    }

    let result = hasher.finalize();
    let mut hash_array = [0u8; 32];
    hash_array.copy_from_slice(&result);
    Ok(hash_array)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env::temp_dir, io::Write};

    #[test]
    fn test_get_file_sha256_hash() {
        // Create a temporary file with known content
        let temp_file_path = temp_dir().join("test_file.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temp file");

        // Allocate a large content to ensure multiple reads are required
        let content: Vec<u8> = vec![b'a'; 2 * MAX_BLOCK_SIZE as usize];

        temp_file
            .write_all(&content)
            .expect("Failed to write to temp file");

        // Get the hash from the function
        let hash = get_file_sha256_hash(&temp_file_path).expect("Failed to get file hash");

        // Calculate the expected hash using the same content
        let mut hasher = Sha256::new();
        hasher.update(content);
        let expected_hash = hasher.finalize();

        // Assert that the calculated hash matches the expected hash
        assert_eq!(hash, expected_hash.as_slice());
    }
}
