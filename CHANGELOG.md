# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.6.0] - 2026-03-17

### Added

- **Restart policies**: `--autorestart always|on-failure|never` with
  `--max-restarts` and `--restart-delay` for automatic crash recovery.
  Processes transition to `Failed` state when max restarts are exhausted.
- **File watch mode**: `--watch "src/**"` auto-restarts processes when watched
  files change. Uses OS-native file watching (FSEvents on macOS, inotify on
  Linux) with 500ms debounce. Default ignore list covers `.git`, `node_modules`,
  `target`, `__pycache__`.
- **Supervisor annotations**: synthetic log lines (`[agent-procs] Restarted`,
  `[agent-procs] Max restarts exhausted`, `[agent-procs] File changed`) written
  to disk logs via per-process mpsc supervisor channel, visible in `logs --tail`,
  TUI, and `--follow`.
- **`ProcessState::Failed`** and **`ProcessState::Unknown`** variants with
  custom `Deserialize` for forward compatibility.
- **`respawn_in_place()`**: drains capture tasks, rotates logs, re-spawns with
  original args, carries over supervisor metadata. Creates tombstone record on
  spawn failure.
- **RESTARTS and WATCH columns** in `status` output (shown conditionally).
- **Restart count** (`2/5↻`) and **watch indicator** (`W`) in TUI process list.
- **Failed count** in TUI status bar.
- Config file fields: `autorestart`, `max_restarts`, `restart_delay`, `watch`,
  `watch_ignore`.
- CLI flags: `--autorestart`, `--max-restarts`, `--restart-delay`, `--watch`,
  `--watch-ignore`.
- New dependencies: `notify` 7 (file watching), `globset` 0.4 (glob matching).

### Changed

- `capture_output()` now accepts an mpsc supervisor channel for synthetic log
  lines. Uses `tokio::select!` to interleave pipe reads with supervisor messages.
- `ManagedProcess` extended with supervisor fields: `restart_policy`,
  `watch_config`, `restart_count`, `manually_stopped`, `restart_pending`,
  `failed`, `supervisor_tx`, `capture_handles`, `watch_handle`.
- Actor stores a self-sender for scheduling delayed `AutoRestart` commands.
- `stop` sets `manually_stopped` flag; `restart` clears it along with
  `restart_count` and `failed`.

## [0.5.1] - 2026-03-17

### Fixed

- `wait --exit` now detects quiet process exits instead of timing out while
  waiting for a log line.
- Proxy route state and auto-assigned port tracking now refresh when processes
  exit naturally, preventing stale routes from lingering.
- `logs --follow` now honors `--stderr` and replays `--tail` output before live
  streaming instead of silently ignoring those flags.
- Daemon startup failures now report early-exit diagnostics instead of only
  surfacing a generic 5-second timeout.
- Integration tests now isolate `XDG_STATE_HOME` per spawned CLI process so the
  suite is stable under the default parallel `cargo test` settings.

## [0.5.0] - 2026-03-17

### Added

- **Disk-backed virtual scrollback**: the TUI can now scroll through the entire
  output history of a process, not just the last 10,000 lines held in memory.
  The daemon writes a binary sidecar `.idx` file alongside each log file,
  mapping line numbers to byte offsets for O(1) random-access reads.
- **Sidecar line index format** (`LIDX`): 16-byte header + 16-byte records with
  byte offset and global sequence number per line. Enables direct seeking to any
  line without scanning the log file.
- **Interleaved "Both" mode** uses per-process atomic sequence counters shared
  between stdout/stderr capture tasks, so merge-sorting by sequence number
  produces correct chronological ordering across streams.
- **`DiskLogReader`** provides windowed random-access reads across current and
  rotated log files, with automatic fallback to sequential scan when index files
  are missing (backwards compatible with pre-0.5 logs).
- **Segment cache** (500 ms TTL) in `DiskLogReader` eliminates repeated
  filesystem stat calls within a render frame.
- **`OutputBuffer` per-source counters** (`stdout_count`, `stderr_count`) for
  O(1) line counting without iterating the ring buffer.

### Changed

- `capture_output()` now accepts an `Arc<AtomicU64>` sequence counter and writes
  index entries alongside log lines. Index is flushed every 64 lines (not per
  line) to reduce syscalls while keeping the sidecar reasonably fresh.
- `rotate_log_files()` now rotates `.idx` companion files alongside log files.
- TUI startup replaced `load_historical_logs()` with `init_disk_readers()` which
  creates `DiskLogReader` instances and pre-populates the hot buffer.
- TUI rendering uses windowed `visible_lines()` — only the visible window of
  lines is fetched (from disk or hot buffer), eliminating the previous approach
  of collecting all lines and using `Paragraph::scroll()`.
- When a filter is active, disk-backed scrollback is bypassed and the TUI falls
  back to the 10,000-line hot buffer (filtered disk scrollback is a follow-up).
- MSRV raised from 1.85 to 1.88 (required by `ansi-to-tui` v7 dependencies).
- CI tests run single-threaded to prevent env var races in path tests.
- CI security audit job granted `checks: write` permission.

## [0.4.1] - 2026-03-17

### Fixed

- TUI now renders ANSI colors natively via `ansi-to-tui` instead of stripping
  them -- process output shows original colors (Vite green, Elixir log levels).
- Trackpad/mouse scroll moves 3 lines per event instead of half a page,
  preventing content from flying past on swipe.
- Scroll offset clamped to content bounds -- scrolling past the top no longer
  accumulates invisible debt requiring equal downward scrolling to undo.

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

[Unreleased]: https://github.com/jkhoffman/agent-procs/compare/v0.4.1...HEAD
[0.4.1]: https://github.com/jkhoffman/agent-procs/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/jkhoffman/agent-procs/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/jkhoffman/agent-procs/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/jkhoffman/agent-procs/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/jkhoffman/agent-procs/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/jkhoffman/agent-procs/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/jkhoffman/agent-procs/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/jkhoffman/agent-procs/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/jkhoffman/agent-procs/releases/tag/v0.1.0
