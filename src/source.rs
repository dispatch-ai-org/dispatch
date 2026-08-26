use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, Metadata},
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::Builder;
use walkdir::{DirEntry, WalkDir};

use crate::{
    DiffStats, RunRecord, SourceKind,
    executor::{trusted_host_executable, trusted_host_path},
};

const BASELINE_DIRECTORY: &str = "baseline";
const EXCLUDED_DIRECTORIES: [&str; 2] = [".git", ".dispatch"];
const MAX_CANDIDATE_FILES: u64 = 200_000;
const MAX_CANDIDATE_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_CANDIDATE_TREE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_GIT_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
const GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

/// The immutable, Dispatch-managed representation of a source tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSnapshot {
    pub kind: SourceKind,
    pub git_head: Option<String>,
    pub fingerprint: String,
    pub baseline_path: PathBuf,
    pub baseline_commit: String,
}

/// A summary of an explicitly applied candidate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApplyReport {
    pub candidate_label: String,
    pub files_changed: u64,
}

/// Resolve an explicit source directory, or the current directory when omitted.
pub fn resolve_source(input: Option<&Path>) -> Result<PathBuf> {
    let input = input.unwrap_or_else(|| Path::new("."));
    let source = fs::canonicalize(input)
        .with_context(|| format!("failed to resolve source {}", input.display()))?;
    ensure!(
        fs::metadata(&source)
            .with_context(|| format!("failed to inspect source {}", source.display()))?
            .is_dir(),
        "source is not a directory: {}",
        source.display()
    );
    Ok(source)
}

/// Identify whether a source is an ordinary directory, a Git repository, or a
/// linked Git worktree. The returned commit is `None` for an unborn repository.
pub fn inspect_source(source: &Path) -> Result<(SourceKind, Option<String>)> {
    let source = resolve_source(Some(source))?;

    let mut inside = git_command(&source);
    inside.args(["rev-parse", "--is-inside-work-tree"]);
    let output = inside
        .output()
        .with_context(|| "failed to run Git while inspecting the source")?;
    if !output.status.success() || trim_ascii(&output.stdout) != b"true" {
        return Ok((SourceKind::Directory, None));
    }

    let mut git_dir_command = git_command(&source);
    git_dir_command.args(["rev-parse", "--absolute-git-dir"]);
    let git_dir_output = checked_output(
        git_dir_command,
        "failed to locate the source repository's Git directory",
    )?;
    let git_dir = bytes_to_path(trim_ascii(&git_dir_output.stdout));
    let kind = if git_dir.join("commondir").is_file() {
        SourceKind::GitWorktree
    } else {
        SourceKind::Git
    };

    let mut head_command = git_command(&source);
    head_command.args(["rev-parse", "--verify", "HEAD"]);
    let head_output = head_command
        .output()
        .with_context(|| "failed to query the source repository's HEAD")?;
    let head = if head_output.status.success() {
        Some(
            String::from_utf8(head_output.stdout)
                .context("Git returned a non-UTF-8 HEAD commit")?
                .trim()
                .to_owned(),
        )
    } else {
        None
    };

    Ok((kind, head))
}

/// Freeze the exact current source contents in a private Git repository.
///
/// This deliberately snapshots the working tree rather than merely cloning
/// `HEAD`, so dirty and untracked source files are part of every candidate's
/// common baseline. Git and Dispatch state directories are never copied.
pub fn create_snapshot(source: &Path, run_dir: &Path) -> Result<SourceSnapshot> {
    let source = resolve_source(Some(source))?;
    let projected_run_dir = canonicalize_allow_missing(run_dir)?;
    ensure!(
        !projected_run_dir.starts_with(&source),
        "Dispatch run directory must be outside the source tree: {}",
        run_dir.display()
    );

    let (kind, git_head) = inspect_source(&source)?;
    let fingerprint_before = fingerprint_tree(&source)?;

    fs::create_dir_all(run_dir)
        .with_context(|| format!("failed to create run directory {}", run_dir.display()))?;
    let baseline_path = run_dir.join(BASELINE_DIRECTORY);
    ensure!(
        fs::symlink_metadata(&baseline_path).is_err(),
        "baseline already exists: {}",
        baseline_path.display()
    );

    let staging = Builder::new()
        .prefix(".dispatch-baseline-")
        .tempdir_in(run_dir)
        .with_context(|| format!("failed to create a snapshot in {}", run_dir.display()))?;
    copy_tree_contents(&source, staging.path())?;

    // A source changed while it was being copied is not a trustworthy baseline.
    // Check both ends so a mixed snapshot cannot be accepted silently.
    let fingerprint_after = fingerprint_tree(&source)?;
    let copied_fingerprint = fingerprint_tree(staging.path())?;
    ensure!(
        fingerprint_before == fingerprint_after && fingerprint_before == copied_fingerprint,
        "source changed while Dispatch was creating its baseline; retry the run"
    );

    initialize_internal_repository(staging.path())?;
    let baseline_commit = git_head_at(staging.path())?
        .context("internal baseline repository did not produce a commit")?;

    fs::rename(staging.path(), &baseline_path)
        .with_context(|| format!("failed to finalize baseline {}", baseline_path.display()))?;

    Ok(SourceSnapshot {
        kind,
        git_head,
        fingerprint: fingerprint_before,
        baseline_path,
        baseline_commit,
    })
}

