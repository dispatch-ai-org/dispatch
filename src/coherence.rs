//! Work coherence: is a run's finished work still valid against a source tree
//! that may have moved since the run's baseline was taken?
//!
//! This is the pure L0 evaluator. It reads the source tree and the candidate
//! patch and runs `git apply` only with `--check`, so it never mutates
//! anything, and it never touches the database or the run record.

use std::{fmt, fs, path::Path};

use anyhow::{Context, Result, bail};
use chrono::Utc;

use crate::{
    AnalysisLevel, Decision, Reason, ReasonCode, RunRecord, Validity,
    coherence::world::WorldObservation,
    config::{AcceptMode, Config},
    source::{fingerprint_tree, git_command, run_git},
};

pub mod symbols;
pub mod world;

const MAX_REASONS: usize = 20;
const MAX_DETAIL_CHARS: usize = 300;

/// What the evaluator needs to know about the work: where the source lives and
/// the patch (the work's delta against its baseline).
pub struct WorkView<'a> {
    pub source: &'a Path,
    pub delta_patch: &'a Path,
}

/// Observe the current source tree and evaluate one candidate's patch of `run`.
pub fn evaluate_run(run: &RunRecord, candidate_label: &str) -> Result<Validity> {
    let matches = run
        .candidates
        .iter()
        .filter(|candidate| candidate.label == candidate_label)
        .collect::<Vec<_>>();
    let candidate = match matches.as_slice() {
        [candidate] => *candidate,
        [] => bail!("candidate not found: {candidate_label}"),
        _ => bail!("candidate label is ambiguous: {candidate_label}"),
    };
    let world = world::observe(
        &run.source_path,
        &run.baseline_path,
        &run.baseline_commit,
        &run.source_kind,
    )?;
    evaluate(
        &world,
        &WorkView {
            source: &run.source_path,
            delta_patch: &candidate.diff_path,
        },
    )
}

/// The typed refusal returned when accepted work is no longer valid against the
/// moved source. It replaces matching on error text to recognize drift.
#[derive(Debug)]
pub struct CoherenceBlocked {
    pub validity: Validity,
}

impl fmt::Display for CoherenceBlocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let verdict = match self.validity.decision {
            Decision::Stop => "STOP",
            _ => "REFRESH",
        };
        write!(f, "source has changed; this work is stale ({verdict})")?;
        for reason in &self.validity.reasons {
            write!(f, ": {}", reason.detail)?;
        }
        write!(f, ". The source was left unchanged.")
    }
}

impl std::error::Error for CoherenceBlocked {}

/// What acceptance may do about a possibly moved source.
pub enum AcceptGate {
    /// Strict mode, or the whole tree is byte-identical to the snapshot: the
    /// existing all-or-nothing fingerprint rules apply unchanged.
    Legacy,
    /// The world moved but the work is still valid; apply against that world.
    Compatible(Validity),
    /// The work is stale; nothing may be accepted or applied.
    Blocked(Validity),
}

/// The accept policy frozen with the run. An unreadable snapshot is treated as
/// strict so that a missing file can only make acceptance more conservative.
pub fn accept_mode(run_dir: &Path) -> AcceptMode {
    fs::read_to_string(run_dir.join("config.snapshot.yml"))
        .ok()
        .and_then(|text| serde_yaml::from_str::<Config>(&text).ok())
        .map_or(AcceptMode::Strict, |config| config.coherence.accept)
}

/// Decide whether `candidate_label` of `run` may be accepted onto the source as
/// it is now. Runs no external work when the tree is unchanged.
pub fn gate(run: &RunRecord, candidate_label: &str, mode: AcceptMode) -> Result<AcceptGate> {
    if mode == AcceptMode::Strict || fingerprint_tree(&run.source_path)? == run.source_fingerprint {
        return Ok(AcceptGate::Legacy);
    }
    let validity = evaluate_run(run, candidate_label)?;
    Ok(match validity.decision {
        Decision::Continue => AcceptGate::Compatible(validity),
        Decision::Refresh | Decision::Stop => AcceptGate::Blocked(validity),
    })
}

