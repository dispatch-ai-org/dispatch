use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use dispatch::{orchestrator, state::State};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "dispatch",
    version,
    about = "Compare coding-agent harnesses on local software tasks"
)]
struct Cli {
    /// Override ~/.dispatch (also available as DISPATCH_HOME).
    #[arg(long, global = true, env = "DISPATCH_HOME")]
    state_dir: Option<PathBuf>,

    /// Show internal diagnostics. Repeat for more detail.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a small project-local Dispatch configuration.
    Init {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Replace an existing dispatch.yml.
        #[arg(long)]
        force: bool,
    },
    /// Check local dependencies and harness availability.
    Doctor {
        #[arg(default_value = ".")]
        source: PathBuf,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Freeze a source tree and execute independent candidates.
    Run(RunArgs),
    /// Show the state of one run (or the latest run).
    Status { run_id: Option<String> },
    /// List recent runs.
    History {
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    /// Show a run and its persisted signals.
    Show { run_id: String },
    /// Print a candidate's patch.
    Diff {
        run_id: String,
        candidate: String,
        #[arg(long, conflicts_with = "name_only")]
        stat: bool,
        #[arg(long, conflicts_with = "stat")]
        name_only: bool,
    },
    /// Print the path to a candidate's complete workspace.
    Inspect {
        run_id: String,
        candidate: String,
        /// Start the user's shell in the candidate workspace.
        #[arg(long)]
        shell: bool,
    },
    /// Compare blind candidates and optionally persist an evaluation.
    Compare(CompareArgs),
    /// Safely apply one candidate to the original source.
    Apply { run_id: String, candidate: String },
    /// Manage explicit opt-in evaluation sync.
    Sync(SyncArgs),
    /// Print the Dispatch version.
    Version,
}

#[derive(Debug, Args)]
struct SyncArgs {
    #[command(subcommand)]
    command: Option<SyncCommand>,
}

#[derive(Debug, Subcommand)]
enum SyncCommand {
    /// Enable evaluation sharing and queue completed evaluations.
    Enable,
    /// Disable all evaluation uploads.
    Disable,
    /// Show consent and outbox state without contacting the cloud.
    Status,
    /// Print the exact JSON payload for one evaluated run.
    Preview { run_id: String },
    /// Manage the developer-preview Dispatch Cloud ingestion token.
    Token(SyncTokenArgs),
}

#[derive(Debug, Args)]
struct SyncTokenArgs {
    #[command(subcommand)]
    command: SyncTokenCommand,
}

#[derive(Debug, Subcommand)]
enum SyncTokenCommand {
    /// Store a server-issued ingestion token locally.
    Set { token: String },
    /// Report only whether a token is configured.
    Status,
    /// Remove the locally stored ingestion token.
    Clear,
}

#[derive(Debug, Args)]
struct RunArgs {
    #[arg(default_value = ".")]
    source: PathBuf,

    #[arg(
        long,
        required_unless_present = "task_file",
        conflicts_with = "task_file"
    )]
    task: Option<String>,

    #[arg(long, required_unless_present = "task", conflicts_with = "task")]
    task_file: Option<PathBuf>,

    /// Comma-separated adapter IDs. Fakes make the complete workflow testable offline.
    #[arg(long, value_delimiter = ',', default_value = "fake-good,fake-bad")]
    harnesses: Vec<String>,

    #[arg(long)]
    config: Option<PathBuf>,

    #[arg(long, value_parser = ["local", "docker"])]
    backend: Option<String>,

    #[arg(long)]
    timeout: Option<u64>,

    #[arg(long)]
    max_parallel: Option<usize>,

    /// Explicitly allow real agents or project checks to execute on the host.
    #[arg(long)]
    allow_unsafe_local: bool,

    /// Forward only the environment variable names allowlisted in dispatch.yml.
    #[arg(long)]
    allow_forwarded_env: bool,
}

