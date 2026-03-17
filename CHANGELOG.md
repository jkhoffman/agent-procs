# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- TUI now renders ANSI colors natively via `ansi-to-tui` instead of stripping
  them -- process output shows original colors (Vite green, Elixir log levels).

### Changed

- CI release workflow publishes to crates.io automatically on tag push
  (requires `CARGO_REGISTRY_TOKEN` secret).
- Added pre-commit hook (fmt, clippy, test) in `.githooks/`.

## [0.4.0] - 2026-03-17

### Added

- **Typed `ErrorCode` enum** (`General` / `NotFound`) replacing raw `i32` error
  codes, wire-compatible via `serde(into = "i32", from = "i32")`.
- **Protocol versioning**: `PROTOCOL_VERSION` constant, `Hello` / `Unknown`
  variants on both `Request` and `Response` with `#[serde(other)]` for forward
  compatibility.
- **Centralized `process_url()`** for URL construction (removes 4 duplicate
  `format!` calls).
- **`request_and_handle()` CLI helper** eliminating error-dispatch boilerplate
  in `stop`, `restart`, `run`, `status`, and `wait` commands.
- **Actor / channel model** (`ProcessManagerActor` + `PmHandle`) replacing
  `Arc<Mutex<DaemonState>>` -- commands are processed sequentially via an
  `mpsc` channel with `oneshot` reply channels.
- **Lock-free proxy port lookup** via `watch` channel for the reverse proxy hot
  path.
- **`PortAllocator`** extracted from `ProcessManager` into its own module.
- **N-file cascading log rotation** (default 5 rotated files, up from 1).
- **`TuiEventLoop`** extracted from `tui/mod.rs` with `ReconnectState` state
  machine replacing `Arc<AtomicU32>` approach.
- **Hidden `RunDaemon` clap subcommand** replacing raw `std::env::args()`
  parsing.

### Changed

- `OutputBuffer` methods (`stdout_lines`, `stderr_lines`, `all_lines`) return
  iterators instead of `Vec` allocations.
- TUI loads last 1000 lines via `tail_file` instead of reading entire log
  files on startup.
- Proxy status page now correctly shows proxy URLs (was showing direct
  `127.0.0.1` URLs).
- `proxy_port` consolidated as single source of truth inside the actor
  (removed redundant `Arc<Mutex>`).
- Proxy state watch channel skips no-op updates via change detection.

### Removed

- `DaemonState` struct (replaced by actor model).
- Dead `get_process_port` method (superseded by `running_ports()`).

## [0.3.1] - 2026-03-16

### Added

- Mouse support in TUI (scroll wheel, click to select process).
- Scroll and filter in TUI output pane (vim-style `u`/`d` half-page scroll,
  `g`/`G` top/bottom, `/` to filter).

### Fixed

- `Q` in TUI sends `Shutdown` (stop all + quit) instead of `StopAll`.

## [0.3.0] - 2026-03-16

### Added

- `#![deny(unsafe_code)]` crate-wide with targeted allow on `pre_exec` call.
- Clippy pedantic lints enabled via `[lints.clippy]` in Cargo.toml.
- `#[must_use]` annotations on key public functions and types.
- Module-level doc comments (`//!`) on all modules.
- Doc tests on `startup_order`, `Request`, and `Response`.
- Typed error enums (`ConfigError`, `ClientError`, `ProxyError`) replacing `Result<T, String>`.
- 33 unit tests across config, process_manager, protocol, session, paths, and tui/app.
- 5 property-based tests with proptest (protocol roundtrips, DNS validation, config parsing).
- Criterion benchmarks for config parsing, protocol serde, and DNS validation.
- Structured logging with `tracing` in daemon code (writes to `daemon.log`).
- Shell completion generation (`completions` subcommand) via `clap_complete`.
- `cargo-audit` security audit job in CI.
- MSRV CI job testing with Rust 1.85.
- Coverage reporting with `cargo-llvm-cov` and Codecov.
- Binary release automation (`.github/workflows/release.yml`) for 4 targets.
- CONTRIBUTING.md, CHANGELOG.md, SECURITY.md.
- GitHub issue templates and PR template.
- Dockerfile for containerized builds.
- Homebrew formula template.
- Message size limit (1 MiB) on daemon socket reads.
- Connection rate limiting (max 64 concurrent connections) with atomic counter.

