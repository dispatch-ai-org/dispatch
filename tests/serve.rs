//! `dispatch serve`: the repo-scoped foreground owner loop for foreign and
//! orphaned attached Work (`docs/plan-0.3-auto-apply-and-attach.md`, parts
//! 6.7, 6.9, 14.4, 14.11). The fixture mirrors `tests/attach_cli.rs`: a Git
//! repository (`root`) with `checks.verify: ['true']` and a fast
//! `coherence.poll_secs: 1`, plus a linked worktree (`workspace`) on a new
//! branch that a test edits directly, playing the role of an already-running
//! agent Dispatch did not launch.
#![cfg(unix)]

use std::{
    collections::HashSet,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

use assert_cmd::cargo_bin_cmd;
use serde_json::Value;

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    workspace: PathBuf,
    state: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn f() -> i32 {\n    1\n}\n").unwrap();
        fs::write(
            root.join("dispatch.yml"),
            "checks:\n  verify: ['true']\ncoherence:\n  poll_secs: 1\n",
        )
        .unwrap();
        git(&root, &["init", "--quiet"]);
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "--quiet", "-m", "initial"]);

        let workspace = temp.path().join("wt");
        git(
            &root,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "agent-branch",
                workspace.to_str().unwrap(),
            ],
        );

        let state = temp.path().join("state");
        Self {
            root: fs::canonicalize(&root).unwrap(),
            workspace: fs::canonicalize(&workspace).unwrap(),
            state,
            _temp: temp,
        }
    }

    /// A second linked worktree of the same repository, on its own branch.
    fn extra_worktree(&self, name: &str) -> PathBuf {
        let path = self._temp.path().join(name);
        git(
            &self.root,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                &format!("{name}-branch"),
                path.to_str().unwrap(),
            ],
        );
        fs::canonicalize(&path).unwrap()
    }

    fn dispatch(&self, args: &[&str]) -> assert_cmd::assert::Assert {
        cargo_bin_cmd!("dispatch")
            .arg("--state-dir")
            .arg(&self.state)
            .args(args)
            .assert()
    }

    /// `dispatch attach --workspace <workspace> --root <root>
    /// --allow-unsafe-local <extra...>`, returning the new run's ID.
    fn attach_workspace(&self, workspace: &Path, extra: &[&str]) -> String {
        let mut args = vec![
            "attach",
            "--workspace",
            workspace.to_str().unwrap(),
            "--root",
            self.root.to_str().unwrap(),
            "--allow-unsafe-local",
        ];
        args.extend_from_slice(extra);
        let assert = self.dispatch(&args).success();
        Self::stdout(&assert)
            .lines()
            .find_map(|line| line.strip_prefix("ATTACHED "))
            .expect("attach prints ATTACHED <id>")
            .to_owned()
    }

    fn attach(&self, extra: &[&str]) -> String {
        self.attach_workspace(&self.workspace, extra)
    }

    fn metadata_path(&self, id: &str) -> PathBuf {
        self.state.join("runs").join(id).join("metadata.json")
    }

    fn metadata(&self, id: &str) -> Value {
        serde_json::from_slice(&fs::read(self.metadata_path(id)).unwrap()).unwrap()
    }

    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.state.join("dispatch.db")).unwrap()
    }

    fn event_count(&self, id: &str, kind: &str) -> i64 {
        self.db()
            .query_row(
                "SELECT COUNT(*) FROM events WHERE run_id = ?1 AND event_type = ?2",
                rusqlite::params![id, kind],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn stdout(assert: &assert_cmd::assert::Assert) -> String {
        String::from_utf8_lossy(&assert.get_output().stdout).into_owned()
    }

    fn stderr(assert: &assert_cmd::assert::Assert) -> String {
        String::from_utf8_lossy(&assert.get_output().stderr).into_owned()
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env("LC_ALL", "C")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn wait_until(timeout: Duration, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "condition not met in {timeout:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A background `dispatch serve` process: stdout lines are streamed to a
/// channel by a reader thread so the test can poll with a deadline instead
/// of blocking forever. Killed and reaped on drop, so a panicking assertion
/// never leaves a `serve` process (and its lock) behind.
struct ServeProcess {
    child: std::process::Child,
    lines: mpsc::Receiver<String>,
}

impl ServeProcess {
    fn spawn(fixture: &Fixture, extra: &[&str]) -> Self {
        let mut command = Command::new(assert_cmd::cargo_bin!("dispatch"));
        command
            .arg("--state-dir")
            .arg(&fixture.state)
            .arg("serve")
            .arg("--root")
            .arg(&fixture.root)
            .args(extra)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("failed to start dispatch serve");
        let stdout = child.stdout.take().expect("serve stdout is piped");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Self { child, lines: rx }
    }

    fn next_line(&self, timeout: Duration) -> Option<String> {
        self.lines.recv_timeout(timeout).ok()
    }

    /// The first line matching `predicate`, waiting up to `timeout` total.
    fn wait_for(
        &self,
        timeout: Duration,
        mut predicate: impl FnMut(&Value) -> bool,
    ) -> Option<Value> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            let line = self.next_line(remaining)?;
            if let Ok(value) = serde_json::from_str::<Value>(&line)
                && predicate(&value)
            {
                return Some(value);
            }
        }
    }
}

impl Drop for ServeProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn second_serve_on_the_same_root_is_refused() {
    let fixture = Fixture::new();

    let first = ServeProcess::spawn(&fixture, &["--json"]);
    let world = first.wait_for(Duration::from_secs(10), |value| value["type"] == "world");
    assert!(world.is_some(), "serve did not print a world line in time");

    let second = fixture
        .dispatch(&["serve", "--root", fixture.root.to_str().unwrap(), "--json"])
        .failure();
    assert!(
        Fixture::stderr(&second).contains("already serving this root"),
        "{}",
        Fixture::stderr(&second)
    );

    drop(first);

    let third = ServeProcess::spawn(&fixture, &["--json"]);
    let world = third.wait_for(Duration::from_secs(10), |value| value["type"] == "world");
    assert!(world.is_some(), "a third serve did not start cleanly");
}

#[test]
fn serve_reevaluates_foreign_work_when_the_root_moves() {
    let fixture = Fixture::new();
    let id = fixture.attach(&[]);
    fs::write(
        fixture.workspace.join("src/lib.rs"),
        "pub fn f() -> i32 {\n    2\n}\n",
    )
    .unwrap();

    let serve = ServeProcess::spawn(&fixture, &["--json"]);
    serve.wait_for(Duration::from_secs(10), |value| value["type"] == "world");

    // The root moves underneath the active attached work, editing the same
    // line to a third, conflicting value.
    fs::write(
        fixture.root.join("src/lib.rs"),
        "pub fn f() -> i32 {\n    3\n}\n",
    )
    .unwrap();

    wait_until(Duration::from_secs(30), || {
        fixture.event_count(&id, "coherence.invalidated") > 0
    });
}

#[test]
fn serve_auto_applies_ready_foreign_work_with_integrate() {
    let fixture = Fixture::new();

    let id = fixture.attach(&["--auto-apply"]);
    fs::write(
        fixture.workspace.join("src/lib.rs"),
        "pub fn f() -> i32 {\n    2\n}\n",
    )
    .unwrap();
    fixture.dispatch(&["finish", &id]).success();

    let _serve = ServeProcess::spawn(&fixture, &["--json"]);
    wait_until(Duration::from_secs(30), || {
        let metadata = fixture.metadata(&id);
        metadata["outcome"]["application"] == "applied"
            && metadata["outcome"]["applied_by"] == "auto_apply"
    });

    // A second Work, attached without --auto-apply and finished, is not
    // applied after two ticks (poll_secs is 1s).
    let workspace2 = fixture.extra_worktree("wt2");
    let id2 = fixture.attach_workspace(&workspace2, &[]);
    fs::write(
        workspace2.join("src/lib.rs"),
        "pub fn f() -> i32 {\n    4\n}\n",
    )
    .unwrap();
    fixture.dispatch(&["finish", &id2]).success();

    std::thread::sleep(Duration::from_millis(2_500));
    let metadata2 = fixture.metadata(&id2);
    assert_eq!(metadata2["outcome"]["application"], "not_applied");
}

/// `orchestrator::attach::create`'s `AttachRequest` (`src/orchestrator/attach.rs`)
/// has no field for `attachment.owner` or `owner_state`: every run it creates
/// carries `owner: None, owner_state: Unknown`, which `serve`'s adoption
/// logic never touches (only a stored `Live` that is now `Gone` adopts; see
/// `orchestrator::serve::adopt_orphans`). Wrapped attach (S4), which is what
/// would actually record a live wrapper as `owner`, refuses to run on this
/// branch ("wrapped attach is not available yet"). There is no production
/// hook to construct a run with `owner_state: Live` short of hand-editing
/// persisted state, which this packet's task instructions say to avoid in
/// favor of reporting the gap. `adopt_orphans`'s pure pieces
/// (`live_owner_state`) are covered by unit tests in
/// `src/orchestrator/serve.rs` in the meantime.
#[test]
#[ignore = "no production hook constructs a run with attachment.owner_state: Live before S4 (wrapped attach) lands; see the doc comment above"]
fn serve_adopts_work_whose_owner_is_gone() {}

#[test]
fn serve_view_lists_native_and_attached_runs() {
    let fixture = Fixture::new();

    let native_id = {
        let assert = fixture
            .dispatch(&[
                "run",
                fixture.root.to_str().unwrap(),
                "--allow-unsafe-local",
                "--task",
                "Create the fake artifact.",
                "--harnesses",
                "fake-good",
            ])
            .success();
        Fixture::stdout(&assert)
            .lines()
            .find_map(|line| line.strip_prefix("RUN "))
            .expect("run prints RUN <id>")
            .trim()
            .to_owned()
    };
    let attached_id = fixture.attach(&[]);

    let serve = ServeProcess::spawn(&fixture, &["--json"]);
    let mut seen = HashSet::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while seen.len() < 2 && Instant::now() < deadline {
        let Some(line) = serve.next_line(Duration::from_secs(1)) else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<Value>(&line)
            && value["type"] == "work"
            && let Some(run_id) = value["run_id"].as_str()
        {
            seen.insert(run_id.to_owned());
        }
    }
    assert!(seen.contains(&native_id), "{seen:?}");
    assert!(seen.contains(&attached_id), "{seen:?}");
}
