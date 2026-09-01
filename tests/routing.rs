use chrono::{DateTime, TimeZone, Utc};
use dispatch::{
    BenchmarkPrior, TaskFeatures, TaskKind, TaskScope, db::Database, router::rank_harnesses,
};

fn at(second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 31, 12, 0, second).unwrap()
}

fn features() -> TaskFeatures {
    TaskFeatures {
        language: Some("rust".to_owned()),
        task_kind: TaskKind::BugFix,
        scope: TaskScope::Localized,
    }
}

fn prior(harness: &str, model: Option<&str>, successes: u64, attempts: u64) -> BenchmarkPrior {
    evidence_prior(
        "public-benchmark",
        "swe-bench",
        harness,
        model,
        Some("rust"),
        TaskKind::BugFix,
        TaskScope::Localized,
        successes,
        attempts,
    )
}

#[allow(clippy::too_many_arguments)]
fn evidence_prior(
    source: &str,
    dataset: &str,
    harness: &str,
    model: Option<&str>,
    language: Option<&str>,
    task_kind: TaskKind,
    scope: TaskScope,
    successes: u64,
    attempts: u64,
) -> BenchmarkPrior {
    BenchmarkPrior {
        source: source.to_owned(),
        dataset: dataset.to_owned(),
        dataset_version: "verified-2026-08".to_owned(),
        harness: harness.to_owned(),
        model: model.map(str::to_owned),
        language: language.map(str::to_owned),
        task_kind,
        scope,
        successes,
        attempts,
        updated_at: at(1),
    }
}

#[test]
fn cached_prior_round_trips_with_distinct_identity_and_provenance() -> anyhow::Result<()> {
    let database = Database::open_in_memory()?;
    let cached = prior("codex", Some("gpt-5"), 7, 10);

    database.upsert_benchmark_prior(&cached)?;

    let matches = database.matching_benchmark_priors(&features(), "codex")?;
    assert_eq!(matches, vec![cached]);
    assert_eq!(matches[0].harness, "codex");
    assert_eq!(matches[0].model.as_deref(), Some("gpt-5"));
    assert_eq!(matches[0].source, "public-benchmark");
    assert_eq!(matches[0].dataset, "swe-bench");
    assert_eq!(matches[0].dataset_version, "verified-2026-08");
    Ok(())
}

#[test]
fn cached_prior_upsert_replaces_observations_for_the_same_identity() -> anyhow::Result<()> {
    let database = Database::open_in_memory()?;
    let original = prior("codex", None, 2, 4);
    let mut updated = original.clone();
    updated.successes = 8;
    updated.attempts = 10;
    updated.updated_at = at(2);

    database.upsert_benchmark_prior(&original)?;
    database.upsert_benchmark_prior(&updated)?;

    assert_eq!(
        database.matching_benchmark_priors(&features(), "codex")?,
        vec![updated]
    );
    Ok(())
}

#[test]
fn prior_lookup_requires_exact_task_morphology_and_harness() -> anyhow::Result<()> {
    let database = Database::open_in_memory()?;
    let cached = prior("codex", None, 3, 5);
    database.upsert_benchmark_prior(&cached)?;

    assert_eq!(
        database.matching_benchmark_priors(&features(), "codex")?,
        vec![cached]
    );
    assert!(
        database
            .matching_benchmark_priors(
                &TaskFeatures {
                    task_kind: TaskKind::Feature,
                    ..features()
                },
                "codex",
            )?
            .is_empty()
    );
    assert!(
        database
            .matching_benchmark_priors(
                &TaskFeatures {
                    scope: TaskScope::Broad,
                    ..features()
                },
                "codex",
            )?
            .is_empty()
    );
    assert!(
        database
            .matching_benchmark_priors(
                &TaskFeatures {
                    language: None,
                    ..features()
                },
                "codex",
            )?
            .is_empty()
    );
    assert!(
        database
            .matching_benchmark_priors(&features(), "cursor")?
            .is_empty()
    );
    Ok(())
}

#[test]
fn router_ranks_higher_observed_success_ratio_first() -> anyhow::Result<()> {
    let database = Database::open_in_memory()?;
    database.upsert_benchmark_prior(&prior("codex", None, 9, 10))?;
    database.upsert_benchmark_prior(&prior("cursor", None, 3, 5))?;
    let available = vec!["cursor".to_owned(), "unseen".to_owned(), "codex".to_owned()];

    let ranked = rank_harnesses(&database, &features(), &available)?;

    assert_eq!(ranked[0].harness, "codex");
    assert_eq!(ranked[0].score, Some(0.9));
    assert_eq!(ranked[1].harness, "cursor");
    assert_eq!(ranked[1].score, Some(0.6));
    assert_eq!(ranked[2].harness, "unseen");
    assert_eq!(ranked[2].score, None);
    Ok(())
}

