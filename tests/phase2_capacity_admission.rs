#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use serde_json::Value;

fn executable(path: &Path, contents: &str) -> Result<()> {
    fs::write(path, contents)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn source(root: &Path, name: &str, agent: &Path, check: &Path) -> Result<PathBuf> {
    let source = root.join(name);
    fs::create_dir_all(source.join("src"))?;
    fs::write(source.join("src/lib.rs"), "pub fn original() {}\n")?;
    fs::write(
        source.join("dispatch.yml"),
        format!(
            "execution:\n  timeout_secs: 10\nchecks:\n  baseline: ['true']\n  verify:\n    - {}\nharnesses:\n  codex:\n    executable: \"{}\"\n",
            check.display(),
            agent.display()
        ),
    )?;
    Ok(source)
}

fn resources(agent_probe: bool, admission: bool) -> String {
    format!(
        "version: 1\nallocation_enabled: true\ncapacity:\n  codex_probe: {agent_probe}\n  admission: {admission}\n  lease_secs: 4\n  heartbeat_secs: 1\n  aging_secs: 2\nprofiles:\n  - provider: openai\n    funding_source: chatgpt-plus\n    harness: codex\n    model: configured-model\n    effort: medium\n    service_mode: standard\n    runtime: local\n    pool: shared-chatgpt-codex\n    provider_buckets: [codex]\n    tier: standard\n    included: true\n    no_overage_verified: true\n    authorization_revision: 1\n"
    )
}

fn probed_agent(
    path: &Path,
    account_id: &str,
    model_started: &Path,
    model_sleep_secs: u64,
) -> Result<()> {
    executable(
        path,
        &format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"--version\" ]; then printf 'codex fixture 2.0\\n'; exit 0; fi\n\
             if [ \"$1\" = \"app-server\" ]; then\n\
             while IFS= read -r line; do\n\
             case \"$line\" in\n\
             *'\"id\":0'*) printf '%s\\n' '{{\"id\":0,\"result\":{{\"userAgent\":\"fixture\"}}}}' ;;\n\
             *'\"id\":1'*) printf '%s\\n' '{{\"id\":1,\"result\":{{\"account\":{{\"type\":\"chatgpt\",\"planType\":\"plus\",\"id\":\"{}\"}}}}}}' ;;\n\
             *'\"id\":2'*) printf '%s\\n' '{{\"id\":2,\"result\":{{\"rateLimitsByLimitId\":{{\"codex\":{{\"primary\":{{\"usedPercent\":10}},\"secondary\":{{\"usedPercent\":10}},\"credits\":{{\"hasCredits\":false}}}}}}}}}}' ;;\n\
             esac\n\
             done\n\
             exit 0\n\
             fi\n\
             : > '{}'\n\
             sleep {}\n\
             printf '{{\"type\":\"result\"}}\\n'\n",
            account_id,
            model_started.display(),
            model_sleep_secs,
        ),
    )
}

