use std::{
    fmt::Display,
    io::{self},
    str::FromStr,
};

use crate::transport::{
    CURRENT_PROTOCOL_VERSION, LENGTH_HEADER_PREFIX, MAX_MESSAGE_SIZE, MESSAGE_DELIMITER,
    VERSION_HEADER_PRIFIX,
};
use serde::Deserialize;

/// Errors that can occur when reading from a stream.
#[derive(thiserror::Error, Debug)]
pub enum StreamReadError {
    /// An I/O error occurred.
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    /// Unexpected EOF: connection closed by peer while reading.
    #[error("Unexpected EOF: connection closed by peer while reading")]
    UnexpectedEof,

    /// The provided buffer was too small to hold the message.
    #[error("Provided buffer is smaller than the requested number of bytes to read (needed at least {min_expected} bytes)")]
    BufferSmallerThanExpected { min_expected: usize },

    /// The message format was invalid (e.g. missing headers, invalid UTF-8).
    #[error("Message format is invalid: {details}")]
    InvalidMessageFormat { details: String },

    /// The protocol version is not supported.
    #[error("Unsupported protocol version: found {found}, expected {expected}")]
    UnsupportedProtocolVersion { found: u8, expected: u8 },

    /// Failed to deserialize the message payload.
    #[error("Failed to parse message payload: {0}")]
    PayloadParseError(#[from] postcard::Error),
}

/// Result of reading a message from the stream
///
/// Includes the parsed message, the index of the next payload in the buffer, and the total number of
/// bytes read from the stream.
///
/// The `next_payload_index` and `total_bytes_reac` fields are used to ensure that any extra data
/// read from the stream is properly handled and that the buffer is correctly updated for subsequent reads.
///
/// `next_payload_index` will be `None` if there is no extra data in the buffer after the current message.
#[derive(Debug)]
pub struct ReadPayloadResult<T> {
    pub message: T,
    pub total_bytes_read: usize,
    pub next_payload_index: Option<usize>,
}

/// Reads a message from the stream, ensuring it starts with the expected headers and version.
///
/// - `stream`: The input stream to read from.
/// - `buffer`: A buffer to store the incoming message data.
/// - `filled_len`: The number of bytes already filled in the buffer (if any).
///
/// Returns a [ReadPayloadResult] containing the parsed message and metadata about the read operation, or a [StreamReadError] on failure.
///
/// ## Guarantees:
/// The function will block until a complete message is read. Uses the `Len: ` header to determine
/// the expected message length and ensures that the entire message is read before returning.
///
/// ## Expectations:
/// 1. The caller must provide a buffer that is large enough to hold the entire message
/// 2. The caller must ensure that any previous message data in the buffer is properly accounted for
/// using the `filled_len` parameter
///
/// ## Errors:
/// - cStreamReadError::BufferSmallerThanExpectedc: If the provided buffer is smaller than the expected message length
/// - [StreamReadError::InvalidMessageFormat]: If the message does not start with the expected headers or version information
/// - [StreamReadError::Io]: For any I/O errors that occur during reading from the stream
pub fn read_next_payload<'a, T, S: io::Read>(
    stream: &mut S,
    buffer: &'a mut [u8],
    filled_len: usize,
) -> Result<ReadPayloadResult<T>, StreamReadError>
where
    T: Deserialize<'a>,
{
    let mut total_bytes_read = filled_len; // Total bytes read from stream

    // Extract header bytes
    let header = loop {
        if total_bytes_read == buffer.len() {
            return Err(StreamReadError::BufferSmallerThanExpected {
                min_expected: MAX_MESSAGE_SIZE,
            });
        }

        let curr_bytes_read = stream.read(&mut buffer[total_bytes_read..])?;
        if curr_bytes_read == 0 {
            return Err(StreamReadError::UnexpectedEof);
        }
        let previous_total = total_bytes_read;
        total_bytes_read = previous_total + curr_bytes_read;

        // Check if the header delimiter is present in the newly read bytes
        let test_crlf_from_idx = previous_total.saturating_sub(2 * MESSAGE_DELIMITER.len() - 1);
        let header_end_index_opt = buffer[test_crlf_from_idx..total_bytes_read]
            .windows(2 * MESSAGE_DELIMITER.len())
            .position(|window| window == [MESSAGE_DELIMITER, MESSAGE_DELIMITER].concat())
            .map(|index| index + test_crlf_from_idx); // Adjust index to account for the offset

        if let Some(header_end) = header_end_index_opt {
            break &buffer[..header_end]; // We have the full header, break with the header slice
        }
    };

    let (version, length) = parse_all_headers(header)?;

    if version != CURRENT_PROTOCOL_VERSION {
        return Err(StreamReadError::UnsupportedProtocolVersion {
            found: version,
            expected: CURRENT_PROTOCOL_VERSION,
        });
    }

    let payload_start_index = header.len() + 2 * MESSAGE_DELIMITER.len();
    let expected_total_length = payload_start_index + length;

    if expected_total_length > buffer.len() {
        return Err(StreamReadError::BufferSmallerThanExpected {
            min_expected: expected_total_length,
        });
    }

    while total_bytes_read < expected_total_length {
        let bytes_read = stream.read(&mut buffer[total_bytes_read..])?;
        total_bytes_read += bytes_read;
    }

    let payload_bytes = &buffer[payload_start_index..expected_total_length];
    let message: T = postcard::from_bytes(payload_bytes)?;
    let next_payload_index = if total_bytes_read > expected_total_length {
        Some(expected_total_length)
    } else {
        None
    };

    Ok(ReadPayloadResult {
        message,
        total_bytes_read,
        next_payload_index,
    })
}