/// L0 verdict from file and patch state alone. Order matters: an unchanged
/// world needs no analysis, an empty patch cannot conflict, a patch that
/// already reverses cleanly is present in the source, and a patch that does not
/// apply cleanly needs a refresh.
pub fn evaluate(world: &WorldObservation, work: &WorkView) -> Result<Validity> {
    let world_changed = !world.changes.is_empty();
    // An unchanged world is checked first so that it costs no file or Git work.
    let (decision, mut reasons) = if !world_changed || patch_is_empty(work.delta_patch)? {
        (Decision::Continue, Vec::new())
    } else if apply_check(work, true)?.is_none() {
        let reason = Reason {
            code: ReasonCode::AlreadyApplied,
            fact_id: None,
            path: None,
            detail: "the patch is already present in the source".into(),
        };
        (Decision::Stop, vec![reason])
    } else if let Some(stderr) = apply_check(work, false)? {
        let detail = if stderr.is_empty() {
            "the patch does not apply to the current source".to_owned()
        } else {
            stderr.chars().take(MAX_DETAIL_CHARS).collect()
        };
        let reason = Reason {
            code: ReasonCode::PatchConflict,
            fact_id: None,
            path: None,
            detail,
        };
        (Decision::Refresh, vec![reason])
    } else {
        (Decision::Continue, Vec::new())
    };
    reasons.truncate(MAX_REASONS);
    Ok(Validity {
        decision,
        evaluated_at: Utc::now(),
        world_digest: world.digest.clone(),
        world_changed,
        changed_files: u32::try_from(world.changes.len()).unwrap_or(u32::MAX),
        reasons,
        analysis: AnalysisLevel::FilesOnly,
    })
}

fn patch_is_empty(patch: &Path) -> Result<bool> {
    let metadata = fs::metadata(patch)
        .with_context(|| format!("failed to inspect delta {}", patch.display()))?;
    Ok(metadata.len() == 0)
}

