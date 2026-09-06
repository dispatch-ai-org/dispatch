#![cfg(unix)]

use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    path::Path,
    thread,
};

use assert_cmd::cargo_bin_cmd;
use chrono::{TimeZone, Utc};
use dispatch::{
    BenchmarkPrior, RoutingDecision, TaskKind, TaskScope, db::Database,
    public_priors::PublicPriorSnapshotV1, router::rank_harnesses,
};
use predicates::prelude::PredicateBooleanExt;
use serde_json::Value;

fn repository(path: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(path.join("src"))?;
    fs::write(
        path.join("src/lib.rs"),
        "pub fn greeting() -> &'static str { \"helo\" }\n",
    )?;
    Ok(())
}

fn agent(path: &Path, id: &str, marker: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(
        path,
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"--version\" ]; then printf '{id} fixture 1.0\\n'; exit 0; fi\n\
             printf '{id}\\n' >> '{}'\n\
             printf '{id} changed this\\n' > agent-work.txt\n\
             printf '{{\"type\":\"result\",\"model\":\"fixture-{id}\"}}\\n'\n",
            marker.display()
        ),
    )?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn metadata(state: &Path) -> anyhow::Result<Value> {
    let path = fs::read_dir(state.join("runs"))?
        .next()
        .unwrap()?
        .path()
        .join("metadata.json");
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn serve_snapshot(body: Vec<u8>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let task = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let read = stream.read(&mut request).unwrap();
        assert!(
            String::from_utf8_lossy(&request[..read]).starts_with("GET /v1/public-priors/latest ")
        );
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(&body).unwrap();
    });
    (format!("http://{address}"), task)
}

#[test]
fn cold_start_routes_from_bundled_evidence_without_setup_or_network() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("repo");
    let state = temp.path().join("state");
    let bin = temp.path().join("bin");
    let marker = temp.path().join("agents-ran");
    repository(&source)?;
    agent(&bin.join("codex"), "codex", &marker)?;
    agent(&bin.join("cursor-agent"), "cursor", &marker)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;

    let output = cargo_bin_cmd!("dispatch")
        .current_dir(&source)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env(
            "DISPATCH_CLOUD_URL",
            format!("http://{}", listener.local_addr()?),
        )
        .args(["--state-dir"])
        .arg(&state)
        .args(["run", "Fix the greeting typo"])
        .write_stdin("y\n")
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8(output.stdout)?;

    assert!(stdout.contains("Agent\n  Codex"), "{stdout}");
    assert!(stdout.contains("Selection\n  Dispatch default"), "{stdout}");
    assert!(stdout.contains(
        "Why:\n  Dispatch did not have enough comparable public performance data\n  to make an evidence-based choice.\n  Available benchmark evidence did not determine the selection."
    ), "{stdout}");
    assert!(stdout.contains("Cursor: no compatible public evidence"));
    assert!(!stdout.contains("Evidence-based"));
    assert!(stdout.contains("123/330"), "{stdout}");
    assert!(
        stdout.contains("Verification\n  Not configured"),
        "{stdout}"
    );
    assert!(!stdout.to_lowercase().contains("confidence"));
    assert!(!stdout.to_lowercase().contains("probability"));
    assert_eq!(fs::read_to_string(&marker)?, "codex\n");
    let run = metadata(&state)?;
    assert_eq!(run["routing"]["selection_basis"], "default");
    assert_eq!(
        run["source_path"],
        source.canonicalize()?.to_string_lossy().as_ref()
    );
    assert_eq!(run["candidates"].as_array().unwrap().len(), 1);
    assert_eq!(run["candidates"][0]["harness_id"], "codex");
    assert!(!state.join("datasets").exists());
    assert!(!source.join("agent-work.txt").exists());
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    Ok(())
}

