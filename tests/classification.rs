use std::{fs, path::Path};

use chrono::{TimeZone, Utc};
use dispatch::{
    BenchmarkPrior, TaskFeatures, TaskKind, TaskScope, classifier::classify_task, db::Database,
    router::rank_harnesses,
};

fn write(root: &Path, relative: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, "source\n").unwrap();
}

fn repository(files: &[&str]) -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    for file in files {
        write(temp.path(), file);
    }
    temp
}

fn classify_text(text: &str) -> TaskFeatures {
    let source = repository(&["README.md"]);
    classify_task(source.path(), text).unwrap()
}

#[test]
fn classifies_an_obviously_rust_repository() -> anyhow::Result<()> {
    let source = repository(&["src/lib.rs", "src/main.rs", "tests/smoke.rs"]);

    let features = classify_task(source.path(), "Improve the documentation")?;

    assert_eq!(features.language.as_deref(), Some("rust"));
    Ok(())
}

#[test]
fn classifies_an_obviously_python_repository() -> anyhow::Result<()> {
    let source = repository(&["app.py", "package/api.py", "tests/test_api.py"]);

    let features = classify_task(source.path(), "Improve the documentation")?;

    assert_eq!(features.language.as_deref(), Some("python"));
    Ok(())
}

#[test]
fn classifies_an_obviously_go_repository() -> anyhow::Result<()> {
    let source = repository(&["main.go", "worker.go", "worker_test.go"]);

    let features = classify_task(source.path(), "Improve the documentation")?;

    assert_eq!(features.language.as_deref(), Some("go"));
    Ok(())
}

#[test]
fn classifies_an_obviously_cpp_repository() -> anyhow::Result<()> {
    let source = repository(&["src/main.cpp", "src/widget.cc", "include/widget.h"]);

    let features = classify_task(source.path(), "Improve the documentation")?;

    assert_eq!(features.language.as_deref(), Some("cpp"));
    Ok(())
}

#[test]
fn ambiguous_language_evidence_remains_unknown() -> anyhow::Result<()> {
    let source = repository(&[
        "src/lib.rs",
        "src/main.rs",
        "src/extra.rs",
        "app.py",
        "worker.py",
    ]);

    let features = classify_task(source.path(), "Improve the documentation")?;

    assert_eq!(features.language, None);
    Ok(())
}

#[test]
fn build_vendor_and_cache_artifacts_do_not_dominate_language() -> anyhow::Result<()> {
    let source = repository(&[
        "app.py",
        "worker.py",
        "target/generated_1.rs",
        "target/generated_2.rs",
        "build/generated_3.rs",
        "vendor/generated_4.rs",
        ".cache/generated_5.rs",
        "node_modules/generated_6.rs",
    ]);

    let features = classify_task(source.path(), "Improve the documentation")?;

    assert_eq!(features.language.as_deref(), Some("python"));
    Ok(())
}

#[test]
fn clear_repair_text_is_a_bug_fix() {
    assert_eq!(
        classify_text("Fix the pagination bug").task_kind,
        TaskKind::BugFix
    );
    assert_eq!(
        classify_text("Please resolve the race condition").task_kind,
        TaskKind::BugFix
    );
    assert_eq!(
        classify_text("Correct retry behavior").task_kind,
        TaskKind::BugFix
    );
}

#[test]
fn clear_feature_text_is_a_feature() {
    assert_eq!(
        classify_text("Add support for signed archives").task_kind,
        TaskKind::Feature
    );
    assert_eq!(
        classify_text("Implement a new export format").task_kind,
        TaskKind::Feature
    );
    assert_eq!(
        classify_text("Introduce structured logging").task_kind,
        TaskKind::Feature
    );
}

#[test]
fn clear_refactor_text_is_a_refactor() {
    assert_eq!(
        classify_text("Refactor the persistence layer").task_kind,
        TaskKind::Refactor
    );
    assert_eq!(
        classify_text("Restructure the command modules").task_kind,
        TaskKind::Refactor
    );
    assert_eq!(
        classify_text("Rename and reorganize the adapters").task_kind,
        TaskKind::Refactor
    );
}

#[test]
fn clear_test_writing_text_is_tests() {
    assert_eq!(
        classify_text("Add tests for retry behavior").task_kind,
        TaskKind::Tests
    );
    assert_eq!(
        classify_text("Write unit tests for pagination").task_kind,
        TaskKind::Tests
    );
    assert_eq!(
        classify_text("Increase test coverage for the parser").task_kind,
        TaskKind::Tests
    );
}

