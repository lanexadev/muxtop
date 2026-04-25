use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Parser;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use muxtop_core::collector::Collector;
use muxtop_core::container_engine::ContainerEngine;
use muxtop_core::docker_engine::maybe_connect_default_engine;
use muxtop_core::process::SortField;
use muxtop_core::system::SystemSnapshot;
use muxtop_proto::{RemoteCollector, parse_remote_target};
use muxtop_tui::{CliConfig, ConnectionMode};

/// Newtype wrapper around the in-memory authentication token whose `Debug`
/// impl deliberately redacts the secret (MED-S3). Mirrors the same type in
/// `muxtop-server::main` — kept as a private per-binary type to avoid pulling
/// the secret across crate boundaries.
#[derive(Clone)]
struct Token(String);

impl Token {
    fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(\"[REDACTED]\")")
    }
}

/// Read an auth token from a file (trim whitespace, enforce 16-char minimum).
fn read_token_file(path: &Path) -> Result<Token> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read --token-file {}", path.display()))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!(
            "--token-file {} is empty after trimming whitespace",
            path.display()
        );
    }
    if trimmed.len() < 16 {
        bail!(
            "--token-file {} contains a {}-char token; minimum is 16 characters",
            path.display(),
            trimmed.len()
        );
    }
    Ok(Token(trimmed.to_string()))
}

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

    /// Authentication token for remote server (required for --remote, ≥16
    /// chars). Note: --token leaks via /proc/<pid>/cmdline and `ps eww` on
    /// shared hosts; prefer --token-file there.
    #[arg(long, env = "MUXTOP_TOKEN", conflicts_with = "token_file")]
    token: Option<String>,

    /// Read the authentication token from a file (preferred on shared hosts).
    /// File is read once at startup; trailing whitespace is trimmed.
    #[arg(long, value_name = "PATH")]
    token_file: Option<PathBuf>,

    /// Path to CA certificate for TLS verification (PEM format)
    #[arg(long)]
    tls_ca: Option<PathBuf>,

    /// Skip TLS certificate verification (INSECURE — for development only)
    #[arg(long, conflicts_with = "tls_ca")]
    tls_skip_verify: bool,

    /// Override Docker/Podman socket path (e.g. /var/run/docker.sock). If
    /// omitted, muxtop auto-detects via $DOCKER_HOST, the standard socket
    /// locations, and falls back to no-engine if nothing is reachable.
    #[arg(long, value_name = "PATH")]
    docker_socket: Option<PathBuf>,

    /// Disable container engine autodetection entirely (Containers tab stays
    /// in "no engine configured" state).
    #[arg(long)]
    no_containers: bool,

    /// [benchmark] Run the collector + apply snapshots through AppState for N
    /// seconds without rendering, then exit cleanly. Used by the macro
    /// benchmark to measure steady-state RSS without a TTY.
    #[arg(long, hide = true, value_name = "SECS")]
    bench_run: Option<u64>,
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

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Fast path: `--about` must not pay the cost of building the tokio runtime
    // (clap short-circuits `--version`/`--help` inside `parse()`, but `--about`
    // is a custom flag and reaches main). Spinning up `rt-multi-thread` for
    // a trivial print adds ~500 ms of cold-start latency.
    if cli.about {
        print_about();
        return Ok(());
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;
    runtime.block_on(run_app(cli))
}

