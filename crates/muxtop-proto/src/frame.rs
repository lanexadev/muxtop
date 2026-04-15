use crate::ProtoError;

/// Message type discriminants for the wire protocol.
pub const MSG_SNAPSHOT: u8 = 0x01;
pub const MSG_HEARTBEAT: u8 = 0x02;
pub const MSG_ERROR: u8 = 0x03;
pub const MSG_HELLO: u8 = 0x04;
pub const MSG_WELCOME: u8 = 0x05;

/// Maximum frame payload size (4 MiB).
pub const MAX_FRAME_SIZE: u32 = 4 * 1024 * 1024;

/// A framed message on the wire.
///
/// Wire format: `[4 bytes BE length][1 byte msg_type][payload]`
/// where length = 1 + payload.len() (includes the type byte).
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub msg_type: u8,
    pub payload: Vec<u8>,
}

/// Header size: 4 bytes length.
const HEADER_SIZE: usize = 4;

/// Encode a frame into bytes: `[4B BE length][1B type][payload]`.
///
/// # Panics
///
/// Panics if the payload exceeds [`MAX_FRAME_SIZE`] minus 1 (the type byte).
pub fn encode_frame(frame: &Frame) -> Vec<u8> {
    let content_len = 1 + frame.payload.len(); // type byte + payload
    assert!(
        content_len <= MAX_FRAME_SIZE as usize,
        "frame payload too large: {} bytes (max {})",
        frame.payload.len(),
        MAX_FRAME_SIZE - 1,
    );
    let mut buf = Vec::with_capacity(HEADER_SIZE + content_len);
    buf.extend_from_slice(&(content_len as u32).to_be_bytes());
    buf.push(frame.msg_type);
    buf.extend_from_slice(&frame.payload);
    buf
}

/// Decode a frame from a byte slice.
///
/// Returns `(frame, bytes_consumed)` on success.
pub fn decode_frame(buf: &[u8]) -> Result<(Frame, usize), ProtoError> {
    if buf.len() < HEADER_SIZE {
        return Err(ProtoError::IncompleteFrame {
            expected: HEADER_SIZE,
            actual: buf.len(),
        });
    }

    let content_len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);

    if content_len > MAX_FRAME_SIZE {
        return Err(ProtoError::FrameTooLarge {
            size: content_len,
            max: MAX_FRAME_SIZE,
        });
    }

    let total = HEADER_SIZE + content_len as usize;
    if buf.len() < total {
        return Err(ProtoError::IncompleteFrame {
            expected: total,
            actual: buf.len(),
        });
    }

    if content_len == 0 {
        return Err(ProtoError::IncompleteFrame {
            expected: HEADER_SIZE + 1,
            actual: HEADER_SIZE,
        });
    }

    let msg_type = buf[HEADER_SIZE];
    let payload = buf[HEADER_SIZE + 1..total].to_vec();

    Ok((Frame { msg_type, payload }, total))
}

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Reads length-prefixed frames from an async stream.
pub struct FrameReader<R> {
    reader: R,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    /// Read one complete frame from the stream.
    ///
    /// Returns `Ok(None)` on clean EOF (connection closed).
    pub async fn read_frame(&mut self) -> Result<Option<Frame>, ProtoError> {
        // Read 4-byte length header.
        let mut header = [0u8; HEADER_SIZE];
        match self.reader.read_exact(&mut header).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e.into()),
        }

        let content_len = u32::from_be_bytes(header);

        if content_len > MAX_FRAME_SIZE {
            return Err(ProtoError::FrameTooLarge {
                size: content_len,
                max: MAX_FRAME_SIZE,
            });
        }

        if content_len == 0 {
            return Err(ProtoError::IncompleteFrame {
                expected: 1,
                actual: 0,
            });
        }

        // Read type byte.
        let mut type_buf = [0u8; 1];
        self.reader.read_exact(&mut type_buf).await?;
        let msg_type = type_buf[0];

        // Read payload directly into final Vec (no intermediate copy).
        let payload_len = content_len as usize - 1;
        let mut payload = vec![0u8; payload_len];
        if payload_len > 0 {
            self.reader.read_exact(&mut payload).await?;
        }

        Ok(Some(Frame { msg_type, payload }))
    }
}

/// Writes length-prefixed frames to an async stream.
pub struct FrameWriter<W> {
    writer: W,
}

