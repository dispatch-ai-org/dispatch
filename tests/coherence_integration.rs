//! The L2 integration oracle: when the source moved but the static layers see
//! no conflict, the run's `checks.verify` commands run against the merged tree
//! before anything is applied. Uses the built-in `fake-good` harness, which
//! creates one empty file, `dispatch-fake-good.txt`.
#![cfg(unix)]

use std::{fs, path::PathBuf};

use assert_cmd::cargo_bin_cmd;
use serde_json::Value;

struct Fixture {
    _temp: tempfile::TempDir,
    source: PathBuf,
    state: PathBuf,
    run_id: String,
    label: String,
}

impl Fixture {
    /// `config` is appended to a `dispatch.yml` that sets a 30 second timeout.
    fn new(config: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let state = temp.path().join("state");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("original.txt"), "baseline\n").unwrap();
        fs::write(
            source.join("dispatch.yml"),
            format!("execution:\n  timeout_secs: 30\n{config}"),
        )
        .unwrap();
        let output = cargo_bin_cmd!("dispatch")
            .arg("--state-dir")
            .arg(&state)
            .arg("run")
            .arg(&source)
            .arg("--allow-unsafe-local")
            .args([
                "--task",
                "Create the fake artifact.",
                "--harnesses",
                "fake-good",
            ])
            .assert()
            .success()
            .get_output()
            .clone();
        let run_id = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix("RUN "))
            .expect("run output includes an ID")
            .to_owned();
        let fixture = Self {
            _temp: temp,
            source,
            state,
            run_id,
            label: String::new(),
        };
        let label = fixture.metadata()["candidates"][0]["label"]
            .as_str()
            .unwrap()
            .to_owned();
        Self { label, ..fixture }
    }

    fn run_dir(&self) -> PathBuf {
        self.state.join("runs").join(&self.run_id)
    }

    fn metadata(&self) -> Value {
        serde_json::from_slice(&fs::read(self.run_dir().join("metadata.json")).unwrap()).unwrap()
    }

    fn apply(&self) -> assert_cmd::assert::Assert {
        cargo_bin_cmd!("dispatch")
            .arg("--state-dir")
            .arg(&self.state)
            .args(["apply", &self.run_id, &self.label])
            .assert()
    }

    /// Names directly under the run directory that start with `prefix`.
    fn run_dir_entries(&self, prefix: &str) -> Vec<String> {
        fs::read_dir(self.run_dir())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(prefix))
            .collect()
    }

    fn assert_no_scratch(&self) {
        assert_eq!(
            self.run_dir_entries("coherence-scratch-"),
            Vec::<String>::new()
        );
    }

    fn analysis(&self) -> Value {
        self.metadata()["coherence"]["validity"]["analysis"].clone()
    }
}

const VERIFY_NO_FORBIDDEN: &str = "checks:\n  verify:\n    - test ! -f forbidden.txt\n";

/// The world adds a file that only the merged tree's check objects to: the
/// check passes at the baseline and for the patch alone.
fn add_forbidden(fixture: &Fixture) {
    fs::write(
        fixture.source.join("forbidden.txt"),
        "added by someone else\n",
    )
    .unwrap();
}

#[test]
fn passing_check_on_the_merged_tree_applies_and_records_integration() {
    let fixture = Fixture::new("checks:\n  verify:\n    - test -f original.txt\n");
    fs::write(fixture.source.join("notes.txt"), "someone else's work\n").unwrap();

    fixture.apply().success();

    assert!(fixture.source.join("dispatch-fake-good.txt").is_file());
    assert!(fixture.source.join("notes.txt").is_file());
    let metadata = fixture.metadata();
    assert_eq!(metadata["status"], "applied");
    assert_eq!(fixture.analysis(), "integration");
    assert_eq!(metadata["coherence"]["validity"]["decision"], "continue");
    fixture.assert_no_scratch();
    assert!(fixture.run_dir().join("coherence-checks").is_dir());
}

