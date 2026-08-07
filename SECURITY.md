# Security Policy

muxtop reads process tables, container APIs, Kubernetes clusters and GPU
telemetry, and `muxtop-server` accepts connections from the network. That
combination deserves a policy that says something concrete, so this document
states what is defended, what is not, and how to report a hole.

---

## Reporting a vulnerability

**Report privately, through GitHub:**

➡️ **[Open a private security advisory](https://github.com/lucasschimmel/muxtop/security/advisories/new)**

That form is visible only to the maintainers. Please do **not** open a public
issue, a pull request, or a discussion for a suspected vulnerability — a public
report is a disclosure, and it starts the clock for every muxtop user at once.

A useful report contains:

| | |
|---|---|
| **Version** | output of `muxtop --version` (and `muxtop-server --version` if relevant) |
| **Platform** | OS, architecture, and how muxtop was installed |
| **Mode** | local, or `--remote` — and if remote, which TLS flags were in use |
| **Impact** | what an attacker gains: code execution, credential disclosure, privilege escalation, crash |
| **Reproduction** | the smallest sequence of steps, commands or frames that triggers it |
| **Attacker position** | unauthenticated on the network, authenticated client, local unprivileged user, malicious container name, hostile terminal |

That last row matters more than it looks. muxtop's interesting boundaries are
crossed by data, not by users — a container label, a process command line or a
protocol frame is attacker-controlled input in a way a CLI flag is not.

### What to expect

| Stage | Target |
|---|---|
| Acknowledgement | within **3 working days** |
| Initial assessment (in scope? severity?) | within **7 working days** |
| Fix or documented mitigation for a confirmed high-severity issue | within **30 days** |
| Public advisory | after a fix ships, or **90 days** after the report, whichever comes first |

muxtop is maintained by one person and there is **no bug bounty** — no money,
no swag. What you get is credit in the advisory and the CHANGELOG, unless you
ask to stay anonymous. If a deadline above is going to slip, you will be told
why rather than left waiting.

---

## Supported versions

muxtop is pre-1.0 and there are no long-term support branches. Security fixes
land on `develop` and ship in the next release.

| Version | Supported |
|---|---|
| latest release | ✅ |
| anything older | ❌ — upgrade |

Check what you are running with `muxtop --version`, and verify a downloaded
release before trusting it:

```sh
# Checksum published beside the archive
shasum -a 256 -c muxtop-x86_64-unknown-linux-musl.tar.gz.sha256

# Stronger: cryptographic proof the archive came from this repository's
# release workflow, not from someone who replaced both files
gh attestation verify muxtop-x86_64-unknown-linux-musl.tar.gz \
  --repo lucasschimmel/muxtop
```

---

## Scope

### In scope

Anything that lets an attacker do something the design says they cannot:

- **Remote protocol** (`muxtop-proto`, `muxtop-server`) — memory-safety or
  panic-on-malformed-frame in the decoder, unbounded allocation driven by a
  length field, authentication bypass, token comparison weaknesses, TLS
  misconfiguration, rate-limiter bypass or exhaustion.
- **Credential confinement** — any path by which kubeconfig contents, a bearer
  token, a TLS private key or the auth token reaches the wire, a log file, a
  panic message, `Debug` output, or a remote client.
- **Terminal injection** — a process name, container label, pod name or
  interface name that escapes the ANSI sanitizer and reaches the terminal as a
  control sequence.
- **Process actions** — any way to make `kill` or `setpriority` hit a PID other
  than the one displayed and confirmed, or to reach a signal outside the
  permitted set.
- **Privilege handling** — permissions on generated TLS keys, symlink or TOCTOU
  races on certificate and token file reads, unsafe temporary files.
- **Read-only guarantees** — any `CREATE` / `UPDATE` / `DELETE` / `PATCH`
  issued against a Kubernetes cluster, or any GPU state written.
- **Supply chain** — a compromise path through the release workflow, the
  published crates, or the installer script.

### Out of scope

These are known, documented properties, not vulnerabilities:

- **Docker socket access is root-equivalent.** Anyone who can reach
  `/var/run/docker.sock` can already own the host. muxtop documents this and
  recommends rootless Podman; it is not a bug that muxtop can use the socket
  it was pointed at.
- **`--tls-skip-verify` disables certificate verification.** It says so, in the
  flag name and in the help text, and it exists for development.
- **Running muxtop or muxtop-server as root grants root's view.** Don't.
- **An operator who already has the auth token can read the metrics.** That is
  what the token is for.
- **Local resource use** — a 1 Hz refresh on a 5000-process host costs CPU.
  A pathological case worth fixing is a performance bug; report it as an issue.
- **Advisories in dependencies with no reachable path in muxtop.** Report those
  upstream. If you *can* show the path, that is in scope — say so.
- **Missing hardening with no exploit path** (a compiler flag not set, a header
  not sent). Welcome as an issue, not as an advisory.

---

## Security model in one page

Trust boundaries, from the outside in:

1. **The network → `muxtop-server`.** TLS 1.3 only, a mandatory auth token of
   at least 16 characters, a per-source-IP connection rate limit
   (`--rate-limit-per-ip`, default 10/s) and a hard client cap
   (`--max-clients`, default 8). An unauthenticated peer should be able to
   cause nothing but its own disconnection.
2. **`muxtop-server` → the client.** Only digested snapshots cross the wire.
   Credentials never do: the server is the only side that opens a kubeconfig
   or a container socket, and the test suite asserts byte-for-byte that no
   `BEGIN PRIVATE KEY`, `Bearer `, or `client-key-data:` appears in an encoded
   frame.
3. **Collected data → the terminal.** Process names, container labels and pod
   names are attacker-controlled strings. They pass through an ANSI sanitizer
   before rendering, because a terminal that interprets them is a terminal an
   attacker can drive.
4. **The user → the host.** Kill, force-kill and renice are gated behind a
   confirmation dialog, restricted to a fixed signal set, and disabled in
   `--remote` mode. Kubernetes and GPU access is read-only by construction.

The full write-up, with the deployment hardening checklist, lives in the wiki:
**[Security model](https://github.com/lucasschimmel/muxtop/wiki/Security-model)**.

---

## Hardening in the pipeline

Because the supply chain is part of the attack surface:

- Every third-party GitHub Action is pinned to a **commit SHA**, so a re-pointed
  tag cannot inject code. Dependabot moves those SHAs forward under review.
- Workflow tokens are **read-only by default**; write scopes are granted per
  job.
- `cargo deny` runs in CI **and** on a daily schedule, so an advisory published
  against an already-shipped dependency opens an issue instead of waiting for
  the next commit.
- CodeQL analyses the workspace on every push and weekly.
- Release binaries carry a **build-provenance attestation** verifiable with
  `gh attestation verify`.
- Releases are gated on the tag matching the workspace version and on a
  CHANGELOG entry existing.

---

## Thank you

Reporting a vulnerability responsibly costs you time and earns you nothing but
credit. It is genuinely appreciated.
