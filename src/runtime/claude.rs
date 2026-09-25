//! Claude Code's hooks (`SessionStart`, `SessionEnd`, `WorktreeRemove`) as
//! `RuntimeEvent`s, and the reply Claude Code shows the person. Everything
//! Claude-specific ends here.

use std::{sync::mpsc, time::Duration};

use anyhow::{Context, Result};
use serde::Deserialize;

use super::{EventKind, Reply, RuntimeEvent};
use crate::state::State;

#[derive(Deserialize)]
struct Input {
    hook_event_name: String,
    session_id: String,
    cwd: std::path::PathBuf,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    model: Option<serde_json::Value>,
    /// `WorktreeRemove`: the worktree about to be deleted.
    #[serde(default)]
    worktree_path: Option<std::path::PathBuf>,
}

/// The event in a hook's input, or `None` for an event Dispatch does not use.
pub fn parse(input: &[u8]) -> Result<Option<RuntimeEvent>> {
    anyhow::ensure!(
        input.len() <= super::MAX_INPUT_BYTES,
        "hook input is larger than {} bytes",
        super::MAX_INPUT_BYTES
    );
    let input: Input = serde_json::from_slice(input).context("unreadable hook input")?;
    let kind = match input.hook_event_name.as_str() {
        "SessionStart" => {
            let source = input.source.unwrap_or_else(|| "startup".into());
            // A new session, or a fork into its own workspace, begins before
            // any edit; anything else may follow edits made earlier.
            let resumed = !matches!(source.as_str(), "startup" | "fork");
            EventKind::Start { source, resumed }
        }
        "SessionEnd" => EventKind::End {
            reason: input.reason.unwrap_or_else(|| "other".into()),
        },
        "WorktreeRemove" => EventKind::WorkspaceRemoved,
        _ => return Ok(None),
    };
    let cwd = match kind {
        EventKind::WorkspaceRemoved => input
            .worktree_path
            .context("WorktreeRemove names no worktree")?,
        _ => input.cwd,
    };
    // Claude Code reports the model as a name or as an object with an id.
    let model = input.model.and_then(|model| match model {
        serde_json::Value::String(name) => Some(name),
        other => other["id"].as_str().map(str::to_owned),
    });
    Ok(Some(RuntimeEvent {
        provider: "claude",
        session_id: input.session_id,
        cwd,
        model,
        kind,
    }))
}

/// What `dispatch hook claude` prints, and its exit status.
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

/// How long Dispatch may take to keep a removed worktree's Δ: below the
/// `WorktreeRemove` timeout setup installs, so Dispatch decides, not Claude.
const REMOVAL_DEADLINE: Duration = Duration::from_secs(240);

/// `dispatch hook claude`. A session is never held up or failed by Dispatch:
/// a problem is shown to the person. The one exception is a worktree about to
/// be deleted whose Δ Dispatch could not keep: the hook fails, so Claude Code
/// keeps the worktree and the work in it.
pub fn handle(state: &State, input: &[u8]) -> Output {
    let event = match parse(input) {
        Ok(Some(event)) => event,
        Ok(None) => return notice(Ok(Reply::Silent)),
        Err(error) => return notice(Err(error)),
    };
    if !matches!(event.kind, EventKind::WorkspaceRemoved) {
        return notice(super::ingest(state, event));
    }
    // Run it where a deadline can be kept; if the deadline passes, this
    // process exits and the attempt with it, leaving nothing half-written
    // (the Δ is renamed into place and the removal committed atomically).
    let (sender, receiver) = mpsc::channel();
    let state = state.clone();
    std::thread::spawn(move || {
        let _ = sender.send(super::ingest(&state, event));
    });
    let outcome = receiver
        .recv_timeout(REMOVAL_DEADLINE)
        .unwrap_or_else(|_| Err(anyhow::anyhow!("keeping its changes took too long")));
    match outcome {
        Ok(reply) => notice(Ok(reply)),
        Err(error) => Output {
            stdout: String::new(),
            stderr: format!(
                "Dispatch could not keep this worktree's changes, so it stopped the removal; \
                 the worktree is intact: {error:#}"
            ),
            code: 2,
        },
    }
}

fn notice(reply: Result<Reply>) -> Output {
    let stdout = match reply {
        Ok(Reply::Silent) => String::new(),
        Ok(Reply::Notice(text)) => serde_json::json!({"systemMessage": text}).to_string(),
        Err(error) => serde_json::json!({
            "systemMessage": format!("Dispatch could not track this session: {error:#}")
        })
        .to_string(),
    };
    Output {
        stdout,
        stderr: String::new(),
        code: 0,
    }
}
