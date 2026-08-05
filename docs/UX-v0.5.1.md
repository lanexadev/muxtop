# muxtop — TUI ergonomics & UI/UX overhaul (v0.5.1)

> Status: **proposal / ready for implementation**
> Baseline audited: `develop` @ `45728c8` (v0.4.2)
> Target: lands **after** v0.5 (GPU monitoring + `docker exec` PTY), rebased on it.

---

## 0. Scope and constraints

**In scope.** Everything the user sees and touches: layout, navigation, keymap, discoverability,
feedback, empty/error states, colour system, density, mouse, overlays. Plus the internal refactor
that makes those consistent (a shared table/widget layer instead of five hand-rolled tables).

**Out of scope.** New data sources, new collectors, remote protocol changes, the config file and the
WASM plugin system (both v1.0). GPU is v0.5's job — this document only reserves its slot in the
chrome and the keymap.

**Hard constraints — the redesign must not cost what makes muxtop good:**

| Constraint | Current value | Rule |
|---|---|---|
| Peak RSS (30 s) | 11.3 MiB | must stay < 13 MiB |
| Binary size | 5.3 MiB | must stay < 7 MiB |
| Startup (`--about`) | ~12 ms | must stay < 30 ms |
| Idle redraws | ~0 (event-driven, `PERF-H1`) | no widget may force a per-tick repaint |
| Dependencies | ratatui + crossterm + nucleo | no new UI dependency; everything below is buildable on ratatui 0.30 |
| Minimum viable terminal | 80×24 | must degrade to 60×20 without panic or unreadable output |

**Versioning note.** This changes default key bindings. That is not a patch-level change in spirit.
Recommendation: ship it as **v0.6.0**, or ship under v0.5.1 with `--legacy-keys` restoring the
v0.4 map for one release. The rest of this document says "v0.5.1" for continuity with the roadmap.

---

## 1. Audit

### 1.1 What is good — keep it, don't touch it

- **Tokyo Night identity.** Coherent, legible, non-generic. It should become a token system, not be
  replaced.
- **htop-style zone bars** (`ui/general.rs:245` `build_htop_bar`). The green/yellow/red *zone*
  colouring — not a single colour for the whole bar — is exactly right, and the info label is
  protected from being overwritten by the fill. Keep the algorithm, restyle the glyphs.
- **Event-driven rendering** (`lib.rs`, `needs_redraw`). Rare and correct. Every new widget must
  respect it.
- **Virtualised table bodies** (`processes.rs:107` `draw_body` slices `scroll..end`). 3000+ processes
  cost nothing. Keep.
- **Honest empty states in Containers and Kube** (`containers.rs:122-164`, `kube.rs:58-79`,
  `kube.rs:359-362`). "No nodes — listing nodes needs cluster-scoped access" tells the user *why*
  and *what to do*. This is the quality bar the rest of the app should meet.
- **ANSI scrubbing of untrusted strings** (`ui/sanitize.rs`, applied at `processes.rs:191`).
  Non-negotiable, keep it wired into the new shared row renderer.
- **Remote-mode command exclusion** (`app.rs:1697` `REMOTE_BLOCKED_COMMANDS`). The palette hides what
  cannot work. Extend the same idea to the footer, the help screen and the actions menu.
- **Test discipline.** Every view has `TestBackend` buffer assertions. The refactor must keep them
  green and add snapshot tests, not delete them.

### 1.2 What does not work

Ordered by user impact.

#### A. Discoverability — the app does not explain itself

- **No help screen.** The README says so out loud (`README.md:176`). `?` and `F1` do nothing.
  Every other monitor in this category (htop, btop, k9s, lazydocker, bottom) has one.
- **The footer is a fixed string list** (`ui/mod.rs:143-222`). It never shows *state* (current
  sort, active filter, position in list, paused/live), and on a 80-column terminal the Containers
  hint row is already ~70 chars — it silently truncates.
- **Half the keymap is undocumented and unreachable.** `P`/`N`/`D`/`A` (Kube) exist only in the
  README. The palette lists no Kube command except "Switch to Kubernetes tab", no sub-view switch,
  no scope toggle, no filter-clear for Kube.
- **The README promises things the code does not do.** `README.md:70` advertises palette commands
  with arguments (`kill firefox`, `stop nginx`, `restart postgres`) — `Command` is a plain fieldless
  enum (`app.rs:97-131`), there is no argument parsing anywhere. `README.md:74` documents `+`/`-`
  for renice — there is no `Char('+')` handler in `app.rs`. Two documented features that do not exist.