/// Create a complete, independent workspace at the frozen baseline commit.
pub fn create_candidate_workspace(baseline_path: &Path, candidate_dir: &Path) -> Result<PathBuf> {
    let baseline_path = resolve_source(Some(baseline_path))?;
    ensure_internal_repository(&baseline_path)?;
    ensure_repository_clean(&baseline_path, "baseline")?;

    ensure!(
        fs::symlink_metadata(candidate_dir).is_err(),
        "candidate workspace already exists: {}",
        candidate_dir.display()
    );
    let candidate_dir = canonicalize_allow_missing(candidate_dir)?;
    ensure!(
        !candidate_dir.starts_with(&baseline_path),
        "candidate workspace must not be inside the baseline repository"
    );
    if let Some(parent) = candidate_dir.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create candidate parent {}", parent.display()))?;
    }

    let result = (|| {
        let mut clone = plain_git_command();
        clone
            .args(["clone", "--quiet", "--no-hardlinks", "--no-checkout", "--"])
            .arg(&baseline_path)
            .arg(&candidate_dir);
        checked_output(clone, "failed to clone the frozen baseline")?;

        configure_candidate_repository(&candidate_dir)?;
        let mut read_tree = git_command(&candidate_dir);
        read_tree.args(["read-tree", "HEAD"]);
        checked_output(read_tree, "failed to prepare the candidate Git index")?;

        // The clone intentionally has no checkout. Copying the pristine
        // worktree creates the exact source bytes, empty directories, symlink
        // targets, and permissions instead of asking Git to reconstruct the
        // subset of metadata its object model represents.
        copy_tree_contents(&baseline_path, &candidate_dir)?;
        ensure_repository_clean(&candidate_dir, "candidate workspace")?;
        ensure!(
            fingerprint_tree(&baseline_path)? == fingerprint_tree(&candidate_dir)?,
            "candidate workspace does not exactly match the frozen baseline"
        );
        Ok(candidate_dir.clone())
    })();

    if result.is_err() {
        remove_created_directory(&candidate_dir);
    }
    result
}

/// Collect a binary-capable unified patch and deterministic change statistics.
///
/// A temporary Git index is used so untracked files can be represented in the
/// patch without altering the candidate's real index.
pub fn collect_diff(baseline_path: &Path, workspace: &Path, diff_path: &Path) -> Result<DiffStats> {
    let baseline_path = resolve_source(Some(baseline_path))?;
    let workspace = resolve_source(Some(workspace))?;
    ensure_internal_repository(&baseline_path)?;
    ensure_repository_clean(&baseline_path, "baseline")?;

    let projected_diff_path = canonicalize_allow_missing(diff_path)?;
    ensure!(
        !projected_diff_path.starts_with(&workspace),
        "diff artifact must be stored outside the candidate workspace"
    );
    validate_candidate_tree(&workspace)?;

    let untracked_files = list_untracked_files(&baseline_path, &workspace)?;
    let index_directory = Builder::new()
        .prefix("dispatch-index-")
        .tempdir()
        .context("failed to create a temporary Git index")?;
    let index_path = index_directory.path().join("index");

    let mut read_tree = comparison_git_command(&baseline_path, &workspace);
    read_tree
        .env("GIT_INDEX_FILE", &index_path)
        .args(["read-tree", "HEAD"]);
    checked_output(read_tree, "failed to initialize temporary Git index")?;

    let mut add = comparison_git_command(&baseline_path, &workspace);
    add.env("GIT_INDEX_FILE", &index_path)
        .args(["add", "-A", "--", "."])
        .args(dispatch_exclusion_pathspecs());
    checked_output(add, "failed to stage candidate changes for diff collection")?;

    let mut diff = comparison_git_command(&baseline_path, &workspace);
    diff.env("GIT_INDEX_FILE", &index_path).args([
        "diff",
        "--cached",
        "--binary",
        "--full-index",
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        "HEAD",
        "--",
    ]);
    let patch = checked_output(diff, "failed to collect candidate diff")?.stdout;

    let mut numstat = comparison_git_command(&baseline_path, &workspace);
    numstat.env("GIT_INDEX_FILE", &index_path).args([
        "diff",
        "--cached",
        "--numstat",
        "--no-renames",
        "-z",
        "HEAD",
        "--",
    ]);
    let numstat = checked_output(numstat, "failed to collect candidate diff statistics")?;
    let (files_changed, lines_added, lines_removed, paths) = parse_numstat(&numstat.stdout)?;
    let mut changed_files = paths
        .into_iter()
        .map(|path| String::from_utf8_lossy(&path).into_owned())
        .collect::<Vec<_>>();
    changed_files.sort();

    if let Some(parent) = diff_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create diff directory {}", parent.display()))?;
    }
    fs::write(diff_path, patch)
        .with_context(|| format!("failed to write diff artifact {}", diff_path.display()))?;

    Ok(DiffStats {
        files_changed,
        lines_added,
        lines_removed,
        changed_files,
        untracked_files,
    })
}

