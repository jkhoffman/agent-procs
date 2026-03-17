# Contributing to agent-procs

Thank you for your interest in contributing to agent-procs. This guide covers
everything you need to get started.

## License

This project is licensed under the [MIT License](LICENSE). By contributing, you
agree that your contributions will be licensed under the same terms.

## Development setup

### Prerequisites

- Rust toolchain (MSRV: **1.88**)
- A Unix-like OS (macOS or Linux; the daemon uses Unix domain sockets)

### Building and testing

```bash
# Build the project
cargo build

# Run the full test suite
cargo test

# Run clippy (must pass with zero warnings)
cargo clippy --all-targets -- -D warnings

# Format all source files
cargo fmt
```

All four commands must pass before submitting a pull request. CI runs the same
checks.

## Code style

The project uses **clippy pedantic** linting, configured in `Cargo.toml` under
`[lints.clippy]`. A handful of pedantic lints are selectively allowed (e.g.,
`module_name_repetitions`, `wildcard_imports`). If you believe an additional
lint should be allowed, mention it in your PR description with a rationale.

Run `cargo fmt` before committing. The project uses default rustfmt settings.

## Test conventions

Integration tests live in `tests/` and cover real daemon interactions. They do
not use mocks -- tests spawn actual daemon processes.

### TestContext helper

Tests use the `TestContext` helper defined in `tests/helpers/mod.rs`. It:

- Kills any stale daemon for the given session name before the test starts
- Creates an isolated temporary state directory (`XDG_STATE_HOME`)
- Cleans up the daemon (via `SIGTERM`) when the context is dropped

Usage:

```rust
use crate::helpers::TestContext;

#[test]
fn test_something() {
    let ctx = TestContext::new("unique_session_name");
    ctx.set_env();
    // ... test code ...
}
```

### Session naming

Each test must use a **unique session name** to avoid conflicts when tests run
in parallel. Use a descriptive name that reflects the test, for example
`"proxy_routing"` or `"wait_timeout"`.

## Commit messages

This project follows the [Conventional Commits](https://www.conventionalcommits.org/)
format:

```
<type>: <short description>
```

Supported types:

| Type       | Purpose                                   |
|------------|-------------------------------------------|
| `feat`     | New feature                               |
| `fix`      | Bug fix                                   |
| `docs`     | Documentation changes                     |
| `refactor` | Code change that neither fixes nor adds   |
| `test`     | Adding or updating tests                  |
| `style`    | Formatting, whitespace, lint fixes        |
| `chore`    | Build, CI, dependencies, tooling          |

Keep the first line under 72 characters. Use the body for additional context
when needed.

## Pull request process

1. **Fork** the repository and create a feature branch from `main`.
2. **Make your changes** in small, focused commits using conventional commit
   messages.
3. **Run all checks** locally:
   ```bash
   cargo fmt
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```
4. **Push** your branch and open a pull request against `main`.
5. **Describe** what your PR does, why, and how to test it.
6. Address any review feedback. Once approved, a maintainer will merge.

## Reporting issues

Use GitHub Issues. Include:

- What you did (steps to reproduce)
- What you expected
- What actually happened
- OS, Rust version, and agent-procs version
