//! Claude Code's hooks (`SessionStart`, `SessionEnd`) as `RuntimeEvent`s,
//! and the reply Claude Code shows the person. Everything Claude-specific
//! ends here.

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
        _ => return Ok(None),
    };
    // Claude Code reports the model as a name or as an object with an id.
    let model = input.model.and_then(|model| match model {
        serde_json::Value::String(name) => Some(name),
        other => other["id"].as_str().map(str::to_owned),
    });
    Ok(Some(RuntimeEvent {
        provider: "claude",
        session_id: input.session_id,
        cwd: input.cwd,
        model,
        kind,
    }))
}

/// `dispatch hook claude`: what to print for Claude Code. It never fails the
/// hook: a session is never held up by Dispatch, and a problem is shown.
pub fn handle(state: &State, input: &[u8]) -> String {
    let reply = parse(input).and_then(|event| match event {
        Some(event) => super::ingest(state, event),
        None => Ok(Reply::Silent),
    });
    match reply {
        Ok(Reply::Silent) => String::new(),
        Ok(Reply::Notice(text)) => serde_json::json!({"systemMessage": text}).to_string(),
        Err(error) => serde_json::json!({
            "systemMessage": format!("Dispatch could not track this session: {error:#}")
        })
        .to_string(),
    }
}
