# Remote monitoring

`muxtop-server` collects on one host; `muxtop --remote` renders on another. Only
digested snapshots cross the wire — never a kubeconfig, a bearer token or a TLS
key.

```
┌────────────────────┐   TLS 1.3 + token    ┌──────────────────────────┐
│ muxtop --remote    │ ───────────────────► │ muxtop-server            │
│ (your laptop)      │ ◄─────────────────── │ (the host being watched) │
│ renders only       │   snapshots only     │ reads /proc, sockets, API│
└────────────────────┘                      └──────────────────────────┘
```

TLS and a token are both **mandatory**. There is no plaintext mode and no
anonymous mode to forget to turn off.

---

## Start here: don't expose the port

The most secure deployment is also the simplest one. Bind the server to
localhost — its default — and reach it through an ssh tunnel you already trust:

```sh
# On the monitored host
muxtop-server --token-file /etc/muxtop/token --tls-generate

# On your machine
ssh -N -L 4242:127.0.0.1:4242 admin@host &
muxtop --remote 127.0.0.1:4242 --token-file ~/.muxtop-token --tls-skip-verify
```

This gives you ssh's authentication and transport security, with muxtop's TLS
inside it, and **no new port open to the internet**. `--tls-skip-verify` is
acceptable *here specifically* because the tunnel already guarantees you are
talking to the host you authenticated to — the certificate is not what is
protecting you.

Everything below is for when you genuinely need a listening port.

---

## 1. The token

At least 16 characters, enforced. Generate one properly:

```sh
openssl rand -base64 32
```

Pass it one of three ways:

| Method | Use |
|---|---|
| `--token-file /etc/muxtop/token` | **Preferred.** Read once at startup |
| `MUXTOP_TOKEN` env var | Containers, CI |
| `--token "…"` | Interactive testing only |

> **`--token` leaks on a shared host.** Command-line arguments are readable by
> any local user through `/proc/<pid>/cmdline` and `ps eww` — including, with
> some irony, through muxtop's own Processes tab. Use `--token-file` anywhere
> other people have shell access.

```sh
sudo install -d -m 0750 -o muxtop -g muxtop /etc/muxtop
openssl rand -base64 32 | sudo tee /etc/muxtop/token >/dev/null
sudo chmod 0400 /etc/muxtop/token
sudo chown muxtop:muxtop /etc/muxtop/token
```

Trailing whitespace is trimmed, so a stray newline from `tee` is fine.

## 2. The certificate

### Development: let the server generate one

```sh
muxtop-server --token-file /etc/muxtop/token --tls-generate
```

It writes three files into muxtop's data directory
(`~/.local/share/muxtop/` on Linux, `~/Library/Application Support/muxtop/` on
macOS), with the directory at mode `0700`:

| File | Mode | |
|---|---|---|
| `server.crt` | 0644 | the certificate |
| `server.key` | 0600 | the private key — never leaves the host |
| `server.fingerprint` | 0644 | SHA-256 of the certificate |

The fingerprint is also printed to stderr on generation. It exists in a file
because that `eprintln` disappears into a systemd journal, and you need the
value later to check you are talking to the right server.

Copy `server.crt` to the client and pass it as the CA:

```sh
scp admin@host:~/.local/share/muxtop/server.crt ./muxtop-host.crt
muxtop --remote host:4242 --token-file ~/.muxtop-token --tls-ca ./muxtop-host.crt
```

### Production: bring your own

```sh
openssl req -x509 -newkey rsa:4096 -sha256 -days 365 -nodes \
  -keyout server.key -out server.crt \
  -subj "/CN=monitor.example.com" \
  -addext "subjectAltName=DNS:monitor.example.com"
```

```sh
muxtop-server \
  --token-file /etc/muxtop/token \
  --tls-cert /etc/muxtop/server.crt \
  --tls-key /etc/muxtop/server.key \
  --bind 0.0.0.0:4242
```

> **The `subjectAltName` is not optional.** muxtop uses rustls, which does not
> fall back to the deprecated Common Name — a certificate with only `/CN=` is
> rejected with a name-mismatch error no matter how correct the CN looks.
>
> **Connect by the name in the certificate.** `--remote 10.0.0.7:4242` fails
> against a `DNS:monitor.example.com` certificate. Either use the hostname, or
> add an IP entry: `-addext "subjectAltName=IP:10.0.0.7"`.

Certificate and key are loaded once at startup, so **restart the service after
rotating them**. Renewal from cron needs a `systemctl restart muxtop-server`
next to it.

## 3. The client

```sh
# Production — verify against the CA
muxtop --remote monitor.example.com:4242 --token-file ~/.muxtop-token \
       --tls-ca ./ca.crt

# Token from the environment instead
MUXTOP_TOKEN="…" muxtop --remote monitor.example.com:4242 --tls-ca ./ca.crt

# Development only — no verification at all
muxtop --remote 127.0.0.1:4242 --token "…" --tls-skip-verify
```

`--tls-skip-verify` and `--tls-ca` are mutually exclusive, on purpose: passing
both would leave it ambiguous which one won.

---

## Running it as a service

Create an unprivileged user and a hardened unit. This one is deliberately
restrictive; the notes after it explain what each restriction costs.

