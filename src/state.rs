use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::{EventRecord, RunRecord};

#[derive(Debug, Clone)]
pub struct State {
    pub root: PathBuf,
}

impl State {
    pub fn discover(explicit: Option<PathBuf>) -> Result<Self> {
        let root = explicit
            .or_else(|| std::env::var_os("DISPATCH_HOME").map(PathBuf::from))
            .or_else(|| dirs::home_dir().map(|p| p.join(".dispatch")))
            .context("could not determine Dispatch state directory; set DISPATCH_HOME")?;
        let root = if root.is_absolute() {
            root
        } else {
            std::env::current_dir()
                .context("failed to resolve relative Dispatch state directory")?
                .join(root)
        };
        Ok(Self { root })
    }

    pub fn initialize(&self) -> Result<()> {
        let root_is_new = !self.root.exists();
        fs::create_dir_all(&self.root)
            .with_context(|| format!("failed to create {}", self.root.display()))?;
        anyhow::ensure!(
            self.root.is_dir(),
            "Dispatch state path is not a directory: {}",
            self.root.display()
        );
        let runs_dir = self.runs_dir();
        let runs_is_new = !runs_dir.exists();
        fs::create_dir_all(&runs_dir)
            .with_context(|| format!("failed to create {}", runs_dir.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if root_is_new {
                fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))?;
            }
            if runs_is_new {
                fs::set_permissions(&runs_dir, fs::Permissions::from_mode(0o700))?;
            }
        }
        Ok(())
    }

    pub fn db_path(&self) -> PathBuf {
        self.root.join("dispatch.db")
    }
    pub fn runs_dir(&self) -> PathBuf {
        self.root.join("runs")
    }
    pub fn datasets_dir(&self) -> PathBuf {
        self.root.join("datasets")
    }
    pub fn run_dir(&self, run_id: &str) -> PathBuf {
        self.runs_dir().join(run_id)
    }
    pub fn metadata_path(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("metadata.json")
    }
    pub fn events_path(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("events.jsonl")
    }

    pub fn save_run(&self, run: &RunRecord) -> Result<()> {
        let path = self.metadata_path(&run.id);
        let parent = path.parent().context("metadata path has no parent")?;
        fs::create_dir_all(parent)?;
        let tmp = parent.join("metadata.json.tmp");
        let bytes = serde_json::to_vec_pretty(run)?;
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn append_event(&self, event: &EventRecord) -> Result<()> {
        let path = self.events_path(&event.run_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        serde_json::to_writer(&mut file, event)?;
        writeln!(file)?;
        Ok(())
    }

    pub fn load_run(&self, id_or_prefix: &str) -> Result<RunRecord> {
        let id = self.resolve_run_id(id_or_prefix)?;
        let bytes = fs::read(self.metadata_path(&id))
            .with_context(|| format!("run {id} has no readable metadata"))?;
        let run: RunRecord = serde_json::from_slice(&bytes)
            .with_context(|| format!("run {id} metadata is invalid"))?;
        anyhow::ensure!(
            run.id == id,
            "run metadata identity does not match directory {id}"
        );
        Ok(run)
    }

    pub fn resolve_run_id(&self, id_or_prefix: &str) -> Result<String> {
        anyhow::ensure!(
            !id_or_prefix.is_empty()
                && id_or_prefix.len() <= 26
                && id_or_prefix
                    .chars()
                    .all(|value| value.is_ascii_alphanumeric()),
            "invalid run ID or prefix: {id_or_prefix:?}"
        );
        let normalized = id_or_prefix.to_ascii_uppercase();
        if self.metadata_path(&normalized).is_file() {
            return Ok(normalized);
        }
        let mut matches = Vec::new();
        if self.runs_dir().is_dir() {
            for entry in fs::read_dir(self.runs_dir())? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with(&normalized) && entry.path().join("metadata.json").is_file() {
                    matches.push(name);
                }
            }
        }
        matches.sort();
        match matches.as_slice() {
            [only] => Ok(only.clone()),
            [] => bail!("run not found: {id_or_prefix}"),
            _ => bail!("run prefix is ambiguous: {id_or_prefix}"),
        }
    }

    pub fn list_metadata_paths(&self) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        if !self.runs_dir().is_dir() {
            return Ok(paths);
        }
        for entry in fs::read_dir(self.runs_dir())? {
            let path = entry?.path().join("metadata.json");
            if path.is_file() {
                paths.push(path);
            }
        }
        paths.sort();
        paths.reverse();
        Ok(paths)
    }
}

pub fn write_text(path: &Path, value: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, value).with_context(|| format!("failed to write {}", path.display()))
}
