use log::debug;

pub mod utils;

#[derive(Debug)]
pub struct FileMetadata {
    // Original file name, used for allocating the file on the receiver side and for display purposes.
    name: String,
    /// Size of the file in bytes
    size: u64,
    /// BLAKE3 hash of the file content
    hash: [u8; 32],
}

impl FileMetadata {
    /// Creates a new `FileMetadata` instance.
    pub fn new(filename: String, filesize: u64, filehash: [u8; 32]) -> Self {
        Self {
            name: filename,
            size: filesize,
            hash: filehash,
        }
    }

    /// Creates a `FileMetadata` instance from a file path.
    ///
    /// This function reads the file metadata to get the size and calculates the BLAKE3 hash of the file content.
    ///
    /// # Arguments
    ///
    /// * `path` - The path to the file.
    ///
    /// # Returns
    ///
    /// A `Result` containing the `FileMetadata` or an `io::Error`.
    pub fn from_file(path: &std::path::Path) -> std::io::Result<Self> {
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unnamed_file")
            .to_string();
        debug!("Calculating metadata for file: {:?}", path);

        let filesize = std::fs::metadata(path)?.len();
        debug!("File size: {} bytes", filesize);

        let filehash = utils::get_file_blake3_hash(path)?;
        debug!("File hash (BLAKE3): {:x?}", filehash);

        Ok(Self {
            name: filename,
            size: filesize,
            hash: filehash,
        })
    }

    /// Returns the name of the file.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the size of the file in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Returns the BLAKE3 hash of the file.
    pub fn hash(&self) -> [u8; 32] {
        self.hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blake3::Hasher;
    use std::{env::temp_dir, fs::File, io::Write, thread};

    #[test]
    fn test_file_metadata_from_file() {
        // Create a temporary file with known content
        let temp_file_path = temp_dir().join("file_metadata_test.txt");
        let mut temp_file = File::create(&temp_file_path).expect("Failed to create temp file");

        let content = b"Hello, world!";
        temp_file
            .write_all(content)
            .expect("Failed to write to temp file");

        temp_file.flush().expect("Failed to flush temp file"); // Ensure all data is written to disk

        thread::sleep(std::time::Duration::from_millis(100)); // Small delay to ensure file system updates

        // Create FileMetadata from the file
        let metadata =
            FileMetadata::from_file(&temp_file_path).expect("Failed to create FileMetadata");

        let mut hasher = Hasher::new();
        hasher.update(content);
        let expected_hash = hasher.finalize();

        // Verify the metadata
        assert_eq!(
            metadata.name(),
            "file_metadata_test.txt",
            "File name mismatch"
        );
        assert_eq!(metadata.size(), content.len() as u64, "File size mismatch");
        assert_eq!(
            metadata.hash(),
            expected_hash.as_slice(),
            "File hash mismatch"
        );
    }
}
