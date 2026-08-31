use std::{
    fs,
    io::ErrorKind,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use assert_cmd::cargo_bin_cmd;
use rusqlite::Connection;
use serde_json::Value;

#[test]
fn normal_local_commands_do_not_contact_cloud() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let cloud_url = format!("http://{}", listener.local_addr().unwrap());
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let state = temp.path().join("state");

    dispatch_command(&state, &cloud_url)
        .arg("init")
        .arg(&source)
        .assert()
        .success();
    dispatch_command(&state, &cloud_url)
        .arg("doctor")
        .arg(&source)
        .assert()
        .success();
    let run = dispatch_command(&state, &cloud_url)
        .arg("run")
        .arg(&source)
        .args(["--task", "Exercise the local workflow."])
        .assert()
        .success()
        .get_output()
        .clone();
    let run_id = String::from_utf8(run.stdout)
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("RUN "))
        .unwrap()
        .to_owned();
    dispatch_command(&state, &cloud_url)
        .args(["compare", &run_id, "--winner", "tie"])
        .assert()
        .success();
    dispatch_command(&state, &cloud_url)
        .arg("history")
        .assert()
        .success();

    assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
}

#[test]
fn non_git_fake_harness_evaluation_and_safe_apply_work_end_to_end() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("plain-source");
    let state = temp.path().join("dispatch-state");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("original.txt"), "unchanged baseline\n").unwrap();
    fs::write(
        source.join("dispatch.yml"),
        "execution:\n  timeout_secs: 1\n  max_parallel: 3\nchecks:\n  verify:\n    - test -f dispatch-fake-good.txt || test -f dispatch-fake-bad.txt\n",
    )
    .unwrap();

    let output = cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .arg("run")
        .arg(&source)
        .arg("--allow-unsafe-local")
        .args([
            "--task",
            "Make a deterministic candidate artifact.",
            "--harnesses",
            "fake-good,fake-bad,fake-crash",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let run_id = stdout
        .lines()
        .find_map(|line| line.strip_prefix("RUN "))
        .expect("run output includes an ID")
        .to_owned();

    assert_eq!(
        fs::read_to_string(source.join("original.txt")).unwrap(),
        "unchanged baseline\n"
    );
    assert!(!source.join("dispatch-fake-good.txt").exists());
    assert!(!source.join("dispatch-fake-bad.txt").exists());

    let metadata_path = state.join("runs").join(&run_id).join("metadata.json");
    let metadata: Value = serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
    assert_eq!(metadata["source_kind"], "directory");
    assert_eq!(metadata["status"], "ready_for_evaluation");
    assert_eq!(metadata["baseline_checks"][0]["status"], "failed");
    let candidates = metadata["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 3);
    let good = candidate_for(candidates, "fake-good");
    let bad = candidate_for(candidates, "fake-bad");
    let crash = candidate_for(candidates, "fake-crash");
    assert_eq!(good["status"], "completed");
    assert_eq!(bad["status"], "completed");
    assert_eq!(crash["status"], "failed");
    assert_eq!(good["checks"].as_array().unwrap().len(), 1);
    assert_eq!(good["checks"][0]["status"], "passed");
    assert_eq!(bad["checks"][0]["status"], "passed");
    assert_eq!(good["diff_stats"]["files_changed"], 1);
    assert!(
        Path::new(good["workspace_path"].as_str().unwrap())
            .join("original.txt")
            .is_file()
    );
    assert!(Path::new(good["diff_path"].as_str().unwrap()).is_file());

    let blind_output = cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["compare", &run_id])
        .assert()
        .success()
        .get_output()
        .clone();
    let blind_output = String::from_utf8(blind_output.stdout).unwrap();
    assert!(blind_output.contains("Harness mapping  blind"));
    assert!(blind_output.contains("Baseline verification FAIL"));
    assert!(!blind_output.contains("fake-good"));
    assert!(!blind_output.contains("fake-bad"));

    let good_label = good["label"].as_str().unwrap();
    let invalid = cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["compare", &run_id, "--winner", good_label])
        .args(["--reason", "some-unsupported-value"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let invalid_output = format!(
        "{}{}",
        String::from_utf8_lossy(&invalid.stdout),
        String::from_utf8_lossy(&invalid.stderr)
    );
    assert!(invalid_output.contains("unknown structured reason"));
    assert!(invalid_output.contains("cleaner-change"));
    assert!(!invalid_output.contains("fake-good"));
    assert!(!invalid_output.contains("fake-bad"));
    let rejected: Value = serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
    assert!(rejected["evaluation"].is_null());

    let explanation = "\nThe exact regression behavior is right.\nKeep this spacing verbatim.  \n";
    let explanation_path = temp.path().join("reasoning.txt");
    fs::write(&explanation_path, explanation).unwrap();
    let evaluated_output = cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["compare", &run_id, "--winner", good_label])
        .args(["--reason", "correctness", "--reason", "tests"])
        .arg("--explanation-file")
        .arg(&explanation_path)
        .assert()
        .success()
        .get_output()
        .clone();
    let evaluated_output = String::from_utf8(evaluated_output.stdout).unwrap();
    assert!(evaluated_output.contains("Harness identities are now revealed"));
    assert!(evaluated_output.contains("fake-good"));

    let evaluated: Value = serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
    assert_eq!(evaluated["evaluation"]["outcome"]["kind"], "candidate");
    assert_eq!(evaluated["evaluation"]["explanation"], explanation);
    assert_eq!(
        evaluated["evaluation"]["reasons"],
        serde_json::json!(["correctness", "tests"])
    );
    let reloaded = cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["compare", &run_id])
        .assert()
        .success()
        .get_output()
        .clone();
    let reloaded = String::from_utf8(reloaded.stdout).unwrap();
    assert!(reloaded.contains("Harness mapping  revealed"));
    assert!(reloaded.contains("fake-good"));
    assert!(reloaded.contains("The exact regression behavior is right."));

    let database = Connection::open(state.join("dispatch.db")).unwrap();
    let sync_enabled: bool = database
        .query_row(
            "SELECT enabled FROM sync_settings WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let queued: i64 = database
        .query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0))
        .unwrap();
    assert!(!sync_enabled);
    assert_eq!(queued, 0);
    drop(database);

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["sync", "status"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Evaluation sync: disabled"));
    let token = "developer-preview-token-secret";
    let set_token = cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .arg("-vv")
        .args(["sync", "token", "set", token])
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(!String::from_utf8_lossy(&set_token.stdout).contains(token));
    assert!(!String::from_utf8_lossy(&set_token.stderr).contains(token));
    let token_status = cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .arg("-vv")
        .args(["sync", "token", "status"])
        .assert()
        .success()
        .get_output()
        .clone();
    let token_status_stdout = String::from_utf8_lossy(&token_status.stdout);
    assert!(token_status_stdout.contains("ingestion token: configured"));
    assert!(!token_status_stdout.contains(token));
    assert!(!String::from_utf8_lossy(&token_status.stderr).contains(token));
    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["sync", "enable"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Source code, full diffs, logs"));
    let preview = cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["sync", "preview", &run_id])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(!String::from_utf8_lossy(&preview).contains(token));
    let preview: Value = serde_json::from_slice(&preview).unwrap();
    assert_eq!(preview["schema_version"], 1);
    assert_eq!(preview["run"]["run_id"], run_id);
    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["sync", "disable"])
        .assert()
        .success();
    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["sync", "token", "status"])
        .assert()
        .success()
        .stdout(predicates::str::contains("ingestion token: configured"));
    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["sync", "token", "clear"])
        .assert()
        .success();

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["apply", &run_id, good_label])
        .assert()
        .success();
    assert!(source.join("dispatch-fake-good.txt").is_file());
    assert_eq!(
        fs::read_to_string(source.join("original.txt")).unwrap(),
        "unchanged baseline\n"
    );

    let database = Connection::open(state.join("dispatch.db")).unwrap();
    let stored_explanation: String = database
        .query_row(
            "SELECT explanation FROM evaluations WHERE run_id = ?1",
            [&run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored_explanation, explanation);
    let event_count: i64 = database
        .query_row(
            "SELECT COUNT(*) FROM events WHERE run_id = ?1",
            [&run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(event_count >= 10);

    // A later run snapshots the now-current source. Any edit after that point
    // makes apply fail before writing even one candidate file.
    let second_output = cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .arg("run")
        .arg(&source)
        .arg("--allow-unsafe-local")
        .args([
            "--task",
            "Create another fake artifact.",
            "--harnesses",
            "fake-bad",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let second_stdout = String::from_utf8(second_output.stdout).unwrap();
    let second_run = second_stdout
        .lines()
        .find_map(|line| line.strip_prefix("RUN "))
        .unwrap();
    database
        .execute("DELETE FROM candidates WHERE run_id = ?1", [second_run])
        .unwrap();
    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["compare", second_run, "--winner", "A"])
        .assert()
        .failure();
    let second_metadata = state.join("runs").join(second_run).join("metadata.json");
    let rejected: Value = serde_json::from_slice(&fs::read(second_metadata).unwrap()).unwrap();
    assert!(rejected["evaluation"].is_null());
    let still_blind = cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["compare", second_run])
        .assert()
        .success()
        .get_output()
        .clone();
    let still_blind = String::from_utf8(still_blind.stdout).unwrap();
    assert!(still_blind.contains("Harness mapping  blind"));
    assert!(!still_blind.contains("fake-bad"));
    fs::write(
        source.join("original.txt"),
        "user changed this after the run\n",
    )
    .unwrap();
    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["apply", second_run, "A"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "source has changed since this run was created",
        ));
}

fn candidate_for<'a>(candidates: &'a [Value], harness: &str) -> &'a Value {
    candidates
        .iter()
        .find(|candidate| candidate["harness_id"] == harness)
        .unwrap_or_else(|| panic!("missing {harness} candidate"))
}

