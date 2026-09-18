//! Scoped projections, artifact lineage and journal-based semantic waiting.
use super::{ArtifactKind, Predicate, Scope};
use crate::{
    LifecycleState, RunOutcome, WaitingOn, db::Database, executor::CancellationToken, orchestrator,
    state::State,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
pub(crate) fn snapshot(scope: &Scope, state: &State, id: &str) -> Result<Value> {
    scope.run(state, id)?;
    let db = Database::open_read_only(state.db_path())?;
    // Re-read cursor and projection in one SQLite snapshot.
    let transaction = db.connection().unchecked_transaction()?;
    let cursor: u64 = transaction.query_row(
        "SELECT COALESCE(MAX(sequence),0) FROM events WHERE run_id=?1",
        [id],
        |r| r.get(0),
    )?;
    let run = db
        .committed_run_projection(id)?
        .context("no committed projection")?;
    transaction.commit()?;
    Ok(
        json!({"run_id":id,"cursor":cursor,"state_revision":run.state_revision,"outcome":run.outcome,"phase3":run.phase3,"admission":run.admission}),
    )
}

pub(crate) fn artifact(
    scope: &Scope,
    state: &State,
    id: &str,
    attempt_id: &str,
    kind: ArtifactKind,
    offset: u64,
) -> Result<Value> {
    use std::io::{Read, Seek, SeekFrom};
    let run = scope.run(state, id)?;
    let attempt = run
        .attempts
        .iter()
        .find(|a| a.id == attempt_id)
        .context("unauthorized artifact lineage")?;
    let candidate = attempt
        .detail
        .result
        .as_ref()
        .context("not_ready: immutable attempt artifacts unavailable")?;
    let final_diff = matches!(kind, ArtifactKind::FinalDiff);
    let path = match kind {
        ArtifactKind::FinalDiff => {
            ensure!(
                run.phase3
                    .as_ref()
                    .and_then(|p| p.final_attempt_id.as_deref())
                    == Some(attempt_id),
                "unauthorized final artifact identity"
            );
            crate::planning::verify_delivery(&run)?;
            &run.candidates
                .first()
                .context("not_ready: no final delivery")?
                .diff_path
        }
        ArtifactKind::Diff => &candidate.diff_path,
        ArtifactKind::Stdout => &candidate.stdout_path,
        ArtifactKind::Stderr => &candidate.stderr_path,
    };
    let resolved = path.canonicalize()?;
    let expected = if final_diff {
        scope.state_root.join("runs").join(id).join("delivery")
    } else {
        scope
            .state_root
            .join("runs")
            .join(id)
            .join("attempts")
            .join(attempt_id)
    };
    ensure!(
        expected.canonicalize()? == expected && resolved.starts_with(&expected),
        "unauthorized artifact path"
    );
    #[cfg(unix)]
    let mut file = fs_options(&resolved, libc::O_NOFOLLOW)?;
    #[cfg(not(unix))]
    let mut file = std::fs::File::open(&resolved)?;
    ensure!(file.metadata()?.is_file(), "unauthorized artifact type");
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.take(16384).read_to_end(&mut bytes)?;
    Ok(
        json!({"run_id":id,"attempt_id":attempt_id,"kind":kind,"offset":offset,"next_offset":offset+bytes.len() as u64,"eof":bytes.len()<16384,"text":String::from_utf8_lossy(&bytes)}),
    )
}
#[cfg(unix)]
fn fs_options(path: &std::path::Path, flags: i32) -> Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    Ok(std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(flags)
        .open(path)?)
}

