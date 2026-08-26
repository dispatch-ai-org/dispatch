use std::{
    collections::BTreeMap,
    env,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::{fs, io::AsyncReadExt, process::Command, sync::Notify};

use crate::{
    config::ExecutionConfig,
    models::{CheckPhase, CheckResult, CheckStatus},
};

#[cfg(not(test))]
const MAX_CAPTURE_BYTES: usize = 16 * 1024 * 1024;
#[cfg(test)]
const MAX_CAPTURE_BYTES: usize = 64 * 1024;
const REDACTION_MARKER: &[u8] = b"[REDACTED]";
#[cfg(not(test))]
const OUTPUT_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const OUTPUT_DRAIN_TIMEOUT: Duration = Duration::from_millis(150);
#[cfg(unix)]
const TRUSTED_HOST_DIRECTORIES: &[&str] = &[
    "/usr/bin",
    "/bin",
    "/usr/local/bin",
    "/opt/homebrew/bin",
    "/Applications/Docker.app/Contents/Resources/bin",
];

/// An argv-style command. Arguments are never reparsed by a shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    /// Adapter-owned environment additions. The executor still filters these
    /// against `execution.forwarded_env` before passing them to a child.
    pub env: BTreeMap<String, String>,
}

impl CommandSpec {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: BTreeMap::new(),
        }
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(name.into(), value.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionRequest {
    pub command: CommandSpec,
    /// The only candidate tree made available to the command.
    pub workspace: PathBuf,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
    /// Overrides the configured timeout when set.
    pub timeout: Option<Duration>,
}

impl ExecutionRequest {
    pub fn new(
        command: CommandSpec,
        workspace: impl Into<PathBuf>,
        stdout_path: impl Into<PathBuf>,
        stderr_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            command,
            workspace: workspace.into(),
            stdout_path: stdout_path.into(),
            stderr_path: stderr_path.into(),
            timeout: None,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
    SpawnFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub status: ExecutionStatus,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
    pub error: Option<String>,
    #[serde(skip)]
    raw_stdout: Vec<u8>,
    #[serde(skip)]
    raw_stderr: Vec<u8>,
}

impl ExecutionResult {
    pub fn success(&self) -> bool {
        self.status == ExecutionStatus::Succeeded
    }

    pub(crate) fn raw_stdout_lossy(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.raw_stdout)
    }

    pub(crate) fn raw_stderr_lossy(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.raw_stderr)
    }

    pub(crate) fn clear_raw_capture(&mut self) {
        self.raw_stdout.clear();
        self.raw_stderr.clear();
    }
}

#[derive(Debug, Default)]
struct CancellationState {
    cancelled: AtomicBool,
    notify: Notify,
}

