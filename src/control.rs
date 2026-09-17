//! Bounded JSONL framing and multiplexing over the shared foreground commands.
use crate::{
    LifecycleState,
    commands::{
        self, Operation, Session,
        inspection::{Observation, artifact, observe, result, snapshot},
    },
    executor::CancellationToken,
    orchestrator::{self, Presentation},
    state::State,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, io, sync::Mutex, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    sync::{mpsc, oneshot},
    task::JoinSet,
};

pub const FRAME_LIMIT: usize = 65536;
const OUTPUT_LIMIT: usize = 1024 * 1024;
const PENDING_LIMIT: usize = 16;
const OUTPUT_QUEUE: usize = 32;
const STALL_SECS: u64 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub protocol_version: u32,
    pub request_id: String,
    #[serde(flatten)]
    pub operation: Operation,
}

fn response(id: &str, result: Value) -> Value {
    json!({"type":"response","protocol_version":1,"request_id":id,"ok":true,"result":result})
}
fn error(id: Option<&str>, code: &str, message: impl ToString) -> Value {
    json!({"type":"response","protocol_version":1,"request_id":id,"ok":false,"error":{"code":code,"message":message.to_string()}})
}
fn core_error(id: &str, e: anyhow::Error) -> Value {
    let text = format!("{e:#}");
    let code = if text.contains("request_conflict") {
        "request_conflict"
    } else if text.contains("unauthorized") {
        "unauthorized"
    } else if text.contains("authorization_required") || text.contains("grant_expired") {
        "authorization_required"
    } else if text.contains("cursor") {
        "invalid_cursor"
    } else if text.contains("stale") || text.contains("wrong question") {
        "stale_revision"
    } else if text.contains("recovery_ineligible") || text.contains("owner_live_or_uncertain") {
        "recovery_ineligible"
    } else if text.contains("not_ready") {
        "not_ready"
    } else {
        "command_rejected"
    };
    error(Some(id), code, text)
}
#[derive(Clone)]
struct Output {
    tx: mpsc::Sender<Vec<u8>>,
    closed: CancellationToken,
}
impl Output {
    fn send(&self, value: Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(&value)?;
        if bytes.len() > OUTPUT_LIMIT {
            self.closed.cancel();
            anyhow::bail!("response_too_large: reconnect and use scoped artifacts");
        }
        bytes.push(b'\n');
        if self.tx.try_send(bytes).is_err() {
            self.closed.cancel();
            anyhow::bail!("output_overflow: reconnect and replay journal");
        }
        Ok(())
    }
}

async fn read_frame<R: AsyncRead + Unpin>(reader: &mut BufReader<R>) -> Result<Option<Vec<u8>>> {
    let mut frame = Vec::with_capacity(1024);
    loop {
        let buf = reader.fill_buf().await?;
        if buf.is_empty() {
            ensure!(frame.is_empty(), "incomplete_frame");
            return Ok(None);
        }
        let end = buf.iter().position(|b| *b == b'\n');
        let n = end.map_or(buf.len(), |n| n + 1);
        ensure!(frame.len() + n <= FRAME_LIMIT, "frame_too_large");
        frame.extend_from_slice(&buf[..n]);
        reader.consume(n);
        if end.is_some() {
            return Ok(Some(frame));
        }
    }
}