```ini
# /etc/systemd/system/muxtop-server.service
[Unit]
Description=muxtop remote monitoring server
Documentation=https://github.com/lucasschimmel/muxtop/wiki/Remote-monitoring
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=muxtop
Group=muxtop
StateDirectory=muxtop

# muxtop-server writes its log under the XDG data directory. A system user has
# no $HOME, so without this the path resolves to the working directory — which
# ProtectSystem=strict makes read-only, and startup fails with
# "failed to create muxtop data directory".
Environment=XDG_DATA_HOME=/var/lib/muxtop

ExecStart=/usr/bin/muxtop-server \
    --bind 127.0.0.1:4242 \
    --token-file /etc/muxtop/token \
    --tls-cert /etc/muxtop/server.crt \
    --tls-key /etc/muxtop/server.key \
    --max-clients 4 \
    --rate-limit-per-ip 5

Restart=on-failure
RestartSec=5

# --- Hardening ---
NoNewPrivileges=true
CapabilityBoundingSet=
AmbientCapabilities=
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectKernelLogs=true
ProtectControlGroups=true
ProtectClock=true
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
RestrictNamespaces=true
RestrictRealtime=true
RestrictSUIDSGID=true
LockPersonality=true
MemoryDenyWriteExecute=true
SystemCallArchitectures=native
SystemCallFilter=@system-service
UMask=0077

[Install]
WantedBy=multi-user.target
```

```sh
sudo useradd --system --no-create-home --shell /usr/sbin/nologin muxtop
sudo systemctl daemon-reload
sudo systemctl enable --now muxtop-server
sudo journalctl -u muxtop-server -f
```

### What the hardening costs

Three directives are **deliberately absent**, because each one breaks a feature:

| Not set | Why |
|---|---|
| `ProtectProc=invisible` | Hides other users' processes from `/proc`. The Processes tab would show only muxtop itself — it defeats the entire point |
| `PrivateDevices=true` | Blocks `/dev/nvidia*` and `/dev/dri/*`. The GPU tab goes empty. Set it if you pass `--no-gpu` |
| `PrivateUsers=true` | Breaks reading other users' process metadata |

Two more caveats worth knowing:

- **`hidepid`**: if `/proc` is mounted with `hidepid=1` or `hidepid=2`, an
  unprivileged `muxtop-server` sees only its own processes. Add the service
  user to the `gid=` group configured on that mount, or accept the reduced view.
- **Container socket access**: adding `muxtop` to the `docker` group makes it
  **root-equivalent** on the host and throws away everything the unit above
  buys you. Use a rootless Podman socket instead — see [Containers](Containers).

---

## Firewall and exposure

If you must open the port:

```sh
# nftables — one source only
sudo nft add rule inet filter input ip saddr 203.0.113.10 tcp dport 4242 accept

# ufw
sudo ufw allow from 203.0.113.10 to any port 4242 proto tcp
```

muxtop's own limits are a second layer, not the first:

| Flag | Default | Effect |
|---|---|---|
| `--max-clients` | 8 | Hard cap on concurrent connections |
| `--rate-limit-per-ip` | 10 | Connections/second per source IP, token bucket with a burst of 10. `0` disables it |
| `--refresh` | 1 | Collection interval in seconds (1–3600) |

On a host with thousands of processes, raise `--refresh` — the snapshot is
rebuilt and serialised every interval, and 5 seconds is usually plenty for a
server you are watching rather than debugging.

---

## Limiting what the server can see

The server decides what exists; the client cannot ask for more. Turning a data
source off at the server turns it off for every remote client:

```sh
muxtop-server --token-file /etc/muxtop/token --tls-generate \
  --no-containers \
  --no-kube \
  --no-gpu
```

Or scope rather than disable:

```sh
--kube-namespace production    # Pods/Deployments from one namespace only
--kube-context staging         # a specific kubeconfig context
--docker-socket "$XDG_RUNTIME_DIR/podman/podman.sock"
```

In `--remote` mode the client's own `--kube-namespace` is irrelevant — the
server's setting decides, and `A` (toggle namespace scope) does nothing.

## What is disabled over the wire

Every action that changes the host: renice, kill, force-kill, and container
stop / kill / restart. They are rejected with a message rather than silently
ignored. `muxtop --remote` is a viewer.

---

## Verifying you are talking to the right server

```sh
# On the server
cat ~/.local/share/muxtop/server.fingerprint

# From the client, against the certificate you were given
openssl x509 -in muxtop-host.crt -noout -fingerprint -sha256
```

The two must match. If they don't, stop — either the certificate you have is not
the one the server is serving, or something is in the middle.

## Troubleshooting

| Symptom | Cause |
|---|---|
| `invalid peer certificate: NotValidForName` | The certificate has no `subjectAltName`, or you connected by IP against a DNS-only certificate |
| `invalid peer certificate: UnknownIssuer` | `--tls-ca` missing, or pointing at the wrong certificate |
| Connection accepted then dropped immediately | Token mismatch, or shorter than 16 characters |
| `failed to create muxtop data directory` | No writable `XDG_DATA_HOME`/`HOME` — see the systemd note above |
| Connections refused under load | `--max-clients` reached, or the per-IP rate limit tripped |
| Containers or Kube tab empty remotely | The **server** has no engine configured, or was started with `--no-containers` / `--no-kube` |

Server-side detail lands in `muxtop-server.log` inside the data directory. For
more:

```sh
MUXTOP_LOG=debug muxtop-server …
```

See also: **[Security model](Security-model)** for the threat model behind these
defaults, and **[Troubleshooting](Troubleshooting)** for the general table.
