//! Read-only review material and explicitly selected local viewers.
//! None of this module grants review/apply authority or invokes a model.
use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use crate::{
    executor::{trusted_host_executable, trusted_host_path},
    orchestrator::{self, ReviewCommand},
    source,
    state::State,
};

const MAX_HEADER: usize = 16 * 1024;
const MAX_FILES: usize = 200_000;
const MAX_HUNKS: usize = 100_000;
const MAX_PREVIEW: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
}
impl ChangeKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Modified => "modified",
            Self::Added => "added",
            Self::Deleted => "deleted",
            Self::Renamed => "renamed",
            Self::Copied => "copied",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChangeEntry {
    pub path: PathBuf,
    pub previous_path: Option<PathBuf>,
    pub kind: ChangeKind,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub binary: bool,
    pub permission_change: bool,
    pub bytes: u64,
    /// Relative patch offsets; a bounded navigation index, never patch authority.
    pub hunks: Vec<u64>,
    start: u64,
    ranges: Vec<(u64, u64)>,
    blob: Option<String>,
    regular: bool,
}

impl ChangeEntry {
    pub fn category(&self) -> &'static str {
        if self.binary {
            "binary"
        } else if self.permission_change {
            "permissions"
        } else if self.path.file_name().is_some_and(|n| {
            matches!(
                n.to_str(),
                Some(
                    "Cargo.lock"
                        | "package-lock.json"
                        | "yarn.lock"
                        | "pnpm-lock.yaml"
                        | "generated.lock"
                )
            )
        }) || self
            .path
            .components()
            .any(|c| matches!(c.as_os_str().to_str(), Some("generated" | "dist" | "build")))
        {
            "generated?"
        } else {
            "text"
        }
    }
}

pub struct Preview {
    pub text: String,
    pub next_offset: u64,
    pub truncated: bool,
}
pub struct ReviewIntegrity {
    pub copies_changed: bool,
}

/// Index stores metadata and offsets, never the entire patch in memory.
pub struct ReviewBundle {
    pub entries: Vec<ChangeEntry>,
    root: TempDir,
    patch: PathBuf,
    baseline: PathBuf,
    evidence: [(PathBuf, String); 3],
    copies: Vec<(PathBuf, String)>,
}

impl ReviewBundle {
    pub fn prepare(state: &State, command: &ReviewCommand) -> Result<Self> {
        let (run, _lock) = orchestrator::review_target(state, command)?;
        let candidate = &run.candidates[0];
        let baseline_hash = source::fingerprint_tree(&run.baseline_path)?;
        ensure!(
            baseline_hash == run.source_fingerprint,
            "the frozen baseline changed; review is unavailable"
        );
        let workspace_hash = source::fingerprint_tree(&candidate.workspace_path)?;
        let patch_hash = file_hash(&candidate.diff_path)?;
        let mut entries = index_patch(&candidate.diff_path)?;
        detect_identical_moves(&mut entries);
        ensure!(
            file_hash(&candidate.diff_path)? == patch_hash,
            "the candidate patch changed during indexing"
        );
        let root = tempfile::Builder::new()
            .prefix("dispatch-review-")
            .tempdir()?;
        Ok(Self {
            entries,
            patch: candidate.diff_path.clone(),
            baseline: run.baseline_path.clone(),
            evidence: [
                (run.baseline_path, baseline_hash),
                (candidate.workspace_path.clone(), workspace_hash),
                (candidate.diff_path.clone(), patch_hash),
            ],
            root,
            copies: Vec::new(),
        })
    }

    pub fn preview(&self, index: usize, offset: u64, budget: usize) -> Result<Preview> {
        let entry = self.entries.get(index).context("no selected change")?;
        let offset = offset.min(entry.bytes);
        let mut file = File::open(&self.patch)?;
        let mut bytes = Vec::new();
        let mut skip = offset;
        let mut remaining = budget.clamp(1, MAX_PREVIEW) as u64;
        for &(start, length) in &entry.ranges {
            if skip >= length {
                skip -= length;
                continue;
            }
            file.seek(SeekFrom::Start(start + skip))?;
            let count = remaining.min(length - skip);
            (&mut file).take(count).read_to_end(&mut bytes)?;
            remaining -= count;
            skip = 0;
            if remaining == 0 {
                break;
            }
        }
        let next_offset = offset + bytes.len() as u64;
        Ok(Preview {
            text: String::from_utf8_lossy(&bytes).into_owned(),
            next_offset,
            truncated: next_offset < entry.bytes,
        })
    }

    /// Only selected changed files are copied. Symlinks become link-target text;
    /// no reviewer can traverse them into the source or immutable artifacts.
    pub fn file_pair(&mut self, index: usize) -> Result<(PathBuf, PathBuf)> {
        ensure!(
            !self.entries.iter().any(|entry| sensitive_path(&entry.path)
                || entry
                    .previous_path
                    .as_ref()
                    .is_some_and(|path| sensitive_path(path))),
            "this change set includes authentication or tool configuration paths; use built-in inspection"
        );
        let entry = self.entries.get(index).context("no selected change")?;
        let old = entry.previous_path.as_ref().unwrap_or(&entry.path);
        ensure!(
            !sensitive_path(old) && !sensitive_path(&entry.path),
            "this path may contain authentication or tool configuration; use built-in inspection"
        );
        let directory = self.root.path().join(format!("file-{index}"));
        fs::create_dir_all(&directory)?;
        // Preserve even maximum-length filenames and their syntax extension.
        // The absolute path keeps leading dashes out of the argument namespace.
        let name = entry.path.file_name().context("change has no filename")?;
        fs::create_dir_all(directory.join("before"))?;
        fs::create_dir_all(directory.join("after"))?;
        let before = directory.join("before").join(name);
        let after = directory.join("after").join(name);
        // A previous reviewer may have edited its disposable copies. Rebuild
        // the selected pair on every open rather than reusing those edits.
        for path in [&before, &after] {
            if fs::symlink_metadata(path).is_ok() {
                fs::remove_file(path)?;
            }
        }
        self.copies
            .retain(|(path, _)| path != &before && path != &after);
        {
            copy_review_file(
                &self.baseline,
                old,
                &before,
                entry.kind == ChangeKind::Added,
            )?;
            // Materialize the retained patch against the frozen baseline. A
            // candidate workspace edited after delivery is never review input.
            let staging = tempfile::Builder::new()
                .prefix("dispatch-review-apply-")
                .tempdir()?;
            for change in &self.entries {
                if change.kind == ChangeKind::Added {
                    continue;
                }
                let path = change.previous_path.as_ref().unwrap_or(&change.path);
                let target = staging.path().join(path);
                fs::create_dir_all(target.parent().context("missing review parent")?)?;
                copy_staging_file(&self.baseline, path, &target)?;
            }
            let status = isolated_git(staging.path())?
                .args(["apply", "--binary", "--whitespace=nowarn", "--"])
                .arg(&self.patch)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()?;
            ensure!(
                status.success(),
                "the retained patch could not be reconstructed safely; use built-in inspection"
            );
            copy_review_file(
                staging.path(),
                &entry.path,
                &after,
                entry.kind == ChangeKind::Deleted,
            )?;
            self.copies.push((before.clone(), file_hash(&before)?));
            self.copies.push((after.clone(), file_hash(&after)?));
        }
        self.verify()?;
        Ok((before, after))
    }