#[test]
fn no_evidence_uses_stable_real_agent_default_and_override_wins() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("repo");
    let state = temp.path().join("state");
    let bin = temp.path().join("bin");
    let marker = temp.path().join("agents-ran");
    repository(&source)?;
    agent(&bin.join("cursor-agent"), "cursor", &marker)?;

    let fallback = cargo_bin_cmd!("dispatch")
        .current_dir(&source)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .args(["--state-dir"])
        .arg(&state)
        .args([
            "run",
            "Perform an intentionally unsupported morphology task",
        ])
        .write_stdin("y\n")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let fallback = String::from_utf8(fallback)?;
    assert!(fallback.contains("Agent\n  Cursor"), "{fallback}");
    assert!(
        fallback.contains("Selection\n  Only available agent"),
        "{fallback}"
    );
    assert!(!fallback.contains("benchmark success"));
    assert_eq!(fs::read_to_string(&marker)?, "cursor\n");
    cargo_bin_cmd!("dispatch")
        .current_dir(&source)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .args(["--state-dir"])
        .arg(&state)
        .arg("explain")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "Selection basis\n  Only available agent",
        ))
        .stdout(predicates::str::contains(
            "no performance comparison was possible",
        ));

    let second_state = temp.path().join("override-state");
    agent(&bin.join("codex"), "codex", &marker)?;
    let override_output = cargo_bin_cmd!("dispatch")
        .current_dir(&source)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .args(["--state-dir"])
        .arg(&second_state)
        .args(["run", "Fix the greeting typo", "--agent", "cursor"])
        .write_stdin("y\n")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let override_output = String::from_utf8(override_output)?;
    assert!(override_output.contains("Selection\n  Agent override"));
    assert!(fs::read_to_string(&marker)?.ends_with("cursor\n"));
    Ok(())
}

#[test]
fn refreshed_public_evidence_beats_the_default_agent_order() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("repo");
    let state = temp.path().join("state");
    let bin = temp.path().join("bin");
    let marker = temp.path().join("agents-ran");
    repository(&source)?;
    agent(&bin.join("codex"), "codex", &marker)?;
    agent(&bin.join("cursor-agent"), "cursor", &marker)?;
    let snapshot = PublicPriorSnapshotV1::from_priors(vec![
        public_fixture("codex", 4, 10),
        public_fixture("cursor", 9, 10),
    ])?;
    let (cloud_url, server) = serve_snapshot(snapshot.to_json_bytes()?);

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["data", "refresh"])
        .env("DISPATCH_CLOUD_URL", &cloud_url)
        .assert()
        .success();
    server.join().unwrap();

    let output = cargo_bin_cmd!("dispatch")
        .current_dir(&source)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("DISPATCH_CLOUD_URL", &cloud_url)
        .args(["--state-dir"])
        .arg(&state)
        .args(["run", "Fix the greeting typo"])
        .write_stdin("y\n")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output = String::from_utf8(output)?;
    assert!(output.contains("Agent\n  Cursor"), "{output}");
    assert!(output.contains("Selection\n  Evidence-based"), "{output}");
    assert_eq!(fs::read_to_string(marker)?, "cursor\n");
    Ok(())
}

#[test]
fn latest_project_diff_accept_reject_and_explain_need_no_run_id() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("repo");
    let state = temp.path().join("state");
    let bin = temp.path().join("bin");
    let marker = temp.path().join("agents-ran");
    repository(&source)?;
    agent(&bin.join("codex"), "codex", &marker)?;
    let base = || {
        let mut command = cargo_bin_cmd!("dispatch");
        command
            .current_dir(&source)
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .args(["--state-dir"])
            .arg(&state);
        command
    };

    base()
        .args(["run", "Fix the greeting typo"])
        .write_stdin("y\n")
        .assert()
        .success();
    base()
        .arg("diff")
        .assert()
        .success()
        .stdout(predicates::str::contains("agent-work.txt"));
    base()
        .arg("status")
        .assert()
        .success()
        .stdout(predicates::str::contains("Fix the greeting typo"));
    base()
        .arg("explain")
        .assert()
        .success()
        .stdout(predicates::str::contains("Task classification"))
        .stdout(predicates::str::contains("Harbor"));
    base()
        .arg("accept")
        .assert()
        .success()
        .stdout(predicates::str::contains("Applied Candidate"))
        .stdout(predicates::str::contains("routing-observation").not());
    assert!(source.join("agent-work.txt").is_file());

    fs::remove_file(source.join("agent-work.txt"))?;
    base()
        .args(["run", "Fix another typo", "--agent", "codex"])
        .write_stdin("y\n")
        .assert()
        .success();
    base()
        .arg("reject")
        .assert()
        .success()
        .stdout(predicates::str::contains("Result rejected"))
        .stdout(predicates::str::contains("routing-observation").not());
    assert!(!source.join("agent-work.txt").exists());
    base()
        .arg("status")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "Result\n  Rejected; source tree unchanged",
        ));
    Ok(())
}

