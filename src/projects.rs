//! Multiple open floors. Switch without losing the other project's memory/floor.
//!
//! New module only. Glue (engine/cli/ui/store) is out of this file.
//! Incomplete evidence is WAIT, never a fake PASS. Never writes
//! `C:\TextPCB Platform`.
//!
//! Each open project keeps its own live `Shop` (its `.shop/` store, floor, and
//! memory). Opening another project does not drop the first. Switch changes
//! which handle is current.

use std::path::{Path, PathBuf};

use crate::error::{Result, ShopError};
use crate::project::{
    default_recents_path, project_name, project_root_from_store, refuse_platform, store_dir_for,
};
use crate::Shop;

/// One open floor in the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenFloor {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
    pub store: PathBuf,
}

/// Session of live floors. One current; the rest stay loaded.
pub struct FloorSet {
    recents_path: PathBuf,
    floors: Vec<LiveFloor>,
    current: Option<usize>,
}

struct LiveFloor {
    id: String,
    shop: Shop,
}

impl FloorSet {
    pub fn new() -> Self {
        Self::with_recents(default_recents_path())
    }

    pub fn with_recents(recents_path: impl Into<PathBuf>) -> Self {
        Self {
            recents_path: recents_path.into(),
            floors: Vec::new(),
            current: None,
        }
    }

    pub fn recents_path(&self) -> &Path {
        &self.recents_path
    }

    pub fn len(&self) -> usize {
        self.floors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.floors.is_empty()
    }

    pub fn list(&self) -> Vec<OpenFloor> {
        self.floors
            .iter()
            .map(|live| describe(live, self.current_id().as_deref()))
            .collect()
    }

    pub fn current_id(&self) -> Option<String> {
        self.current
            .and_then(|i| self.floors.get(i))
            .map(|live| live.id.clone())
    }

    /// Open a project folder and make it current. A second open of the same
    /// path switches back; it does not drop the other live floors.
    pub fn open(&mut self, path: impl AsRef<Path>) -> Result<OpenFloor> {
        let path = path.as_ref();
        refuse_open_path(path)?;
        refuse_platform(path)?;
        refuse_platform(&self.recents_path)?;
        let store_dir = store_dir_for(path)?;
        refuse_platform(&store_dir)?;
        if let Some(parent) = store_dir.parent() {
            refuse_platform(parent)?;
        }

        let shop = Shop::open_project_with_recents(path, &self.recents_path)?;
        let root = shop.project_root().to_path_buf();
        let store = shop.store_root().to_path_buf();
        let id = floor_id(&root, &store);

        if let Some(idx) = self.index_of(&id) {
            self.floors[idx].shop = shop;
            self.current = Some(idx);
            return Ok(self.handle_at(idx));
        }

        self.floors.push(LiveFloor { id, shop });
        let idx = self.floors.len() - 1;
        self.current = Some(idx);
        Ok(self.handle_at(idx))
    }

    /// Switch current to an already-open floor. Does not unload the others.
    /// Unknown or empty key is WAIT, never an invented project.
    pub fn switch(&mut self, key: &str) -> Result<OpenFloor> {
        let key = key.trim();
        if key.is_empty() {
            return Err(ShopError::IncompleteEvidence(
                "WAIT: switch key is empty; no invented project".into(),
            ));
        }
        let idx = self
            .index_of(key)
            .ok_or_else(|| ShopError::Wait(format!("project not open: {key}")))?;
        self.current = Some(idx);
        Ok(self.handle_at(idx))
    }

    pub fn current(&self) -> Result<&Shop> {
        let idx = self.current.ok_or_else(|| {
            ShopError::IncompleteEvidence("WAIT: no open floor; open a project first".into())
        })?;
        self.floors
            .get(idx)
            .map(|live| &live.shop)
            .ok_or_else(|| ShopError::IncompleteEvidence("WAIT: current floor missing".into()))
    }

    pub fn current_mut(&mut self) -> Result<&mut Shop> {
        let idx = self.current.ok_or_else(|| {
            ShopError::IncompleteEvidence("WAIT: no open floor; open a project first".into())
        })?;
        self.floors
            .get_mut(idx)
            .map(|live| &mut live.shop)
            .ok_or_else(|| ShopError::IncompleteEvidence("WAIT: current floor missing".into()))
    }

    pub fn get(&self, key: &str) -> Result<&Shop> {
        let key = key.trim();
        if key.is_empty() {
            return Err(ShopError::IncompleteEvidence(
                "WAIT: empty project key".into(),
            ));
        }
        let idx = self
            .index_of(key)
            .ok_or_else(|| ShopError::Wait(format!("project not open: {key}")))?;
        Ok(&self.floors[idx].shop)
    }

