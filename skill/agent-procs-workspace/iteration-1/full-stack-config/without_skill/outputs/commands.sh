#!/usr/bin/env bash
set -euo pipefail

# Start all processes defined in agent-procs.yaml.
# This reads the config file, resolves the dependency graph (db -> api -> web),
# and launches each process in order, waiting for the "ready" pattern before
# starting dependents.
agent-procs up

# Verify all three processes are running.
agent-procs status

# (Optional) Tail logs from each service to confirm healthy startup.
agent-procs logs db --tail 20
agent-procs logs api --tail 20
agent-procs logs web --tail 20
