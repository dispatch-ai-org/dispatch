//! `dispatch hook claude`: Claude Code's session hooks make Work appear by
//! themselves in a watched project. The payloads are the ones Claude Code
//! sends; the worktree sits where `claude --worktree` puts it, inside the
//! checkout under `.claude/worktrees/`.
#![cfg(unix)]

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use serde_json::Value;

struct Project {
    _temp: tempfile::TempDir,
    root: PathBuf,
    worktree: PathBuf,
    state: PathBuf,
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
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
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

impl Project {
    fn new(config: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn f() -> i32 {\n    1\n}\n").unwrap();
        fs::write(
            root.join("dispatch.yml"),
            format!("coherence:\n  poll_secs: 1\n{config}"),
        )
        .unwrap();
        git(&root, &["init", "--quiet"]);
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "--quiet", "-m", "initial"]);
        let worktree = root.join(".claude/worktrees/x");
        git(
            &root,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "worktree-x",
                ".claude/worktrees/x",
            ],
        );
        Self {
            root: fs::canonicalize(&root).unwrap(),
            worktree: fs::canonicalize(&worktree).unwrap(),
            state: temp.path().join("state"),
            _temp: temp,
        }
    }

    fn dispatch(&self, args: &[&str]) -> Output {
        Command::new(assert_cmd::cargo_bin!("dispatch"))
            .arg("--state-dir")
            .arg(&self.state)
            .args(args)
            .current_dir(&self.root)
            .output()
            .unwrap()
    }

    fn watch(&self) {
        let output = self.dispatch(&["start"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Run the hook with raw input; the hook itself must always succeed.
    fn hook_raw(&self, input: &[u8]) -> String {
        let mut child = Command::new(assert_cmd::cargo_bin!("dispatch"))
            .arg("--state-dir")
            .arg(&self.state)
            .args(["hook", "claude"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn hook(&self, event: Value) -> String {
        self.hook_raw(event.to_string().as_bytes())
    }

    fn start(&self, session: &str, source: &str, cwd: &Path) -> String {
        self.hook(serde_json::json!({
            "hook_event_name": "SessionStart", "session_id": session, "source": source,
            "cwd": cwd, "transcript_path": "/dev/null", "model": "claude-sonnet-5",
        }))
    }

    fn runs(&self) -> Vec<Value> {
        let Ok(entries) = fs::read_dir(self.state.join("runs")) else {
            return Vec::new();
        };
        entries
            .map(|entry| {
                let path = entry.unwrap().path().join("metadata.json");
                serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
            })
            .collect()
    }

    fn only_run(&self) -> Value {
        let runs = self.runs();
        assert_eq!(runs.len(), 1, "{runs:?}");
        runs.into_iter().next().unwrap()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = self.dispatch(&["stop"]);
    }
}

#[test]
fn an_unwatched_project_is_never_touched() {
    let p = Project::new("");
    assert_eq!(p.start("s1", "startup", &p.worktree), "");
    assert!(p.runs().is_empty());
}

#[test]
fn a_session_in_the_checkout_itself_gets_a_notice_and_no_work() {
    let p = Project::new("");
    p.watch();
    let reply: Value = serde_json::from_str(&p.start("s1", "startup", &p.root)).unwrap();
    let notice = reply["systemMessage"].as_str().unwrap();
    assert!(notice.contains("directly in your checkout"), "{notice}");
    assert!(p.runs().is_empty());
}

#[test]
fn a_session_in_its_own_worktree_becomes_work_with_s0_before_its_edits() {
    let p = Project::new("checks:\n  verify: ['true']\n");
    p.watch();
    // The worktree as the session finds it is S0, uncommitted file and all.
    fs::write(p.worktree.join("before.txt"), "there before the session\n").unwrap();
    let reply = p.start("s1", "startup", &p.worktree.join("src"));
    assert!(reply.contains("tracking this worktree"), "{reply}");

    let run = p.only_run();
    let attachment = &run["attachment"];
    assert_eq!(run["mode"], "attached");
    assert_eq!(attachment["workspace"], p.worktree.to_str().unwrap());
    assert_eq!(attachment["integration_root"], p.root.to_str().unwrap());
    assert_eq!(attachment["workspace_owner"], "runtime");
    assert_eq!(attachment["confidence"], "full");
    assert!(attachment["provenance"]["workspace_at_start"]["commit"].is_string());
    assert_eq!(attachment["agent"], "claude");
    assert_eq!(attachment["sessions"][0]["session_id"], "s1");
    assert_eq!(attachment["sessions"][0]["model"], "claude-sonnet-5");
    assert_eq!(run["environment"]["unsafe_local"], false);

    // The session's own edit is Δ; what was there before it is not.
    fs::write(
        p.worktree.join("src/lib.rs"),
        "pub fn f() -> i32 {\n    2\n}\n",
    )
    .unwrap();
    let id = run["id"].as_str().unwrap();
    let refused = p.dispatch(&["finish", id]);
    assert!(
        !refused.status.success(),
        "checks need a person's authority"
    );
    let finish = p.dispatch(&["finish", id, "--allow-unsafe-local"]);
    assert!(
        finish.status.success(),
        "{}",
        String::from_utf8_lossy(&finish.stderr)
    );
    let patch = fs::read_to_string(p.state.join("runs").join(id).join("delta.patch")).unwrap();
    assert!(
        patch.contains("+    2") && !patch.contains("before.txt"),
        "{patch}"
    );
}

#[test]
fn repeated_and_later_sessions_land_on_one_work() {
    let p = Project::new("");
    p.watch();
    p.start("s1", "startup", &p.worktree);
    p.start("s1", "startup", &p.worktree);
    assert_eq!(p.start("s2", "resume", &p.worktree), "");
    p.hook(serde_json::json!({
        "hook_event_name": "SessionEnd", "session_id": "s1", "reason": "clear",
        "cwd": p.worktree,
    }));

    let run = p.only_run();
    let sessions = run["attachment"]["sessions"].as_array().unwrap();
    let ids: Vec<&str> = sessions
        .iter()
        .map(|s| s["session_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["s1", "s2"]);
    assert_eq!(sessions[0]["end_reason"], "clear");
    assert!(sessions[1]["ended_at"].is_null());
    // An ended session does not end the Work.
    assert_eq!(run["outcome"]["lifecycle"], "working");
}

#[test]
fn a_resume_into_a_worktree_dispatch_has_not_seen_is_partial() {
    let p = Project::new("");
    p.watch();
    p.start("s9", "resume", &p.worktree);
    assert_eq!(p.only_run()["attachment"]["confidence"], "partial");
}

#[test]
fn malformed_hook_input_is_refused_whole_and_never_fails_the_session() {
    let p = Project::new("");
    p.watch();
    for reply in [
        p.start("../escape", "startup", &p.worktree),
        p.hook(serde_json::json!({
            "hook_event_name": "SessionStart", "session_id": "s1", "cwd": "relative/dir",
        })),
        p.hook_raw(&vec![b' '; 70 * 1024]),
        p.hook_raw(b"not json"),
    ] {
        assert!(reply.contains("could not track this session"), "{reply}");
    }
    assert!(p.runs().is_empty());
    // An event Dispatch does not use is silently ignored.
    let other = p.hook(serde_json::json!({
        "hook_event_name": "Stop", "session_id": "s1", "cwd": p.worktree,
    }));
    assert_eq!(other, "");
}
