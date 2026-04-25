# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.1] - 2026-04-25

Hardening + performance follow-up to the v0.3.0 Containers release. Closes every finding from the 2026-04-25 security & performance audit, plus a build-profile sweep that almost halves the binary size.

### Security

#### Server / wire protocol (`muxtop-server`, `muxtop-proto`)
- **TLS 1.3 only.** `ServerConfig` and `ClientConfig` are now pinned via `builder_with_protocol_versions(&[&TLS13])`; a regression test asserts a TLS-1.2 client handshake fails.
- **Hardened self-signed certificates.** Rebuilt around explicit `CertificateParams` with `iPAddress` + `DnsName` SAN (was DNS-only), `PKCS_ECDSA_P256_SHA256`, 90-day validity. The generated key file is opened with `O_NOFOLLOW` + mode `0600`; the parent data dir is `chmod 0700` (Unix). A `<data_dir>/server.fingerprint` is persisted (mode `0644`) so operators can recover the SHA-256 even if the startup banner is swallowed by systemd / CI.
- **Per-IP token-bucket rate limiter** (default 10/s, configurable via `--rate-limit-per-ip`; `0` disables). No new dependency.
- **`max_clients` semaphore acquired in the accept loop *before* the TLS handshake.** Over-quota TCP streams are dropped silently — no TLS handshake, no Error frame.
- **Pre-auth Hello frame capped at 4 KiB** via `FrameReader::read_frame_with_max_payload(usize)`. Post-handshake reads keep the 4 MiB cap.
- **Allocation-bounded bincode decode** (`config::standard().with_limit::<MAX_DECODE_BYTES>()`); a payload claiming a 100 MiB string is rejected without allocation.
- **`--token-file <path>`** flag on both binaries (mutually exclusive with `--token`). 16-char minimum after trim. The in-memory token is wrapped in a private `Token(String)` newtype that redacts on `Debug`. `--token` help now warns about `/proc/<pid>/cmdline` leakage.
- **Insecure-mode visibility.** `--tls-skip-verify` fires `tracing::warn!(target: "muxtop::insecure")` on every handshake; the CLI prints a bordered banner immediately after parsing.
- **Hostname-aware SNI.** New `muxtop_proto::parse_remote_target(s) -> (SocketAddr, ServerName)`: IP literals → `ServerName::IpAddress`, DNS names → `ServerName::DnsName(host)`. Drops the previous SocketAddr-only parse that forced IP-bound certs.

#### Containers + TUI (`muxtop-core`, `muxtop-tui`)
- **`DOCKER_HOST` exfiltration warning.** `container_engine::detect_with` emits `tracing::warn!` whenever `$DOCKER_HOST` resolves to a non-loopback `http://` / `tcp://` target. New `http_host_is_loopback` helper handles IP literals and bracketed IPv6.
- **Symlinked-socket rejection.** `DockerEngine::connect_explicit(allow_symlink: bool)` is the new primary entry point. Auto-detection refuses to follow a symlinked Unix socket; explicit user paths log a warning but proceed.
- **Per-container stats failure isolation.** `list_and_stats` no longer aborts the whole tab when one container returns `PermissionDenied` / `Timeout` / `Other` on stats — the bad row is dropped with a warn log, the rest render normally.
- **ANSI / control-char sanitizer.** New `tui::ui::sanitize::scrub_ctrl(&str) -> Cow<str>` strips bytes in `0x00..=0x1f` (except `\t`) and `0x7f`, applied at every row-render site that displays attacker-controlled strings (process name/command/user, container name/image/status, network interface name). Closes the terminal-escape spoofing surface.

### Performance

#### Event-driven render (TUI keystone)
- `terminal.draw` is now called only on `Snapshot | Resize | Key | Mouse | needs_redraw_flag | status_message_just_expired`. Tick events no longer trigger an unconditional 60 Hz redraw against 1 Hz data. New `AppState::needs_redraw` flag armed by `apply_snapshot`, `pump_action_results`, `set_status`, and any state-mutating key handler. **Idle CPU drops ~5–10×; render-loop allocations from ~24k/s to near-zero.**

