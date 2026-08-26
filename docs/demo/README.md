# AgentProcs terminal demo

![Animated terminal recording of AgentProcs starting dependent API and web services, routing a web request through stable localhost proxy URLs, automatically restarting the API after a controlled exit with status 42, reporting generation 2, and shutting down without residual processes, listeners, sockets, or state.](../assets/agent-procs-demo.gif)

This demo runs real local processes. It starts a small API and a web service, exposes them through AgentProcs' reverse proxy, makes HTTP requests, forces the API to exit with code 42, waits for automatic recovery, and verifies a clean shutdown.

## Prerequisites

- Linux or macOS
- Bash 3.2 or newer, Python 3.9 or newer, and a Rust toolchain with Cargo
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

`--record` builds AgentProcs in release mode and installs missing recording tools in a revision-specific directory beneath `${AGENT_PROCS_DEMO_TOOLS:-${TMPDIR:-/tmp}/agent-procs-demo-tools}`. Set `AGENT_PROCS_DEMO_TOOLS` to reuse another portable tool cache:

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

Each invocation generates a new `agent-procs-demo-<32 lowercase hex>` session with 128 random bits from Python's `secrets` module; the exact session shown in the recording therefore varies per run. Every AgentProcs command, including `up`, receives that session explicitly. Runtime files and `XDG_STATE_HOME` live in a fresh mode-0700 `mktemp` directory beneath `${TMPDIR:-/tmp}`, never in the repository. The toy services require the inherited `AGENT_PROCS_DEMO_RUNTIME` value and bind unconditionally to `127.0.0.1`.

After `up`, the script grants itself cleanup ownership only when JSON status contains exactly the expected, running `api` and `web` commands. Normal and signal-driven cleanup may call `down` after that validation. Before validation completes, cleanup stops only the named demo processes; it retires the random session only if status is then empty, otherwise it preserves session socket/PID artifacts for inspection and prints a warning.

The cleanup boundary is intentionally sized for accidental same-user collisions and clean interruption: the unpredictable per-run session and private temporary directory make accidental reuse negligible. Malicious same-user replacement races and root are out of scope; the script does not claim exact process identity or TOCTOU resistance. Before removing the run directory with Python's `shutil.rmtree`, it verifies a real, non-symlink directory owned by the current user, mode `0700`, with the expected basename directly beneath the resolved temporary directory. `INT` and `TERM` retain statuses `130` and `143` through one `EXIT` cleanup path.

## Pinned recording tools

The script verifies these versions and installs them with Cargo when needed:

- `asciinema 3.0.0` from crates.io: `cargo install --locked --version 3.0.0 asciinema`
- `agg 1.5.0` from immutable Git revision `5592b9790ba7c6d5ffa232176e29a1d3cadf8fe2`: `cargo install --locked --git https://github.com/asciinema/agg --rev 5592b9790ba7c6d5ffa232176e29a1d3cadf8fe2 agg`

The revision is the commit referenced by the upstream `v1.5.0` tag. The script installs into a path containing the full revision and parses Cargo's `.crates2.json` install record to require the exact Git source revision and `agg` binary, rather than trusting `agg --version` alone.