#### B. Navigation — the mental model contradicts itself

- **`←`/`→` switch tabs** (`app.rs:1352-1357`) while `↑`/`↓` move the row cursor. Horizontal arrows
  meaning "change screen" and vertical arrows meaning "change row" inside the same table is the
  single most disorienting thing in the current UI. `Tab`/`Shift+Tab` already do tab switching.
- **Global keys that are not global.** `t` toggles the process tree from any tab
  (`app.rs:1368`); `F1`–`F5` re-sort the *process* list while the user is looking at the Network tab
  (`app.rs:1441-1460`). Silent, invisible, surprising.
- **`F1`–`F5` for sort breaks the htop convention muxtop claims.** htop uses `F1` Help, `F3` Search,
  `F4` Filter, `F5` Tree, `F6` Sort, `F7`/`F8` Nice, `F9` Kill, `F10` Quit. muxtop advertises "htop
  shortcuts" and then binds `F1`=sort-by-PID. Users coming from htop press `F1` expecting help and
  silently re-sort a table.
- **No way to inspect a row.** `Enter` is unbound in normal mode. A monitor where you cannot open
  the selected process/container/pod for details is missing its second layer.
- **No pause.** You cannot freeze the refresh to read a fast-moving table. Every serious monitor has
  this.
- **`Esc` is overloaded and non-progressive.** Depending on mode it exits filter-edit, clears the
  filter, or closes the palette, with no consistent "one step back" semantics.

#### C. Confirmed bugs

| # | Bug | Evidence |
|---|---|---|
| 1 | **Mouse wheel does nothing in Processes.** The handler moves `scroll_offset` but not `selected`; `effective_scroll` then snaps the view back to `selected` on the very next frame. | `app.rs:1932-1943` vs `processes.rs:254-265` |
| 2 | **Mouse wheel is tab-blind.** It always mutates the *process* scroll offset, even on Network / Containers / Kube. | `app.rs:1934-1940` |
| 3 | **Palette "Clear filter" ignores the Kube tab** — it clears the *process* filter instead. | `app.rs:1789-1796` (no `Tab::Kube` arm) |
| 4 | **Status severity is inferred by string sniffing** — `status.contains("failed") \|\| contains("denied")`. A localised or reworded message silently renders success-green on failure. | `ui/mod.rs:133-137` |
| 5 | **`ColorSupport::Colors256` is detected and then thrown away.** `Theme::new` branches only on `TrueColor`; a 256-colour terminal gets the 16-colour fallback. `NoColor` also gets colours (`Cyan`, `Green`…), and `NO_COLOR` / `--no-color` are not honoured at all. | `terminal.rs:79-96` vs `theme.rs:33-73` |

#### D. Layout and density

- **General wastes the screen.** `Constraint::Min(0)` at `general.rs:41` absorbs the remaining
  height with *nothing in it*. On a 50-row terminal, roughly half the General tab is blank.
- **No responsive behaviour.** Column widths are hard constants (`processes.rs:17-22`,
  `containers.rs:23-30`). Below ~100 columns the Containers table truncates image names to
  uselessness while `NAME` keeps its full 20 columns; the process `COMMAND` column is the one that
  gets crushed even though it is the most information-dense.
- **Three rows of chrome at the top** (header 1 + tab bar 2, `ui/mod.rs:32-38`) for app name,
  version and five words. On 24 rows that is 12.5% of the screen carrying almost no information.
- **No scrollbar, no position indicator.** In a 3000-process list the user has no idea where they are.

#### E. Visual design

- **`Theme` is a flat 14-field struct with two hardcoded branches** (`theme.rs`). No named
  semantic layers, no second theme, no light variant, no per-component derivation.
- **Bars are ASCII `|` even on Unicode terminals** (`general.rs:245`, the `unicode` flag is
  received as `_unicode` and ignored). Modern terminals can render proper block/braille meters.
- **Zebra striping exists only in Processes** (`processes.rs:198-204`). Network, Containers and
  Kube rows are flat, so wide rows are harder to track across.
- **Sort arrows are re-declared per file** (`containers.rs:32-35` vs `processes.rs:58-68`) with
  different glyph choices.
- **Selection is background-only.** No accent edge, so on 16-colour terminals the selected row is
  barely distinguishable.

#### F. Code-level consequence

