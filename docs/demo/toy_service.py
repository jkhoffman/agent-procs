#!/usr/bin/env python3
"""Small deterministic HTTP services and demo controls for AgentProcs."""

from __future__ import annotations

import argparse
import json
import os
import signal
import sys
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

PROXY_PORT = 49095
try:
    runtime_value = os.environ["AGENT_PROCS_DEMO_RUNTIME"]
except KeyError as error:
    raise SystemExit("AGENT_PROCS_DEMO_RUNTIME is required") from error
if not runtime_value:
    raise SystemExit("AGENT_PROCS_DEMO_RUNTIME must not be empty")
RUNTIME_DIR = Path(runtime_value).resolve(strict=True)
if not RUNTIME_DIR.is_dir():
    raise SystemExit("AGENT_PROCS_DEMO_RUNTIME must name an existing directory")
CRASH_FILE = RUNTIME_DIR / "crash-api"
GENERATION_FILE = RUNTIME_DIR / "api-generation"


def api_url(path: str = "/health") -> str:
    return f"http://api.localhost:{PROXY_PORT}{path}"


def web_url(path: str = "/") -> str:
    return f"http://web.localhost:{PROXY_PORT}{path}"


def next_generation() -> int:
    RUNTIME_DIR.mkdir(exist_ok=True)
    try:
        generation = int(GENERATION_FILE.read_text(encoding="utf-8")) + 1
    except (FileNotFoundError, ValueError):
        generation = 1
    GENERATION_FILE.write_text(f"{generation}\n", encoding="utf-8")
    return generation


def fetch_json(url: str) -> dict[str, object]:
    request = urllib.request.Request(url, headers={"User-Agent": "agent-procs-demo"})
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(request, timeout=3) as response:
        return json.load(response)


class Handler(BaseHTTPRequestHandler):
    server_version = "AgentProcsDemo/1"

    def log_message(self, format: str, *args: object) -> None:
        del format, args
        return

    def do_GET(self) -> None:  # noqa: N802 - required by BaseHTTPRequestHandler
        role = self.server.role  # type: ignore[attr-defined]
        generation = self.server.generation  # type: ignore[attr-defined]
        if self.path not in ("/", "/health"):
            self.send_error(404)
            return

        payload: dict[str, object] = {
            "service": role,
            "status": "ready",
            "generation": generation,
        }
        if role == "web":
            payload["upstream"] = fetch_json(api_url())

        body = (json.dumps(payload, sort_keys=True) + "\n").encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def watch_for_crash() -> None:
    while True:
        if CRASH_FILE.exists():
            CRASH_FILE.unlink(missing_ok=True)
            print("api: controlled crash requested (exit 42)", flush=True)
            os._exit(42)
        time.sleep(0.05)


def serve(role: str) -> None:
    # This demo is intentionally local-only. Do not let an ambient HOST value
    # widen the listening interface.
    host = "127.0.0.1"
    port = int(os.environ["PORT"])
    generation = next_generation() if role == "api" else 1
    server = ThreadingHTTPServer((host, port), Handler)
    server.role = role  # type: ignore[attr-defined]
    server.generation = generation  # type: ignore[attr-defined]

    if role == "api":
        threading.Thread(target=watch_for_crash, daemon=True).start()

    def stop(_signum: int, _frame: object) -> None:
        threading.Thread(target=server.shutdown, daemon=True).start()

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    print(f"{role}: READY on {host}:{port} (generation {generation})", flush=True)
    server.serve_forever(poll_interval=0.1)
    server.server_close()
    print(f"{role}: stopped", flush=True)


def request(service: str) -> None:
    payload = fetch_json(api_url() if service == "api" else web_url())
    print(json.dumps(payload, indent=2, sort_keys=True))


def crash() -> None:
    RUNTIME_DIR.mkdir(exist_ok=True)
    CRASH_FILE.write_text("exit 42\n", encoding="utf-8")
    print("crash trigger written for api")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    serve_parser = subparsers.add_parser("serve")
    serve_parser.add_argument("role", choices=("api", "web"))
    request_parser = subparsers.add_parser("request")
    request_parser.add_argument("service", choices=("api", "web"))
    subparsers.add_parser("crash")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    if args.command == "serve":
        serve(args.role)
    elif args.command == "request":
        request(args.service)
    else:
        crash()


if __name__ == "__main__":
    try:
        main()
    except (OSError, urllib.error.URLError) as error:
        print(f"demo error: {error}", file=sys.stderr)
        raise SystemExit(1) from error