/// Parses the headers from the provided header buffer and extracts the protocol
/// version and payload length.
///
/// Returns the tuple `(version, length)` on success, or a [StreamReadError] if the
/// headers are not in the expected format.
fn parse_all_headers(header_buffer: &[u8]) -> Result<(u8, usize), StreamReadError> {
    let header_lines: Vec<&[u8]> = header_buffer
        .split(|byte| byte == &b'\r' || byte == &b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| line.trim_ascii())
        .collect();

    // First header should be the version header, second should be the length header
    let version = parse_header_line::<u8>(header_lines[0], VERSION_HEADER_PRIFIX.len())?;
    let length = parse_header_line::<usize>(header_lines[1], LENGTH_HEADER_PREFIX.len())?;

    Ok((version, length))
}

/// Parses a header line of the format "Prefix: Value" and extracts the value,
/// converting it to the specified type.
///
/// Returns the parsed value on success, or a [StreamReadError] if the line does not contain the expected prefix,
/// ## Arguments
/// - `line`: The header line to parse, as a byte slice.
/// - `prefix_len`: The length of the expected prefix (including the ": " separator). This is used to
/// split the header line and extract the value portion.
pub fn parse_header_line<ParsedValue: FromStr<Err = impl Display>>(
    line: &[u8],
    prefix_len: usize,
) -> Result<ParsedValue, StreamReadError> {
    let header_value_bytes =
        line.get(prefix_len..)
            .ok_or_else(|| StreamReadError::InvalidMessageFormat {
                details: format!(
                    "Header is too short to contain expected prefix of length {prefix_len}"
                ),
            })?;
    let parsed_value = str::from_utf8(header_value_bytes.trim_ascii())
        .map_err(|e| StreamReadError::InvalidMessageFormat {
            details: format!("Version header is not valid UTF-8: {e}"),
        })?
        .parse::<ParsedValue>()
        .map_err(|e| StreamReadError::InvalidMessageFormat {
            details: format!("Version header does not contain a valid number - {e}"),
        })?;

    Ok(parsed_value)
}

#[cfg(test)]
mod tests {
    use std::io::{PipeReader, Write};

    use super::*;
    use serde::Serialize;

    /// Create a test struct to reduce the complexity of sending
    /// an actual message, while capturing the full flow of reading a message from the stream and parsing it.
    #[derive(Deserialize, Serialize, Debug, PartialEq)]
    struct MockMessage {
        field1: String,
        field2: u32,
    }

