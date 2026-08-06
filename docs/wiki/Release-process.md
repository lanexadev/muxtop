# Release process

Maintainer runbook, and a description of what a release actually does — useful
if you maintain a fork or want to know what a tag triggers before trusting a
binary.

## The branch model

| Branch | |
|---|---|
| `develop` | Default branch. Every pull request targets it |
| `main` | Stable releases only. Never committed to directly |

A release is a `develop` → `main` pull request, then a tag on `main`.

CI runs on pull requests to **both** branches, because `main`'s protection
requires those checks and a trigger scoped to `develop` alone would deadlock
every release behind checks that could never run.

---

## Cutting a release

### 1. Prepare on `develop`

- Move `CHANGELOG.md`'s `[Unreleased]` entries under a new `[X.Y.Z]` heading
  with the date. **The release fails without this** — a `verify` job checks it.
- Bump `version` in `[workspace.package]` in the root `Cargo.toml`, and the
  three internal `muxtop-* = { path = …, version = "X.Y.Z" }` entries in
  `[workspace.dependencies]` that must match it.
- Update the README where it would otherwise be wrong: the feature table, the
  roadmap, keybindings.
- Update `docs/wiki/` for anything a user acts on. It publishes from `main`, so
  wiki changes ship with the release rather than ahead of it.

```sh
just check          # fmt + clippy + deny + tests
cargo build --release && ./target/release/muxtop --about
just bench-thomas   # confirm the budgets still hold
```

### 2. Merge to `main`

Open the `develop` → `main` pull request and wait for all seven required checks:
Format, Clippy, Cargo Deny, Test (ubuntu-latest), Test (macos-latest),
Test (windows-latest), Bench compile check.

### 3. Tag

```sh
git switch main && git pull
git tag -a v0.5.2 -m "v0.5.2"
git push origin v0.5.2
```

The tag **must** match the workspace version — `v0.5.2` for version `0.5.2`. A
mismatch stops the pipeline before anything is published.

### 4. Watch it

```sh
gh run watch
```

---

## What the tag triggers

`.github/workflows/release.yml`, in order:

```
verify ──► build (×4 targets) ──► release ──┬──► update-homebrew
                                            ├──► update-apt
                                            └──► publish (crates.io)
```

**`verify`** — the gate. Tag matches the workspace version; `CHANGELOG.md` has a
section for it. This exists because everything downstream is irreversible: a
crates.io publish is permanent, and two package managers will have taken the
version before anyone notices it was wrong.

**`build`** — four targets, `fail-fast: false` so one platform failing does not
hide the others:

| Target | How |
|---|---|
| `x86_64-unknown-linux-musl` | `cross` |
| `aarch64-unknown-linux-musl` | `cross` |
| `x86_64-apple-darwin` | native |
| `aarch64-apple-darwin` | native |

Each produces a stripped binary, a `.tar.gz`, a `.sha256`, and on Linux a
`.deb`. Each also gets a **build-provenance attestation** — a signed statement,
recorded in a public transparency log, that this exact artefact came out of this
workflow at this commit.

**`release`** — creates the GitHub release with generated notes and uploads every
archive, `.deb`, checksum, and `scripts/install.sh`.

**`update-homebrew`** — regenerates `Formula/muxtop.rb` in
`lucasschimmel/homebrew-tap` with the published SHA-256 of each archive, and
pushes.

**`update-apt`** — dispatches the repository-update workflow in
`lucasschimmel/apt`.

**`publish`** — publishes to crates.io in dependency order:
`muxtop-core` → `muxtop-proto` → `muxtop-tui` → `muxtop`, with a 30-second pause
between each so the index catches up. An "already exists" error is tolerated, so
a partially-completed publish can be resumed by re-running the job.

### Least privilege

The workflow's token is **read-only by default**. `release` gets
`contents: write`; `build` gets `id-token` and `attestations` write to sign
provenance; the Homebrew and APT jobs get **nothing** on this repository — they
authenticate to the other repos with a PAT. So a compromised build step cannot
publish a release, and a compromised release step cannot mint an attestation.

Every third-party action is pinned to a commit SHA, with the version in a
comment beside it. Dependabot advances them in reviewed pull requests.

---

## If something fails

**A build target failed.** Fix it, delete the tag, re-tag. Nothing was published
because `release` needs all of `build`.

```sh
git tag -d v0.5.2 && git push origin :refs/tags/v0.5.2
```

**crates.io publish failed after the GitHub release succeeded.** Do not re-tag —
the release exists. Re-run the `Publish to crates.io (manual)` workflow, which
skips crates already published:

```sh
gh workflow run publish-manual.yml
```

**The Homebrew or APT job failed.** They are independent of each other and of
crates.io. Re-run the individual job from the Actions tab.

**A published version is broken.** crates.io versions cannot be deleted, only
yanked:

```sh
cargo yank --version 0.5.2 muxtop
```

Then ship `0.5.3`. Yanking stops new dependents from resolving to it; it does not
remove it.

---

## Other automation

| Workflow | Trigger | Does |
|---|---|---|
| `ci.yml` | push / PR on `develop`, `main` | fmt, clippy, deny, tests on three OSes, bench compile, MSRV, rustdoc, coverage, semver |
| `advisories.yml` | daily 06:17 UTC, and on `Cargo.lock` changes | `cargo deny check advisories`; files or updates one rolling issue on failure |
| `codeql.yml` | push / PR / weekly | CodeQL static analysis into the Security tab |
| `wiki-sync.yml` | push to `main` touching `docs/wiki/**` | Mirrors `docs/wiki/` to the GitHub wiki |
| `publish-manual.yml` | manual | crates.io publish, resumable |

`advisories.yml` exists because CI only runs when somebody pushes. An advisory
published against a dependency muxtop already ships is not a code change, so
nothing would trigger a build — and the vulnerability would sit unnoticed until
the next unrelated commit.

---

## Versioning

Pre-1.0, Cargo's 0.x rules apply: `0.5.1` → `0.5.2` must not break the public
API of a published crate, `0.5` → `0.6` may. The `Semver` CI job checks
`muxtop-core`, `muxtop-proto` and `muxtop-tui` against their published baseline.

There are no long-term support branches. Security fixes land on `develop` and
ship in the next release — see
[SECURITY.md](https://github.com/lucasschimmel/muxtop/blob/main/SECURITY.md).
