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
    let cli = Cli::parse();
    // Commands will be wired up in later tasks
    eprintln!("agent-procs: command not yet implemented");
    std::process::exit(1);
}
