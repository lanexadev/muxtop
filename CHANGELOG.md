# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-04-14

### Added
- **Tabbed Interface**: Switch between General (System Overview) and Processes tabs with `Alt+1`/`Alt+2`.
- **Command Palette**: Natural language-style command bar (`Ctrl+P`) to sort, filter, and kill processes.
- **Process Tree View**: Hierarchical parent/child display toggled with `F5`.
- **Fuzzy Search**: High-performance process filtering powered by the `nucleo` library.
- **htop-compatible keybindings**: Familiar shortcuts for search, filter, sort, kill, and quit.
- **Async Collector**: Non-blocking data collection using `tokio` and `sysinfo`.
- **Performance Guarantees**: Startup time under 100ms and memory usage under 10MB.
- **Zero Telemetry**: Guaranteed privacy with no outbound network calls.
- **Installation Scripts**: One-liner install script and `crates.io` support.
