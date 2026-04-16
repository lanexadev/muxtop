use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use muxtop_core::system::SystemSnapshot;

use crate::client;

/// Server configuration derived from CLI arguments.
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub max_clients: usize,
    pub auth_token: Option<String>,
    pub refresh_hz: u32,
}

/// Shared state accessible by all client tasks.
pub struct SharedState {
    pub auth_token: Option<String>,
    pub refresh_hz: u32,
    pub hostname: String,
    pub server_version: String,
    pub start_time: Instant,
    pub semaphore: Arc<Semaphore>,
}

/// Run the TCP server: relay collector snapshots to broadcast, accept clients.
pub async fn run(
    config: ServerConfig,
    mut collector_rx: mpsc::Receiver<SystemSnapshot>,
    token: CancellationToken,
) -> Result<()> {
    let listener = TcpListener::bind(config.bind).await?;
    tracing::info!(addr = %config.bind, "listening");

    // Create the broadcast channel for fan-out to clients.
    let (broadcast_tx, _) = broadcast::channel::<SystemSnapshot>(16);

    // Shared state for all client tasks.
    let state = Arc::new(SharedState {
        auth_token: config.auth_token,
        refresh_hz: config.refresh_hz,
        hostname: hostname(),
        server_version: env!("CARGO_PKG_VERSION").to_string(),
        start_time: Instant::now(),
        semaphore: Arc::new(Semaphore::new(config.max_clients)),
    });

    // G-20: Relay task — bridges mpsc (collector) to broadcast (clients).
    let relay_tx = broadcast_tx.clone();
    let relay_token = token.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                snapshot = collector_rx.recv() => {
                    match snapshot {
                        Some(snap) => {
                            let client_count = relay_tx.receiver_count();
                            if client_count > 0 {
                                let _ = relay_tx.send(snap);
                                tracing::debug!(clients = client_count, "broadcast snapshot");
                            }
                        }
                        None => {
                            tracing::debug!("collector channel closed");
                            break;
                        }
                    }
                }
                _ = relay_token.cancelled() => {
                    tracing::debug!("relay task shutting down");
                    break;
                }
            }
        }
    });

    // Accept loop.
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, peer) = result?;
                tracing::info!(peer = %peer, "client connected");

                let state = Arc::clone(&state);
                let snapshot_rx = broadcast_tx.subscribe();
                let client_token = token.clone();

                tokio::spawn(async move {
                    if let Err(e) = client::handle(stream, peer, state, snapshot_rx, client_token).await {
                        tracing::debug!(peer = %peer, error = %e, "client session ended");
                    }
                    tracing::info!(peer = %peer, "client disconnected");
                });
            }
            _ = token.cancelled() => {
                tracing::info!("server shutting down, stopping accept loop");
                break;
            }
        }
    }

    Ok(())
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()))
        .unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hostname_returns_string() {
        let h = hostname();
        assert!(!h.is_empty());
    }

    #[tokio::test]
    async fn test_server_binds_to_random_port() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        assert_ne!(addr.port(), 0);
    }
}