/// Generic duplex boundary also used by the real stdio entrypoint and tests.
async fn serve_inner<R, W>(state: State, session: Session, input: R, writer: W) -> Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let closed = CancellationToken::new();
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(OUTPUT_QUEUE);
    let output = Output {
        tx: out_tx,
        closed: closed.clone(),
    };
    let writer_closed = closed.clone();
    let writer_task = tokio::spawn(async move {
        let mut writer = writer;
        while let Some(bytes) = out_rx.recv().await {
            match tokio::time::timeout(Duration::from_secs(STALL_SECS), async {
                writer.write_all(&bytes).await?;
                writer.flush().await
            })
            .await
            {
                Ok(Ok(())) => (),
                _ => {
                    writer_closed.cancel();
                    break;
                }
            }
        }
    });
    let (in_tx, mut in_rx) = mpsc::channel(PENDING_LIMIT);
    let reader_task = tokio::spawn(async move {
        let mut reader = BufReader::with_capacity(8192, input);
        loop {
            let frame = read_frame(&mut reader).await;
            let done = !matches!(&frame, Ok(Some(_)));
            if in_tx.send(frame).await.is_err() || done {
                break;
            }
        }
    });
    let mut initialized = false;
    let mut pending: HashMap<String, String> = HashMap::new();
    let mut observers: JoinSet<(String, Result<Value>)> = JoinSet::new();
    let mut worker: Option<tokio::task::JoinHandle<Result<crate::RunRecord>>> = None;
    let mut committed: Option<oneshot::Receiver<Value>> = None;
    let mut worker_request: Option<String> = None;
    let mut active: Option<String> = None;
    let mut cancel = CancellationToken::new();
    let mut failure = None;
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    loop {
        tokio::select! {
            biased;
            _=closed.cancelled()=>{failure=Some("output closed or stalled");break},
            _=orchestrator::shutdown_signal()=>break,
            _=tick.tick()=>{
                if let Some(rx) = committed.as_mut()
                    && let Ok(value) = rx.try_recv()
                {
                    active = value["result"]["run_id"].as_str().map(str::to_owned);
                    if output.send(value).is_err() {
                        failure = Some("output overflow");
                        break;
                    }
                    committed = None;
                }
                if worker.as_ref().is_some_and(|w| w.is_finished()) {
                    let done = worker.take().unwrap().await;
                    // The receipt always wins over publication/execution errors after commit.
                    if let Some(mut rx) = committed.take() {
                        if let Ok(value) = rx.try_recv() {
                            active = value["result"]["run_id"].as_str().map(str::to_owned);
                            let _ = output.send(value);
                        } else if let Some(id) = worker_request.as_deref() {
                            let e = match &done {
                                Ok(Err(e)) => format!("{e:#}"),
                                Err(e) => e.to_string(),
                                _ => "command completed without a receipt".into(),
                            };
                            let _ = output.send(core_error(id, anyhow::anyhow!(e)));
                        }
                    }
                    if let Some(id) = worker_request.take() {
                        pending.remove(&id);
                    }
                    if let Some(id) = active.as_deref()
                        && let Ok(run) = session.scope.run(&state, id)
                    {
                        if cancel.is_cancelled() {
                            let _ = commands::close_question(&state, &session, &run);
                        }
                        if run.outcome.lifecycle == LifecycleState::Finished || cancel.is_cancelled() {
                            active = None;
                        }
                    }
                }

            },
            Some(done)=observers.join_next(),if !observers.is_empty()=>{if let Ok((id,result))=done {pending.remove(&id);let value=match result {Ok(v)=>response(&id,v),Err(e)=>core_error(&id,e)};let _=output.send(value);}},
            frame=in_rx.recv()=>{
                let bytes = match frame {
                    Some(Ok(Some(bytes))) => bytes,
                    Some(Err(e)) => {
                        let _ = output.send(error(None, "framing_error", e));
                        failure = Some("invalid framing");
                        break;
                    }
                    _ => break,
                };
                let raw: Value = match serde_json::from_slice(&bytes) {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = output.send(error(None, "invalid_json", e));
                        continue;
                    }
                };
                let id = raw
                    .get("request_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let request: Request = match serde_json::from_value(raw.clone()) {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = output.send(error(id.as_deref(), "invalid_request", e));
                        continue;
                    }
                };
                let known = serde_json::to_value(&request)?;
                if raw
                    .as_object()
                    .is_some_and(|map| map.keys().any(|k| known.get(k).is_none()))
                {
                    let _ = output.send(error(
                        id.as_deref(),
                        "invalid_request",
                        "unknown request field",
                    ));
                    continue;
                }
                let id = request.request_id.clone();
                if request.protocol_version != 1 {
                    let _ = output.send(error(
                        Some(&id),
                        "unsupported_version",
                        "supported versions: [1]",
                    ));
                    continue;
                }
                if id.is_empty() || id.len() > 128 {
                    let _ = output.send(error(
                        None,
                        "invalid_request",
                        "request_id must be 1..128 bytes",
                    ));
                    continue;
                }
                if matches!(request.operation, Operation::Initialize) {
                    initialized = true;
                    let _=output.send(response(&id,json!({"protocol_version":1,"supported_versions":[1],"operations":commands::OPERATIONS,"session_id":session.id,"principal":session.scope.principal,"ownership_mode":"foreground","read_only":session.read_only,"scope":*session.scope,"limits":{"request_bytes":FRAME_LIMIT,"response_bytes":OUTPUT_LIMIT,"pending_requests":PENDING_LIMIT,"observers":8,"output_messages":OUTPUT_QUEUE,"stalled_output_seconds":STALL_SECS,"wait_timeout_ms":60000,"artifact_preview_bytes":16384}})));
                    continue;
                }
                if !initialized {
                    let _ = output.send(error(
                        Some(&id),
                        "initialize_required",
                        "negotiate protocol before requests",
                    ));
                    continue;
                }
                let payload = serde_json::to_value(&request.operation)?;
                if request.operation.mutation() && !session.read_only {
                    match session.scope.receipt(&state, &id, &payload) {
                        Ok(Some(reply)) => {
                            let _ = output.send(reply);
                            continue;
                        }
                        Err(e) => {
                            let _ = output.send(core_error(&id, e));
                            continue;
                        }
                        _ => (),
                    }
                }
                let payload_digest = commands::digest(&payload)?;
                if let Some(original) = pending.get(&id) {
                    let (code, message) = if original == &payload_digest {
                        ("request_pending", "request is pending; retry after its response")
                    } else {
                        ("request_conflict", "pending request ID has different content")
                    };
                    let _ = output.send(error(Some(&id), code, message));
                    continue;
                }
                if pending.len() >= PENDING_LIMIT {
                    let _ = output.send(error(
                        Some(&id),
                        "too_many_requests",
                        "pending request bound reached",
                    ));
                    continue;
                }
                if request.operation.mutation() {
                    if session.read_only {
                        let _ = output.send(error(Some(&id), "unauthorized", "read-only session"));
                        continue;
                    }
                    if matches!(request.operation, Operation::Submit { .. })
                        && (worker.is_some() || active.is_some())
                    {
                        let _ = output.send(error(Some(&id), "busy", "one active goal per session"));
                        continue;
                    }
                    let (tx, rx) = oneshot::channel();
                    let context = commands::CommandContext {
                        scope: session.scope.clone(),
                        session_id: session.id.clone(),
                        request_id: id.clone(),
                        payload_digest: commands::digest(&payload)?,
                        operation: request.operation.name().into(),
                        committed: Mutex::new(Some(tx)),
                    };
                    if let Operation::Cancel { run_id, revision } = request.operation {
                        if active.as_deref() != Some(&run_id) {
                            let _ = output.send(error(
                                Some(&id),
                                "unauthorized",
                                "cancel requires current foreground ownership",
                            ));
                            continue;
                        }
                        match commands::scoped(context, async {
                            commands::cancel(&state, &session.scope, &run_id, revision)
                        })
                        .await
                        {
                            Ok(run) => {
                                cancel.cancel();
                                if worker.is_none() {
                                    commands::close_question(&state, &session, &run)?;
                                    active = None;
                                }
                                if let Ok(value) = rx.await {
                                    let _ = output.send(value);
                                }
                            }
                            Err(e) => {
                                let _ = output.send(core_error(&id, e));
                            }
                        }
                        continue;
                    }
                    if worker.is_some() {
                        let _ = output.send(error(
                            Some(&id),
                            "busy",
                            "foreground command is still running",
                        ));
                        continue;
                    }
                    if let Operation::Answer { run_id, .. } = &request.operation
                        && active.as_deref() != Some(run_id)
                    {
                        let _ = output.send(error(
                            Some(&id),
                            "recovery_required",
                            "old checkpoints require explicit recovery ownership",
                        ));
                        continue;
                    }
                    if matches!(request.operation, Operation::Recover { .. }) && active.is_some() {
                        let _ = output.send(error(Some(&id), "busy", "one active goal per session"));
                        continue;
                    }
                    let state = state.clone();
                    let scope = session.scope.clone();
                    cancel = CancellationToken::new();
                    let cancellation = cancel.clone();
                    let (updates, _) = tokio::sync::watch::channel(None);
                    worker = Some(tokio::task::spawn_local(async move {
                        commands::scoped(
                            context,
                            orchestrator::present(
                                Presentation {
                                    updates,
                                    cancellation,
                                },
                                async { commands::execute(&state, &scope, request.operation).await },
                            ),
                        )
                        .await
                    }));
                    pending.insert(id.clone(), payload_digest);
                    worker_request = Some(id);
                    committed = Some(rx);
                    continue;
                }
                let scope = session.scope.clone();
                let state = state.clone();
                let output = output.clone();
                let observer_id = id.clone();
                if observers.len() >= 8 {
                    let _ = output.send(error(
                        Some(&id),
                        "too_many_requests",
                        "observer bound reached",
                    ));
                    continue;
                }
                pending.insert(id, payload_digest);
                observers.spawn_local(async move {let result=async {match request.operation {
                            Operation::Status{run_id}=>snapshot(&scope,&state,&run_id),
                            Operation::Result{run_id}=>result(&scope,&state,&run_id),
                            Operation::Capacity{run_id}=>{let run=scope.run(&state,&run_id)?;Ok(json!({"run_id":run_id,"observation":run.capacity,"unknown_is_unlimited":false}))},
                            Operation::Artifact{run_id,attempt_id,kind,offset}=>artifact(&scope,&state,&run_id,&attempt_id,kind,offset),
                            Operation::Events{run_id,after}=>observe(scope,state,Observation{run_id,after,predicate:None,timeout_ms:0,follow:false},observer_id.clone(),|value|output.send(value),output.closed.clone()).await,
                            Operation::Subscribe{run_id,after,timeout_ms}=>observe(scope,state,Observation{run_id,after,predicate:None,timeout_ms,follow:true},observer_id.clone(),|value|output.send(value),output.closed.clone()).await,
                            Operation::Await{run_id,after,predicate,timeout_ms}=>observe(scope,state,Observation{run_id,after,predicate:Some(predicate),timeout_ms,follow:true},observer_id.clone(),|value|output.send(value),output.closed.clone()).await,
                            _=>unreachable!()
                        }}.await;(observer_id,result)});

            }
        }
    }
    reader_task.abort();
    observers.abort_all();
    cancel.cancel();
    if let Some(work) = worker {
        match work.await {
            Ok(Ok(run)) => {
                if let Err(e) = commands::close_question(&state, &session, &run) {
                    eprintln!("control cleanup: {e:#}")
                }
            }
            Ok(Err(e)) => eprintln!("control supervisor: {e:#}"),
            Err(e) => eprintln!("control supervisor: {e}"),
        }
    }
    if let Some(id) = active
        && let Ok(run) = state.load_run(&id)
    {
        commands::close_question(&state, &session, &run)?;
    }
    drop(output);
    let _ = writer_task.await;
    if let Some(reason) = failure {
        anyhow::bail!(reason)
    }
    Ok(())
}