/// Run `git apply --check` (optionally `--reverse`) in the source. `None`
/// means the patch would apply; `Some(stderr)` is Git's trimmed complaint.
fn apply_check(work: &WorkView, reverse: bool) -> Result<Option<String>> {
    let mut command = git_command(work.source);
    command.args(["apply", "--check"]);
    if reverse {
        command.arg("--reverse");
    }
    command
        .args(["--binary", "--whitespace=nowarn", "--"])
        .arg(work.delta_patch);
    let output = run_git(command, None, "failed to check the candidate patch")?;
    if output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, process::Command};

    use tempfile::TempDir;

    use super::*;
    use crate::{
        CandidateRecord, CandidateStatus, CoherenceRecord, DiffStats, FactKind, FactOrigin,
        MustHold, SourceKind,
        source::{SourceSnapshot, create_snapshot},
    };

    struct Fixture {
        root: TempDir,
        source: PathBuf,
        snapshot: SourceSnapshot,
    }

    impl Fixture {
        fn new(git: bool, files: &[(&str, &str)]) -> Self {
            let root = TempDir::new().unwrap();
            let source = root.path().join("source");
            fs::create_dir_all(&source).unwrap();
            let source = fs::canonicalize(source).unwrap();
            for (path, contents) in files {
                write(&source.join(path), contents);
            }
            if git {
                git_in(&source, &["init", "--quiet"]);
                git_in(&source, &["add", "-A"]);
                git_in(&source, &["commit", "--quiet", "-m", "initial"]);
            }
            let snapshot = create_snapshot(&source, &root.path().join("run")).unwrap();
            Self {
                root,
                source,
                snapshot,
            }
        }

        fn write(&self, path: &str, contents: &str) {
            write(&self.source.join(path), contents);
        }

        fn patch(&self, contents: &str) -> PathBuf {
            let path = self.root.path().join("delta.patch");
            fs::write(&path, contents).unwrap();
            path
        }

        fn world(&self) -> WorldObservation {
            world::observe(
                &self.source,
                &self.snapshot.baseline_path,
                &self.snapshot.baseline_commit,
                &self.snapshot.kind,
            )
            .unwrap()
        }

        fn evaluate(&self, patch: &Path) -> Validity {
            evaluate(
                &self.world(),
                &WorkView {
                    source: &self.source,
                    delta_patch: patch,
                },
            )
            .unwrap()
        }

        fn run(&self, patch: PathBuf) -> RunRecord {
            let candidate = CandidateRecord {
                id: "c1".into(),
                label: "A".into(),
                harness_id: "codex".into(),
                harness_version: None,
                model: None,
                status: CandidateStatus::Completed,
                workspace_path: self.root.path().join("workspace"),
                prompt_path: PathBuf::new(),
                stdout_path: PathBuf::new(),
                stderr_path: PathBuf::new(),
                diff_path: patch,
                duration_ms: 0,
                exit_code: Some(0),
                timed_out: false,
                tokens: None,
                token_semantics: None,
                cost_usd: None,
                error: None,
                diff_stats: DiffStats::default(),
                checks: Vec::new(),
            };
            let mut run = old_run_record();
            run.source_path = self.source.clone();
            run.source_kind = self.snapshot.kind.clone();
            run.baseline_path = self.snapshot.baseline_path.clone();
            run.baseline_commit = self.snapshot.baseline_commit.clone();
            run.candidates = vec![candidate];
            run
        }
    }

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn git_in(path: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
            ])
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// The JSON shape of a run.json written before `coherence` existed.
    const OLD_RUN_JSON: &str = r#"{
        "id":"01TEST", "task":"Fix cache", "exact_prompt":"Fix cache",
        "source_path":"/source", "source_kind":"directory", "source_git_head":null,
        "source_fingerprint":"baseline", "baseline_path":"/baseline", "baseline_commit":"abc",
        "status":"running", "created_at":"2026-09-17T00:00:00Z", "completed_at":null,
        "environment":{"dispatch_version":"test","os":"test","architecture":"test","execution_backend":"local","timeout_secs":30,"cpus":1.0,"memory":"1g","max_parallel":1},
        "evaluation":null,"applied_candidate":null
    }"#;

    fn old_run_record() -> RunRecord {
        serde_json::from_str(OLD_RUN_JSON).unwrap()
    }

    const A_TXT: &str = "one\ntwo\nthree\nfour\nfive\n";
    const PATCH_A: &str =
        "--- a/a.txt\n+++ b/a.txt\n@@ -1,5 +1,5 @@\n one\n-two\n+TWO\n three\n four\n five\n";

    fn base_files() -> Vec<(&'static str, &'static str)> {
        vec![
            (".gitignore", "target/\n"),
            ("a.txt", A_TXT),
            ("b.txt", "b\n"),
        ]
    }

    fn reason_codes(validity: &Validity) -> Vec<ReasonCode> {
        validity.reasons.iter().map(|reason| reason.code).collect()
    }

    /// Mutates the source after the baseline snapshot.
    type Mutate = fn(&Fixture);

    struct Case {
        name: &'static str,
        git: bool,
        mutate: Mutate,
        patch: &'static str,
        decision: Decision,
        reasons: &'static [ReasonCode],
        world_changed: bool,
    }

    #[test]
    fn l0_verdict_table() {
        let cases = [
            Case {
                name: "unrelated file change continues",
                git: true,
                mutate: |f| f.write("b.txt", "b edited\n"),
                patch: PATCH_A,
                decision: Decision::Continue,
                reasons: &[],
                world_changed: true,
            },
            Case {
                name: "untracked new file continues",
                git: true,
                mutate: |f| f.write("new.txt", "new\n"),
                patch: PATCH_A,
                decision: Decision::Continue,
                reasons: &[],
                world_changed: true,
            },
            Case {
                name: "ignored build directory change is not drift",
                git: true,
                mutate: |f| f.write("target/out.bin", "built\n"),
                patch: PATCH_A,
                decision: Decision::Continue,
                reasons: &[],
                world_changed: false,
            },
            Case {
                name: "conflicting hunk refreshes",
                git: true,
                mutate: |f| f.write("a.txt", "one\ntwo changed elsewhere\nthree\nfour\nfive\n"),
                patch: PATCH_A,
                decision: Decision::Refresh,
                reasons: &[ReasonCode::PatchConflict],
                world_changed: true,
            },
            Case {
                name: "identical patch already applied stops",
                git: true,
                mutate: |f| f.write("a.txt", "one\nTWO\nthree\nfour\nfive\n"),
                patch: PATCH_A,
                decision: Decision::Stop,
                reasons: &[ReasonCode::AlreadyApplied],
                world_changed: true,
            },
            Case {
                name: "empty patch continues",
                git: true,
                mutate: |f| f.write("a.txt", "one\ntwo changed elsewhere\nthree\nfour\nfive\n"),
                patch: "",
                decision: Decision::Continue,
                reasons: &[],
                world_changed: true,
            },
            Case {
                name: "deleted target file refreshes",
                git: true,
                mutate: |f| fs::remove_file(f.source.join("a.txt")).unwrap(),
                patch: PATCH_A,
                decision: Decision::Refresh,
                reasons: &[ReasonCode::PatchConflict],
                world_changed: true,
            },
            Case {
                name: "directory source unrelated change continues",
                git: false,
                mutate: |f| f.write("b.txt", "b edited\n"),
                patch: PATCH_A,
                decision: Decision::Continue,
                reasons: &[],
                world_changed: true,
            },
            Case {
                name: "directory source conflict refreshes",
                git: false,
                mutate: |f| f.write("a.txt", "one\ntwo changed elsewhere\nthree\nfour\nfive\n"),
                patch: PATCH_A,
                decision: Decision::Refresh,
                reasons: &[ReasonCode::PatchConflict],
                world_changed: true,
            },
            Case {
                name: "directory source already applied stops",
                git: false,
                mutate: |f| f.write("a.txt", "one\nTWO\nthree\nfour\nfive\n"),
                patch: PATCH_A,
                decision: Decision::Stop,
                reasons: &[ReasonCode::AlreadyApplied],
                world_changed: true,
            },
        ];
        for case in cases {
            let fixture = Fixture::new(case.git, &base_files());
            let patch = fixture.patch(case.patch);
            (case.mutate)(&fixture);
            let validity = fixture.evaluate(&patch);
            assert_eq!(validity.decision, case.decision, "{}", case.name);
            assert_eq!(reason_codes(&validity), case.reasons, "{}", case.name);
            assert_eq!(validity.world_changed, case.world_changed, "{}", case.name);
            assert_eq!(validity.analysis, AnalysisLevel::FilesOnly, "{}", case.name);
        }
    }

    #[test]
    fn unchanged_world_continues_without_running_git_apply() {
        for git in [true, false] {
            let fixture = Fixture::new(git, &base_files());
            // Git apply on a missing patch would fail, so any call would show.
            let missing = fixture.root.path().join("missing.patch");
            let validity = fixture.evaluate(&missing);
            assert_eq!(validity.decision, Decision::Continue);
            assert!(!validity.world_changed);
            assert_eq!(validity.changed_files, 0);
            assert!(validity.reasons.is_empty());
            assert_eq!(validity.world_digest, fixture.world().digest);
        }
    }

    #[test]
    fn conflict_reason_carries_bounded_git_detail() {
        let fixture = Fixture::new(true, &base_files());
        let patch = fixture.patch(PATCH_A);
        fixture.write("a.txt", "one\ntwo changed elsewhere\nthree\nfour\nfive\n");
        let validity = fixture.evaluate(&patch);
        let detail = &validity.reasons[0].detail;
        assert!(detail.contains("a.txt"), "{detail}");
        assert!(detail.chars().count() <= MAX_DETAIL_CHARS);
        assert_eq!(validity.changed_files, 1);
    }

    #[test]
    fn evaluation_never_mutates_the_source() {
        let fixture = Fixture::new(true, &base_files());
        let patch = fixture.patch(PATCH_A);
        fixture.write("b.txt", "b edited\n");
        let before = crate::source::fingerprint_tree(&fixture.source).unwrap();
        assert_eq!(fixture.evaluate(&patch).decision, Decision::Continue);
        assert_eq!(
            crate::source::fingerprint_tree(&fixture.source).unwrap(),
            before
        );
        assert_eq!(
            fs::read_to_string(fixture.source.join("a.txt")).unwrap(),
            A_TXT
        );
    }

    #[test]
    fn evaluate_run_observes_and_selects_the_candidate() {
        let fixture = Fixture::new(true, &base_files());
        let patch = fixture.patch(PATCH_A);
        let run = fixture.run(patch);
        fixture.write("a.txt", "one\nTWO\nthree\nfour\nfive\n");
        let validity = evaluate_run(&run, "A").unwrap();
        assert_eq!(validity.decision, Decision::Stop);
        assert!(evaluate_run(&run, "B").is_err());
        assert!(matches!(run.source_kind, SourceKind::Git));
    }

    #[test]
    fn old_run_json_deserializes_without_coherence() {
        assert!(old_run_record().coherence.is_none());
    }

    #[test]
    fn coherence_record_round_trips() {
        let fixture = Fixture::new(true, &base_files());
        let patch = fixture.patch(PATCH_A);
        fixture.write("a.txt", "one\ntwo changed elsewhere\nthree\nfour\nfive\n");
        let record = CoherenceRecord {
            version: 1,
            refreshed_from: Some("01OLD".into()),
            facts: vec![MustHold {
                id: "abc123".into(),
                kind: FactKind::Signature,
                path: "a.txt".into(),
                subject: "auth::validate".into(),
                origin: FactOrigin::Referenced,
                sig_fp: "sig".into(),
                full_fp: None,
                display: "fn validate()".into(),
            }],
            validity: Some(fixture.evaluate(&patch)),
            first_invalid_at: Some(Utc::now()),
        };
        let json = serde_json::to_string(&record).unwrap();
        assert!(json.contains("\"decision\":\"refresh\""));
        assert!(json.contains("\"patch_conflict\""));
        assert!(json.contains("\"files_only\""));
        assert_eq!(
            serde_json::from_str::<CoherenceRecord>(&json).unwrap(),
            record
        );

        let mut run = old_run_record();
        run.coherence = Some(record.clone());
        let back: RunRecord = serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
        assert_eq!(back.coherence, Some(record));

        let defaulted: CoherenceRecord = serde_json::from_str("{}").unwrap();
        assert_eq!(defaulted.version, 1);
        assert!(defaulted.facts.is_empty() && defaulted.validity.is_none());
    }
}
