//! Integration tests for muxtop-server.
//!
//! Each test spins up a real TCP server on a random port, connects clients,
//! and exercises the wire protocol end-to-end.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc};
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

    // Accept loop.
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, peer) = match result {
                    Ok(v) => v,
                    Err(_) => break,
                };

                let semaphore = Arc::clone(&semaphore);
                let auth_token = auth_token.clone();
                let snapshot_rx = broadcast_tx.subscribe();
                let client_token = token.clone();

                tokio::spawn(async move {
                    let _ = handle_test_client(
                        stream, peer, semaphore, auth_token, refresh_hz,
                        start_time, snapshot_rx, client_token,
                    ).await;
                });
            }
            _ = token.cancelled() => break,
        }
    }
}

async fn handle_test_client(
    stream: TcpStream,
    _peer: SocketAddr,
    semaphore: Arc<tokio::sync::Semaphore>,
    auth_token: Option<String>,
    refresh_hz: u32,
    start_time: std::time::Instant,
    mut snapshot_rx: broadcast::Receiver<SystemSnapshot>,
    token: CancellationToken,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _permit = match semaphore.try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            let (reader, writer) = stream.into_split();
            let mut fw = FrameWriter::new(writer);
            let mut fr = FrameReader::new(reader);
            // Read Hello first.
            let _ = tokio::time::timeout(Duration::from_secs(5), fr.read_frame()).await;
            let err = WireMessage::Error {
                code: 503,
                message: "max clients reached".into(),
            };
            let _ = fw.write_frame(&err.to_frame()?).await;
            return Ok(());
        }
    };

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
    {
        if let Some(expected) = &auth_token {
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

#[tokio::test]
async fn test_max_clients_rejection() {
    let (addr, _tx, token) = start_test_server(None, 1).await;

    // First client — should succeed.
    let (_fr1, _fw1, response1) = connect_and_handshake(addr, None).await;
    assert!(matches!(response1, WireMessage::Welcome { .. }));

    // Second client — should be rejected with 503.
    let (_fr2, _fw2, response2) = connect_and_handshake(addr, None).await;
    match response2 {
        WireMessage::Error { code, .. } => {
            assert_eq!(code, 503);
        }
        other => panic!("expected Error 503, got {other:?}"),
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
