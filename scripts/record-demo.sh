#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

ROOT=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
BINARY="$ROOT/target/release/agent-procs"
CONFIG="docs/demo/agent-procs.yaml"
DEMO_DIR="$ROOT/docs/demo"
RUNTIME="$DEMO_DIR/.demo-runtime"
STATE_HOME="$DEMO_DIR/.agent-procs-state"
TOOLS_BASE=${AGENT_PROCS_DEMO_TOOLS:-${TMPDIR:-/tmp}/agent-procs-demo-tools}
ASCIINEMA_VERSION=3.0.0
AGG_VERSION=1.5.0
AGG_REVISION=5592b9790ba7c6d5ffa232176e29a1d3cadf8fe2
TOOLS_ROOT="$TOOLS_BASE/asciinema-$ASCIINEMA_VERSION-agg-$AGG_REVISION"
CAST="$DEMO_DIR/agent-procs-demo.cast"
GIF="$ROOT/docs/assets/agent-procs-demo.gif"
SESSION=agent-procs-portfolio-demo
SOCKET_BASE="/tmp/agent-procs-$(id -u)"
SOCKET="$SOCKET_BASE/$SESSION.sock"
PID_FILE="$SOCKET_BASE/$SESSION.pid"
OWNER_MARKER="$SOCKET_BASE/$SESSION.demo-owner"
OWNER_TOKEN="pid=$$;repo=$ROOT;session=$SESSION"
PORTS=(43111 43112 49095)
export XDG_STATE_HOME="$STATE_HOME"
export TERM=xterm-256color
export NO_COLOR=1

CLEANED=0
OWNED=0
MANAGE_SESSION=0

pause() {
  local seconds=$1
  local delay=${DEMO_DELAY:-1}
  python3 -c 'import sys, time; time.sleep(float(sys.argv[1]) * float(sys.argv[2]))' "$seconds" "$delay"
}

prompt() {
  printf '\n\033[1;36m❯ %s\033[0m\n' "$1"
  pause 0.7
}

ap() {
  "$BINARY" "$@"
}

owner_matches() {
  [[ $OWNED -eq 1 && -f "$OWNER_MARKER" && ! -L "$OWNER_MARKER" ]] || return 1
  python3 - "$OWNER_MARKER" "$OWNER_TOKEN" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
expected = (sys.argv[2] + "\n").encode()
try:
    actual = path.read_bytes()
except OSError:
    raise SystemExit(1)
raise SystemExit(0 if actual == expected else 1)
PY
}

remove_owned_artifacts() {
  owner_matches || return 1
  rm -rf -- "$RUNTIME" "$STATE_HOME" || return 1
  owner_matches || return 1
  rm -f -- "$OWNER_MARKER" || return 1
  OWNED=0
}

remove_owner_marker() {
  owner_matches || return 1
  rm -f -- "$OWNER_MARKER" || return 1
  OWNED=0
}

cleanup() {
  local _status=$?
  if [[ $CLEANED -eq 1 ]]; then
    return "$_status"
  fi
  CLEANED=1

  if owner_matches; then
    if [[ $MANAGE_SESSION -eq 1 ]]; then
      "$BINARY" --session "$SESSION" down >/dev/null 2>&1 || true
      local attempt
      for attempt in {1..50}; do
        if [[ ! -e "$SOCKET" && ! -e "$PID_FILE" ]] && assert_ports_free 2>/dev/null; then
          break
        fi
        if (( attempt % 10 == 0 )) && owner_matches; then
          "$BINARY" --session "$SESSION" down >/dev/null 2>&1 || true
        fi
        sleep 0.1
      done
      if [[ ! -e "$SOCKET" && ! -e "$PID_FILE" ]] && assert_ports_free 2>/dev/null; then
        remove_owned_artifacts || true
      fi
    else
      # Acquisition lost a race before this script was allowed to manage the
      # session. Relinquish only our marker; preserve every collision path.
      remove_owner_marker || true
    fi
  fi
  return "$_status"
}

on_int() {
  exit 130
}

on_term() {
  exit 143
}