`ui/processes.rs`, `network.rs`, `containers.rs`, `kube.rs` each hand-roll: block + border, summary
bar, column header with sort arrow, virtualised body, zebra/selection styling, filter bar, empty
state. ~1600 duplicated lines, four subtly different behaviours, four places to fix every bug. This
is the root cause of most of section 1.2 and it is why the fix is a *refactor*, not a repaint.

---

## 2. Design principles

1. **State is always visible.** Sort, filter, scope, position, connection, live/paused — never
   hidden in the user's memory.
2. **Every key is discoverable from inside the app.** `?` shows everything; the palette finds
   anything by name; the footer teaches the five most useful keys for the current context.
3. **One mental model for every table.** Same navigation, same sorting, same filtering, same
   selection, same empty state — Processes, Network, Containers, Kube, GPU.
4. **Context beats globality.** A key does what makes sense *here*, or it does nothing and says so.
   No silent action on an invisible tab.
5. **Degrade, never break.** TrueColor → 256 → 16 → none; Unicode → ASCII; 140 cols → 60 cols. Each
   step loses polish, never information or safety.
6. **Destructive actions are always two-step, always labelled.** Kill, renice, stop, restart.
7. **Fast stays fast.** No redraw the user did not cause; no allocation in a row loop.

---

## 3. Design system

### 3.1 Token layers

Replace the flat `Theme` struct with three layers (`ui/theme/`):

```
palette.rs   primitives   ink, paper, slate[0..5], cyan, purple, green, yellow, red, orange, blue
tokens.rs    semantic     bg, surface, surface_alt, overlay, border, border_focus,
                          fg, fg_muted, fg_subtle, fg_inverse,
                          accent, accent_alt, selection_bg, selection_fg, selection_edge,
                          success, warning, danger, info, neutral,
                          meter_low, meter_mid, meter_high, meter_track
theme.rs     components   header, tabbar_active/inactive/disabled, table_header, row_odd,
                          badge_*, toast_*, scrollbar_thumb/track, sparkline
```

Components derive from semantics; views only ever read component or semantic tokens — never a
primitive. That is what makes a second theme a 30-line file instead of a rewrite.

### 3.2 Colour-support matrix

| Support | Behaviour |
|---|---|
| `TrueColor` | Full Tokyo Night RGB (unchanged) |
| `Colors256` | **New.** xterm-256 indices approximating the ramp (e.g. bg 234, surface 235, fg 189, accent 117, purple 141, green 149, yellow 179, red 210) |
| `Basic` | 16 ANSI, current fallback, plus `Modifier::DIM` for `fg_muted` |
| `NoColor` | **Fixed.** `Color::Reset` everywhere; hierarchy carried by `BOLD` / `DIM` / `REVERSED` only |

Honour `NO_COLOR` (any non-empty value → `NoColor`) and add `--no-color`, `--ascii`,
`--theme <name>`. Built-in themes for v0.5.1: `tokyo-night` (default), `tokyo-night-light`, `mono`.
User-defined themes stay v1.0 (config file).

### 3.3 Glyphs and ASCII fallback

Every glyph goes through one table (`ui/glyphs.rs`), selected once by `term_caps.unicode`:

| Role | Unicode | ASCII |
|---|---|---|
| meter fill / track | `█` `▓` `░` | `\|` `:` ` ` |
| meter partial | `▏▎▍▌▋▊▉` | `\|` |
| sort desc / asc | `▼` `▲` | `v` `^` |
| selection edge | `▎` | `>` |
| scrollbar thumb / track | `█` / `│` | `#` / `\|` |
| tree branch / last / pipe | `├── ` `└── ` `│ ` | `\|-- ` `\\-- ` `\| ` |
| status running / idle / stopped / zombie | `●` `○` `⏸` `⚠` | `R` `S` `T` `Z` |
| rate up / down | `↑` `↓` | `^` `v` |
| spinner | `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` | `-\\\|/` |
| separator | `·` | `.` |

`general.rs` currently takes `unicode` and ignores it (`_unicode`); this table is where that gets
fixed.

### 3.4 Density and responsive breakpoints

| Class | Width | Behaviour |
|---|---|---|
| XS | < 60 | single column, chrome collapses to 1 row, footer shows 3 hints, sparklines off |
| S | 60–99 | priority-3 columns dropped, CPU meters single column, detail pane becomes an overlay |
| M | 100–139 | full tables, detail pane as a 40% right split |
| L | ≥ 140 | + optional second panel (top-processes / mini graphs) on General |

