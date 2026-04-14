# muxtop

**A modern, multiplexed system monitor for the terminal.**

[![CI](https://github.com/lanexadev/muxtop/actions/workflows/ci.yml/badge.svg)](https://github.com/lanexadev/muxtop/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/muxtop.svg)](https://crates.io/crates/muxtop)
[![License](https://img.shields.io/crates/l/muxtop.svg)](LICENSE-MIT)

muxtop replaces the `htop` + `iftop` + `ctop` workflow with a single tabbed interface.
Think htop, but with a multiplexer UX (like tmux/zellij) and a VS Code-style command palette.

## Install

### From crates.io

```sh
cargo install muxtop
```

### From binary (Linux / macOS)

```sh
curl -sSfL https://raw.githubusercontent.com/lanexadev/muxtop/main/scripts/install.sh | sh
```

### From source

```sh
git clone https://github.com/lanexadev/muxtop.git
cd muxtop
cargo build --release
# Binary at target/release/muxtop
```

## Features

- **Tabbed interface** — General and Processes tabs, switch with `Alt+1`/`Alt+2`
- **Command palette** — `Ctrl+P` to open, type `kill firefox` or `sort memory` in natural language
- **htop-compatible keybindings** — `F3` search, `F4` filter, `F5` tree view, `F6` sort, `F9` kill, `F10` quit
- **Fuzzy search** — powered by [nucleo](https://github.com/helix-editor/nucleo) (from the Helix editor)
- **Process tree view** — `F5` toggles hierarchical parent/child display
- **Async data collection** — tokio-based collector never blocks the UI, even at 3000+ processes
- **Zero dependencies** — single static musl binary, deploys like ripgrep
- **Zero telemetry** — no network calls, ever (see [Privacy](#privacy--telemetry))

## Benchmarks

Tested on macOS with 500+ processes (Thomas benchmark):

| Metric | Target | muxtop |
|--------|--------|--------|
| Startup (`--about`) | < 100ms | ~5ms |
| Binary size | < 10 MB | ~4 MB |
| FPS (TUI) | > 30 | ~60 |
| RAM | < 10 MB | < 8 MB |

Run the benchmark yourself:

```sh
just bench-thomas
# or
./scripts/bench-thomas.sh
```

## Usage

```sh
muxtop                          # launch normally
muxtop --refresh 2              # refresh every 2 seconds
muxtop --filter firefox         # start with a process filter
muxtop --sort mem               # sort by memory
muxtop --tree                   # start in tree view
muxtop --about                  # version, license, privacy pledge
```

### Key bindings

| Key | Action |
|-----|--------|
| `Ctrl+P` | Command palette |
| `Alt+1` / `Alt+2` | Switch tabs |
| `F1` | Help |
| `F3` / `/` | Search |
| `F4` | Filter processes |
| `F5` | Toggle tree view |
| `F6` | Sort menu |
| `F9` | Kill process |
| `F10` / `q` | Quit |
| `j` / `k` | Navigate (vim-style) |

## Roadmap

| Version | Focus |
|---------|-------|
| **v0.1** | htop replacement — tabs, command palette, tree view |
| v0.2 | Network tab (replaces iftop) + client-server architecture |
| v0.3 | Containers tab (Docker/Podman/K8s) + GPU monitoring |
| v1.0 | WASM plugin system + themes + config file |

## Privacy & Telemetry

muxtop collects **NO** telemetry, **NO** analytics, and phones home to **NOBODY**. Ever.

It makes zero network calls. It is designed for air-gapped production servers.
If you observe any outbound network activity from muxtop, it is a bug — please [report it](https://github.com/lanexadev/muxtop/issues).

## Contributing

Contributions are welcome. Please open an issue first to discuss what you'd like to change.

```sh
just check    # fmt + clippy + test
just bench    # criterion micro-benchmarks
just dev      # continuous checking with bacon
```

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