trap cleanup EXIT
trap on_int INT
trap on_term TERM

assert_ports_free() {
  python3 - "${PORTS[@]}" <<'PY'
import socket
import sys

for value in sys.argv[1:]:
    port = int(value)
    with socket.socket() as sock:
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        sock.bind(("127.0.0.1", port))
PY
}

refuse_existing_session() {
  local path
  for path in "$SOCKET" "$PID_FILE" "$OWNER_MARKER" "$RUNTIME" "$STATE_HOME"; do
    if [[ -e "$path" || -L "$path" ]]; then
      printf 'error: refusing to use session %s: pre-existing path %s\n' "$SESSION" "$path" >&2
      printf 'Remove it manually only after confirming no foreign or interrupted session owns it.\n' >&2
      return 1
    fi
  done
}

acquire_ownership() {
  refuse_existing_session
  mkdir -p -- "$SOCKET_BASE"
  if [[ ! -d "$SOCKET_BASE" || -L "$SOCKET_BASE" || ! -O "$SOCKET_BASE" ]]; then
    printf 'error: unsafe socket directory: %s\n' "$SOCKET_BASE" >&2
    return 1
  fi
  chmod 700 "$SOCKET_BASE"

  if ! (set -o noclobber; printf '%s\n' "$OWNER_TOKEN" >"$OWNER_MARKER") 2>/dev/null; then
    printf 'error: refusing to use session %s: ownership marker already exists\n' "$SESSION" >&2
    return 1
  fi
  OWNED=1

  # Close the pre-check/acquire race. Never call `down` if another session
  # published artifacts before this process established ownership.
  local path
  for path in "$SOCKET" "$PID_FILE" "$RUNTIME" "$STATE_HOME"; do
    if [[ -e "$path" || -L "$path" ]]; then
      printf 'error: session collision detected after ownership acquisition: %s\n' "$path" >&2
      return 1
    fi
  done
}

verify_cleanup() {
  local attempt
  for attempt in {1..50}; do
    if [[ ! -e "$SOCKET" && ! -e "$PID_FILE" ]] && assert_ports_free 2>/dev/null; then
      break
    fi
    if (( attempt % 10 == 0 )) && owner_matches; then
      "$BINARY" --session "$SESSION" down >/dev/null 2>&1 || true
    fi
    sleep 0.1
  done

  [[ ! -e "$SOCKET" ]]
  [[ ! -e "$PID_FILE" ]]
  assert_ports_free

  python3 - "$RUNTIME/status-before-down.json" <<'PY'
import json
import os
import sys

status_path = sys.argv[1]
with open(status_path, encoding="utf-8") as stream:
    processes = json.load(stream)
for process in processes:
    pid = int(process["pid"])
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        continue
    raise SystemExit(f"process still alive: {pid}")
PY

  remove_owned_artifacts
  [[ ! -e "$RUNTIME" && ! -e "$STATE_HOME" && ! -e "$OWNER_MARKER" ]]
}

