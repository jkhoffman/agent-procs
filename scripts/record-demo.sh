#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

ROOT=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
BINARY="$ROOT/target/release/agent-procs"
CONFIG="docs/demo/agent-procs.yaml"
DEMO_DIR="$ROOT/docs/demo"
TOOLS_BASE=${AGENT_PROCS_DEMO_TOOLS:-${TMPDIR:-/tmp}/agent-procs-demo-tools}
ASCIINEMA_VERSION=3.0.0
AGG_VERSION=1.5.0
AGG_REVISION=5592b9790ba7c6d5ffa232176e29a1d3cadf8fe2
TOOLS_ROOT="$TOOLS_BASE/asciinema-$ASCIINEMA_VERSION-agg-$AGG_REVISION"
CAST="$DEMO_DIR/agent-procs-demo.cast"
GIF="$ROOT/docs/assets/agent-procs-demo.gif"
SESSION="agent-procs-demo-$(python3 -c 'import secrets; print(secrets.token_hex(16))')"
SOCKET_BASE="/tmp/agent-procs-$(id -u)"
SOCKET="$SOCKET_BASE/$SESSION.sock"
PID_FILE="$SOCKET_BASE/$SESSION.pid"
PORTS=(43111 43112 49095)
RUN_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/agent-procs-demo.XXXXXX")
chmod 700 "$RUN_ROOT"
RUNTIME="$RUN_ROOT/runtime"
STATE_HOME="$RUN_ROOT/state"
STATUS_BEFORE_DOWN="$RUN_ROOT/status-before-down.json"
export AGENT_PROCS_DEMO_RUNTIME="$RUNTIME"
export XDG_STATE_HOME="$STATE_HOME"
export TERM=xterm-256color
export NO_COLOR=1
mkdir -m 700 "$RUNTIME" "$STATE_HOME"

CLEANED=0
OWN_SESSION=0
DOWN_DONE=0

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
  "$BINARY" --session "$SESSION" "$@"
}

assert_ports_free() {
  python3 - "${PORTS[@]}" <<'PY'
import socket, sys
for value in sys.argv[1:]:
    with socket.socket() as sock:
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        sock.bind(("127.0.0.1", int(value)))
PY
}

preflight() {
  mkdir -p -- "$SOCKET_BASE"
  [[ -d "$SOCKET_BASE" && ! -L "$SOCKET_BASE" && -O "$SOCKET_BASE" ]] || {
    printf 'error: unsafe socket directory: %s\n' "$SOCKET_BASE" >&2
    return 1
  }
  chmod 700 "$SOCKET_BASE"
  [[ ! -e "$SOCKET" && ! -L "$SOCKET" && ! -e "$PID_FILE" && ! -L "$PID_FILE" ]] || {
    printf 'error: random session unexpectedly already exists: %s\n' "$SESSION" >&2
    return 1
  }
  assert_ports_free
}

verify_expected_status() {
  python3 - "$1" <<'PY'
import json, os, sys
with open(sys.argv[1], encoding="utf-8") as stream:
    processes = json.load(stream)
expected = {"api": "python3 toy_service.py serve api", "web": "python3 toy_service.py serve web"}
if len(processes) != 2 or {item.get("name") for item in processes} != set(expected):
    raise SystemExit("session must contain exactly api and web")
for item in processes:
    name = item["name"]
    if item.get("command") != expected[name] or item.get("state") != "running":
        raise SystemExit(f"unexpected command or state for {name}")
    os.kill(int(item["pid"]), 0)
PY
}

