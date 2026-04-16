use bincode::{config, decode_from_slice, encode_to_vec};
use serde::{Deserialize, Serialize};

use muxtop_core::system::SystemSnapshot;

use crate::ProtoError;
use crate::frame::{Frame, MSG_ERROR, MSG_HEARTBEAT, MSG_HELLO, MSG_SNAPSHOT, MSG_WELCOME};

/// Wire protocol messages exchanged between muxtop client and server.
///
/// Uses a custom `Debug` impl to redact `auth_token` in `Hello` messages,
/// preventing accidental token leakage in logs or panic messages.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum WireMessage {
    /// Full system snapshot (server → client).
    Snapshot(SystemSnapshot),

    /// Keepalive heartbeat (server → client).
    Heartbeat {
        server_version: String,
        uptime_secs: u64,
    },

    /// Error message (server → client).
    Error { code: u16, message: String },

    /// Client handshake (client → server).
    Hello {
        client_version: String,
        auth_token: Option<String>,
    },

    /// Server handshake response (server → client).
    Welcome {
        server_version: String,
        hostname: String,
        refresh_hz: u32,
    },
}

impl std::fmt::Debug for WireMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WireMessage::Snapshot(s) => f.debug_tuple("Snapshot").field(s).finish(),
            WireMessage::Heartbeat {
                server_version,
                uptime_secs,
            } => f
                .debug_struct("Heartbeat")
                .field("server_version", server_version)
                .field("uptime_secs", uptime_secs)
                .finish(),
            WireMessage::Error { code, message } => f
                .debug_struct("Error")
                .field("code", code)
                .field("message", message)
                .finish(),
            WireMessage::Hello {
                client_version,
                auth_token,
            } => f
                .debug_struct("Hello")
                .field("client_version", client_version)
                .field("auth_token", &auth_token.as_ref().map(|_| "[REDACTED]"))
                .finish(),
            WireMessage::Welcome {
                server_version,
                hostname,
                refresh_hz,
            } => f
                .debug_struct("Welcome")
                .field("server_version", server_version)
                .field("hostname", hostname)
                .field("refresh_hz", refresh_hz)
                .finish(),
        }
    }
}

fn bincode_config() -> impl bincode::config::Config {
    config::standard()
}

impl WireMessage {
    /// Serialize this message into a [`Frame`].
    pub fn to_frame(&self) -> Result<Frame, ProtoError> {
        let (msg_type, payload) = match self {
            WireMessage::Snapshot(snap) => (MSG_SNAPSHOT, encode_to_vec(snap, bincode_config())?),
            WireMessage::Heartbeat {
                server_version,
                uptime_secs,
            } => (
                MSG_HEARTBEAT,
                encode_to_vec((server_version, uptime_secs), bincode_config())?,
            ),
            WireMessage::Error { code, message } => {
                (MSG_ERROR, encode_to_vec((code, message), bincode_config())?)
            }
            WireMessage::Hello {
                client_version,
                auth_token,
            } => (
                MSG_HELLO,
                encode_to_vec((client_version, auth_token), bincode_config())?,
            ),
            WireMessage::Welcome {
                server_version,
                hostname,
                refresh_hz,
            } => (
                MSG_WELCOME,
                encode_to_vec((server_version, hostname, refresh_hz), bincode_config())?,
            ),
        };

        Ok(Frame { msg_type, payload })
    }

