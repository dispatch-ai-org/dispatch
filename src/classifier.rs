use std::{collections::BTreeSet, ffi::OsStr, path::Path};

use anyhow::{Context, Result};
use walkdir::{DirEntry, WalkDir};

use crate::{TaskFeatures, TaskKind, TaskScope, source};

const IGNORED_DIRECTORIES: [&str; 9] = [
    "target",
    "build",
    "dist",
    "node_modules",
    "vendor",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
];

#[derive(Default)]
struct SourceEvidence {
    rust: usize,
    python: usize,
    go: usize,
    c: usize,
    cpp_implementations: usize,
    cpp_headers: usize,
    files: BTreeSet<String>,
}

/// Derive conservative routing features from local pre-execution inputs.
pub fn classify_task(source_path: &Path, task: &str) -> Result<TaskFeatures> {
    let source_path = source::resolve_source(Some(source_path))?;
    let evidence = inspect_source(&source_path)?;
    Ok(TaskFeatures {
        language: primary_language(&evidence),
        task_kind: classify_task_kind(task),
        scope: classify_scope(&evidence.files, task),
    })
}

fn inspect_source(root: &Path) -> Result<SourceEvidence> {
    let mut evidence = SourceEvidence::default();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(include_entry)
    {
        let entry = entry.with_context(|| format!("failed to walk {}", root.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .expect("walked path is under source");
        if let Some(path) = portable_path(relative) {
            evidence.files.insert(path);
        }
        let Some(extension) = entry.path().extension().and_then(OsStr::to_str) else {
            continue;
        };
        match extension.to_ascii_lowercase().as_str() {
            "rs" => evidence.rust += 1,
            "py" => evidence.python += 1,
            "go" => evidence.go += 1,
            // .h is ambiguous; uppercase .C conventionally denotes C++, not C.
            "c" if extension == "c" => evidence.c += 1,
            "cpp" | "cc" | "cxx" => evidence.cpp_implementations += 1,
            "h" | "hpp" | "hh" | "hxx" => evidence.cpp_headers += 1,
            _ => {}
        }
    }
    Ok(evidence)
}

fn include_entry(entry: &DirEntry) -> bool {
    source::include_entry(entry)
        && (entry.depth() == 0
            || !entry.file_type().is_dir()
            || !IGNORED_DIRECTORIES
                .iter()
                .any(|name| entry.file_name() == OsStr::new(name)))
}

fn portable_path(path: &Path) -> Option<String> {
    path.components()
        .map(|component| component.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()
        .map(|parts| parts.join("/"))
}

fn primary_language(evidence: &SourceEvidence) -> Option<String> {
    let cpp = if evidence.cpp_implementations > 0 {
        evidence.cpp_implementations + evidence.cpp_headers
    } else {
        0
    };
    let mut languages = [
        ("rust", evidence.rust),
        ("python", evidence.python),
        ("go", evidence.go),
        ("c", evidence.c),
        ("cpp", cpp),
    ];
    languages.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(right.0)));
    let (language, count) = languages[0];
    let runner_up = languages[1].1;
    (count >= 2
        && count.saturating_sub(runner_up) >= 2
        && (runner_up == 0 || count >= runner_up.saturating_mul(2)))
    .then(|| language.to_owned())
}

fn classify_task_kind(task: &str) -> TaskKind {
    let task = task.to_ascii_lowercase();
    let words = task
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let words = if words.first().is_some_and(|word| *word == "please") {
        &words[1..]
    } else {
        &words
    };

    // Explicit test-writing intent can include a small count and a test category.
    // Inspect only the leading object; later verification instructions are not intent.
    let mut object = words.get(1..).unwrap_or_default();
    object = object.strip_prefix(&["yet"]).unwrap_or(object);
    if object.first().is_some_and(|w| matches!(*w, "a" | "an")) {
        object = &object[1..];
    }
    if object.first().is_some_and(|w| {
        matches!(
            *w,
            "one" | "two" | "three" | "another" | "additional" | "second" | "third"
        )
    }) {
        object = &object[1..];
    }
    if object.first().is_some_and(|w| matches!(*w, "new" | "more")) {
        object = &object[1..];
    }
    if object
        .first()
        .is_some_and(|w| matches!(*w, "unit" | "regression" | "collision"))
    {
        object = &object[1..];
    }
    let writing_tests = words.first().is_some_and(|w| matches!(*w, "add" | "write"))
        && object
            .first()
            .is_some_and(|w| matches!(*w, "test" | "tests"));

    if writing_tests || starts_with_any(words, &[&["increase", "test", "coverage"]]) {
        TaskKind::Tests
    } else if starts_with_any(words, &[&["fix"], &["resolve"], &["correct"]]) {
        TaskKind::BugFix
    } else if starts_with_any(
        words,
        &[
            &["add", "support", "for"],
            &["implement", "a", "new"],
            &["introduce"],
        ],
    ) || [
        &["add", "another"][..],
        &["add", "yet", "another"][..],
        &["add", "one", "more"][..],
        &["add", "a", "new"][..],
        &["add", "a", "second"][..],
        &["add", "a", "third"][..],
    ]
    .iter()
    .any(|phrase| words.starts_with(phrase) && words.len() > phrase.len())
    {
        TaskKind::Feature
    } else if starts_with_any(
        words,
        &[
            &["refactor"],
            &["restructure"],
            &["rename", "and", "reorganize"],
        ],
    ) {
        TaskKind::Refactor
    } else {
        TaskKind::Unknown
    }
}

fn starts_with_any(words: &[&str], phrases: &[&[&str]]) -> bool {
    phrases.iter().any(|phrase| words.starts_with(phrase))
}

fn classify_scope(files: &BTreeSet<String>, task: &str) -> TaskScope {
    let tokens = task
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|character: char| {
                !character.is_alphanumeric() && !matches!(character, '/' | '\\' | '.' | '_' | '-')
            })
        })
        .collect::<BTreeSet<_>>();
    match files
        .iter()
        .filter(|path| tokens.contains(path.as_str()))
        .count()
    {
        0 => TaskScope::Unknown,
        1 => TaskScope::Localized,
        _ => TaskScope::MultiFile,
    }
}