    /// Viewers receive a disposable copy, never the apply artifact. The full
    /// patch is retained, including binary records and long lines.
    fn external_patch(&mut self) -> Result<PathBuf> {
        self.verify()?;
        ensure!(
            !self.entries.iter().any(|entry| sensitive_path(&entry.path)
                || entry
                    .previous_path
                    .as_ref()
                    .is_some_and(|path| sensitive_path(path))),
            "this change set includes authentication or tool configuration paths; use built-in inspection"
        );
        let path = self.root.path().join("review-only.patch");
        if fs::symlink_metadata(&path).is_ok() {
            fs::remove_file(&path)?;
        }
        self.copies.retain(|(copy, _)| copy != &path);
        {
            display_patch(&self.patch, &path)?;
            private_readonly(&path)?;
            self.copies.push((path.clone(), file_hash(&path)?));
        }
        self.verify()?;
        Ok(path)
    }

    pub fn verify(&self) -> Result<ReviewIntegrity> {
        for (index, (path, expected)) in self.evidence.iter().enumerate() {
            let actual = if index == 2 {
                file_hash(path)?
            } else {
                source::fingerprint_tree(path)?
            };
            ensure!(
                &actual == expected,
                "immutable review evidence changed; close this review and do not apply it"
            );
        }
        Ok(ReviewIntegrity {
            copies_changed: self
                .copies
                .iter()
                .any(|(path, expected)| file_hash(path).map_or(true, |hash| &hash != expected)),
        })
    }
}

fn private_readonly(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o400))?;
    }
    #[cfg(not(unix))]
    {
        let mut p = fs::metadata(path)?.permissions();
        p.set_readonly(true);
        fs::set_permissions(path, p)?;
    }
    Ok(())
}

fn copy_review_file(root: &Path, relative: &Path, target: &Path, absent: bool) -> Result<()> {
    safe_path(relative)?;
    if absent {
        fs::write(target, [])?;
        return private_readonly(target);
    }
    let mut source = root.to_path_buf();
    let parts = relative.components().collect::<Vec<_>>();
    for (index, part) in parts.iter().enumerate() {
        source.push(part);
        let metadata = fs::symlink_metadata(&source)?;
        if index + 1 != parts.len() {
            ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "review path traverses a symlink or non-directory"
            );
        } else if metadata.file_type().is_symlink() {
            let link = fs::read_link(&source)?;
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStrExt;
                fs::write(target, link.as_os_str().as_bytes())?;
            }
            #[cfg(not(unix))]
            fs::write(target, link.to_string_lossy().as_bytes())?;
        } else {
            ensure!(
                metadata.is_file(),
                "only regular files and symlink descriptions can be reviewed externally"
            );
            let mut options = fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW);
            }
            let mut input = options.open(&source)?;
            let mut output = File::create(target)?;
            std::io::copy(&mut input, &mut output)?;
        }
    }
    private_readonly(target)
}

fn copy_staging_file(root: &Path, relative: &Path, target: &Path) -> Result<()> {
    let source = root.join(relative);
    if fs::symlink_metadata(&source)?.file_type().is_symlink() {
        // Never expose these staging links to a viewer. git apply refuses to
        // write through them; final review copies contain their target text.
        #[cfg(unix)]
        std::os::unix::fs::symlink(fs::read_link(source)?, target)?;
        #[cfg(not(unix))]
        bail!("symlink comparison is unavailable on this platform");
        Ok(())
    } else {
        copy_review_file(root, relative, target, false)
    }
}

fn isolated_git(directory: &Path) -> Result<Command> {
    let mut command = Command::new(trusted_host_executable("git")?);
    command
        .env_clear()
        .env("PATH", trusted_host_path())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env(
            "GIT_CEILING_DIRECTORIES",
            directory
                .parent()
                .context("missing review directory parent")?,
        )
        .arg("-C")
        .arg(directory)
        .args(["-c", "core.hooksPath=/dev/null"]);
    Ok(command)
}

