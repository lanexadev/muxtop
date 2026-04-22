#!/usr/bin/env bash
# bench-thomas.sh — Macro-benchmark for muxtop
#
# Measures:
#   1. Release build time
#   2. Binary size
#   3. Startup latency across CLI entry points (--about / --version / --help / error)
#   4. Peak resident set size (RSS) during a headless bench run
#
# Usage:
#   ./scripts/bench-thomas.sh
#   just bench-thomas

set -euo pipefail

BOLD='\033[1m'
DIM='\033[2m'
GREEN='\033[32m'
CYAN='\033[36m'
RESET='\033[0m'

BIN="target/release/muxtop"
BENCH_RUN_SECS=30

printf "${BOLD}${CYAN}muxtop macro-benchmark (Thomas)${RESET}\n"
printf "${DIM}──────────────────────────────────${RESET}\n\n"

# 1. Build release
printf "${BOLD}[1/3] Building release binary...${RESET}\n"
build_start=$(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time()*1e9))')
cargo build --release --quiet 2>&1
build_end=$(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time()*1e9))')
build_ms=$(( (build_end - build_start) / 1000000 ))
printf "  Build time: ${GREEN}${build_ms}ms${RESET}\n"

# 2. Binary size
if [ -f "$BIN" ]; then
    size_bytes=$(wc -c < "$BIN" | tr -d ' ')
    if [ "$(uname)" = "Darwin" ]; then
        size_human=$(ls -lh "$BIN" | awk '{print $5}')
    else
        size_human=$(numfmt --to=iec "$size_bytes" 2>/dev/null || echo "${size_bytes}B")
    fi
    printf "  Binary size: ${GREEN}${size_human}${RESET} (${size_bytes} bytes)\n"
else
    printf "  Binary not found at ${BIN}\n"
    exit 1
fi

# 3. Startup latency
printf "\n${BOLD}[2/3] Measuring startup latency...${RESET}\n"

# Warm-up: the first execution of a freshly-built binary on macOS pays a
# Gatekeeper / XProtect scan (~500 ms) that has nothing to do with our code.
# Discard the first run so subsequent measurements reflect actual startup.
"$BIN" --version > /dev/null 2>&1

# We can't run the TUI interactively, so we measure --about (instant exit).
start=$(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time()*1e9))')
"$BIN" --about > /dev/null 2>&1
end=$(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time()*1e9))')
startup_ms=$(( (end - start) / 1000000 ))
printf "  Startup (--about): ${GREEN}${startup_ms}ms${RESET}\n"

# 4. --version
start=$(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time()*1e9))')
"$BIN" --version > /dev/null 2>&1
end=$(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time()*1e9))')
version_ms=$(( (end - start) / 1000000 ))
printf "  Startup (--version): ${GREEN}${version_ms}ms${RESET}\n"

# 5. --help
start=$(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time()*1e9))')
"$BIN" --help > /dev/null 2>&1
end=$(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time()*1e9))')
help_ms=$(( (end - start) / 1000000 ))
printf "  Startup (--help): ${GREEN}${help_ms}ms${RESET}\n"

# 6. Invalid flag (error path)
start=$(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time()*1e9))')
"$BIN" --invalid-flag > /dev/null 2>&1 || true
end=$(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time()*1e9))')
error_ms=$(( (end - start) / 1000000 ))
printf "  Error path (bad flag): ${GREEN}${error_ms}ms${RESET}\n"

# 7. Peak RSS during a ${BENCH_RUN_SECS}s headless collector session.
#    --bench-run spawns the collector and feeds snapshots through AppState
#    (apply_snapshot + recompute_visible) without a TUI, then exits cleanly.
printf "\n${BOLD}[3/3] Measuring peak RSS over ${BENCH_RUN_SECS}s headless run...${RESET}\n"
rss_tmp=$(mktemp)
trap 'rm -f "$rss_tmp"' EXIT
if [ "$(uname)" = "Darwin" ]; then
    # macOS: /usr/bin/time -l reports "maximum resident set size" in bytes.
    /usr/bin/time -l "$BIN" --bench-run "$BENCH_RUN_SECS" > /dev/null 2>"$rss_tmp"
    rss_bytes=$(awk '/maximum resident set size/ {print $1}' "$rss_tmp")
    if [ -n "${rss_bytes:-}" ]; then
        rss_mib=$(awk -v b="$rss_bytes" 'BEGIN {printf "%.1f", b/1048576}')
        printf "  Peak RSS: ${GREEN}${rss_mib} MiB${RESET} (${rss_bytes} bytes)\n"
    else
        printf "  Peak RSS: ${GREEN}unknown${RESET} (could not parse time output)\n"
        rss_mib="?"
    fi
elif command -v /usr/bin/time > /dev/null 2>&1; then
    # Linux GNU time -v reports "Maximum resident set size (kbytes)".
    /usr/bin/time -v "$BIN" --bench-run "$BENCH_RUN_SECS" > /dev/null 2>"$rss_tmp"
    rss_kb=$(awk -F': ' '/Maximum resident set size/ {print $2}' "$rss_tmp")
    if [ -n "${rss_kb:-}" ]; then
        rss_mib=$(awk -v k="$rss_kb" 'BEGIN {printf "%.1f", k/1024}')
        printf "  Peak RSS: ${GREEN}${rss_mib} MiB${RESET} (${rss_kb} KiB)\n"
    else
        printf "  Peak RSS: ${GREEN}unknown${RESET} (could not parse time output)\n"
        rss_mib="?"
    fi
else
    printf "  Peak RSS: ${GREEN}unsupported${RESET} (/usr/bin/time not available)\n"
    rss_mib="?"
fi

# Summary
printf "\n${BOLD}${CYAN}Summary${RESET}\n"
printf "${DIM}──────────────────────────────────${RESET}\n"
printf "  Build:        %6d ms\n" "$build_ms"
printf "  Binary:       %s\n" "$size_human"
printf "  --about:      %6d ms\n" "$startup_ms"
printf "  --version:    %6d ms\n" "$version_ms"
printf "  --help:       %6d ms\n" "$help_ms"
printf "  error path:   %6d ms\n" "$error_ms"
printf "  Peak RSS:     %s MiB (over ${BENCH_RUN_SECS}s)\n" "$rss_mib"
printf "\n${GREEN}Done.${RESET}\n"