/// A small cancellation primitive that avoids coupling the execution API to a
/// particular task owner. Clones observe the same cancellation event.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    state: Arc<CancellationState>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        if !self.state.cancelled.swap(true, Ordering::SeqCst) {
            self.state.notify.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::SeqCst)
    }

    pub async fn cancelled(&self) {
        loop {
            let notified = self.state.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

#[derive(Debug, Clone)]
pub struct Executor {
    config: ExecutionConfig,
}

impl Executor {
    pub fn new(config: ExecutionConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &ExecutionConfig {
        &self.config
    }

    pub async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionResult> {
        self.execute_with_cancel(request, CancellationToken::new())
            .await
    }

    pub async fn execute_with_cancel(
        &self,
        request: ExecutionRequest,
        cancellation: CancellationToken,
    ) -> Result<ExecutionResult> {
        validate_workspace(&request.workspace)?;
        prepare_log_path(&request.stdout_path).await?;
        prepare_log_path(&request.stderr_path).await?;
        let redactions = redaction_values(&self.config.forwarded_env, &request.command.env);

        let started = Instant::now();
        if cancellation.is_cancelled() {
            return self
                .finish_without_child(
                    request,
                    ExecutionStatus::Cancelled,
                    started,
                    "execution cancelled before spawn".into(),
                )
                .await;
        }

        let container_name = (self.config.backend == "docker")
            .then(|| format!("dispatch-{}", ulid::Ulid::new().to_string().to_lowercase()));
        let mut docker_cleanup = container_name.as_deref().map(DockerCleanupGuard::new);
        let mut command = self.process_command(&request, container_name.as_deref())?;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                return self
                    .finish_without_child(
                        request,
                        ExecutionStatus::SpawnFailed,
                        started,
                        format!("failed to spawn process: {error}"),
                    )
                    .await;
            }
        };

        let mut process_group = ProcessGroupGuard::new(child.id());
        let stdout = child
            .stdout
            .take()
            .context("child stdout was not captured")?;
        let stderr = child
            .stderr
            .take()
            .context("child stderr was not captured")?;
        let stdout_task = tokio::spawn(read_capped(stdout));
        let stderr_task = tokio::spawn(read_capped(stderr));
        let timeout = request
            .timeout
            .unwrap_or_else(|| Duration::from_secs(self.config.timeout_secs));

        enum Completion {
            Exited(std::io::Result<std::process::ExitStatus>),
            TimedOut,
            Cancelled,
        }

        let completion = tokio::select! {
            status = child.wait() => Completion::Exited(status),
            _ = tokio::time::sleep(timeout) => Completion::TimedOut,
            _ = cancellation.cancelled() => Completion::Cancelled,
        };

        let (status, exit_code, error) = match completion {
            Completion::Exited(Ok(exit)) if exit.success() => {
                (ExecutionStatus::Succeeded, exit.code(), None)
            }
            Completion::Exited(Ok(exit)) => (
                ExecutionStatus::Failed,
                exit.code(),
                Some(format!("process exited unsuccessfully: {exit}")),
            ),
            Completion::Exited(Err(error)) => (
                ExecutionStatus::Failed,
                None,
                Some(format!("failed while waiting for process: {error}")),
            ),
            Completion::TimedOut => {
                terminate_child(&mut child, &mut process_group).await;
                (
                    ExecutionStatus::TimedOut,
                    None,
                    Some(format!(
                        "process exceeded timeout of {} ms",
                        timeout.as_millis()
                    )),
                )
            }
            Completion::Cancelled => {
                terminate_child(&mut child, &mut process_group).await;
                (
                    ExecutionStatus::Cancelled,
                    None,
                    Some("execution cancelled".into()),
                )
            }
        };

        // Kill background descendants that outlived a normally exiting parent,
        // and make sure they cannot keep captured pipe descriptors open.
        process_group.kill();
        let cleanup_confirmed = if status != ExecutionStatus::Succeeded {
            match &container_name {
                Some(name) => cleanup_docker_container(name).await,
                None => true,
            }
        } else {
            true
        };
        let (mut stdout, mut stderr) = join_readers(stdout_task, stderr_task).await?;
        if matches!(
            status,
            ExecutionStatus::TimedOut | ExecutionStatus::Cancelled
        ) && stderr.is_empty()
            && let Some(message) = &error
        {
            stderr.extend_from_slice(message.as_bytes());
            stderr.push(b'\n');
        }
        let raw_stdout = stdout.clone();
        let raw_stderr = stderr.clone();
        redact_bytes(&mut stdout, &redactions);
        redact_bytes(&mut stderr, &redactions);

        write_logs(&request.stdout_path, &stdout, &request.stderr_path, &stderr).await?;
        if cleanup_confirmed && let Some(guard) = &mut docker_cleanup {
            guard.disarm();
        }

        Ok(ExecutionResult {
            status,
            exit_code,
            duration_ms: duration_ms(started.elapsed()),
            timed_out: status == ExecutionStatus::TimedOut,
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
            stdout_path: request.stdout_path,
            stderr_path: request.stderr_path,
            error,
            raw_stdout,
            raw_stderr,
        })
    }

    /// Returns the exact host command used for a request. This is public both
    /// for diagnostics and so callers can display an auditable execution plan.
    pub fn resolved_command(&self, request: &ExecutionRequest) -> Result<CommandSpec> {
        self.resolved_command_with_container(request, None)
    }

    fn resolved_command_with_container(
        &self,
        request: &ExecutionRequest,
        container_name: Option<&str>,
    ) -> Result<CommandSpec> {
        match self.config.backend.as_str() {
            "local" => {
                let mut command = request.command.clone();
                command
                    .env
                    .retain(|name, _| self.config.forwarded_env.contains(name));
                for name in &self.config.forwarded_env {
                    if !command.env.contains_key(name)
                        && let Ok(value) = env::var(name)
                    {
                        command.env.insert(name.clone(), value);
                    }
                }
                Ok(command)
            }
            "docker" => self.docker_command(request, container_name),
            backend => anyhow::bail!("unsupported execution backend {backend:?}"),
        }
    }

    fn process_command(
        &self,
        request: &ExecutionRequest,
        container_name: Option<&str>,
    ) -> Result<Command> {
        let spec = self.resolved_command_with_container(request, container_name)?;
        let mut command = Command::new(&spec.program);
        command.args(&spec.args);
        command.current_dir(&request.workspace);
        command.env_clear();

        // These values configure the host process. For the Docker backend they
        // help the Docker CLI itself; container environment is still supplied
        // exclusively through explicit `--env` arguments below.
        for (name, value) in safe_local_environment() {
            command.env(name, value);
        }
        if self.config.backend == "docker" {
            command.env("PATH", trusted_host_path());
            for (name, value) in docker_host_environment() {
                command.env(name, value);
            }
        }
        for (name, value) in &spec.env {
            command.env(name, value);
        }

        Ok(command)
    }

    fn docker_command(
        &self,
        request: &ExecutionRequest,
        container_name: Option<&str>,
    ) -> Result<CommandSpec> {
        anyhow::ensure!(
            !self.config.docker_image.trim().is_empty(),
            "execution.docker_image must not be empty"
        );
        anyhow::ensure!(
            !self.config.docker_image.starts_with('-'),
            "execution.docker_image must not start with '-'"
        );

        let workspace = request.workspace.canonicalize().with_context(|| {
            format!(
                "failed to resolve workspace {}",
                request.workspace.display()
            )
        })?;
        let workspace_text = workspace.to_str().with_context(|| {
            format!(
                "Docker backend does not support a non-UTF-8 workspace path: {}",
                workspace.display()
            )
        })?;
        anyhow::ensure!(
            !workspace_text.contains(','),
            "Docker backend does not support a workspace path containing a comma: {}",
            workspace.display()
        );
        let mount = format!("type=bind,source={workspace_text},target=/workspace");
        let mut args = vec!["run".into(), "--rm".into()];
        if let Some(name) = container_name {
            args.extend(["--name".into(), name.into()]);
        }
        args.extend([
            "--cpus".into(),
            self.config.cpus.to_string(),
            "--memory".into(),
            self.config.memory.clone(),
            "--pids-limit".into(),
            "512".into(),
        ]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let metadata = std::fs::metadata(&workspace).with_context(|| {
                format!("failed to inspect workspace owner {}", workspace.display())
            })?;
            args.extend([
                "--user".into(),
                format!("{}:{}", metadata.uid(), metadata.gid()),
            ]);
        }
        args.extend([
            "--mount".into(),
            mount,
            "--workdir".into(),
            "/workspace".into(),
        ]);

        let mut forwarded = BTreeMap::new();
        for name in &self.config.forwarded_env {
            if forwarded.contains_key(name) {
                continue;
            }
            if let Some(value) = explicitly_forwarded_value(name, &request.command.env) {
                args.push("--env".into());
                args.push(name.clone());
                forwarded.insert(name.clone(), value);
            }
        }

        args.push(self.config.docker_image.clone());
        args.push(request.command.program.clone());
        args.extend(request.command.args.iter().cloned());
        let docker = trusted_host_executable("docker")?;
        let mut command = CommandSpec::new(docker.to_string_lossy()).args(args);
        command.env = forwarded;
        Ok(command)
    }

    async fn finish_without_child(
        &self,
        request: ExecutionRequest,
        status: ExecutionStatus,
        started: Instant,
        message: String,
    ) -> Result<ExecutionResult> {
        write_logs(
            &request.stdout_path,
            &[],
            &request.stderr_path,
            format!("{message}\n").as_bytes(),
        )
        .await?;
        Ok(ExecutionResult {
            status,
            exit_code: None,
            duration_ms: duration_ms(started.elapsed()),
            timed_out: status == ExecutionStatus::TimedOut,
            stdout: String::new(),
            stderr: format!("{message}\n"),
            stdout_path: request.stdout_path,
            stderr_path: request.stderr_path,
            error: Some(message),
            raw_stdout: Vec::new(),
            raw_stderr: Vec::new(),
        })
    }
}

async fn read_capped<R: tokio::io::AsyncRead + Unpin>(mut reader: R) -> std::io::Result<Vec<u8>> {
    let head_limit = MAX_CAPTURE_BYTES / 2;
    let tail_limit = MAX_CAPTURE_BYTES - head_limit;
    let mut head = Vec::with_capacity(head_limit.min(64 * 1024));
    let mut tail = vec![0_u8; tail_limit];
    let mut tail_seen = 0_u64;
    let mut buffer = [0_u8; 8192];
    let mut total = 0_u64;
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        let head_remaining = head_limit.saturating_sub(head.len());
        let head_bytes = read.min(head_remaining);
        head.extend_from_slice(&buffer[..head_bytes]);
        for byte in &buffer[head_bytes..read] {
            tail[(tail_seen % tail_limit as u64) as usize] = *byte;
            tail_seen = tail_seen.saturating_add(1);
        }
    }
    if total <= MAX_CAPTURE_BYTES as u64 {
        let tail_len = usize::try_from(tail_seen).unwrap_or(tail_limit);
        head.extend_from_slice(&tail[..tail_len]);
    } else {
        head.extend_from_slice(
            format!(
                "\n[dispatch: output truncated after {MAX_CAPTURE_BYTES} bytes; {} bytes omitted]\n",
                total - MAX_CAPTURE_BYTES as u64
            )
            .as_bytes(),
        );
        let start = (tail_seen % tail_limit as u64) as usize;
        head.extend_from_slice(&tail[start..]);
        head.extend_from_slice(&tail[..start]);
    }
    Ok(head)
}

