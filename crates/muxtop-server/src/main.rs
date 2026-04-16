mod client;
mod error;
mod server;
pub mod tls;

use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
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
    /// Bind address (e.g., 127.0.0.1:4242)
    #[arg(long, default_value = "127.0.0.1:4242")]
    bind: SocketAddr,

    /// Refresh interval in seconds (1–3600)
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u64).range(1..=3600))]
    refresh: u64,

    /// Authentication token (required). Also reads from MUXTOP_TOKEN env var.
    #[arg(long, env = "MUXTOP_TOKEN")]
    token: Option<String>,

    /// Maximum concurrent client connections
    #[arg(long, default_value_t = 8)]
    max_clients: usize,

    /// Path to PEM-encoded TLS certificate file
    #[arg(long, requires = "tls_key")]
    tls_cert: Option<PathBuf>,

    /// Path to PEM-encoded TLS private key file
    #[arg(long, requires = "tls_cert")]
    tls_key: Option<PathBuf>,

    /// Auto-generate a self-signed TLS certificate (for development)
    #[arg(long, conflicts_with_all = ["tls_cert", "tls_key"])]
    tls_generate: bool,
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

    // Mandatory authentication: token is required (minimum 16 characters).
    let auth_token = match cli.token {
        Some(t) if t.len() >= 16 => t,
        Some(t) if !t.is_empty() => {
            bail!(
                "Authentication token is too short ({} chars). \
                 Use at least 16 characters for security.",
                t.len()
            );
        }
        _ => {
            bail!(
                "Authentication token is required. Set --token <secret> or MUXTOP_TOKEN env var \
                 (minimum 16 characters)."
            );
        }
    };

    // Build TLS acceptor.
    let tls_acceptor = if let (Some(cert_path), Some(key_path)) = (&cli.tls_cert, &cli.tls_key) {
        tls::acceptor_from_pem(cert_path, key_path).context("failed to load TLS certificate/key")?
    } else if cli.tls_generate {
        let hostname = cli.bind.ip().to_string();
        let (cert_pem, key_pem) =
            tls::generate_self_signed(&hostname).context("failed to generate self-signed cert")?;

        // Parse cert DER for fingerprint.
        use rustls_pki_types::pem::PemObject;
        let certs: Vec<rustls_pki_types::CertificateDer<'static>> =
            rustls_pki_types::CertificateDer::pem_slice_iter(cert_pem.as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .context("failed to parse generated certificate")?;
        let fingerprint = tls::cert_fingerprint(certs[0].as_ref());
        eprintln!("TLS: using auto-generated self-signed certificate");
        eprintln!("TLS: SHA-256 fingerprint: {fingerprint}");

        // Write to temp files so acceptor_from_pem can load them.
        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("muxtop");
        std::fs::create_dir_all(&data_dir)?;

        let cert_path = data_dir.join("server.crt");
        let key_path = data_dir.join("server.key");
        std::fs::write(&cert_path, &cert_pem)?;

        // Write key file with restricted permissions from the start (no TOCTOU race).
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&key_path)?
                .write_all(key_pem.as_bytes())
                .map_err(|e| anyhow::anyhow!("failed to write key file: {e}"))?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&key_path, &key_pem)?;
        }

        eprintln!(
            "TLS: cert saved to {}, key saved to {}",
            cert_path.display(),
            key_path.display()
        );

        tls::acceptor_from_pem(&cert_path, &key_path)
            .context("failed to load generated TLS certificate")?
    } else {
        bail!(
            "TLS is required. Use --tls-cert/--tls-key to provide certificates, \
             or --tls-generate for a self-signed certificate."
        );
    };

    tracing::info!(
        bind = %cli.bind,
        refresh = cli.refresh,
        max_clients = cli.max_clients,
        "muxtop-server starting (TLS enabled)"
    );

    let token = CancellationToken::new();

    // Spawn the system collector.
    let (collector_tx, collector_rx) = mpsc::channel::<SystemSnapshot>(4);
    let collector = Collector::new(Duration::from_secs(cli.refresh));
    let collector_handle = collector.spawn(collector_tx, token.clone());

    // Run the TCP+TLS server.
    let server_config = server::ServerConfig {
        bind: cli.bind,
        max_clients: cli.max_clients,
        auth_token,
        refresh_hz: cli.refresh as u32,
        tls_acceptor,
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
        assert_eq!(cli.bind, "127.0.0.1:4242".parse().unwrap());
        assert_eq!(cli.refresh, 1);
        assert_eq!(cli.max_clients, 8);
        assert!(cli.token.is_none());
        assert!(!cli.tls_generate);
        assert!(cli.tls_cert.is_none());
        assert!(cli.tls_key.is_none());
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
            "my-super-secret-token-1234",
            "--max-clients",
            "4",
            "--tls-generate",
        ]);
        assert_eq!(cli.bind, "127.0.0.1:9999".parse().unwrap());
        assert_eq!(cli.refresh, 5);
        assert_eq!(cli.max_clients, 4);
        assert_eq!(cli.token.as_deref(), Some("my-super-secret-token-1234"));
        assert!(cli.tls_generate);
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

    #[test]
    fn test_cli_tls_cert_requires_key() {
        let result = Cli::try_parse_from(["muxtop-server", "--tls-cert", "cert.pem"]);
        assert!(result.is_err(), "--tls-cert requires --tls-key");
    }

    #[test]
    fn test_cli_tls_generate_conflicts_with_cert() {
        let result = Cli::try_parse_from([
            "muxtop-server",
            "--tls-generate",
            "--tls-cert",
            "cert.pem",
            "--tls-key",
            "key.pem",
        ]);
        assert!(
            result.is_err(),
            "--tls-generate conflicts with --tls-cert/--tls-key"
        );
    }
}