    impl MockMessage {
        fn new_dummy_message() -> Self {
            MockMessage {
                field1: "Test".to_string(),
                field2: 42,
            }
        }

        fn get_message_bytes<'a>(&self) -> Vec<u8> {
            let mut buffer = vec![0u8; MAX_MESSAGE_SIZE];
            let payload_bytes =
                postcard::to_slice(self, &mut buffer).expect("Failed to serialize MockMessage");

            let version_header = format!("Ver: {}\r\n", CURRENT_PROTOCOL_VERSION);
            let length_header = format!("Len: {}\r\n", payload_bytes.len());
            let full_message = [
                version_header.as_bytes(),
                length_header.as_bytes(),
                b"\r\n",
                payload_bytes,
            ]
            .concat();
            let full_message_chars = full_message
                .iter()
                .map(|byte| *byte as char)
                .collect::<Vec<_>>();
            println!("Full message bytes: {:?}", full_message_chars);
            full_message.to_owned()
        }
    }

    #[test]
    fn test_parse_all_headers_valid() {
        let header = b"Ver: 1\r\nLen: 42\r\n";
        let (version, length) = parse_all_headers(header).expect("Failed to parse valid headers");
        assert_eq!(version, 1);
        assert_eq!(length, 42);
    }

    #[test]
    fn test_parse_all_headers_invalid_format() {
        let header = b"InvalidHeader\r\n\r\n";
        let err = parse_all_headers(header).unwrap_err();
        assert!(matches!(err, StreamReadError::InvalidMessageFormat { .. }));
    }

    #[test]
    fn test_parse_all_headers_invalid_version() {
        let header = b"Ver: NotANumber\r\nLen: 42\r\n\r\n";
        let err = parse_all_headers(header).unwrap_err();
        match err {
            StreamReadError::InvalidMessageFormat { details } => {
                assert!(details.contains("Version header does not contain a valid number"));
            }
            _ => panic!("Expected InvalidMessageFormat error"),
        }
    }

    #[test]
    fn test_read_next_payload_valid() {
        use std::io::Cursor;

        let message = MockMessage {
            field1: "Hello".to_string(),
            field2: 123,
        };

        let mut buffer = vec![0; 1024];
        let payload_bytes =
            postcard::to_slice(&message, &mut buffer).expect("Failed to serialize test message");

        let full_message = [
            format!("Ver: {}\r\n", CURRENT_PROTOCOL_VERSION).as_bytes(),
            format!("Len: {}\r\n", payload_bytes.len()).as_bytes(),
            b"\r\n",
            payload_bytes,
        ]
        .concat();

        let mut cursor = Cursor::new(&full_message);
        let result = read_next_payload::<MockMessage, _>(&mut cursor, &mut buffer, 0)
            .expect("Failed to read valid payload");
        assert_eq!(result.message, message);
    }

    #[test]
    fn test_read_next_payload_slow_writer() {
        // This test simulates a slow writer by writing the message in small chunks with delays in between.
        // It ensures that read_next_payload can handle partial reads and still correctly parse the message once fully received.
        let message = MockMessage::new_dummy_message();
        let payload_bytes = message.get_message_bytes();

        let (mut reader, mut writer) = io::pipe().expect("Failed to create pipe for testing");

        // Simulate a slow writer by writing the message in small chunks with delays
        std::thread::spawn({
            move || {
                // Write 5 bytes at a time with a delay to simulate slowness
                for chunk in payload_bytes.chunks(5) {
                    writer.write(chunk).expect("Failed to write chunk");
                    writer.flush().expect("Failed to flush writer");
                    std::thread::sleep(std::time::Duration::from_millis(100)); // 100ms delay between chunks
                }
            }
        });

        let mut read_buffer = [0u8; 1024];
        let result = read_next_payload::<MockMessage, PipeReader>(&mut reader, &mut read_buffer, 0)
            .expect("Failed to read payload from slow writer");
        assert_eq!(result.message, message);
    }
}
