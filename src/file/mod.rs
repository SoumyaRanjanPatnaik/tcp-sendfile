pub mod utils;

#[derive(Debug)]
pub struct FileMetadata {
    // Original file name, used for allocating the file on the receiver side and for display purposes.
    name: String,
    /// Size of the file in bytes
    size: u64,
    /// SHA-256 hash of the file content
    hash: [u8; 32],
}

impl FileMetadata {
    pub fn new(filename: String, filesize: u64, filehash: [u8; 32]) -> Self {
        Self {
            name: filename,
            size: filesize,
            hash: filehash,
        }
    }

    pub fn from_file(path: &std::path::Path) -> std::io::Result<Self> {
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unnamed_file")
            .to_string();

        let filesize = std::fs::metadata(path)?.len();
        let filehash = utils::get_file_sha256_hash(path)?;

        Ok(Self {
            name: filename,
            size: filesize,
            hash: filehash,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn hash(&self) -> [u8; 32] {
        self.hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{env::temp_dir, fs::File, io::Write};

    #[test]
    fn test_file_metadata_from_file() {
        // Create a temporary file with known content
        let temp_file_path = temp_dir().join("test_file.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temp file");

        let content = b"Hello, world!";
        temp_file
            .write_all(content)
            .expect("Failed to write to temp file");

        temp_file.flush().expect("Failed to flush temp file"); // Ensure all data is written to disk

        // Create FileMetadata from the file
        let metadata =
            FileMetadata::from_file(&temp_file_path).expect("Failed to create FileMetadata");

        let mut hasher = Sha256::new();
        hasher.update(content);
        let expected_hash = hasher.finalize();

        // Verify the metadata
        assert_eq!(metadata.name(), "test_file.txt");
        assert_eq!(metadata.size(), content.len() as u64);
        assert_eq!(metadata.hash(), expected_hash.as_slice(),);
    }
}
