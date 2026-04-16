mod client;
mod error;
mod server;

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use muxtop_core::collector::Collector;
use muxtop_core::system::SystemSnapshot;

#[derive(Parser, Debug)]
#[command(
    name = "muxtop-server",
    about = "TCP server daemon for muxtop remote system monitoring",
    version,
    author
)]
struct Cli {
    /// Bind address (e.g., 0.0.0.0:4242)
    #[arg(long, default_value = "0.0.0.0:4242")]
    bind: SocketAddr,

    /// Refresh interval in seconds (1–3600)
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u64).range(1..=3600))]
    refresh: u64,

    /// Authentication token (optional; if set, clients must provide it).
    /// Also reads from MUXTOP_TOKEN env var if not passed on command line.
    #[arg(long, env = "MUXTOP_TOKEN")]
    token: Option<String>,

    /// Maximum concurrent client connections
    #[arg(long, default_value_t = 8)]
    max_clients: usize,
}

fn init_tracing() -> Result<()> {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};

    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("muxtop");

    std::fs::create_dir_all(&data_dir).context("failed to create muxtop data directory")?;

    let log_path = data_dir.join("muxtop-server.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .context("failed to open log file")?;

    let env_filter =
        EnvFilter::try_from_env("MUXTOP_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(log_file)
                .with_ansi(false)
                .with_target(true)
                .with_thread_ids(true),
        )
        .with(env_filter)
        .init();

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    init_tracing()?;

    tracing::info!(
        bind = %cli.bind,
        refresh = cli.refresh,
        max_clients = cli.max_clients,
        auth = cli.token.is_some(),
        "muxtop-server starting"
    );

    let token = CancellationToken::new();

    // Spawn the system collector.
    let (collector_tx, collector_rx) = mpsc::channel::<SystemSnapshot>(4);
    let collector = Collector::new(Duration::from_secs(cli.refresh));
    let collector_handle = collector.spawn(collector_tx, token.clone());

    // Run the TCP server.
    let server_config = server::ServerConfig {
        bind: cli.bind,
        max_clients: cli.max_clients,
        auth_token: cli.token,
        refresh_hz: cli.refresh as u32,
    };

    // Install signal handler for graceful shutdown.
    let shutdown_token = token.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install CTRL+C handler");
        tracing::info!("received SIGINT, shutting down");
        shutdown_token.cancel();
    });

    server::run(server_config, collector_rx, token.clone()).await?;

    // Shut down collector.
    token.cancel();
    if let Err(e) = collector_handle.await {
        tracing::error!("collector task panicked: {e:?}");
    }

    tracing::info!("muxtop-server stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_cli_defaults() {
        let cli = Cli::parse_from(["muxtop-server"]);
        assert_eq!(cli.bind, "0.0.0.0:4242".parse().unwrap());
        assert_eq!(cli.refresh, 1);
        assert_eq!(cli.max_clients, 8);
        assert!(cli.token.is_none());
    }

    #[test]
    fn test_cli_custom_args() {
        let cli = Cli::parse_from([
            "muxtop-server",
            "--bind",
            "127.0.0.1:9999",
            "--refresh",
            "5",
            "--token",
            "secret123",
            "--max-clients",
            "4",
        ]);
        assert_eq!(cli.bind, "127.0.0.1:9999".parse().unwrap());
        assert_eq!(cli.refresh, 5);
        assert_eq!(cli.max_clients, 4);
        assert_eq!(cli.token.as_deref(), Some("secret123"));
    }

    #[test]
    fn test_cli_refresh_range_validation() {
        // 0 is out of range
        let result = Cli::try_parse_from(["muxtop-server", "--refresh", "0"]);
        assert!(result.is_err());

        // 3601 is out of range
        let result = Cli::try_parse_from(["muxtop-server", "--refresh", "3601"]);
        assert!(result.is_err());
    }
}