#[test]
fn ambiguous_task_text_remains_unknown() {
    assert_eq!(
        classify_text("Improve pagination behavior").task_kind,
        TaskKind::Unknown
    );
}

#[test]
fn scope_remains_unknown_without_explicit_known_files() -> anyhow::Result<()> {
    let source = repository(&["src/lib.rs", "src/main.rs"]);

    let features = classify_task(source.path(), "Fix the pagination bug")?;

    assert_eq!(features.scope, TaskScope::Unknown);
    Ok(())
}

#[test]
fn explicit_known_file_paths_support_conservative_scope() -> anyhow::Result<()> {
    let source = repository(&["src/lib.rs", "src/main.rs"]);

    let localized = classify_task(source.path(), "Fix src/lib.rs")?;
    let multi_file = classify_task(source.path(), "Refactor src/lib.rs and src/main.rs")?;

    assert_eq!(localized.scope, TaskScope::Localized);
    assert_eq!(multi_file.scope, TaskScope::MultiFile);
    Ok(())
}

#[test]
fn classification_is_deterministic_and_does_not_modify_the_source() -> anyhow::Result<()> {
    let source = repository(&["src/lib.rs", "src/main.rs"]);
    let task = "Fix src/lib.rs";
    let expected = classify_task(source.path(), task)?;

    for _ in 0..5 {
        assert_eq!(classify_task(source.path(), task)?, expected);
    }
    assert_eq!(
        fs::read_to_string(source.path().join("src/lib.rs"))?,
        "source\n"
    );
    Ok(())
}

#[test]
fn classified_features_feed_the_router_and_prefer_specific_evidence() -> anyhow::Result<()> {
    let source = repository(&["src/lib.rs", "src/main.rs", "tests/router.rs"]);
    let features = classify_task(source.path(), "Fix the retry race")?;
    let database = Database::open_in_memory()?;
    let updated_at = Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap();
    for prior in [
        BenchmarkPrior {
            source: "harbor-framework/harbor".to_owned(),
            dataset: "terminal-bench/terminal-bench-2".to_owned(),
            dataset_version: "2.0".to_owned(),
            harness: "codex".to_owned(),
            model: Some("openai/gpt-5".to_owned()),
            language: None,
            task_kind: TaskKind::Unknown,
            scope: TaskScope::Unknown,
            successes: 9,
            attempts: 10,
            updated_at,
        },
        BenchmarkPrior {
            source: "specific-public-evidence".to_owned(),
            dataset: "rust-repairs".to_owned(),
            dataset_version: "v1".to_owned(),
            harness: "codex".to_owned(),
            model: None,
            language: Some("rust".to_owned()),
            task_kind: TaskKind::BugFix,
            scope: TaskScope::Unknown,
            successes: 1,
            attempts: 2,
            updated_at,
        },
    ] {
        database.upsert_benchmark_prior(&prior)?;
    }

    let ranked = rank_harnesses(&database, &features, &["codex".to_owned()])?;

    assert_eq!(
        features,
        TaskFeatures {
            language: Some("rust".to_owned()),
            task_kind: TaskKind::BugFix,
            scope: TaskScope::Unknown,
        }
    );
    assert_eq!(ranked[0].score, Some(0.5));
    assert_eq!(ranked[0].evidence.as_ref().unwrap().specificity, 2);
    assert_eq!(
        ranked[0].evidence.as_ref().unwrap().prior.source,
        "specific-public-evidence"
    );
    Ok(())
}

#[test]
fn c_evidence_preserves_feature_kind_and_known_scope() -> anyhow::Result<()> {
    let source = repository(&["main.c", "tests/collision_test.c"]);
    let features = classify_task(
        source.path(),
        "Add support for a platform in main.c and tests/collision_test.c",
    )?;
    assert_eq!(features.language.as_deref(), Some("c"));
    assert_eq!(features.task_kind, TaskKind::Feature);
    assert_eq!(features.scope, TaskScope::MultiFile);
    let unspecified = classify_task(source.path(), "Change the simulation")?;
    assert_eq!(unspecified.language.as_deref(), Some("c"));
    assert_eq!(unspecified.task_kind, TaskKind::Unknown);
    assert_eq!(unspecified.scope, TaskScope::Unknown);
    Ok(())
}