/// Escape controls incrementally for external rendering. This is a labeled
/// display copy, not apply input. Invalid UTF-8 bytes remain visible as escapes;
/// no long line or escape sequence is silently truncated.
fn display_patch(source: &Path, destination: &Path) -> Result<()> {
    let mut reader = BufReader::new(File::open(source)?);
    let mut output = std::io::BufWriter::new(File::create(destination)?);
    let mut pending = Vec::new();
    loop {
        let mut buffer = [0; 8192];
        let count = reader.read(&mut buffer)?;
        pending.extend_from_slice(&buffer[..count]);
        let mut position = 0;
        while position < pending.len() {
            match std::str::from_utf8(&pending[position..]) {
                Ok(text) => {
                    write_display_text(&mut output, text)?;
                    position = pending.len();
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    write_display_text(
                        &mut output,
                        std::str::from_utf8(&pending[position..position + valid])?,
                    )?;
                    position += valid;
                    match error.error_len() {
                        Some(length) => {
                            for byte in &pending[position..position + length] {
                                write!(output, "\\x{byte:02x}")?;
                            }
                            position += length;
                        }
                        None if count == 0 => {
                            for byte in &pending[position..] {
                                write!(output, "\\x{byte:02x}")?;
                            }
                            position = pending.len();
                        }
                        None => break,
                    }
                }
            }
        }
        pending.drain(..position);
        if count == 0 {
            break;
        }
    }
    output.flush()?;
    Ok(())
}
fn write_display_text(output: &mut impl Write, text: &str) -> Result<()> {
    for character in text.chars() {
        if (character.is_control() && !matches!(character, '\n' | '\t'))
            || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        {
            write!(output, "\\u{{{:x}}}", character as u32)?;
        } else {
            write!(output, "{character}")?;
        }
    }
    Ok(())
}

fn file_hash(path: &Path) -> Result<String> {
    ensure!(
        fs::symlink_metadata(path)?.is_file(),
        "review artifact is not a regular file"
    );
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        hash.update(&buffer[..size]);
    }
    Ok(hex::encode(hash.finalize()))
}

fn safe_path(path: &Path) -> Result<()> {
    ensure!(
        !path.as_os_str().is_empty()
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
        "unsafe path in candidate patch"
    );
    ensure!(
        !path
            .components()
            .any(|part| part.as_os_str() == ".git" || part.as_os_str() == ".dispatch"),
        "repository internals are not review material"
    );
    Ok(())
}

fn sensitive_path(path: &Path) -> bool {
    path.components().any(|part| {
        let name = part.as_os_str().to_string_lossy().to_ascii_lowercase();
        matches!(
            name.as_str(),
            ".git"
                | ".dispatch"
                | ".ssh"
                | ".aws"
                | ".azure"
                | ".codex"
                | ".npmrc"
                | ".netrc"
                | "credentials"
                | "credentials.json"
                | "auth.json"
                | "id_rsa"
                | "id_ed25519"
        ) || name == ".env"
            || name.starts_with(".env.")
    })
}

/// Read one logical line without allocating for arbitrary-length source lines.
fn header_line(reader: &mut impl BufRead, buffer: &mut Vec<u8>) -> Result<u64> {
    buffer.clear();
    let mut length = 0;
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        let size = chunk
            .iter()
            .position(|b| *b == b'\n')
            .map_or(chunk.len(), |n| n + 1);
        let done = chunk[size - 1] == b'\n';
        buffer.extend_from_slice(&chunk[..size.min(MAX_HEADER.saturating_sub(buffer.len()))]);
        length += size as u64;
        reader.consume(size);
        if done {
            break;
        }
    }
    Ok(length)
}

pub fn index_patch(path: &Path) -> Result<Vec<ChangeEntry>> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut entries: Vec<ChangeEntry> = Vec::new();
    let mut line = Vec::new();
    let mut offset = 0;
    loop {
        let length = header_line(&mut reader, &mut line)?;
        if length == 0 {
            break;
        }
        let text = line.strip_suffix(b"\n").unwrap_or(&line);
        if let Some(header) = text.strip_prefix(b"diff --git ") {
            ensure!(
                entries.len() < MAX_FILES && length <= MAX_HEADER as u64,
                "patch index exceeds supported metadata bounds; complete patch remains available"
            );
            if let Some(previous) = entries.last_mut() {
                previous.bytes = offset - previous.start;
                previous.ranges.push((previous.start, previous.bytes));
            }
            let (before, after) = header_paths(header)?;
            safe_path(&before)?;
            safe_path(&after)?;
            entries.push(ChangeEntry {
                path: after.clone(),
                previous_path: (before != after).then_some(before),
                kind: ChangeKind::Modified,
                additions: Some(0),
                deletions: Some(0),
                binary: false,
                permission_change: false,
                bytes: 0,
                hunks: Vec::new(),
                start: offset,
                ranges: Vec::new(),
                blob: None,
                regular: false,
            });
        } else if let Some(entry) = entries.last_mut() {
            if text.starts_with(b"old mode ") || text.starts_with(b"new mode ") {
                entry.permission_change = true;
            } else if text.starts_with(b"new file mode ") {
                entry.kind = ChangeKind::Added;
                entry.regular = text.starts_with(b"new file mode 100");
            } else if text.starts_with(b"deleted file mode ") {
                entry.kind = ChangeKind::Deleted;
                entry.regular = text.starts_with(b"deleted file mode 100");
            } else if let Some(index) = text.strip_prefix(b"index ") {
                if let Some((before, after)) = std::str::from_utf8(index)
                    .ok()
                    .and_then(|index| index.split_once(".."))
                {
                    let blob = if entry.kind == ChangeKind::Deleted {
                        before
                    } else {
                        after.split(' ').next().unwrap_or_default()
                    };
                    if matches!(blob.len(), 40 | 64)
                        && blob.bytes().all(|byte| byte.is_ascii_hexdigit())
                        && blob.bytes().any(|byte| byte != b'0')
                    {
                        entry.blob = Some(blob.into());
                    }
                }
            } else if let Some(path) = text.strip_prefix(b"rename from ") {
                entry.previous_path = Some(decode_path(path)?);
                entry.kind = ChangeKind::Renamed;
            } else if let Some(path) = text.strip_prefix(b"rename to ") {
                entry.path = decode_path(path)?;
            } else if let Some(path) = text.strip_prefix(b"copy from ") {
                entry.previous_path = Some(decode_path(path)?);
                entry.kind = ChangeKind::Copied;
            } else if let Some(path) = text.strip_prefix(b"copy to ") {
                entry.path = decode_path(path)?;
            } else if text == b"GIT binary patch" || text.starts_with(b"Binary files ") {
                entry.binary = true;
                entry.additions = None;
                entry.deletions = None;
            } else if text.starts_with(b"@@ ") {
                if entry.hunks.len() < MAX_HUNKS {
                    entry.hunks.push(offset - entry.start);
                }
            } else if !entry.hunks.is_empty() && text.starts_with(b"+") {
                if let Some(count) = &mut entry.additions {
                    *count += 1;
                }
            } else if !entry.hunks.is_empty()
                && text.starts_with(b"-")
                && let Some(count) = &mut entry.deletions
            {
                *count += 1;
            }
        }
        offset += length;
    }
    if let Some(entry) = entries.last_mut() {
        entry.bytes = offset - entry.start;
        entry.ranges.push((entry.start, entry.bytes));
    }
    for entry in &entries {
        safe_path(&entry.path)?;
        if let Some(old) = &entry.previous_path {
            safe_path(old)?;
        }
    }
    Ok(entries)
}

