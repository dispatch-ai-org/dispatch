//! L2 integration oracle: when the source moved but the static layers see no
//! conflict, run the project's own `checks.verify` commands against the merged
//! result (current source plus the candidate's patch) in a scratch tree before
//! anything is applied. Static facts cannot see transitive or behavioral
//! breakage; the user's checks can, and they cost no tokens.
//!
//! The source is only read. The scratch tree omits ignored build output (see
//! `create_scratch_tree`) and is removed on every exit path; the
//! check logs are kept under the run directory as evidence.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use ulid::Ulid;

use super::{MAX_DETAIL_CHARS, MAX_REASONS, find_candidate, patch_is_empty};
use crate::{
    AnalysisLevel, CheckPhase, CheckResult, CheckStatus, Decision, Reason, ReasonCode, RunRecord,
    Validity,
    config::Config,
    executor::run_checks_with_config,
    source::{apply_patch_in_workspace, create_scratch_tree},
};

/// Longest slice of a configured command quoted in a reason, so the log path
/// after it survives the overall detail cap.
const MAX_COMMAND_CHARS: usize = 80;

/// Removes the scratch tree when dropped, whatever path leaves the function.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Verify `candidate_label` against the merged tree. Returns `validity`
/// unchanged when nothing needs verifying or this run has no authority to run
/// local commands. A failing check downgrades the verdict to Refresh; so does
/// an infrastructure error, because a false Refresh costs a rerun while a false
/// Continue would corrupt the source.
pub async fn verify_integration(
    run: &RunRecord,
    candidate_label: &str,
    validity: Validity,
    config: &Config,
    run_dir: &Path,
) -> Result<Validity> {
    if !validity.world_changed
        || !config.coherence.integration_checks
        || config.checks.verify.is_empty()
        // Never widen authority at accept time: local commands need the
        // acknowledgement the run itself recorded.
        || (config.execution.backend == "local" && !run.environment.unsafe_local)
    {
        return Ok(validity);
    }
    let patch = find_candidate(run, candidate_label)?.diff_path.clone();
    if patch_is_empty(&patch)? {
        return Ok(validity);
    }

    let id = Ulid::new().to_string();
    let output_dir = run_dir.join("coherence-checks").join(&id);
    let new_reasons = match run_checks(run, &patch, config, run_dir, &id, &output_dir).await {
        Ok(results) => match results
            .iter()
            .find(|result| result.status != CheckStatus::Passed)
        {
            None => {
                return Ok(Validity {
                    analysis: AnalysisLevel::Integration,
                    ..validity
                });
            }
            Some(_) => failure_reasons(&results),
        },
        Err(error) => vec![Reason {
            code: ReasonCode::AnalysisUncertain,
            fact_id: None,
            path: None,
            detail: cap(format!(
                "{COULD_NOT_RUN}: {error:#}; set coherence.integration_checks: false to skip"
            )),
        }],
    };
    let mut reasons = validity.reasons;
    reasons.extend(new_reasons);
    reasons.truncate(MAX_REASONS);
    Ok(Validity {
        decision: Decision::Refresh,
        reasons,
        ..validity
    })
}

/// Build the merged tree in a scratch area and run the verify commands there.
async fn run_checks(
    run: &RunRecord,
    patch: &Path,
    config: &Config,
    run_dir: &Path,
    id: &str,
    output_dir: &Path,
) -> Result<Vec<CheckResult>> {
    let scratch = Scratch(run_dir.join(format!("coherence-scratch-{id}")));
    // The current source minus ignored build output, as a plain directory;
    // `git apply` does not need a repository.
    create_scratch_tree(&run.source_path, &run.source_kind, &scratch.0)
        .context("failed to copy the current source")?;
    apply_patch_in_workspace(&scratch.0, patch)?;
    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;
    let results = run_checks_with_config(
        &scratch.0,
        &config.checks.verify,
        CheckPhase::Verify,
        output_dir,
        config.execution.clone(),
    )
    .await;
    if results.len() != config.checks.verify.len() {
        bail!("verification checks did not all report a result");
    }
    Ok(results)
}

/// One bounded reason per check that did not pass.
fn failure_reasons(results: &[CheckResult]) -> Vec<Reason> {
    results
        .iter()
        .filter(|result| result.status != CheckStatus::Passed)
        .map(|result| {
            let command = result
                .command
                .chars()
                .take(MAX_COMMAND_CHARS)
                .collect::<String>();
            let logs = result.stderr_path.display();
            let (code, what) = match result.status {
                CheckStatus::Failed => (
                    ReasonCode::IntegrationCheckFailed,
                    match result.exit_code {
                        Some(code) => format!("failed (exit {code})"),
                        None => "failed (no exit code)".to_owned(),
                    },
                ),
                CheckStatus::TimedOut => {
                    (ReasonCode::IntegrationCheckFailed, "timed out".to_owned())
                }
                _ => (ReasonCode::AnalysisUncertain, "could not run".to_owned()),
            };
            Reason {
                code,
                fact_id: None,
                path: None,
                detail: cap(format!("check `{command}` {what}; logs: {logs}")),
            }
        })
        .collect()
}

const COULD_NOT_RUN: &str = "integration checks could not run";

/// Whether `reason` came from running the checks on the merged tree, which only
/// accept does; a file and symbol evaluation can never reproduce it.
pub fn is_integration_reason(reason: &Reason) -> bool {
    reason.code == ReasonCode::IntegrationCheckFailed || reason.detail.starts_with(COULD_NOT_RUN)
}

fn cap(detail: String) -> String {
    detail.chars().take(MAX_DETAIL_CHARS).collect()
}