#[test]
fn weak_ambiguous_and_generated_c_evidence_stays_unknown() -> anyhow::Result<()> {
    for files in [
        vec!["main.c"],
        vec!["main.c", "library.h", "other.h"],
        vec!["library.h", "other.h"],
        vec![
            "main.c",
            "build/generated.c",
            "vendor/dependency.c",
            ".cache/cached.c",
        ],
        vec!["main.c", "test.c", "widget.cpp"],
        vec!["main.c", "test.c", "app.py", "worker.py"],
        vec!["main.C", "test.C"],
    ] {
        let source = repository(&files);
        assert_eq!(
            classify_task(source.path(), "Add tests in main.c")?.language,
            None,
            "{files:?}"
        );
    }
    Ok(())
}

#[test]
fn recorded_real_prompts_preserve_independent_kind_and_scope() -> anyhow::Result<()> {
    // Original pre-execution source inventory, not the delivered diff.
    let source = repository(&[
        "main.c",
        "tests/collision_test.c",
        "verify.sh",
        "build.sh",
        "dispatch.yml",
        "main",
    ]);
    let tests = "Add tests in tests/collision_test.c for two missing collision regression cases: an overlapping ball below a platform already moving downward, and an overlapping ball to the right already moving rightward. Assert that each ball is pushed out to the appropriate face but its velocity stays unchanged because it is moving away. Reuse the existing helpers and keep all current cases. Modify only this test file, run the configured verification, and do not change application behavior, configuration, feedback, or routing policy.";
    let feature = "Add yet another platform. make them equidistant from one another and the border of the sim.";
    for (prompt, kind, scope) in [
        (tests, TaskKind::Tests, TaskScope::Localized),
        (feature, TaskKind::Feature, TaskScope::Unknown),
        (
            "Add two collision tests for balls already moving away.",
            TaskKind::Tests,
            TaskScope::Unknown,
        ),
        (
            "Add a third platform with equal spacing between platforms and screen edges.",
            TaskKind::Feature,
            TaskScope::Unknown,
        ),
    ] {
        let f = classify_task(source.path(), prompt)?;
        assert_eq!(f.language.as_deref(), Some("c"));
        assert_eq!((f.task_kind, f.scope), (kind, scope), "{prompt}");
    }
    Ok(())
}

#[test]
fn additive_wording_does_not_infer_locality_or_confuse_tests_with_features() -> anyhow::Result<()> {
    let source = repository(&["main.c", "tests/collision_test.c"]);
    for prompt in [
        "Please add another platform and space the platforms evenly.",
        "Add one more platform across the simulation. Run tests afterward.",
        "Add a new platform and update every subsystem as needed.",
        "Add a second platform; add tests afterward.",
        "Add another export format across the application",
    ] {
        let f = classify_task(source.path(), prompt)?;
        assert_eq!(
            (f.task_kind, f.scope),
            (TaskKind::Feature, TaskScope::Unknown),
            "{prompt}"
        );
    }
    for prompt in [
        "Add two collision tests in tests/collision_test.c",
        "Please write three regression tests in tests/collision_test.c",
        "Add another test in tests/collision_test.c",
        "Add yet another collision test in tests/collision_test.c",
        "Add a new unit test in tests/collision_test.c",
        "Add a third regression test in tests/collision_test.c",
        "Add one more test in tests/collision_test.c",
        "Add an additional unit test in tests/collision_test.c",
    ] {
        let f = classify_task(source.path(), prompt)?;
        assert_eq!(
            (f.task_kind, f.scope),
            (TaskKind::Tests, TaskScope::Localized),
            "{prompt}"
        );
    }
    let f = classify_task(
        source.path(),
        "Add another platform in main.c and tests/collision_test.c",
    )?;
    assert_eq!(
        (f.task_kind, f.scope),
        (TaskKind::Feature, TaskScope::MultiFile)
    );
    Ok(())
}

#[test]
fn vague_and_cross_cutting_intent_stays_unknown() -> anyhow::Result<()> {
    let source = repository(&["main.c", "tests/collision_test.c"]);
    for prompt in [
        "Make it better",
        "Improve the simulation across all subsystems",
        "Change how everything works",
        "Add something useful",
        "Add whatever is missing",
        "Make the platforms feel right",
        "Add another",
        "Add a third",
        "Consider whether we should add another platform",
        "Do not add another platform",
    ] {
        let f = classify_task(source.path(), prompt)?;
        assert_eq!(
            (f.task_kind, f.scope),
            (TaskKind::Unknown, TaskScope::Unknown),
            "{prompt}"
        );
    }
    Ok(())
}