#[test]
fn two_foreground_processes_share_one_slot_and_checks_do_not_retain_it() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let state = temp.path().join("state");
    fs::create_dir_all(&state)?;
    let model_lock = temp.path().join("model-active");
    let verification_lock = temp.path().join("verification-active");
    let overlap = temp.path().join("model-overlap");
    let model_completed = temp.path().join("model-completed");
    let second_model_started = temp.path().join("second-model-started");
    let agent = temp.path().join("codex-fixture");
    executable(
        &agent,
        &format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"--version\" ]; then printf 'codex fixture 2.0\\n'; exit 0; fi\n\
             if ! mkdir \"{}\" 2>/dev/null; then printf 'overlap\\n' >> \"{}\"; exit 9; fi\n\
             trap 'rmdir \"{}\" 2>/dev/null' EXIT INT TERM\n\
             if [ -f \"{}\" ]; then printf 'yes\\n' >> \"{}\"; fi\n\
             sleep 1\n\
             : > \"{}\"\n\
             printf 'allocated\\n' > allocated.txt\n\
             printf '{{\"type\":\"result\",\"model\":\"fixture\"}}\\n'\n",
            model_lock.display(),
            overlap.display(),
            model_lock.display(),
            model_completed.display(),
            second_model_started.display(),
            model_completed.display(),
        ),
    )?;
    let check = temp.path().join("verify-fixture");
    executable(
        &check,
        &format!(
            "#!/bin/sh\n\
             owned=0\n\
             if mkdir \"{}\" 2>/dev/null; then owned=1; fi\n\
             attempts=0\n\
             while [ ! -f \"{}\" ] && [ \"$attempts\" -lt 80 ]; do attempts=$((attempts + 1)); sleep 0.05; done\n\
             test -f \"{}\"\n\
             sleep 1\n\
             if [ \"$owned\" = 1 ]; then rmdir \"{}\"; fi\n\
             test -f allocated.txt\n",
            verification_lock.display(),
            second_model_started.display(),
            second_model_started.display(),
            verification_lock.display(),
        ),
    )?;
    let first_source = source(temp.path(), "source-one", &agent, &check)?;
    let second_source = source(temp.path(), "source-two", &agent, &check)?;
    fs::write(
        state.join("resources.yml"),
        "version: 1\nallocation_enabled: true\ncapacity:\n  codex_probe: false\n  lease_secs: 4\n  heartbeat_secs: 1\n  aging_secs: 2\nprofiles:\n  - provider: openai\n    funding_source: chatgpt-plus\n    harness: codex\n    model: configured-model\n    effort: medium\n    service_mode: standard\n    runtime: local\n    pool: shared-chatgpt-codex\n    provider_buckets: [codex]\n    tier: standard\n    included: true\n    no_overage_verified: true\n",
    )?;

    let binary = assert_cmd::cargo_bin!("dispatch");
    let launch = |source: &Path| -> Result<std::process::Child> {
        Ok(Command::new(binary.as_os_str())
            .args(["--state-dir"])
            .arg(&state)
            .arg("run")
            .arg(source)
            .args([
                "--task",
                "Implement the deterministic fixture change.",
                "--agent",
                "codex",
                "--model",
                "configured-model",
                "--effort",
                "medium",
                "--allow-unsafe-local",
                "--json",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?)
    };
    let first = launch(&first_source)?;
    let second = launch(&second_source)?;
    let deadline = Instant::now() + Duration::from_secs(8);
    let released_run = loop {
        if state.join("dispatch.db").is_file() && verification_lock.exists() {
            let database = rusqlite::Connection::open(state.join("dispatch.db"))?;
            let released = database
                .query_row(
                    "SELECT a.run_id,r.run_projection_json FROM admission_requests a JOIN runs r ON r.id=a.run_id WHERE a.status='released' ORDER BY a.released_at LIMIT 1",
                    [],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .ok()
                .and_then(|(run_id, projection)| {
                    serde_json::from_str::<Value>(&projection)
                        .ok()
                        .filter(|value| value["admission"]["state"] == "released")
                        .map(|_| run_id)
                });
            if released.is_some() {
                break released;
            }
        }
        if Instant::now() >= deadline {
            break None;
        }
        thread::sleep(Duration::from_millis(25));
    };
    let status = released_run
        .as_ref()
        .map(|released_run| {
            Command::new(binary.as_os_str())
                .args(["--state-dir"])
                .arg(&state)
                .args(["status", released_run, "--json"])
                .output()
        })
        .transpose()?;
    let first = first.wait_with_output()?;
    let second = second.wait_with_output()?;
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let status = status.expect("a lease should release while local verification is active");
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status: Value = serde_json::from_slice(&status.stdout)?;
    assert_eq!(status["admission"]["state"], "released");
    assert!(
        !overlap.exists(),
        "two model children entered the shared pool at once"
    );
    assert!(second_model_started.exists());

    for output in [&first.stdout, &second.stdout] {
        let result: Value = serde_json::from_slice(output)?;
        assert_eq!(result["capacity"]["scarcity"], "unknown");
        assert_eq!(result["admission"]["state"], "released");
    }
    let database = rusqlite::Connection::open(state.join("dispatch.db"))?;
    let leases: i64 =
        database.query_row("SELECT COUNT(*) FROM pool_leases", [], |row| row.get(0))?;
    let observations: i64 = database.query_row(
        "SELECT COUNT(DISTINCT payload_json) FROM capacity_observations",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(leases, 0);
    assert_eq!(observations, 2);
    Ok(())
}

#[test]
fn queued_process_rechecks_newer_conflicting_evidence_from_another_process() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let state = temp.path().join("state");
    fs::create_dir_all(&state)?;
    fs::write(state.join("resources.yml"), resources(true, true))?;
    let check = temp.path().join("check");
    executable(&check, "#!/bin/sh\nexit 0\n")?;

    let holder_started = temp.path().join("holder-started");
    let waiter_started = temp.path().join("waiter-started");
    let conflicting_started = temp.path().join("conflicting-started");
    for directory in ["holder-agent", "waiter-agent", "conflicting-agent"] {
        fs::create_dir_all(temp.path().join(directory))?;
    }
    let holder_agent = temp.path().join("holder-agent/codex");
    let waiter_agent = temp.path().join("waiter-agent/codex");
    let conflicting_agent = temp.path().join("conflicting-agent/codex");
    probed_agent(&holder_agent, "account-a", &holder_started, 0)?;
    // Hold the scarce slot until the test has published conflicting evidence;
    // a fixed sleep can expire before the waiter queues on a loaded machine.
    let release = temp.path().join("release-holder");
    struct ReleaseOnDrop(PathBuf);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            let _ = fs::write(&self.0, "");
        }
    }
    let release_guard = ReleaseOnDrop(release.clone());
    let script = fs::read_to_string(&holder_agent)?.replace(
        "sleep 0",
        &format!(
            "i=0; while [ ! -f '{}' ] && [ \"$i\" -lt 600 ]; do sleep 0.05; i=$((i+1)); done",
            release.display()
        ),
    );
    fs::write(&holder_agent, script)?;
    probed_agent(&waiter_agent, "account-a", &waiter_started, 0)?;
    probed_agent(&conflicting_agent, "account-b", &conflicting_started, 0)?;
    let holder_source = source(temp.path(), "source-holder", &holder_agent, &check)?;
    let waiter_source = source(temp.path(), "source-waiter", &waiter_agent, &check)?;
    let conflicting_source = source(
        temp.path(),
        "source-conflicting",
        &conflicting_agent,
        &check,
    )?;
    let binary = assert_cmd::cargo_bin!("dispatch");
    let launch = |source: &Path| -> Result<std::process::Child> {
        Ok(Command::new(binary.as_os_str())
            .args(["--state-dir"])
            .arg(&state)
            .arg("run")
            .arg(source)
            .args([
                "--task",
                "Run the deterministic evidence fixture.",
                "--agent",
                "codex",
                "--model",
                "configured-model",
                "--effort",
                "medium",
                "--allow-unsafe-local",
                "--json",
                "--timeout",
                "30",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?)
    };

    let holder = launch(&holder_source)?;
    let deadline = Instant::now() + Duration::from_secs(8);
    while !holder_started.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(25));
    }
    assert!(
        holder_started.exists(),
        "holder did not acquire the shared slot"
    );
    let waiter = launch(&waiter_source)?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let queued = loop {
        let found = rusqlite::Connection::open(state.join("dispatch.db"))
            .ok()
            .and_then(|database| {
                database
                    .query_row(
                        "SELECT COUNT(*) FROM admission_requests WHERE status='queued'",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .ok()
            })
            .is_some_and(|count| count > 0);
        if found || Instant::now() >= deadline {
            break found;
        }
        thread::sleep(Duration::from_millis(25));
    };
    assert!(queued, "waiter did not enter shared admission");

    let conflicting = launch(&conflicting_source)?.wait_with_output()?;
    assert!(!conflicting.status.success());
    assert!(
        String::from_utf8_lossy(&conflicting.stderr).contains("funding revalidation failed"),
        "{}",
        String::from_utf8_lossy(&conflicting.stderr)
    );
    drop(release_guard);
    let holder = holder.wait_with_output()?;
    let waiter = waiter.wait_with_output()?;
    assert!(
        holder.status.success(),
        "{}",
        String::from_utf8_lossy(&holder.stderr)
    );
    assert!(!waiter.status.success());
    assert!(
        String::from_utf8_lossy(&waiter.stderr).contains("funding revalidation failed"),
        "stderr: {}\nstdout: {}",
        String::from_utf8_lossy(&waiter.stderr),
        String::from_utf8_lossy(&waiter.stdout)
    );
    assert!(
        !waiter_started.exists(),
        "stale waiter launched after shared conflict"
    );
    assert!(
        !conflicting_started.exists(),
        "conflicting evidence process must not launch"
    );
    Ok(())
}

#[test]
fn timed_out_optional_probe_keeps_a_normal_run_usable_and_explained() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let state = temp.path().join("state");
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("src"))?;
    fs::create_dir_all(&state)?;
    fs::write(source.join("src/lib.rs"), "pub fn original() {}\n")?;
    let agent = temp.path().join("codex");
    executable(
        &agent,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'codex fixture 2.0\\n'; exit 0; fi\nif [ \"$1\" = \"app-server\" ]; then sleep 2; exit 0; fi\nprintf 'allocated\\n' > allocated.txt\nprintf '{\"type\":\"result\"}\\n'\n",
    )?;
    fs::write(
        source.join("dispatch.yml"),
        format!(
            "execution:\n  timeout_secs: 5\nchecks:\n  verify: []\nharnesses:\n  codex:\n    executable: \"{}\"\n",
            agent.display()
        ),
    )?;
    fs::write(
        state.join("resources.yml"),
        "version: 1\nallocation_enabled: true\ncapacity:\n  probe_timeout_secs: 1\nprofiles:\n  - provider: openai\n    funding_source: chatgpt-plus\n    harness: codex\n    model: configured-model\n    effort: medium\n    service_mode: standard\n    runtime: local\n    pool: shared-chatgpt-codex\n    provider_buckets: [codex]\n    tier: standard\n    included: true\n    no_overage_verified: true\n",
    )?;
    let output = Command::new(assert_cmd::cargo_bin!("dispatch"))
        .args(["--state-dir"])
        .arg(&state)
        .arg("run")
        .arg(&source)
        .args([
            "--task",
            "Run the deterministic fixture.",
            "--agent",
            "codex",
            "--model",
            "configured-model",
            "--effort",
            "medium",
            "--allow-unsafe-local",
            "--json",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(result["capacity"]["scarcity"], "unknown");
    assert_eq!(result["capacity"]["auth_mode"]["knowledge"], "unknown");
    let raw = Path::new(result["capacity"]["raw_observation_ref"].as_str().unwrap());
    assert!(raw.is_file());
    assert!(fs::read_to_string(raw)?.contains("probe timeout"));
    let run_id = result["run_id"].as_str().unwrap();
    let explanation = Command::new(assert_cmd::cargo_bin!("dispatch"))
        .args(["--state-dir"])
        .arg(&state)
        .args(["explain", run_id])
        .output()?;
    assert!(explanation.status.success());
    let explanation = String::from_utf8(explanation.stdout)?;
    assert!(explanation.contains("Capacity observation"));
    assert!(explanation.contains("probe timeout"));
    Ok(())
}

#[test]
fn reserve_defers_non_urgent_work_without_launching() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let state = temp.path().join("state");
    fs::create_dir_all(&state)?;
    let launched = temp.path().join("launched");
    let agent = temp.path().join("codex");
    executable(
        &agent,
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'codex fixture 2.0\\n'; exit 0; fi\nif [ \"$1\" = \"app-server\" ]; then\nwhile IFS= read -r line; do\ncase \"$line\" in\n*'\"id\":0'*) printf '%s\\n' '{{\"id\":0,\"result\":{{\"userAgent\":\"fixture\"}}}}' ;;\n*'\"id\":1'*) printf '%s\\n' '{{\"id\":1,\"result\":{{\"account\":{{\"type\":\"chatgpt\",\"planType\":\"plus\",\"id\":\"account-a\"}}}}}}' ;;\n*'\"id\":2'*) printf '%s\\n' '{{\"id\":2,\"result\":{{\"rateLimitsByLimitId\":{{\"codex\":{{\"primary\":{{\"usedPercent\":85}},\"secondary\":{{\"usedPercent\":10}},\"credits\":{{\"hasCredits\":false}}}}}}}}}}' ;;\nesac\ndone\nexit 0\nfi\n: > '{}'\nprintf '{{\"type\":\"result\"}}\\n'\n",
            launched.display()
        ),
    )?;
    let check = temp.path().join("check");
    executable(&check, "#!/bin/sh\nexit 0\n")?;
    let source = source(temp.path(), "source-reserve", &agent, &check)?;
    fs::write(state.join("resources.yml"), resources(true, true))?;
    let output = Command::new(assert_cmd::cargo_bin!("dispatch"))
        .args(["--state-dir"])
        .arg(&state)
        .arg("run")
        .arg(&source)
        .args([
            "--task",
            "Optional deterministic fixture.",
            "--agent",
            "codex",
            "--model",
            "configured-model",
            "--effort",
            "medium",
            "--allow-unsafe-local",
            "--json",
        ])
        .output()?;
    assert!(!output.status.success());
    assert!(
        !launched.exists(),
        "reserve work must not launch without explicit urgency"
    );
    let database = rusqlite::Connection::open(state.join("dispatch.db"))?;
    let status: String = database.query_row(
        "SELECT status FROM runs ORDER BY created_at DESC LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(status, "deferred");
    let result: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(result["exit_code"], 5);
    assert_eq!(result["outcome"]["work_result"], "deferred");
    Ok(())
}

#[test]
fn foreground_configuration_changes_respect_overlap_and_allow_disjoint_work() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let state = temp.path().join("state");
    fs::create_dir(&state)?;
    let release = temp.path().join("release-holder");
    let holder_started = temp.path().join("holder-started");
    let expanded_started = temp.path().join("expanded-started");
    let disjoint_started = temp.path().join("disjoint-started");
    let check = temp.path().join("check");
    executable(&check, "#!/bin/sh\nexit 0\n")?;
    let mut sources = Vec::new();
    for (name, marker) in [
        ("holder", &holder_started),
        ("expanded", &expanded_started),
        ("disjoint", &disjoint_started),
    ] {
        let dir = temp.path().join(name);
        fs::create_dir(&dir)?;
        let agent = dir.join("codex");
        probed_agent(&agent, "account-a", marker, 0)?;
        if name == "holder" {
            let script = fs::read_to_string(&agent)?.replace(
                "sleep 0",
                &format!(
                    "while [ ! -f '{}' ]; do sleep 0.05; done",
                    release.display()
                ),
            );
            executable(&agent, &script)?;
        }
        sources.push(source(
            temp.path(),
            &format!("source-{name}"),
            &agent,
            &check,
        )?);
    }
    let launch = |project: &Path| -> Result<std::process::Child> {
        Ok(Command::new(assert_cmd::cargo_bin!("dispatch"))
            .arg("--state-dir")
            .arg(&state)
            .arg("run")
            .arg(project)
            .args([
                "--task",
                "Deterministic overlap fixture",
                "--agent",
                "codex",
                "--model",
                "configured-model",
                "--effort",
                "medium",
                "--allow-unsafe-local",
                "--json",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?)
    };
    fs::write(state.join("resources.yml"), resources(true, true))?;
    let holder = launch(&sources[0])?;
    let deadline = Instant::now() + Duration::from_secs(8);
    while !holder_started.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    let holder_was_running = holder_started.exists();
    fs::write(
        state.join("resources.yml"),
        resources(true, true)
            .replace("shared-chatgpt-codex", "expanded")
            .replace("[codex]", "[codex, additional]"),
    )?;
    let expanded = launch(&sources[1])?;
    let mut queued = false;
    while Instant::now() < deadline {
        queued=rusqlite::Connection::open(state.join("dispatch.db"))?.query_row("SELECT EXISTS(SELECT 1 FROM admission_requests WHERE pool_id='expanded' AND status='queued')",[],|r|r.get::<_,bool>(0))?;
        if queued {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    fs::write(
        state.join("resources.yml"),
        resources(true, true)
            .replace("shared-chatgpt-codex", "disjoint")
            .replace("[codex]", "[independent-other-pool]"),
    )?;
    let disjoint = launch(&sources[2])?.wait_with_output()?;
    let overlapped = expanded_started.exists();
    let independent_ran = disjoint_started.exists();
    fs::write(&release, "release")?;
    let holder = holder.wait_with_output()?;
    let expanded = expanded.wait_with_output()?;
    for output in [&holder, &disjoint] {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(holder_was_running && queued);
    assert!(
        !overlapped,
        "changed mapping overlapped the existing holder"
    );
    assert!(
        independent_ran,
        "disjoint allowance was needlessly serialized"
    );
    assert!(
        !expanded.status.success(),
        "stale queued result: stdout={} stderr={}",
        String::from_utf8_lossy(&expanded.stdout),
        String::from_utf8_lossy(&expanded.stderr)
    );
    assert!(
        String::from_utf8_lossy(&expanded.stderr)
            .contains("resource configuration changed after admission")
    );
    assert!(
        !expanded_started.exists(),
        "removed queued profile must not launch"
    );
    Ok(())
}

#[test]
fn disabling_admission_cannot_bypass_an_active_owner() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let state = temp.path().join("state");
    fs::create_dir_all(&state)?;
    let active = temp.path().join("active");
    let overlap = temp.path().join("overlap");
    let agent = temp.path().join("codex-fixture");
    executable(
        &agent,
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then exit 0; fi\nif ! mkdir '{}' 2>/dev/null; then : > '{}'; exit 9; fi\ntrap 'rmdir \"{}\"' EXIT INT TERM\nsleep 2\nprintf '{{\"type\":\"result\"}}\\n'\n",
            active.display(),
            overlap.display(),
            active.display()
        ),
    )?;
    let check = temp.path().join("check");
    executable(&check, "#!/bin/sh\nexit 0\n")?;
    let first_source = source(temp.path(), "source-enabled", &agent, &check)?;
    let second_source = source(temp.path(), "source-disabled", &agent, &check)?;
    fs::write(state.join("resources.yml"), resources(false, true))?;
    let binary = assert_cmd::cargo_bin!("dispatch");
    let first = Command::new(binary.as_os_str())
        .args(["--state-dir"])
        .arg(&state)
        .arg("run")
        .arg(&first_source)
        .args([
            "--task",
            "Hold the shared model slot.",
            "--agent",
            "codex",
            "--model",
            "configured-model",
            "--effort",
            "medium",
            "--allow-unsafe-local",
            "--json",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(8);
    while !active.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(25));
    }
    assert!(active.exists(), "first coordinated owner did not start");
    fs::write(state.join("resources.yml"), resources(false, false))?;
    let second = Command::new(binary.as_os_str())
        .args(["--state-dir"])
        .arg(&state)
        .arg("run")
        .arg(&second_source)
        .args([
            "--task",
            "Must not bypass admission.",
            "--agent",
            "codex",
            "--model",
            "configured-model",
            "--effort",
            "medium",
            "--allow-unsafe-local",
            "--json",
        ])
        .output()?;
    assert!(!second.status.success());
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("shared admission cannot be disabled")
    );
    assert!(!overlap.exists());
    let first = first.wait_with_output()?;
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    Ok(())
}