Each table column declares `priority: u8` and `min_width`. The layout engine drops the lowest
priority first and always keeps the identity column (COMMAND / INTERFACE / NAME / POD).

---

## 4. Information architecture

### 4.1 Chrome — 2 rows top, 1 row bottom (one row reclaimed)

```
┌ row 0 ─ status line ───────────────────────────────────────────────────────────────────────┐
 muxtop v0.5.1  ●local  thinkpad  up 3d 4h   CPU ▓▓▓▓▓░░░ 42%  MEM ▓▓▓▓▓▓░░ 61%   14:22:07
├ row 1 ─ tabs ──────────────────────────────────────────────────────────────────────────────┤
 ¹General  ²Processes 342  ³Network 4  ⁴Containers 7  ⁵Kube 128  ⁶GPU 2 ─────────────────────
│                                                                                            │
│  content                                                                                   │
│                                                                                            │
├ last row ─ status bar ─────────────────────────────────────────────────────────────────────┤
 NORMAL │ sort cpu▼ │ filter "ngin" 12/342 │ 7/12 │ ? help  / filter  s sort  x actions  : cmd
└────────────────────────────────────────────────────────────────────────────────────────────┘
```

- **Row 0** carries what you always want: identity, connection (`●local` green / `◆remote host:port`
  purple / `✕disconnected` red), hostname, uptime, global CPU/MEM micro-meters, clock. Replaces a
  row that carried only "muxtop v0.4.2".
- **Row 1** — tabs with **live counts** and superscript Alt-numbers. Tabs with no data source render
  dim (Containers with no daemon, Kube with no cluster, GPU with no device) instead of pretending.
  Removes the hardcoded `FUTURE_TABS` `"GPU [soon]"` placeholder (`ui/mod.rs:26`).
- **Status bar** — segmented, priority-ordered, right-truncating: mode chip, sort chip, filter chip
  with match count, cursor position, then contextual hints filling whatever remains. Toasts overlay
  it temporarily (§6.4).
- **XS** collapses rows 0+1 into one: `muxtop ²Processes 342  42%/61%`.

### 4.2 Content region

Every tab is composed from the same slots, all optional:

```
[ context bar ]   sub-views, scope, engine, cluster — tab-specific chips
[ table         ]   shared component: header, virtualised body, scrollbar
[ inspector     ]   sparklines / detail pane, toggled with Enter or i
[ filter input  ]   only while editing
```

---

## 5. Interaction model

### 5.1 Modes

```
NORMAL ──/──> FILTER ──Esc/Enter──> NORMAL
   │
   ├──:──> COMMAND (typed, with args)   ──Esc/Enter──> NORMAL
   ├──Ctrl+P──> PALETTE (fuzzy)         ──Esc/Enter──> NORMAL
   ├──?──> HELP                          ──?/Esc/q──> NORMAL
   ├──x──> ACTIONS menu                  ──Esc──> NORMAL
   └──(destructive)──> CONFIRM           ──y/Enter | n/Esc──> NORMAL
```

`Esc` becomes strictly **progressive** — one step back, one press at a time:
close overlay → leave filter editing → clear filter text → do nothing (never quits).
`Ctrl+C` always quits, from any mode (already true, keep).

### 5.2 Keymap

**Global**

| Key | Action | Change |
|---|---|---|
| `?` · `F1` | Help overlay | **new** |
| `Ctrl+P` · `Ctrl+K` | Command palette (fuzzy) | `Ctrl+K` new |
| `:` | Command mode (typed, arguments) | **new** |
| `Tab` · `Shift+Tab` | Next / previous tab | unchanged |
| `Alt+1..6` | Direct tab | + GPU slot |
| `[` · `]` | Previous / next sub-view (Kube P/N/D, extensible) | **new** |
| `q` · `Ctrl+C` | Quit | unchanged |
| `Space` | Pause / resume refresh | **new** |
| `r` | Force refresh now | **new** |
| `?`→ any | which-key: pressing a prefix shows its continuations | **new** |

**Table navigation (identical in every tab)**

| Key | Action | Change |
|---|---|---|
| `j`/`k` · `↑`/`↓` | Move cursor | unchanged |
| `h`/`l` · `←`/`→` | Horizontal column scroll | **breaking** — no longer switches tabs |
| `g`/`G` · `Home`/`End` | First / last row | unchanged |
| `Ctrl+D`/`Ctrl+U` · `PgDn`/`PgUp` | Half / full page | `Ctrl+D/U` new |
| `Enter` · `i` | Open inspector for the selected row | **new** |
| `/` | Filter (tab-scoped) | unchanged |
| `Esc` | Progressive back | **changed** |
| `s` · `F6` | Sort menu (choose column) — `s` cycles when the menu is off | `F6` new, htop-compatible |
| `S` · `I` | Reverse sort order | unchanged |
| `x` | Contextual actions menu | **new** |
| `y` | Copy the selected row's identifier (PID / id / name) to the OSC-52 clipboard | **new** |

