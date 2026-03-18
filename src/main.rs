use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

#[derive(Parser)]
#[command(
    name = "agent-procs",
    version,
    about = "Concurrent process runner for AI agents",
    long_about = "\
Concurrent process runner for AI agents.

Processes run in a background daemon and persist across CLI invocations.
Use --session to isolate process groups (e.g. per-project).",
    before_long_help = "\
Typical workflow:
  agent-procs run \"npm run dev\" --name server
  agent-procs wait server --until \"Listening on\" --timeout 30
  agent-procs logs server --tail 50
  agent-procs stop server

Config-driven (agent-procs.yaml):
  agent-procs up                              # start all from config
  agent-procs down                            # stop all config processes",
    after_long_help = "\
Exit codes:
  0  Success
  1  Error (timeout, connection failure, unexpected response)
  2  No logs found for target process

Config file format (agent-procs.yaml):
  session: myproject                          # optional, isolates processes
  proxy: true                                 # optional, enables reverse proxy
  proxy_port: 9095                            # optional, pin proxy port
  processes:
    db:
      cmd: docker compose up postgres
      ready: \"ready to accept connections\"
    api:
      cmd: ./start-api-server
      cwd: ./backend                          # relative to config file
      env:
        DATABASE_URL: postgres://...
      ready: \"Listening on :8080\"
      port: 8080                              # injected as PORT env var
      depends_on: [db]                        # db must be ready first

  Top-level fields (all optional):
    session     — session name (overridden by --session flag)
    proxy       — enable reverse proxy (default: false)
    proxy_port  — pin proxy to a port (default: auto 9090-9190)

  Per-process fields:
    cmd         — shell command to execute (required)
    cwd         — working directory, relative to config file
    env         — environment variables (key: value map)
    ready       — stdout pattern that signals readiness
    depends_on  — list of processes that must be ready first
    port        — port number, injected as PORT and HOST env vars

  Processes start in dependency order; independent ones run concurrently.
  With proxy: true, processes get named URLs (e.g. http://api.localhost:9090)."
)]
struct Cli {
    /// Session name for isolating process groups (e.g. per-project)
    #[arg(long, global = true)]
    session: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Spawn a new process
    #[command(display_order = 1)]
    Run {
        /// Command to execute
        command: String,
        /// Process name (auto-generated if omitted)
        #[arg(long)]
        name: Option<String>,
        /// Assign a specific port (injected as PORT env var)
        #[arg(long)]
        port: Option<u16>,
        /// Enable reverse proxy for this session
        #[arg(long)]
        proxy: bool,
        /// Auto-restart policy: always, on-failure, or never
        #[arg(long)]
        autorestart: Option<String>,
        /// Maximum number of restarts (unlimited if omitted)
        #[arg(long)]
        max_restarts: Option<u32>,
        /// Delay between crash and restart in milliseconds
        #[arg(long)]
        restart_delay: Option<u64>,
        /// File glob patterns to watch for changes (repeatable)
        #[arg(long)]
        watch: Vec<String>,
        /// Glob patterns to ignore when watching (repeatable)
        #[arg(long)]
        watch_ignore: Vec<String>,
    },
    /// Stop a process
    #[command(display_order = 2)]
    Stop {
        /// Process name or ID
        target: String,
    },
    /// Stop all processes
    #[command(display_order = 3)]
    StopAll,
    /// Restart a process
    #[command(display_order = 4)]
    Restart {
        /// Process name or ID
        target: String,
    },
    /// Show status of all processes
    #[command(display_order = 5)]
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// View process logs
    #[command(display_order = 6)]
    Logs {
        /// Process name or ID (omit with --all)
        target: Option<String>,
        /// Number of lines from end
        #[arg(long, default_value = "100")]
        tail: usize,
        /// Stream output in real-time
        #[arg(long)]
        follow: bool,
        /// Show stderr instead of stdout
        #[arg(long)]
        stderr: bool,
        /// Show all processes interleaved
        #[arg(long)]
        all: bool,
        /// Timeout in seconds (default: 30 for --follow)
        #[arg(long)]
        timeout: Option<u64>,
        /// Max lines to stream (for --follow)
        #[arg(long)]
        lines: Option<usize>,
    },
    /// Wait for a process condition
    #[command(display_order = 7)]
    Wait {
        /// Process name or ID
        target: String,
        /// Wait until pattern appears in output
        #[arg(long)]
        until: Option<String>,
        /// Interpret --until pattern as regex
        #[arg(long)]
        regex: bool,
        /// Wait until process exits
        #[arg(long)]
        exit: bool,
        /// Timeout in seconds
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Start all processes from config
    #[command(display_order = 8)]
    Up {
        /// Start only these processes (comma-separated)
        #[arg(long)]
        only: Option<String>,
        /// Config file path (default: auto-discover)
        #[arg(long)]
        config: Option<String>,
        /// Enable reverse proxy for this session
        #[arg(long)]
        proxy: bool,
    },
    /// Stop all config-managed processes
    #[command(display_order = 9)]
    Down,
    /// Session management
    #[command(display_order = 10)]
    Session {
        #[command(subcommand)]
        command: SessionCommands,
    },
    /// Open terminal UI for monitoring processes
    #[command(display_order = 11)]
    Ui,
    /// Generate shell completions
    #[command(display_order = 12)]
    Completions {
        /// Shell to generate completions for
        shell: Shell,
    },
    /// Internal: run as daemon (used by `spawn_daemon`)
    #[command(hide = true)]
    RunDaemon { session: String },
}

#[derive(Subcommand)]
enum SessionCommands {
    /// List active sessions
    List,
    /// Remove stale sessions
    Clean,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Handle internal daemon runner before session setup
    if let Commands::RunDaemon { ref session } = cli.command {
        std::process::exit(agent_procs::daemon::spawn::run_daemon(session).await);
    }

