//! Utility functions for file handling, such as calculating the BLAKE3 hash of a file.
use crate::transport::MAX_BLOCK_SIZE;
use blake3::Hasher;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::thread;

const PARALLEL_CHUNK_SIZE: u64 = 8 * 1024 * 1024; // 8 MB per chunk for parallel hashing

/// Calculates the BLAKE3 hash of a file at the given path using parallel hashing.
pub fn get_file_blake3_hash(file_path: &std::path::Path) -> Result<[u8; 32], std::io::Error> {
    let metadata = std::fs::metadata(file_path)?;
    let file_size = metadata.len();

    if file_size <= PARALLEL_CHUNK_SIZE {
        return hash_sequential(file_path);
    }

    let num_chunks = file_size.div_ceil(PARALLEL_CHUNK_SIZE);
    let _num_threads = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(num_chunks as usize)
        .max(1);

    let chunk_handles: Vec<_> = (0..num_chunks)
        .map(|chunk_idx| {
            let path = file_path.to_path_buf();
            let start = chunk_idx * PARALLEL_CHUNK_SIZE;
            let end = ((chunk_idx + 1) * PARALLEL_CHUNK_SIZE).min(file_size);

            thread::spawn(move || {
                let mut file = File::open(&path)?;
                file.seek(SeekFrom::Start(start))?;
                let chunk_size = (end - start) as usize;
                let mut buffer = vec![0u8; chunk_size];
                file.read_exact(&mut buffer)?;
                let mut hasher = Hasher::new();
                hasher.update(&buffer);
                Ok::<_, std::io::Error>(hasher.finalize())
            })
        })
        .collect();

    let mut final_hasher = Hasher::new();
    for handle in chunk_handles {
        let chunk_hash = handle.join().unwrap()?;
        final_hasher.update(chunk_hash.as_bytes());
    }

    let result = final_hasher.finalize();
    let mut hash_array = [0u8; 32];
    hash_array.copy_from_slice(result.as_bytes());
    Ok(hash_array)
}

fn hash_sequential(file_path: &std::path::Path) -> Result<[u8; 32], std::io::Error> {
    let file = File::open(file_path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Hasher::new();

    let mut buffer = vec![0u8; MAX_BLOCK_SIZE as usize];
    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    let result = hasher.finalize();
    let mut hash_array = [0u8; 32];
    hash_array.copy_from_slice(result.as_bytes());
    Ok(hash_array)
}

/// Reads a specific block from the file.
///
/// Seeks to `seq * block_size` and reads up to `block_size` bytes.
/// Returns the bytes read. If EOF is reached, the returned vector will be smaller than `block_size`.
pub fn read_file_block(
    file: &mut File,
    seq: u32,
    block_size: u32,
) -> Result<Vec<u8>, std::io::Error> {
    let offset = seq as u64 * block_size as u64;
    file.seek(SeekFrom::Start(offset))?;

    let mut buffer = vec![0u8; block_size as usize];
    let mut bytes_read = 0;
    while bytes_read < block_size as usize {
        let read = file.read(&mut buffer[bytes_read..])?;
        if read == 0 {
            break;
        }
        bytes_read += read;
    }
    buffer.truncate(bytes_read);
    Ok(buffer)
}

/// Writes a specific block to the file.
///
/// Seeks to `seq * block_size` and writes the data bytes.
pub fn write_file_block(
    file: &mut File,
    seq: u32,
    block_size: u32,
    data: &[u8],
) -> Result<(), std::io::Error> {
    let offset = seq as u64 * block_size as u64;
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env::temp_dir, fs::OpenOptions, io::Write};

    #[test]
    fn test_get_file_blake3_hash() {
        let temp_file_path = temp_dir().join("test_file.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temp file");

        let content: Vec<u8> = vec![b'a'; 2 * MAX_BLOCK_SIZE as usize];

        temp_file
            .write_all(&content)
            .expect("Failed to write to temp file");

        let hash = get_file_blake3_hash(&temp_file_path).expect("Failed to get file hash");

        let mut hasher = Hasher::new();
        hasher.update(&content);
        let expected_hash: [u8; 32] = hasher.finalize().into();

        assert_eq!(hash, expected_hash);
    }

    #[test]
    fn test_read_file_block() {
        let temp_file_path = temp_dir().join("test_read_block.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temp file");

        let content = b"1234567890";
        temp_file
            .write_all(content)
            .expect("Failed to write to temp file");
        temp_file.flush().expect("Failed to flush temp file");

        let mut file = File::open(&temp_file_path).expect("Failed to open temp file");

        // Read first block (size 4)
        let block1 = read_file_block(&mut file, 0, 4).expect("Failed to read block 1");
        assert_eq!(block1, b"1234");

        // Read second block (size 4)
        let block2 = read_file_block(&mut file, 1, 4).expect("Failed to read block 2");
        assert_eq!(block2, b"5678");

        // Read partial block (size 4, but only 2 left)
        let block3 = read_file_block(&mut file, 2, 4).expect("Failed to read block 3");
        assert_eq!(block3, b"90");

        // Read past EOF
        let block4 = read_file_block(&mut file, 3, 4).expect("Failed to read block 4");
        assert!(block4.is_empty());
    }

    #[test]
    fn test_write_file_block() {
        let temp_file_path = temp_dir().join("test_write_block.txt");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&temp_file_path)
            .expect("Failed to create temp file");

        file.set_len(12).expect("Failed to pre-allocate file");

        write_file_block(&mut file, 0, 4, b"AAAA").expect("Failed to write block 0");
        write_file_block(&mut file, 1, 4, b"BBBB").expect("Failed to write block 1");
        write_file_block(&mut file, 2, 4, b"CC").expect("Failed to write partial block 2");

        let written = read_file_block(&mut file, 0, 4).expect("Failed to read block 0");
        assert_eq!(written, b"AAAA");

        let written = read_file_block(&mut file, 1, 4).expect("Failed to read block 1");
        assert_eq!(written, b"BBBB");

        let written = read_file_block(&mut file, 2, 4).expect("Failed to read block 2");
        assert_eq!(written, b"CC\x00\x00");

        file.set_len(10).expect("Failed to truncate file");
        let written =
            read_file_block(&mut file, 2, 4).expect("Failed to read block 2 after truncate");
        assert_eq!(written, b"CC");
    }
}
