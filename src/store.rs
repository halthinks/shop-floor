//! Durable JSON store under the shop directory.
//!
//! Crash-safe enough: write temp + rename. Snapshot is the source of truth.
//! Parent files and the claim ledger are readable projections.
//! Event lines are Evidence, not authority.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Result, ShopError};
use crate::github::GitHubRepo;
use crate::types::ShopState;
use crate::workers::WorkerRoster;

const META: &str = r#"{
  "kind": "shop",
  "version": 1,
  "note": "hold-split-join store. Snapshot is durable state. Events and handoffs are Evidence, not authority."
}
"#;

#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn init(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        fs::create_dir_all(root.join("parents"))?;
        fs::create_dir_all(root.join("outbox"))?;
        fs::create_dir_all(root.join("reduce"))?;
        fs::create_dir_all(root.join("verify"))?;
        if !root.join("meta.json").exists() {
            write_atomic(&root.join("meta.json"), META.as_bytes())?;
        }
        if !root.join("snapshot.json").exists() {
            let empty = ShopState::default();
            write_json_atomic(&root.join("snapshot.json"), &empty)?;
            write_json_atomic(&root.join("claims.json"), &empty.claims)?;
        }
        if !root.join("workers.json").exists() {
            write_json_atomic(&root.join("workers.json"), &WorkerRoster::default())?;
        }
        Ok(Self { root })
    }

    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        if !root.join("meta.json").exists() && !root.join("snapshot.json").exists() {
            return Err(ShopError::StoreNotFound(root));
        }
        fs::create_dir_all(root.join("parents"))?;
        fs::create_dir_all(root.join("outbox"))?;
        fs::create_dir_all(root.join("reduce"))?;
        fs::create_dir_all(root.join("verify"))?;
        if !root.join("workers.json").exists() {
            write_json_atomic(&root.join("workers.json"), &WorkerRoster::default())?;
        }
        Ok(Self { root })
    }

    pub fn load_workers(&self) -> Result<WorkerRoster> {
        let path = self.root.join("workers.json");
        if !path.exists() {
            return Ok(WorkerRoster::default());
        }
        Ok(serde_json::from_slice(&fs::read(&path)?)?)
    }

    pub fn save_workers(&self, roster: &WorkerRoster) -> Result<()> {
        write_json_atomic(&self.root.join("workers.json"), roster)
    }

    pub fn load_github(&self) -> Result<Option<GitHubRepo>> {
        let path = self.root.join("github.json");
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_slice(&fs::read(&path)?)?))
    }

    pub fn save_github(&self, repo: &GitHubRepo) -> Result<()> {
        write_json_atomic(&self.root.join("github.json"), repo)
    }

    pub fn write_token(&self, token: &str) -> Result<()> {
        let path = self.root.join("github.token");
        ensure_under(&path, &[self.root.clone()])?;
        write_atomic(&path, token.trim().as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&path)?.permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&path, perms)?;
        }
        Ok(())
    }

    pub fn has_token_file(&self) -> bool {
        self.root.join("github.token").is_file()
    }

    pub fn load(&self) -> Result<ShopState> {
        let snap = self.root.join("snapshot.json");
        if snap.exists() {
            let data = fs::read(&snap)?;
            return Ok(serde_json::from_slice(&data)?);
        }
        let mut state = ShopState::default();
        let claims_path = self.root.join("claims.json");
        if claims_path.exists() {
            state.claims = serde_json::from_slice(&fs::read(&claims_path)?)?;
        }
        let parents_dir = self.root.join("parents");
        if parents_dir.is_dir() {
            for entry in fs::read_dir(&parents_dir)? {
                let path = entry?.path();
                if path.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }
                let parent = serde_json::from_slice(&fs::read(&path)?)?;
                let id = match &parent {
                    crate::types::Parent { id, .. } => id.clone(),
                };
                state.parents.insert(id, parent);
            }
        }
        Ok(state)
    }

    pub fn save(&self, state: &ShopState) -> Result<()> {
        write_json_atomic(&self.root.join("snapshot.json"), state)?;
        write_json_atomic(&self.root.join("claims.json"), &state.claims)?;

        let parents_dir = self.root.join("parents");
        fs::create_dir_all(&parents_dir)?;
        let mut keep = std::collections::BTreeSet::new();
        for (id, parent) in &state.parents {
            let path = parents_dir.join(format!("{id}.json"));
            write_json_atomic(&path, parent)?;
            keep.insert(path);
        }
        for entry in fs::read_dir(&parents_dir)? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") && !keep.contains(&path) {
                let _ = fs::remove_file(path);
            }
        }
        Ok(())
    }

    pub fn append_event(&self, event: &impl Serialize) -> Result<()> {
        let path = self.root.join("events.jsonl");
        let mut f = OpenOptions::new().create(true).append(true).open(path)?;
        serde_json::to_writer(&mut f, event)?;
        f.write_all(b"\n")?;
        Ok(())
    }

    pub fn write_under(&self, rel: impl AsRef<Path>, bytes: &[u8]) -> Result<PathBuf> {
        let path = self.root.join(rel);
        ensure_under(&path, &[self.root.clone()])?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_atomic(&path, bytes)?;
        Ok(path)
    }

    pub fn outbox_dir(&self) -> PathBuf {
        self.root.join("outbox")
    }
}

pub fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_file_name(format!(
        ".{}.tmp",
        path.file_name()
            .map(|s| s.to_string_lossy())
            .unwrap_or_else(|| "shop".into())
    ));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn ensure_under(path: &Path, roots: &[PathBuf]) -> Result<()> {
    let text = path.to_string_lossy();
    if text.contains("TextPCB Platform") || text.contains("C:\\TextPCB") {
        return Err(ShopError::ForbiddenWrite(path.to_path_buf()));
    }
    let abs = make_abs(path);
    for root in roots {
        let root_abs = make_abs(root);
        if abs.starts_with(&root_abs) {
            return Ok(());
        }
    }
    Err(ShopError::ForbiddenWrite(path.to_path_buf()))
}

fn make_abs(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}
