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

> **Platforms.** Linux and macOS (x86_64 and aarch64) are the supported targets — those are what the release pipeline publishes. The workspace also builds and passes its tests on Windows so the project can be developed there, but no Windows binaries are published and the process actions (`F7`/`F8` renice, `F9`/`F10` kill) are POSIX-only and return an error.

---

## Features

| Feature | Detail |
|---|---|
| **Tabs** | General, Processes, Network, Containers, Kubernetes and GPU — `Alt+1` … `Alt+6` |
| **Network tab** | Interface table with RX/s, TX/s, totals, errors + real-time sparklines |
| **Containers tab** | Docker/Podman via [bollard](https://github.com/fussybeaver/bollard) — CPU/memory/network/IO table, CPU+RX sparklines, `F9` stop / `F10` kill / `F11` restart actions, automatic socket detection |
| **Kubernetes tab** | Read-only Pods / Nodes / Deployments via [kube-rs](https://github.com/kube-rs/kube) — switch sub-views with `P` / `N` / `D`, sort with `s`, filter with `/`. Auto-detects `$KUBECONFIG` / `~/.kube/config` / in-cluster ServiceAccount; graceful fallback when `metrics-server` is absent (CPU/MEM render `—`). Lists cluster-wide by default; `--kube-namespace <NS>` scopes Pods and Deployments to one namespace so a namespace-bound Role is enough, and `A` toggles between the two at runtime (see [Kubernetes permissions](#kubernetes-permissions)) |
| **GPU tab** | Read-only NVIDIA (via NVML) and AMD (via `amdgpu` sysfs) — utilisation, VRAM, temperature, power, clocks, fan, NVENC/NVDEC. Switch sub-views with `D` / `P`, sort with `s`, filter with `/`. Multiple vendors are merged into one list, and any metric a driver cannot report renders `—` rather than a misleading `0` (see [GPU support](#gpu-support)) |
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

## Kubernetes permissions

By default muxtop lists Pods, Nodes and Deployments across every namespace, which needs cluster-scoped `list` on all three. On a shared cluster you usually don't have that.

Pass `--kube-namespace <NS>` to scope Pods and Deployments to a single namespace — that works with a plain `Role` bound to it, no cluster-wide grant required. Press `A` in the Kube tab to switch between the scoped and cluster-wide views at runtime (local mode only; in `--remote` mode the server's `--kube-namespace` decides).

**Nodes are cluster-scoped in Kubernetes** — there is no namespaced variant of the resource, so the Nodes sub-view always needs cluster-wide access and renders empty without it. The Pods and Deployments views are unaffected. The same split applies to metrics: pod CPU/MEM follows the namespace scope, node CPU/MEM does not.

A minimal namespace-scoped Role:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: muxtop-readonly
  namespace: my-namespace
rules:
  - apiGroups: [""]
    resources: ["pods"]
    verbs: ["list"]
  - apiGroups: ["apps"]
    resources: ["deployments"]
    verbs: ["list"]
  - apiGroups: ["metrics.k8s.io"]
    resources: ["pods"]
    verbs: ["list"]
```

---

## GPU support

muxtop probes for GPUs at startup and needs **no extra privileges** — NVML is
readable by any user, and the `amdgpu` sysfs nodes are world-readable. Nothing
is installed and no vendor SDK is required at build time: the NVIDIA library is
loaded dynamically at runtime, so the same binary runs on machines with and
without an NVIDIA driver. Disable the probe with `--no-gpu`.

| Vendor | Backend | Platforms | Status |
|---|---|---|---|
| **NVIDIA** | NVML (`libnvidia-ml.so` / `nvml.dll`, loaded at runtime) | Linux, Windows | Full — including per-process usage |
| **AMD** | `amdgpu` sysfs (`/sys/class/drm/card*/device`) | Linux | Devices only — no per-process usage |
| **Intel** | — | — | Not implemented |
| **Apple Silicon** | IOReport | macOS | **Planned for v0.6** |

### What each backend can and cannot report

Unlike CPU or memory, no GPU metric is universally available, so every field is
optional and the UI renders an unavailable one as `—`. **A `—` means "this
driver cannot report this", not "zero"** — conflating the two would make the tab
lie about an idle GPU.

- **AMD has no per-process accounting.** The `amdgpu` sysfs interface exposes no
  equivalent of NVML's process queries, so the Procs sub-view says so explicitly
  instead of showing an empty list that reads as "nothing is using the GPU".
- **Encoder/decoder utilisation is NVML-only.** AMD renders `—` for that column.
- **On Windows, per-process GPU memory is unavailable and every process shows
  `both`.** Under the WDDM driver model the OS owns video-memory allocation, so
  NVML reports the per-process figure as unavailable; it also returns identical
  compute and graphics process lists, so the TYPE column carries no information
  there. Both behaviours are the driver's, and both are reported honestly rather
  than papered over. On Linux the figures and the distinction are real.
- **Fanless cards report no fan**, and a laptop dGPU parked in runtime-D3 may
  report nothing at all until something wakes it.

### Why Apple Silicon is not in this release

macOS exposes GPU counters only through the private `IOReport` framework — the
same source `powermetrics` reads, and `powermetrics` requires root. Shipping
that in v0.5 would have meant either taking a private-framework dependency or
asking every macOS user to run muxtop as root, neither of which fits a tool
whose premise is staying out of the way. It is scheduled for **v0.6**; the data
model, the wire format and the tab already accommodate it.

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
muxtop --kube-namespace kube-system           # scope Pods/Deployments to one namespace
muxtop --no-kube                              # disable cluster collection entirely

# GPU tab — NVIDIA (NVML) and AMD (amdgpu sysfs) are probed automatically.
muxtop --no-gpu                               # disable GPU detection entirely

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
| `Alt+1` … `Alt+6` | Switch tab (General / Processes / Network / Containers / Kubernetes / GPU) |
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
| `A` | Toggle namespace scope — one namespace ↔ **A**ll namespaces (Kubernetes tab, local mode) |
| `D` / `P` | Switch GPU sub-view to **D**evices / **P**rocs (GPU tab only) |

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
    │   ├── src/collector.rs     # 4 async loops: sysinfo 1 Hz, containers 0.5 Hz, cluster 0.2 Hz, gpu 1 Hz
    │   ├── src/process.rs       # Sort, filter, process tree
    │   ├── src/system.rs        # CPU / memory / load snapshots
    │   ├── src/network.rs       # Network interfaces + history
    │   ├── src/containers.rs    # Container model (ContainerSnapshot, states, engine)
    │   ├── src/container_engine.rs # Async trait + Docker/Podman socket detection
    │   ├── src/docker_engine.rs # Concrete bollard-backed implementation
    │   ├── src/kube.rs          # Pod / Node / Deployment / Cluster snapshots
    │   ├── src/cluster_engine.rs # Async trait + kubeconfig detection
    │   ├── src/kube_engine.rs   # Concrete kube-rs-backed implementation
    │   ├── src/gpu.rs           # GPU device / process snapshots (every metric optional)
    │   ├── src/gpu_engine.rs    # Async trait + composite that merges vendor backends
    │   ├── src/nvml_engine.rs   # NVIDIA backend (NVML, dynamically loaded)
    │   └── src/amd_engine.rs    # AMD backend (amdgpu sysfs)
    ├── muxtop-tui/              # ratatui interface
    │   ├── src/app.rs           # State machine, event handling
    │   └── src/ui/              # Tabs General/Processes/Network/Containers/Kube/GPU, palette, theme
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
| **v0.5** ✓ | GPU tab — NVIDIA via NVML, AMD via `amdgpu` sysfs, per-process usage, graceful per-metric degradation |
| v0.6 | Apple Silicon GPU support (IOReport) + interactive `docker exec` (PTY) |
| v1.0 | WASM plugin system + themes + configuration file |

---

## Privacy & telemetry

muxtop collects **NO** telemetry, **NO** statistics and contacts **NO ONE**. Ever.

It makes no outbound network calls of its own. It is designed for air-gapped production servers.
If you observe outbound activity from muxtop that isn't tied to a feature you've enabled, that is a bug — please [report it](https://github.com/lucasschimmel/muxtop/issues).

### Read-only by design

When the Kubernetes or Containers tab is active, muxtop only **reads** from the corresponding API:
- Kubernetes : `LIST` on Pods / Nodes / Deployments + `GET` on `metrics.k8s.io/v1beta1`, scoped to one namespace or cluster-wide per [Kubernetes permissions](#kubernetes-permissions). No `CREATE` / `UPDATE` / `DELETE` / `PATCH` is ever issued. Write actions (Delete pod, Scale deployment, Rollout restart) are explicitly out of scope for v0.4.
- Containers : `GET /containers/json` + `/stats?stream=false`. The Stop / Kill / Restart actions are gated behind a confirmation dialog and are local-only — disabled in `--remote` mode.
- GPU : NVML query calls and reads of `/sys/class/drm/card*/device`. Both are read-only by construction — muxtop never sets a clock, a power limit or a fan curve, and the GPU tab has no actions at all. `--no-gpu` skips the probe entirely.

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