impl<W: AsyncWrite + Unpin> FrameWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    /// Write one frame to the stream and flush.
    pub async fn write_frame(&mut self, frame: &Frame) -> Result<(), ProtoError> {
        let bytes = encode_frame(frame);
        self.writer.write_all(&bytes).await?;
        self.writer.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_encode() {
        let frame = Frame {
            msg_type: MSG_SNAPSHOT,
            payload: vec![0xDE, 0xAD],
        };
        let bytes = encode_frame(&frame);
        // length = 3 (1 type + 2 payload), big-endian
        assert_eq!(&bytes[0..4], &[0, 0, 0, 3]);
        assert_eq!(bytes[4], MSG_SNAPSHOT);
        assert_eq!(&bytes[5..], &[0xDE, 0xAD]);
    }

    #[test]
    fn test_frame_decode() {
        let bytes = [0, 0, 0, 3, MSG_HEARTBEAT, 0xCA, 0xFE];
        let (frame, consumed) = decode_frame(&bytes).unwrap();
        assert_eq!(frame.msg_type, MSG_HEARTBEAT);
        assert_eq!(frame.payload, vec![0xCA, 0xFE]);
        assert_eq!(consumed, 7);
    }

    #[test]
    fn test_frame_roundtrip() {
        let original = Frame {
            msg_type: MSG_ERROR,
            payload: vec![1, 2, 3, 4, 5],
        };
        let bytes = encode_frame(&original);
        let (decoded, consumed) = decode_frame(&bytes).unwrap();
        assert_eq!(original, decoded);
        assert_eq!(consumed, bytes.len());
    }

    #[test]
    fn test_frame_empty_payload() {
        let frame = Frame {
            msg_type: MSG_HELLO,
            payload: vec![],
        };
        let bytes = encode_frame(&frame);
        // length = 1 (just the type byte)
        assert_eq!(&bytes[0..4], &[0, 0, 0, 1]);
        let (decoded, _) = decode_frame(&bytes).unwrap();
        assert_eq!(frame, decoded);
    }

    #[test]
    fn test_frame_truncated_header() {
        let bytes = [0, 0]; // only 2 bytes, need 4
        let err = decode_frame(&bytes).unwrap_err();
        assert!(matches!(
            err,
            ProtoError::IncompleteFrame {
                expected: 4,
                actual: 2
            }
        ));
    }

    #[test]
    fn test_frame_truncated_payload() {
        // Header says 10 bytes of content, but only 3 available after header
        let bytes = [0, 0, 0, 10, MSG_SNAPSHOT, 0xAA, 0xBB];
        let err = decode_frame(&bytes).unwrap_err();
        assert!(matches!(err, ProtoError::IncompleteFrame { .. }));
    }

    #[test]
    fn test_frame_too_large() {
        // Header claims > MAX_FRAME_SIZE
        let size = MAX_FRAME_SIZE + 1;
        let bytes_header = size.to_be_bytes();
        let err = decode_frame(&bytes_header).unwrap_err();
        assert!(matches!(err, ProtoError::FrameTooLarge { .. }));
    }

    #[test]
    fn test_message_type_discriminants() {
        assert_eq!(MSG_SNAPSHOT, 0x01);
        assert_eq!(MSG_HEARTBEAT, 0x02);
        assert_eq!(MSG_ERROR, 0x03);
        assert_eq!(MSG_HELLO, 0x04);
        assert_eq!(MSG_WELCOME, 0x05);
    }

    #[test]
    fn test_frame_all_message_types() {
        for &msg_type in &[
            MSG_SNAPSHOT,
            MSG_HEARTBEAT,
            MSG_ERROR,
            MSG_HELLO,
            MSG_WELCOME,
        ] {
            let frame = Frame {
                msg_type,
                payload: vec![42],
            };
            let bytes = encode_frame(&frame);
            let (decoded, _) = decode_frame(&bytes).unwrap();
            assert_eq!(frame, decoded);
        }
    }

    #[tokio::test]
    async fn test_frame_reader_basic() {
        let frame = Frame {
            msg_type: MSG_SNAPSHOT,
            payload: vec![1, 2, 3],
        };
        let bytes = encode_frame(&frame);
        let cursor = std::io::Cursor::new(bytes);
        let mut reader = FrameReader::new(cursor);

        let result = reader.read_frame().await.unwrap().unwrap();
        assert_eq!(result, frame);
    }

    #[tokio::test]
    async fn test_frame_writer_basic() {
        let frame = Frame {
            msg_type: MSG_HEARTBEAT,
            payload: vec![0xAA, 0xBB],
        };
        let mut buf = Vec::new();
        let mut writer = FrameWriter::new(&mut buf);
        writer.write_frame(&frame).await.unwrap();

        // Verify the written bytes decode correctly.
        let (decoded, _) = decode_frame(&buf).unwrap();
        assert_eq!(decoded, frame);
    }

    #[tokio::test]
    async fn test_frame_reader_writer_duplex() {
        let (client, server) = tokio::io::duplex(1024);
        let mut writer = FrameWriter::new(client);
        let mut reader = FrameReader::new(server);

        let frames = vec![
            Frame {
                msg_type: MSG_HELLO,
                payload: vec![1],
            },
            Frame {
                msg_type: MSG_WELCOME,
                payload: vec![2, 3],
            },
            Frame {
                msg_type: MSG_SNAPSHOT,
                payload: vec![4, 5, 6],
            },
        ];

        for f in &frames {
            writer.write_frame(f).await.unwrap();
        }
        drop(writer); // Close write side.

        for expected in &frames {
            let received = reader.read_frame().await.unwrap().unwrap();
            assert_eq!(&received, expected);
        }

        // Next read should return None (EOF).
        let eof = reader.read_frame().await.unwrap();
        assert!(eof.is_none());
    }

    #[tokio::test]
    async fn test_frame_reader_eof_on_empty() {
        let cursor = std::io::Cursor::new(Vec::<u8>::new());
        let mut reader = FrameReader::new(cursor);
        let result = reader.read_frame().await.unwrap();
        assert!(result.is_none());
    }
}
