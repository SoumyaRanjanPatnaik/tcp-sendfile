use std::io::{self, Read};

#[derive(thiserror::Error, Debug)]
pub enum StreamReadError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Provided buffer is smaller than the requested number of bytes to read")]
    BufferTooSmall,
}

/// Reads exactly `n` bytes from the provided stream into the given buffer.
pub fn read_n_bytes<S: io::Read>(
    stream: &mut S,
    n: usize,
    buffer: &mut [u8],
) -> Result<(), StreamReadError> {
    if buffer.len() < n {
        return Err(StreamReadError::BufferTooSmall);
    }

    let mut total_read = 0;

    while total_read < n {
        let bytes_read = stream.read(&mut buffer[total_read..])?;
        if bytes_read == 0 {
            break; // EOF reached
        }
        total_read += bytes_read;
    }

    Ok(())
}

/// Reads a message from the stream, ensuring it starts with the expected headers and version.
pub fn read_message<S: io::Read>(
    stream: &mut S,
    buffer: &mut [u8],
) -> Result<usize, StreamReadError> {
    let mut reader = io::BufReader::new(stream);

    let mut total_read = 0;
    loop {
        let Ok(bytes_read) = reader.read(&mut buffer[total_read..]) else {
            continue;
        };
        total_read += bytes_read;

        let ver_prefix = b"Ver: ";
        let len_prefix = b"Len: ";
        if buffer.starts_with(b"Ver: ") {
            // Find the position of the first CRLF
            if let Some(pos) = buffer.windows(CRLF.len()).position(|window| window == CRLF) {
                // Extract the version string
                let version_str = &buffer[ver_prefix.len()..pos];
                if version_str != b"1" {
                    return Err(StreamReadError::Io(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Unsupported protocol version: {}",
                            String::from_utf8_lossy(version_str)
                        ),
                    )));
                }
            } else {
                continue; // Wait for more data to arrive
            }
        }
    }
}
