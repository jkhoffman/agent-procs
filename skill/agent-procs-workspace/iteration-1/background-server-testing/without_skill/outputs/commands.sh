#!/usr/bin/env bash
set -euo pipefail

# Step 1: Start the Rails server as a background process managed by agent-procs.
# The --name flag gives it a human-readable handle ("rails") for later commands.
agent-procs run "bin/rails server -b 0.0.0.0 -p 3000" --name rails

# Step 2: Wait for the server to finish booting.
# --until watches stdout for the readiness message.
# --timeout 30 gives it up to 30 seconds (generous margin over the ~10s boot).
# This command blocks until the pattern appears or the timeout is reached.
# If the timeout is reached, agent-procs exits with code 1 and the script
# stops (due to set -e).
agent-procs wait rails --until "Listening on http://0.0.0.0:3000" --timeout 30

# Step 3: Run the integration test suite against the now-running server.
# Replace the command below with whatever your project uses (e.g., rspec,
# bin/rails test, pytest, etc.).
bin/rails test test/integration

# Step 4: Stop the Rails server. This sends SIGTERM to the process and waits
# for it to exit gracefully.
agent-procs stop rails
