# muxtop

**A modern, multiplexed system monitor for the terminal.**

[![CI](https://github.com/lucasschimmel/muxtop/actions/workflows/ci.yml/badge.svg)](https://github.com/lucasschimmel/muxtop/actions/workflows/ci.yml)
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
brew tap lucasschimmel/tap
brew install muxtop
```

### Via APT (Debian / Ubuntu)

```sh
# Add the repo (one time)
curl -fsSL https://lucasschimmel.github.io/apt/gpg.key | sudo gpg --dearmor -o /usr/share/keyrings/lucasschimmel.gpg
echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/lucasschimmel.gpg] https://lucasschimmel.github.io/apt stable main" | sudo tee /etc/apt/sources.list.d/lucasschimmel.list

# Install
sudo apt update
sudo apt install muxtop
```

### Pre-built binary (Linux / macOS)

```sh
curl -sSfL https://raw.githubusercontent.com/lucasschimmel/muxtop/main/scripts/install.sh | sh
```

### From source

```sh
git clone https://github.com/lucasschimmel/muxtop.git
cd muxtop
cargo build --release
# Binary available at target/release/muxtop
```

> MSRV: Rust **1.88**

---

## Features

| Feature | Detail |
|---|---|
| **Tabs** | General, Processes, Network, Containers and Kubernetes — `Alt+1` / `Alt+2` / `Alt+3` / `Alt+4` / `Alt+5` |
| **Network tab** | Interface table with RX/s, TX/s, totals, errors + real-time sparklines |
| **Containers tab** | Docker/Podman via [bollard](https://github.com/fussybeaver/bollard) — CPU/memory/network/IO table, CPU+RX sparklines, `F9` stop / `F10` kill / `F11` restart actions, automatic socket detection |
| **Kubernetes tab** | Read-only Pods / Nodes / Deployments via [kube-rs](https://github.com/kube-rs/kube) — switch sub-views with `P` / `N` / `D`, sort with `s`, filter with `/`. Auto-detects `$KUBECONFIG` / `~/.kube/config` / in-cluster ServiceAccount; graceful fallback when `metrics-server` is absent (CPU/MEM render `—`). Lists **cluster-wide** (requires cluster-scoped `list` on pods, nodes and deployments) |
| **Command palette** | `Ctrl+P` — `kill firefox`, `sort memory`, `stop nginx`, `restart postgres`, etc. |
| **htop shortcuts** | `F1`–`F5` sort columns, `F7`/`F8` renice, `F9` kill, `F10` force kill |
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

# Kubernetes tab — by default muxtop reads $KUBECONFIG, then ~/.kube/config,
# then falls back to in-cluster ServiceAccount credentials. Override or disable:
muxtop --kube-context kind-kind               # use a specific kubeconfig context
muxtop --kube-namespace kube-system           # set the displayed default namespace
muxtop --no-kube                              # disable cluster collection entirely

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
| `Alt+1` … `Alt+5` | Switch tab (General / Processes / Network / Containers / Kubernetes) |
| `Tab` / `Shift+Tab` · `←` / `→` | Cycle to the next / previous tab |
| `q` · `Ctrl+C` | Quit |
| `j` / `k` · `↑` / `↓` | Navigation (vim-style) |
| `g` / `G` · `Home` / `End` | Jump to first / last row |
| `PageUp` / `PageDown` | Scroll by 20 rows |
| `/` | Filter (applies to the active tab) |
| `Esc` | Clear the active filter |
| `t` | Tree view (Processes) |
| `s` | Cycle sort field (active tab) |
| `S` / `I` | Toggle sort direction (active tab) |
| `F1` … `F5` | Sort processes by PID / name / CPU / memory / user |
| `F7` / `F8` | Renice — lower (+1) / raise (−1) priority, Processes tab, local mode |
| `F9` | Kill process, SIGTERM (Processes) · Stop container (Containers) |
| `F10` | Force kill, SIGKILL (Processes) · Kill container (Containers) |
| `F11` | Restart container (Containers) |
| `P` / `N` / `D` | Switch Kube sub-view to **P**ods / **N**odes / **D**eployments (Kubernetes tab only) |

> There is no built-in help screen yet — `Ctrl+P` lists every command with its shortcut.

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
    │   ├── src/collector.rs     # 3 async loops: sysinfo 1 Hz, containers 0.5 Hz, cluster 0.2 Hz
    │   ├── src/process.rs       # Sort, filter, process tree
    │   ├── src/system.rs        # CPU / memory / load snapshots
    │   ├── src/network.rs       # Network interfaces + history
    │   ├── src/containers.rs    # Container model (ContainerSnapshot, states, engine)
    │   ├── src/container_engine.rs # Async trait + Docker/Podman socket detection
    │   ├── src/docker_engine.rs # Concrete bollard-backed implementation
    │   ├── src/kube.rs          # Pod / Node / Deployment / Cluster snapshots
    │   ├── src/cluster_engine.rs # Async trait + kubeconfig detection
    │   └── src/kube_engine.rs   # Concrete kube-rs-backed implementation
    ├── muxtop-tui/              # ratatui interface
    │   ├── src/app.rs           # State machine, event handling
    │   └── src/ui/              # Tabs General/Processes/Network/Containers/Kube, palette, theme
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

It makes no outbound network calls of its own. It is designed for air-gapped production servers.
If you observe outbound activity from muxtop that isn't tied to a feature you've enabled, that is a bug — please [report it](https://github.com/lucasschimmel/muxtop/issues).

### Read-only by design

When the Kubernetes or Containers tab is active, muxtop only **reads** from the corresponding API:
- Kubernetes : `LIST` on Pods / Nodes / Deployments + `GET` on `metrics.k8s.io/v1beta1`. No `CREATE` / `UPDATE` / `DELETE` / `PATCH` is ever issued. Write actions (Delete pod, Scale deployment, Rollout restart) are explicitly out of scope for v0.4.
- Containers : `GET /containers/json` + `/stats?stream=false`. The Stop / Kill / Restart actions are gated behind a confirmation dialog and are local-only — disabled in `--remote` mode.

### Remote mode and credentials

In `--remote` mode, the **server** is the only side that opens kubeconfig or Docker socket files. Credentials never traverse the wire — only the digested snapshots do. Anti-leak guards in the test suite (`muxtop-proto/tests/integration.rs`) verify byte-for-byte that no `BEGIN PRIVATE KEY`, `Bearer ` token, or `client-key-data:` field appears in any encoded frame.

---

## Contributing

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for prerequisites, code conventions, the branch workflow and PR submission instructions.

---

## License

Available under either of the following licenses, at your option:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))
