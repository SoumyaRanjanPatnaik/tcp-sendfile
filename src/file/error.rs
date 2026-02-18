use thiserror::Error;

#[derive(Debug, Error)]
pub enum FileHashError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Error calculating chunk hash: {chunk_index}")]
    ChunkHashError {
        chunk_index: usize,
        source: std::io::Error,
    },
    #[error("Error joining worker thread for hashing chunk with index: {chunk_index}")]
    ThreadJoinError { chunk_index: usize },
}

#[derive(Debug, Error)]
pub enum GetFileMetadataError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Error calculating BLAKE3 hash of the file: {0}")]
    Hash(#[from] FileHashError),
}