    pub fn get_mut(&mut self, key: &str) -> Result<&mut Shop> {
        let key = key.trim();
        if key.is_empty() {
            return Err(ShopError::IncompleteEvidence(
                "WAIT: empty project key".into(),
            ));
        }
        let idx = self
            .index_of(key)
            .ok_or_else(|| ShopError::Wait(format!("project not open: {key}")))?;
        Ok(&mut self.floors[idx].shop)
    }

    /// Drop a live handle. Disk store (memory/floor) stays. Other opens stay.
    pub fn close(&mut self, key: &str) -> Result<OpenFloor> {
        let key = key.trim();
        if key.is_empty() {
            return Err(ShopError::IncompleteEvidence(
                "WAIT: close key is empty".into(),
            ));
        }
        let idx = self
            .index_of(key)
            .ok_or_else(|| ShopError::Wait(format!("project not open: {key}")))?;
        let live = self.floors.remove(idx);
        let closed = describe(&live, None);
        match self.current {
            Some(cur) if cur == idx => {
                self.current = if self.floors.is_empty() {
                    None
                } else if idx < self.floors.len() {
                    Some(idx)
                } else {
                    Some(self.floors.len() - 1)
                };
            }
            Some(cur) if cur > idx => self.current = Some(cur - 1),
            _ => {}
        }
        Ok(closed)
    }
}

impl Default for FloorSet {
    fn default() -> Self {
        Self::new()
    }
}

fn refuse_open_path(path: &Path) -> Result<()> {
    let raw = path.as_os_str();
    if raw.is_empty() || path == Path::new("") {
        return Err(ShopError::IncompleteEvidence(
            "WAIT: empty project path".into(),
        ));
    }
    Ok(())
}

fn floor_id(root: &Path, store: &Path) -> String {
    let from_root = root.as_os_str().is_empty().then_some(store).unwrap_or(root);
    let canonical = std::fs::canonicalize(from_root).unwrap_or_else(|_| from_root.to_path_buf());
    let project = project_root_from_store(&canonical);
    normalize_key(&project)
}

fn normalize_key(path: &Path) -> String {
    let mut s = path.display().to_string();
    while s.ends_with(['/', '\\']) && s.len() > 1 {
        s.pop();
    }
    if cfg!(windows) {
        s = s.replace('/', "\\");
        s.to_ascii_lowercase()
    } else {
        s
    }
}

fn keys_match(id: &str, key: &str) -> bool {
    if id == key {
        return true;
    }
    let nk = if cfg!(windows) {
        key.replace('/', "\\").to_ascii_lowercase()
    } else {
        key.to_string()
    };
    if id == nk {
        return true;
    }
    let key_path = Path::new(key);
    let canon = std::fs::canonicalize(key_path).unwrap_or_else(|_| key_path.to_path_buf());
    id == normalize_key(&canon) || id == normalize_key(key_path)
}

impl FloorSet {
    fn index_of(&self, key: &str) -> Option<usize> {
        let id_hits: Vec<usize> = self
            .floors
            .iter()
            .enumerate()
            .filter(|(_, live)| keys_match(&live.id, key))
            .map(|(i, _)| i)
            .collect();
        if id_hits.len() == 1 {
            return Some(id_hits[0]);
        }
        if id_hits.len() > 1 {
            return None;
        }

        let path_hits: Vec<usize> = self
            .floors
            .iter()
            .enumerate()
            .filter(|(_, live)| {
                let root = live.shop.project_root();
                let store = live.shop.store_root();
                keys_match(&normalize_key(root), key) || keys_match(&normalize_key(store), key)
            })
            .map(|(i, _)| i)
            .collect();
        if path_hits.len() == 1 {
            return Some(path_hits[0]);
        }
        if path_hits.len() > 1 {
            return None;
        }

        let name_hits: Vec<usize> = self
            .floors
            .iter()
            .enumerate()
            .filter(|(_, live)| project_name(live.shop.project_root()) == key)
            .map(|(i, _)| i)
            .collect();
        if name_hits.len() == 1 {
            Some(name_hits[0])
        } else {
            None
        }
    }

    fn handle_at(&self, idx: usize) -> OpenFloor {
        describe(&self.floors[idx], self.current_id().as_deref())
    }

    pub fn format(&self) -> String {
        if self.floors.is_empty() {
            return "no open floors\n".into();
        }
        let current = self.current_id();
        let mut out = String::from("OPEN FLOORS\n");
        for live in &self.floors {
            let mark = if current.as_deref() == Some(live.id.as_str()) {
                "*"
            } else {
                " "
            };
            let h = describe(live, current.as_deref());
            out.push_str(&format!(
                "{mark}  {}  {}  {}\n",
                h.name,
                h.root.display(),
                h.store.display()
            ));
        }
        out
    }
}

fn describe(live: &LiveFloor, current_id: Option<&str>) -> OpenFloor {
    let _ = current_id;
    OpenFloor {
        id: live.id.clone(),
        name: project_name(live.shop.project_root()),
        root: live.shop.project_root().to_path_buf(),
        store: live.shop.store_root().to_path_buf(),
    }
}
