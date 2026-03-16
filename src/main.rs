use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "agent-procs", about = "Concurrent process runner for AI agents")]
struct Cli {
    /// Session name (default: "default")
    #[arg(long, global = true, default_value = "default")]
    session: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Spawn a new process
    Run {
        /// Command to execute
        command: String,
        /// Process name (auto-generated if omitted)
        #[arg(long)]
        name: Option<String>,
    },
    /// Stop a process
    Stop { target: String },
    /// Stop all processes
    StopAll,
    /// Restart a process
    Restart { target: String },
    /// Show status of all processes
    Status {
        #[arg(long)]
        json: bool,
    },
    /// View process logs
    Logs {
        target: Option<String>,
        #[arg(long, default_value = "100")]
        tail: usize,
        #[arg(long)]
        follow: bool,
        #[arg(long)]
        stderr: bool,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Wait for a process condition
    Wait {
        target: String,
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        regex: bool,
        #[arg(long)]
        exit: bool,
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Start all processes from config
    Up {
        #[arg(long)]
        only: Option<String>,
        #[arg(long)]
        config: Option<String>,
    },
    /// Stop all config-managed processes
    Down,
    /// Session management
    Session {
        #[command(subcommand)]
        command: SessionCommands,
    },
}

#[derive(Subcommand)]
enum SessionCommands {
    List,
    Clean,
}

#[tokio::main]
async fn main() {
    // Check for hidden internal daemon-runner flag before clap parsing.
    // This is invoked by spawn_daemon() to start the background daemon process.
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 3 && args[1] == "--run-daemon" {
        let session = &args[2];
        agent_procs::daemon::spawn::run_daemon(session).await;
        return;
    }

    let cli = Cli::parse();
    let session = &cli.session;

    let exit_code = match cli.command {
        Commands::Run { command, name } => agent_procs::cli::run::execute(session, &command, name).await,
        Commands::Stop { target } => agent_procs::cli::stop::execute(session, &target).await,
        Commands::StopAll => agent_procs::cli::stop::execute_all(session).await,
        Commands::Restart { target } => agent_procs::cli::restart::execute(session, &target).await,
        Commands::Status { json } => agent_procs::cli::status::execute(session, json).await,
        Commands::Logs { target, tail, follow, stderr, all, timeout } => {
            agent_procs::cli::logs::execute(session, target.as_deref(), tail, follow, stderr, all, timeout).await
        }
        Commands::Wait { target, until, regex, exit, timeout } => {
            agent_procs::cli::wait::execute(session, &target, until, regex, exit, timeout).await
        }
        Commands::Up { only, config } => {
            agent_procs::cli::up::execute(session, only.as_deref(), config.as_deref()).await
        }
        Commands::Down => agent_procs::cli::down::execute(session).await,
        Commands::Session { command } => match command {
            SessionCommands::List => agent_procs::cli::session_cmd::list().await,
            SessionCommands::Clean => agent_procs::cli::session_cmd::clean().await,
        },
    };
    std::process::exit(exit_code);
}
