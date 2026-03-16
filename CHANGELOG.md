# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

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