### Changed

- Edition upgraded from 2021 to 2024; MSRV raised to 1.85.
- `load_config` and `startup_order` now return typed `ConfigError` instead of `String`.
- `connect`, `request`, and `stream_responses` now return `ClientError` instead of `String`.
- `bind_proxy_port` now returns `ProxyError` instead of `String`.
- `start_proxy` now returns `std::io::Result<()>` instead of `Result<(), String>`.
- `session list` and `session clean` are no longer async (no await needed).
- Daemon `eprintln!` calls replaced with structured `tracing` macros.
- Wait handler historical log scan compiles regex once and uses single-pass find.
- Deduplicated request-send logic in CLI client (`send_request` helper).
- Deduplicated PID-file iteration in session commands (`pid_entries` helper).

## [0.2.2] - 2026-03-16

### Fixed

- Check historical logs in Wait handler to fix race condition where a pattern
  that already appeared would never be detected.
- Address code review feedback on reverse proxy implementation.

### Changed

- Extract `enable_proxy` helper and eliminate TOCTOU double-bind in proxy
  startup.
- Apply `cargo fmt` formatting fixes.

### Documentation

- Document reverse proxy feature in README, CLI `--help` output, and skill
  file.

## [0.2.1] - 2026-03-16

### Fixed

- TUI now loads all historical log output instead of only the last 200 lines.
- TUI loads historical output from log files on startup so existing output is
  visible immediately.

## [0.2.0] - 2026-03-16

### Added

- **Reverse proxy** with subdomain routing -- run `agent-procs up --proxy` and
  access processes via `http://<name>.localhost:9090`.
- Port auto-assignment with bind-test to find available ports.
- DNS label validation when the proxy is active (process names must be valid
  subdomains).
- `--port` and `--proxy` flags on the `run` command.
- `--proxy` flag on the `up` command; port read from config.
- URL column in `status` output showing the proxy URL for each process.
- `port` and `url` fields in protocol messages (`RunOk`, `ProcessInfo`).
- `EnableProxy` request variant in the daemon protocol.
- `proxy` and `port` fields in config file format.
- Integration tests for reverse proxy routing.

### Fixed

- Address 5 bugs, crash sites, and tech debt items.
- `restart` preserves original working directory and environment variables.
- `down` command now shuts down the daemon.

## [0.1.1] - 2026-03-16

### Fixed

- `stop_all` deregisters processes and honors the `session` field in config
  files.

### Added

- GitHub Actions CI workflow.
- MIT license and crates.io metadata.

## [0.1.0] - 2026-03-15

Initial release.

### Added

- **Daemon architecture** -- background daemon per session communicating over
  Unix domain sockets.
- **Process lifecycle management** -- `run`, `stop`, `restart`, and `status`
  commands.
- **Log capture** with 50 MB rotation and `logs` command with `--tail`,
  `--stderr`, and `--all` flags.
- **Wait command** with pattern matching (literal and regex), exit detection,
  and configurable timeout.
- **Config-driven startup** -- `up` / `down` commands read
  `agent-procs.yaml` and start processes with dependency ordering via
  topological sort.
- **Session isolation** -- `--session` flag for running multiple independent
  daemon instances.
- **Terminal UI** (`ui` command) with split-pane layout and live log
  streaming.
- **`--follow` log streaming** with `--lines` and `--timeout` options.
- **Environment variable passing** through the protocol.
- **Process groups** for clean signal handling -- `SIGTERM` is sent to the
  entire process tree on stop.
- XDG-compliant path resolution for state directories.
- CLI `--help` with workflow examples, daemon model, exit codes, and config
  format documentation.

[Unreleased]: https://github.com/jkhoffman/agent-procs/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/jkhoffman/agent-procs/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/jkhoffman/agent-procs/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/jkhoffman/agent-procs/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/jkhoffman/agent-procs/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/jkhoffman/agent-procs/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/jkhoffman/agent-procs/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/jkhoffman/agent-procs/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/jkhoffman/agent-procs/releases/tag/v0.1.0
