# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

#### Network (`muxtop-core` — Epic 11)
- `NetworkInterfaceSnapshot`, `NetworkSnapshot`, and `NetworkHistory` types in a new `network` module tracking per-interface bytes, packets, and errors.
- `NetworkHistory` circular buffer (60-entry `VecDeque`) computing bandwidth (bytes/s with timestamp-based division) and sparkline data.
- `SystemSnapshot` extended with a `networks` field collected from `sysinfo::Networks` on each tick.
- Guard hardening: saturating arithmetic for totals, capacity min 2, counter-reset handling, `refresh(false)` in hot path.

#### Network tab (`muxtop-tui` — Epic 12)
- New `Tab::Network` with `Alt+3` keybinding and full navigation.
- Interface table with columns: Interface, State, RX/s, TX/s, Total RX/TX, Errors; color-coded rates (green RX, yellow TX, red errors).
- Summary bar showing total bandwidth and active/total interface count.
- RX/TX sparklines for the selected interface using `NetworkHistory`.
- Per-tab selection, scroll, sort (6 fields: name, rx rate, tx rate, total rx, total tx, errors), and filter state.
- 5 new command palette commands: `SwitchToNetwork`, `SortNetByRx/Tx/Name/Errors`.

#### Wire protocol (`muxtop-proto` — Epic 13)
- New `muxtop-proto` crate implementing the muxtop wire protocol.
- Length-prefixed framing: 4B big-endian length + 1B message type + bincode payload.
- Async `FrameReader` / `FrameWriter` over `tokio::AsyncRead` / `AsyncWrite`.
- `WireMessage` enum: `Snapshot`, `Heartbeat`, `Error`, `Hello`, `Welcome`.
- `MAX_FRAME_SIZE` capped at 4 MiB to limit DoS surface.
- `Serialize`, `Deserialize`, `Encode`, `Decode`, and `PartialEq` derives on all public core types.
- `SystemSnapshot.timestamp` migrated from `Instant` to `timestamp_ms: u64` (milliseconds since Unix epoch) to enable wire serialization.

#### Server daemon (`muxtop-server` — Epic 14)
- New `muxtop-server` crate: TCP daemon that broadcasts system snapshots to connected clients over the muxtop wire protocol.
- Hello/Welcome handshake, token authentication (`--token` / `MUXTOP_TOKEN`), and constant-time comparison.
- `--max-clients` semaphore limiting concurrent connections.
- Heartbeat frame emitted every 5 seconds per client.
- Snapshot broadcast relay from the local collector.
- Graceful shutdown via `CancellationToken`.

#### Remote monitoring (`muxtop-proto` + `muxtop-tui` + CLI — Epic 15)
- `RemoteCollector` TCP client in `muxtop-proto`: connects to a `muxtop-server`, performs Hello/Welcome handshake, and streams `SystemSnapshot` frames through the same `mpsc` channel interface as the local `Collector`.
- Exponential backoff reconnection (1 s → 30 s cap, resets on successful connection).
- `ConnectionEvent` channel for real-time TUI status notifications.
- `--remote host:port` CLI flag: spawns `RemoteCollector` instead of local `Collector`.
- `--token` flag and `MUXTOP_TOKEN` env var for server authentication.
- `ConnectionMode` enum (`Local` | `Remote { hostname, addr }`) in `CliConfig` and `AppState`.
- Remote mode TUI: header displays `→ remote:hostname:port`; kill/renice actions and palette commands disabled with a clear notice; footer hides Kill/Nice hints; warning emitted when `--refresh` is combined with `--remote`.