stop_without_ownership() {
  local probe="$RUN_ROOT/status-unowned.json"
  if [[ -S "$SOCKET" ]] && ap status --json >"$probe" 2>/dev/null; then
    ap stop api >/dev/null 2>&1 || true
    ap stop web >/dev/null 2>&1 || true
    local attempt state
    for attempt in {1..200}; do
      if ! ap status --json >"$probe" 2>/dev/null; then break; fi
      state=$(python3 - "$probe" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as stream:
    processes = json.load(stream)
expected = {"api": "python3 toy_service.py serve api", "web": "python3 toy_service.py serve web"}
names = {item.get("name") for item in processes}
if not processes:
    print("retirable")
elif names != set(expected) or any(item.get("command") != expected.get(item.get("name")) for item in processes):
    print("foreign")
elif all(item.get("state") in {"exited", "failed"} for item in processes):
    print("retirable")
else:
    print("stopping")
PY
      )
      [[ $state != foreign ]] || break
      if [[ $state == retirable ]]; then
        ap down >/dev/null 2>&1 || true
        local retire_attempt
        for retire_attempt in {1..50}; do
          if [[ ! -e "$SOCKET" && ! -L "$SOCKET" && ! -e "$PID_FILE" && ! -L "$PID_FILE" ]]; then
            break
          fi
          if [[ $retire_attempt -eq 10 || $retire_attempt -eq 20 ]]; then
            ap down >/dev/null 2>&1 || true
          fi
          sleep 0.1
        done
        break
      fi
      sleep 0.1
    done
  fi
  if [[ -e "$SOCKET" || -L "$SOCKET" || -e "$PID_FILE" || -L "$PID_FILE" ]]; then
    printf 'warning: session was not validated; preserved artifacts for %s in %s\n' "$SESSION" "$SOCKET_BASE" >&2
  fi
}

safe_down() {
  [[ $OWN_SESSION -eq 1 ]] || return 1
  ap down
  local attempt
  for attempt in {1..50}; do
    if [[ ! -e "$SOCKET" && ! -L "$SOCKET" && ! -e "$PID_FILE" && ! -L "$PID_FILE" ]]; then
      DOWN_DONE=1
      return 0
    fi
    if [[ $attempt -eq 10 || $attempt -eq 20 ]]; then
      ap down >/dev/null 2>&1 || true
    fi
    sleep 0.1
  done
  printf 'error: session did not retire its socket and PID file\n' >&2
  return 1
}

remove_run_root() {
  # This private high-entropy mktemp boundary prevents accidental collisions.
  # Malicious same-user replacement races and root are explicitly out of scope.
  python3 - "$RUN_ROOT" "${TMPDIR:-/tmp}" <<'PY'
import os, shutil, stat, sys
path, temp = sys.argv[1:]
entry = os.lstat(path)
resolved = os.path.realpath(path)
parent = os.path.realpath(temp)
if not stat.S_ISDIR(entry.st_mode) or stat.S_ISLNK(entry.st_mode):
    raise SystemExit("refusing to remove non-directory run root")
if entry.st_uid != os.getuid() or stat.S_IMODE(entry.st_mode) != 0o700:
    raise SystemExit("refusing run root with unexpected owner or mode")
if os.path.dirname(resolved) != parent or not os.path.basename(resolved).startswith("agent-procs-demo."):
    raise SystemExit("refusing run root outside expected temporary namespace")
shutil.rmtree(resolved)
PY
}

cleanup() {
  local status=$?
  [[ $CLEANED -eq 0 ]] || return "$status"
  CLEANED=1
  local cleanup_failed=0
  if [[ $DOWN_DONE -eq 0 && $OWN_SESSION -eq 1 ]]; then
    safe_down >/dev/null 2>&1 || cleanup_failed=1
  elif [[ $DOWN_DONE -eq 0 ]]; then
    stop_without_ownership || cleanup_failed=1
  fi
  remove_run_root || cleanup_failed=1
  if [[ $status -eq 0 && $cleanup_failed -ne 0 ]]; then status=1; fi
  return "$status"
}

on_int() { exit 130; }
on_term() { exit 143; }
trap cleanup EXIT
trap on_int INT
trap on_term TERM

verify_cleanup() {
  local attempt
  for attempt in {1..50}; do
    if [[ ! -e "$SOCKET" && ! -e "$PID_FILE" ]] && assert_ports_free 2>/dev/null; then break; fi
    sleep 0.1
  done
  [[ ! -e "$SOCKET" && ! -L "$SOCKET" && ! -e "$PID_FILE" && ! -L "$PID_FILE" ]]
  assert_ports_free
  python3 - "$STATUS_BEFORE_DOWN" <<'PY'
import json, os, sys
with open(sys.argv[1], encoding="utf-8") as stream:
    processes = json.load(stream)
for process in processes:
    try: os.kill(int(process["pid"]), 0)
    except ProcessLookupError: continue
    raise SystemExit(f"process still alive: {process['pid']}")
PY
}