**Tab-scoped — only active on their own tab, never silently global**

| Tab | Keys |
|---|---|
| Processes | `t`/`F5` tree · `F7`/`F8` · `+`/`-` renice (**`+`/`-` finally implemented**) · `F9`/`F10` kill · `u` filter by user |
| Network | `p` toggle packets/bytes · `e` errors-only |
| Containers | `F9` stop · `F10` kill · `F11` restart · `a` show stopped |
| Kube | `[`/`]` or `P`/`N`/`D` sub-view · `A` namespace scope · `n` namespace filter |
| GPU (v0.5) | reserved: `Alt+6`, `g` cycle device |

**htop F-key alignment** — muxtop advertises "htop shortcuts", so honour the actual htop map:
`F1` Help, `F3` Search, `F4` Filter, `F5` Tree, `F6` Sort, `F7`/`F8` Nice, `F9` Kill, `F10` Quit.
The current `F1`–`F5`-as-sort bindings are dropped (kept behind `--legacy-keys` for one release,
with a one-shot toast: *"F1 is now Help — sort with s or F6"*).

### 5.3 Command palette v2

- **Context-aware ranking.** Commands for the active tab rank first; irrelevant ones (Kube sorts
  while in Network) rank last or are hidden.
- **Categories** in the list: `Navigation · Sort · Filter · Actions · View · App`.
- **Match highlighting** — nucleo already returns match indices; render them in `accent`.
- **Arguments** — `Command` becomes an enum with payloads plus a parsed form:
  `kill firefox`, `stop nginx`, `restart postgres`, `filter ngin`, `sort mem`, `theme mono`,
  `ns kube-system`. This is what the README already promises (`README.md:70`).
  Argument commands resolve their target against the current table and, when ambiguous, open a
  disambiguation list instead of guessing.
- **Session history** — last 5 executed commands float to the top on an empty query.
- **Remote-mode exclusion** stays (`REMOTE_BLOCKED_COMMANDS`), and excluded commands are shown
  greyed with the reason on hover-selection rather than vanishing — hidden commands teach nothing.

---

## 6. Screen specs

### 6.1 General → a real dashboard

Today: CPU bars, memory bars, one info line, then `Min(0)` of nothing.
Proposed: fill the space with the summary that makes a tabbed monitor worth having.

```
 ¹General  ²Processes 342  ³Network 4  ⁴Containers 7  ⁵Kube 128  ⁶GPU 2
╭─ CPU ─────────────────────────────────────────╮╭─ Load ──────────────────────────────────╮
│ cpu0 ▊▊▊▊▊▊▊░░░░░░░░░░░  38.2%  cpu8  ▊▊░ 9%  ││  1m  2.31 ▊▊▊▊▊░░░░░                    │
│ cpu1 ▊▊▊▊░░░░░░░░░░░░░░  21.0%  cpu9  ▊░░ 4%  ││  5m  1.87 ▊▊▊▊░░░░░░                    │
│ …                                             ││ 15m  1.42 ▊▊▊░░░░░░░   8 cores          │
╰───────────────────────────────────────────────╯╰─────────────────────────────────────────╯
╭─ Memory ──────────────────────────────────────╮╭─ Network ───────────────────────────────╮
│ Mem ▊▊▊▊▊▊▊▊▊▊░░░░░░  61%  9.8/16.0G          ││ ↓ 1.2 MB/s  ▁▂▅▇▆▃▂▁▂▄▆▇▅▃▂▁▁▂▃         │
│ Swp ▊▊░░░░░░░░░░░░░░  12%  0.5/4.0G           ││ ↑ 340 kB/s  ▁▁▂▂▁▃▂▁▁▂▂▁▁▂▃▂▁▁▁▁        │
╰───────────────────────────────────────────────╯╰─────────────────────────────────────────╯
╭─ Top processes ───────────────────────────────╮╭─ Workloads ─────────────────────────────╮
│  CPU%  MEM%  COMMAND                          ││ Containers  7 running · 2 exited        │
│  38.2   4.1  /usr/lib/firefox/firefox         ││ Kube        128 pods · 3 nodes · 12 dep │
│  12.0  11.7  /usr/bin/node server.js          ││ GPU         RTX 4090  61% · 8.2/24 GB   │
╰───────────────────────────────────────────────╯╰─────────────────────────────────────────╯
 NORMAL │ live │ 8 cores · 16 GB │ ? help  Tab next  Space pause  : cmd
```