/// Core apply patches deliberately disable rename heuristics. The display may
/// pair only byte-identical regular-file deletion/addition records; both patch
/// ranges remain available, and apply still uses the untouched original patch.
fn detect_identical_moves(entries: &mut Vec<ChangeEntry>) {
    let mut removed: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        if entry.kind == ChangeKind::Deleted
            && entry.regular
            && let Some(hash) = &entry.blob
        {
            removed.entry(hash.clone()).or_default().push(index);
        }
    }
    let mut paired = HashSet::new();
    for index in 0..entries.len() {
        if entries[index].kind != ChangeKind::Added || !entries[index].regular {
            continue;
        }
        let Some(hash) = &entries[index].blob else {
            continue;
        };
        let Some(old) = removed.get_mut(hash).and_then(Vec::pop) else {
            continue;
        };
        let previous = entries[old].clone();
        let current = &mut entries[index];
        current.kind = ChangeKind::Renamed;
        current.previous_path = Some(previous.path);
        current
            .hunks
            .iter_mut()
            .for_each(|offset| *offset += previous.bytes);
        current.hunks.splice(0..0, previous.hunks);
        current.ranges.splice(0..0, previous.ranges);
        current.bytes += previous.bytes;
        current.additions = (!current.binary).then_some(0);
        current.deletions = (!current.binary).then_some(0);
        paired.insert(old);
    }
    let mut index = 0;
    entries.retain(|_| {
        let keep = !paired.contains(&index);
        index += 1;
        keep
    });
}

fn header_paths(header: &[u8]) -> Result<(PathBuf, PathBuf)> {
    if header.starts_with(b"\"") {
        let end = quoted_end(header)?;
        let before = decode_path(&header[..end])?;
        let after = decode_path(
            header[end..]
                .strip_prefix(b" ")
                .context("invalid diff header")?,
        )?;
        return Ok((strip_side(before, "a")?, strip_side(after, "b")?));
    }
    if let Some(split) = header.windows(4).position(|part| part == b" \"b/") {
        return Ok((
            strip_side(decode_path(&header[..split])?, "a")?,
            strip_side(decode_path(&header[split + 1..])?, "b")?,
        ));
    }
    // Git leaves spaces unquoted. Prefer the exact matching-path split, then
    // rename metadata below disambiguates headers whose two paths differ.
    let splits = header
        .windows(3)
        .enumerate()
        .filter_map(|(i, part)| (part == b" b/").then_some(i))
        .collect::<Vec<_>>();
    let split = splits
        .iter()
        .copied()
        .find(|i| header.get(2..*i) == header.get(i + 3..))
        .or_else(|| splits.first().copied())
        .context("invalid diff paths")?;
    Ok((
        strip_side(decode_path(&header[..split])?, "a")?,
        strip_side(decode_path(&header[split + 1..])?, "b")?,
    ))
}

fn quoted_end(bytes: &[u8]) -> Result<usize> {
    let mut escaped = false;
    for (index, byte) in bytes.iter().enumerate().skip(1) {
        if *byte == b'"' && !escaped {
            return Ok(index + 1);
        }
        escaped = *byte == b'\\' && !escaped;
    }
    bail!("unterminated quoted diff path")
}

fn decode_path(bytes: &[u8]) -> Result<PathBuf> {
    let mut decoded = Vec::new();
    if bytes.starts_with(b"\"") {
        ensure!(
            quoted_end(bytes)? == bytes.len(),
            "invalid quoted diff path"
        );
        let mut index = 1;
        while index + 1 < bytes.len() {
            let byte = bytes[index];
            index += 1;
            if byte != b'\\' {
                decoded.push(byte);
                continue;
            }
            let escape = bytes[index];
            index += 1;
            match escape {
                b'0'..=b'3' => {
                    ensure!(
                        index + 1 < bytes.len()
                            && bytes[index..index + 2]
                                .iter()
                                .all(|b| (b'0'..=b'7').contains(b)),
                        "invalid octal diff path"
                    );
                    decoded.push(
                        (escape - b'0') * 64 + (bytes[index] - b'0') * 8 + bytes[index + 1] - b'0',
                    );
                    index += 2;
                }
                b'a' => decoded.push(7),
                b'b' => decoded.push(8),
                b't' => decoded.push(b'\t'),
                b'n' => decoded.push(b'\n'),
                b'v' => decoded.push(11),
                b'f' => decoded.push(12),
                b'r' => decoded.push(b'\r'),
                b'\\' | b'"' => decoded.push(escape),
                _ => bail!("invalid escaped diff path"),
            }
        }
    } else {
        decoded.extend_from_slice(bytes);
    }
    ensure!(!decoded.contains(&0), "NUL in diff path");
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(OsString::from_vec(decoded).into())
    }
    #[cfg(not(unix))]
    {
        Ok(String::from_utf8(decoded)?.into())
    }
}
fn strip_side(path: PathBuf, side: &str) -> Result<PathBuf> {
    Ok(path
        .strip_prefix(side)
        .context("invalid diff path prefix")?
        .to_path_buf())
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewerKind {
    Pager,
    Delta,
    Code,
    Nvim,
    Editor,
}
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WaitMode {
    Process,
    #[default]
    ExplicitReturn,
}

/// Trusted local user preference; arguments are never interpreted by a shell.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewerPreference {
    pub kind: ReviewerKind,
    #[serde(default)]
    pub executable: Option<PathBuf>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub wait: WaitMode,
}