async fn run_app(cli: Cli) -> Result<()> {
    init_tracing()?;

    tracing::info!("muxtop starting");

    // HIGH-S1: bright startup banner when TLS verification is off. The
    // single eprintln! is not enough on its own (it scrolls away in tmux,
    // systemd, CI), but it's the only signal a user gets *before* a TUI
    // session starts. The persistent log heartbeat lives inside
    // `NoVerifier::verify_server_cert` and fires per handshake.
    if cli.tls_skip_verify {
        eprintln!(
            "============================================================\n\
             WARNING: --tls-skip-verify is set.\n\
             Server certificates will NOT be validated. Anyone on the\n\
             network path can impersonate the muxtop-server. Use only\n\
             on a trusted local-loopback or development network.\n\
             ============================================================"
        );
    }

    // Warn about ignored --refresh in remote mode.
    if cli.remote.is_some() && cli.refresh != 1 {
        eprintln!("warning: --refresh is ignored in remote mode (server dictates timing)");
    }

    // Create channel for collector → TUI communication.
    let (tx, rx) = mpsc::channel::<SystemSnapshot>(4);
    let cancel = CancellationToken::new();

    // Build a container engine in local mode (opt-out via --no-containers).
    // `None` on remote, or when detection/connection fails — the UI will
    // render the appropriate fallback.
    let container_engine: Option<Arc<dyn ContainerEngine + Send + Sync>> =
        if cli.remote.is_none() && !cli.no_containers {
            maybe_connect_default_engine(cli.docker_socket.as_deref()).await
        } else {
            None
        };

    // Determine connection mode and spawn appropriate collector.
    //
    // ADR-30-1: --remote accepts `host:port` where `host` may be an IP
    // literal OR a DNS name. The hostname is preserved as-is for SNI; the
    // resolved IP is used for the TCP connect. `parse_remote_target` lives
    // in `muxtop-proto::tls` so its parsing/error semantics are shared with
    // any future client.
    let parsed_remote = if let Some(ref addr_str) = cli.remote {
        let (addr, sni) = parse_remote_target(addr_str)
            .with_context(|| format!("invalid --remote target: {addr_str}"))?;
        Some((addr_str.clone(), addr, sni))
    } else {
        None
    };

    let collector_handle = if let Some((_, addr, ref server_name)) = parsed_remote {
        // Token is mandatory for remote connections.
        let token: Token = if let Some(path) = cli.token_file.as_deref() {
            read_token_file(path)?
        } else {
            match cli.token.clone() {
                Some(t) if t.len() >= 16 => Token(t),
                Some(t) if !t.is_empty() => bail!(
                    "Authentication token is too short ({} chars). \
                     Use at least 16 characters for security.",
                    t.len()
                ),
                _ => bail!(
                    "Authentication token is required for remote connections. \
                     Set --token <secret>, --token-file <path>, or the MUXTOP_TOKEN env var \
                     (minimum 16 characters)."
                ),
            }
        };

        // Build TLS connector.
        let tls_connector = if let Some(ref ca_path) = cli.tls_ca {
            muxtop_proto::tls::connector_from_ca(ca_path)
                .context("failed to load TLS CA certificate")?
        } else if cli.tls_skip_verify {
            // The bright startup banner has already fired above; the
            // per-handshake log heartbeat lives inside `NoVerifier`.
            muxtop_proto::tls::connector_insecure()
        } else {
            bail!(
                "TLS CA certificate is required. Use --tls-ca <path> to specify the server's \
                 certificate, or --tls-skip-verify for development (INSECURE)."
            );
        };

        let remote = RemoteCollector::new(
            addr,
            Some(token.into_inner()),
            tls_connector,
            server_name.clone(),
        );
        remote.spawn(tx, None, cancel.clone())
    } else {
        let collector = Collector::with_container_engine(
            Duration::from_secs(cli.refresh),
            container_engine.clone(),
        );
        collector.spawn(tx, cancel.clone())
    };

    // Build connection mode for TUI.
    let connection_mode = if let Some((raw, addr, _)) = parsed_remote {
        ConnectionMode::Remote {
            hostname: raw,
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

    // Benchmark mode: drain snapshots into an AppState for N seconds with no
    // rendering, then exit. Lets external tools measure steady-state RSS
    // without a TTY.
    if let Some(secs) = cli.bench_run {
        bench_run_loop(rx, config, Duration::from_secs(secs)).await;
        cancel.cancel();
        if let Err(e) = collector_handle.await {
            tracing::error!("collector task panicked: {e:?}");
        }
        tracing::info!("muxtop bench-run shutting down");
        return Ok(());
    }

    // G-05: Run the TUI on a dedicated blocking thread so it doesn't
    // block the tokio runtime (crossterm::event::poll is a blocking syscall).
    // The container engine is shared (Arc) with the Collector so Stop/Kill/
    // Restart actions hit the same daemon.
    let tui_engine = container_engine.clone();
    let tui_result = tokio::task::spawn_blocking(move || muxtop_tui::run(rx, config, tui_engine))
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

/// Headless benchmark loop: drains snapshots into an `AppState` and exercises
/// `apply_snapshot` + `recompute_visible` for `duration` seconds, then exits.
/// Used to measure steady-state RSS without a TTY.
async fn bench_run_loop(
    mut rx: mpsc::Receiver<SystemSnapshot>,
    config: CliConfig,
    duration: Duration,
) {
    let mut app =
        muxtop_tui::AppState::with_config(config, muxtop_tui::terminal::TermCaps::default());
    let deadline = tokio::time::Instant::now() + duration;
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            maybe_snap = rx.recv() => match maybe_snap {
                Some(snap) => {
                    app.apply_snapshot(snap);
                    app.recompute_visible();
                }
                None => break,
            }
        }
    }
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