- Two-column grid at M/L, single column stacked at S, CPU+MEM only at XS.
- The "Workloads" card is the cross-tab summary the tabbed model currently lacks — and it is where
  v0.5's GPU summary lands with zero layout work.
- Clicking (or `Enter` on) a card jumps to its tab.
- Meters use the block ramp with the existing zone colouring, ASCII-degrading to `|`.

### 6.2 Processes

```
╭─ Processes ─ 342 total · 12 matching "ngin" ─────────────────────────────────────────────╮
│   PID USER        S  CPU%   MEM%  TIME    COMMAND                                       ▲│
│▎ 1042 www-data    ●  38.2    4.1  1:22:04 nginx: worker process                         █│
│  1043 www-data    ○   0.4    3.9  0:11:20   ├── nginx: cache manager                    █│
│  1044 root        ○   0.0    0.2  0:00:03   └── nginx: master process                   │ │
│                                                                            (odd rows shaded) │
╰──────────────────────────────────────────────────────────────────────── 7/12 ───────────╯
 NORMAL │ sort cpu▼ │ filter "ngin" 12/342 │ 7/12 │ Enter details  x actions  F9 kill  ? help
```

- `▎` accent edge on the selected row (works even at 16 colours).
- New `TIME` column (priority 2, dropped below 100 cols) — CPU time is standard in htop and missing.
- Tree connectors get the vertical continuation `│` for non-last ancestors — the current prefix
  builder (`processes.rs:313`) indents with spaces and loses the lineage on deep trees.
- Filter chip shows `matched/total` — today a filter that matches nothing is indistinguishable from
  a system with no processes.
- Scrollbar + `7/12` position indicator.
- `Enter` opens the inspector: full cmdline (wrapped, scrollable), env-free metadata, parent chain,
  nice value, threads, open-files count, memory breakdown.

### 6.3 Network / Containers / Kube

Same skeleton, same keys, differing only in columns and context chips:

```
╭─ Containers ─ Docker · 7 running / 9 total ──────────────────────────────────────────────╮
│ NAME                 IMAGE                    STATE      CPU%  MEM            RX     TX ▲│
│▎api-gateway          nginx:1.27-alpine        ● running   4.2  128M/512M   1.2M/s  340k █│
│ postgres             postgres:16              ● running  11.9  1.1G/2.0G    88k/s   12k █│
│ old-worker           ghcr.io/acme/worker:v3   ✕ exited    0.0        —          —      — │
╰───────────────────────────────────────────── 2/9 ────────────────────────────────────────╯
```

- **State becomes a badge**, not plain text: coloured pill, `● running` / `⏸ paused` /
  `✕ exited(1)` / `⚠ restarting`, with the exit code inline.
- **Memory as `used/limit`** plus a micro-meter when a limit exists.
- **Zebra striping everywhere** (currently Processes-only).
- **Kube** gets its sub-views in the context bar as real sub-tabs with counts
  (`Pods 128 │ Nodes 3 │ Deployments 12`), plus the namespace-scope chip
  (`ns: kube-system` / `ns: all`) that today is invisible until you press `A` and read a toast.
  `CrashLoopBackOff` / `ImagePullBackOff` pods sort to the top by default and render in `danger`.
- The inspector for a pod shows containers, restart reasons, node, age, labels; for a container,
  ports, mounts, env-var *names* (never values), and the last exit code.

### 6.4 Overlays

**Help (`?`)** — the missing screen. Two columns, grouped, context-first:

```
╭─ Help ─ muxtop v0.5.1 ───────────────────────────────────────────────────────────────────╮
│  THIS TAB — Processes                    GLOBAL                                          │
│  t  F5   tree view                       ?  F1    this help                              │
│  F7 F8   renice  (+1 / -1)               Ctrl+P   command palette                        │
│  +  -    renice  (+1 / -1)               :        command mode                           │
│  F9      kill (SIGTERM)                  Tab      next tab       Alt+1..6  direct tab     │
│  F10     force kill (SIGKILL)            Space    pause          r         refresh        │
│  u       filter by user                  q        quit                                    │
│                                                                                           │
│  NAVIGATION                              SORT & FILTER                                    │
│  j k ↑ ↓  move          g G  first/last  s F6   sort column     S I  reverse               │
│  Ctrl+D/U half page     Enter  details   /      filter          Esc  back / clear          │
│                                                                                           │
│  Actions disabled in remote mode: kill, renice, container stop/kill/restart                │
╰─ ? or Esc to close ─ ↑↓ scroll ──────────────────────────────────────────────────────────╯
```