#[test]
fn router_prefers_exact_evidence_over_better_supported_generic_evidence() -> anyhow::Result<()> {
    let database = Database::open_in_memory()?;
    database.upsert_benchmark_prior(&evidence_prior(
        "generic-source",
        "terminal-bench",
        "codex",
        None,
        None,
        TaskKind::Unknown,
        TaskScope::Unknown,
        90,
        100,
    ))?;
    database.upsert_benchmark_prior(&evidence_prior(
        "exact-source",
        "rust-bugs",
        "codex",
        None,
        Some("rust"),
        TaskKind::BugFix,
        TaskScope::Localized,
        1,
        2,
    ))?;

    let ranked = rank_harnesses(&database, &features(), &["codex".to_owned()])?;

    assert_eq!((ranked[0].successes, ranked[0].attempts), (1, 2));
    assert_eq!(ranked[0].score, Some(0.5));
    let evidence = ranked[0].evidence.as_ref().unwrap();
    assert_eq!(evidence.specificity, 3);
    assert_eq!(evidence.prior.source, "exact-source");
    Ok(())
}

#[test]
fn known_task_falls_back_to_generic_evidence_with_explicit_specificity() -> anyhow::Result<()> {
    let database = Database::open_in_memory()?;
    database.upsert_benchmark_prior(&evidence_prior(
        "harbor-framework/harbor",
        "terminal-bench/terminal-bench-2",
        "codex",
        Some("openai/gpt-5"),
        None,
        TaskKind::Unknown,
        TaskScope::Unknown,
        74,
        89,
    ))?;

    let ranked = rank_harnesses(&database, &features(), &["codex".to_owned()])?;

    assert_eq!((ranked[0].successes, ranked[0].attempts), (74, 89));
    assert_eq!(ranked[0].score, Some(74.0 / 89.0));
    let evidence = ranked[0].evidence.as_ref().unwrap();
    assert_eq!(evidence.specificity, 0);
    assert_eq!(evidence.prior.source, "harbor-framework/harbor");
    Ok(())
}

#[test]
fn conflicting_known_dimensions_are_incompatible() -> anyhow::Result<()> {
    let database = Database::open_in_memory()?;
    for prior in [
        evidence_prior(
            "language-conflict",
            "benchmark",
            "codex",
            None,
            Some("python"),
            TaskKind::BugFix,
            TaskScope::Localized,
            1,
            1,
        ),
        evidence_prior(
            "kind-conflict",
            "benchmark",
            "codex",
            None,
            Some("rust"),
            TaskKind::Feature,
            TaskScope::Localized,
            1,
            1,
        ),
        evidence_prior(
            "scope-conflict",
            "benchmark",
            "codex",
            None,
            Some("rust"),
            TaskKind::BugFix,
            TaskScope::Broad,
            1,
            1,
        ),
    ] {
        database.upsert_benchmark_prior(&prior)?;
    }

    let ranked = rank_harnesses(&database, &features(), &["codex".to_owned()])?;

    assert_eq!(ranked[0].score, None);
    assert!(ranked[0].evidence.is_none());
    Ok(())
}

#[test]
fn unknown_task_dimension_does_not_consume_known_prior_value() -> anyhow::Result<()> {
    let database = Database::open_in_memory()?;
    database.upsert_benchmark_prior(&prior("codex", None, 1, 1))?;
    let task = TaskFeatures {
        language: None,
        ..features()
    };

    let ranked = rank_harnesses(&database, &task, &["codex".to_owned()])?;

    assert_eq!(ranked[0].score, None);
    assert!(ranked[0].evidence.is_none());
    Ok(())
}

#[test]
fn unknown_task_and_unknown_prior_dimensions_remain_compatible() -> anyhow::Result<()> {
    let database = Database::open_in_memory()?;
    database.upsert_benchmark_prior(&evidence_prior(
        "generic-source",
        "terminal-bench",
        "codex",
        None,
        None,
        TaskKind::Unknown,
        TaskScope::Unknown,
        1,
        2,
    ))?;
    let task = TaskFeatures {
        language: None,
        task_kind: TaskKind::Unknown,
        scope: TaskScope::Unknown,
    };

    let ranked = rank_harnesses(&database, &task, &["codex".to_owned()])?;

    assert_eq!(ranked[0].score, Some(0.5));
    assert_eq!(ranked[0].evidence.as_ref().unwrap().specificity, 0);
    Ok(())
}

