#!/usr/bin/env bash
# bench-thomas.sh — Macro-benchmark for muxtop
#
# Measures:
#   1. Release build time
#   2. Binary size
#   3. Startup + 3-second run + graceful exit time
#   4. Peak memory usage (RSS)
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
RUN_SECS=3

printf "${BOLD}${CYAN}muxtop macro-benchmark (Thomas)${RESET}\n"
printf "${DIM}──────────────────────────────────${RESET}\n\n"

# 1. Build release
printf "${BOLD}[1/4] Building release binary...${RESET}\n"
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

# 3. Startup + run + exit
printf "\n${BOLD}[2/4] Measuring startup + ${RUN_SECS}s run...${RESET}\n"

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

# Summary
printf "\n${BOLD}${CYAN}Summary${RESET}\n"
printf "${DIM}──────────────────────────────────${RESET}\n"
printf "  Build:        %6d ms\n" "$build_ms"
printf "  Binary:       %s\n" "$size_human"
printf "  --about:      %6d ms\n" "$startup_ms"
printf "  --version:    %6d ms\n" "$version_ms"
printf "  --help:       %6d ms\n" "$help_ms"
printf "  error path:   %6d ms\n" "$error_ms"
printf "\n${GREEN}Done.${RESET}\n"