    /// Deserialize a [`Frame`] into a `WireMessage`.
    pub fn from_frame(frame: &Frame) -> Result<Self, ProtoError> {
        match frame.msg_type {
            MSG_SNAPSHOT => {
                let (snap, _): (SystemSnapshot, _) =
                    decode_from_slice(&frame.payload, bincode_config())?;
                Ok(WireMessage::Snapshot(snap))
            }
            MSG_HEARTBEAT => {
                let ((server_version, uptime_secs), _): ((String, u64), _) =
                    decode_from_slice(&frame.payload, bincode_config())?;
                Ok(WireMessage::Heartbeat {
                    server_version,
                    uptime_secs,
                })
            }
            MSG_ERROR => {
                let ((code, message), _): ((u16, String), _) =
                    decode_from_slice(&frame.payload, bincode_config())?;
                Ok(WireMessage::Error { code, message })
            }
            MSG_HELLO => {
                let ((client_version, auth_token), _): ((String, Option<String>), _) =
                    decode_from_slice(&frame.payload, bincode_config())?;
                Ok(WireMessage::Hello {
                    client_version,
                    auth_token,
                })
            }
            MSG_WELCOME => {
                let ((server_version, hostname, refresh_hz), _): ((String, String, u32), _) =
                    decode_from_slice(&frame.payload, bincode_config())?;
                Ok(WireMessage::Welcome {
                    server_version,
                    hostname,
                    refresh_hz,
                })
            }
            other => Err(ProtoError::UnknownMessageType(other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxtop_core::network::{NetworkInterfaceSnapshot, NetworkSnapshot};
    use muxtop_core::process::ProcessInfo;
    use muxtop_core::system::{CoreSnapshot, CpuSnapshot, LoadSnapshot, MemorySnapshot};

    fn make_test_snapshot() -> SystemSnapshot {
        SystemSnapshot {
            cpu: CpuSnapshot {
                global_usage: 45.2,
                cores: vec![CoreSnapshot {
                    name: "cpu0".into(),
                    usage: 45.2,
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
                one: 1.5,
                five: 1.2,
                fifteen: 0.8,
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

    #[test]
    fn test_wire_snapshot_roundtrip() {
        let msg = WireMessage::Snapshot(make_test_snapshot());
        let frame = msg.to_frame().unwrap();
        assert_eq!(frame.msg_type, MSG_SNAPSHOT);
        let decoded = WireMessage::from_frame(&frame).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_wire_heartbeat_roundtrip() {
        let msg = WireMessage::Heartbeat {
            server_version: "0.2.0".into(),
            uptime_secs: 86400,
        };
        let frame = msg.to_frame().unwrap();
        assert_eq!(frame.msg_type, MSG_HEARTBEAT);
        let decoded = WireMessage::from_frame(&frame).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_wire_error_roundtrip() {
        let msg = WireMessage::Error {
            code: 503,
            message: "max clients reached".into(),
        };
        let frame = msg.to_frame().unwrap();
        assert_eq!(frame.msg_type, MSG_ERROR);
        let decoded = WireMessage::from_frame(&frame).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_wire_hello_roundtrip() {
        let msg = WireMessage::Hello {
            client_version: "0.2.0".into(),
            auth_token: Some("secret-token".into()),
        };
        let frame = msg.to_frame().unwrap();
        assert_eq!(frame.msg_type, MSG_HELLO);
        let decoded = WireMessage::from_frame(&frame).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_wire_hello_no_token_roundtrip() {
        let msg = WireMessage::Hello {
            client_version: "0.2.0".into(),
            auth_token: None,
        };
        let frame = msg.to_frame().unwrap();
        let decoded = WireMessage::from_frame(&frame).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_wire_welcome_roundtrip() {
        let msg = WireMessage::Welcome {
            server_version: "0.2.0".into(),
            hostname: "prod-server-01".into(),
            refresh_hz: 1,
        };
        let frame = msg.to_frame().unwrap();
        assert_eq!(frame.msg_type, MSG_WELCOME);
        let decoded = WireMessage::from_frame(&frame).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_wire_unknown_message_type() {
        let frame = Frame {
            msg_type: 0xFF,
            payload: vec![1, 2, 3],
        };
        let err = WireMessage::from_frame(&frame).unwrap_err();
        assert!(matches!(err, ProtoError::UnknownMessageType(0xFF)));
    }

    #[test]
    fn test_hello_token_validation() {
        let hello = WireMessage::Hello {
            client_version: "0.2.0".into(),
            auth_token: Some("wrong-token".into()),
        };
        let expected_token = "correct-token";

        // Extract and compare token.
        if let WireMessage::Hello { auth_token, .. } = &hello {
            let valid = auth_token.as_deref().is_some_and(|t| t == expected_token);
            assert!(!valid, "wrong token should not validate");
        }

        let hello_correct = WireMessage::Hello {
            client_version: "0.2.0".into(),
            auth_token: Some("correct-token".into()),
        };
        if let WireMessage::Hello { auth_token, .. } = &hello_correct {
            let valid = auth_token.as_deref().is_some_and(|t| t == expected_token);
            assert!(valid, "correct token should validate");
        }
    }
}