#### Hot-path allocation cuts
- `recompute_visible` no longer calls `filter_processes` twice in tree mode (was both at 866 and 877; now reuses the first result).
- 50 ms debounce on burst typing in the filter (`FILTER_DEBOUNCE` + `last_filter_change`); `Enter` / `Esc` commit immediately.
- `AppState::sorted_filtered_containers_cache` populated in `apply_snapshot` and refreshed on every container sort/filter mutation. `draw_body`, `draw_sparklines`, and `selected_container` read from the cache (was three independent `Vec` clones + sorts per render).
- Sparkline data built single-pass with `iter().skip(len.saturating_sub(N))` (was double-reverse + double collect).
- New `process::contains_ignore_case` helper — ASCII fast path with no per-row `to_lowercase` allocation, falls back to a Unicode-correct path.
- `PaletteState::matcher` caches the nucleo `Matcher` across keystrokes; `Command::search_texts()` interns haystacks via `OnceLock<Vec<String>>`. Result: `palette_refilter/short_query` **5 allocs / 257 B** vs 52 allocs / 134 KB before; `palette_refilter/no_match` **1 alloc / 10 B** vs 49 allocs / 134 KB; `long_query` and `no_match` time **−83 to −85 %**.
- `network::draw_network_tab` pre-computes a `BandwidthMap` once per render and threads it into the summary bar, body, and sort comparators (was O(N² log N) string-compare lookups).
- Server-side `Collector` now uses targeted `refresh_memory_specifics` + `refresh_cpu_usage` + `refresh_processes_specifics(...)` instead of `refresh_all` (was walking `/proc` per-process every tick). Per-core CPU labels interned via `OnceLock<RwLock<Vec<String>>>`.

#### Tree mode + recompute_visible
- `apply_snapshot/tree` allocations: 37 376 / 2 088 KB → **29 374 / 1 744 KB** per tick (**−21 % allocs, −16 % bytes**).
- `recompute_visible/tree/500`: **−37 %** time (statistically significant, p = 0.00).

#### Build profile sweep
- New workspace `[profile.release]`: `lto = "fat"`, `codegen-units = 1`, `strip = "symbols"`, `panic = "abort"`. **Binary size 9.2 MiB → 5.3 MiB (−42 %)**, with a small win on cold startup (`--about` 14 ms → 12 ms). `mimalloc` was evaluated but degraded RSS on macOS by ~0.6 MiB (Apple `libmalloc` already returns pages aggressively); not adopted.
- Peak RSS: 10.3 MiB → **11.3 MiB** — net cost of v0.3.0 Containers + bollard, not a regression of this release.

#### Container-action hygiene
- Container Stop/Kill/Restart spawns now race their engine call against a `CancellationToken` cancelled in `quit()` — avoids 10 s of detached tasks surviving past TUI shutdown.
- Engine actions now dispatch with `c.id_full.clone()` instead of the truncated 12-char id (closes the Docker prefix-match risk).

### Wire protocol break

- `ContainerSnapshot` gains `id_full: String` (the 64-char ID). bincode is order-sensitive, so this is a wire-format break — pre-v0.3.1 clients cannot decode v0.3.1 snapshots and vice versa.

### Server / CLI follow-up (carrying v0.3.0 functionality across to remote)
- `maybe_connect_default_engine()` extracted from `src/main.rs` and hoisted into `muxtop-core/src/docker_engine.rs` as the single source of truth for both binaries.
- `muxtop-server` gains `--docker-socket <PATH>` and `--no-containers` flags mirroring the client. The server now calls `Collector::with_container_engine`, so remote clients see actual containers in their `Alt+4` tab.

### Tests

- Workspace test count: 488 (v0.3.0) → **560** (v0.3.1) + 1 `#[ignore]` integration test requiring a live Docker daemon. Breakdown of new tests: rate_limit, frame cap, bincode limit, cert generation (parsed via `x509-parser`), TLS 1.3 enforcement, key file permissions, fingerprint persistence, `--token-file` path, hostname SNI parsing, `scrub_ctrl` clean/dirty/tab/OSC/null/multi-byte UTF-8, `connect_explicit` symlink rejection, per-container error isolation, `http_host_is_loopback` truth table, `tick_does_not_request_redraw`, `pump_action_results_marks_dirty`, `apply_snapshot_populates_container_cache`, `quit_cancels_shutdown_token`, `palette_matcher_is_cached`, `filter_debounce_coalesces_bursts`, `broadcast_arc_frame_shared_across_subscribers`, `contains_ignore_case` ASCII + Unicode paths.
- `cargo check / test --workspace / clippy -D warnings / fmt / deny`: all green.

## [0.3.0] - 2026-04-25

