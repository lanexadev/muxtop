# Troubleshooting

Start here, then follow the link into the tab-specific page.

## First: where the evidence is

muxtop prints nothing to the terminal — the terminal is the UI. Warnings and
errors go to a log file:

| Platform | Path |
|---|---|
| Linux | `~/.local/share/muxtop/muxtop.log` |
| macOS | `~/Library/Application Support/muxtop/muxtop.log` |
| server | same directory, `muxtop-server.log` |

Default level is `warn`. For much more:

```sh
MUXTOP_LOG=debug muxtop
MUXTOP_LOG=muxtop_core=trace,muxtop_tui=debug muxtop   # per-crate
```

`MUXTOP_LOG` takes the standard `tracing` filter syntax. Logs can contain
process command lines, container names and hostnames — read before pasting one
into an issue.

Also useful:

```sh
muxtop --version
muxtop --about        # version, license, repository, privacy pledge
echo "$TERM / $COLORTERM"
```

---

## Display and colours

| Symptom | Cause and fix |
|---|---|
| No colour at all | `NO_COLOR` is set in the environment, `TERM=dumb`, or you passed `--no-color`. Hierarchy falls back to bold / dim / reverse |
| Colours look wrong or washed out | The terminal reports fewer than 24 bits. muxtop uses a 256-colour or 16-colour rendition of Tokyo Night — set `COLORTERM=truecolor` if your terminal really does support it |
| Colours wrong inside tmux | tmux needs telling: `set -g default-terminal "tmux-256color"` and `set -as terminal-features ",*:RGB"` |
| Blocks and sparklines render as garbage | No UTF-8 locale, or the Linux console font has no block glyphs. muxtop switches to ASCII automatically; force it with `--ascii` |
| Unreadable on a light background | `muxtop --theme tokyo-night-light`, or `--theme mono` |
| `?` characters inside a process or container name | The source string contained control bytes and the sanitizer replaced them. Working as intended — see [Security model](Security-model) |
| Table columns cut off | Scroll them with `h` / `l`, or press `Enter` to inspect the row and see everything |
| Layout broken after resizing | Should not happen — please [report it](https://github.com/lucasschimmel/muxtop/issues) with your terminal and dimensions |

## Mouse and clipboard

| Symptom | Cause and fix |
|---|---|
| Cannot select text with the mouse | Mouse capture owns the events. `--no-mouse` gives selection back; every gesture has a keyboard equivalent |
| Mouse does nothing | No pointer detected (`TERM=linux`, `TERM=dumb`) — deliberate |
| `y` does not reach my clipboard | `y` uses OSC 52. Your terminal must allow it — kitty, iTerm2, WezTerm and foot do; some need it enabled, and tmux needs `set -g set-clipboard on` |

## Processes

| Symptom | Cause and fix |
|---|---|
| Other users' processes missing | `/proc` is mounted with `hidepid=1`/`hidepid=2`, or you are inside a container with its own PID namespace |
| `F9` / `F10` do nothing | You are on the wrong tab (they are Processes/Containers-scoped), or in `--remote` mode where they are disabled |
| Kill says permission denied | You do not own the process. muxtop does not escalate |
| `F8` (raise priority) fails | Lowering a nice value needs privilege the kernel does not give unprivileged users, even to undo your own change |
| Kill or renice unsupported on Windows | Correct — POSIX-only, and the stub says so rather than pretending |
| Tree view shows odd parents | Reparented orphans (`PPID 1`) after their parent exited. That is the real hierarchy |

## Startup and CLI

| Symptom | Cause and fix |
|---|---|
| `failed to create muxtop data directory` | No writable `HOME` / `XDG_DATA_HOME`. Common under systemd — see [Remote monitoring](Remote-monitoring) |
| `command not found` after `install.sh` | It installed to `~/.local/bin`, which is not on your `PATH` |
| `cargo check --workspace` fails on a fresh clone | Expected — needs `--all-targets` for the `k8s-openapi` feature gate. Use `just check` |
| Startup slower than the advertised ~12 ms | `--about` is the measured path. A full launch also probes container, cluster and GPU backends; `--no-containers --no-kube --no-gpu` isolates which one is slow |

## High CPU or memory

See **[Performance](Performance)** for the full treatment. The short version:

```sh
muxtop --refresh 5                             # poll less often
muxtop --no-containers --no-kube --no-gpu      # drop data sources
```

Container stats are usually the expensive one — the engine computes them per
container.

## Tab-specific

- Containers tab empty or *"no engine configured"* → **[Containers](Containers)**
- Kubernetes nodes empty, CPU/MEM `—`, *"No cluster"* → **[Kubernetes](Kubernetes)**
- GPU tab empty, columns `—` → **[GPU](GPU)**
- Remote connection refused, TLS errors → **[Remote monitoring](Remote-monitoring)**

---

## Still stuck

[Open an issue](https://github.com/lucasschimmel/muxtop/issues/new/choose). The
bug template asks for version, platform, terminal, `$TERM`, local vs remote and
which tab — those six answers are what make a report reproducible instead of a
conversation.

If it is a **vulnerability**, use [private
reporting](https://github.com/lucasschimmel/muxtop/security/advisories/new)
instead.
