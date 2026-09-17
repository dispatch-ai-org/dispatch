//! Read-only following of committed semantic events. No execution authority.
use crate::{LifecycleState, RunOutcome, WaitingOn, db::Database, state::State};
use anyhow::{Context, Result};
use std::{io::Write, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Until {
    Attention,
    Finished,
}
impl Until {
    pub fn reached(self, outcome: &RunOutcome) -> bool {
        outcome.lifecycle == LifecycleState::Finished
            || (self == Self::Attention && outcome.waiting_on == WaitingOn::Human)
    }
}

pub async fn events(
    state: &State,
    id: &str,
    mut after: u64,
    until: Option<Until>,
    timeout: Duration,
    mut output: impl Write,
) -> Result<bool> {
    let id = state.resolve_run_id(id)?;
    let db = Database::open_read_only(state.db_path())?;
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let (events, current) = db.event_page(&id, after, 64)?;
        let current = current.context("run has no committed projection")?;
        let empty = events.is_empty();
        for event in events {
            after = event.sequence;
            let outcome = event
                .payload
                .get("outcome")
                .cloned()
                .map(serde_json::from_value::<RunOutcome>)
                .transpose()?;
            serde_json::to_writer(
                &mut output,
                &serde_json::json!({"type":"event","event":event}),
            )?;
            writeln!(output)?;
            if until.is_some_and(|until| outcome.as_ref().is_some_and(|o| until.reached(o))) {
                serde_json::to_writer(
                    &mut output,
                    &serde_json::json!({"type":"cursor","run_id":id,"after":after,"reached":true}),
                )?;
                writeln!(output)?;
                output.flush()?;
                return Ok(true);
            }
        }
        output.flush()?;
        if empty {
            let reached = until.is_some_and(|u| u.reached(&current.outcome));
            if until.is_none() || reached || tokio::time::Instant::now() >= deadline {
                serde_json::to_writer(
                    &mut output,
                    &serde_json::json!({"type":"cursor","run_id":id,"after":after,"reached":reached,"timed_out":until.is_some() && !reached}),
                )?;
                writeln!(output)?;
                output.flush()?;
                return Ok(until.is_none() || reached);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}