Generated **from the keymap table**, not hand-written — so it can never drift from the bindings, and
remote-blocked actions are automatically annotated instead of silently listed.

**Which-key.** After a prefix key, a small bottom-anchored panel lists the continuations after
~400 ms. Turns the keymap into something learnable without the help screen.

**Toasts.** Replace the single status line and its string sniffing (`ui/mod.rs:133`) with a typed
stack:

```rust
enum Level { Info, Success, Warning, Error }
struct Toast { level: Level, text: String, at: Instant, ttl: Duration }
```

Max 3 stacked bottom-right, colour and icon from `level`, errors sticky until dismissed (`Esc`),
5 s TTL otherwise. `Ctrl+L` reopens the last 20 messages as a log overlay — useful when an action
fails while you are looking elsewhere.

**Confirm.** Keep the two-step gate, restyle it: danger-bordered, target identity spelled out,
consequence stated, `y`/`Enter` confirm · `n`/`Esc` cancel, cancel focused by default.

**Inspector.** `Enter` — right 40% split at M/L, full overlay at S/XS. `Esc`/`Enter` closes,
`j`/`k` scrolls, `y` copies.

### 6.5 Mouse (fixed and extended)

| Gesture | Action |
|---|---|
| wheel | scroll the **active tab's** table (fixes bugs 1 & 2) |
| click row | select |
| double-click row | open inspector |
| click tab | switch tab |
| click column header | sort by that column / reverse if already active |
| drag scrollbar | scroll |
| click card (General) | jump to that tab |

Plus `--no-mouse`: mouse capture steals the terminal's native text selection, which is a real
complaint against TUIs. The flag disables `EnableMouseCapture` (`terminal.rs:132`), and the help
screen mentions the `Shift+drag` escape hatch that most terminals offer.

---

## 7. Component inventory

New module tree under `crates/muxtop-tui/src/ui/`:

```
chrome.rs          status line, tab bar, status bar, breakpoint resolution
glyphs.rs          Unicode/ASCII glyph table (§3.3)
theme/             palette.rs · tokens.rs · themes/{tokyo_night,tokyo_night_light,mono}.rs
keymap.rs          single source of truth: (mode, tab, key) -> Action; feeds help + which-key
widgets/
  table.rs         Column{label, width, priority, align, sort_key} + virtualised body,
                   zebra, selection edge, sort header, scrollbar, empty state
  meter.rs         zone-coloured block/ASCII bar (extracted from general.rs:245)
  sparkline.rs     threshold-coloured, shared by Network/Containers/GPU
  badge.rs         status pills
  card.rs          General dashboard cards
  kv.rs            key/value list for the inspector
  empty.rs         title + reason + remedy (generalising containers.rs:122-164)
  toast.rs         stack + log overlay
help.rs            keymap-generated help overlay
inspector.rs       per-tab detail panes
```

Each existing tab view shrinks to *column definitions + row formatting + tab-specific chips*.
Expected net effect: ~1600 lines of duplicated table code → ~600 shared + ~150 per tab, with one
place to fix a scrolling or styling bug.

---

## 8. Implementation plan

Ordered so that each epic is independently shippable and testable. E1–E3 are the load-bearing
refactor; E4+ are visible wins.

| Epic | Content | Risk | Est. |
|---|---|---|---|
| **E1 Design system** | token layers, 256-colour ramp, `NoColor` fix, `NO_COLOR`/`--no-color`/`--ascii`/`--theme`, glyph table | low | S |
| **E2 Keymap engine** | `keymap.rs`, mode state machine, progressive `Esc`, tab-scoping of `t`/F-keys, htop F-key alignment, `--legacy-keys` | medium | M |
| **E3 Table widget** | shared `widgets/table.rs` + scrollbar + responsive columns; migrate the 4 tabs one at a time | **high** (touches every view) | L |
| **E4 Chrome** | status line, tab bar with counts, segmented status bar, breakpoints | low | M |
| **E5 Discoverability** | help overlay, which-key, footer state chips | low | M |
| **E6 Feedback** | typed toasts, log overlay, spinners, pause/refresh | low | S |
| **E7 Palette v2** | context ranking, categories, match highlighting, **argument commands**, history | medium | M |
| **E8 Inspector + actions menu** | `Enter` detail panes per tab, `x` menu, `y` OSC-52 copy | medium | M |
| **E9 General dashboard** | card grid, top-processes, network mini-graph, workloads card (GPU slot) | low | M |
| **E10 Mouse** | wheel fix, click select/sort/tab, scrollbar drag, `--no-mouse` | low | S |
| **E11 Docs** | README keymap table rewrite, remove the two false claims, CHANGELOG, help screenshot | low | S |