pub struct ReviewLaunch {
    pub command: Command,
    pub wait: WaitMode,
    pub terminal: bool,
    pub warning: Option<String>,
}

impl ReviewerPreference {
    pub fn pager() -> Self {
        Self {
            kind: ReviewerKind::Pager,
            executable: None,
            args: vec![],
            wait: WaitMode::Process,
        }
    }

    pub fn discover(state: &State) -> Result<Self> {
        let path = state.root.join("reviewer.yml");
        if path.exists() {
            trusted_preference_file(&path)?;
            ensure!(
                fs::metadata(&path)?.len() <= 16 * 1024,
                "reviewer preference is too large"
            );
            return Ok(serde_yaml::from_reader(File::open(path)?)?);
        }
        // Only known tool names are reused. Never evaluate difftool.*.cmd, and
        // --global prevents project .git/config from selecting an executable.
        if let Some(tool) = global_git_preference("diff.tool") {
            let kind = match tool.as_str() {
                "code" | "vscode" => Some(ReviewerKind::Code),
                "nvimdiff" | "nvim" => Some(ReviewerKind::Nvim),
                _ => None,
            };
            if let Some(kind) = kind {
                return Ok(Self {
                    kind,
                    ..Self::pager()
                });
            }
        }
        let editor = global_git_preference("core.editor")
            .or_else(|| std::env::var("VISUAL").ok())
            .or_else(|| std::env::var("EDITOR").ok());
        if let Some(editor) = editor {
            return Self::editor_hint(&editor);
        }
        Ok(Self::pager())
    }

    fn editor_hint(hint: &str) -> Result<Self> {
        let mut words = shell_words::split(hint)
            .context("editor hint is not valid quoted arguments")?
            .into_iter();
        let executable = PathBuf::from(words.next().context("empty editor hint")?);
        let args = words.collect();
        let kind = match executable.file_name().and_then(OsStr::to_str) {
            Some("code" | "code-insiders") => ReviewerKind::Code,
            Some("nvim") => ReviewerKind::Nvim,
            _ => ReviewerKind::Editor,
        };
        // Unknown programs may launch a GUI then exit. Keep copies alive until
        // an explicit return action instead of interpreting launcher exit.
        let wait = if kind == ReviewerKind::Editor {
            WaitMode::ExplicitReturn
        } else {
            WaitMode::Process
        };
        Ok(Self {
            kind,
            executable: Some(executable),
            args,
            wait,
        })
    }

    pub fn command(
        &self,
        bundle: &mut ReviewBundle,
        index: Option<usize>,
        no_color: bool,
        light: bool,
    ) -> Result<ReviewLaunch> {
        if self.kind == ReviewerKind::Delta && no_color {
            return Self::pager().command(bundle, index, true, light);
        }
        let default = match self.kind {
            ReviewerKind::Pager => "less",
            ReviewerKind::Delta => "delta",
            ReviewerKind::Code => "code",
            ReviewerKind::Nvim => "nvim",
            ReviewerKind::Editor => "",
        };
        let executable = resolve_executable(
            self.executable
                .as_deref()
                .unwrap_or_else(|| Path::new(default)),
        )?;
        let name = executable
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or_default();
        let resolved = executable.canonicalize()?;
        let resolved_name = resolved
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or_default();
        ensure!(
            !shell_program(name) && !shell_program(resolved_name),
            "shell reviewer commands are unsupported; configure an editor executable and literal arguments"
        );
        let mut command = Command::new(executable);
        command
            .current_dir(bundle.root.path())
            .args(&self.args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        for name in [
            "LESSOPEN",
            "LESSCLOSE",
            "LESS",
            "LESSKEY",
            "LESSKEYIN",
            "DELTA_PAGER",
            "BAT_PAGER",
            "PAGER",
            "GIT_PAGER",
            "GIT_EXTERNAL_DIFF",
            "GIT_CONFIG",
            "GIT_CONFIG_COUNT",
            "GIT_DIR",
            "GIT_WORK_TREE",
            "VIMINIT",
            "EXINIT",
        ] {
            command.env_remove(name);
        }
        command
            .env("LESSSECURE", "1")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            );
        let mut warning = Some("Read-only review copies; edits are not imported. Patch display escapes control characters.".into());
        let mut terminal = true;
        match self.kind {
            ReviewerKind::Pager => {
                command.args(["-S", "--"]).arg(bundle.external_patch()?);
            }
            ReviewerKind::Delta => {
                let pager = resolve_executable(Path::new("less"))?;
                command
                    .args(["--no-gitconfig", "--navigate", "--paging=always", "--pager"])
                    .arg(pager);
                command.arg(if light { "--light" } else { "--dark" });
                command.stdin(File::open(bundle.external_patch()?)?);
            }
            ReviewerKind::Code | ReviewerKind::Nvim => {
                let (before, after) = bundle
                    .file_pair(index.context("select one file before opening this reviewer")?)?;
                if self.kind == ReviewerKind::Code {
                    command.args(["--wait", "--diff"]).arg(before).arg(after);
                    terminal = false;
                } else {
                    command
                        .args(["-R", "-n", "-d", "-i", "NONE", "--"])
                        .arg(before)
                        .arg(after);
                }
            }
            ReviewerKind::Editor => {
                command.arg(bundle.external_patch()?);
                warning = Some("Patch-file fallback: this editor has no supported diff adapter. Close its review document, then return here. Edits to review copies are never imported.".into());
            }
        }
        let wait = if matches!(
            self.kind,
            ReviewerKind::Code | ReviewerKind::Nvim | ReviewerKind::Pager | ReviewerKind::Delta
        ) {
            WaitMode::Process
        } else {
            self.wait
        };
        Ok(ReviewLaunch {
            command,
            wait,
            terminal,
            warning,
        })
    }
}

