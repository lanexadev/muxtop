use tokio::net::{TcpListener, TcpStream};

use muxtop_core::network::{NetworkInterfaceSnapshot, NetworkSnapshot};
use muxtop_core::process::ProcessInfo;
use muxtop_core::system::{
    CoreSnapshot, CpuSnapshot, LoadSnapshot, MemorySnapshot, SystemSnapshot,
};
use muxtop_proto::{FrameReader, FrameWriter, WireMessage};

fn make_test_snapshot() -> SystemSnapshot {
    SystemSnapshot {
        cpu: CpuSnapshot {
            global_usage: 50.0,
            cores: vec![CoreSnapshot {
                name: "cpu0".into(),
                usage: 50.0,
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
            fifteen: 0.6,
            uptime_secs: 7200,
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
                bytes_rx: 5000,
                bytes_tx: 5000,
                packets_rx: 50,
                packets_tx: 50,
                errors_rx: 0,
                errors_tx: 0,
                mac_address: "00:00:00:00:00:00".into(),
                is_up: true,
            }],
            total_rx: 5000,
            total_tx: 5000,
        },
        timestamp_ms: 1_713_200_000_000,
    }
}

/// Test the full Hello → Welcome → Snapshot → Heartbeat sequence over TCP.
#[tokio::test]
async fn test_tcp_roundtrip_sequence() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (reader, writer) = stream.into_split();
        let mut reader = FrameReader::new(reader);
        let mut writer = FrameWriter::new(writer);

        // Server receives Hello.
        let hello = reader.read_frame().await.unwrap().unwrap();
        let msg = WireMessage::from_frame(&hello).unwrap();
        assert!(matches!(msg, WireMessage::Hello { .. }));

        // Server sends Welcome.
        let welcome = WireMessage::Welcome {
            server_version: "0.2.0".into(),
            hostname: "test-host".into(),
            refresh_hz: 1,
        };
        writer
            .write_frame(&welcome.to_frame().unwrap())
            .await
            .unwrap();

        // Server sends Snapshot.
        let snap_msg = WireMessage::Snapshot(make_test_snapshot());
        writer
            .write_frame(&snap_msg.to_frame().unwrap())
            .await
            .unwrap();

        // Server sends Heartbeat.
        let hb = WireMessage::Heartbeat {
            server_version: "0.2.0".into(),
            uptime_secs: 3600,
        };
        writer.write_frame(&hb.to_frame().unwrap()).await.unwrap();

        // Drop writer to signal EOF.
        drop(writer);
    });

    // Client side.
    let stream = TcpStream::connect(addr).await.unwrap();
    let (reader, writer) = stream.into_split();
    let mut reader = FrameReader::new(reader);
    let mut writer = FrameWriter::new(writer);

    // Client sends Hello.
    let hello = WireMessage::Hello {
        client_version: "0.2.0".into(),
        auth_token: Some("test-token".into()),
    };
    writer
        .write_frame(&hello.to_frame().unwrap())
        .await
        .unwrap();

    // Client receives Welcome.
    let frame = reader.read_frame().await.unwrap().unwrap();
    let welcome = WireMessage::from_frame(&frame).unwrap();
    match &welcome {
        WireMessage::Welcome {
            server_version,
            hostname,
            refresh_hz,
        } => {
            assert_eq!(server_version, "0.2.0");
            assert_eq!(hostname, "test-host");
            assert_eq!(*refresh_hz, 1);
        }
        other => panic!("expected Welcome, got {other:?}"),
    }

    // Client receives Snapshot.
    let frame = reader.read_frame().await.unwrap().unwrap();
    let snap = WireMessage::from_frame(&frame).unwrap();
    match &snap {
        WireMessage::Snapshot(s) => {
            assert_eq!(s.cpu.cores.len(), 1);
            assert_eq!(s.processes.len(), 1);
            assert!(!s.networks.interfaces.is_empty());
        }
        other => panic!("expected Snapshot, got {other:?}"),
    }

    // Client receives Heartbeat.
    let frame = reader.read_frame().await.unwrap().unwrap();
    let hb = WireMessage::from_frame(&frame).unwrap();
    assert!(matches!(hb, WireMessage::Heartbeat { .. }));

    // EOF: server closed.
    let eof = reader.read_frame().await.unwrap();
    assert!(eof.is_none());

    server.await.unwrap();
}

/// Test that a clean disconnect returns None from read_frame.
#[tokio::test]
async fn test_tcp_clean_disconnect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        // Immediately drop — clean close.
        drop(stream);
    });

    let stream = TcpStream::connect(addr).await.unwrap();
    let mut reader = FrameReader::new(stream);

    let result = reader.read_frame().await.unwrap();
    assert!(result.is_none(), "expected None on clean disconnect");

    server.await.unwrap();
}

/// Test token validation logic.
#[tokio::test]
async fn test_hello_token_validation() {
    let expected_token = "secret-123";

    // Valid token.
    let hello_valid = WireMessage::Hello {
        client_version: "0.2.0".into(),
        auth_token: Some("secret-123".into()),
    };

    // Invalid token.
    let hello_invalid = WireMessage::Hello {
        client_version: "0.2.0".into(),
        auth_token: Some("wrong-token".into()),
    };

    // No token.
    let hello_none = WireMessage::Hello {
        client_version: "0.2.0".into(),
        auth_token: None,
    };

    // Round-trip through frame to verify wire integrity.
    for hello in [&hello_valid, &hello_invalid, &hello_none] {
        let frame = hello.to_frame().unwrap();
        let decoded = WireMessage::from_frame(&frame).unwrap();
        assert_eq!(hello, &decoded);
    }

    // Validate tokens.
    let validate = |msg: &WireMessage| -> bool {
        if let WireMessage::Hello { auth_token, .. } = msg {
            auth_token.as_deref().is_some_and(|t| t == expected_token)
        } else {
            false
        }
    };

    assert!(validate(&hello_valid));
    assert!(!validate(&hello_invalid));
    assert!(!validate(&hello_none));
}