/// Hash the complete logical source tree, excluding Git and Dispatch state.
/// Content, paths, symlink targets, file kinds, and permission bits are covered;
/// timestamps are intentionally ignored.
pub fn fingerprint_tree(root: &Path) -> Result<String> {
    let root = resolve_source(Some(root))?;
    let mut entries = Vec::new();
    for entry in WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_entry(include_entry)
    {
        let entry = entry.with_context(|| format!("failed to walk {}", root.display()))?;
        if entry.depth() > 0 {
            entries.push(entry.into_path());
        }
    }
    entries.sort_by(|left, right| {
        path_bytes(left.strip_prefix(&root).expect("walked path is under root")).cmp(&path_bytes(
            right
                .strip_prefix(&root)
                .expect("walked path is under root"),
        ))
    });

    let mut hasher = Sha256::new();
    hasher.update(b"dispatch-tree-v1\0");
    let mut buffer = vec![0_u8; 64 * 1024];
    for path in entries {
        let relative = path.strip_prefix(&root).expect("walked path is under root");
        let path_key = path_bytes(relative);
        hash_sized_bytes(&mut hasher, &path_key);

        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("failed to inspect {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            hasher.update(b"l");
            hash_mode(&mut hasher, &metadata);
            let target = fs::read_link(&path)
                .with_context(|| format!("failed to read symlink {}", path.display()))?;
            ensure_symlink_stays_within(&root, &path, &target)?;
            hash_sized_bytes(&mut hasher, &path_bytes(&target));
        } else if metadata.is_dir() {
            hasher.update(b"d");
            hash_mode(&mut hasher, &metadata);
        } else if metadata.is_file() {
            hasher.update(b"f");
            hash_mode(&mut hasher, &metadata);
            hasher.update(metadata.len().to_le_bytes());
            let mut file =
                File::open(&path).with_context(|| format!("failed to read {}", path.display()))?;
            loop {
                let read = file
                    .read(&mut buffer)
                    .with_context(|| format!("failed to read {}", path.display()))?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
        } else {
            bail!(
                "unsupported special file in source tree: {}",
                path.display()
            );
        }
    }

    Ok(hex::encode(hasher.finalize()))
}

/// Apply one stored candidate patch after proving that the original source has
/// not changed since snapshotting. `git apply` performs a complete dry run
/// before the real application, and its default all-or-nothing behavior is kept.
pub fn safe_apply(run: &RunRecord, candidate_label: &str) -> Result<ApplyReport> {
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

    let source = resolve_source(Some(&run.source_path))?;
    let current_fingerprint = fingerprint_tree(&source)?;
    ensure!(
        current_fingerprint == run.source_fingerprint,
        "source has changed since this run was created; refusing to apply candidate {candidate_label}"
    );

    let patch_metadata = fs::metadata(&candidate.diff_path).with_context(|| {
        format!(
            "failed to inspect candidate diff {}",
            candidate.diff_path.display()
        )
    })?;
    ensure!(
        patch_metadata.is_file(),
        "candidate diff is not a file: {}",
        candidate.diff_path.display()
    );
    if patch_metadata.len() == 0 {
        return Ok(ApplyReport {
            candidate_label: candidate_label.to_owned(),
            files_changed: 0,
        });
    }

    let patch_paths = inspect_patch_paths(&source, &candidate.diff_path)?;
    ensure!(
        !patch_paths.is_empty(),
        "candidate diff contains no applicable file changes"
    );
    for path in &patch_paths {
        ensure_safe_patch_path(path)?;
    }

    let mut check = git_command(&source);
    check
        .args(["apply", "--check", "--binary", "--whitespace=nowarn", "--"])
        .arg(&candidate.diff_path);
    checked_output(
        check,
        "candidate cannot be applied cleanly; the source was left unchanged",
    )?;

    // Close the most useful check/apply race window. The real `git apply` also
    // validates every hunk before writing any file.
    ensure!(
        fingerprint_tree(&source)? == run.source_fingerprint,
        "source changed during apply validation; the source was left unchanged"
    );

    let mut apply = git_command(&source);
    apply
        .args(["apply", "--binary", "--whitespace=nowarn", "--"])
        .arg(&candidate.diff_path);
    checked_output(
        apply,
        "candidate application failed; Git did not apply the patch",
    )?;

    Ok(ApplyReport {
        candidate_label: candidate_label.to_owned(),
        files_changed: patch_paths.len() as u64,
    })
}

fn copy_tree_contents(source: &Path, destination: &Path) -> Result<()> {
    let mut directory_permissions = Vec::new();
    for entry in WalkDir::new(source)
        .follow_links(false)
        .into_iter()
        .filter_entry(include_entry)
    {
        let entry = entry.with_context(|| format!("failed to walk {}", source.display()))?;
        if entry.depth() == 0 {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(source)
            .expect("walked path is under source");
        let target = destination.join(relative);
        let metadata = fs::symlink_metadata(entry.path())
            .with_context(|| format!("failed to inspect {}", entry.path().display()))?;

        if metadata.is_dir() {
            match fs::symlink_metadata(&target) {
                Ok(existing) if !existing.is_dir() || existing.file_type().is_symlink() => bail!(
                    "snapshot destination unexpectedly contains {}",
                    target.display()
                ),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    fs::create_dir(&target).with_context(|| {
                        format!("failed to create directory {}", target.display())
                    })?;
                }
                Err(error) => return Err(error.into()),
            }
            directory_permissions.push((entry.depth(), target, metadata.permissions()));
        } else if metadata.is_file() {
            ensure!(
                fs::symlink_metadata(&target).is_err(),
                "snapshot destination unexpectedly contains {}",
                target.display()
            );
            fs::copy(entry.path(), &target).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    entry.path().display(),
                    target.display()
                )
            })?;
            fs::set_permissions(&target, metadata.permissions()).with_context(|| {
                format!("failed to preserve permissions on {}", target.display())
            })?;
        } else if metadata.file_type().is_symlink() {
            ensure!(
                fs::symlink_metadata(&target).is_err(),
                "snapshot destination unexpectedly contains {}",
                target.display()
            );
            let link_target = fs::read_link(entry.path())
                .with_context(|| format!("failed to read symlink {}", entry.path().display()))?;
            create_symlink(&link_target, &target, entry.path())
                .with_context(|| format!("failed to copy symlink {}", entry.path().display()))?;
        } else {
            bail!(
                "unsupported special file in source tree: {}",
                entry.path().display()
            );
        }
    }

    // Apply child directory modes first so restrictive parents cannot prevent
    // the remaining metadata operations.
    directory_permissions.sort_by_key(|(depth, _, _)| std::cmp::Reverse(*depth));
    for (_, path, permissions) in directory_permissions {
        fs::set_permissions(&path, permissions)
            .with_context(|| format!("failed to preserve permissions on {}", path.display()))?;
    }
    Ok(())
}

fn ensure_symlink_stays_within(root: &Path, link: &Path, target: &Path) -> Result<()> {
    use std::path::Component;

    ensure!(
        !target.is_absolute(),
        "source contains an absolute symlink that escapes candidate isolation: {} -> {}",
        link.display(),
        target.display()
    );
    let parent = link
        .parent()
        .context("source symlink has no parent")?
        .strip_prefix(root)
        .context("source symlink is outside the source root")?;
    let mut components = Vec::new();
    for component in parent.join(target).components() {
        match component {
            Component::Normal(value) => components.push(value.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                ensure!(
                    components.pop().is_some(),
                    "source contains a symlink that escapes candidate isolation: {} -> {}",
                    link.display(),
                    target.display()
                );
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!(
                    "source contains a symlink that escapes candidate isolation: {} -> {}",
                    link.display(),
                    target.display()
                );
            }
        }
    }
    Ok(())
}

