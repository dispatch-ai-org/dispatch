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
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Inspect local resource funding configuration without model calls.
    Resources,
    /// Guided local setup / explicit funding revalidation (requires a terminal).
    Setup {
        /// Choose and explicitly approve project verification commands.
        #[arg(long, conflicts_with = "provider")]
        checks: bool,
        #[arg(value_parser = ["codex", "claude"])]
        provider: Option<String>,
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
    /// Check whether a finished result is still valid against the source as it is now.
    Check {
        run_id: Option<String>,
        /// Print the verdict as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Redo a stale result as a new run on the current source (a new agent launch).
    Refresh(RefreshArgs),
    /// Attach external work Dispatch did not launch (an existing worktree).
    Attach(AttachArgs),
    /// Finish attached external work: freeze the delta, verify, become ready.
    Finish {
        run_id: String,
        /// Required when the integration root configures checks.verify.
        #[arg(long)]
        allow_unsafe_local: bool,
    },
    /// One foreground process per integration root: observes attached work
    /// with no live owner, auto-applies ready work with INTEGRATE, and
    /// prints the project view.
    Serve {
        /// Defaults to the current directory.
        #[arg(long)]
        root: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
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
    /// Print the Dispatch version.
    Version,
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
    #[arg(long, value_delimiter = ',', conflicts_with = "agent", hide = true)]
    harnesses: Option<Vec<String>>,

    /// Deliberately choose one supported coding agent.
    #[arg(long, value_parser = ["claude", "codex", "cursor"], conflicts_with = "harnesses")]
    agent: Option<String>,

    /// Select a configured model resource.
    #[arg(long, conflicts_with = "harnesses")]
    model: Option<String>,

    /// Select the configured provider-specific effort for this attempt.
    #[arg(long, value_parser = ["minimal", "low", "medium", "high", "xhigh"], conflicts_with = "harnesses", hide = true)]
    effort: Option<String>,

    #[arg(long, hide = true)]
    config: Option<PathBuf>,

    #[arg(long, value_parser = ["local", "docker"], hide = true)]
    backend: Option<String>,

    #[arg(long, hide = true)]
    timeout: Option<u64>,

    #[arg(long, hide = true)]
    max_parallel: Option<usize>,

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

    /// Apply the result automatically when verification passed and the work
    /// is coherent with the current source. Records no human review.
    #[arg(long)]
    auto_apply: bool,
}

#[derive(Debug, Args)]
struct RefreshArgs {
    run_id: Option<String>,

    /// Required again if the original run executed on the host.
    #[arg(long)]
    allow_unsafe_local: bool,

    /// Required again if the original run forwarded environment variables.
    #[arg(long)]
    allow_forwarded_env: bool,

    #[arg(long, hide = true)]
    config: Option<PathBuf>,

    /// Print only the final versioned result projection.
    #[arg(long, conflicts_with = "jsonl")]
    json: bool,

    /// Stream committed events followed by the final result as JSON Lines.
    #[arg(long, conflicts_with = "json")]
    jsonl: bool,

    /// Apply the result automatically when verification passed and the work
    /// is coherent with the current source. Records no human review.
    #[arg(long)]
    auto_apply: bool,
}

#[derive(Debug, Args)]
struct AttachArgs {
    /// Existing worktree to observe. Defaults to the current directory.
    #[arg(long)]
    workspace: Option<PathBuf>,

    /// The integration root. Defaults to the repository's main worktree when
    /// `--workspace` is a linked Git worktree; required for a plain directory.
    #[arg(long)]
    root: Option<PathBuf>,

    /// Describes the attached work; never sent to the agent.
    #[arg(long)]
    task: Option<String>,

    /// Free-text label for the external agent; never guessed.
    #[arg(long)]
    agent: Option<String>,

    /// The external agent's process ID, for liveness only; never signaled.
    #[arg(long)]
    pid: Option<u32>,

    /// Explicitly allow `dispatch finish` to run checks.verify on the host.
    #[arg(long)]
    allow_unsafe_local: bool,

    /// Apply automatically once the work is ready and coherent.
    #[arg(long)]
    auto_apply: bool,

    /// Wrapped form: the agent command to run after `--`.
    #[arg(last = true)]
    command: Vec<String>,
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
            },
        )
        .await;
    };
    match command {
        Command::Resources => {
            println!("{}", dispatch::setup::status(&state)?);
            Ok(())
        }
        Command::Setup { provider, checks } => {
            dispatch::presenter::resource_setup(
                &state,
                provider,
                checks,
                dispatch::presenter::Options {
                    plain: cli.plain,
                    ascii: cli.ascii,
                    no_color: cli.no_color,
                },
            )
            .await
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
            let auto_apply = args.auto_apply;
            let json = args.json;
            let jsonl = args.jsonl;
            let (source, task) = read_run_input(&args)?;
            let request = orchestrator::RunRequest {
                source,
                task,
                harnesses: args.harnesses.unwrap_or_default(),
                agent: args.agent,
                model: args.model,
                effort: args.effort,
                config_path: args.config,
                backend: args.backend,
                timeout_secs: args.timeout,
                max_parallel: args.max_parallel,
                priority: match args.priority.as_str() {
                    "background" => -1,
                    "urgent" => 1,
                    _ => 0,
                },
                allow_unsafe_local: args.allow_unsafe_local,
                allow_forwarded_env: args.allow_forwarded_env,
                output: if json {
                    if auto_apply {
                        orchestrator::RunOutputMode::Silent
                    } else {
                        orchestrator::RunOutputMode::Json
                    }
                } else if jsonl {
                    orchestrator::RunOutputMode::Jsonl
                } else {
                    orchestrator::RunOutputMode::Human
                },
                refreshed_from: None,
            };
            let run = orchestrator::run_dispatch(&state, request).await?;
            finish_run(&state, run, auto_apply, json, jsonl)
        }
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
        Command::Check { run_id, json } => {
            orchestrator::check(&state, run_id.as_deref(), &std::env::current_dir()?, json)
        }
        Command::Refresh(args) => {
            let auto_apply = args.auto_apply;
            let json = args.json;
            let jsonl = args.jsonl;
            let output = if json {
                if auto_apply {
                    orchestrator::RunOutputMode::Silent
                } else {
                    orchestrator::RunOutputMode::Json
                }
            } else if jsonl {
                orchestrator::RunOutputMode::Jsonl
            } else {
                orchestrator::RunOutputMode::Human
            };
            let request = orchestrator::refresh_request(
                &state,
                args.run_id.as_deref(),
                &std::env::current_dir()?,
                orchestrator::RefreshOptions {
                    allow_unsafe_local: args.allow_unsafe_local,
                    allow_forwarded_env: args.allow_forwarded_env,
                    config_path: args.config,
                    output,
                },
            )?;
            let old = request.refreshed_from.clone().unwrap_or_default();
            let run = orchestrator::run_dispatch(&state, request).await?;
            if output == orchestrator::RunOutputMode::Human {
                println!("Refreshed from {old}; new run {}", run.id);
            }
            finish_run(&state, run, auto_apply, json, jsonl)
        }
        Command::Attach(args) => {
            let workspace = args.workspace.unwrap_or_else(|| PathBuf::from("."));
            let wrapped = !args.command.is_empty();
            let request = orchestrator::attach::AttachRequest {
                workspace,
                root: args.root,
                task: args.task,
                agent: args.agent,
                pid: args.pid,
                command: wrapped.then_some(args.command),
                allow_unsafe_local: args.allow_unsafe_local,
                auto_apply: args.auto_apply,
            };
            if wrapped {
                let code = orchestrator::attach::run_wrapped(&state, request).await?;
                if code != 0 {
                    std::process::exit(code);
                }
                Ok(())
            } else {
                orchestrator::attach::create(&state, request)?;
                Ok(())
            }
        }
        Command::Finish {
            run_id,
            allow_unsafe_local,
        } => {
            orchestrator::attach::finish(&state, &run_id, allow_unsafe_local).await?;
            Ok(())
        }
        Command::Serve { root, json } => orchestrator::serve::serve(&state, root, json).await,
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
        Command::Version => {
            println!("dispatch {}", dispatch::VERSION);
            Ok(())
        }
    }
}

