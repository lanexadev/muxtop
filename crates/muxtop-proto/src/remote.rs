// Remote collector: TCP client that receives snapshots from muxtop-server via TLS.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tokio_util::sync::CancellationToken;

use crate::{FrameReader, FrameWriter, WireMessage};

use muxtop_core::system::SystemSnapshot;

/// Maximum backoff delay between reconnection attempts.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Initial backoff delay.
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);

/// Timeout for the initial handshake (Hello/Welcome exchange).
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Connection event sent to the TUI for status display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionEvent {
    /// Successfully connected (or reconnected) to server.
    Connected { hostname: String },
    /// Connection lost — reconnection will be attempted.
    Disconnected,
    /// Server sent an error (e.g. 401 unauthorized, 503 max clients).
    ServerError { code: u16, message: String },
}

/// A TLS client that connects to a muxtop-server and receives snapshots.
pub struct RemoteCollector {
    addr: SocketAddr,
    token: Option<String>,
    tls_connector: TlsConnector,
    server_name: rustls_pki_types::ServerName<'static>,
}

impl RemoteCollector {
    /// Create a new remote collector targeting the given server address.
    pub fn new(
        addr: SocketAddr,
        token: Option<String>,
        tls_connector: TlsConnector,
        server_name: rustls_pki_types::ServerName<'static>,
    ) -> Self {
        Self {
            addr,
            token,
            tls_connector,
            server_name,
        }
    }

    /// Spawn the remote collector as a background tokio task.
    ///
    /// This has the same signature pattern as [`crate::collector::Collector::spawn`]:
    /// it sends `SystemSnapshot` values into `tx` and shuts down when `cancel` fires.
    ///
    /// An optional `conn_tx` channel receives [`ConnectionEvent`] notifications
    /// so the TUI can display connection status.
    pub fn spawn(
        self,
        tx: mpsc::Sender<SystemSnapshot>,
        conn_tx: Option<mpsc::Sender<ConnectionEvent>>,
        cancel: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(self.run(tx, conn_tx, cancel))
    }

    async fn run(
        self,
        tx: mpsc::Sender<SystemSnapshot>,
        conn_tx: Option<mpsc::Sender<ConnectionEvent>>,
        cancel: CancellationToken,
    ) {
        let mut backoff = INITIAL_BACKOFF;

        loop {
            if cancel.is_cancelled() {
                break;
            }

            match self.connect_and_stream(&tx, &conn_tx, &cancel).await {
                Ok(()) => {
                    // Clean shutdown (cancel fired).
                    break;
                }
                Err(e) => {
                    tracing::warn!("remote connection error: {e}");
                    Self::send_conn_event(&conn_tx, ConnectionEvent::Disconnected).await;

                    // Wait with exponential backoff, but respect cancellation.
                    tokio::select! {
                        () = tokio::time::sleep(backoff) => {}
                        () = cancel.cancelled() => break,
                    }

                    // G-01: Only grow backoff for consecutive failures.
                    // ConnectionClosed means we had a successful connection that dropped,
                    // so reset backoff for a fresh reconnection attempt.
                    if matches!(e, RemoteError::ConnectionClosed) {
                        backoff = INITIAL_BACKOFF;
                    } else {
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                    }
                }
            }
        }

        tracing::debug!("remote collector shutting down");
    }

