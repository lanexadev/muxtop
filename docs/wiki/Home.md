# muxtop wiki

A modern, multiplexed system monitor for the terminal — htop, iftop, ctop and
`nvidia-smi` in one tabbed interface, in a 5 MiB static binary that makes no
network calls of its own.

> **This wiki is generated.** Its source lives in
> [`docs/wiki/`](https://github.com/lucasschimmel/muxtop/tree/main/docs/wiki) and
> is mirrored here on every release. Editing a page in the browser works right
> up until the next sync overwrites it — send a pull request against
> `docs/wiki/` instead.

---

## Where things are

The [README](https://github.com/lucasschimmel/muxtop#readme) is the tour: what
muxtop is, how to install it, the feature table, the keybinding summary. It is
the right place to start and this wiki does not repeat it.

The wiki is for what a README cannot hold — the operational detail you need
once muxtop is running on a machine that matters.

| Page | Read it when |
|---|---|
| **[Installation](Installation)** | Choosing between five install methods, verifying a download, pinning a version, uninstalling |
| **[Keyboard reference](Keyboard-reference)** | You want the complete keymap, including what is tab-scoped and what is disabled over `--remote` |
| **[Remote monitoring](Remote-monitoring)** | Running `muxtop-server` on a real host: certificates, tokens, systemd, firewalls |
| **[Security model](Security-model)** | Deciding whether to expose the server, or reviewing muxtop before deploying it |
| **[Containers](Containers)** | The Containers tab is empty, or you want it without granting root-equivalent access |
| **[Kubernetes](Kubernetes)** | Pods list but nodes don't, CPU/MEM show `—`, or you need a minimal RBAC role |
| **[GPU](GPU)** | Your card is missing, or a metric renders `—` and you want to know whether that means zero |
| **[Troubleshooting](Troubleshooting)** | Something is wrong and you want the symptom → cause table |
| **[Performance](Performance)** | Tuning refresh rates on a big host, or reproducing the published benchmarks |
| **[Architecture](Architecture)** | Contributing, or curious how the collection loops and the remote protocol fit together |
| **[Release process](Release-process)** | You maintain a fork, or want to know what a release actually does |
| **[FAQ](FAQ)** | Short answers, including the ones that are "no, and here is why" |

---

## Three things worth knowing up front

**It contacts no one.** muxtop makes no outbound network calls of its own — no
telemetry, no update check, no crash reporting. The only sockets it opens are
the ones a feature you enabled needs: a container socket, a Kubernetes API
server, or the `--remote` connection you asked for. It is meant to be usable on
an air-gapped host. If you see traffic that is not one of those, that is a bug
worth [reporting](https://github.com/lucasschimmel/muxtop/issues).

**It degrades honestly.** A metric the driver or the API cannot report renders
`—`, never `0`. On a Linux console it drops to ASCII glyphs; with `NO_COLOR`
set it uses bold and reverse instead of hue; without a pointer it leaves mouse
reporting off. A monitor that lies about an idle GPU is worse than no GPU tab.

**It reads, with two exceptions.** Kubernetes and GPU access is read-only by
construction — no `CREATE`/`UPDATE`/`DELETE`/`PATCH`, no clock or fan writes.
The two things muxtop can change are process state (kill, renice) and container
state (stop, kill, restart), both behind a confirmation dialog and both
disabled in `--remote` mode.

---

## Getting help

- **A bug or a feature idea** → [issues](https://github.com/lucasschimmel/muxtop/issues);
  the templates ask for the terminal and platform details that make a report
  reproducible.
- **A vulnerability** → [private security
  advisory](https://github.com/lucasschimmel/muxtop/security/advisories/new),
  never a public issue. See
  [SECURITY.md](https://github.com/lucasschimmel/muxtop/blob/main/SECURITY.md).
- **Contributing** →
  [CONTRIBUTING.md](https://github.com/lucasschimmel/muxtop/blob/main/CONTRIBUTING.md).
