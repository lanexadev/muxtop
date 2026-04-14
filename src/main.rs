use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use muxtop_core::collector::Collector;
use muxtop_core::system::SystemSnapshot;

#[derive(Parser, Debug)]
#[command(
    name = "muxtop",
    about = "A modern, multiplexed system monitor for the terminal",
    version,
    author
)]
struct Cli {
    /// Refresh interval in seconds
    #[arg(long, default_value_t = 1)]
    refresh: u64,

    /// Initial process filter pattern
    #[arg(long)]
    filter: Option<String>,

    /// Initial sort field (cpu, mem, pid, name, user)
    #[arg(long, default_value = "cpu")]
    sort: String,

    /// Start in tree view mode
    #[arg(long)]
    tree: bool,
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

    init_tracing()?;

    tracing::info!("muxtop starting");

    // Create channel for collector → TUI communication.
    let (tx, rx) = mpsc::channel::<SystemSnapshot>(4);
    let token = CancellationToken::new();

    // Spawn the async collector on a background tokio task.
    let collector = Collector::new(Duration::from_secs(cli.refresh));
    let collector_handle = collector.spawn(tx, token.clone());

    // G-05: Run the TUI on a dedicated blocking thread so it doesn't
    // block the tokio runtime (crossterm::event::poll is a blocking syscall).
    let tui_result = tokio::task::spawn_blocking(move || muxtop_tui::run(rx))
        .await
        .context("TUI thread panicked")?;

    // After TUI exits, shut down the collector.
    token.cancel();
    // G-10: Log collector panics instead of silently discarding.
    if let Err(e) = collector_handle.await {
        tracing::error!("collector task panicked: {e:?}");
    }

    tracing::info!("muxtop shutting down");

    tui_result.context("TUI error")?;

    Ok(())
}