#[test]
fn more_specifically_matched_dimensions_win_within_fallback_evidence() -> anyhow::Result<()> {
    let database = Database::open_in_memory()?;
    database.upsert_benchmark_prior(&evidence_prior(
        "one-dimension",
        "benchmark",
        "codex",
        None,
        Some("rust"),
        TaskKind::Unknown,
        TaskScope::Unknown,
        99,
        100,
    ))?;
    database.upsert_benchmark_prior(&evidence_prior(
        "two-dimensions",
        "benchmark",
        "codex",
        None,
        Some("rust"),
        TaskKind::BugFix,
        TaskScope::Unknown,
        1,
        2,
    ))?;

    let ranked = rank_harnesses(&database, &features(), &["codex".to_owned()])?;

    let evidence = ranked[0].evidence.as_ref().unwrap();
    assert_eq!(evidence.specificity, 2);
    assert_eq!(evidence.prior.source, "two-dimensions");
    assert_eq!((ranked[0].successes, ranked[0].attempts), (1, 2));
    Ok(())
}

#[test]
fn evidence_selection_prefers_attempts_then_stable_provenance_without_summing() -> anyhow::Result<()>
{
    let database = Database::open_in_memory()?;
    for prior in [
        evidence_prior(
            "fewer-attempts",
            "benchmark",
            "codex",
            None,
            Some("rust"),
            TaskKind::BugFix,
            TaskScope::Unknown,
            4,
            4,
        ),
        evidence_prior(
            "more-attempts",
            "benchmark",
            "codex",
            None,
            Some("rust"),
            TaskKind::BugFix,
            TaskScope::Unknown,
            1,
            10,
        ),
    ] {
        database.upsert_benchmark_prior(&prior)?;
    }

    let ranked = rank_harnesses(&database, &features(), &["codex".to_owned()])?;
    assert_eq!((ranked[0].successes, ranked[0].attempts), (1, 10));
    assert_eq!(
        ranked[0].evidence.as_ref().unwrap().prior.source,
        "more-attempts"
    );

    let database = Database::open_in_memory()?;
    for prior in [
        evidence_prior(
            "beta-source",
            "benchmark",
            "codex",
            None,
            Some("rust"),
            TaskKind::BugFix,
            TaskScope::Unknown,
            4,
            4,
        ),
        evidence_prior(
            "alpha-source",
            "benchmark",
            "codex",
            None,
            Some("rust"),
            TaskKind::BugFix,
            TaskScope::Unknown,
            1,
            4,
        ),
    ] {
        database.upsert_benchmark_prior(&prior)?;
    }

    let ranked = rank_harnesses(&database, &features(), &["codex".to_owned()])?;
    assert_eq!((ranked[0].successes, ranked[0].attempts), (1, 4));
    assert_eq!(
        ranked[0].evidence.as_ref().unwrap().prior.source,
        "alpha-source"
    );
    Ok(())
}

#[test]
fn router_reports_no_score_for_missing_or_zero_attempt_evidence() -> anyhow::Result<()> {
    let database = Database::open_in_memory()?;
    database.upsert_benchmark_prior(&prior("codex", None, 0, 0))?;
    let available = vec!["cursor".to_owned(), "codex".to_owned()];

    let ranked = rank_harnesses(&database, &features(), &available)?;

    assert_eq!(ranked[0].harness, "codex");
    assert_eq!(ranked[0].attempts, 0);
    assert_eq!(ranked[0].score, None);
    assert!(ranked[0].evidence.is_none());
    assert_eq!(ranked[1].harness, "cursor");
    assert_eq!(ranked[1].attempts, 0);
    assert_eq!(ranked[1].score, None);
    assert!(ranked[1].evidence.is_none());
    Ok(())
}

#[test]
fn router_breaks_equal_score_ties_by_harness_identity() -> anyhow::Result<()> {
    let database = Database::open_in_memory()?;
    database.upsert_benchmark_prior(&prior("zeta", None, 2, 4))?;
    database.upsert_benchmark_prior(&prior("alpha", None, 1, 2))?;
    let available = vec!["zeta".to_owned(), "alpha".to_owned()];

    let ranked = rank_harnesses(&database, &features(), &available)?;

    assert_eq!(
        ranked
            .iter()
            .map(|prediction| prediction.harness.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "zeta"]
    );
    Ok(())
}