#[cfg(unix)]
mod pipes {
    use super::*;
    use std::{
        fs::File,
        os::fd::{AsRawFd, FromRawFd},
        pin::Pin,
        task::{Context as TaskContext, Poll},
    };
    use tokio::io::{ReadBuf, unix::AsyncFd};
    pub struct Pipe(AsyncFd<File>);
    impl Pipe {
        pub fn new(fd: i32) -> io::Result<Self> {
            let copied = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
            if copied < 0 {
                return Err(io::Error::last_os_error());
            }
            let file = unsafe { File::from_raw_fd(copied) };
            let flags = unsafe { libc::fcntl(copied, libc::F_GETFL) };
            if flags < 0
                || unsafe { libc::fcntl(copied, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(Self(AsyncFd::new(file)?))
        }
    }
    impl AsyncRead for Pipe {
        fn poll_read(
            self: Pin<&mut Self>,
            cx: &mut TaskContext<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            loop {
                let mut ready = std::task::ready!(self.0.poll_read_ready(cx))?;
                let slice = buf.initialize_unfilled();
                match ready.try_io(|fd| {
                    let n = unsafe {
                        libc::read(fd.as_raw_fd(), slice.as_mut_ptr().cast(), slice.len())
                    };
                    if n < 0 {
                        Err(io::Error::last_os_error())
                    } else {
                        Ok(n as usize)
                    }
                }) {
                    Ok(Ok(n)) => {
                        buf.advance(n);
                        return Poll::Ready(Ok(()));
                    }
                    Ok(Err(e)) => return Poll::Ready(Err(e)),
                    Err(_) => continue,
                }
            }
        }
    }
    impl AsyncWrite for Pipe {
        fn poll_write(
            self: Pin<&mut Self>,
            cx: &mut TaskContext<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            loop {
                let mut ready = std::task::ready!(self.0.poll_write_ready(cx))?;
                match ready.try_io(|fd| {
                    let n = unsafe { libc::write(fd.as_raw_fd(), buf.as_ptr().cast(), buf.len()) };
                    if n < 0 {
                        Err(io::Error::last_os_error())
                    } else {
                        Ok(n as usize)
                    }
                }) {
                    Ok(v) => return Poll::Ready(v),
                    Err(_) => continue,
                }
            }
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
}
#[cfg(unix)]
pub async fn stdio(state: State, grant_fd: i32, read_only: bool) -> Result<()> {
    let session = Session::from_fd(&state, grant_fd, read_only)?;
    tokio::task::LocalSet::new()
        .run_until(serve_inner(
            state,
            session,
            pipes::Pipe::new(0)?,
            pipes::Pipe::new(1)?,
        ))
        .await
}
#[cfg(not(unix))]
pub async fn stdio(_state: State, _grant_fd: i32, _read_only: bool) -> Result<()> {
    anyhow::bail!("control private inherited handles currently require Unix")
}
