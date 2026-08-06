# Performance

muxtop's premise is that a monitor which costs noticeable resources is a monitor
that changes what it measures. The budgets below are treated as constraints, not
aspirations — a feature that breaks one does not ship until it stops breaking it.

## Published figures

Measured on macOS with 500+ processes, via the repository's own benchmark:

| Metric | Budget | Measured |
|---|---|---|
| Startup (`--about`) | < 100 ms | ~12 ms |
| Binary size | < 10 MiB | **5.3 MiB** (LTO + strip) |
| FPS (TUI) | > 30 | ~60, event-driven — idle ≈ 0 redraws |
| Peak RSS (30 s) | < 15 MiB | **11.3 MiB** (htop ≈ 15, btop ≈ 40) |

```sh
just bench-thomas       # macro: build, startup, RSS
just bench              # criterion micro-benchmarks
just bench-alloc        # dhat heap profile of the hot paths
```

Numbers move with hardware, process count and terminal. Reproduce them on your
own box before comparing.

## Why it is cheap

**Rendering is event-driven, not frame-driven.** An idle muxtop does
approximately zero redraws — it repaints on a key press, a mouse event, a
terminal resize or a new snapshot, and otherwise sleeps. This is where most of
the difference against a fixed-FPS TUI comes from.

**Collection runs on four independent loops**, each at the rate its data source
deserves:

| Loop | Rate | Why |
|---|---|---|
| System / processes | 1 Hz | `sysinfo`, cheap, and the numbers change every second |
| Containers | 0.5 Hz | The engine computes stats per container — the expensive one |
| Cluster | 0.2 Hz | API server round trips are slow and the data is slow-moving |
| GPU | 1 Hz | NVML queries and sysfs reads are cheap |

Nothing blocks the UI: collection is `tokio` tasks, and the render loop reads the
latest snapshot rather than waiting for one. A cluster that stops responding
freezes its own tab's data, not your keyboard.

**The release profile is tuned for size and speed**: `lto = "fat"`,
`codegen-units = 1`, `strip = "symbols"`, `panic = "abort"`. That is what turns a
multi-megabyte debug build into a 5.3 MiB static binary.

---

## Tuning on a large host

### Poll less often

```sh
muxtop --refresh 5      # seconds, 1–3600
```

This is the single biggest lever. On a host you are watching rather than
debugging, 5 or 10 seconds is plenty, and it scales down every loop.

### Drop what you are not looking at

```sh
muxtop --no-containers    # skip container stats — usually the most expensive
muxtop --no-kube          # skip API server polling
muxtop --no-gpu           # skip the NVML / sysfs probe
```

Each flag removes both the startup probe and the polling loop. On a host with
2000 containers, `--no-containers` is dramatic.

### On thousands of processes

The process table is sorted and filtered on every snapshot. At 3000+ processes:

- Start filtered — `--filter <pattern>` — so the table you sort is smaller
- Tree view (`t`) costs more than the flat list: it builds the parent/child
  hierarchy each cycle
- Raise `--refresh`

muxtop is designed to stay responsive at 3000+ processes. If it does not on your
host, that is a bug worth
[reporting](https://github.com/lucasschimmel/muxtop/issues) with the process
count and the profile.

### On the server side

`muxtop-server` pays the collection cost *and* the serialisation cost of every
snapshot, for every connected client:

```sh
muxtop-server --refresh 5 --max-clients 4 --token-file /etc/muxtop/token --tls-generate
```

`--max-clients` (default 8) bounds the multiplier. Disabling a data source on the
server disables it for everyone, which is usually what you want on a monitoring
host.

---

## Measuring for yourself

```sh
# Startup, the isolated path
time muxtop --about

# Peak RSS over 30 s (Linux)
/usr/bin/time -v muxtop --refresh 1        # look at "Maximum resident set size"

# Peak RSS (macOS)
/usr/bin/time -l muxtop --refresh 1        # look at "maximum resident set size"

# Where the allocations are
just bench-alloc                            # dhat profile of the render hot path
```

If you file a performance issue, the useful numbers are: process count
(`ps aux | wc -l`), container count, whether the Kube and GPU tabs are active,
`--refresh`, and the RSS you measured.

---

## The constraints, for contributors

Any change must keep:

| | |
|---|---|
| Peak RSS (30 s) | < 13 MiB |
| Binary size | < 7 MiB |
| Startup (`--about`) | < 30 ms |

Those are tighter than the published budgets on purpose — they are the ceilings
the current numbers are allowed to drift toward, not targets to grow into. A
feature that needs more argues for itself in the pull request.
