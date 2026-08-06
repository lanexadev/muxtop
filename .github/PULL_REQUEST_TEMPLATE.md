<!--
Target `develop`, not `main` — `main` takes release merges only.
See CONTRIBUTING.md for the branch model.
-->

## What changed

<!-- One or two sentences. The commit log has the detail. -->

## Why

<!--
The problem, not the patch. If it closes an issue, say `Closes #N` — that is
what links the two and closes it on merge.
-->

## How to test it

<!--
The reviewer has your branch checked out and a terminal. What do they run, and
what should they see? "cargo test" is already covered by CI — this is about the
behaviour a test cannot assert, like how a panel renders at 80 columns.
-->

---

## Checklist

- [ ] `just check` passes locally (fmt + clippy + deny + tests)
- [ ] `CHANGELOG.md` updated under `[Unreleased]`
- [ ] New behaviour is covered by a test, or the PR says why it cannot be
- [ ] Documentation updated where it would otherwise become wrong — README,
      `docs/wiki/`, `--help` text
- [ ] Any new `#[allow(...)]` carries a comment explaining why

### If this touches the public API of a published crate

<!-- muxtop-core, muxtop-proto, muxtop-tui are on crates.io. -->

- [ ] The `Semver` CI job is green, or the break is intentional and the version
      bump accounts for it

### If this touches security-relevant code

<!--
muxtop-server, muxtop-proto, actions.rs, the ANSI sanitizer, TLS or token
handling, file permissions.
-->

- [ ] Attacker-controlled input paths are stated in the description
- [ ] No credential, token or key can reach a log, a `Debug` output, or the wire
- [ ] `SECURITY.md` still describes reality

### If this changes a platform boundary

- [ ] Compiles on all three CI platforms, or the new code is `cfg`-gated with a
      stub that fails honestly (see `crates/muxtop-core/src/actions.rs`)
