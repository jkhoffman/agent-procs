#!/usr/bin/env bash
# =============================================================================
# Start a Rails server in the background, wait for it to be ready, run
# integration tests against it, then shut everything down.
#
# Uses agent-procs for background process management instead of manual
# nohup/& so we get readiness detection, log capture, and clean shutdown.
# =============================================================================

# Step 1: Start the Rails server as a background process named "rails".
# agent-procs hands it off to its daemon so it persists while we work.
agent-procs run "bin/rails server -b 0.0.0.0 -p 3000" --name rails

# Step 2: Wait for the server to finish booting. The server prints
# "Listening on http://0.0.0.0:3000" when it is ready to accept requests.
# We set a 30-second timeout (generous, since boot takes ~10s) so we don't
# hang forever if something goes wrong.
agent-procs wait rails --until "Listening on http://0.0.0.0:3000" --timeout 30

# Step 3: Verify the server process is actually running and healthy before
# we kick off tests. If it exited early we want to know now.
agent-procs status

# Step 4: Run the integration test suite. This is a normal short-lived
# command, so we run it directly in the foreground and capture its exit code.
bundle exec rails test:integration
TEST_EXIT_CODE=$?

# Step 5: Regardless of whether tests passed or failed, grab the last chunk
# of server logs. Useful for debugging failures or confirming clean behavior.
agent-procs logs rails --tail 50

# Step 6: Stop the Rails server and shut down the agent-procs daemon.
# "down" stops all managed processes and tears down the daemon cleanly,
# preventing orphaned server processes.
agent-procs down

# Step 7: Exit with the test suite's exit code so CI or the caller can
# detect pass/fail.
exit $TEST_EXIT_CODE
