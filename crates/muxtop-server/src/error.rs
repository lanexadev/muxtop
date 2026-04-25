use thiserror::Error;

/// Errors from the muxtop server.
#[derive(Debug, Error)]
pub enum ServerError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("protocol error: {0}")]
    Proto(#[from] muxtop_proto::ProtoError),

    #[error("unauthorized")]
    Unauthorized,

    #[error("handshake timeout")]
    HandshakeTimeout,

    #[error("unexpected message: expected {expected}, got {actual}")]
    UnexpectedMessage {
        expected: &'static str,
        actual: String,
    },
}