fn dispatch_command(state: &Path, cloud_url: &str) -> assert_cmd::Command {
    let mut command = cargo_bin_cmd!("dispatch");
    command
        .args(["--state-dir"])
        .arg(state)
        .env("DISPATCH_CLOUD_URL", cloud_url);
    command
}

#[cfg(unix)]
#[test]
fn interrupt_cancels_children_and_persists_terminal_status() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let state = temp.path().join("state");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("input.txt"), "baseline\n").unwrap();
    fs::write(
        source.join("dispatch.yml"),
        "execution:\n  timeout_secs: 30\n",
    )
    .unwrap();

    let mut child = Command::new(assert_cmd::cargo_bin!("dispatch"))
        .args(["--state-dir"])
        .arg(&state)
        .arg("run")
        .arg(&source)
        .args([
            "--task",
            "Wait until interrupted.",
            "--harnesses",
            "fake-timeout",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(8);
    let metadata_path = loop {
        if let Some(path) = first_metadata_path(&state) {
            let metadata: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            if metadata["status"] == "running" {
                break path;
            }
        }
        assert!(
            Instant::now() < deadline,
            "run never reached running status"
        );
        thread::sleep(Duration::from_millis(20));
    };

    // SAFETY: this targets only the Dispatch child spawned by this test.
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGINT) }, 0);
    let exit_deadline = Instant::now() + Duration::from_secs(8);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= exit_deadline {
            let _ = child.kill();
            panic!("Dispatch did not exit after SIGINT");
        }
        thread::sleep(Duration::from_millis(20));
    };
    assert!(!status.success());

    let metadata: Value = serde_json::from_slice(&fs::read(metadata_path).unwrap()).unwrap();
    assert_eq!(metadata["status"], "interrupted");
    assert_eq!(metadata["candidates"][0]["status"], "cancelled");
}

fn first_metadata_path(state: &Path) -> Option<PathBuf> {
    fs::read_dir(state.join("runs"))
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("metadata.json"))
        .find(|path| path.is_file())
}
