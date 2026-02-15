use std::{
    io::{self},
    str::FromStr,
};

use crate::transport::{
    TransportMessageV1, CURRENT_PROTOCOL_VERSION, LENGTH_HEADER_PREFIX, MAX_MESSAGE_SIZE,
    MESSAGE_DELIMITER, VERSION_HEADER_PRIFIX,
};

#[derive(thiserror::Error, Debug)]
pub enum StreamReadError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Provided buffer is smaller than the requested number of bytes to read (needed at least {min_expected} bytes)")]
    BufferSmallerThanExpected { min_expected: usize },

    #[error("Message format is invalid: {details}")]
    InvalidMessageFormat { details: String },

    #[error("Unsupported protocol version: found {found}, expected {expected}")]
    UnsupportedProtocolVersion { found: u8, expected: u8 },

    #[error("Failed to parse message payload: {0}")]
    PayloadParseError(#[from] crate::transport::TransportError),
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
pub struct ReadPayloadResult<'a> {
    pub message: TransportMessageV1<'a>,
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
pub fn read_next_payload<'a, S: io::Read>(
    stream: &mut S,
    buffer: &'a mut [u8],
    filled_len: usize,
) -> Result<ReadPayloadResult<'a>, StreamReadError> {
    let mut total_bytes_read = filled_len; // Total bytes read from stream

    // Extract header bytes
    let header = loop {
        if total_bytes_read == buffer.len() {
            return Err(StreamReadError::BufferSmallerThanExpected {
                min_expected: MAX_MESSAGE_SIZE,
            });
        }

        let curr_bytes_read = stream.read(&mut buffer[total_bytes_read..])?;
        let previous_total = total_bytes_read;
        let total_bytes_read = previous_total + curr_bytes_read;

        // Check if the header delimiter is present in the newly read bytes
        let header_end_index_opt = buffer[previous_total..total_bytes_read]
            .windows(2 * MESSAGE_DELIMITER.len())
            .position(|window| window == [MESSAGE_DELIMITER, MESSAGE_DELIMITER].concat())
            .map(|index| index + previous_total);

        if let Some(header_end) = header_end_index_opt {
            break &buffer[..header_end];
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
    let message = TransportMessageV1::from_bytes(payload_bytes)?;
    let next_payload_index = if total_bytes_read > expected_total_length {
        Some(expected_total_length)
    } else {
        None
    };

    return Ok(ReadPayloadResult {
        message,
        total_bytes_read,
        next_payload_index,
    });
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
fn parse_header_line<ParsedValue: FromStr>(
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
    let parsed_value = str::from_utf8(header_value_bytes)
        .map_err(|e| StreamReadError::InvalidMessageFormat {
            details: format!("Version header is not valid UTF-8: {e}"),
        })?
        .parse::<ParsedValue>()
        .map_err(|_| StreamReadError::InvalidMessageFormat {
            details: format!("Version header does not contain a valid number"),
        })?;

    Ok(parsed_value)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
