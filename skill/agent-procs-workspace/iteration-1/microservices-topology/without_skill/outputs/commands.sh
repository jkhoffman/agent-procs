#!/usr/bin/env bash
# Microservices local dev startup using agent-procs
# Manages: gateway (4000), users-service (4001), orders-service (4002)

# Start all three services in dependency order from agent-procs.yaml.
# users-service and orders-service start concurrently (no dependencies).
# gateway starts only after both are ready (depends_on).
# Each service is considered ready when it prints "Server started".
agent-procs up --config agent-procs.yaml

# Verify all services are running
agent-procs status --session microservices

# --- Optional: view logs for individual services ---
# agent-procs logs users-service --session microservices --tail 50
# agent-procs logs orders-service --session microservices --tail 50
# agent-procs logs gateway --session microservices --tail 50

# --- Optional: open the TUI to monitor all processes ---
# agent-procs ui --session microservices

# --- To shut everything down ---
# agent-procs down --config agent-procs.yaml