#[test]
fn default_help_leads_with_task_completion_commands() -> anyhow::Result<()> {
    let output = cargo_bin_cmd!("dispatch")
        .arg("--help")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let help = String::from_utf8(output)?;
    assert!(help.contains("give a software task to the best available coding agent"));
    for command in [
        "run", "status", "diff", "accept", "reject", "history", "explain",
    ] {
        assert!(help.contains(command), "help omits {command}: {help}");
    }
    assert!(!help.contains("routing observations"));
    Ok(())
}

fn public_fixture(harness: &str, successes: u64, attempts: u64) -> BenchmarkPrior {
    BenchmarkPrior {
        source: "trusted-public-fixture".into(),
        dataset: "fixture-benchmark".into(),
        dataset_version: "2".into(),
        harness: harness.into(),
        model: Some(format!("fixture/{harness}-model")),
        language: None,
        task_kind: TaskKind::Unknown,
        scope: TaskScope::Unknown,
        successes,
        attempts,
        updated_at: Utc.with_ymd_and_hms(2026, 9, 4, 0, 0, 0).unwrap(),
    }
}

// Real adapters with local executable fixtures: only the selected process writes
// the marker. A fresh normalized cache replaces the bundled fixture independently.
fn selection_case(
    eligible: &[&str],
    priors: Vec<BenchmarkPrior>,
) -> anyhow::Result<(RoutingDecision, String, String)> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("repo");
    let state = temp.path().join("state");
    let bin = temp.path().join("bin");
    let marker = temp.path().join("agents-ran");
    repository(&source)?;
    for id in eligible {
        let executable = if *id == "cursor" { "cursor-agent" } else { id };
        agent(&bin.join(executable), id, &marker)?;
    }
    // Neither run nor explain may reach the optional Cloud endpoint.
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let cloud_url = format!("http://{}", listener.local_addr()?);
    let mut database = Database::open(state.join("dispatch.db"))?;
    database.replace_distributed_public_priors(
        &PublicPriorSnapshotV1::from_priors(vec![public_fixture("unsupported-agent", 1, 1)])?,
        "downloaded",
    )?;
    for prior in priors {
        database.upsert_benchmark_prior(&prior)?;
    }
    let base = || {
        let mut command = cargo_bin_cmd!("dispatch");
        command
            .current_dir(&source)
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("DISPATCH_CLOUD_URL", &cloud_url)
            .arg("--state-dir")
            .arg(&state);
        command
    };
    let output = base()
        .args(["run", "Fix the greeting typo", "--allow-unsafe-local"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let explain = base()
        .arg("explain")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let run = metadata(&state)?;
    let decision: RoutingDecision = serde_json::from_value(run["routing"].clone())?;
    assert_eq!(run["candidates"].as_array().unwrap().len(), 1);
    assert_eq!(
        run["candidates"][0]["harness_id"],
        decision.selected_harness
    );
    assert_eq!(
        fs::read_to_string(marker)?,
        format!("{}\n", decision.selected_harness)
    );
    assert!(!source.join("agent-work.txt").exists());
    assert!(!state.join("datasets").exists());
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );

    // The selection seam must preserve Router predictions, including unknowns
    // and tie order, rather than inventing its own scores or comparisons.
    let predictions = rank_harnesses(
        &database,
        &decision.task_features,
        &eligible
            .iter()
            .map(|id| (*id).to_owned())
            .collect::<Vec<_>>(),
    )?;
    assert_eq!(decision.alternatives.len(), predictions.len());
    for (alternative, prediction) in decision.alternatives.iter().zip(&predictions) {
        assert_eq!(alternative.harness, prediction.harness);
        assert_eq!(alternative.successes, prediction.successes);
        assert_eq!(alternative.attempts, prediction.attempts);
        assert_eq!(
            alternative.specificity,
            prediction.evidence.as_ref().map(|e| e.specificity)
        );
    }
    if decision.selection_basis == dispatch::SelectionBasis::Evidence {
        assert_eq!(decision.selected_harness, predictions[0].harness);
    }
    Ok((
        decision,
        String::from_utf8(output)?,
        String::from_utf8(explain)?,
    ))
}

