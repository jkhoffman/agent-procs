# AgentProcs terminal demo

![Animated terminal recording of AgentProcs starting dependent API and web services, routing a web request through stable localhost proxy URLs, automatically restarting the API after a controlled exit with status 42, reporting generation 2, and shutting down without residual processes, listeners, sockets, or state.](../assets/agent-procs-demo.gif)

This demo runs real local processes. It starts a small API and a web service, exposes them through AgentProcs' reverse proxy, makes HTTP requests, forces the API to exit with code 42, waits for automatic recovery, and verifies a clean shutdown.

## Prerequisites

- Linux or macOS
- Bash, Python 3, and a Rust toolchain with Cargo
- `git` and network access when the pinned recording tools are not already installed
- DejaVu Sans Mono (used to render the GIF)

The demo binds only to loopback on ports `43111`, `43112`, and `49095`; those ports must be free. The proxy uses the wildcard `*.localhost` names `api.localhost` and `web.localhost`, which current Linux and macOS resolvers normally map to loopback without `/etc/hosts` changes.

## Play, record, and render

Run commands from the repository root.

```bash
# Build once, then play the live scenario without recording.
cargo build --locked --release
scripts/record-demo.sh --play

# Record a fresh asciicast and render the GIF.
scripts/record-demo.sh --record

# Re-render the existing asciicast without replaying the scenario.
scripts/record-demo.sh --render
```

`--record` builds AgentProcs in release mode and installs missing recording tools under `${AGENT_PROCS_DEMO_TOOLS:-${TMPDIR:-/tmp}/agent-procs-demo-tools}`. Set `AGENT_PROCS_DEMO_TOOLS` to reuse another tool directory:

```bash
AGENT_PROCS_DEMO_TOOLS="$HOME/.cache/agent-procs-demo-tools" scripts/record-demo.sh --record
```

Outputs are committed at:

- `docs/demo/agent-procs-demo.cast` — editable asciicast v2 source, recorded in a `100x30` PTY
- `docs/assets/agent-procs-demo.gif` — rendered animated demo

## Expected scenario

1. `agent-procs up` starts `api`, waits for its readiness message, then starts the dependent `web` service and reverse proxy.
2. `status` reports both processes running with stable named URLs.
3. A request to `web.localhost:49095` returns ready responses from the web service and the API it fetched upstream.
4. The API receives a controlled crash request and exits with status `42`.
5. AgentProcs logs `[agent-procs] Restarted`; the API comes back as generation `2`.
6. A final API request succeeds, `down` stops the session, and the script confirms that no demo processes, listeners, socket, PID file, runtime files, or isolated state remain.

The script traps normal exit, interruption, and termination and makes cleanup idempotent. Before each run it removes stale demo state, asks AgentProcs to stop the fixed `portfolio-demo` session, and refuses to start if a demo port is occupied. Runtime and AgentProcs state stay inside `docs/demo` while the demo is active and are deleted after verification.

## Pinned recording tools

The script verifies these versions and installs them with Cargo when needed:

- `asciinema 3.0.0` from crates.io: `cargo install --locked --version 3.0.0 asciinema`
- `agg 1.5.0` from immutable Git revision `5592b9790ba7c6d5ffa232176e29a1d3cadf8fe2`: `cargo install --locked --git https://github.com/asciinema/agg --rev 5592b9790ba7c6d5ffa232176e29a1d3cadf8fe2 agg`

The revision is the commit referenced by the upstream `v1.5.0` tag; the script deliberately installs by full revision rather than by the mutable tag name.