Major feature release: the **Containers** tab (replaces `ctop`) with full Docker/Podman integration via [bollard](https://github.com/fussybeaver/bollard). Auto-detection at startup means `muxtop` gains a fourth tab on any host running a container engine with no extra flags.

### Added

#### Containers (`muxtop-core`, `muxtop-tui`, `muxtop`)
- New `Tab::Containers` (keybind `Alt+4`) with a full rendering path in `muxtop-tui/src/ui/containers.rs`: sortable table of containers with columns NAME / IMAGE (truncated to 30 chars) / STATE / CPU % / MEM used/limit / NET RX/TX / UPTIME, color-coded by state (running=green, paused/restarting=yellow, dead=red, exited/created=dim), zebra stripes, summary bar with engine kind + running/total counts.
- Per-selected-row sparklines: CPU % and RX-delta (60-sample rings per container id, dropped when a container disappears).
- Sort cycles 6 fields: CPU, Mem, Name, NetRx, NetTx, Uptime (`s` cycles, `I/S` toggles direction, header arrow).
- Filter by name / image / id (`/` to open, `Esc` to clear).
- Container actions: `F9` Stop (SIGTERM + 10s grace), `F10` Kill (SIGKILL), `F11` Restart, each gated by a y/n confirmation dialog. Disabled in remote mode with the same notice style as Processes kill/renice.
- 5 new palette commands: `SwitchToContainers`, `SortContainersByCpu/Mem/Name/NetRx`. 3 additional action commands (`StopContainer`, `KillContainer`, `RestartContainer`) with `F9`/`F10`/`F11` shortcut labels and exclusion from the palette in remote mode.
- Three render fallbacks: `containers = None` → "Waiting for data...", engine configured but `daemon_up = false` → "No container daemon detected" with a CLI hint, empty list → "No containers" or "No containers match filter".

#### Container engine (`muxtop-core`)
- `ContainerEngine` async trait (`async-trait` crate, see ADR-01 in `forge/24-epic1-container-engine-trait`) with methods `list_and_stats`, `stop`, `kill`, `restart`, `kind`.
- `DockerEngine` concrete implementation on top of `bollard 0.20`: handles Unix socket + HTTP/TCP targets, probes `/info` within 5 s, detects Docker / Podman / Unknown, fetches stats in parallel via `futures::stream::buffer_unordered(16)`, filters `ContainerNotFound` silently on race-with-removal.
- CPU percentage computed client-side from a cached `(cpu_usage, system_cpu_usage)` per container with `saturating_sub` on counter resets. First tick after startup yields 0 % — acceptable 2 s warm-up at the collector's 0.5 Hz refresh rate.
- Socket auto-detection (`detect_socket`) with fallback chain: `$DOCKER_HOST` → `/var/run/docker.sock` → `$XDG_RUNTIME_DIR/podman/podman.sock` → `/run/podman/podman.sock`. Pure path selection only (reachability is `DockerEngine::connect`'s job).
- `EnvLookup` trait for parallel-safe tests (no `std::env` global mutation).
- `EngineError` enum with granular variants (`ConnectFailed`, `ContainerNotFound`, `PermissionDenied`, `Timeout`, `Other`) and a `#[from] EngineError` bridge to `CoreError`.
- `Collector::with_container_engine(interval, Option<Arc<dyn ContainerEngine + Send + Sync>>)`: drives a second `tokio::time::interval(2s)` task that calls the engine and publishes the result (or `ContainersSnapshot::unavailable()`) into a shared `Arc<Mutex<Option<ContainersSnapshot>>>`. Each system-tick `SystemSnapshot` carries the latest container snapshot through the new `containers: Option<ContainersSnapshot>` field.

#### CLI (`muxtop`)
- `--docker-socket <PATH>` flag to override autodetection.
- `--no-containers` flag to disable the container engine entirely.
- `maybe_build_container_engine()` runs autodetection + `DockerEngine::connect` at startup; on failure it logs a tracing warning and degrades to a None engine so muxtop always boots. The built `Arc<dyn ContainerEngine>` is cloned into both the Collector (stats) and the TUI (actions) so both hit the same daemon.

#### Wire protocol (`muxtop-proto`)
- `ContainerSnapshot`, `ContainersSnapshot`, `ContainerState` (7 variants), `EngineKind` derive `Serialize`, `Deserialize`, `Encode`, `Decode`, `PartialEq`, `Clone`, `Debug` so they cross the wire via `WireMessage::Snapshot(SystemSnapshot)` unchanged.
- Integration tests: 20-container round-trip, `unavailable()` sentinel round-trip, 100-container frame-size sanity check (< 256 KiB vs the 4 MiB `MAX_FRAME_SIZE`).
- Criterion benches `containers_serialize_100` + `containers_deserialize_100` for regression tracking.

### Dependencies (workspace)
- `async-trait = "0.1"` — dyn-safe async trait macro (see ADR-01 in forge/24).
- `bollard = "0.20"` — Docker/Podman client (brings `hyper 1`, `http 1`, `futures 0.3`).
- `futures = "0.3"` — `stream::buffer_unordered`.
- `tempfile = "3"` added as dev-dep to `muxtop-core` for socket-detection tests.

### Changed
- `SystemSnapshot::collect` signature gained a third argument `containers: Option<ContainersSnapshot>`. All internal call sites updated; the Collector is the sole production caller and passes the latest container snapshot from its shared slot.
- `muxtop_tui::run` signature gained an `Option<Arc<dyn ContainerEngine + Send + Sync>>` parameter. `src/main.rs` forwards the autodetected engine; passing `None` disables actions (they surface "Container engine not configured" as a status message).
- `Tab::ALL` now has 4 variants; `Tab::next()` / `Tab::prev()` cycle through General → Processes → Network → Containers. Arrow / Tab / BackTab navigation updated accordingly.
- `FUTURE_TABS` in the tab bar no longer shows "Containers [soon]" — only "GPU [soon]" remains.

### Tests
- Workspace test count: 421 (v0.2.2) → **488** (v0.3.0). Breakdown of the +67 new tests: `muxtop-core` containers/container_engine/docker_engine (+44), `muxtop-tui` ui::containers + app container actions (+19), `muxtop-proto` integration (+4). One new `#[ignore]` integration test requires a live Docker daemon (run with `cargo test -- --ignored`).
- `cargo-deny check` remains clean with the new transitive deps (hyper 1.9, http 1.4, tokio-util features).

## [0.2.3] - 2026-04-24

### Added
- `scripts/bench-thomas.sh` now measures peak RSS over a 30 s headless collector run (uses `/usr/bin/time -l` on macOS, `/usr/bin/time -v` on Linux). Gives a publishable memory footprint number for comparison with other monitors.
- Hidden `--bench-run <secs>` flag on the `muxtop` binary: runs the collector + `AppState::apply_snapshot` / `recompute_visible` loop without a TUI, then exits. Lets external tools measure steady-state RSS without a TTY.
- `cargo run --example alloc_profile -p muxtop-tui` (also `just bench-alloc`) — runs the hot paths (`PaletteState::refilter`, `sort_processes`, `AppState::apply_snapshot`) under the `dhat` global allocator and reports per-iteration allocation counts and bytes. Complements the criterion time benches for catching allocation regressions.

### Security
- Bump `rustls-webpki` to 0.103.13 to remediate **RUSTSEC-2026-0104**.

## [0.2.2] - 2026-04-20

### Performance
- `PaletteState::refilter_excluding` no longer allocates a throwaway `Vec<Command>` on every call; the empty-input hot path is **−84 %** faster (178 ns → 28 ns). Other palette variants improve 3–11 %.
- `sort_processes` uses `sort_by_cached_key` for `Name` / `User` fields so `to_lowercase()` runs O(n) instead of O(n log n) times. `name_asc/5000` drops from 4.69 ms to **765 µs (−84 %)**; `cpu_desc/5000` from 966 µs to **436 µs (−55 %)**.
- `muxtop --about` no longer builds a Tokio multi-threaded runtime before printing. `main()` is now synchronous and constructs the runtime only when entering the TUI path. Cuts `--about` startup from an effective cold-start cost to ~18 ms on warm runs.

### Fixed
- `scripts/bench-thomas.sh` now warms up the binary with `--version` before timing `--about`, so measurements don't capture the one-time macOS Gatekeeper scan cost of a freshly-rebuilt binary.

## [0.2.1] - 2026-04-16

### Fixed
- Clippy lints: replaced `sort_by` with `sort_by_key` for cleaner sort expressions, and collapsed single-branch `if` blocks inside `match` arms into match guards.
- CI: fixed `cargo publish` workflow to include `muxtop-proto` in the correct dependency order, and fixed a bash `errexit` bug that silently swallowed publish errors.

## [0.2.0] - 2026-04-16

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