fn redaction_values(allowlist: &[String], command_env: &BTreeMap<String, String>) -> Vec<Vec<u8>> {
    let mut values = allowlist
        .iter()
        .filter_map(|name| explicitly_forwarded_value(name, command_env))
        .filter(|value| !value.is_empty())
        .map(String::into_bytes)
        .collect::<Vec<_>>();
    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    values.dedup();
    values
}

fn redact_bytes(bytes: &mut Vec<u8>, secrets: &[Vec<u8>]) {
    for secret in secrets {
        if secret.is_empty() || bytes.len() < secret.len() {
            continue;
        }
        let mut cursor = 0;
        let mut redacted = Vec::with_capacity(bytes.len());
        while cursor + secret.len() <= bytes.len() {
            let Some(relative) = bytes[cursor..]
                .windows(secret.len())
                .position(|window| window == secret.as_slice())
            else {
                break;
            };
            let start = cursor + relative;
            redacted.extend_from_slice(&bytes[cursor..start]);
            redacted.extend_from_slice(REDACTION_MARKER);
            cursor = start + secret.len();
        }
        if cursor > 0 {
            redacted.extend_from_slice(&bytes[cursor..]);
            *bytes = redacted;
        }
    }
}

async fn join_reader(
    task: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
    stream: &str,
) -> Result<Vec<u8>> {
    task.await
        .with_context(|| format!("{stream} capture task failed"))?
        .with_context(|| format!("failed to read child {stream}"))
}