#### TLS & Security Hardening (`muxtop-server`, `muxtop-proto`, CLI)
- TLS encryption for all client-server communication via `tokio-rustls` (rustls 0.23). All data is now encrypted in transit — tokens, snapshots, and heartbeats are never sent in plaintext.
- Self-signed certificate auto-generation with `--tls-generate` via `rcgen`: generates cert+key, prints SHA-256 fingerprint to stderr, persists to `~/.local/share/muxtop/`.
- Server TLS configuration: `--tls-cert` / `--tls-key` flags for PEM-encoded certificate and private key files.
- Client TLS verification: `--tls-ca <path>` to trust a specific CA/self-signed cert, `--tls-skip-verify` for development (insecure, with warning).
- Mandatory authentication: server refuses to start without `--token` / `MUXTOP_TOKEN` (minimum 16 characters). Client requires `--token` for `--remote` connections. No more unauthenticated plaintext mode.
- `WireMessage` custom `Debug` impl redacting `auth_token` as `[REDACTED]` to prevent accidental token leakage in logs.
- TLS handshake timeouts on both server (10s) and client (5s) to prevent slowloris-style resource exhaustion.
- Private key file created with `0o600` permissions atomically on Unix (no TOCTOU race).
- Generic `AsyncRead`/`AsyncWrite` handler in `client::handle()` — works transparently with TLS streams.
- 6 new TLS integration tests: TLS handshake, snapshot streaming over TLS, cert rejection, skip-verify, auth rejection over TLS, full streaming.

#### Tests & Benchmarks (Epic 16)
- 7 new `muxtop-core` network edge-case unit tests: multi-interface, empty snapshots, sparkline TX, bandwidth, and `is_up` heuristic.
- 2 new `muxtop-server` E2E tests: multi-snapshot streaming (3 snapshots) and snapshot content verification (CPU, memory, processes, networks, timestamp fields).
- Network benchmarks: `NetworkSnapshot::collect`, `NetworkHistory::push_60`, bandwidth calculation with sparklines.
- Proto benchmarks: snapshot serialize/deserialize with 3 000 processes, frame encode/decode round-trip.

#### Documentation
- `CONTRIBUTING.md`: contributor guide covering prerequisites, dev setup, crate architecture, branch model, commit conventions, code standards, and PR process.

---

## [0.1.1] - 2026-04-15

### Added

#### Distribution
- `.deb` package generation for Linux targets (x86_64 and aarch64) via `cargo-deb`, attached to GitHub Releases for Debian/Ubuntu installation.
- Homebrew tap (`lanexadev/homebrew-tap`) with a formula supporting macOS (Intel + Apple Silicon) and Linux (x86_64 + aarch64).
- Automatic Homebrew formula update in the release workflow on each new tag.

### Fixed

#### Security
- Addressed findings from security audit SEC-20260415: refactored action handling in `muxtop-core`, hardened confirmation dialog, and reduced collector surface area.

## [0.1.0] - 2026-04-15

Initial release of **muxtop** — a modern, multiplexed system monitor for the terminal.

### Added

#### Core (`muxtop-core`)
- `SystemSnapshot` collecting CPU, memory, swap, and per-process data via `sysinfo`.
- Process sort (CPU, memory, PID, name, user), filter, and tree builder (parent/child hierarchy).
- Async 1 Hz collector running on a dedicated `tokio` task with graceful shutdown via a cancel token.
- Kill (`SIGTERM`/`SIGKILL`) and renice actions on live processes using `libc`.
- `Display` and `FromStr` implementations for `SortField`, enabling case-insensitive CLI parsing.
- End-to-end integration tests for the collector and process pipeline.
- Criterion benchmark targets: `process_bench` (sort, filter, tree, flatten at 100–5000 procs) and `snapshot_bench` (full `SystemSnapshot::collect`).