fn shell_program(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "sh" | "bash"
            | "zsh"
            | "dash"
            | "fish"
            | "ksh"
            | "csh"
            | "tcsh"
            | "env"
            | "xargs"
            | "cmd"
            | "cmd.exe"
            | "powershell"
            | "powershell.exe"
            | "pwsh"
            | "pwsh.exe"
    )
}

fn resolve_executable(path: &Path) -> Result<PathBuf> {
    ensure!(
        !path.as_os_str().is_empty() && !path.as_os_str().to_string_lossy().starts_with('-'),
        "invalid reviewer executable"
    );
    if path.is_absolute() {
        ensure!(path.is_file(), "configured reviewer is unavailable");
        return Ok(path.to_path_buf());
    }
    ensure!(
        path.components().count() == 1,
        "reviewer executable must be a name or an absolute path"
    );
    // Never search the current directory or a relative PATH entry.
    let paths = std::env::var_os("PATH").unwrap_or_default();
    for directory in std::env::split_paths(&paths).filter(|path| path.is_absolute()) {
        let candidate = directory.join(path);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!(
        "reviewer {} is unavailable; use built-in inspection",
        path.display()
    )
}

fn trusted_preference_file(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file(),
        "reviewer preference must be a regular user file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o022 == 0,
            "reviewer preference must be owned by you and not writable by others"
        );
    }
    Ok(())
}

