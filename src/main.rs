use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, CommandFactory, Parser, Subcommand};
use dispatch::{orchestrator, state::State};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "dispatch",
    version,
    about = "Dispatch — give a software task to the best available coding agent"
)]
struct Cli {
    /// Override ~/.dispatch (also available as DISPATCH_HOME).
    #[arg(long, global = true, env = "DISPATCH_HOME")]
    state_dir: Option<PathBuf>,

    /// Show internal diagnostics. Repeat for more detail.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Use ordinary line input and scrollback instead of the live viewport.
    #[arg(long, global = true)]
    plain: bool,
    /// Use ASCII graph characters.
    #[arg(long, global = true)]
    ascii: bool,
    /// Keep native terminal colors (also respects NO_COLOR).
    #[arg(long, global = true)]
    no_color: bool,
    /// Disable bounded recovery in the interactive session.
    #[arg(long, hide = true)]
    no_retry: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Foreground, scoped bidirectional JSON Lines (requires a human-issued grant).
    Control {
        #[arg(long, required = true)]
        stdio: bool,
        #[arg(long)]
        grant_fd: i32,
        #[arg(long)]
        read_only: bool,
    },
    /// Authorize a fixed machine scope as the local human owner; prints a private key path.
    ControlGrant {
        source: PathBuf,
        #[arg(long, default_value_t = 600)]
        timeout: u64,
        #[arg(long, default_value_t = 2)]
        max_invocations: u32,
        #[arg(long)]
        allow_unsafe_local: bool,
        #[arg(long)]
        delegate_factual: bool,
    },
    /// Follow committed semantic events (advanced, read-only JSON Lines).
    #[command(hide = true)]
    Events {
        run_id: String,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, value_enum)]
        until: Option<dispatch::follow::Until>,
        #[arg(long, default_value_t = 30)]
        timeout: u64,
    },
    /// Create a small project-local Dispatch configuration.
    #[command(hide = true)]
    Init {
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Replace an existing dispatch.yml.
        #[arg(long)]
        force: bool,
    },
    /// Check local dependencies and harness availability.
    #[command(hide = true)]
    Doctor {
        #[arg(default_value = ".")]
        source: PathBuf,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Work on a software task with the best available coding agent.
    Run(RunArgs),
    /// Answer a durable clarification and continue within the existing goal limit.
    Answer {
        run_id: String,
        question_id: String,
        #[arg(long)]
        revision: u64,
        #[arg(long, default_value_t = 1)]
        generation: u32,
        #[arg(long)]
        answer: String,
        #[arg(long)]
        json: bool,
    },
    /// Cancel a goal awaiting a durable clarification.
    Cancel {
        run_id: String,
        question_id: String,
        #[arg(long)]
        revision: u64,
        #[arg(long, default_value_t = 1)]
        generation: u32,
        #[arg(long)]
        json: bool,
    },
    /// Inspect locally cached evidence for supported real harnesses.
    #[command(hide = true)]
    Recommend(RecommendArgs),
    /// Inspect observed local outcomes without changing routing.
    #[command(hide = true)]
    Evidence {
        #[command(subcommand)]
        command: EvidenceCommand,
    },
    /// Show the state of one run (or the latest run).
    Status {
        run_id: Option<String>,
        /// Print the versioned result projection as JSON.
        #[arg(long)]
        json: bool,
        /// Replay committed events and the result as JSON Lines.
        #[arg(long, conflicts_with = "json")]
        jsonl: bool,
    },
    /// List recent runs.
    History {
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    /// Explain why Dispatch chose the agent for the latest task.
    Explain { run_id: Option<String> },
    /// Show a run and its persisted signals.
    #[command(hide = true)]
    Show { run_id: String },
    /// Print a candidate's patch.
    Diff {
        run_id: Option<String>,
        candidate: Option<String>,
        #[arg(long, conflicts_with = "name_only")]
        stat: bool,
        #[arg(long, conflicts_with = "stat")]
        name_only: bool,
    },
    /// Accept and safely apply the latest single result.
    Accept(ReviewArgs),
    /// Reject the latest single result without changing the source tree.
    Reject(ReviewArgs),
    /// Print the path to a candidate's complete workspace.
    #[command(hide = true)]
    Inspect {
        run_id: String,
        candidate: String,
        /// Start the user's shell in the candidate workspace.
        #[arg(long)]
        shell: bool,
    },
    /// Compare blind candidates and optionally persist an evaluation.
    #[command(hide = true)]
    Compare(CompareArgs),
    /// Record accept/reject feedback for one predictively routed result.
    #[command(hide = true)]
    Evaluate(EvaluateArgs),
    /// Safely apply one candidate to the original source.
    #[command(hide = true)]
    Apply { run_id: String, candidate: String },
    /// Import public benchmark snapshots into the local evidence cache.
    #[command(hide = true)]
    Datasets {
        #[command(subcommand)]
        command: DatasetCommand,
    },
    /// Refresh the compact public routing data cache.
    #[command(hide = true)]
    Data {
        #[command(subcommand)]
        command: DataCommand,
    },
    /// Review opt-in records; bare `dispatch sync` explicitly transmits queued data.
    #[command(hide = true)]
    Sync {
        #[command(subcommand)]
        command: Option<SyncCommand>,
    },
    /// Print the Dispatch version.
    Version,
}

#[derive(Debug, Subcommand)]
enum DatasetCommand {
    /// Import a local benchmark snapshot without network access.
    Import {
        #[arg(value_parser = ["swe-bench", "terminal-bench"])]
        dataset: String,
        path: PathBuf,
    },
    /// Export imported normalized priors as a versioned maintainer snapshot.
    ExportPublicPriors { output: PathBuf },
}

#[derive(Debug, Subcommand)]
enum DataCommand {
    /// Fetch the latest compact public routing data; existing data remains on failure.
    Refresh,
}

#[derive(Debug, Subcommand)]
enum EvidenceCommand {
    /// Count routed outcomes for this source location, without ranking harnesses.
    Local { source: PathBuf },
}

#[derive(Debug, Subcommand)]
enum SyncCommand {
    /// Record current contribution consent and queue eligible records; does not upload.
    Enable,
    /// Disable all contribution uploads.
    Disable,
    /// Show consent and outbox state without contacting the cloud.
    Status,
    /// Print the exact eligible upload body without transmitting; requires consent.
    Preview {
        run_id: String,
        /// Select a record when the run has both evaluation and routing data.
        #[arg(long = "type", value_parser = ["evaluation", "routing-observation", "routing-feedback"])]
        record_type: Option<String>,
        /// Select one immutable routed-feedback revision.
        #[arg(long, requires = "record_type", value_parser = clap::value_parser!(u32).range(1..))]
        revision: Option<u32>,
    },
    /// Manage the developer-preview Dispatch Cloud ingestion token.
    Token {
        #[command(subcommand)]
        command: SyncTokenCommand,
    },
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
    /// Desired software change. The source defaults to the current directory.
    task_or_legacy_source: Option<String>,

    /// Work on a source tree other than the current directory.
    #[arg(long)]
    source: Option<PathBuf>,

    /// Legacy task form retained for scripts using `run <source> --task ...`.
    #[arg(long, conflicts_with = "task_file", hide = true)]
    task: Option<String>,

    /// Read the task verbatim from a file, or '-' for stdin.
    #[arg(long, conflicts_with = "task")]
    task_file: Option<PathBuf>,

    /// Comma-separated adapter IDs. Fakes make the complete workflow testable offline.
    #[arg(long, value_delimiter = ',', conflicts_with_all = ["route", "agent"], hide = true)]
    harnesses: Option<Vec<String>>,

    /// Deliberately choose one supported coding agent.
    #[arg(long, value_parser = ["claude", "codex", "cursor"], conflicts_with_all = ["route", "harnesses"])]
    agent: Option<String>,

    /// Select a configured model resource.
    #[arg(long, conflicts_with_all = ["route", "harnesses"])]
    model: Option<String>,

    /// Select the configured provider-specific effort for this attempt.
    #[arg(long, value_parser = ["minimal", "low", "medium", "high", "xhigh"], conflicts_with_all = ["route", "harnesses"], hide = true)]
    effort: Option<String>,

    /// Select one locally runnable real harness using cached routing evidence.
    #[arg(long, conflicts_with_all = ["harnesses", "agent"], hide = true)]
    route: bool,

    #[arg(long, hide = true)]
    config: Option<PathBuf>,

    #[arg(long, value_parser = ["local", "docker"], hide = true)]
    backend: Option<String>,

    #[arg(long, hide = true)]
    timeout: Option<u64>,

    #[arg(long, hide = true)]
    max_parallel: Option<usize>,

    /// Disable the single automatic stronger recovery.
    #[arg(long, hide = true)]
    no_retry: bool,

    /// Admission priority within the shared local subscription pool.
    #[arg(long, value_parser = ["background", "normal", "urgent"], default_value = "normal", hide = true)]
    priority: String,

    /// Explicitly allow real agents or project checks to execute on the host.
    #[arg(long, hide = true)]
    allow_unsafe_local: bool,

    /// Forward only the environment variable names allowlisted in dispatch.yml.
    #[arg(long, hide = true)]
    allow_forwarded_env: bool,

    /// Print only the final versioned result projection.
    #[arg(long, conflicts_with = "jsonl")]
    json: bool,

    /// Stream committed events followed by the final result as JSON Lines.
    #[arg(long, conflicts_with = "json")]
    jsonl: bool,
}

#[derive(Debug, Args)]
struct RecommendArgs {
    #[arg(default_value = ".")]
    source: PathBuf,

    #[command(flatten)]
    task_input: TaskInput,
}

#[derive(Debug, Args)]
struct TaskInput {
    #[arg(
        long,
        required_unless_present = "task_file",
        conflicts_with = "task_file"
    )]
    task: Option<String>,

    #[arg(long, required_unless_present = "task", conflicts_with = "task")]
    task_file: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct CompareArgs {
    run_id: String,

    /// Submit A/B/.../tie/neither without the interactive prompt.
    #[arg(long)]
    winner: Option<String>,

    #[command(flatten)]
    feedback: FeedbackArgs,

    /// Prompt for outcome, labels, and multiline freeform text.
    #[arg(long)]
    evaluate: bool,
}