    let cli_session = cli.session;
    let cli_session_ref = cli_session.as_deref();
    let session = cli_session_ref.unwrap_or(agent_procs::config::DEFAULT_SESSION);

    let exit_code = match cli.command {
        Commands::Run {
            command,
            name,
            port,
            proxy,
            autorestart,
            max_restarts,
            restart_delay,
            watch,
            watch_ignore,
        } => {
            agent_procs::cli::run::execute(
                session,
                &command,
                name,
                port,
                proxy,
                autorestart,
                max_restarts,
                restart_delay,
                watch,
                watch_ignore,
            )
            .await
        }
        Commands::Stop { target } => agent_procs::cli::stop::execute(session, &target).await,
        Commands::StopAll => agent_procs::cli::stop::execute_all(session).await,
        Commands::Restart { target } => agent_procs::cli::restart::execute(session, &target).await,
        Commands::Status { json } => agent_procs::cli::status::execute(session, json).await,
        Commands::Logs {
            target,
            tail,
            follow,
            stderr,
            all,
            timeout,
            lines,
        } => {
            agent_procs::cli::logs::execute(
                session,
                target.as_deref(),
                tail,
                follow,
                stderr,
                all,
                timeout,
                lines,
            )
            .await
        }
        Commands::Wait {
            target,
            until,
            regex,
            exit,
            timeout,
        } => agent_procs::cli::wait::execute(session, &target, until, regex, exit, timeout).await,
        Commands::Up {
            only,
            config,
            proxy,
        } => {
            agent_procs::cli::up::execute(
                cli_session_ref,
                only.as_deref(),
                config.as_deref(),
                proxy,
            )
            .await
        }
        Commands::Down => agent_procs::cli::down::execute(cli_session_ref).await,
        Commands::Session { command } => match command {
            SessionCommands::List => agent_procs::cli::session_cmd::list(),
            SessionCommands::Clean => agent_procs::cli::session_cmd::clean(),
        },
        Commands::Ui => agent_procs::tui::run(session).await,
        Commands::Completions { shell } => {
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "agent-procs",
                &mut std::io::stdout(),
            );
            0
        }
        // RunDaemon is handled by early return above
        Commands::RunDaemon { .. } => unreachable!(),
    };
    std::process::exit(exit_code);
}