    /// Connect to the server, perform handshake, and stream snapshots.
    ///
    /// Returns `Ok(())` only when cancel is fired (clean shutdown).
    /// Returns `Err` on any connection/protocol error (caller will reconnect).
    async fn connect_and_stream(
        &self,
        tx: &mpsc::Sender<SystemSnapshot>,
        conn_tx: &Option<mpsc::Sender<ConnectionEvent>>,
        cancel: &CancellationToken,
    ) -> Result<(), RemoteError> {
        // Connect with cancellation support.
        let tcp_stream = tokio::select! {
            result = TcpStream::connect(self.addr) => result?,
            () = cancel.cancelled() => return Ok(()),
        };

        // TLS handshake (with timeout and cancellation).
        let tls_stream = tokio::select! {
            result = timeout(HANDSHAKE_TIMEOUT, self.tls_connector.connect(self.server_name.clone(), tcp_stream)) => {
                result
                    .map_err(|_| RemoteError::Protocol("TLS handshake timed out".into()))?
                    .map_err(|e| RemoteError::Protocol(format!("TLS handshake failed: {e}")))?
            }
            () = cancel.cancelled() => return Ok(()),
        };

        let (reader, writer) = tokio::io::split(tls_stream);
        let mut frame_reader = FrameReader::new(reader);
        let mut frame_writer = FrameWriter::new(writer);

        // --- Handshake ---
        let hello = WireMessage::Hello {
            client_version: env!("CARGO_PKG_VERSION").to_string(),
            auth_token: self.token.clone(),
        };
        let hello_frame = hello
            .to_frame()
            .map_err(|e| RemoteError::Protocol(e.to_string()))?;
        frame_writer
            .write_frame(&hello_frame)
            .await
            .map_err(|e| RemoteError::Protocol(e.to_string()))?;

        // Read Welcome (with timeout).
        let welcome_frame = timeout(HANDSHAKE_TIMEOUT, frame_reader.read_frame())
            .await
            .map_err(|_| RemoteError::HandshakeTimeout)?
            .map_err(|e| RemoteError::Protocol(e.to_string()))?
            .ok_or(RemoteError::ConnectionClosed)?;

        let welcome_msg = WireMessage::from_frame(&welcome_frame)
            .map_err(|e| RemoteError::Protocol(e.to_string()))?;

        let hostname = match welcome_msg {
            WireMessage::Welcome { hostname, .. } => hostname,
            WireMessage::Error { code, message } => {
                Self::send_conn_event(
                    conn_tx,
                    ConnectionEvent::ServerError {
                        code,
                        message: message.clone(),
                    },
                )
                .await;
                return Err(RemoteError::ServerError { code, message });
            }
            other => {
                return Err(RemoteError::Protocol(format!(
                    "expected Welcome, got {other:?}"
                )));
            }
        };

        tracing::info!("connected to {hostname} at {}", self.addr);
        Self::send_conn_event(conn_tx, ConnectionEvent::Connected { hostname }).await;

        // Reset backoff on successful connection — caller handles this by restarting.
        // But we can't mutate backoff here since it's in the caller scope.
        // The caller resets backoff in the loop.

        // --- Streaming loop ---
        loop {
            tokio::select! {
                frame_result = frame_reader.read_frame() => {
                    let frame = match frame_result {
                        Ok(Some(f)) => f,
                        Ok(None) => return Err(RemoteError::ConnectionClosed),
                        Err(e) => return Err(RemoteError::Protocol(e.to_string())),
                    };

                    let msg = WireMessage::from_frame(&frame)
                        .map_err(|e| RemoteError::Protocol(e.to_string()))?;

                    match msg {
                        WireMessage::Snapshot(snapshot) => {
                            match tx.try_send(snapshot) {
                                Ok(()) => {}
                                Err(mpsc::error::TrySendError::Full(_)) => {
                                    tracing::trace!("channel full, dropping remote snapshot");
                                }
                                Err(mpsc::error::TrySendError::Closed(_)) => {
                                    tracing::debug!("channel closed, stopping remote collector");
                                    return Ok(());
                                }
                            }
                        }
                        WireMessage::Heartbeat { .. } => {
                            tracing::trace!("heartbeat received");
                        }
                        WireMessage::Error { code, message } => {
                            tracing::error!("server error {code}: {message}");
                            Self::send_conn_event(
                                conn_tx,
                                ConnectionEvent::ServerError { code, message: message.clone() },
                            ).await;
                            return Err(RemoteError::ServerError { code, message });
                        }
                        other => {
                            tracing::warn!("unexpected message: {other:?}");
                        }
                    }
                }
                () = cancel.cancelled() => {
                    tracing::debug!("remote collector cancelled");
                    return Ok(());
                }
            }
        }
    }

    async fn send_conn_event(
        conn_tx: &Option<mpsc::Sender<ConnectionEvent>>,
        event: ConnectionEvent,
    ) {
        if let Some(tx) = conn_tx {
            let _ = tx.try_send(event);
        }
    }
}

