#!/usr/bin/env bash
set -Eeuo pipefail

ROOT=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
BINARY="$ROOT/target/release/agent-procs"
CONFIG="docs/demo/agent-procs.yaml"
DEMO_DIR="$ROOT/docs/demo"
RUNTIME="$DEMO_DIR/.demo-runtime"
STATE_HOME="$DEMO_DIR/.agent-procs-state"
TOOLS_ROOT=${AGENT_PROCS_DEMO_TOOLS:-${TMPDIR:-/tmp}/agent-procs-demo-tools}
ASCIINEMA_VERSION=3.0.0
AGG_VERSION=1.5.0
AGG_REVISION=5592b9790ba7c6d5ffa232176e29a1d3cadf8fe2
CAST="$DEMO_DIR/agent-procs-demo.cast"
GIF="$ROOT/docs/assets/agent-procs-demo.gif"
SESSION=portfolio-demo
PORTS=(43111 43112 49095)
export XDG_STATE_HOME="$STATE_HOME"
export TERM=xterm-256color
export NO_COLOR=1

CLEANED=0

pause() {
  local seconds=$1
  local delay=${DEMO_DELAY:-1}
  sleep "$(python3 -c 'import sys; print(float(sys.argv[1]) * float(sys.argv[2]))' "$seconds" "$delay")"
}

prompt() {
  printf '\n\033[1;36m❯ %s\033[0m\n' "$1"
  pause 0.7
}

ap() {
  "$BINARY" "$@"
}

cleanup() {
  if [[ $CLEANED -eq 1 ]]; then
    return
  fi
  "$BINARY" --session "$SESSION" down >/dev/null 2>&1 || true
  for _ in {1..30}; do
    [[ ! -e "/tmp/agent-procs-$(id -u)/$SESSION.sock" ]] && break
    sleep 0.1
  done
  rm -rf "$RUNTIME" "$STATE_HOME"
  CLEANED=1
}
trap cleanup EXIT INT TERM

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

verify_cleanup() {
  local socket_base
  socket_base="/tmp/agent-procs-$(id -u)"
  for attempt in {1..50}; do
    if [[ ! -e "$socket_base/$SESSION.sock" && ! -e "$socket_base/$SESSION.pid" ]] && assert_ports_free 2>/dev/null; then
      break
    fi
    if (( attempt % 10 == 0 )); then
      "$BINARY" --session "$SESSION" down >/dev/null 2>&1 || true
    fi
    sleep 0.1
  done

  [[ ! -e "$socket_base/$SESSION.sock" ]]
  [[ ! -e "$socket_base/$SESSION.pid" ]]
  assert_ports_free

  python3 - "$RUNTIME/status-before-down.json" <<'PY'
import json
import os
import sys

status_path = sys.argv[1]
for process in json.load(open(status_path, encoding="utf-8")):
    pid = int(process["pid"])
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        continue
    raise SystemExit(f"process still alive: {pid}")
PY

  rm -rf "$RUNTIME" "$STATE_HOME"
  [[ ! -e "$RUNTIME" && ! -e "$STATE_HOME" ]]
}

play_demo() {
  cd "$ROOT"
  cleanup
  CLEANED=0
  assert_ports_free
  mkdir -p "$RUNTIME"

  printf '\033[2J\033[H'
  printf '\033[1;37mAgentProcs: resilient local agent workflow\033[0m\n'
  printf '\033[0;37mTwo dependent services · stable URLs · automatic crash recovery\033[0m\n'
  pause 3

  prompt 'agent-procs up --config docs/demo/agent-procs.yaml'
  ap up --config "$CONFIG"
  pause 4

  prompt 'agent-procs --session portfolio-demo status'
  ap --session "$SESSION" status
  pause 4

  prompt 'python3 docs/demo/toy_service.py request web'
  python3 docs/demo/toy_service.py request web
  pause 4

  prompt 'agent-procs --session portfolio-demo logs --all --tail 8'
  ap --session "$SESSION" logs --all --tail 8
  pause 3

  printf '\n\033[1;37mNow trigger a controlled API failure...\033[0m\n'
  prompt 'python3 docs/demo/toy_service.py crash'
  python3 docs/demo/toy_service.py crash
  pause 2

  prompt 'agent-procs --session portfolio-demo wait api --until "[agent-procs] Restarted" --timeout 10'
  ap --session "$SESSION" wait api --until '[agent-procs] Restarted' --timeout 10
  pause 3

  prompt 'agent-procs --session portfolio-demo logs api --tail 8'
  ap --session "$SESSION" logs api --tail 8
  pause 5

  prompt 'agent-procs --session portfolio-demo status'
  ap --session "$SESSION" status
  pause 4

  prompt 'python3 docs/demo/toy_service.py request api'
  python3 docs/demo/toy_service.py request api
  pause 4

  ap --session "$SESSION" status --json >"$RUNTIME/status-before-down.json"
  prompt 'agent-procs --session portfolio-demo down'
  ap --session "$SESSION" down
  pause 3

  verify_cleanup
  CLEANED=1
  printf '\n\033[1;32m✓ clean shutdown verified\033[0m\n'
  printf '  no demo processes · no listeners · no socket, state, or runtime files\n'
  pause 6
}

install_tools() {
  mkdir -p "$TOOLS_ROOT"
  if [[ ! -x "$TOOLS_ROOT/bin/asciinema" ]] || [[ $("$TOOLS_ROOT/bin/asciinema" --version) != "asciinema $ASCIINEMA_VERSION" ]]; then
    cargo install --locked --root "$TOOLS_ROOT" --version "$ASCIINEMA_VERSION" asciinema
  fi
  if [[ ! -x "$TOOLS_ROOT/bin/agg" ]] || [[ $("$TOOLS_ROOT/bin/agg" --version) != "agg $AGG_VERSION" ]]; then
    cargo install --locked --root "$TOOLS_ROOT" --git https://github.com/asciinema/agg --rev "$AGG_REVISION" agg
  fi
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
