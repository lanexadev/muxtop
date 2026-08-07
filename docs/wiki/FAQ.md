# FAQ

## Why another system monitor?

Because the workflow it replaces is three or four programs. Watching a host
usually means htop in one pane, `iftop` in another, `ctop` or `docker stats` in a
third, and `nvidia-smi` on a loop in a fourth. muxtop puts them in one tabbed
interface with a shared keymap, a command palette, and one 5 MiB binary to
install.

## How is it different from btop or bottom?

Mostly in scope and in cost. btop is beautiful and uses about 40 MiB of RSS;
muxtop uses about 11. Neither btop nor bottom covers containers, Kubernetes and
remote monitoring over TLS. muxtop's differentiators are the **Containers,
Kubernetes and GPU tabs**, the **command palette**, and **`--remote`**. If you
want the prettiest local monitor, btop is a fine answer.

## Does it phone home?

No. muxtop makes **no outbound network calls of its own** — no telemetry, no
update check, no crash reporting, no analytics, ever. The only sockets it opens
belong to a feature you enabled: a container socket, a Kubernetes API server, or
the `--remote` connection you asked for. It is meant to be usable on an
air-gapped host. If you see anything else,
[report it](https://github.com/lucasschimmel/muxtop/issues) — that would be a
bug.

## Does it need root?

No, and you should not. Unprivileged muxtop reads `/proc`, NVML and sysfs
happily. Root only widens what you can *kill* and *renice*, and `/proc` is
world-readable by default so the view barely changes.

The one thing that does need privilege is a container socket — and adding
yourself to the `docker` group is root-equivalent. Use rootless Podman instead.

## Why does a column show `—` instead of a number?

Because the source could not report it, and `0` would be a lie. A `—` on a GPU
temperature means the driver did not answer; a `—` in the Kube CPU column means
the cluster has no metrics-server. Both are different from "zero", and
conflating them would make the tab claim a busy GPU is idle. See
[GPU](GPU) and [Kubernetes](Kubernetes).

## Why is there a `?` in a process name?

The name contained an ASCII control byte and the sanitizer replaced it. A
terminal interprets escape sequences, so a hostile container label could
otherwise rewrite your window title or worse. See
[Security model](Security-model).

## Is there a configuration file?

Not yet — it is scheduled for **v1.0**, with the WASM plugin system and
user themes. Today everything is CLI flags: `--theme`, `--refresh`, `--sort`,
`--filter`, `--tree`, `--no-color`, `--ascii`, `--no-mouse`. A shell alias covers
most of what a config file would.

## Does it support Windows?

It **compiles and passes its tests** on Windows, so you can develop muxtop there,
but no Windows binaries are published and process actions (kill, renice) are
POSIX-only stubs that return an error. Windows is in CI as a regression guard,
not as a support claim. WSL works properly.

## Why is the Kubernetes tab read-only?

So that handing muxtop a credential is a decision you can make without thinking
about it. "Read-only by construction" is a claim you can verify by grepping for
the verbs it issues — `list` and `get`, nothing else. Delete pod, scale
deployment and rollout restart are explicitly out of scope.

Containers are the exception: stop, kill and restart exist because they are the
actions people actually reach for during an incident. They ask for confirmation
and are disabled over `--remote`.

## Can I use it to monitor several servers at once?

One `muxtop --remote` connects to one server. There is no multi-host dashboard —
open several muxtops in a tmux layout, which is also what the multiplexer-style
UX is designed for.

## Is `--remote` safe to expose to the internet?

It is TLS 1.3-only with a mandatory ≥16-character token, a per-IP rate limit and
a client cap. But the strongest answer is not to expose it: bind to `127.0.0.1`
(the default) and reach it through an ssh tunnel, which adds ssh's
authentication and opens no new port. See
[Remote monitoring](Remote-monitoring) and [Security model](Security-model).

## Why is `cargo check` failing on a fresh clone?

`cargo check --workspace` alone fails; you need `cargo check --workspace
--all-targets`, or just `just check`. `k8s-openapi` refuses to compile a library
crate without a `v1_*` feature and equally refuses to let library crates enable
one, so only the leaf binaries and the dev-dependencies enable it — and
`--all-targets` is what activates dev-dependencies. Full explanation in
[Architecture](Architecture).

## Why bincode when it is marked unmaintained?

RUSTSEC-2025-0141 marks bincode v2 unmaintained, and `deny.toml` documents the
exception: the maintainers consider v2 complete, muxtop uses it for its own wire
protocol between two versions it controls, and alternatives (postcard, bitcode)
are on the table for a future release. The ignore entry is a stated decision with
a reason, which is the only kind of ignore entry worth having.

## How do I know the binary I downloaded is genuine?

Verify the checksum, then the signature:

```sh
shasum -a 256 -c muxtop-x86_64-unknown-linux-musl.tar.gz.sha256
gh attestation verify muxtop-x86_64-unknown-linux-musl.tar.gz \
  --repo lucasschimmel/muxtop
```

The checksum catches corruption; the attestation proves the archive came out of
this repository's release workflow, which a checksum cannot — someone who can
replace the archive can replace the `.sha256` beside it. See
[Installation](Installation).

## What is on the roadmap?

| | |
|---|---|
| **v0.7** ✓ | Apple Silicon GPU support (IOKit + `IOReport`, no root) |
| **v0.8** | Interactive `docker exec` with a PTY |
| **v1.0** | WASM plugin system, user themes, configuration file |

The [README roadmap](https://github.com/lucasschimmel/muxtop#roadmap) is
authoritative.

## How can I help?

Read [CONTRIBUTING.md](https://github.com/lucasschimmel/muxtop/blob/main/CONTRIBUTING.md).
The most useful contributions right now are **bug reports from unusual
terminals** — the layout and colour code has a long tail of platforms nobody has
tried. A report that says "this renders wrong in this terminal at these
dimensions" is worth a lot.

## I found a security problem.

[Report it privately](https://github.com/lucasschimmel/muxtop/security/advisories/new),
never as a public issue. Scope and response targets are in
[SECURITY.md](https://github.com/lucasschimmel/muxtop/blob/main/SECURITY.md).
