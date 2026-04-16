use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use muxtop_core::collector::Collector;
use muxtop_core::process::SortField;
use muxtop_core::system::SystemSnapshot;
use muxtop_proto::RemoteCollector;
use muxtop_tui::{CliConfig, ConnectionMode};

#[derive(Parser, Debug)]
#[command(
    name = "muxtop",
    about = "A modern, multiplexed system monitor for the terminal",
    version,
    author
)]
struct Cli {
    /// Refresh interval in seconds (1–3600)
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u64).range(1..=3600))]
    refresh: u64,

    /// Initial process filter pattern
    #[arg(long)]
    filter: Option<String>,

    /// Initial sort field (cpu, mem, pid, name, user)
    #[arg(long, default_value = "cpu", value_parser = clap::value_parser!(SortField))]
    sort: SortField,

    /// Start in tree view mode
    #[arg(long)]
    tree: bool,

    /// Show version, license, repository, and privacy pledge
    #[arg(long)]
    about: bool,

    /// Connect to a remote muxtop-server (host:port)
    #[arg(long)]
    remote: Option<String>,

    /// Authentication token for remote server
    #[arg(long, env = "MUXTOP_TOKEN")]
    token: Option<String>,
}

fn init_tracing() -> Result<()> {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};

    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("muxtop");

    std::fs::create_dir_all(&data_dir).context("failed to create muxtop data directory")?;

    let log_path = data_dir.join("muxtop.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .context("failed to open log file")?;

    let env_filter =
        EnvFilter::try_from_env("MUXTOP_LOG").unwrap_or_else(|_| EnvFilter::new("warn"));

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

    if cli.about {
        print_about();
        return Ok(());
    }

    init_tracing()?;

    tracing::info!("muxtop starting");

    // Warn about ignored --refresh in remote mode.
    if cli.remote.is_some() && cli.refresh != 1 {
        eprintln!("warning: --refresh is ignored in remote mode (server dictates timing)");
    }

    // Create channel for collector → TUI communication.
    let (tx, rx) = mpsc::channel::<SystemSnapshot>(4);
    let cancel = CancellationToken::new();

    // Determine connection mode and spawn appropriate collector.
    let collector_handle = if let Some(ref addr_str) = cli.remote {
        let addr: SocketAddr = addr_str
            .parse()
            .context("invalid --remote address (expected host:port, e.g. 127.0.0.1:4242)")?;

        let remote = RemoteCollector::new(addr, cli.token.clone());
        remote.spawn(tx, None, cancel.clone())
    } else {
        let collector = Collector::new(Duration::from_secs(cli.refresh));
        collector.spawn(tx, cancel.clone())
    };

    // Build connection mode for TUI.
    let connection_mode = if let Some(ref addr_str) = cli.remote {
        let addr: SocketAddr = addr_str.parse().unwrap(); // already validated above
        ConnectionMode::Remote {
            hostname: addr_str.clone(),
            addr,
        }
    } else {
        ConnectionMode::Local
    };

    let config = CliConfig {
        filter: cli.filter,
        sort_field: cli.sort,
        tree_mode: cli.tree,
        connection_mode,
    };

    // G-05: Run the TUI on a dedicated blocking thread so it doesn't
    // block the tokio runtime (crossterm::event::poll is a blocking syscall).
    let tui_result = tokio::task::spawn_blocking(move || muxtop_tui::run(rx, config))
        .await
        .context("TUI thread panicked")?;

    // After TUI exits, shut down the collector.
    cancel.cancel();
    // G-10: Log collector panics instead of silently discarding.
    if let Err(e) = collector_handle.await {
        tracing::error!("collector task panicked: {e:?}");
    }

    tracing::info!("muxtop shutting down");

    tui_result.context("TUI error")?;

    Ok(())
}

fn print_about() {
    let version = env!("CARGO_PKG_VERSION");
    println!("muxtop v{version}");
    println!("A modern, multiplexed system monitor for the terminal");
    println!();
    println!("License:     MIT OR Apache-2.0");
    println!("Repository:  https://github.com/lanexadev/muxtop");
    println!("Authors:     Lucas Schimmel");
    println!();
    println!("Privacy:     muxtop collects NO telemetry, NO analytics,");
    println!("             and phones home to NOBODY. Ever.");
}
