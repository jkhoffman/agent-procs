#!/usr/bin/env bash
# Microservices local dev startup using agent-procs
#
# Topology:
#   users-service (port 4001)  ─┐
#                                ├──> gateway (port 4000)
#   orders-service (port 4002) ─┘
#
# The gateway depends on both users-service and orders-service.
# agent-procs will start users-service and orders-service concurrently,
# wait for each to print "Server started", then start the gateway.

# Start all services from the config file (respects dependency order)
agent-procs up

# Verify all three services are running
agent-procs status

# --- Useful commands for ongoing development ---

# Check logs for a specific service
# agent-procs logs gateway --tail 50
# agent-procs logs users-service --tail 50
# agent-procs logs orders-service --tail 50

# Check stderr for a specific service (useful for debugging)
# agent-procs logs gateway --stderr

# Stream live logs for 10 seconds
# agent-procs logs gateway --follow --timeout 10

# Open the TUI for interactive monitoring
# agent-procs ui

# Stop everything and shut down the daemon when done
# agent-procs down