#### TUI (`muxtop-tui`)
- Terminal lifecycle management: raw mode, alternate screen, RAII restore guard, and panic hook that restores the terminal before unwinding.
- `AppState` with `Tab` enum, keyboard input dispatch, and mpsc snapshot consumption.
- Crossterm event loop with non-blocking polling and per-frame snapshot drain.
- 4-zone layout: header, tab bar, scrollable content area, and footer.
- `Alt+1` / `Alt+2` and arrow-key tab navigation between General and Processes tabs.
- **General tab**: per-core CPU gauge bars, memory and swap bars, and a system info line (hostname, OS, uptime, kernel).
- **Processes tab**: sortable table (CPU, memory, PID, name, user), inline filter bar (`/`), process tree toggle (`F5`), and column header indicators.
- **Command palette** (`Ctrl+P`): fuzzy-matched command registry powered by `nucleo`; commands for sort, filter, kill, and navigation.
- Kill and renice workflow: `F9` (SIGTERM), `F10` (SIGKILL), `F7` / `F8` (renice ±1) behind a `y`/`n` confirmation dialog.
- `ConfirmAction` enum with per-action prompt text rendered as a centered overlay.
- Status message bar in the footer with auto-clear after 5 seconds; green for success, red for error.
- `Esc` clears the active filter; `I` reverses sort order.
- `CliConfig` struct carrying `--filter`, `--sort`, and `--tree` flags from the CLI into `AppState`.
- `TermCaps` with `ColorSupport` detection from `$TERM` / `$COLORTERM` / `$LANG` at startup.
- `detect_terminal_caps()` for runtime color and Unicode detection.
- **Tokyo Night** TrueColor theme (`theme.rs`) with full palette (background, foreground, accents, status colors), ANSI 16-color fallback for basic terminals, and a `gauge_color()` helper for green/yellow/red gradients.
- Alternating zebra-stripe row backgrounds in the Processes table using the `surface` theme color.
- Bold selected row text and cyan (`accent_primary`) column headers for stronger visual hierarchy.
- Powerline-style system info bar and footer key-hint strip.
- ASCII fallback for non-Unicode terminals: block characters (`#`/`-`), sort arrows (`v`/`^`), tree connectors (`|--`/`\--`), filter cursor (`_`).
- Unit tests covering `CliConfig`, `ConfirmAction::prompt()`, `next_sort_field()`, `AppState::with_config()`, and edge cases (empty snapshot, `PageDown`/`PageUp`/`Home`/`End`).
- Criterion benchmark target `app_bench`: `recompute_visible` (flat, tree, filtered) and palette re-filter.

#### CLI & Distribution
- `--filter <PATTERN>` to pre-seed the process filter on launch.
- `--sort <FIELD>` to set the initial sort column (cpu, mem, pid, name, user).
- `--tree` to start in process tree view.
- `--refresh <HZ>` to override the collector tick rate.
- `--about` flag printing version, license, repository URL, and a no-telemetry pledge.
- POSIX-compatible `scripts/install.sh`: detects OS/arch, downloads the correct binary from GitHub Releases, verifies SHA-256 checksum, and installs to `/usr/local/bin` (root) or `~/.local/bin` (non-root).
- GitHub Actions release workflow uploading `install.sh` alongside pre-built binaries and checksum files.

#### CI / Tooling
- GitHub Actions CI pipeline: `cargo check`, `clippy`, `test`, `fmt`, `cargo-deny` audit, and a bench compile check (`--no-run`) on every push and pull request.
- `cargo-deny` configuration for license and advisory auditing (deny.toml, cargo-deny 0.19 schema).
- `clippy.toml` with MSRV pinned to 1.88.
- `scripts/bench-thomas.sh` macro-benchmark measuring release build time, binary size, startup latency, and all CLI flag paths.

#### Documentation
- Launch-ready README with tagline, badges, one-liner install (Cargo + curl), feature overview, benchmark results, keybinding reference, roadmap, privacy pledge, contributing guide, and license.

### Fixed
- TUI clippy warnings: `items_after_test_module` (moved `run()` above `#[cfg(test)]`) and `io_other_error` (use `std::io::Error::other()`).
- Security: bump `time` crate to v0.3.47 to remediate **RUSTSEC-2026-0009** (stack exhaustion via crafted RFC 2822 input).
- General tab layout: compute CPU block height dynamically from core count to eliminate the large empty gap when few cores are present.
- Wrap Memory bars in a bordered block consistent with the CPU block style.

### Changed
- MSRV bumped from 1.85 to 1.88 to pull in `time` v0.3.47 and enable let-chain collapsing.
- `deny.toml` migrated to cargo-deny 0.19 schema (removed deprecated `advisory` / `license` top-level fields).
- `muxtop-tui::run(rx)` signature extended to `run(rx, config)` accepting `CliConfig` + `TermCaps`.
- `bar_empty` color separated from `selection_bg` so gauge empty portions no longer inherit the selection highlight.
