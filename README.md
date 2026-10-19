Rust process supervisor for agent-driven development: daemon persistence across CLI invocations, dependency-ordered startup, readiness checks, durable logs, restart policies.

Coding agents often start an API, web app, or worker in one shell, then lose track of the process on a later tool call. AgentProcs keeps those services under a per-session daemon, so the next call can read logs, check status, restart a process, or stop everything cleanly. It is for developers using agentic coding workflows on Linux and macOS (Unix only), and it supervises processes without sandboxing them.

## Install

AgentProcs needs Rust 1.88 or newer. To run the code on the main branch, install from a clone:

```bash
git clone https://github.com/jkhoffman/agent-procs.git
cd agent-procs
cargo install --path . --locked
```

`cargo install agent-procs` installs the latest crates.io release, 0.6.3, which does not include the unreleased changes listed in [CHANGELOG.md](CHANGELOG.md).

## Usage

```bash
agent-procs run "python3 -u -m http.server 8000" --name server
agent-procs wait server --until "Serving HTTP" --timeout 30
agent-procs logs server --tail 50
agent-procs status
agent-procs stop server
```

Each command is a separate invocation. The first `run` starts the daemon, and later commands talk to it. For multi-process projects, put an `agent-procs.yaml` next to your code and use `agent-procs up` and `agent-procs down`. The config file, every command, restart policies, file watching, sessions, and the reverse proxy are covered in [docs/reference.md](https://github.com/jkhoffman/agent-procs/blob/main/docs/reference.md). For how it compares with tmux, PM2, Docker Compose and similar tools, see [docs/alternatives.md](https://github.com/jkhoffman/agent-procs/blob/main/docs/alternatives.md).

## Architecture

Each CLI invocation is a short-lived client. A per-session daemon outlives the invocations, owns the control plane, and supervises each managed command as a process group, writing durable logs to disk. The CLI reaches the daemon over a Unix domain socket whose `0700` parent directory limits access to the current user. The main design decision is that all state lives in the daemon and on disk, not in the CLI, which is why a later agent tool call can pick up where an earlier one stopped. Managed commands keep your filesystem and network access, so use a container or VM when you need isolation. See [docs/architecture.md](https://github.com/jkhoffman/agent-procs/blob/main/docs/architecture.md) and [SECURITY.md](https://github.com/jkhoffman/agent-procs/blob/main/SECURITY.md) for details.

## Tests

```bash
cargo test --locked --all-features -- --test-threads=1
```

The tests start real daemons and processes, so `--test-threads=1` runs them one at a time.