**Suggested order:** E1 → E2 → E3 → E4 → E5 → E6 → E10 → E7 → E8 → E9 → E11.
E1+E2+E3+E4+E5 alone already fix every item in §1.2 A–D and are a coherent release on their own.

### Test strategy

- **Snapshot tests.** `TestBackend` buffers rendered at 60×20, 80×24, 100×30, 140×40 for every tab ×
  {TrueColor, 256, Basic, NoColor} × {Unicode, ASCII}, compared against committed golden files.
  This is the only way a refactor of this size stays safe, and the existing `buffer_contains` tests
  become the coarse layer above it.
- **Keymap property test.** Every `(mode, tab, key)` in `keymap.rs` resolves to an action that
  exists; no key is bound twice in the same context; every action is reachable from the palette.
- **Help/keymap coherence test.** The help overlay is generated from the keymap, so assert that
  every binding appears exactly once — the drift that produced the README's two false claims becomes
  structurally impossible.
- **Bug regression tests.** One test per confirmed bug in §1.2 C.
- **Perf gates.** Re-run `just bench-thomas` per epic; fail the PR on RSS > 13 MiB or binary > 7 MiB.

### Rebase strategy against v0.5 (GPU)

v0.5 will add a `Tab::Gpu`, a GPU view module, palette commands and footer hints. Conflict surface:
`app.rs` (`Tab::ALL`, `Command::ALL`, `handle_key_event`), `ui/mod.rs` (`draw_content`, footer,
`FUTURE_TABS`).

Mitigation:
1. Branch **from the v0.5 merge commit**, never from v0.4.2 — do not start E2/E3 before v0.5 lands.
2. Land **E1 first** (touches only `theme.rs` + `terminal.rs`, near-zero overlap) — it can even be
   merged before v0.5.
3. Ask the v0.5 agent to render the GPU tab through the existing per-tab pattern without touching
   the four other views; E3 then migrates all five uniformly.
4. `FUTURE_TABS` (`ui/mod.rs:26`) disappears in E4 — whoever gets there first removes it.

---

## 9. Deferred to v1.0

Config file (`~/.config/muxtop/config.toml`: theme, layout, keybindings, column sets), user themes,
saved layouts and per-tab widget arrangement, WASM plugin surface for custom tabs, session
persistence of sort/filter, i18n. None of them are blocked by this work — the token layer, the
keymap table and the column descriptors are exactly the three things a config file would need to
address, which is why they are introduced now.

---

## 10. Summary of behaviour changes (for the CHANGELOG)

**Breaking**
- `←`/`→` no longer switch tabs (use `Tab`/`Shift+Tab` or `Alt+1..6`); they now scroll columns.
- `F1`–`F5` no longer sort; the htop map applies (`F1` Help, `F5` Tree, `F6` Sort). `--legacy-keys` restores the old behaviour for one release.
- `t` and the F-key actions only apply on their own tab.
- `Esc` is progressive and no longer clears a filter in a single press from filter-edit mode.

**Fixed**
- Mouse wheel scrolls, and scrolls the tab you are looking at.
- Palette "Clear filter" clears the Kube filter on the Kube tab.
- Status severity is typed, not inferred from message text.
- 256-colour terminals get a 256-colour theme; `NO_COLOR` and `--no-color` are honoured.
- `+`/`-` renice now exists (it was documented but unimplemented).
- Palette commands accept arguments (`kill firefox`, `sort mem`) as the README already claimed.

**Added**
- `?` help overlay, which-key hints, `:` command mode, `Enter` inspector, `x` actions menu,
  `Space` pause, `r` refresh, `y` copy, toasts + `Ctrl+L` log, scrollbars, position indicators,
  live tab counts, General dashboard, `--theme` / `--ascii` / `--no-color` / `--no-mouse`.