#[test]
fn check_failing_only_on_the_merged_tree_blocks_apply() {
    let fixture = Fixture::new(VERIFY_NO_FORBIDDEN);
    add_forbidden(&fixture);

    let assert = fixture.apply().failure();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(stderr.contains("stale"), "unexpected error: {stderr}");
    assert!(
        stderr.contains("test ! -f forbidden.txt") && stderr.contains("exit 1"),
        "unexpected error: {stderr}"
    );
    assert!(!fixture.source.join("dispatch-fake-good.txt").exists());
    assert!(fixture.source.join("forbidden.txt").is_file());
    let metadata = fixture.metadata();
    assert_ne!(metadata["status"], "applied");
    assert_eq!(
        metadata["outcome"]["application"],
        "blocked_by_source_drift"
    );
    let validity = &metadata["coherence"]["validity"];
    assert_eq!(validity["decision"], "refresh");
    assert_eq!(validity["reasons"][0]["code"], "integration_check_failed");
    fixture.assert_no_scratch();

    // The logs named in the reason are kept as evidence.
    let detail = validity["reasons"][0]["detail"].as_str().unwrap();
    let log = detail.rsplit("logs: ").next().unwrap();
    assert!(
        PathBuf::from(log).starts_with(fixture.run_dir().join("coherence-checks")),
        "{detail}"
    );
    assert!(PathBuf::from(log).is_file(), "{detail}");
}

#[test]
fn disabling_integration_checks_skips_the_oracle() {
    let fixture = Fixture::new(&format!(
        "{VERIFY_NO_FORBIDDEN}coherence:\n  integration_checks: false\n"
    ));
    add_forbidden(&fixture);

    fixture.apply().success();

    assert!(fixture.source.join("dispatch-fake-good.txt").is_file());
    assert_eq!(fixture.analysis(), "files_only");
    assert!(!fixture.run_dir().join("coherence-checks").exists());
    fixture.assert_no_scratch();
}

#[test]
fn no_verify_checks_means_nothing_to_run() {
    let fixture = Fixture::new("");
    add_forbidden(&fixture);

    fixture.apply().success();

    assert!(fixture.source.join("dispatch-fake-good.txt").is_file());
    assert_eq!(fixture.analysis(), "files_only");
    assert!(!fixture.run_dir().join("coherence-checks").exists());
    fixture.assert_no_scratch();
}

#[test]
fn unmoved_source_never_creates_a_scratch_tree() {
    let fixture = Fixture::new(VERIFY_NO_FORBIDDEN);

    fixture.apply().success();

    assert!(fixture.source.join("dispatch-fake-good.txt").is_file());
    assert!(fixture.metadata()["coherence"].is_null());
    assert!(fixture.run_dir_entries("coherence-").is_empty());
}

#[test]
fn hanging_check_times_out_and_refreshes() {
    let fixture = Fixture::new("checks:\n  verify:\n    - if [ -f hang.txt ]; then sleep 60; fi\n");
    // The timeout applies to every verify command, including this one.
    let snapshot = fixture.run_dir().join("config.snapshot.yml");
    let text = fs::read_to_string(&snapshot).unwrap();
    assert!(text.contains("timeout_secs: 30"), "{text}");
    fs::write(
        &snapshot,
        text.replace("timeout_secs: 30", "timeout_secs: 2"),
    )
    .unwrap();
    fs::write(fixture.source.join("hang.txt"), "hang\n").unwrap();

    let assert = fixture.apply().failure();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(stderr.contains("stale"), "unexpected error: {stderr}");
    assert!(stderr.contains("timed out"), "unexpected error: {stderr}");
    assert!(!fixture.source.join("dispatch-fake-good.txt").exists());
    assert_eq!(
        fixture.metadata()["coherence"]["validity"]["reasons"][0]["code"],
        "integration_check_failed"
    );
    fixture.assert_no_scratch();
}

#[test]
fn a_run_without_local_execution_authority_is_never_verified() {
    let fixture = Fixture::new("");
    // Grant checks after the fact: the run recorded no unsafe-local
    // acknowledgement, so accept time must not run them.
    let snapshot = fixture.run_dir().join("config.snapshot.yml");
    let text = fs::read_to_string(&snapshot).unwrap();
    assert!(text.contains("verify: []"), "{text}");
    fs::write(
        &snapshot,
        text.replace("verify: []", "verify:\n  - 'false'"),
    )
    .unwrap();
    add_forbidden(&fixture);

    fixture.apply().success();

    assert!(fixture.source.join("dispatch-fake-good.txt").is_file());
    assert_eq!(fixture.analysis(), "files_only");
    assert!(!fixture.run_dir().join("coherence-checks").exists());
    fixture.assert_no_scratch();
}