pub(crate) fn result(scope: &Scope, state: &State, id: &str) -> Result<Value> {
    let run = scope.run(state, id)?;
    ensure!(
        run.outcome.lifecycle == LifecycleState::Finished,
        "not_ready: execution has not finished"
    );
    let mut artifacts: Vec<Value> = run
        .attempts
        .iter()
        .filter(|a| a.detail.result.is_some())
        .flat_map(|a| {
            [
                ArtifactKind::Diff,
                ArtifactKind::Stdout,
                ArtifactKind::Stderr,
            ]
            .map(|kind| json!({"run_id":id,"attempt_id":a.id,"kind":kind}))
        })
        .collect();
    if let Some(p) = crate::planning::planning(&run)
        && p.final_candidate.is_some()
    {
        artifacts.push(json!({"run_id":id,"attempt_id":run.phase3.as_ref().unwrap().final_attempt_id,"kind":"final_diff"}));
    }
    let mut value = json!({"result_id":run.phase3.as_ref().and_then(|p|p.final_attempt_id.as_ref()),"delivery_revision":run.phase3.as_ref().and_then(|p|p.final_attempt_id.as_ref()),"result":orchestrator::run_result(&run),"candidates":run.candidates,"baseline_checks":run.baseline_checks,"artifacts":artifacts});
    remove_paths(&mut value);
    Ok(value)
}
fn remove_paths(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|k, _| {
                !k.ends_with("_path")
                    && !matches!(
                        k.as_str(),
                        "input_baseline" | "private_evidence" | "path" | "patch" | "manifest"
                    )
            });
            for v in map.values_mut() {
                remove_paths(v)
            }
        }
        Value::Array(a) => {
            for v in a {
                remove_paths(v)
            }
        }
        _ => (),
    }
}

pub(crate) struct Observation {
    pub run_id: String,
    pub after: u64,
    pub predicate: Option<Predicate>,
    pub timeout_ms: u64,
    pub follow: bool,
}

pub(crate) async fn observe(
    scope: Arc<Scope>,
    state: State,
    observation: Observation,
    request_id: String,
    mut publish: impl FnMut(Value) -> Result<()>,
    closed: CancellationToken,
) -> Result<Value> {
    let Observation {
        run_id: id,
        mut after,
        predicate,
        timeout_ms,
        follow,
    } = observation;
    ensure!(timeout_ms <= 60000, "timeout exceeds 60000 ms");
    scope.run(&state, &id)?;
    let db = Database::open_read_only(state.db_path())?;
    let (_, current) = db.event_page(&id, after, 0)?;
    current.context("no committed projection")?;
    let initial_wait = if after == 0 {
        WaitingOn::None
    } else {
        let event = db
            .events_after(&id, after - 1, 1)?
            .into_iter()
            .next()
            .context("invalid cursor")?;
        serde_json::from_value::<RunOutcome>(event.payload["outcome"].clone())?.waiting_on
    };
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        scope.run(&state, &id)?;
        let (events, current) = db.event_page(&id, after, 16)?;
        let current = current.context("missing projection")?;
        let empty = events.is_empty();
        for event in events {
            after = event.sequence;
            let outcome: RunOutcome = serde_json::from_value(event.payload["outcome"].clone())?;
            if predicate.is_none() {
                publish(
                    json!({"type":"event","protocol_version":1,"subscription_id":request_id,"event_id":format!("{}:{}",id,event.sequence),"event":event}),
                )?;
            }
            let reached = match predicate {
                Some(Predicate::AttentionRequired) => {
                    outcome.waiting_on == WaitingOn::Human
                        || outcome.lifecycle == LifecycleState::Finished
                }
                Some(Predicate::ExecutionFinished) => outcome.lifecycle == LifecycleState::Finished,
                Some(Predicate::WaitingReasonChanged) => outcome.waiting_on != initial_wait,
                Some(Predicate::StateChanged) => true,
                None => false,
            };
            if reached {
                return Ok(
                    json!({"run_id":id,"cursor":after,"reached":true,"timed_out":false,"outcome":outcome,"event":event}),
                );
            }
        }
        let reached = match predicate {
            Some(Predicate::AttentionRequired) => {
                current.outcome.waiting_on == WaitingOn::Human
                    || current.outcome.lifecycle == LifecycleState::Finished
            }
            Some(Predicate::ExecutionFinished) => {
                current.outcome.lifecycle == LifecycleState::Finished
            }
            Some(Predicate::WaitingReasonChanged) => current.outcome.waiting_on != initial_wait,
            _ => false,
        };
        if empty
            && (reached
                || (!follow && predicate.is_none())
                || tokio::time::Instant::now() >= deadline)
        {
            return Ok(
                json!({"run_id":id,"cursor":after,"reached":reached,"timed_out":!reached && (follow || predicate.is_some()),"outcome":current.outcome}),
            );
        }
        if empty {
            tokio::select! {_=closed.cancelled()=>anyhow::bail!("connection closed"),_=tokio::time::sleep(Duration::from_millis(25))=>{}}
        }
    }
}