play_demo() {
  cd "$ROOT"
  preflight
  printf '\033[2J\033[H'
  printf '\033[1;37mAgentProcs: resilient local agent workflow\033[0m\n'
  printf '\033[0;37mTwo dependent services · stable URLs · automatic crash recovery\033[0m\n'
  printf '\033[0;37mIsolated session: %s\033[0m\n' "$SESSION"
  printf '\033[0;37m$ DEMO_SESSION=%s\033[0m\n' "$SESSION"
  pause 3

  prompt "agent-procs --session \"\$DEMO_SESSION\" up --config docs/demo/agent-procs.yaml"
  ap up --config "$CONFIG"
  if ! ap status --json >"$RUN_ROOT/status-after-up.json" || \
      ! verify_expected_status "$RUN_ROOT/status-after-up.json"; then
    printf 'error: startup did not produce exactly the expected running api/web processes\n' >&2
    return 1
  fi
  OWN_SESSION=1
  pause 4

  prompt "agent-procs --session \"\$DEMO_SESSION\" status"
  ap status
  pause 4
  prompt 'python3 docs/demo/toy_service.py request web'
  python3 docs/demo/toy_service.py request web
  pause 4
  prompt "agent-procs --session \"\$DEMO_SESSION\" logs --all --tail 8"
  ap logs --all --tail 8
  pause 3

  printf '\n\033[1;37mNow trigger a controlled API failure...\033[0m\n'
  prompt 'python3 docs/demo/toy_service.py crash'
  python3 docs/demo/toy_service.py crash
  pause 2
  prompt "agent-procs --session \"\$DEMO_SESSION\" wait api --until \"[agent-procs] Restarted\" --timeout 10"
  ap wait api --until '[agent-procs] Restarted' --timeout 10
  pause 3
  prompt "agent-procs --session \"\$DEMO_SESSION\" logs api --tail 8"
  ap logs api --tail 8
  pause 5
  prompt "agent-procs --session \"\$DEMO_SESSION\" status"
  ap status
  pause 4
  prompt 'python3 docs/demo/toy_service.py request api'
  python3 docs/demo/toy_service.py request api
  pause 4

  ap status --json >"$STATUS_BEFORE_DOWN"
  verify_expected_status "$STATUS_BEFORE_DOWN"
  prompt "agent-procs --session \"\$DEMO_SESSION\" down"
  safe_down
  pause 3
  verify_cleanup
  printf '\n\033[1;32m✓ clean shutdown verified\033[0m\n'
  printf '  no demo processes · no listeners · no socket or PID files\n'
  printf '  private runtime/state namespace removed on exit\n'
  pause 6
}

agg_provenance_ok() {
  [[ -x "$TOOLS_ROOT/bin/agg" && -f "$TOOLS_ROOT/.crates2.json" ]] || return 1
  [[ $("$TOOLS_ROOT/bin/agg" --version) == "agg $AGG_VERSION" ]] || return 1
  python3 - "$TOOLS_ROOT/.crates2.json" "$AGG_VERSION" "$AGG_REVISION" <<'PY'
import json, sys
metadata_path, version, revision = sys.argv[1:]
with open(metadata_path, encoding="utf-8") as stream:
    installs = json.load(stream).get("installs", {})
expected = f"agg {version} (git+https://github.com/asciinema/agg?rev={revision}#{revision})"
raise SystemExit(0 if "agg" in installs.get(expected, {}).get("bins", []) else 1)
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
  agg_provenance_ok || { printf 'error: cached agg does not prove Git revision %s\n' "$AGG_REVISION" >&2; return 1; }
}

render_demo() {
  install_tools
  mkdir -p "$(dirname -- "$GIF")"
  "$TOOLS_ROOT/bin/agg" --theme github-dark --font-family 'DejaVu Sans Mono' --font-size 16 \
    --line-height 1.25 --fps-cap 12 --idle-time-limit 6 --last-frame-duration 3 "$CAST" "$GIF"
}

record_demo() {
  cd "$ROOT"
  cargo build --locked --release >/dev/null
  install_tools
  mkdir -p "$(dirname -- "$GIF")"
  DEMO_DELAY=1 "$TOOLS_ROOT/bin/asciinema" record \
    --quiet --headless --return --overwrite --output-format asciicast-v2 \
    --window-size 100x30 --idle-time-limit 6 --title 'AgentProcs resilient agent workflow' \
    --command "bash scripts/record-demo.sh --play" "$CAST"
  render_demo
}

case ${1:---play} in
  --play) play_demo ;;
  --record) record_demo ;;
  --render) render_demo ;;
  *) printf 'usage: %s [--play|--record|--render]\n' "$0" >&2; exit 2 ;;
esac