fn validate_candidate_tree(root: &Path) -> Result<()> {
    let mut files = 0_u64;
    let mut logical_bytes = 0_u64;
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(include_entry)
    {
        let entry =
            entry.with_context(|| format!("failed to walk candidate {}", root.display()))?;
        if entry.depth() == 0 {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path()).with_context(|| {
            format!(
                "failed to inspect candidate path {}",
                entry.path().display()
            )
        })?;
        if metadata.is_dir() {
            continue;
        }
        files = files.saturating_add(1);
        ensure!(
            files <= MAX_CANDIDATE_FILES,
            "candidate contains more than {MAX_CANDIDATE_FILES} files; diff collection was refused"
        );
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(entry.path())
                .with_context(|| format!("failed to read symlink {}", entry.path().display()))?;
            ensure_symlink_stays_within(root, entry.path(), &target)?;
        } else if metadata.is_file() {
            ensure!(
                metadata.len() <= MAX_CANDIDATE_FILE_BYTES,
                "candidate file {} is larger than the {} MiB safety limit",
                entry.path().display(),
                MAX_CANDIDATE_FILE_BYTES / (1024 * 1024)
            );
            logical_bytes = logical_bytes.saturating_add(metadata.len());
            ensure!(
                logical_bytes <= MAX_CANDIDATE_TREE_BYTES,
                "candidate tree exceeds the {} GiB logical-size safety limit",
                MAX_CANDIDATE_TREE_BYTES / (1024 * 1024 * 1024)
            );
        } else {
            bail!(
                "candidate contains unsupported special file: {}",
                entry.path().display()
            );
        }
    }
    Ok(())
}

fn initialize_internal_repository(path: &Path) -> Result<()> {
    let mut init = git_command(path);
    init.args([
        "-c",
        "init.defaultBranch=dispatch-baseline",
        "init",
        "--quiet",
    ]);
    checked_output(init, "failed to initialize internal baseline repository")?;

    install_internal_attributes(path)?;

    for (key, value) in [
        ("user.name", "Dispatch"),
        ("user.email", "dispatch@localhost"),
        ("commit.gpgSign", "false"),
        ("core.autocrlf", "false"),
    ] {
        let mut config = git_command(path);
        config.args(["config", "--local", key, value]);
        checked_output(config, "failed to configure internal baseline repository")?;
    }

    let mut add = git_command(path);
    add.args(["add", "-A", "-f", "--", "."]);
    checked_output(add, "failed to stage internal baseline")?;

    let mut commit = git_command(path);
    commit
        .env("GIT_AUTHOR_NAME", "Dispatch")
        .env("GIT_AUTHOR_EMAIL", "dispatch@localhost")
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_NAME", "Dispatch")
        .env("GIT_COMMITTER_EMAIL", "dispatch@localhost")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .args([
            "commit",
            "--quiet",
            "--no-gpg-sign",
            "--no-verify",
            "--allow-empty",
            "-m",
            "Dispatch baseline",
        ]);
    checked_output(commit, "failed to commit internal baseline")?;
    Ok(())
}

fn configure_candidate_repository(path: &Path) -> Result<()> {
    let mut config = git_command(path);
    config.args(["config", "--local", "core.autocrlf", "false"]);
    checked_output(config, "failed to configure candidate repository")?;
    install_internal_attributes(path)
}

/// The private repositories must describe the literal local files. Higher
/// priority info attributes neutralize source/global clean filters (including
/// LFS), newline normalization, and working-tree encodings that could otherwise
/// make an internal commit differ from the bytes Dispatch froze.
fn install_internal_attributes(path: &Path) -> Result<()> {
    let mut command = git_command(path);
    command.args(["rev-parse", "--absolute-git-dir"]);
    let output = checked_output(command, "failed to locate internal Git directory")?;
    let git_dir = bytes_to_path(trim_ascii(&output.stdout));
    let info = git_dir.join("info");
    fs::create_dir_all(&info).with_context(|| format!("failed to create {}", info.display()))?;
    fs::write(
        info.join("attributes"),
        b"* -text -eol -filter -ident -working-tree-encoding\n",
    )
    .context("failed to disable content filters in internal repository")?;
    Ok(())
}

fn ensure_internal_repository(path: &Path) -> Result<()> {
    let mut command = git_command(path);
    command.args(["rev-parse", "--verify", "HEAD"]);
    checked_output(command, "path is not a usable baseline Git repository")?;
    Ok(())
}

fn ensure_repository_clean(path: &Path, description: &str) -> Result<()> {
    let mut status = git_command(path);
    status.args([
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--ignored=no",
    ]);
    let output = checked_output(status, &format!("failed to inspect {description}"))?;
    ensure!(
        output.stdout.is_empty(),
        "{description} has unexpected changes and cannot be used safely"
    );
    Ok(())
}

fn git_head_at(path: &Path) -> Result<Option<String>> {
    let mut command = git_command(path);
    command.args(["rev-parse", "--verify", "HEAD"]);
    let output = command.output().context("failed to query Git HEAD")?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8(output.stdout)
            .context("Git returned a non-UTF-8 commit identifier")?
            .trim()
            .to_owned(),
    ))
}

fn list_untracked_files(baseline_path: &Path, workspace: &Path) -> Result<Vec<String>> {
    let mut command = comparison_git_command(baseline_path, workspace);
    command
        .args([
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            ".",
        ])
        .args(dispatch_exclusion_pathspecs());
    let output = checked_output(command, "failed to enumerate untracked candidate files")?;
    let mut files = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}

fn inspect_patch_paths(source: &Path, diff_path: &Path) -> Result<Vec<PathBuf>> {
    let mut command = git_command(source);
    command
        .args(["apply", "--numstat", "-z", "--"])
        .arg(diff_path);
    let output = checked_output(command, "candidate diff is not a valid Git patch")?;
    let (_, _, _, paths) = parse_numstat(&output.stdout)?;
    Ok(paths.into_iter().map(|path| bytes_to_path(&path)).collect())
}

fn parse_numstat(bytes: &[u8]) -> Result<(u64, u64, u64, Vec<Vec<u8>>)> {
    let mut files = 0_u64;
    let mut added = 0_u64;
    let mut removed = 0_u64;
    let mut paths = Vec::new();

    for record in bytes.split(|byte| *byte == 0).filter(|r| !r.is_empty()) {
        let mut fields = record.splitn(3, |byte| *byte == b'\t');
        let added_field = fields.next().context("invalid Git numstat output")?;
        let removed_field = fields.next().context("invalid Git numstat output")?;
        let path = fields.next().context("invalid Git numstat output")?;
        ensure!(!path.is_empty(), "invalid empty path in Git numstat output");

        files += 1;
        added += parse_numstat_number(added_field)?;
        removed += parse_numstat_number(removed_field)?;
        paths.push(path.to_vec());
    }
    Ok((files, added, removed, paths))
}