/// After `run_dispatch` returns for `run`/`refresh`: report the result, and
/// when `--auto-apply` was set and the run reached Ready, apply it
/// automatically and report that outcome too, on every output mode.
///
/// Without the flag (or when the run is not Ready) this reproduces exactly
/// today's behavior: the exit code from `run_result`, nothing else. `--json`
/// only ever needed `RunOutputMode::Silent` (instead of `Json`) to defer its
/// print until the auto-apply decision is known; the not-Ready path below
/// prints the identical JSON itself so that deferral is invisible from the
/// outside. `--jsonl` never changes its `run_dispatch` output mode: the
/// auto-apply attempt's own events are streamed by the existing publisher
/// because the global output mode set by `run_dispatch` is still JSONL.
///
/// Exit code: `0` when applied; `6` when the run was Ready but the outcome
/// is skipped, blocked or failed and the run would otherwise have exited
/// `0`; otherwise the run's own exit code, unchanged (a run that already
/// failed for its own reason, for example verification, is not relabeled
/// "not applied automatically").
fn finish_run(
    state: &State,
    run: dispatch::RunRecord,
    auto_apply: bool,
    json: bool,
    jsonl: bool,
) -> Result<()> {
    let result = orchestrator::run_result(&run);
    let base_exit_code = result.exit_code;
    if !auto_apply || run.outcome.work_result != dispatch::WorkResult::Ready {
        if auto_apply && json {
            println!("{}", serde_json::to_string(&result)?);
        }
        if base_exit_code != 0 {
            std::process::exit(base_exit_code);
        }
        return Ok(());
    }

    let outcome = orchestrator::auto_apply(state, &run.id)?;
    let exit_code = match &outcome {
        orchestrator::ApplyOutcome::Applied { .. } => 0,
        _ if base_exit_code == 0 => 6,
        _ => base_exit_code,
    };

    if json {
        let reloaded = state.load_run(&run.id)?;
        let mut result = orchestrator::run_result(&reloaded);
        result.auto_apply = Some(outcome.summary());
        println!("{}", serde_json::to_string(&result)?);
    } else if jsonl {
        println!(
            "{}",
            serde_json::json!({
                "type": "auto_apply",
                "run_id": run.id,
                "auto_apply": outcome.summary(),
            })
        );
    } else if base_exit_code == 0 {
        match &outcome {
            orchestrator::ApplyOutcome::Applied { report, .. } => println!(
                "Auto-applied Candidate {} to {} ({} file(s) changed). Review not performed.",
                report.candidate_label,
                run.source_path.display(),
                report.files_changed
            ),
            orchestrator::ApplyOutcome::Blocked { reason, .. } => println!(
                "Not applied automatically: {reason}. Review with dispatch check {0} or dispatch accept {0}.",
                run.id
            ),
            orchestrator::ApplyOutcome::Skipped { reason } => println!(
                "Not applied automatically: {reason}. Review with dispatch check {0} or dispatch accept {0}.",
                run.id
            ),
            orchestrator::ApplyOutcome::Failed { error } => println!(
                "Not applied automatically: {error}. Review with dispatch check {0} or dispatch accept {0}.",
                run.id
            ),
        }
    }

    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
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