play_demo() {
  cd "$ROOT"
  acquire_ownership
  assert_ports_free
  MANAGE_SESSION=1
  mkdir -p "$RUNTIME"

  printf '\033[2J\033[H'
  printf '\033[1;37mAgentProcs: resilient local agent workflow\033[0m\n'
  printf '\033[0;37mTwo dependent services · stable URLs · automatic crash recovery\033[0m\n'
  pause 3

  prompt 'agent-procs up --config docs/demo/agent-procs.yaml'
  ap up --config "$CONFIG"
  pause 4

  prompt 'agent-procs --session agent-procs-portfolio-demo status'
  ap --session "$SESSION" status
  pause 4

  prompt 'python3 docs/demo/toy_service.py request web'
  python3 docs/demo/toy_service.py request web
  pause 4

  prompt 'agent-procs --session agent-procs-portfolio-demo logs --all --tail 8'
  ap --session "$SESSION" logs --all --tail 8
  pause 3

  printf '\n\033[1;37mNow trigger a controlled API failure...\033[0m\n'
  prompt 'python3 docs/demo/toy_service.py crash'
  python3 docs/demo/toy_service.py crash
  pause 2

  prompt 'agent-procs --session agent-procs-portfolio-demo wait api --until "[agent-procs] Restarted" --timeout 10'
  ap --session "$SESSION" wait api --until '[agent-procs] Restarted' --timeout 10
  pause 3

  prompt 'agent-procs --session agent-procs-portfolio-demo logs api --tail 8'
  ap --session "$SESSION" logs api --tail 8
  pause 5

  prompt 'agent-procs --session agent-procs-portfolio-demo status'
  ap --session "$SESSION" status
  pause 4

  prompt 'python3 docs/demo/toy_service.py request api'
  python3 docs/demo/toy_service.py request api
  pause 4

  ap --session "$SESSION" status --json >"$RUNTIME/status-before-down.json"
  prompt 'agent-procs --session agent-procs-portfolio-demo down'
  ap --session "$SESSION" down
  pause 3

  verify_cleanup
  MANAGE_SESSION=0
  CLEANED=1
  printf '\n\033[1;32m✓ clean shutdown verified\033[0m\n'
  printf '  no demo processes · no listeners · no socket, state, runtime, or owner files\n'
  pause 6
}

agg_provenance_ok() {
  [[ -x "$TOOLS_ROOT/bin/agg" && -f "$TOOLS_ROOT/.crates2.json" ]] || return 1
  [[ $("$TOOLS_ROOT/bin/agg" --version) == "agg $AGG_VERSION" ]] || return 1
  python3 - "$TOOLS_ROOT/.crates2.json" "$AGG_VERSION" "$AGG_REVISION" <<'PY'
import json
import sys

metadata_path, version, revision = sys.argv[1:]
with open(metadata_path, encoding="utf-8") as stream:
    installs = json.load(stream).get("installs", {})
expected = (
    f"agg {version} (git+https://github.com/asciinema/agg"
    f"?rev={revision}#{revision})"
)
valid = "agg" in installs.get(expected, {}).get("bins", [])
raise SystemExit(0 if valid else 1)
PY
}

install_tools() {
  mkdir -p "$TOOLS_BASE"
  if [[ ! -x "$TOOLS_ROOT/bin/asciinema" ]] || [[ $("$TOOLS_ROOT/bin/asciinema" --version) != "asciinema $ASCIINEMA_VERSION" ]]; then
    cargo install --locked --root "$TOOLS_ROOT" --version "$ASCIINEMA_VERSION" asciinema
  fi
  if ! agg_provenance_ok; then
    rm -f -- "$TOOLS_ROOT/bin/agg"
    cargo install --locked --root "$TOOLS_ROOT" --git https://github.com/asciinema/agg --rev "$AGG_REVISION" agg
  fi
  agg_provenance_ok || {
    printf 'error: cached agg does not prove Git revision %s\n' "$AGG_REVISION" >&2
    return 1
  }
}

record_demo() {
  cd "$ROOT"
  cargo build --locked --release >/dev/null
  install_tools
  mkdir -p "$(dirname -- "$GIF")"
  DEMO_DELAY=1 "$TOOLS_ROOT/bin/asciinema" record \
    --quiet --headless --return --overwrite --output-format asciicast-v2 \
    --window-size 100x30 --idle-time-limit 6 \
    --title 'AgentProcs resilient agent workflow' \
    --command "bash scripts/record-demo.sh --play" "$CAST"
  render_demo
}

render_demo() {
  install_tools
  mkdir -p "$(dirname -- "$GIF")"
  "$TOOLS_ROOT/bin/agg" \
    --theme github-dark --font-family 'DejaVu Sans Mono' --font-size 16 \
    --line-height 1.25 --fps-cap 12 --idle-time-limit 6 \
    --last-frame-duration 3 "$CAST" "$GIF"
}

case ${1:---play} in
  --play) play_demo ;;
  --record) record_demo ;;
  --render) render_demo ;;
  *) printf 'usage: %s [--play|--record|--render]\n' "$0" >&2; exit 2 ;;
esac