fn parse_numstat_number(value: &[u8]) -> Result<u64> {
    if value == b"-" {
        return Ok(0);
    }
    let value = std::str::from_utf8(value).context("invalid Git numstat number")?;
    value
        .parse::<u64>()
        .with_context(|| format!("invalid Git numstat number: {value}"))
}

fn ensure_safe_patch_path(path: &Path) -> Result<()> {
    ensure!(
        !path.is_absolute(),
        "candidate diff contains an absolute path"
    );
    for component in path.components() {
        match component {
            Component::Normal(value) => ensure!(
                !is_excluded_name(value),
                "candidate diff attempts to modify Dispatch or Git state: {}",
                path.display()
            ),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("candidate diff contains an unsafe path: {}", path.display())
            }
        }
    }
    Ok(())
}

fn dispatch_exclusion_pathspecs() -> [&'static str; 2] {
    [
        ":(exclude,glob).dispatch/**",
        ":(exclude,glob)**/.dispatch/**",
    ]
}

fn include_entry(entry: &DirEntry) -> bool {
    entry.depth() == 0 || !is_excluded_name(entry.file_name())
}

fn is_excluded_name(name: &OsStr) -> bool {
    EXCLUDED_DIRECTORIES
        .iter()
        .any(|excluded| name == OsStr::new(excluded))
}

fn remove_created_directory(path: &Path) {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            let _ = fs::remove_dir_all(path);
        } else {
            let _ = fs::remove_file(path);
        }
    }
}

fn canonicalize_allow_missing(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to resolve current directory")?
            .join(path)
    };

    let mut cursor = absolute.as_path();
    let mut missing = Vec::<OsString>::new();
    loop {
        match fs::canonicalize(cursor) {
            Ok(mut resolved) => {
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = cursor
                    .file_name()
                    .with_context(|| format!("could not resolve destination {}", path.display()))?;
                missing.push(name.to_os_string());
                cursor = cursor
                    .parent()
                    .with_context(|| format!("could not resolve destination {}", path.display()))?;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to resolve destination {}", path.display()));
            }
        }
    }
}

fn plain_git_command() -> Command {
    let git = trusted_host_executable("git").unwrap_or_else(|_| PathBuf::from("/usr/bin/git"));
    let mut command = Command::new(git);
    command.env_clear();
    for name in ["TMPDIR", "SYSTEMROOT"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .env("PATH", trusted_host_path())
        .env("LC_ALL", "C")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}

fn git_command(directory: &Path) -> Command {
    let mut command = plain_git_command();
    command.arg("-C").arg(directory);
    command
}

/// Compare a candidate worktree using only the trusted baseline's repository
/// metadata. Candidate-controlled `.git/config`, hooks, objects, and info
/// attributes must never influence a host-side Git process.
fn comparison_git_command(baseline_path: &Path, workspace: &Path) -> Command {
    let mut command = plain_git_command();
    command
        .arg("--git-dir")
        .arg(baseline_path.join(".git"))
        .arg("--work-tree")
        .arg(workspace)
        .arg("-c")
        .arg("core.hooksPath=/dev/null");
    command
}

fn checked_output(mut command: Command, context: &str) -> Result<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().with_context(|| context.to_owned())?;
    let child_id = child.id();
    let stdout = child.stdout.take().context("Git stdout was not captured")?;
    let stderr = child.stderr.take().context("Git stderr was not captured")?;
    let stdout_reader = thread::spawn(move || read_limited(stdout));
    let stderr_reader = thread::spawn(move || read_limited(stderr));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().with_context(|| context.to_owned())? {
            break status;
        }
        if started.elapsed() >= GIT_COMMAND_TIMEOUT {
            kill_command_group(child_id);
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            bail!(
                "{context}: Git exceeded the {} second safety timeout",
                GIT_COMMAND_TIMEOUT.as_secs()
            );
        }
        thread::sleep(Duration::from_millis(20));
    };
    kill_command_group(child_id);
    let stdout = stdout_reader
        .join()
        .map_err(|_| anyhow::anyhow!("{context}: stdout reader panicked"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| anyhow::anyhow!("{context}: stderr reader panicked"))??;
    ensure!(
        stdout.total <= MAX_GIT_OUTPUT_BYTES as u64,
        "{context}: Git output exceeded the {} MiB safety limit",
        MAX_GIT_OUTPUT_BYTES / (1024 * 1024)
    );
    ensure!(
        stderr.total <= MAX_GIT_OUTPUT_BYTES as u64,
        "{context}: Git error output exceeded the {} MiB safety limit",
        MAX_GIT_OUTPUT_BYTES / (1024 * 1024)
    );
    let output = Output {
        status,
        stdout: stdout.bytes,
        stderr: stderr.bytes,
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            bail!("{context} (Git exited with {})", output.status);
        }
        bail!("{context}: {stderr}");
    }
    Ok(output)
}

struct LimitedOutput {
    bytes: Vec<u8>,
    total: u64,
}

fn read_limited(mut reader: impl Read) -> std::io::Result<LimitedOutput> {
    let mut bytes = Vec::with_capacity(MAX_GIT_OUTPUT_BYTES.min(64 * 1024));
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        let remaining = MAX_GIT_OUTPUT_BYTES.saturating_sub(bytes.len());
        bytes.extend_from_slice(&buffer[..read.min(remaining)]);
    }
    Ok(LimitedOutput { bytes, total })
}

fn kill_command_group(child_id: u32) {
    #[cfg(unix)]
    if let Ok(pgid) = i32::try_from(child_id) {
        // SAFETY: the command was placed in its own process group immediately
        // before spawn; a negative PID targets that group only.
        unsafe {
            libc::kill(-pgid, libc::SIGKILL);
        }
    }
}

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn hash_sized_bytes(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value);
}

#[cfg(unix)]
fn hash_mode(hasher: &mut Sha256, metadata: &Metadata) {
    use std::os::unix::fs::MetadataExt;
    hasher.update((metadata.mode() & 0o7777).to_le_bytes());
}

#[cfg(not(unix))]
fn hash_mode(hasher: &mut Sha256, metadata: &Metadata) {
    hasher.update([u8::from(metadata.permissions().readonly())]);
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(windows)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(unix)]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

