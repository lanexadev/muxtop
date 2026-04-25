//! Integration tests for muxtop-server.
//!
//! Each test spins up a real TCP server on a random port, connects clients,
//! and exercises the wire protocol end-to-end.
//! Tests cover both plain TCP (legacy helpers) and TLS connections.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ClientConfig, RootCertStore, ServerConfig};
use tokio_util::sync::CancellationToken;

use muxtop_core::network::{NetworkInterfaceSnapshot, NetworkSnapshot};
use muxtop_core::process::ProcessInfo;
use muxtop_core::system::{
    CoreSnapshot, CpuSnapshot, LoadSnapshot, MemorySnapshot, SystemSnapshot,
};
use muxtop_proto::{FrameReader, FrameWriter, WireMessage};

/// Create a minimal test snapshot.
fn make_snapshot() -> SystemSnapshot {
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
        containers: None,
        timestamp_ms: 1_713_200_000_000,
    }
}

/// Helper: start a test server on a random port with a fake collector channel.
/// Returns (server_addr, snapshot_sender, shutdown_token).
async fn start_test_server(
    auth_token: Option<String>,
    max_clients: usize,
) -> (SocketAddr, mpsc::Sender<SystemSnapshot>, CancellationToken) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (collector_tx, collector_rx) = mpsc::channel::<SystemSnapshot>(16);
    let token = CancellationToken::new();

    let config = TestServerConfig {
        auth_token,
        max_clients,
        refresh_hz: 1,
    };

    let server_token = token.clone();
    tokio::spawn(async move {
        run_test_server(listener, config, collector_rx, server_token).await;
    });

    (addr, collector_tx, token)
}

struct TestServerConfig {
    auth_token: Option<String>,
    max_clients: usize,
    refresh_hz: u32,
}

