#!/usr/bin/env bash
set -euo pipefail

# Start all processes defined in agent-procs.yaml.
# This launches db, api, and web in dependency order:
#   1. db starts first and waits for "ready to accept connections"
#   2. api starts after db is ready and waits for "Uvicorn running on"
#   3. web starts after api is ready and waits for "Local:"
agent-procs up

# Verify all three processes are running
agent-procs status