#[cfg(windows)]
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(unix)]
fn create_symlink(target: &Path, destination: &Path, _source_link: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, destination).map_err(Into::into)
}

#[cfg(windows)]
fn create_symlink(target: &Path, destination: &Path, source_link: &Path) -> Result<()> {
    let resolved_target = source_link
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(target);
    if resolved_target.is_dir() {
        std::os::windows::fs::symlink_dir(target, destination).map_err(Into::into)
    } else {
        std::os::windows::fs::symlink_file(target, destination).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path, process::Command};

    use chrono::Utc;
    use tempfile::TempDir;

    use super::*;
    use crate::{CandidateRecord, CandidateStatus, EnvironmentRecord, RunStatus};

    fn write(path: &Path, contents: impl AsRef<[u8]>) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn run_git(path: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env("LC_ALL", "C")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    fn initialize_user_repository(path: &Path) {
        run_git(path, &["init", "--quiet"]);
        run_git(path, &["config", "user.name", "Test User"]);
        run_git(path, &["config", "user.email", "test@example.invalid"]);
        run_git(path, &["add", "-A"]);
        run_git(path, &["commit", "--quiet", "-m", "initial"]);
    }

    fn candidate_record(
        label: &str,
        workspace: PathBuf,
        diff: PathBuf,
        stats: DiffStats,
    ) -> CandidateRecord {
        CandidateRecord {
            id: label.to_lowercase(),
            label: label.to_owned(),
            harness_id: "fake".into(),
            harness_version: None,
            model: None,
            status: CandidateStatus::Completed,
            workspace_path: workspace.clone(),
            prompt_path: workspace.join("prompt.txt"),
            stdout_path: workspace.join("stdout.log"),
            stderr_path: workspace.join("stderr.log"),
            diff_path: diff,
            duration_ms: 1,
            exit_code: Some(0),
            timed_out: false,
            tokens: None,
            token_semantics: None,
            cost_usd: None,
            error: None,
            diff_stats: stats,
            checks: Vec::new(),
        }
    }

    fn run_record(
        source: &Path,
        snapshot: &SourceSnapshot,
        candidate: CandidateRecord,
    ) -> RunRecord {
        RunRecord {
            id: "test-run".into(),
            task: "test task".into(),
            exact_prompt: "test prompt".into(),
            source_path: source.to_path_buf(),
            source_kind: snapshot.kind.clone(),
            source_git_head: snapshot.git_head.clone(),
            source_fingerprint: snapshot.fingerprint.clone(),
            baseline_path: snapshot.baseline_path.clone(),
            baseline_commit: snapshot.baseline_commit.clone(),
            status: RunStatus::Evaluated,
            created_at: Utc::now(),
            completed_at: None,
            environment: EnvironmentRecord {
                dispatch_version: "test".into(),
                os: "test".into(),
                architecture: "test".into(),
                execution_backend: "local".into(),
                timeout_secs: 1,
                cpus: 1.0,
                memory: "1g".into(),
                max_parallel: 1,
                docker_image: None,
                resource_limits_enforced: false,
                unsafe_local: false,
                forwarded_env: Vec::new(),
            },
            baseline_checks: Vec::new(),
            candidates: vec![candidate],
            evaluation: None,
            applied_candidate: None,
        }
    }

    #[test]
    fn resolves_and_validates_source_directories() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        assert_eq!(
            resolve_source(Some(&source)).unwrap(),
            fs::canonicalize(&source).unwrap()
        );

        let file = temp.path().join("file");
        write(&file, "not a directory");
        assert!(resolve_source(Some(&file)).is_err());
    }

    #[test]
    fn identifies_plain_git_and_linked_worktree_sources() {
        let temp = TempDir::new().unwrap();
        let plain = temp.path().join("plain");
        fs::create_dir(&plain).unwrap();
        write(&plain.join("file.txt"), "plain\n");
        assert_eq!(
            inspect_source(&plain).unwrap(),
            (SourceKind::Directory, None)
        );

        let repository = temp.path().join("repository");
        fs::create_dir(&repository).unwrap();
        write(&repository.join("file.txt"), "git\n");
        initialize_user_repository(&repository);
        let expected_head = run_git(&repository, &["rev-parse", "HEAD"]);
        assert_eq!(
            inspect_source(&repository).unwrap(),
            (SourceKind::Git, Some(expected_head))
        );

        let worktree = temp.path().join("worktree");
        run_git(
            &repository,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "dispatch-source-test",
                worktree.to_str().unwrap(),
            ],
        );
        assert_eq!(
            inspect_source(&worktree).unwrap().0,
            SourceKind::GitWorktree
        );
    }

    #[test]
    fn snapshot_is_exact_private_and_excludes_state() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        let run = temp.path().join("run");
        fs::create_dir(&source).unwrap();
        write(&source.join("src/main.rs"), "fn main() {}\n");
        write(&source.join("empty/.keep"), "");
        fs::create_dir_all(source.join("actually-empty")).unwrap();
        write(&source.join(".git/private"), "do not copy");
        write(&source.join(".dispatch/state"), "do not copy");

        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            let script = source.join("script.sh");
            write(&script, "#!/bin/sh\n");
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
            symlink("src/main.rs", source.join("main-link")).unwrap();
        }

        let original_fingerprint = fingerprint_tree(&source).unwrap();
        let snapshot = create_snapshot(&source, &run).unwrap();
        assert_eq!(snapshot.kind, SourceKind::Directory);
        assert_eq!(snapshot.fingerprint, original_fingerprint);
        assert_eq!(fingerprint_tree(&source).unwrap(), original_fingerprint);
        assert_eq!(
            fingerprint_tree(&snapshot.baseline_path).unwrap(),
            original_fingerprint
        );
        assert!(!snapshot.baseline_path.join(".git/private").exists());
        assert!(!snapshot.baseline_path.join(".dispatch").exists());
        assert!(snapshot.baseline_path.join(".git").is_dir());
        assert!(snapshot.baseline_path.join("actually-empty").is_dir());
        assert_eq!(
            run_git(&snapshot.baseline_path, &["status", "--porcelain"]),
            ""
        );
        assert_eq!(snapshot.baseline_commit.len(), 40);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(snapshot.baseline_path.join("script.sh"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o111,
                0o111
            );
            assert_eq!(
                fs::read_link(snapshot.baseline_path.join("main-link")).unwrap(),
                Path::new("src/main.rs")
            );
        }
    }

    #[test]
    fn rejects_a_run_directory_nested_in_the_source() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        write(&source.join("file.txt"), "source\n");
        let nested_run = source.join("state/runs/test");

        let error = create_snapshot(&source, &nested_run)
            .unwrap_err()
            .to_string();
        assert!(error.contains("outside the source tree"));
        assert!(!nested_run.exists());
    }

    #[test]
    fn snapshot_of_git_source_uses_dirty_worktree_without_touching_original() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        write(&source.join("tracked.txt"), "committed\n");
        initialize_user_repository(&source);
        let head = run_git(&source, &["rev-parse", "HEAD"]);
        write(&source.join("tracked.txt"), "dirty baseline\n");
        write(&source.join("untracked.txt"), "also baseline\n");
        let status_before = run_git(&source, &["status", "--porcelain"]);

        let snapshot = create_snapshot(&source, &temp.path().join("run")).unwrap();
        assert_eq!(snapshot.kind, SourceKind::Git);
        assert_eq!(snapshot.git_head.as_deref(), Some(head.as_str()));
        assert_eq!(
            fs::read_to_string(snapshot.baseline_path.join("tracked.txt")).unwrap(),
            "dirty baseline\n"
        );
        assert_eq!(
            fs::read_to_string(snapshot.baseline_path.join("untracked.txt")).unwrap(),
            "also baseline\n"
        );
        assert_eq!(run_git(&source, &["status", "--porcelain"]), status_before);
    }

    #[test]
    fn internal_git_keeps_literal_bytes_despite_source_attributes() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        write(&source.join(".gitattributes"), "*.txt text\n");
        write(&source.join("line-endings.txt"), b"one\r\ntwo\r\n");

        let snapshot = create_snapshot(&source, &temp.path().join("run")).unwrap();
        assert_eq!(
            fs::read(snapshot.baseline_path.join("line-endings.txt")).unwrap(),
            b"one\r\ntwo\r\n"
        );
        let workspace = temp.path().join("candidate");
        create_candidate_workspace(&snapshot.baseline_path, &workspace).unwrap();
        assert_eq!(
            fs::read(workspace.join("line-endings.txt")).unwrap(),
            b"one\r\ntwo\r\n"
        );

        write(&workspace.join("line-endings.txt"), b"one\r\nchanged\r\n");
        let diff_path = temp.path().join("candidate.patch");
        let stats = collect_diff(&snapshot.baseline_path, &workspace, &diff_path).unwrap();
        let run = run_record(
            &source,
            &snapshot,
            candidate_record("A", workspace, diff_path, stats),
        );
        safe_apply(&run, "A").unwrap();
        assert_eq!(
            fs::read(source.join("line-endings.txt")).unwrap(),
            b"one\r\nchanged\r\n"
        );
    }

    #[test]
    fn diff_collection_ignores_candidate_controlled_git_configuration() {
        use std::io::Write as _;

        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        write(&source.join("tracked.txt"), "baseline\n");
        let snapshot = create_snapshot(&source, &temp.path().join("run")).unwrap();
        let workspace = temp.path().join("candidate");
        create_candidate_workspace(&snapshot.baseline_path, &workspace).unwrap();

        let sentinel = temp.path().join("candidate-config-executed");
        let mut config = fs::OpenOptions::new()
            .append(true)
            .open(workspace.join(".git/config"))
            .unwrap();
        writeln!(
            config,
            "[filter \"dispatch-owned\"]\n\tclean = touch {}\n\tsmudge = cat\n\trequired = true",
            sentinel.display()
        )
        .unwrap();
        fs::write(
            workspace.join(".git/info/attributes"),
            "*.txt filter=dispatch-owned\n",
        )
        .unwrap();
        write(&workspace.join("tracked.txt"), "candidate\n");

        let diff_path = temp.path().join("candidate.patch");
        let stats = collect_diff(&snapshot.baseline_path, &workspace, &diff_path).unwrap();
        assert_eq!(stats.files_changed, 1);
        assert!(!sentinel.exists());
        assert!(
            fs::read_to_string(diff_path)
                .unwrap()
                .contains("+candidate")
        );
    }

    #[test]
    fn candidate_and_diff_include_untracked_text_binary_and_mode_changes() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        write(&source.join("tracked.txt"), "one\ntwo\n");
        write(&source.join(".gitignore"), "ignored.log\n");
        let snapshot = create_snapshot(&source, &temp.path().join("run")).unwrap();
        let candidate = temp.path().join("candidate");
        create_candidate_workspace(&snapshot.baseline_path, &candidate).unwrap();

        write(&candidate.join("tracked.txt"), "one\nchanged\nthree\n");
        write(&candidate.join("new.txt"), "new line\n");
        write(&candidate.join("image.bin"), [0_u8, 1, 2, 0, 255]);
        write(&candidate.join("ignored.log"), "build output\n");
        write(&candidate.join(".dispatch/internal"), "state\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            let executable = candidate.join("new.txt");
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
            symlink("new.txt", candidate.join("new-link")).unwrap();
        }

        let diff_path = temp.path().join("candidate.patch");
        let stats = collect_diff(&snapshot.baseline_path, &candidate, &diff_path).unwrap();
        let expected_files = if cfg!(unix) { 4 } else { 3 };
        assert_eq!(stats.files_changed, expected_files);
        assert!(stats.lines_added >= 3);
        assert!(stats.lines_removed >= 1);
        assert_eq!(
            stats.untracked_files,
            if cfg!(unix) {
                vec!["image.bin", "new-link", "new.txt"]
            } else {
                vec!["image.bin", "new.txt"]
            }
        );
        let patch = fs::read(&diff_path).unwrap();
        assert!(
            patch
                .windows(b"GIT binary patch".len())
                .any(|w| w == b"GIT binary patch")
        );
        assert!(!String::from_utf8_lossy(&patch).contains("ignored.log"));
        assert!(!String::from_utf8_lossy(&patch).contains(".dispatch"));
        assert_eq!(fingerprint_tree(&source).unwrap(), snapshot.fingerprint);
        assert_eq!(
            run_git(&snapshot.baseline_path, &["status", "--porcelain"]),
            ""
        );
    }

    #[test]
    fn safe_apply_updates_a_plain_source_only_when_it_has_not_drifted() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        write(&source.join("old.txt"), "before\n");
        let snapshot = create_snapshot(&source, &temp.path().join("run")).unwrap();
        let workspace = temp.path().join("candidate");
        create_candidate_workspace(&snapshot.baseline_path, &workspace).unwrap();
        write(&workspace.join("old.txt"), "after\n");
        write(&workspace.join("new.txt"), "created\n");
        let diff_path = temp.path().join("candidate.patch");
        let stats = collect_diff(&snapshot.baseline_path, &workspace, &diff_path).unwrap();
        let run = run_record(
            &source,
            &snapshot,
            candidate_record("A", workspace, diff_path, stats),
        );

        let report = safe_apply(&run, "A").unwrap();
        assert_eq!(report.candidate_label, "A");
        assert_eq!(report.files_changed, 2);
        assert_eq!(
            fs::read_to_string(source.join("old.txt")).unwrap(),
            "after\n"
        );
        assert_eq!(
            fs::read_to_string(source.join("new.txt")).unwrap(),
            "created\n"
        );
    }

    #[test]
    fn safe_apply_rejects_source_drift_without_partial_changes() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        write(&source.join("one.txt"), "baseline one\n");
        write(&source.join("two.txt"), "baseline two\n");
        let snapshot = create_snapshot(&source, &temp.path().join("run")).unwrap();
        let workspace = temp.path().join("candidate");
        create_candidate_workspace(&snapshot.baseline_path, &workspace).unwrap();
        write(&workspace.join("one.txt"), "candidate one\n");
        write(&workspace.join("two.txt"), "candidate two\n");
        let diff_path = temp.path().join("candidate.patch");
        let stats = collect_diff(&snapshot.baseline_path, &workspace, &diff_path).unwrap();
        let run = run_record(
            &source,
            &snapshot,
            candidate_record("B", workspace, diff_path, stats),
        );

        write(&source.join("two.txt"), "user edit\n");
        let error = safe_apply(&run, "B").unwrap_err().to_string();
        assert!(error.contains("source has changed"));
        assert_eq!(
            fs::read_to_string(source.join("one.txt")).unwrap(),
            "baseline one\n"
        );
        assert_eq!(
            fs::read_to_string(source.join("two.txt")).unwrap(),
            "user edit\n"
        );
    }

    #[test]
    fn git_apply_check_prevents_partial_application_of_a_conflicting_patch() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        write(&source.join("one.txt"), "baseline one\n");
        write(&source.join("two.txt"), "baseline two\n");
        let snapshot = create_snapshot(&source, &temp.path().join("run")).unwrap();
        let workspace = temp.path().join("candidate");
        create_candidate_workspace(&snapshot.baseline_path, &workspace).unwrap();
        write(&workspace.join("one.txt"), "candidate one\n");
        write(&workspace.join("two.txt"), "candidate two\n");
        let diff_path = temp.path().join("candidate.patch");
        let stats = collect_diff(&snapshot.baseline_path, &workspace, &diff_path).unwrap();
        let mut run = run_record(
            &source,
            &snapshot,
            candidate_record("C", workspace, diff_path, stats),
        );

        // Simulate a run record whose content fingerprint matches a different
        // baseline. This gets past the drift guard and directly exercises the
        // all-files apply check.
        write(&source.join("two.txt"), "incompatible contents\n");
        run.source_fingerprint = fingerprint_tree(&source).unwrap();
        let error = safe_apply(&run, "C").unwrap_err().to_string();
        assert!(error.contains("cannot be applied cleanly"));
        assert_eq!(
            fs::read_to_string(source.join("one.txt")).unwrap(),
            "baseline one\n"
        );
        assert_eq!(
            fs::read_to_string(source.join("two.txt")).unwrap(),
            "incompatible contents\n"
        );
    }

    #[test]
    fn fingerprints_ignore_state_but_detect_content_symlinks_and_execute_bits() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        write(&source.join("file"), "value");
        let initial = fingerprint_tree(&source).unwrap();
        write(&source.join(".dispatch/state"), "ignored");
        write(&source.join(".git/index"), "ignored");
        assert_eq!(fingerprint_tree(&source).unwrap(), initial);

        write(&source.join("file"), "changed");
        assert_ne!(fingerprint_tree(&source).unwrap(), initial);

        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            write(&source.join("file"), "value");
            let normal = fingerprint_tree(&source).unwrap();
            fs::set_permissions(source.join("file"), fs::Permissions::from_mode(0o755)).unwrap();
            assert_ne!(fingerprint_tree(&source).unwrap(), normal);

            symlink("file", source.join("link")).unwrap();
            let first_link = fingerprint_tree(&source).unwrap();
            fs::remove_file(source.join("link")).unwrap();
            symlink("other", source.join("link")).unwrap();
            assert_ne!(fingerprint_tree(&source).unwrap(), first_link);
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_that_escape_candidate_isolation() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(source.join("nested")).unwrap();
        symlink("../../outside", source.join("nested/escape")).unwrap();

        let error = fingerprint_tree(&source).unwrap_err().to_string();
        assert!(error.contains("escapes candidate isolation"));

        fs::remove_file(source.join("nested/escape")).unwrap();
        symlink("/tmp/outside", source.join("absolute")).unwrap();
        let error = fingerprint_tree(&source).unwrap_err().to_string();
        assert!(error.contains("absolute symlink"));
    }

    #[test]
    fn refuses_oversized_sparse_candidate_files_before_git() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        let run_dir = temp.path().join("run");
        fs::create_dir(&source).unwrap();
        write(&source.join("small.txt"), "baseline\n");
        let snapshot = create_snapshot(&source, &run_dir).unwrap();
        let candidate = create_candidate_workspace(
            &snapshot.baseline_path,
            &run_dir.join("candidate/workspace"),
        )
        .unwrap();
        let sparse = File::create(candidate.join("huge.bin")).unwrap();
        sparse.set_len(MAX_CANDIDATE_FILE_BYTES + 1).unwrap();

        let error = collect_diff(
            &snapshot.baseline_path,
            &candidate,
            &run_dir.join("candidate/diff.patch"),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("safety limit"));
    }
}