async fn join_readers(
    stdout_task: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
    stderr_task: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let stdout_abort = stdout_task.abort_handle();
    let stderr_abort = stderr_task.abort_handle();
    let joined = tokio::time::timeout(OUTPUT_DRAIN_TIMEOUT, async move {
        let stdout = join_reader(stdout_task, "stdout").await?;
        let stderr = join_reader(stderr_task, "stderr").await?;
        Ok::<_, anyhow::Error>((stdout, stderr))
    })
    .await;
    match joined {
        Ok(result) => result,
        Err(_) => {
            stdout_abort.abort();
            stderr_abort.abort();
            anyhow::bail!(
                "child output pipes remained open after process exit for more than {} ms",
                OUTPUT_DRAIN_TIMEOUT.as_millis()
            )
        }
    }
}

struct ProcessGroupGuard {
    #[cfg(unix)]
    pgid: Option<i32>,
}

impl ProcessGroupGuard {
    fn new(child_id: Option<u32>) -> Self {
        Self {
            #[cfg(unix)]
            pgid: child_id.and_then(|id| i32::try_from(id).ok()),
        }
    }

    fn kill(&mut self) {
        #[cfg(unix)]
        if let Some(pgid) = self.pgid.take() {
            // SAFETY: `pgid` came directly from the successfully spawned child,
            // which was placed in a new process group before exec. A negative
            // PID asks kill(2) to signal that group only.
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
        }
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

async fn terminate_child(child: &mut tokio::process::Child, group: &mut ProcessGroupGuard) {
    group.kill();
    // `kill` waits for the direct process on supported Tokio platforms. If it
    // races with group cleanup or natural exit there is nothing left to do.
    let _ = child.kill().await;
    let _ = child.wait().await;
}

async fn cleanup_docker_container(name: &str) -> bool {
    let Ok(docker) = trusted_host_executable("docker") else {
        return false;
    };
    let mut command = Command::new(docker);
    command
        .args(["rm", "-f", "--", name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env_clear()
        .kill_on_drop(true);
    for (key, value) in safe_local_environment()
        .into_iter()
        .chain(docker_host_environment())
    {
        command.env(key, value);
    }
    command.env("PATH", trusted_host_path());
    matches!(
        tokio::time::timeout(Duration::from_secs(10), command.status()).await,
        Ok(Ok(status)) if status.success()
    )
}

struct DockerCleanupGuard {
    name: Option<String>,
}

impl DockerCleanupGuard {
    fn new(name: &str) -> Self {
        Self {
            name: Some(name.to_owned()),
        }
    }

    fn disarm(&mut self) {
        self.name = None;
    }
}

impl Drop for DockerCleanupGuard {
    fn drop(&mut self) {
        let Some(name) = self.name.take() else {
            return;
        };
        let Ok(docker) = trusted_host_executable("docker") else {
            return;
        };
        let mut command = std::process::Command::new(docker);
        command
            .args(["rm", "-f", "--", &name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env_clear();
        for (key, value) in safe_local_environment()
            .into_iter()
            .chain(docker_host_environment())
        {
            command.env(key, value);
        }
        command.env("PATH", trusted_host_path());
        // Do not wait in Drop. The detached Docker CLI owns the cleanup even
        // if orchestration is unwinding because persistence failed.
        let _ = command.spawn();
    }
}

/// Resolve Dispatch-owned host tools without consulting relative or
/// project-controlled PATH entries. Local real-agent executables are not
/// routed through this helper because those already require an explicit
/// unsafe opt-in.
pub(crate) fn trusted_host_executable(program: &str) -> Result<PathBuf> {
    anyhow::ensure!(
        !program.is_empty()
            && !program.contains('/')
            && !program.contains('\\')
            && !program.starts_with('-'),
        "invalid trusted host tool name {program:?}"
    );

    #[cfg(unix)]
    {
        for directory in TRUSTED_HOST_DIRECTORIES {
            let candidate = Path::new(directory).join(program);
            if candidate.is_file() {
                return candidate.canonicalize().with_context(|| {
                    format!(
                        "failed to resolve trusted host tool {}",
                        candidate.display()
                    )
                });
            }
        }
        anyhow::bail!("trusted host tool {program:?} was not found in system locations");
    }

    #[cfg(not(unix))]
    {
        let path = which::which(program)
            .with_context(|| format!("trusted host tool {program:?} was not found"))?;
        anyhow::ensure!(
            path.is_absolute(),
            "resolved host tool path is not absolute"
        );
        Ok(path)
    }
}

pub(crate) fn trusted_host_path() -> String {
    #[cfg(unix)]
    {
        TRUSTED_HOST_DIRECTORIES.join(":")
    }
    #[cfg(not(unix))]
    {
        env::var("PATH").unwrap_or_default()
    }
}

fn validate_workspace(workspace: &Path) -> Result<()> {
    anyhow::ensure!(
        workspace.is_dir(),
        "workspace is not a directory: {}",
        workspace.display()
    );
    Ok(())
}

async fn prepare_log_path(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create log directory {}", parent.display()))?;
    }
    Ok(())
}

async fn write_logs(
    stdout_path: &Path,
    stdout: &[u8],
    stderr_path: &Path,
    stderr: &[u8],
) -> Result<()> {
    fs::write(stdout_path, stdout)
        .await
        .with_context(|| format!("failed to write {}", stdout_path.display()))?;
    fs::write(stderr_path, stderr)
        .await
        .with_context(|| format!("failed to write {}", stderr_path.display()))?;
    Ok(())
}

fn explicitly_forwarded_value(
    name: &str,
    command_env: &BTreeMap<String, String>,
) -> Option<String> {
    command_env
        .get(name)
        .cloned()
        .or_else(|| env::var(name).ok())
}

fn safe_local_environment() -> BTreeMap<String, String> {
    const SAFE_NAMES: [&str; 4] = ["PATH", "HOME", "TMPDIR", "LANG"];
    let mut values = BTreeMap::new();
    for name in SAFE_NAMES {
        if let Ok(value) = env::var(name) {
            values.insert(name.to_owned(), value);
        }
    }
    values
}

fn docker_host_environment() -> BTreeMap<String, String> {
    const NAMES: [&str; 6] = [
        "DOCKER_HOST",
        "DOCKER_CONTEXT",
        "DOCKER_CONFIG",
        "DOCKER_TLS_VERIFY",
        "DOCKER_CERT_PATH",
        "XDG_RUNTIME_DIR",
    ];
    let mut values = BTreeMap::new();
    for name in NAMES {
        if let Ok(value) = env::var(name) {
            values.insert(name.to_owned(), value);
        }
    }
    values
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

/// Runs configured repository checks sequentially and preserves an independent
/// result and pair of log files for every command, even after failures.
pub async fn run_checks(
    workspace: &Path,
    commands: &[String],
    phase: CheckPhase,
    output_dir: &Path,
    timeout_secs: u64,
) -> Vec<CheckResult> {
    let config = ExecutionConfig {
        backend: "local".into(),
        timeout_secs,
        ..ExecutionConfig::default()
    };
    run_checks_with_config(workspace, commands, phase, output_dir, config).await
}

/// Runs repository checks with the same local or Docker boundary selected for
/// harness execution. Agent credentials are deliberately not forwarded into
/// project-controlled check commands.
pub async fn run_checks_with_config(
    workspace: &Path,
    commands: &[String],
    phase: CheckPhase,
    output_dir: &Path,
    config: ExecutionConfig,
) -> Vec<CheckResult> {
    run_checks_with_config_and_cancel(
        workspace,
        commands,
        phase,
        output_dir,
        config,
        CancellationToken::new(),
    )
    .await
}

pub async fn run_checks_with_config_and_cancel(
    workspace: &Path,
    commands: &[String],
    phase: CheckPhase,
    output_dir: &Path,
    mut config: ExecutionConfig,
    cancellation: CancellationToken,
) -> Vec<CheckResult> {
    config.forwarded_env.clear();
    let executor = Executor::new(config);
    let mut results = Vec::with_capacity(commands.len());

    for (index, configured_command) in commands.iter().enumerate() {
        let ordinal = index + 1;
        let stem = format!("{}-{ordinal:02}", check_phase_name(&phase));
        let stdout_path = output_dir.join(format!("{stem}.stdout.log"));
        let stderr_path = output_dir.join(format!("{stem}.stderr.log"));
        // A configured check is intentionally a shell command. Passing it as a
        // single argv element avoids a second layer of interpolation here.
        let command =
            CommandSpec::new("/bin/sh").args(["-lc".to_owned(), configured_command.clone()]);
        let request =
            ExecutionRequest::new(command, workspace, stdout_path.clone(), stderr_path.clone());

        let result = match executor
            .execute_with_cancel(request, cancellation.clone())
            .await
        {
            Ok(execution) => CheckResult {
                name: format!("{} {ordinal}", check_phase_name(&phase)),
                phase: phase.clone(),
                command: configured_command.clone(),
                status: match execution.status {
                    ExecutionStatus::Succeeded => CheckStatus::Passed,
                    ExecutionStatus::TimedOut => CheckStatus::TimedOut,
                    ExecutionStatus::Cancelled => CheckStatus::NotRun,
                    ExecutionStatus::Failed | ExecutionStatus::SpawnFailed => CheckStatus::Failed,
                },
                exit_code: execution.exit_code,
                duration_ms: execution.duration_ms,
                stdout_path,
                stderr_path,
            },
            Err(error) => {
                let _ = fs::create_dir_all(output_dir).await;
                let _ = fs::write(&stdout_path, []).await;
                let _ = fs::write(&stderr_path, format!("{error:#}\n")).await;
                CheckResult {
                    name: format!("{} {ordinal}", check_phase_name(&phase)),
                    phase: phase.clone(),
                    command: configured_command.clone(),
                    status: CheckStatus::NotRun,
                    exit_code: None,
                    duration_ms: 0,
                    stdout_path,
                    stderr_path,
                }
            }
        };
        results.push(result);
    }

    results
}

fn check_phase_name(phase: &CheckPhase) -> &'static str {
    match phase {
        CheckPhase::Baseline => "baseline",
        CheckPhase::Verify => "verify",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn captures_success_and_failure_without_merging_streams() {
        let temp = tempfile::tempdir().unwrap();
        let success = ExecutionRequest::new(
            CommandSpec::new("/bin/sh").args(["-c", "printf out; printf err >&2"]),
            temp.path(),
            temp.path().join("stdout"),
            temp.path().join("stderr"),
        );
        let result = Executor::new(ExecutionConfig::default())
            .execute(success)
            .await
            .unwrap();

        assert_eq!(result.status, ExecutionStatus::Succeeded);
        assert_eq!(result.stdout, "out");
        assert_eq!(result.stderr, "err");

        let failure = ExecutionRequest::new(
            CommandSpec::new("/bin/sh").args(["-c", "exit 7"]),
            temp.path(),
            temp.path().join("failed.stdout"),
            temp.path().join("failed.stderr"),
        );
        let result = Executor::new(ExecutionConfig::default())
            .execute(failure)
            .await
            .unwrap();
        assert_eq!(result.status, ExecutionStatus::Failed);
        assert_eq!(result.exit_code, Some(7));
    }

    #[tokio::test]
    async fn local_environment_is_cleared_and_allowlisted() {
        let temp = tempfile::tempdir().unwrap();
        let config = ExecutionConfig {
            forwarded_env: vec!["DISPATCH_ALLOWED_TEST_VALUE".into()],
            ..ExecutionConfig::default()
        };
        let request = ExecutionRequest::new(
            CommandSpec::new("/usr/bin/env")
                .env("DISPATCH_ALLOWED_TEST_VALUE", "visible")
                .env("DISPATCH_BLOCKED_TEST_VALUE", "hidden"),
            temp.path(),
            temp.path().join("stdout"),
            temp.path().join("stderr"),
        );
        let result = Executor::new(config).execute(request).await.unwrap();

        assert!(
            result
                .stdout
                .lines()
                .any(|line| { line == "DISPATCH_ALLOWED_TEST_VALUE=[REDACTED]" })
        );
        assert!(!result.stdout.contains("DISPATCH_BLOCKED_TEST_VALUE"));
    }

    #[tokio::test]
    async fn redacts_forwarded_values_before_persisting_logs() {
        let temp = tempfile::tempdir().unwrap();
        let secret = "dispatch-test-secret-value";
        let config = ExecutionConfig {
            forwarded_env: vec!["DISPATCH_SECRET_TEST_VALUE".into()],
            ..ExecutionConfig::default()
        };
        let stdout_path = temp.path().join("stdout");
        let stderr_path = temp.path().join("stderr");
        let request = ExecutionRequest::new(
            CommandSpec::new("/bin/sh")
                .args([
                    "-c",
                    "printf '\\377%s' \"$DISPATCH_SECRET_TEST_VALUE\"; printf '%s' \"$DISPATCH_SECRET_TEST_VALUE\" >&2",
                ])
                .env("DISPATCH_SECRET_TEST_VALUE", secret),
            temp.path(),
            &stdout_path,
            &stderr_path,
        );
        let result = Executor::new(config).execute(request).await.unwrap();

        assert!(!result.stdout.contains(secret));
        assert!(!result.stderr.contains(secret));
        assert!(result.stdout.contains("[REDACTED]"));
        assert!(result.stderr.contains("[REDACTED]"));
        assert!(
            !fs::read(&stdout_path)
                .await
                .unwrap()
                .windows(secret.len())
                .any(|v| v == secret.as_bytes())
        );
        assert!(
            !fs::read(&stderr_path)
                .await
                .unwrap()
                .windows(secret.len())
                .any(|v| v == secret.as_bytes())
        );
    }

    #[tokio::test]
    async fn drains_but_caps_noisy_process_output() {
        let temp = tempfile::tempdir().unwrap();
        let stdout_path = temp.path().join("stdout");
        let request = ExecutionRequest::new(
            CommandSpec::new("/bin/sh").args(["-c", "head -c 100000 /dev/zero"]),
            temp.path(),
            &stdout_path,
            temp.path().join("stderr"),
        );
        let result = Executor::new(ExecutionConfig::default())
            .execute(request)
            .await
            .unwrap();

        assert_eq!(result.status, ExecutionStatus::Succeeded);
        assert!(result.stdout.contains("[dispatch: output truncated"));
        assert!(fs::metadata(stdout_path).await.unwrap().len() < 70_000);
    }

    #[tokio::test]
    async fn enforces_timeout() {
        let temp = tempfile::tempdir().unwrap();
        let request = ExecutionRequest::new(
            CommandSpec::new("/bin/sleep").arg("10"),
            temp.path(),
            temp.path().join("stdout"),
            temp.path().join("stderr"),
        )
        .with_timeout(Duration::from_millis(30));
        let result = Executor::new(ExecutionConfig::default())
            .execute(request)
            .await
            .unwrap();

        assert_eq!(result.status, ExecutionStatus::TimedOut);
        assert!(result.timed_out);
        assert!(result.duration_ms < 2_000);
    }

    #[tokio::test]
    async fn cancellation_kills_a_running_process() {
        let temp = tempfile::tempdir().unwrap();
        let request = ExecutionRequest::new(
            CommandSpec::new("/bin/sleep").arg("10"),
            temp.path(),
            temp.path().join("stdout"),
            temp.path().join("stderr"),
        );
        let cancellation = CancellationToken::new();
        let cancel_from_task = cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            cancel_from_task.cancel();
        });

        let result = Executor::new(ExecutionConfig::default())
            .execute_with_cancel(request, cancellation)
            .await
            .unwrap();
        assert_eq!(result.status, ExecutionStatus::Cancelled);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn detached_descendant_cannot_hold_capture_pipes_forever() {
        let temp = tempfile::tempdir().unwrap();
        let helper_flag = "DISPATCH_DETACHED_PIPE_HELPER";
        let config = ExecutionConfig {
            forwarded_env: vec![helper_flag.into()],
            ..ExecutionConfig::default()
        };
        let request = ExecutionRequest::new(
            CommandSpec::new(std::env::current_exe().unwrap().to_string_lossy())
                .args(["--exact", "executor::tests::detached_pipe_helper"])
                .env(helper_flag, "1"),
            temp.path(),
            temp.path().join("stdout"),
            temp.path().join("stderr"),
        );
        let started = Instant::now();
        let error = Executor::new(config)
            .execute(request)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("output pipes remained open"));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[cfg(unix)]
    #[test]
    fn detached_pipe_helper() {
        if std::env::var_os("DISPATCH_DETACHED_PIPE_HELPER").is_none() {
            return;
        }
        // SAFETY: both fork branches immediately restrict themselves to
        // async-signal-safe libc calls and terminate with _exit.
        unsafe {
            let child = libc::fork();
            if child == 0 {
                libc::setsid();
                libc::sleep(2);
                libc::_exit(0);
            }
            libc::usleep(100_000);
            libc::_exit(0);
        }
    }

    #[test]
    fn docker_command_has_limits_single_mount_and_forwarded_environment() {
        let temp = tempfile::tempdir().unwrap();
        let config = ExecutionConfig {
            backend: "docker".into(),
            cpus: 1.5,
            memory: "768m".into(),
            docker_image: "dispatch-test:latest".into(),
            forwarded_env: vec!["DISPATCH_TEST_TOKEN".into()],
            ..ExecutionConfig::default()
        };
        let request = ExecutionRequest::new(
            CommandSpec::new("agent")
                .arg("run")
                .env("DISPATCH_TEST_TOKEN", "secret"),
            temp.path(),
            temp.path().join("stdout"),
            temp.path().join("stderr"),
        );
        let command = Executor::new(config).resolved_command(&request).unwrap();

        assert!(Path::new(&command.program).is_absolute());
        assert_eq!(Path::new(&command.program).file_name().unwrap(), "docker");
        assert_eq!(command.args[0..2], ["run", "--rm"]);
        assert!(command.args.windows(2).any(|a| a == ["--cpus", "1.5"]));
        assert!(command.args.windows(2).any(|a| a == ["--memory", "768m"]));
        assert!(
            command
                .args
                .windows(2)
                .any(|a| a == ["--pids-limit", "512"])
        );
        assert!(
            command
                .args
                .windows(2)
                .any(|a| a == ["--workdir", "/workspace"])
        );
        assert!(
            command
                .args
                .windows(2)
                .any(|a| a == ["--env", "DISPATCH_TEST_TOKEN"])
        );
        assert_eq!(
            command.env.get("DISPATCH_TEST_TOKEN").map(String::as_str),
            Some("secret")
        );
        assert_eq!(
            command.args.iter().filter(|arg| *arg == "--mount").count(),
            1
        );
        assert_eq!(
            &command.args[command.args.len() - 3..],
            ["dispatch-test:latest", "agent", "run"]
        );
    }

    #[tokio::test]
    async fn checks_keep_individual_pass_and_fail_results() {
        let temp = tempfile::tempdir().unwrap();
        let commands = vec![
            "printf passed".to_owned(),
            "printf failed >&2; exit 9".to_owned(),
        ];
        let results = run_checks(
            temp.path(),
            &commands,
            CheckPhase::Verify,
            &temp.path().join("checks"),
            2,
        )
        .await;

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].status, CheckStatus::Passed);
        assert_eq!(results[1].status, CheckStatus::Failed);
        assert_eq!(results[1].exit_code, Some(9));
        assert_eq!(
            fs::read_to_string(&results[0].stdout_path).await.unwrap(),
            "passed"
        );
        assert_eq!(
            fs::read_to_string(&results[1].stderr_path).await.unwrap(),
            "failed"
        );
    }
}