/// Errors from the remote collector (internal, not exposed to users).
#[derive(Debug, thiserror::Error)]
pub enum RemoteError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("handshake timeout")]
    HandshakeTimeout,

    #[error("connection closed")]
    ConnectionClosed,

    #[error("server error {code}: {message}")]
    ServerError { code: u16, message: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FrameReader, FrameWriter, WireMessage};
    use crate::tls::connector_insecure;
    use std::io::BufReader;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio_rustls::TlsAcceptor;
    use tokio_rustls::rustls::ServerConfig;

    /// Generate self-signed cert and build a TLS acceptor for testing.
    fn test_tls_acceptor() -> TlsAcceptor {
        let ck = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert_pem = ck.cert.pem();
        let key_pem = ck.signing_key.serialize_pem();

        let certs: Vec<_> = rustls_pemfile::certs(&mut BufReader::new(cert_pem.as_bytes()))
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let key = rustls_pemfile::private_key(&mut BufReader::new(key_pem.as_bytes()))
            .unwrap()
            .unwrap();

        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap();

        TlsAcceptor::from(Arc::new(config))
    }

    /// Build a test RemoteCollector with insecure TLS (skip verify).
    fn test_collector(addr: SocketAddr, token: Option<String>) -> RemoteCollector {
        let tls_connector = connector_insecure();
        let server_name =
            rustls_pki_types::ServerName::IpAddress(addr.ip().into());
        RemoteCollector::new(addr, token, tls_connector, server_name)
    }

    /// Helper: start a TLS mock server that sends Welcome then streams one snapshot.
    async fn mock_server(auth_token: Option<&str>) -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let tls_acceptor = test_tls_acceptor();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let expected_token = auth_token.map(String::from);

        let handle = tokio::spawn(async move {
            let (tcp_stream, _) = listener.accept().await.unwrap();
            let tls_stream = tls_acceptor.accept(tcp_stream).await.unwrap();
            let (reader, writer) = tokio::io::split(tls_stream);
            let mut frame_reader = FrameReader::new(reader);
            let mut frame_writer = FrameWriter::new(writer);

            // Read Hello.
            let frame = frame_reader.read_frame().await.unwrap().unwrap();
            let msg = WireMessage::from_frame(&frame).unwrap();
            match msg {
                WireMessage::Hello { auth_token, .. } => {
                    if let Some(ref expected) = expected_token {
                        if auth_token.as_deref() != Some(expected.as_str()) {
                            let err = WireMessage::Error {
                                code: 401,
                                message: "unauthorized".to_string(),
                            };
                            let f = err.to_frame().unwrap();
                            frame_writer.write_frame(&f).await.unwrap();
                            return;
                        }
                    }
                }
                _ => panic!("expected Hello"),
            }

            // Send Welcome.
            let welcome = WireMessage::Welcome {
                server_version: "0.1.1".to_string(),
                hostname: "test-host".to_string(),
                refresh_hz: 1,
            };
            let f = welcome.to_frame().unwrap();
            frame_writer.write_frame(&f).await.unwrap();

            // Send one snapshot.
            let snapshot = make_test_snapshot();
            let snap_msg = WireMessage::Snapshot(snapshot);
            let f = snap_msg.to_frame().unwrap();
            frame_writer.write_frame(&f).await.unwrap();

            // Keep connection alive briefly.
            tokio::time::sleep(Duration::from_millis(200)).await;
        });

        (addr, handle)
    }

    fn make_test_snapshot() -> SystemSnapshot {
        use muxtop_core::network::{NetworkInterfaceSnapshot, NetworkSnapshot};
        use muxtop_core::process::ProcessInfo;
        use muxtop_core::system::{CoreSnapshot, CpuSnapshot, LoadSnapshot, MemorySnapshot};

        SystemSnapshot {
            cpu: CpuSnapshot {
                global_usage: 25.0,
                cores: vec![CoreSnapshot {
                    name: "cpu0".into(),
                    usage: 25.0,
                    frequency: 3600,
                }],
            },
            memory: MemorySnapshot {
                total: 16_000_000_000,
                used: 8_000_000_000,
                available: 8_000_000_000,
                swap_total: 4_000_000_000,
                swap_used: 1_000_000_000,
            },
            load: LoadSnapshot {
                one: 1.0,
                five: 0.8,
                fifteen: 0.5,
                uptime_secs: 3600,
            },
            processes: vec![ProcessInfo {
                pid: 1,
                parent_pid: None,
                name: "init".into(),
                command: "/sbin/init".into(),
                user: "root".into(),
                cpu_percent: 0.1,
                memory_bytes: 4096,
                memory_percent: 0.01,
                status: "Running".into(),
            }],
            networks: NetworkSnapshot {
                interfaces: vec![NetworkInterfaceSnapshot {
                    name: "lo".into(),
                    bytes_rx: 1000,
                    bytes_tx: 1000,
                    packets_rx: 10,
                    packets_tx: 10,
                    errors_rx: 0,
                    errors_tx: 0,
                    mac_address: "00:00:00:00:00:00".into(),
                    is_up: true,
                }],
                total_rx: 1000,
                total_tx: 1000,
            },
            timestamp_ms: 1_713_200_000_000,
        }
    }

    #[tokio::test]
    async fn test_remote_collector_handshake() {
        let (addr, server) = mock_server(None).await;

        let (tx, mut rx) = mpsc::channel(4);
        let (conn_tx, mut conn_rx) = mpsc::channel(4);
        let token = CancellationToken::new();

        let collector = test_collector(addr, None);
        let handle = collector.spawn(tx, Some(conn_tx), token.clone());

        // Should receive Connected event.
        let event = tokio::time::timeout(Duration::from_secs(3), conn_rx.recv())
            .await
            .expect("timeout waiting for connection event")
            .expect("channel closed");
        assert_eq!(
            event,
            ConnectionEvent::Connected {
                hostname: "test-host".to_string()
            }
        );

        // Should receive a snapshot.
        let snap = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("timeout waiting for snapshot")
            .expect("channel closed");
        assert!(!snap.processes.is_empty());

        token.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
        server.abort();
    }

    #[tokio::test]
    async fn test_remote_collector_receives_snapshots() {
        let (addr, server) = mock_server(None).await;

        let (tx, mut rx) = mpsc::channel(4);
        let token = CancellationToken::new();

        let collector = test_collector(addr, None);
        let handle = collector.spawn(tx, None, token.clone());

        let snap = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        assert_eq!(snap.cpu.global_usage, 25.0);
        assert_eq!(snap.processes.len(), 1);
        assert_eq!(snap.processes[0].name, "init");

        token.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
        server.abort();
    }

    #[tokio::test]
    async fn test_remote_collector_shutdown() {
        let (addr, server) = mock_server(None).await;

        let (tx, _rx) = mpsc::channel(4);
        let token = CancellationToken::new();

        let collector = test_collector(addr, None);
        let handle = collector.spawn(tx, None, token.clone());

        // Wait briefly for connection, then cancel.
        tokio::time::sleep(Duration::from_millis(300)).await;
        token.cancel();

        // Should complete within 2 seconds.
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("collector did not shut down")
            .expect("collector panicked");

        server.abort();
    }

    #[tokio::test]
    async fn test_remote_collector_auth_failure() {
        let (addr, server) = mock_server(Some("correct-token")).await;

        let (tx, _rx) = mpsc::channel(4);
        let (conn_tx, mut conn_rx) = mpsc::channel(4);
        let token = CancellationToken::new();

        let collector = test_collector(addr, Some("wrong-token".to_string()));
        let handle = collector.spawn(tx, Some(conn_tx), token.clone());

        // Should receive ServerError event (401).
        let event = tokio::time::timeout(Duration::from_secs(3), conn_rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");
        assert!(
            matches!(event, ConnectionEvent::ServerError { code: 401, .. }),
            "expected 401, got {event:?}"
        );

        token.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
        server.abort();
    }

    #[tokio::test]
    async fn test_remote_collector_reconnect_backoff() {
        // No server running — connection should fail and retry with backoff.
        let addr: SocketAddr = "127.0.0.1:19999".parse().unwrap();

        let (tx, _rx) = mpsc::channel(4);
        let (conn_tx, mut conn_rx) = mpsc::channel(16);
        let token = CancellationToken::new();

        let collector = test_collector(addr, None);
        let handle = collector.spawn(tx, Some(conn_tx), token.clone());

        // Should get Disconnected events as connection attempts fail.
        let event = tokio::time::timeout(Duration::from_secs(3), conn_rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");
        assert_eq!(event, ConnectionEvent::Disconnected);

        // Cancel before too many retries.
        token.cancel();
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("collector did not shut down")
            .expect("collector panicked");
    }

    #[test]
    fn test_remote_error_display() {
        let errors: Vec<RemoteError> = vec![
            RemoteError::Io(std::io::Error::other("test")),
            RemoteError::Protocol("bad frame".to_string()),
            RemoteError::HandshakeTimeout,
            RemoteError::ConnectionClosed,
            RemoteError::ServerError {
                code: 503,
                message: "max clients".to_string(),
            },
        ];
        for err in &errors {
            assert!(!format!("{err}").is_empty());
        }
    }

    #[test]
    fn test_connection_event_variants() {
        let events = vec![
            ConnectionEvent::Connected {
                hostname: "host".to_string(),
            },
            ConnectionEvent::Disconnected,
            ConnectionEvent::ServerError {
                code: 401,
                message: "unauthorized".to_string(),
            },
        ];
        for event in &events {
            assert!(!format!("{event:?}").is_empty());
        }
    }
}
