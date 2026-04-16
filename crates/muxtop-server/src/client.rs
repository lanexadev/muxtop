use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::broadcast;
use tokio::time;
use tokio_util::sync::CancellationToken;

use muxtop_core::system::SystemSnapshot;
use muxtop_proto::{FrameReader, FrameWriter, WireMessage};

use crate::error::ServerError;
use crate::server::SharedState;

/// Heartbeat interval: send a keepalive if no snapshot was forwarded in 5 seconds.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// Handshake timeout: client must send Hello within 5 seconds of connecting.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Handle a single client connection: handshake, then stream snapshots + heartbeats.
///
/// Accepts any async reader/writer pair (works with both plain TCP and TLS streams).
pub async fn handle<R, W>(
    reader: R,
    writer: W,
    peer: SocketAddr,
    state: Arc<SharedState>,
    mut snapshot_rx: broadcast::Receiver<SystemSnapshot>,
    token: CancellationToken,
) -> Result<(), ServerError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    // G-21: Acquire semaphore permit — auto-released when this function returns.
    let _permit = match state.semaphore.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            tracing::warn!(peer = %peer, "max clients reached, rejecting");
            let mut frame_reader = FrameReader::new(reader);
            let mut frame_writer = FrameWriter::new(writer);

            // Wait for Hello first (so the client gets a proper rejection).
            let _ = time::timeout(HANDSHAKE_TIMEOUT, frame_reader.read_frame()).await;

            let error_msg = WireMessage::Error {
                code: 503,
                message: "max clients reached".into(),
            };
            let _ = frame_writer.write_frame(&error_msg.to_frame()?).await;
            return Err(ServerError::MaxClientsReached);
        }
    };

    let mut frame_reader = FrameReader::new(reader);
    let mut frame_writer = FrameWriter::new(writer);

    // --- Handshake phase ---
    let hello_frame = time::timeout(HANDSHAKE_TIMEOUT, frame_reader.read_frame())
        .await
        .map_err(|_| ServerError::HandshakeTimeout)?
        .map_err(ServerError::Proto)?
        .ok_or(ServerError::HandshakeTimeout)?;

    let hello = WireMessage::from_frame(&hello_frame).map_err(ServerError::Proto)?;

    match &hello {
        WireMessage::Hello {
            client_version,
            auth_token,
        } => {
            tracing::info!(peer = %peer, client_version = %client_version, "received Hello");

            // G-22: Always validate auth token (mandatory).
            let provided = auth_token.as_deref().unwrap_or("");
            if !constant_time_eq(state.auth_token.as_bytes(), provided.as_bytes()) {
                tracing::warn!(peer = %peer, "authentication failed");
                let error_msg = WireMessage::Error {
                    code: 401,
                    message: "unauthorized".into(),
                };
                let _ = frame_writer.write_frame(&error_msg.to_frame()?).await;
                return Err(ServerError::Unauthorized);
            }
        }
        other => {
            return Err(ServerError::UnexpectedMessage {
                expected: "Hello",
                actual: format!("{other:?}"),
            });
        }
    }

    // Send Welcome.
    let welcome = WireMessage::Welcome {
        server_version: state.server_version.clone(),
        hostname: state.hostname.clone(),
        refresh_hz: state.refresh_hz,
    };
    frame_writer
        .write_frame(&welcome.to_frame()?)
        .await
        .map_err(ServerError::Proto)?;

    tracing::info!(peer = %peer, "handshake complete, streaming");

    // --- Streaming phase ---
    let mut heartbeat_interval = time::interval(HEARTBEAT_INTERVAL);
    // Skip the first immediate tick.
    heartbeat_interval.tick().await;

    loop {
        tokio::select! {
            result = snapshot_rx.recv() => {
                match result {
                    Ok(snapshot) => {
                        let msg = WireMessage::Snapshot(snapshot);
                        frame_writer
                            .write_frame(&msg.to_frame()?)
                            .await
                            .map_err(ServerError::Proto)?;
                        // Reset heartbeat timer after sending a snapshot.
                        heartbeat_interval.reset();
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!(peer = %peer, skipped = n, "client lagged, skipping snapshots");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::debug!(peer = %peer, "broadcast closed");
                        break;
                    }
                }
            }
            _ = heartbeat_interval.tick() => {
                let uptime = state.start_time.elapsed().as_secs();
                let msg = WireMessage::Heartbeat {
                    server_version: state.server_version.clone(),
                    uptime_secs: uptime,
                };
                frame_writer
                    .write_frame(&msg.to_frame()?)
                    .await
                    .map_err(ServerError::Proto)?;
                tracing::debug!(peer = %peer, "heartbeat sent");
            }
            _ = token.cancelled() => {
                tracing::debug!(peer = %peer, "shutdown signal received");
                break;
            }
        }
    }

    Ok(())
}

/// Constant-time byte comparison to prevent timing attacks on token validation.
///
/// Both length and content are compared without early returns to avoid
/// leaking the expected token length via timing side-channel.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let len_matches = a.len() == b.len();
    let max_len = std::cmp::max(a.len(), b.len());
    let mut diff = 0u8;
    for i in 0..max_len {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= x ^ y;
    }
    diff == 0 && len_matches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_time_eq_equal() {
        assert!(constant_time_eq(b"secret", b"secret"));
    }

    #[test]
    fn test_constant_time_eq_different() {
        assert!(!constant_time_eq(b"secret", b"wrong!"));
    }

    #[test]
    fn test_constant_time_eq_different_length() {
        assert!(!constant_time_eq(b"short", b"longer"));
    }

    #[test]
    fn test_constant_time_eq_empty() {
        assert!(constant_time_eq(b"", b""));
    }
}
