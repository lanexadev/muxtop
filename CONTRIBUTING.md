# Contributing to muxtop

Thank you for your interest in contributing! This document covers everything you need to get started.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Prerequisites](#prerequisites)
- [Development Setup](#development-setup)
- [Project Architecture](#project-architecture)
- [Making Changes](#making-changes)
- [Code Standards](#code-standards)
- [Testing](#testing)
- [Submitting a Pull Request](#submitting-a-pull-request)
- [License](#license)

---

## Code of Conduct

Be respectful and constructive. We aim to keep this project welcoming to contributors of all experience levels.

---

## Prerequisites

| Tool | Version | Purpose |
|------|---------|---------|
| Rust | stable (≥ 1.88) | Compiler and toolchain |
| `just` | latest | Task runner |
| `bacon` | latest | Optional — continuous check during dev |
| `cargo-deny` | latest | Dependency audit (required for CI) |

Install the Rust toolchain via [rustup](https://rustup.rs/). The others can be installed with:

```sh
cargo install just bacon cargo-deny
```

---

## Development Setup

```sh
git clone https://github.com/lanexadev/muxtop.git
cd muxtop
cargo build --workspace
```

Run `just` (no arguments) to see all available recipes:

```sh
just
```

Key recipes:

| Recipe | Description |
|--------|-------------|
| `just build` | Debug build |
| `just run` | Run muxtop locally |
| `just dev` | Continuous clippy via bacon |
| `just check` | Full CI check (fmt + clippy + deny + test) |
| `just test` | Run the test suite |
| `just bench` | Run criterion benchmarks |
| `just fmt` | Auto-format code |

---

## Project Architecture

muxtop is a Cargo workspace with four crates:

```
muxtop-core     — Data collection engine (CPU, memory, processes, network, disk)
muxtop-tui      — Terminal UI built with ratatui (tabs, panels, fuzzy search, keybindings)
muxtop-proto    — Wire protocol and binary serialization for remote monitoring
muxtop-server   — TCP daemon exposing muxtop-core data over the network
```

The binary entrypoint lives in `src/main.rs` at the workspace root and wires up the crates. When in doubt about where to place new code, prefer keeping data collection in `muxtop-core` and rendering concerns in `muxtop-tui`.

---

## Making Changes

### Branch model

- **`develop`** — active development, target for all PRs
- **`main`** — stable releases only, never commit directly

Always branch off `develop`:

```sh
git switch develop
git pull
git switch -c feat/your-feature
```

### Commit messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(tui): add disk I/O panel
fix(core): handle zombie processes in process tree
perf(proto): reduce allocation in snapshot serialization
docs: update keybindings in README
test(core): add edge cases for network collector
```

Common scopes: `core`, `tui`, `proto`, `server`, `cli`, `ci`, `docs`.

---

## Code Standards

### Formatting

Enforced via `rustfmt` with the config in `rustfmt.toml` (edition 2024, max line width 100). Run before committing:

```sh
just fmt
```

### Lints

All clippy warnings are treated as errors (`-D warnings`). Run:

```sh
just clippy
```

Address every warning — do not use `#[allow(...)]` without a comment explaining why it is necessary.

### MSRV

The minimum supported Rust version is **1.88**. Do not use features stabilized after that version without updating `Cargo.toml` and `clippy.toml` accordingly.

### Dependencies

New dependencies must pass `cargo deny check`. Prefer crates already in `[workspace.dependencies]`. Avoid adding heavy dependencies for small utilities.

---

## Testing

```sh
just test        # unit + integration tests
just bench       # criterion benchmarks (compile + run)
```

- Place unit tests in `#[cfg(test)]` modules within the source file.
- Place integration tests under `tests/` at the workspace root or within each crate's `tests/` directory.
- Benchmarks live under each crate's `benches/` directory using criterion.

CI runs tests on **Ubuntu** and **macOS**. If your change is platform-specific, note it in the PR.

---

## Submitting a Pull Request

1. Ensure `just check` passes locally — this runs fmt, clippy, deny, and tests.
2. Keep the scope of a PR focused. One feature or fix per PR.
3. Update `CHANGELOG.md` under the `[Unreleased]` section following the existing format.
4. Open the PR against the `develop` branch.
5. Fill out the PR description: what changed, why, and how to test it.

CI will run automatically. PRs cannot be merged until all checks pass.

---

## License

By contributing to muxtop, you agree that your contributions will be dual-licensed under the terms of the **MIT** and **Apache 2.0** licenses, consistent with the rest of the project. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