#[derive(Debug, Args)]
struct FeedbackArgs {
    /// Optional structured reason; repeat the flag for multiple labels.
    #[arg(long = "reason")]
    reasons: Vec<String>,

    /// Optional unrestricted explanation, stored verbatim.
    #[arg(long, conflicts_with = "explanation_file")]
    explanation: Option<String>,

    /// Read unrestricted explanation verbatim from this path, or '-' for stdin.
    #[arg(long, conflicts_with = "explanation")]
    explanation_file: Option<PathBuf>,
}

impl FeedbackArgs {
    fn read_explanation(&self) -> Result<Option<String>> {
        match &self.explanation_file {
            Some(path) => orchestrator::read_verbatim(path).map(Some),
            None => Ok(self.explanation.clone()),
        }
    }
}

#[derive(Debug, Args)]
struct EvaluateArgs {
    run_id: String,

    /// Whether the routed result was acceptable for the task.
    #[arg(long, value_parser = ["accept", "reject"])]
    outcome: String,

    #[command(flatten)]
    feedback: FeedbackArgs,
}

#[derive(Debug, Args)]
struct ReviewArgs {
    run_id: Option<String>,

    #[command(flatten)]
    feedback: FeedbackArgs,
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
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();

    let state = State::discover(cli.state_dir)?;
    let Some(command) = cli.command else {
        if !dispatch::presenter::suitable() {
            Cli::command().print_help()?;
            println!();
            std::process::exit(2);
        }
        return dispatch::presenter::session(
            &state,
            dispatch::presenter::Options {
                plain: cli.plain,
                ascii: cli.ascii,
                no_color: cli.no_color,
                no_retry: cli.no_retry,
            },
        )
        .await;
    };
    match command {
        Command::Control {
            stdio: _,
            grant_fd,
            read_only,
        } => dispatch::control::stdio(state, grant_fd, read_only).await,
        Command::ControlGrant {
            source,
            timeout,
            max_invocations,
            allow_unsafe_local,
            delegate_factual,
        } => {
            let path = dispatch::commands::grant(
                &state,
                &source,
                timeout,
                max_invocations,
                allow_unsafe_local,
                delegate_factual,
            )?;
            println!("{}", path.display());
            Ok(())
        }
        Command::Events {
            run_id,
            after,
            until,
            timeout,
        } => {
            let reached = dispatch::follow::events(
                &state,
                &run_id,
                after,
                until,
                std::time::Duration::from_secs(timeout),
                std::io::stdout(),
            )
            .await?;
            if !reached {
                std::process::exit(124);
            }
            Ok(())
        }
        Command::Init { path, force } => orchestrator::init(&state, &path, force),
        Command::Doctor { source, config } => {
            orchestrator::doctor(&state, &source, config.as_deref()).await
        }
        Command::Answer {
            run_id,
            question_id,
            revision,
            generation,
            answer,
            json,
        } => {
            let output = if json {
                orchestrator::RunOutputMode::Json
            } else {
                orchestrator::RunOutputMode::Human
            };
            let run = orchestrator::answer_question(
                &state,
                orchestrator::QuestionCommand {
                    run_id,
                    question_id,
                    revision,
                    generation,
                },
                answer,
                output,
            )
            .await?;
            let code = orchestrator::run_result(&run).exit_code;
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }
        Command::Cancel {
            run_id,
            question_id,
            revision,
            generation,
            json,
        } => {
            orchestrator::cancel_question(
                &state,
                orchestrator::QuestionCommand {
                    run_id,
                    question_id,
                    revision,
                    generation,
                },
                if json {
                    orchestrator::RunOutputMode::Json
                } else {
                    orchestrator::RunOutputMode::Human
                },
            )?;
            Ok(())
        }
        Command::Run(args) => {
            let (source, task) = read_run_input(&args)?;
            let request = orchestrator::RunRequest {
                source,
                task,
                harnesses: args.harnesses.unwrap_or_default(),
                route: args.route,
                agent: args.agent,
                model: args.model,
                effort: args.effort,
                config_path: args.config,
                backend: args.backend,
                timeout_secs: args.timeout,
                max_parallel: args.max_parallel,
                no_retry: args.no_retry,
                priority: match args.priority.as_str() {
                    "background" => -1,
                    "urgent" => 1,
                    _ => 0,
                },
                allow_unsafe_local: args.allow_unsafe_local,
                allow_forwarded_env: args.allow_forwarded_env,
                output: if args.json {
                    orchestrator::RunOutputMode::Json
                } else if args.jsonl {
                    orchestrator::RunOutputMode::Jsonl
                } else {
                    orchestrator::RunOutputMode::Human
                },
            };
            let run = orchestrator::run_dispatch(&state, request).await?;
            let exit_code = orchestrator::run_result(&run).exit_code;
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
            Ok(())
        }
        Command::Recommend(args) => {
            let task = read_task(args.task_input)?;
            orchestrator::recommend(&state, &args.source, &task)
        }
        Command::Evidence {
            command: EvidenceCommand::Local { source },
        } => dispatch::evidence::inspect_local(&state, &source),
        Command::Status {
            run_id,
            json,
            jsonl,
        } => {
            if json {
                orchestrator::status_json(&state, run_id.as_deref(), &std::env::current_dir()?)
            } else if jsonl {
                orchestrator::status_jsonl(&state, run_id.as_deref(), &std::env::current_dir()?)
            } else {
                orchestrator::status(&state, run_id.as_deref(), &std::env::current_dir()?)
            }
        }
        Command::History { limit } => orchestrator::history(&state, limit),
        Command::Explain { run_id } => {
            orchestrator::explain(&state, run_id.as_deref(), &std::env::current_dir()?)
        }
        Command::Show { run_id } => orchestrator::show(&state, &run_id),
        Command::Diff {
            run_id,
            candidate,
            stat,
            name_only,
        } => orchestrator::diff(
            &state,
            run_id.as_deref(),
            candidate.as_deref(),
            &std::env::current_dir()?,
            stat,
            name_only,
        ),
        Command::Accept(args) => {
            let explanation = args.feedback.read_explanation()?;
            orchestrator::accept_or_reject_latest(
                &state,
                args.run_id.as_deref(),
                &std::env::current_dir()?,
                true,
                args.feedback.reasons,
                explanation,
            )
        }
        Command::Reject(args) => {
            let explanation = args.feedback.read_explanation()?;
            orchestrator::accept_or_reject_latest(
                &state,
                args.run_id.as_deref(),
                &std::env::current_dir()?,
                false,
                args.feedback.reasons,
                explanation,
            )
        }
        Command::Inspect {
            run_id,
            candidate,
            shell,
        } => orchestrator::inspect(&state, &run_id, &candidate, shell),
        Command::Compare(args) => {
            let evaluation = orchestrator::EvaluationInput {
                winner: args.winner,
                explanation: args.feedback.read_explanation()?,
                reasons: args.feedback.reasons,
            };
            orchestrator::compare(&state, &args.run_id, evaluation, args.evaluate)
        }
        Command::Evaluate(args) => {
            let evaluation = orchestrator::RoutingEvaluationInput {
                outcome: args.outcome,
                explanation: args.feedback.read_explanation()?,
                reasons: args.feedback.reasons,
            };
            orchestrator::evaluate_routed(&state, &args.run_id, evaluation)
        }
        Command::Apply { run_id, candidate } => orchestrator::apply(&state, &run_id, &candidate),
        Command::Datasets { command } => match command {
            DatasetCommand::Import { dataset, path } => {
                let report = match dataset.as_str() {
                    "swe-bench" => dispatch::datasets::import_swe_bench(&state, &path)?,
                    "terminal-bench" => dispatch::datasets::import_terminal_bench(&state, &path)?,
                    _ => unreachable!("clap validates dataset names"),
                };
                println!(
                    "Imported {} {} for harness {} (model {})",
                    report.prior.dataset,
                    report.prior.dataset_version,
                    report.prior.harness,
                    report.prior.model.as_deref().unwrap_or("unknown")
                );
                println!(
                    "Observed {}/{} successful attempts; cached raw snapshot at {}",
                    report.prior.successes,
                    report.prior.attempts,
                    report.raw_snapshot_path.display()
                );
                Ok(())
            }
            DatasetCommand::ExportPublicPriors { output } => {
                let snapshot = dispatch::public_priors::export_public_priors(&state, &output)?;
                println!(
                    "Exported {} public prior entries to {}\nSnapshot: {}",
                    snapshot.entries.len(),
                    output.display(),
                    snapshot.snapshot_id
                );
                Ok(())
            }
        },
        Command::Data {
            command: DataCommand::Refresh,
        } => {
            match dispatch::public_priors::refresh(&state) {
                Ok((dispatch::public_priors::RefreshResult::Updated, snapshot)) => {
                    let agents = snapshot
                        .entries
                        .iter()
                        .map(|entry| entry.harness.as_str())
                        .collect::<std::collections::BTreeSet<_>>()
                        .len();
                    println!("Public routing data updated.");
                    println!("Snapshot: {}", snapshot.snapshot_id);
                    println!("Agents: {agents}");
                    println!("Evidence groups: {}", snapshot.entries.len());
                }
                Ok((dispatch::public_priors::RefreshResult::Current, snapshot)) => {
                    println!("Public routing data is up to date.");
                    println!("Snapshot: {}", snapshot.snapshot_id);
                }
                Err(_) => {
                    println!("Could not refresh public routing data.");
                    println!("Existing local data remains available.");
                }
            }
            Ok(())
        }
        Command::Sync { command } => match command {
            Some(SyncCommand::Enable) => dispatch::sync::enable(&state),
            Some(SyncCommand::Disable) => dispatch::sync::disable(&state),
            Some(SyncCommand::Status) => dispatch::sync::status(&state),
            Some(SyncCommand::Preview {
                run_id,
                record_type,
                revision,
            }) => dispatch::sync::preview(&state, &run_id, record_type.as_deref(), revision),
            Some(SyncCommand::Token { command }) => match command {
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

fn read_task(input: TaskInput) -> Result<String> {
    match (input.task, input.task_file) {
        (Some(task), None) => Ok(task),
        (None, Some(path)) => std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read task file {}", path.display())),
        _ => unreachable!("clap enforces one task source"),
    }
}

fn read_run_input(args: &RunArgs) -> Result<(PathBuf, String)> {
    let legacy_task = match (&args.task, &args.task_file) {
        (Some(task), None) => Some(task.clone()),
        (None, Some(path)) => Some(orchestrator::read_verbatim(path)?),
        (None, None) => None,
        (Some(_), Some(_)) => unreachable!("clap rejects multiple task inputs"),
    };
    match legacy_task {
        Some(task) => Ok((
            args.source.clone().unwrap_or_else(|| {
                args.task_or_legacy_source
                    .as_deref()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("."))
            }),
            task,
        )),
        None => Ok((
            args.source.clone().unwrap_or_else(|| PathBuf::from(".")),
            args.task_or_legacy_source
                .clone()
                .context("a task is required; use `dispatch run \"<task>\"`")?,
        )),
    }
}