fn global_git_preference(key: &str) -> Option<String> {
    let mut command = Command::new(trusted_host_executable("git").ok()?);
    command
        .env_clear()
        .args(["config", "--global", "--no-includes", "--get", key])
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    for name in ["HOME", "XDG_CONFIG_HOME", "SYSTEMROOT"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let output = command.output().ok()?;
    (output.status.success() && output.stdout.len() < 16 * 1024)
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CandidateStatus, LifecycleState, ReviewState, RunPhase, RunRecord, WorkResult};

    struct Fixture {
        _root: TempDir,
        state: State,
        run: RunRecord,
        target: ReviewCommand,
    }
    impl Fixture {
        fn new() -> Result<Self> {
            let root = tempfile::tempdir()?;
            let state = State::discover(Some(root.path().join("state")))?;
            state.initialize()?;
            let source = root.path().join("source");
            fs::create_dir(&source)?;
            fs::write(source.join("-- odd λ name.txt"), "radius = 20\n")?;
            fs::write(source.join("old name.txt"), "identical\n")?;
            fs::write(source.join("deleted.txt"), "remove\n")?;
            fs::write(source.join("image.bin"), [0, 1, 2, 3])?;
            fs::write(source.join(".env"), "NEVER_COPY_THIS=secret\n")?;
            let id = "01REVIEWTEST";
            let snapshot = source::create_snapshot(&source, &state.run_dir(id))?;
            let workspace = source::create_candidate_workspace(
                &snapshot.baseline_path,
                &root.path().join("workspace"),
            )?;
            fs::write(workspace.join("-- odd λ name.txt"), "radius = 40\n")?;
            fs::rename(
                workspace.join("old name.txt"),
                workspace.join("new name.txt"),
            )?;
            fs::remove_file(workspace.join("deleted.txt"))?;
            fs::write(workspace.join("image.bin"), [0, 4, 5, 6])?;
            let patch = state.run_dir(id).join("candidate.patch");
            let stats = source::collect_diff(&snapshot.baseline_path, &workspace, &patch)?;
            let mut run: RunRecord = serde_json::from_value(serde_json::json!({
                "id":id,"task":"test review","exact_prompt":"test review",
                "source_path":source,"source_kind":"directory","source_git_head":null,
                "source_fingerprint":snapshot.fingerprint,"baseline_path":snapshot.baseline_path,"baseline_commit":snapshot.baseline_commit,
                "status":"ready_for_evaluation","created_at":"2026-09-17T00:00:00Z","completed_at":null,
                "environment":{"dispatch_version":"test","os":"test","architecture":"test","execution_backend":"local","timeout_secs":30,"cpus":1.0,"memory":"1g","max_parallel":1},
                "evaluation":null,"applied_candidate":null,
                "candidates":[{"id":"candidate-one","label":"A","harness_id":"fixture","harness_version":null,"model":null,"status":"completed","workspace_path":workspace,"prompt_path":"prompt","stdout_path":"stdout","stderr_path":"stderr","diff_path":patch,"duration_ms":1,"exit_code":0,"timed_out":false,"tokens":null,"cost_usd":null,"error":null,"diff_stats":stats}]
            }))?;
            run.candidates[0].status = CandidateStatus::Completed;
            run.outcome.lifecycle = LifecycleState::Finished;
            run.outcome.phase = RunPhase::Finished;
            run.outcome.work_result = WorkResult::Ready;
            run.outcome.review = ReviewState::Pending;
            state.save_run(&run)?;
            let target = ReviewCommand {
                run_id: id.into(),
                candidate_id: "candidate-one".into(),
                revision: run.state_revision,
            };
            Ok(Self {
                _root: root,
                state,
                run,
                target,
            })
        }
    }

    #[test]
    fn permission_only_and_generated_hint_remain_reviewable() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let patch = temp.path().join("change.patch");
        fs::write(
            &patch,
            "diff --git a/run.sh b/run.sh\nold mode 100644\nnew mode 100755\ndiff --git a/Cargo.lock b/Cargo.lock\n--- a/Cargo.lock\n+++ b/Cargo.lock\n@@ -1 +1 @@\n-before\n+after\n",
        )?;
        let entries = index_patch(&patch)?;
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].category(), "permissions");
        assert_eq!(entries[1].category(), "generated?");
        assert_eq!(entries[1].additions, Some(1));
        Ok(())
    }

    #[test]
    fn index_is_bounded_and_keeps_every_long_line_and_file() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("large.patch");
        let mut patch = File::create(&path)?;
        for i in 0..150 {
            writeln!(
                patch,
                "diff --git a/file{i} b/file{i}\n--- a/file{i}\n+++ b/file{i}\n@@ -1 +1 @@\n-old\n+new"
            )?;
        }
        writeln!(
            patch,
            "diff --git a/long b/long\n--- a/long\n+++ b/long\n@@ -1 +1 @@\n-old\n+{}",
            "x".repeat(3_000_000)
        )?;
        let entries = index_patch(&path)?;
        assert_eq!(entries.len(), 151);
        assert!(entries.last().unwrap().bytes > 3_000_000);
        assert!(
            entries
                .iter()
                .all(|entry| entry.additions == Some(1) && entry.deletions == Some(1))
        );
        assert_eq!(
            entries.iter().map(|entry| entry.bytes).sum::<u64>(),
            fs::metadata(path)?.len()
        );
        Ok(())
    }

    #[test]
    fn index_understands_binary_rename_deletion_and_quoted_names() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let patch = directory.path().join("patch");
        fs::write(&patch, b"diff --git a/old name b/new name\nsimilarity index 100%\nrename from old name\nrename to new name\ndiff --git \"a/-- \\316\\273\\tfile\" \"b/-- \\316\\273\\tfile\"\ndeleted file mode 100644\n--- a/x\n+++ /dev/null\n@@ -1 +0,0 @@\n-old\ndiff --git a/bin b/bin\nGIT binary patch\nliteral 4\nabcd\n")?;
        let entries = index_patch(&patch)?;
        assert_eq!(entries[0].kind, ChangeKind::Renamed);
        assert_eq!(
            entries[0].previous_path.as_deref(),
            Some(Path::new("old name"))
        );
        assert_eq!(entries[1].path, PathBuf::from("-- λ\tfile"));
        assert_eq!(entries[1].kind, ChangeKind::Deleted);
        assert!(entries[2].binary);
        assert_eq!(entries[2].additions, None);
        fs::write(&patch, "diff --git a/../escape b/../escape\n")?;
        assert!(index_patch(&patch).is_err());
        fs::write(&patch, "diff --git a/.git/config b/.git/config\n")?;
        assert!(index_patch(&patch).is_err());
        Ok(())
    }

    #[test]
    fn pairs_come_from_frozen_baseline_and_authoritative_patch_not_edited_candidate() -> Result<()>
    {
        let fixture = Fixture::new()?;
        // Already-edited candidate bytes must not sneak into an external review.
        fs::write(
            fixture.run.candidates[0]
                .workspace_path
                .join("-- odd λ name.txt"),
            "unverified edit\n",
        )?;
        // The rename label also comes from retained full blob IDs, even if
        // someone has changed the final workspace before opening this view.
        fs::write(
            fixture.run.candidates[0]
                .workspace_path
                .join("new name.txt"),
            "different now\n",
        )?;
        let mut bundle = ReviewBundle::prepare(&fixture.state, &fixture.target)?;
        assert_eq!(bundle.entries.len(), 4);
        assert!(
            bundle
                .entries
                .iter()
                .any(|entry| entry.kind == ChangeKind::Renamed)
        );
        let index = bundle
            .entries
            .iter()
            .position(|entry| entry.path == Path::new("-- odd λ name.txt"))
            .unwrap();
        let (before, after) = bundle.file_pair(index)?;
        assert_eq!(fs::read_to_string(before)?, "radius = 20\n");
        assert_eq!(fs::read_to_string(&after)?, "radius = 40\n");
        assert!(!bundle.root.path().join(".git").exists());
        assert!(!bundle.root.path().join(".env").exists());
        assert_eq!(
            fs::read_to_string(fixture.run.source_path.join("-- odd λ name.txt"))?,
            "radius = 20\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&after, fs::Permissions::from_mode(0o600))?;
        }
        fs::write(after, "discarded review edit")?;
        assert!(bundle.verify()?.copies_changed);
        assert_eq!(
            fs::read_to_string(
                fixture.run.candidates[0]
                    .workspace_path
                    .join("-- odd λ name.txt")
            )?,
            "unverified edit\n"
        );
        fs::write(&fixture.run.candidates[0].diff_path, "tampered")?;
        assert!(bundle.verify().is_err());
        assert!(bundle.external_patch().is_err());
        Ok(())
    }

    #[test]
    fn file_pair_rename_delete_and_binary_preserve_full_content() -> Result<()> {
        let fixture = Fixture::new()?;
        let mut bundle = ReviewBundle::prepare(&fixture.state, &fixture.target)?;
        let rename = bundle
            .entries
            .iter()
            .position(|e| e.kind == ChangeKind::Renamed)
            .unwrap();
        let preview = bundle.preview(rename, 0, MAX_PREVIEW)?;
        assert!(preview.text.contains("old name.txt") && preview.text.contains("new name.txt"));
        let pair = bundle.file_pair(rename)?;
        assert_eq!(fs::read(pair.0)?, fs::read(pair.1)?);
        let deleted = bundle
            .entries
            .iter()
            .position(|e| e.kind == ChangeKind::Deleted)
            .unwrap();
        let pair = bundle.file_pair(deleted)?;
        assert_eq!(fs::read(pair.0)?, b"remove\n");
        assert!(fs::read(pair.1)?.is_empty());
        let binary = bundle.entries.iter().position(|e| e.binary).unwrap();
        let pair = bundle.file_pair(binary)?;
        assert_eq!(fs::read(pair.0)?, [0, 1, 2, 3]);
        assert_eq!(fs::read(pair.1)?, [0, 4, 5, 6]);
        assert!(!bundle.verify()?.copies_changed);
        Ok(())
    }

    #[test]
    fn stale_cross_candidate_and_concurrent_review_cannot_refresh_to_another_delivery() -> Result<()>
    {
        let fixture = Fixture::new()?;
        let mut wrong = fixture.target.clone();
        wrong.candidate_id = "other-run-candidate".into();
        assert!(ReviewBundle::prepare(&fixture.state, &wrong).is_err());
        let mut stale = fixture.target.clone();
        stale.revision += 1;
        assert!(ReviewBundle::prepare(&fixture.state, &stale).is_err());
        let mut run = fixture.run.clone();
        run.state_revision += 1;
        fixture.state.save_run(&run)?;
        assert_eq!(
            orchestrator::refresh_review_target(&fixture.state, &fixture.target)?.state_revision,
            run.state_revision
        );
        run.outcome.review = ReviewState::Rejected;
        fixture.state.save_run(&run)?;
        assert!(orchestrator::refresh_review_target(&fixture.state, &fixture.target).is_err());
        Ok(())
    }

    #[test]
    fn baseline_drift_blocks_preparation_but_source_drift_does_not_change_review_material()
    -> Result<()> {
        let fixture = Fixture::new()?;
        fs::write(
            fixture.run.source_path.join("-- odd λ name.txt"),
            "new source content",
        )?;
        let bundle = ReviewBundle::prepare(&fixture.state, &fixture.target)?;
        assert!(!bundle.verify()?.copies_changed);
        fs::write(
            fixture.run.baseline_path.join("-- odd λ name.txt"),
            "baseline modified",
        )?;
        assert!(bundle.verify().is_err());
        assert!(ReviewBundle::prepare(&fixture.state, &fixture.target).is_err());
        Ok(())
    }

    #[test]
    fn controls_are_escaped_in_full_external_display_copy_without_line_truncation() -> Result<()> {
        let root = tempfile::tempdir()?;
        let input = root.path().join("in");
        let output = root.path().join("out");
        let mut bytes = vec![b'x'; 8191];
        bytes.extend_from_slice("λ\u{009b}\u{202e}".as_bytes());
        bytes.extend_from_slice(b"\x1b]52;clipboard\x07\n\t\xff");
        bytes.extend_from_slice(&vec![b'y'; 300_000]);
        fs::write(&input, bytes)?;
        display_patch(&input, &output)?;
        let display = fs::read_to_string(output)?;
        assert!(display.contains("λ\\u{9b}\\u{202e}\\u{1b}]52;clipboard\\u{7}\n\t\\xff"));
        assert!(display.ends_with(&"y".repeat(300_000)));
        assert!(!display.contains('\x1b'));
        Ok(())
    }

    #[test]
    fn preference_argv_and_wait_contracts_do_not_assume_directory_diff_or_shell() -> Result<()> {
        let fixture = Fixture::new()?;
        let mut bundle = ReviewBundle::prepare(&fixture.state, &fixture.target)?;
        let index = bundle
            .entries
            .iter()
            .position(|e| e.kind == ChangeKind::Modified && !e.binary)
            .unwrap();
        let executable = fixture._root.path().join("fake reviewer λ");
        fs::write(&executable, "fixture")?;
        let config = format!(
            "kind: code\nexecutable: '{}'\nargs: ['literal $(touch nope)']\n",
            executable.display()
        );
        fs::write(fixture.state.root.join("reviewer.yml"), config)?;
        let preference = ReviewerPreference::discover(&fixture.state)?;
        let launch = preference.command(&mut bundle, Some(index), false, false)?;
        let arguments = launch
            .command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            &arguments[..3],
            &["literal $(touch nope)", "--wait", "--diff"]
        );
        assert!(Path::new(&arguments[3]).is_absolute());
        assert_eq!(launch.wait, WaitMode::Process);
        assert!(!launch.terminal);
        assert!(preference.command(&mut bundle, None, false, false).is_err());
        let unknown = ReviewerPreference::editor_hint("'some editor' 'literal;$(touch nope)'")?;
        assert_eq!(unknown.kind, ReviewerKind::Editor);
        assert_eq!(unknown.wait, WaitMode::ExplicitReturn);
        assert_eq!(unknown.args, ["literal;$(touch nope)"]);
        let nvim = ReviewerPreference {
            kind: ReviewerKind::Nvim,
            executable: Some(executable),
            args: vec![],
            wait: WaitMode::ExplicitReturn,
        };
        let launch = nvim.command(&mut bundle, Some(index), false, false)?;
        assert_eq!(
            launch.command.get_args().take(6).collect::<Vec<_>>(),
            ["-R", "-n", "-d", "-i", "NONE", "--"]
        );
        assert!(launch.terminal);
        assert_eq!(launch.wait, WaitMode::Process);
        let shell = ReviewerPreference {
            kind: ReviewerKind::Editor,
            executable: Some(PathBuf::from("/bin/sh")),
            args: vec!["-c".into(), "touch should-never-run".into()],
            wait: WaitMode::Process,
        };
        assert!(
            shell
                .command(&mut bundle, Some(index), false, false)
                .is_err()
        );
        let missing = ReviewerPreference {
            executable: Some(fixture._root.path().join("missing")),
            ..ReviewerPreference::pager()
        };
        assert!(missing.command(&mut bundle, None, false, false).is_err());
        assert!(
            serde_yaml::from_str::<ReviewerPreference>("kind: editor\nshell: 'touch nope'")
                .is_err()
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_text_only_and_authentication_paths_never_export() -> Result<()> {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        fs::create_dir(&source)?;
        fs::write(source.join("target"), "private")?;
        std::os::unix::fs::symlink("target", source.join("link"))?;
        let copy = root.path().join("copy");
        copy_review_file(&source, Path::new("link"), &copy, false)?;
        assert!(!fs::symlink_metadata(&copy)?.file_type().is_symlink());
        assert_eq!(fs::read(copy)?, b"target");
        std::os::unix::fs::symlink(&source, source.join("directory-link"))?;
        assert!(
            copy_review_file(
                &source,
                Path::new("directory-link/target"),
                &root.path().join("bad"),
                false
            )
            .is_err()
        );
        for path in [
            ".env",
            ".aws/credentials",
            ".git/hooks/post-checkout",
            ".codex/auth.json",
            "id_rsa",
        ] {
            assert!(sensitive_path(Path::new(path)));
        }
        Ok(())
    }
}