#[derive(Debug, Args)]
struct CompareArgs {
    run_id: String,

    /// Submit A/B/.../tie/neither without the interactive prompt.
    #[arg(long)]
    winner: Option<String>,

    /// Optional structured reason; repeat the flag for multiple labels.
    #[arg(long = "reason")]
    reasons: Vec<String>,

    /// Optional unrestricted explanation, stored verbatim.
    #[arg(long, conflicts_with = "explanation_file")]
    explanation: Option<String>,

    /// Read unrestricted explanation verbatim from this path, or '-' for stdin.
    #[arg(long, conflicts_with = "explanation")]
    explanation_file: Option<PathBuf>,

    /// Prompt for outcome, labels, and multiline freeform text.
    #[arg(long)]
    evaluate: bool,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let filter = match cli.verbose {
        0 => EnvFilter::new("warn"),
        1 => EnvFilter::new("dispatch=info"),
        _ => EnvFilter::new("dispatch=debug"),
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    let state = State::discover(cli.state_dir)?;
    match cli.command {
        Command::Init { path, force } => orchestrator::init(&state, &path, force),
        Command::Doctor { source, config } => {
            orchestrator::doctor(&state, &source, config.as_deref()).await
        }
        Command::Run(args) => {
            let task = match (args.task, args.task_file) {
                (Some(task), None) => task,
                (None, Some(path)) => std::fs::read_to_string(&path)
                    .with_context(|| format!("failed to read task file {}", path.display()))?,
                _ => unreachable!("clap enforces one task source"),
            };
            let request = orchestrator::RunRequest {
                source: args.source,
                task,
                harnesses: args.harnesses,
                config_path: args.config,
                backend: args.backend,
                timeout_secs: args.timeout,
                max_parallel: args.max_parallel,
                allow_unsafe_local: args.allow_unsafe_local,
                allow_forwarded_env: args.allow_forwarded_env,
            };
            orchestrator::run_dispatch(&state, request)
                .await
                .map(|_| ())
        }
        Command::Status { run_id } => orchestrator::status(&state, run_id.as_deref()),
        Command::History { limit } => orchestrator::history(&state, limit),
        Command::Show { run_id } => orchestrator::show(&state, &run_id),
        Command::Diff {
            run_id,
            candidate,
            stat,
            name_only,
        } => orchestrator::diff(&state, &run_id, &candidate, stat, name_only),
        Command::Inspect {
            run_id,
            candidate,
            shell,
        } => orchestrator::inspect(&state, &run_id, &candidate, shell),
        Command::Compare(args) => {
            let mut evaluation = orchestrator::EvaluationInput {
                winner: args.winner,
                reasons: args.reasons,
                explanation: args.explanation,
            };
            if let Some(path) = args.explanation_file {
                evaluation.explanation = Some(orchestrator::read_verbatim(&path)?);
            }
            orchestrator::compare(&state, &args.run_id, evaluation, args.evaluate)
        }
        Command::Apply { run_id, candidate } => orchestrator::apply(&state, &run_id, &candidate),
        Command::Sync(args) => match args.command {
            Some(SyncCommand::Enable) => dispatch::sync::enable(&state),
            Some(SyncCommand::Disable) => dispatch::sync::disable(&state),
            Some(SyncCommand::Status) => dispatch::sync::status(&state),
            Some(SyncCommand::Preview { run_id }) => dispatch::sync::preview(&state, &run_id),
            Some(SyncCommand::Token(args)) => match args.command {
                SyncTokenCommand::Set { token } => dispatch::sync::token_set(&state, &token),
                SyncTokenCommand::Status => dispatch::sync::token_status(&state),
                SyncTokenCommand::Clear => dispatch::sync::token_clear(&state),
            },
            None => dispatch::sync::flush(&state),
        },
        Command::Version => {
            println!("dispatch {}", dispatch::VERSION);
            Ok(())
        }
    }
}
