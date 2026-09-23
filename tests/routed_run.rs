use std::{
    fs,
    path::{Path, PathBuf},
};

use assert_cmd::cargo_bin_cmd;
use dispatch::db::Database;
use serde_json::Value;

fn source(root: &Path) -> PathBuf {
    let source = root.join("source");
    for relative in ["src/lib.rs", "src/main.rs", "tests/retry.rs"] {
        let path = source.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "source\n").unwrap();
    }
    source
}

#[cfg(unix)]
fn executable(path: &Path, marker: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::write(
        path,
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"--version\" ]; then\n\
               printf 'cursor fixture 1.0\\n'\n\
               exit 0\n\
             fi\n\
             printf 'run\\n' >> '{}'\n\
             printf 'routed\\n' > cursor-routed.txt\n\
             printf '{{\"type\":\"result\",\"model\":\"fixture-cursor\"}}\\n'\n",
            marker.display()
        ),
    )?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn configure(source: &Path, cursor: &Path, codex: &Path, claude: &Path) -> anyhow::Result<()> {
    configure_with_check(
        source,
        cursor,
        codex,
        claude,
        Some("test -f cursor-routed.txt"),
    )
}

fn configure_with_check(
    source: &Path,
    cursor: &Path,
    codex: &Path,
    claude: &Path,
    check: Option<&str>,
) -> anyhow::Result<()> {
    let checks = check.map_or_else(
        || "checks:\n  verify: []\n".to_owned(),
        |check| format!("checks:\n  verify:\n    - {check}\n"),
    );
    fs::write(
        source.join("dispatch.yml"),
        format!(
            r#"execution:
  timeout_secs: 2
  max_parallel: 3
{checks}harnesses:
  claude:
    executable: "{}"
  codex:
    executable: "{}"
  cursor:
    executable: "{}"
"#,
            claude.display(),
            codex.display(),
            cursor.display()
        ),
    )?;
    Ok(())
}

fn only_metadata(state: &Path) -> anyhow::Result<(PathBuf, Value)> {
    let paths = fs::read_dir(state.join("runs"))?
        .map(|entry| entry.map(|entry| entry.path().join("metadata.json")))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(paths.len(), 1);
    let path = paths.into_iter().next().unwrap();
    let value = serde_json::from_slice(&fs::read(&path)?)?;
    Ok((path, value))
}

#[test]
fn explicit_non_routed_run_creates_no_routing_observation() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = source(temp.path());
    let state = temp.path().join("state");

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .arg("run")
        .arg(&source)
        .args([
            "--task",
            "Exercise the explicit workflow.",
            "--harnesses",
            "fake-good",
        ])
        .assert()
        .success();

    let (_, metadata) = only_metadata(&state)?;
    let database = Database::open(state.join("dispatch.db"))?;
    assert!(
        database
            .routing_observation_for_run(metadata["id"].as_str().unwrap())?
            .is_none()
    );
    let run_id = metadata["id"].as_str().unwrap();
    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["evaluate", run_id, "--outcome", "accept"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "This run was not predictively routed.\nUse `dispatch compare",
        ));
    Ok(())
}

#[cfg(unix)]
#[test]
fn explicit_agent_run_requires_the_unsafe_local_acknowledgement() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = source(temp.path());
    let state = temp.path().join("state");
    let marker = temp.path().join("cursor-invocations");
    let cursor = temp.path().join("cursor-fixture");
    let missing = temp.path().join("not-installed");
    executable(&cursor, &marker)?;
    configure(&source, &cursor, &missing, &missing)?;

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .arg("run")
        .arg(&source)
        .args(["--task", "Fix the retry race.", "--agent", "cursor"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "explicitly accept the risk with --allow-unsafe-local",
        ));

    assert!(!marker.exists());
    let runs = state.join("runs");
    assert!(!runs.exists() || fs::read_dir(&runs)?.next().is_none());
    Ok(())
}