#[test]
fn comparative_selection_preserves_router_order_and_ties() -> anyhow::Result<()> {
    for (cursor_successes, expected) in [(8, "cursor"), (4, "codex")] {
        let (decision, output, explain) = selection_case(
            &["codex", "cursor"],
            vec![
                public_fixture("codex", 4, 10),
                public_fixture("cursor", cursor_successes, 10),
            ],
        )?;
        assert_eq!(decision.selected_harness, expected);
        assert!(output.contains("Selection\n  Evidence-based"));
        assert!(explain.contains("Selection basis\n  Evidence-based"));
        assert!(explain.contains("fixture-benchmark"));
    }
    Ok(())
}

#[test]
fn one_scored_agent_cannot_determine_default_selection() -> anyhow::Result<()> {
    for scored in ["cursor", "codex"] {
        let (decision, output, explain) =
            selection_case(&["codex", "cursor"], vec![public_fixture(scored, 9, 10)])?;
        assert_eq!(decision.selected_harness, "codex");
        assert_eq!(decision.selection_basis, dispatch::SelectionBasis::Default);
        assert_eq!(
            decision.attempts, 0,
            "default is not a benchmark prediction"
        );
        for text in [&output, &explain] {
            assert!(text.contains("Dispatch default"), "{text}");
            assert!(
                text.contains("not have enough comparable public performance data"),
                "{text}"
            );
            assert!(text.contains("did not determine the selection"), "{text}");
            assert!(text.contains("9/10"));
            assert!(text.contains("no compatible public evidence"));
            assert!(!text.contains("0/0"));
            assert!(!text.contains("Evidence-based"));
        }
        assert!(explain.contains("trusted-public-fixture"));
        assert!(explain.contains("fixture-benchmark"));
    }
    Ok(())
}

#[test]
fn unusable_and_unavailable_evidence_cannot_justify_comparison() -> anyhow::Result<()> {
    let mut incompatible = public_fixture("codex", 10, 10);
    incompatible.language = Some("python".into());
    for unusable in [
        public_fixture("codex", 0, 0),
        incompatible,
        public_fixture("claude", 10, 10),
    ] {
        let (decision, output, _) = selection_case(
            &["codex", "cursor"],
            vec![public_fixture("cursor", 9, 10), unusable],
        )?;
        assert_eq!(decision.selected_harness, "codex");
        assert!(output.contains("Selection\n  Dispatch default"));
        assert_eq!(
            decision
                .alternatives
                .iter()
                .filter(|p| p.attempts > 0)
                .count(),
            1
        );
    }
    Ok(())
}

#[test]
fn no_scored_agents_use_default_without_fabricated_scores() -> anyhow::Result<()> {
    let (decision, output, explain) = selection_case(
        &["codex", "cursor"],
        vec![public_fixture("fake-good", 10, 10)],
    )?;
    assert_eq!(decision.selected_harness, "codex");
    for text in [&output, &explain] {
        assert!(text.contains("Dispatch default"));
        assert!(text.contains("no compatible public evidence"));
        assert!(!text.contains("0/0"));
        assert!(!text.contains("Evidence-based"));
    }
    Ok(())
}

#[test]
fn sole_available_agent_is_not_a_performance_comparison() -> anyhow::Result<()> {
    let (decision, output, explain) = selection_case(
        &["codex"],
        vec![
            public_fixture("codex", 9, 10),
            public_fixture("cursor", 10, 10),
        ],
    )?;
    assert_eq!(decision.selected_harness, "codex");
    assert_eq!(decision.selection_basis, dispatch::SelectionBasis::Default);
    assert_eq!(decision.alternatives.len(), 1);
    for text in [&output, &explain] {
        assert!(text.contains("Only available agent"), "{text}");
        assert!(text.contains("no performance comparison was possible"));
        assert!(!text.contains("Evidence-based"));
        assert!(!text.contains("Cursor"));
    }
    Ok(())
}

#[test]
fn comparative_subset_does_not_claim_unknown_agent_is_inferior() -> anyhow::Result<()> {
    let (decision, output, explain) = selection_case(
        &["claude", "codex", "cursor"],
        vec![
            public_fixture("codex", 4, 10),
            public_fixture("cursor", 8, 10),
        ],
    )?;
    assert_eq!(decision.selected_harness, "cursor");
    for text in [&output, &explain] {
        assert!(text.contains("Evidence-based"));
        assert!(text.contains("Claude Code: no compatible public evidence"));
        assert!(
            text.contains("not compared; their performance is unknown"),
            "{text}"
        );
        assert!(!text.contains("among the agents available for this task"));
    }
    Ok(())
}