/// Simplified server loop for testing (avoids importing the server module directly,
/// mirrors the real server logic).
async fn run_test_server(
    listener: tokio::net::TcpListener,
    config: TestServerConfig,
    mut collector_rx: mpsc::Receiver<SystemSnapshot>,
    token: CancellationToken,
) {
    use std::sync::Arc;
    use std::time::Instant;
    use tokio::sync::{Semaphore, broadcast};

    let (broadcast_tx, _) = broadcast::channel::<SystemSnapshot>(16);
    let semaphore = Arc::new(Semaphore::new(config.max_clients));
    let auth_token = config.auth_token.clone();
    let refresh_hz = config.refresh_hz;
    let start_time = Instant::now();

    // Relay task.
    let relay_tx = broadcast_tx.clone();
    let relay_token = token.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                snapshot = collector_rx.recv() => {
                    match snapshot {
                        Some(snap) => { let _ = relay_tx.send(snap); }
                        None => break,
                    }
                }
                _ = relay_token.cancelled() => break,
            }
        }
    });

    // Accept loop. Mirrors the production server's MED-S2 ordering:
    // 1) acquire the max_clients permit BEFORE starting any per-client
    //    work, 2) drop the TCP stream silently if the bucket is full.
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, peer) = match result {
                    Ok(v) => v,
                    Err(_) => break,
                };

                let permit = match Arc::clone(&semaphore).try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => {
                        // Drop the stream — production behaviour.
                        drop(stream);
                        continue;
                    }
                };

                let auth_token = auth_token.clone();
                let snapshot_rx = broadcast_tx.subscribe();
                let client_token = token.clone();

                tokio::spawn(async move {
                    let _ = handle_test_client(
                        stream, peer, permit, auth_token, refresh_hz,
                        start_time, snapshot_rx, client_token,
                    ).await;
                });
            }
            _ = token.cancelled() => break,
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_test_client(
    stream: TcpStream,
    _peer: SocketAddr,
    _permit: tokio::sync::OwnedSemaphorePermit,
    auth_token: Option<String>,
    refresh_hz: u32,
    start_time: std::time::Instant,
    mut snapshot_rx: broadcast::Receiver<SystemSnapshot>,
    token: CancellationToken,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (reader, writer) = stream.into_split();
    let mut fr = FrameReader::new(reader);
    let mut fw = FrameWriter::new(writer);

    // Handshake.
    let frame = tokio::time::timeout(Duration::from_secs(5), fr.read_frame())
        .await??
        .ok_or("no Hello received")?;
    let hello = WireMessage::from_frame(&frame)?;

    if let WireMessage::Hello {
        auth_token: client_token,
        ..
    } = &hello
        && let Some(expected) = &auth_token
    {
        let provided = client_token.as_deref().unwrap_or("");
        if provided != expected.as_str() {
            let err = WireMessage::Error {
                code: 401,
                message: "unauthorized".into(),
            };
            let _ = fw.write_frame(&err.to_frame()?).await;
            return Ok(());
        }
    }

    let welcome = WireMessage::Welcome {
        server_version: env!("CARGO_PKG_VERSION").to_string(),
        hostname: "test-host".into(),
        refresh_hz,
    };
    fw.write_frame(&welcome.to_frame()?).await?;

    // Stream.
    let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
    heartbeat.tick().await;

    loop {
        tokio::select! {
            result = snapshot_rx.recv() => {
                match result {
                    Ok(snap) => {
                        let msg = WireMessage::Snapshot(snap);
                        fw.write_frame(&msg.to_frame()?).await?;
                        heartbeat.reset();
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = heartbeat.tick() => {
                let msg = WireMessage::Heartbeat {
                    server_version: env!("CARGO_PKG_VERSION").to_string(),
                    uptime_secs: start_time.elapsed().as_secs(),
                };
                fw.write_frame(&msg.to_frame()?).await?;
            }
            _ = token.cancelled() => break,
        }
    }

    Ok(())
}

/// Helper: connect to server and perform handshake.
async fn connect_and_handshake(
    addr: SocketAddr,
    token: Option<&str>,
) -> (
    FrameReader<tokio::net::tcp::OwnedReadHalf>,
    FrameWriter<tokio::net::tcp::OwnedWriteHalf>,
    WireMessage,
) {
    let stream = TcpStream::connect(addr).await.unwrap();
    let (reader, writer) = stream.into_split();
    let mut fr = FrameReader::new(reader);
    let mut fw = FrameWriter::new(writer);

    let hello = WireMessage::Hello {
        client_version: "test".into(),
        auth_token: token.map(String::from),
    };
    fw.write_frame(&hello.to_frame().unwrap()).await.unwrap();

    let frame = fr.read_frame().await.unwrap().unwrap();
    let response = WireMessage::from_frame(&frame).unwrap();

    (fr, fw, response)
}

// ── AC-03 + AC-04: Connect and handshake without auth ──

#[tokio::test]
async fn test_handshake_no_auth() {
    let (addr, _tx, token) = start_test_server(None, 8).await;

    let (_fr, _fw, response) = connect_and_handshake(addr, None).await;

    match response {
        WireMessage::Welcome {
            hostname,
            refresh_hz,
            ..
        } => {
            assert_eq!(hostname, "test-host");
            assert_eq!(refresh_hz, 1);
        }
        other => panic!("expected Welcome, got {other:?}"),
    }

    token.cancel();
}

// ── AC-05: Client receives snapshots ──

#[tokio::test]
async fn test_client_receives_snapshots() {
    let (addr, snap_tx, token) = start_test_server(None, 8).await;

    let (mut fr, _fw, _welcome) = connect_and_handshake(addr, None).await;

    // Send a snapshot through the collector channel.
    snap_tx.send(make_snapshot()).await.unwrap();

    // Client should receive it.
    let frame = tokio::time::timeout(Duration::from_secs(3), fr.read_frame())
        .await
        .expect("timeout waiting for snapshot")
        .unwrap()
        .unwrap();

    let msg = WireMessage::from_frame(&frame).unwrap();
    match msg {
        WireMessage::Snapshot(snap) => {
            assert!(!snap.processes.is_empty());
            assert!(snap.cpu.global_usage > 0.0);
        }
        other => panic!("expected Snapshot, got {other:?}"),
    }

    token.cancel();
}

// ── AC-07: Max clients rejection ──
//
// MED-S2: post-fix, an over-quota TCP stream is dropped immediately (no TLS
// handshake, no `Error { 503 }` payload). The client observes either a
// connection reset or a clean EOF when it tries to read the Welcome.

#[tokio::test]
async fn test_max_clients_rejection() {
    let (addr, _tx, token) = start_test_server(None, 1).await;

    // First client — should succeed.
    let (_fr1, _fw1, response1) = connect_and_handshake(addr, None).await;
    assert!(matches!(response1, WireMessage::Welcome { .. }));

    // Second client — TCP stream is closed immediately by the server
    // because the semaphore is exhausted. We connect, send Hello (which may
    // succeed or fail depending on close timing), and assert that the read
    // returns either an EOF or an error.
    let stream = TcpStream::connect(addr).await.unwrap();
    let (reader, writer) = stream.into_split();
    let mut fr = FrameReader::new(reader);
    let mut fw = FrameWriter::new(writer);

    let hello = WireMessage::Hello {
        client_version: "test".into(),
        auth_token: None,
    };
    // The write may succeed (TCP buffer absorbs it) or fail (server already
    // closed). Either way is fine — we care about the read.
    let _ = fw.write_frame(&hello.to_frame().unwrap()).await;

    let read_result = tokio::time::timeout(Duration::from_secs(3), fr.read_frame()).await;
    match read_result {
        Ok(Ok(None)) => {} // EOF — server closed cleanly
        Ok(Err(_)) => {}   // I/O error — server reset
        Ok(Ok(Some(frame))) => {
            // If we DID get a frame, it must NOT be a Welcome (we shouldn't be admitted).
            let msg = WireMessage::from_frame(&frame).unwrap();
            assert!(
                !matches!(msg, WireMessage::Welcome { .. }),
                "second client must not be admitted, got {msg:?}"
            );
        }
        Err(_) => panic!("timeout waiting for server to close over-quota client"),
    }

    token.cancel();
}

// ── AC-08: Token authentication ──

#[tokio::test]
async fn test_auth_valid_token() {
    let (addr, _tx, token) = start_test_server(Some("secret123".into()), 8).await;

    let (_fr, _fw, response) = connect_and_handshake(addr, Some("secret123")).await;
    assert!(matches!(response, WireMessage::Welcome { .. }));

    token.cancel();
}

#[tokio::test]
async fn test_auth_invalid_token() {
    let (addr, _tx, token) = start_test_server(Some("secret123".into()), 8).await;

    let (_fr, _fw, response) = connect_and_handshake(addr, Some("wrong")).await;
    match response {
        WireMessage::Error { code, .. } => {
            assert_eq!(code, 401);
        }
        other => panic!("expected Error 401, got {other:?}"),
    }

    token.cancel();
}

#[tokio::test]
async fn test_auth_missing_token() {
    let (addr, _tx, token) = start_test_server(Some("secret123".into()), 8).await;

    let (_fr, _fw, response) = connect_and_handshake(addr, None).await;
    match response {
        WireMessage::Error { code, .. } => {
            assert_eq!(code, 401);
        }
        other => panic!("expected Error 401, got {other:?}"),
    }

    token.cancel();
}

// ── AC-11: Broadcast to multiple clients ──

#[tokio::test]
async fn test_broadcast_to_multiple_clients() {
    let (addr, snap_tx, token) = start_test_server(None, 8).await;

    let (mut fr1, _fw1, _) = connect_and_handshake(addr, None).await;
    let (mut fr2, _fw2, _) = connect_and_handshake(addr, None).await;

    // Send a snapshot.
    snap_tx.send(make_snapshot()).await.unwrap();

    // Both clients should receive it.
    let frame1 = tokio::time::timeout(Duration::from_secs(3), fr1.read_frame())
        .await
        .expect("timeout client 1")
        .unwrap()
        .unwrap();
    let frame2 = tokio::time::timeout(Duration::from_secs(3), fr2.read_frame())
        .await
        .expect("timeout client 2")
        .unwrap()
        .unwrap();

    assert!(matches!(
        WireMessage::from_frame(&frame1).unwrap(),
        WireMessage::Snapshot(_)
    ));
    assert!(matches!(
        WireMessage::from_frame(&frame2).unwrap(),
        WireMessage::Snapshot(_)
    ));

    token.cancel();
}

// ── AC-05b: Client receives multiple snapshots ──

#[tokio::test]
async fn test_client_receives_multiple_snapshots() {
    let (addr, snap_tx, token) = start_test_server(None, 8).await;

    let (mut fr, _fw, _welcome) = connect_and_handshake(addr, None).await;

    // Send 3 snapshots through the collector channel.
    for _ in 0..3 {
        snap_tx.send(make_snapshot()).await.unwrap();
    }

    // Client should receive all 3.
    let mut received = 0;
    for _ in 0..3 {
        let frame = tokio::time::timeout(Duration::from_secs(3), fr.read_frame())
            .await
            .expect("timeout waiting for snapshot")
            .unwrap()
            .unwrap();

        let msg = WireMessage::from_frame(&frame).unwrap();
        assert!(matches!(msg, WireMessage::Snapshot(_)));
        received += 1;
    }

    assert_eq!(received, 3, "should have received 3 snapshots");

    token.cancel();
}

// ── AC-06: Snapshot content verification ──

#[tokio::test]
async fn test_snapshot_content_complete() {
    let (addr, snap_tx, token) = start_test_server(None, 8).await;

    let (mut fr, _fw, _welcome) = connect_and_handshake(addr, None).await;

    snap_tx.send(make_snapshot()).await.unwrap();

    let frame = tokio::time::timeout(Duration::from_secs(3), fr.read_frame())
        .await
        .expect("timeout")
        .unwrap()
        .unwrap();

    let msg = WireMessage::from_frame(&frame).unwrap();
    match msg {
        WireMessage::Snapshot(snap) => {
            // CPU
            assert!(snap.cpu.global_usage >= 0.0);
            assert!(!snap.cpu.cores.is_empty(), "should have CPU cores");

            // Memory
            assert!(snap.memory.total > 0, "should have total memory");

            // Processes
            assert!(!snap.processes.is_empty(), "should have processes");
            let proc = &snap.processes[0];
            assert!(proc.pid > 0);
            assert!(!proc.name.is_empty());

            // Networks
            assert!(
                !snap.networks.interfaces.is_empty(),
                "should have network interfaces"
            );
            let iface = &snap.networks.interfaces[0];
            assert!(!iface.name.is_empty());

            // Timestamp
            assert!(snap.timestamp_ms > 0, "should have timestamp");
        }
        other => panic!("expected Snapshot, got {other:?}"),
    }

    token.cancel();
}

// ── AC-10: Graceful shutdown ──

#[tokio::test]
async fn test_graceful_shutdown() {
    let (addr, _tx, token) = start_test_server(None, 8).await;

    let (mut fr, _fw, _) = connect_and_handshake(addr, None).await;

    // Cancel the server.
    token.cancel();

    // Client should get EOF (None) or an error eventually.
    let result = tokio::time::timeout(Duration::from_secs(3), fr.read_frame()).await;
    match result {
        Ok(Ok(None)) => {}    // Clean EOF — expected.
        Ok(Ok(Some(_))) => {} // Got a frame (e.g., heartbeat) before shutdown completed — fine.
        Ok(Err(_)) => {}      // I/O error from shutdown — fine.
        Err(_) => panic!("timeout: client did not observe shutdown"),
    }
}

// ═══════════════════════════════════════════════════════════════════
// TLS Integration Tests
// ═══════════════════════════════════════════════════════════════════

/// Shared TLS test material: cert PEM, key PEM, TlsAcceptor, TlsConnector.
struct TlsTestMaterial {
    cert_pem: String,
    _key_pem: String,
    acceptor: TlsAcceptor,
}

fn make_tls_test_material() -> TlsTestMaterial {
    let ck = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let cert_pem = ck.cert.pem();
    let key_pem = ck.signing_key.serialize_pem();

    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).unwrap();

    let server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();

    TlsTestMaterial {
        cert_pem,
        _key_pem: key_pem,
        acceptor: TlsAcceptor::from(Arc::new(server_config)),
    }
}

/// Build a TLS connector that trusts the test cert.
fn tls_connector_trusting(cert_pem: &str) -> tokio_rustls::TlsConnector {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let mut root_store = RootCertStore::empty();
    for cert in certs {
        root_store.add(cert).unwrap();
    }

    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    tokio_rustls::TlsConnector::from(Arc::new(config))
}

/// Start a TLS test server.
async fn start_tls_test_server(
    tls: &TlsTestMaterial,
    auth_token: Option<String>,
    max_clients: usize,
) -> (SocketAddr, mpsc::Sender<SystemSnapshot>, CancellationToken) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (collector_tx, collector_rx) = mpsc::channel::<SystemSnapshot>(16);
    let token = CancellationToken::new();
    let acceptor = tls.acceptor.clone();

    let config = TestServerConfig {
        auth_token,
        max_clients,
        refresh_hz: 1,
    };

    let server_token = token.clone();
    tokio::spawn(async move {
        run_tls_test_server(listener, acceptor, config, collector_rx, server_token).await;
    });

    (addr, collector_tx, token)
}

/// TLS server loop for testing.
async fn run_tls_test_server(
    listener: tokio::net::TcpListener,
    acceptor: TlsAcceptor,
    config: TestServerConfig,
    mut collector_rx: mpsc::Receiver<SystemSnapshot>,
    token: CancellationToken,
) {
    use std::time::Instant;
    use tokio::sync::Semaphore;

    let (broadcast_tx, _) = broadcast::channel::<SystemSnapshot>(16);
    let semaphore = Arc::new(Semaphore::new(config.max_clients));
    let auth_token = config.auth_token.clone();
    let refresh_hz = config.refresh_hz;
    let start_time = Instant::now();

    let relay_tx = broadcast_tx.clone();
    let relay_token = token.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                snapshot = collector_rx.recv() => {
                    match snapshot {
                        Some(snap) => { let _ = relay_tx.send(snap); }
                        None => break,
                    }
                }
                _ = relay_token.cancelled() => break,
            }
        }
    });

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (tcp_stream, peer) = match result {
                    Ok(v) => v,
                    Err(_) => break,
                };

                // MED-S2: acquire the permit BEFORE the TLS handshake.
                let permit = match Arc::clone(&semaphore).try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => {
                        drop(tcp_stream);
                        continue;
                    }
                };

                let acceptor = acceptor.clone();
                let auth_token = auth_token.clone();
                let snapshot_rx = broadcast_tx.subscribe();
                let client_token = token.clone();

                tokio::spawn(async move {
                    let tls_stream = match acceptor.accept(tcp_stream).await {
                        Ok(s) => s,
                        Err(_) => return,
                    };
                    let (reader, writer) = tokio::io::split(tls_stream);
                    let _ = handle_tls_test_client(
                        reader, writer, peer, permit, auth_token, refresh_hz,
                        start_time, snapshot_rx, client_token,
                    ).await;
                });
            }
            _ = token.cancelled() => break,
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_tls_test_client<R, W>(
    reader: R,
    writer: W,
    _peer: SocketAddr,
    _permit: tokio::sync::OwnedSemaphorePermit,
    auth_token: Option<String>,
    refresh_hz: u32,
    start_time: std::time::Instant,
    mut snapshot_rx: broadcast::Receiver<SystemSnapshot>,
    token: CancellationToken,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut fr = FrameReader::new(reader);
    let mut fw = FrameWriter::new(writer);

    let frame = tokio::time::timeout(Duration::from_secs(5), fr.read_frame())
        .await??
        .ok_or("no Hello received")?;
    let hello = WireMessage::from_frame(&frame)?;

    if let WireMessage::Hello {
        auth_token: client_token,
        ..
    } = &hello
        && let Some(expected) = &auth_token
    {
        let provided = client_token.as_deref().unwrap_or("");
        if provided != expected.as_str() {
            let err = WireMessage::Error {
                code: 401,
                message: "unauthorized".into(),
            };
            let _ = fw.write_frame(&err.to_frame()?).await;
            return Ok(());
        }
    }

    let welcome = WireMessage::Welcome {
        server_version: env!("CARGO_PKG_VERSION").to_string(),
        hostname: "test-host".into(),
        refresh_hz,
    };
    fw.write_frame(&welcome.to_frame()?).await?;

    let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
    heartbeat.tick().await;

    loop {
        tokio::select! {
            result = snapshot_rx.recv() => {
                match result {
                    Ok(snap) => {
                        let msg = WireMessage::Snapshot(snap);
                        fw.write_frame(&msg.to_frame()?).await?;
                        heartbeat.reset();
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = heartbeat.tick() => {
                let msg = WireMessage::Heartbeat {
                    server_version: env!("CARGO_PKG_VERSION").to_string(),
                    uptime_secs: start_time.elapsed().as_secs(),
                };
                fw.write_frame(&msg.to_frame()?).await?;
            }
            _ = token.cancelled() => break,
        }
    }

    Ok(())
}

/// Helper: TLS connect to server and perform handshake.
async fn tls_connect_and_handshake(
    addr: SocketAddr,
    connector: &tokio_rustls::TlsConnector,
    auth_token: Option<&str>,
) -> Result<
    (
        FrameReader<tokio::io::ReadHalf<tokio_rustls::client::TlsStream<TcpStream>>>,
        FrameWriter<tokio::io::WriteHalf<tokio_rustls::client::TlsStream<TcpStream>>>,
        WireMessage,
    ),
    Box<dyn std::error::Error>,
> {
    let tcp = TcpStream::connect(addr).await?;
    let server_name = rustls_pki_types::ServerName::try_from("localhost").unwrap();
    let tls_stream = connector.connect(server_name, tcp).await?;
    let (reader, writer) = tokio::io::split(tls_stream);
    let mut fr = FrameReader::new(reader);
    let mut fw = FrameWriter::new(writer);

    let hello = WireMessage::Hello {
        client_version: "test".into(),
        auth_token: auth_token.map(String::from),
    };
    fw.write_frame(&hello.to_frame()?).await?;

    let frame = fr.read_frame().await?.unwrap();
    let response = WireMessage::from_frame(&frame)?;

    Ok((fr, fw, response))
}

// ── TLS AC-01: Server accepts TLS connections ──

#[tokio::test]
async fn test_tls_handshake() {
    let tls = make_tls_test_material();
    let (addr, _tx, token) = start_tls_test_server(&tls, Some("test-token".into()), 8).await;

    let connector = tls_connector_trusting(&tls.cert_pem);
    let (_fr, _fw, response) = tls_connect_and_handshake(addr, &connector, Some("test-token"))
        .await
        .unwrap();

    match response {
        WireMessage::Welcome { hostname, .. } => {
            assert_eq!(hostname, "test-host");
        }
        other => panic!("expected Welcome, got {other:?}"),
    }

    token.cancel();
}

// ── TLS AC-02: Client receives snapshots over TLS ──

#[tokio::test]
async fn test_tls_client_receives_snapshot() {
    let tls = make_tls_test_material();
    let (addr, snap_tx, token) = start_tls_test_server(&tls, None, 8).await;

    let connector = tls_connector_trusting(&tls.cert_pem);
    let (mut fr, _fw, _welcome) = tls_connect_and_handshake(addr, &connector, None)
        .await
        .unwrap();

    snap_tx.send(make_snapshot()).await.unwrap();

    let frame = tokio::time::timeout(Duration::from_secs(3), fr.read_frame())
        .await
        .expect("timeout")
        .unwrap()
        .unwrap();

    let msg = WireMessage::from_frame(&frame).unwrap();
    assert!(matches!(msg, WireMessage::Snapshot(_)));

    token.cancel();
}

// ── TLS AC-06: Rejects unknown certs ──

#[tokio::test]
async fn test_tls_rejects_unknown_cert() {
    let tls = make_tls_test_material();
    let (addr, _tx, token) = start_tls_test_server(&tls, None, 8).await;

    // Use a connector with empty root store (doesn't trust the self-signed cert).
    let config = ClientConfig::builder()
        .with_root_certificates(RootCertStore::empty())
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));

    let result = tls_connect_and_handshake(addr, &connector, None).await;
    assert!(result.is_err(), "should fail with untrusted cert");

    token.cancel();
}

// ── TLS AC-07: --tls-skip-verify bypasses cert check ──

#[tokio::test]
async fn test_tls_skip_verify_connects() {
    let tls = make_tls_test_material();
    let (addr, _tx, token) = start_tls_test_server(&tls, None, 8).await;

    // Use insecure connector (skip verify).
    let connector = muxtop_proto::tls::connector_insecure();

    let (_fr, _fw, response) = tls_connect_and_handshake(addr, &connector, None)
        .await
        .unwrap();

    assert!(matches!(response, WireMessage::Welcome { .. }));

    token.cancel();
}

// ── TLS AC-08: Full streaming over TLS ──

#[tokio::test]
async fn test_tls_full_streaming() {
    let tls = make_tls_test_material();
    let (addr, snap_tx, token) = start_tls_test_server(&tls, Some("auth-token".into()), 8).await;

    let connector = tls_connector_trusting(&tls.cert_pem);
    let (mut fr, _fw, response) = tls_connect_and_handshake(addr, &connector, Some("auth-token"))
        .await
        .unwrap();
    assert!(matches!(response, WireMessage::Welcome { .. }));

    // Send 3 snapshots.
    for _ in 0..3 {
        snap_tx.send(make_snapshot()).await.unwrap();
    }

    // Receive all 3.
    let mut received = 0;
    for _ in 0..3 {
        let frame = tokio::time::timeout(Duration::from_secs(3), fr.read_frame())
            .await
            .expect("timeout")
            .unwrap()
            .unwrap();
        let msg = WireMessage::from_frame(&frame).unwrap();
        assert!(matches!(msg, WireMessage::Snapshot(_)));
        received += 1;
    }
    assert_eq!(received, 3);

    token.cancel();
}

// ── MED-S1: pre-auth bincode allocation cap ──
//
// The server must reject Hello frames whose payload exceeds the 4 KiB
// pre-auth cap (per ADR-30-2). We send a hand-crafted frame whose
// length-prefix claims a 5 KiB payload. The server must drop the
// connection BEFORE allocating the payload buffer.
//
// We exercise this against the *test* server's plain-TCP path. The
// production server uses the same `read_frame_with_max_payload(4096)`
// internally — see `crates/muxtop-server/src/client.rs::handle`.

/// Mirror of the production server's pre-auth Hello path: read with a
/// 4 KiB cap and return whatever the proto layer says. Used by the
/// MED-S1 integration test to prove that the cap is enforced.
async fn handle_capped_hello_test_client(
    stream: TcpStream,
) -> Result<WireMessage, Box<dyn std::error::Error + Send + Sync>> {
    let (reader, writer) = stream.into_split();
    let mut fr = FrameReader::new(reader);
    let mut _fw = FrameWriter::new(writer);

    let frame = tokio::time::timeout(Duration::from_secs(5), fr.read_frame_with_max_payload(4096))
        .await??
        .ok_or("no Hello received")?;
    Ok(WireMessage::from_frame(&frame)?)
}

#[tokio::test]
async fn test_pre_auth_hello_cap_rejects_oversize_frame() {
    use tokio::io::AsyncWriteExt;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Server task: accept one stream and read with the 4 KiB pre-auth cap.
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        handle_capped_hello_test_client(stream).await
    });

    // Client task: send a hand-crafted frame whose length-prefix claims
    // 5 KiB of payload (5121 = 5120 payload + 1 type byte). We don't
    // even bother sending the payload — the server should reject on the
    // header alone.
    let mut tcp = TcpStream::connect(addr).await.unwrap();
    let payload_len: u32 = 5 * 1024;
    let content_len: u32 = payload_len + 1;
    tcp.write_all(&content_len.to_be_bytes()).await.unwrap();
    tcp.flush().await.unwrap();
    // Hold the connection open briefly so the server gets a chance to
    // read the header and reject.
    drop(tcp);

    // Server must surface a FrameTooLarge error (proto layer).
    let result = server.await.unwrap();
    let err = result.expect_err("oversize Hello must be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("frame too large") || msg.contains("FrameTooLarge"),
        "expected FrameTooLarge error, got: {msg}"
    );
}

// ── TLS AC-08b: Auth rejection over TLS ──

#[tokio::test]
async fn test_tls_auth_rejection() {
    let tls = make_tls_test_material();
    let (addr, _tx, token) = start_tls_test_server(&tls, Some("correct-token".into()), 8).await;

    let connector = tls_connector_trusting(&tls.cert_pem);
    let (_fr, _fw, response) = tls_connect_and_handshake(addr, &connector, Some("wrong-token"))
        .await
        .unwrap();

    match response {
        WireMessage::Error { code, .. } => assert_eq!(code, 401),
        other => panic!("expected Error 401, got {other:?}"),
    }

    token.cancel();
}
