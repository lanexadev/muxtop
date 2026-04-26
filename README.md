# muxtop

**A modern, multiplexed system monitor for the terminal.**

[![CI](https://github.com/lanexadev/muxtop/actions/workflows/ci.yml/badge.svg)](https://github.com/lanexadev/muxtop/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/muxtop.svg)](https://crates.io/crates/muxtop)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE-MIT)

muxtop replaces the `htop` + `iftop` + `ctop` workflow with a single tabbed interface.
Think htop, but with multiplexer-style UX (à la tmux/zellij) and a VS Code-style command palette.

---

## Installation

### Via crates.io

```sh
cargo install muxtop
```

### Via Homebrew (macOS / Linux)

```sh
brew tap lanexadev/tap
brew install muxtop
```

### Via APT (Debian / Ubuntu)

```sh
# Add the repo (one time)
curl -fsSL https://lanexadev.github.io/apt/gpg.key | sudo gpg --dearmor -o /usr/share/keyrings/lanexadev.gpg
echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/lanexadev.gpg] https://lanexadev.github.io/apt stable main" | sudo tee /etc/apt/sources.list.d/lanexadev.list

# Install
sudo apt update
sudo apt install muxtop
```

### Pre-built binary (Linux / macOS)

```sh
curl -sSfL https://raw.githubusercontent.com/lanexadev/muxtop/main/scripts/install.sh | sh
```

### From source

```sh
git clone https://github.com/lanexadev/muxtop.git
cd muxtop
cargo build --release
# Binary available at target/release/muxtop
```

> MSRV: Rust **1.88**

---

## Features

| Feature | Detail |
|---|---|
| **Tabs** | General, Processes, Network and Containers — `Alt+1` / `Alt+2` / `Alt+3` / `Alt+4` |
| **Network tab** | Interface table with RX/s, TX/s, totals, errors + real-time sparklines |
| **Containers tab** | Docker/Podman via [bollard](https://github.com/fussybeaver/bollard) — CPU/memory/network/IO table, CPU+RX sparklines, `F9` stop / `F10` kill / `F11` restart actions, automatic socket detection |
| **Command palette** | `Ctrl+P` — `kill firefox`, `sort memory`, `stop nginx`, `restart postgres`, etc. |
| **htop shortcuts** | `F3` search, `F4` filter, `F5` tree, `F6` sort, `F9` kill, `F10` quit |
| **Fuzzy search** | Powered by [nucleo](https://github.com/helix-editor/nucleo) (from the Helix editor) |
| **Tree view** | `F5` toggles the parent/child hierarchical display |
| **Renice** | `+` / `-` to adjust process priority |
| **Remote monitoring** | `--remote host:port` + `--token` to monitor a remote server over encrypted TLS |
| **Native TLS** | rustls encryption (TLS 1.3-only since 0.3.1), self-signed cert auto-generation (`--tls-generate`), mandatory token auth |
| **Async collection** | tokio-based — the UI never blocks, even at 3000+ processes |
| **Tokyo Night theme** | Native TrueColor, automatic fallback for ANSI/16-color terminals |
| **Static binary** | Single musl binary, no system dependencies |
| **Zero telemetry** | No client-side network calls, ever (see [Privacy](#privacy--telemetry)) |

---

## Privileges

Access to the Docker socket (`/var/run/docker.sock`) is **equivalent to root access** on the host machine: any user in the `docker` group can launch a privileged container and break out. To run muxtop with a minimal privilege budget, use **rootless Podman** — the user-scoped socket (`$XDG_RUNTIME_DIR/podman/podman.sock`) is isolated per user and muxtop detects it automatically. Avoid running `muxtop-server` as root on an exposed host: prefer a service account with only the rootless Podman socket mounted read/write.

---

## Usage

```sh
muxtop                              # normal launch (auto-detects Docker/Podman)
muxtop --refresh 2                  # refresh every 2 seconds
muxtop --filter firefox             # start with a process filter
muxtop --sort mem                   # sort by memory at startup
muxtop --tree                       # start in tree view
muxtop --about                      # version, license, privacy pledge

# Containers tab — by default muxtop checks $DOCKER_HOST, /var/run/docker.sock,
# then the Podman sockets. Pass a path to force, or disable entirely:
muxtop --docker-socket /var/run/docker.sock   # socket override
muxtop --no-containers                        # disable container collection

# Run the server (TLS + auth required)
muxtop-server --token "my-secret-16chars" --tls-generate
muxtop-server --token "my-secret-16chars" --tls-cert cert.pem --tls-key key.pem
muxtop-server --token "my-secret-16chars" --tls-generate --bind 0.0.0.0:4242 --max-clients 10

# Remote monitoring (TLS)
muxtop --remote host:port --token "my-secret-16chars" --tls-skip-verify  # dev
muxtop --remote host:port --token "my-secret-16chars" --tls-ca cert.pem  # production
MUXTOP_TOKEN="my-secret-16chars" muxtop --remote host:port --tls-ca cert.pem
```

### Keyboard shortcuts

| Key | Action |
|--------|--------|
| `Ctrl+P` | Command palette |
| `Alt+1` / `Alt+2` / `Alt+3` / `Alt+4` | Switch tab (General / Processes / Network / Containers) |
| `F1` | Help |
| `F3` / `/` | Search |
| `F4` | Process filter |
| `F5` | Tree view |
| `F6` | Sort menu |
| `F9` | Kill process (Processes tab) · Stop container (Containers tab) |
| `F10` | Force kill (SIGKILL) — process or container depending on the active tab |
| `F11` | Restart container (Containers tab) |
| `q` | Quit |
| `j` / `k` | Navigation (vim-style) |
| `+` / `-` | Renice (priority) — Processes tab only |

---

## Benchmarks

Tested on macOS with 500+ processes (Thomas benchmark):

| Metric | Target | muxtop |
|----------|-------|--------|
| Startup (`--about`) | < 100 ms | ~12 ms |
| Binary size | < 10 MB | **5.3 MiB** (LTO + strip) |
| FPS (TUI) | > 30 | ~60 (event-driven, idle ≈ 0 redraws) |
| Peak RSS (30 s) | < 15 MiB | **11.3 MiB** (htop ~15, btop ~40) |

Run the benchmark yourself:

```sh
just bench-thomas
# or
./scripts/bench-thomas.sh
```

---

## Architecture

```
muxtop/
├── src/                         # Entry point (clap CLI + tokio bootstrap)
└── crates/
    ├── muxtop-core/             # System collection, data models, actions
    │   ├── src/collector.rs     # Async sysinfo loop (1 Hz) + container loop (0.5 Hz)
    │   ├── src/process.rs       # Sort, filter, process tree
    │   ├── src/system.rs        # CPU / memory / load snapshots
    │   ├── src/network.rs       # Network interfaces + history
    │   ├── src/containers.rs    # Container model (ContainerSnapshot, states, engine)
    │   ├── src/container_engine.rs # Async trait + Docker/Podman socket detection
    │   └── src/docker_engine.rs # Concrete bollard-backed implementation
    ├── muxtop-tui/              # ratatui interface
    │   ├── src/app.rs           # State machine, event handling
    │   └── src/ui/              # Tabs General, Processes, Network, Containers, palette, theme
    ├── muxtop-proto/            # Wire protocol and binary serialization
    └── muxtop-server/           # TCP daemon for remote monitoring
```

---

## Development

```sh
just check    # fmt + clippy + tests
just bench    # criterion micro-benchmarks
just dev      # continuous check with bacon
```

---

## Roadmap

| Version | Goal |
|---------|----------|
| **v0.1** ✓ | htop replacement — tabs, command palette, tree view |
| **v0.2** ✓ | Network tab (replaces iftop) + client/server architecture (`muxtop-server`, `--remote`) |
| **v0.3** ✓ | Docker / Podman Containers tab (via [bollard](https://github.com/fussybeaver/bollard)) + Stop/Kill/Restart actions |
| **v0.3.1** ✓ | TLS 1.3 hardening, per-IP rate limit, ANSI sanitizer, event-driven render, `lto=fat` build sweep |
| **v0.4** ✓ | Kubernetes Pod tab (read-only) via [kube-rs](https://github.com/kube-rs/kube), kubeconfig auto-detection, metrics-server graceful degradation |
| v0.5 | GPU monitoring (NVIDIA / AMD / Apple Silicon) + interactive `docker exec` (PTY) |
| v1.0 | WASM plugin system + themes + configuration file |

---

## Privacy & telemetry

muxtop collects **NO** telemetry, **NO** statistics and contacts **NO ONE**. Ever.

It makes no network calls. It is designed for air-gapped production servers.
If you observe outbound network activity from muxtop, that is a bug — please [report it](https://github.com/lanexadev/muxtop/issues).

---

## Contributing

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for prerequisites, code conventions, the branch workflow and PR submission instructions.

---

## License

Available under either of the following licenses, at your option:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))
