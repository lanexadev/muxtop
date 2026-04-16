use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, broadcast, mpsc};
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;

/// Timeout for TLS handshake with connecting clients.
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

use muxtop_core::system::SystemSnapshot;

use crate::client;

/// Server configuration derived from CLI arguments.
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub max_clients: usize,
    pub auth_token: String,
    pub refresh_hz: u32,
    pub tls_acceptor: TlsAcceptor,
}

/// Shared state accessible by all client tasks.
pub struct SharedState {
    pub auth_token: String,
    pub refresh_hz: u32,
    pub hostname: String,
    pub server_version: String,
    pub start_time: Instant,
    pub semaphore: Arc<Semaphore>,
}

/// Run the TCP+TLS server: relay collector snapshots to broadcast, accept clients.
pub async fn run(
    config: ServerConfig,
    mut collector_rx: mpsc::Receiver<SystemSnapshot>,
    token: CancellationToken,
) -> Result<()> {
    let listener = TcpListener::bind(config.bind).await?;
    tracing::info!(addr = %config.bind, "listening (TLS enabled)");

    let tls_acceptor = config.tls_acceptor;

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
                let (tcp_stream, peer) = result?;
                tracing::info!(peer = %peer, "client connected, starting TLS handshake");

                let tls_acceptor = tls_acceptor.clone();
                let state = Arc::clone(&state);
                let snapshot_rx = broadcast_tx.subscribe();
                let client_token = token.clone();

                tokio::spawn(async move {
                    // TLS handshake (with timeout to prevent slowloris).
                    let tls_stream = match tokio::time::timeout(
                        TLS_HANDSHAKE_TIMEOUT,
                        tls_acceptor.accept(tcp_stream),
                    )
                    .await
                    {
                        Ok(Ok(s)) => s,
                        Ok(Err(e)) => {
                            tracing::warn!(peer = %peer, error = %e, "TLS handshake failed");
                            return;
                        }
                        Err(_) => {
                            tracing::warn!(peer = %peer, "TLS handshake timed out");
                            return;
                        }
                    };
                    tracing::debug!(peer = %peer, "TLS handshake complete");

                    let (reader, writer) = tokio::io::split(tls_stream);
                    if let Err(e) = client::handle(reader, writer, peer, state, snapshot_rx, client_token).await {
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
