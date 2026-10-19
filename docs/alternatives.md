# Alternatives

AgentProcs is intentionally narrow for host-process control in agent-driven development; other tools fit different operating models. Linked tool names and notes point to official documentation.

| Tool | Persistent model / isolation | Readiness, status, and logs | Best fit |
|------|------------------------------|-----------------------------|----------|
| AgentProcs | A per-session host daemon persists across CLI calls; sessions separate process groups but do not sandbox them. | Dependency-ordered startup with bounded stdout string or regex readiness waits; after a timeout, startup warns and continues. `status --json`; durable indexed logs; crash policies and native file watching. | Agent-driven Unix development workflows. |
| [`nohup`](https://www.gnu.org/software/coreutils/manual/html_node/nohup-invocation.html) | Runs one command with hangup signals ignored; control remains with normal shell and PID tools. | Writes to `nohup.out` or redirected output. Readiness, named sessions, reconnectable control, and machine-readable status are not documented as built-in workflows in the reviewed manual. | One command must survive logout, and PID/file control is enough. |
| [`tmux`](https://github.com/tmux/tmux/wiki/Getting-Started) | Detached, reconnectable terminal sessions with window and pane separation; terminal persistence is not process supervision. | Formatted listings, pane history, and `pipe-pane`; readiness and dependency workflows or JSON status are not documented as built-in workflows in the reviewed docs. | Interactive terminals that must be reattached later. |
| [`PM2`](https://pm2.keymetrics.io/docs/usage/quick-start/) | A host daemon manages named apps and ecosystem files. | PM2 normally detects Node's listen event; [`--wait-ready`](https://pm2.keymetrics.io/docs/usage/signals-clean-restart/) instead waits for app IPC (`process.send('ready')`). Neither matches stdout. `jlist` JSON; [stored and streamed logs](https://pm2.keymetrics.io/docs/usage/log-management/); restart, watch, boot, and cluster support. | A mature host application manager, especially for long-running or Node.js apps. |
| [`Docker Compose`](https://docs.docker.com/compose/) | Project-scoped containers, optionally detached, with images, networks, and volumes. | `depends_on` sets order; readiness requires a [`service_healthy` condition and healthcheck](https://docs.docker.com/compose/how-tos/startup-order/). [`ps --format json`](https://docs.docker.com/reference/cli/docker/compose/ps/), aggregated logs, [restart policies](https://docs.docker.com/reference/compose-file/services/#restart), and [Compose Watch](https://docs.docker.com/compose/how-tos/file-watch/). | Container-based stacks where those boundaries and healthchecks are the desired model. |
| [`Overmind`](https://github.com/DarthSim/overmind) | A tmux-backed Procfile runner with working-directory socket and daemon controls. | Text status plus multiplexed or `echo` output; readiness/dependency gating and durable indexed logs are not documented as built-in workflows in the reviewed README; selected processes can auto-restart. | An existing Procfile with direct tmux attachment. |
| [`Foreman`](https://ddollar.github.io/foreman/) | Runs a Procfile directly or exports it; the reviewed manual does not document a reconnectable Foreman daemon/status workflow. | Interleaved stdout; readiness and dependencies are not documented as built-ins. Export targets include systemd, runit, supervisord, and launchd; their runtime behavior belongs to the target supervisor. | A lightweight Procfile runner or native-supervisor configuration generator. |

## Use another tool when

- `nohup` fits when one command only needs to survive logout and PID/files are enough.
- Choose `tmux` when a reconnectable interactive terminal is the main need.
- PM2 suits apps that need an established manager with Node support, restarts, boot integration, metrics, or clustering.
- Docker Compose is the better model when the stack should use containers, images, networks, volumes, and healthchecks.
- Overmind fits a project that already has a Procfile and needs direct tmux attachment.
- Foreman works for a lightweight Procfile workflow or export to a native supervisor.
