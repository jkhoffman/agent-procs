# Security Policy

## Supported versions

| Version | Supported |
|---------|-----------|
| 0.2.x   | Yes       |
| < 0.2   | No        |

## Reporting a vulnerability

Please report security vulnerabilities through
[GitHub Security Advisories](https://github.com/jkhoffman/agent-procs/security/advisories/new)
(private disclosure). Do **not** open a public issue for security reports.

You should receive an acknowledgment within 72 hours. We will work with you to
understand the issue and coordinate a fix before any public disclosure.

## Security model

### Unix domain socket access control

The daemon listens on a Unix domain socket located in
`/tmp/agent-procs-<uid>/`. The socket file and its parent directory are created
with `0700` permissions, restricting access to the owning user. No TCP listener
is opened for daemon communication.

### PID file permissions

PID files are stored in `/tmp/agent-procs-<uid>/` alongside the socket, inside
the same `0700`-permissioned directory. This prevents other users from reading
or tampering with PID files.

### Process group isolation

Each managed child process is spawned in its own process group. When a process
is stopped, `SIGTERM` is sent to the entire process group, ensuring that child
processes of managed processes are also terminated cleanly.

### Reverse proxy

The reverse proxy (`--proxy` flag) binds to **localhost only** (`127.0.0.1`).
It is not exposed to the network by default. The proxy routes HTTP requests to
managed processes based on subdomain (e.g., `http://api.localhost:9090`).

### No network exposure by default

Without the `--proxy` flag, agent-procs has no TCP listeners at all. All
communication between the CLI client and the daemon occurs over the Unix domain
socket.

## Out of scope

agent-procs manages child processes as the **current operating system user**.
It does not sandbox, containerize, or otherwise isolate managed processes from
each other or from the rest of the user's environment. Specifically:

- Managed processes run with the same UID/GID and filesystem access as the
  user who started the daemon.
- Processes can communicate with each other through any mechanism available to
  the user (filesystem, network, shared memory, etc.).
- Environment variables passed to one process are not hidden from other
  processes that can inspect `/proc` (on Linux) or use similar OS facilities.

If you need process-level isolation, consider running agent-procs inside a
container or VM, or using a dedicated sandboxing tool.
