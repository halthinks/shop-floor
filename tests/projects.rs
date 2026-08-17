//! Multiple open floors: switch without losing the other project's memory/floor.
//! Incomplete evidence is WAIT, never a fake PASS.
//!
//! `src/projects.rs` is the crate module (`crate::`) for a later `pub mod
//! projects` glue line. This test crate cannot use that until lib.rs grants
//! it, so the session type is compiled here against the public `shop` API.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use shop::error::{Result, ShopError};
use shop::memory::MemoryTier;
use shop::project::{
    default_recents_path, project_name, project_root_from_store, refuse_platform, store_dir_for,
};
use shop::types::{EvidenceStatus, ParentState};
use shop::Shop;

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

fn describe(live: &LiveFloor, current_id: Option<&str>) -> OpenFloor {
    let _ = current_id;
    OpenFloor {
        id: live.id.clone(),
        name: project_name(live.shop.project_root()),
        root: live.shop.project_root().to_path_buf(),
        store: live.shop.store_root().to_path_buf(),
    }
}

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp_dir() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("shop-projects-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn proj(name: &str) -> PathBuf {
    let p = tmp_dir().join(name);
    fs::create_dir_all(&p).unwrap();
    p
}

fn set() -> (FloorSet, PathBuf) {
    let recents = tmp_dir().join("recents.json");
    (FloorSet::with_recents(&recents), recents)
}

fn is_wait(err: &ShopError) -> bool {
    match err {
        ShopError::IncompleteEvidence(_) | ShopError::Wait(_) | ShopError::ForbiddenWrite(_) => {
            true
        }
        _ => false,
    }
}

#[test]
fn open_two_floors_keeps_both_live() {
    let (mut floors, _) = set();
    let a = proj("alpha");
    let b = proj("beta");

    let ha = floors.open(&a).unwrap();
    assert_eq!(floors.len(), 1);
    assert_eq!(ha.name, "alpha");
    assert_eq!(floors.current().unwrap().project_root(), a.as_path());

    let hb = floors.open(&b).unwrap();
    assert_eq!(floors.len(), 2);
    assert_eq!(hb.name, "beta");
    assert_eq!(floors.current().unwrap().project_root(), b.as_path());

    let listed: Vec<String> = floors.list().into_iter().map(|h| h.name).collect();
    assert!(listed.contains(&"alpha".into()));
    assert!(listed.contains(&"beta".into()));
    let shown = floors.format();
    assert!(shown.contains("alpha"), "{shown}");
    assert!(shown.contains("beta"), "{shown}");
    assert!(shown.contains('*'), "{shown}");
}

#[test]
fn switch_preserves_other_project_memory_and_floor() {
    let (mut floors, _) = set();
    let a = proj("mem-a");
    let b = proj("mem-b");

    floors.open(&a).unwrap();
    floors
        .current_mut()
        .unwrap()
        .remember(MemoryTier::Profile, "alpha-standing-fact")
        .unwrap();
    floors
        .current_mut()
        .unwrap()
        .open("PA", "alpha parent", "alpha body")
        .unwrap();

    floors.open(&b).unwrap();
    floors
        .current_mut()
        .unwrap()
        .remember(MemoryTier::Profile, "beta-standing-fact")
        .unwrap();
    floors
        .current_mut()
        .unwrap()
        .open("PB", "beta parent", "beta body")
        .unwrap();

    // B is current: must not see A's floor or memory.
    let b_shop = floors.current().unwrap();
    let b_mem = b_shop.memory();
    assert!(b_mem.profile.iter().any(|f| f == "beta-standing-fact"));
    assert!(!b_mem.profile.iter().any(|f| f == "alpha-standing-fact"));
    let b_state = b_shop.state().unwrap();
    assert!(b_state.parents.contains_key("PB"));
    assert!(!b_state.parents.contains_key("PA"));
    let b_floor = b_shop.floor();
    assert_eq!(b_floor.current.as_ref().map(|c| c.id.as_str()), Some("PB"));

    // Switch back to A without re-opening from scratch (path or unique name).
    floors.switch("mem-a").unwrap();
    assert_eq!(floors.current().unwrap().project_root(), a.as_path());
    floors.switch(b.to_str().unwrap()).unwrap();
    floors.switch(a.to_str().unwrap()).unwrap();
    assert_eq!(floors.len(), 2, "switch must not drop the other floor");
    let a_shop = floors.current().unwrap();
    assert_eq!(a_shop.project_root(), a.as_path());
    let a_mem = a_shop.memory();
    assert!(
        a_mem.profile.iter().any(|f| f == "alpha-standing-fact"),
        "A memory must survive opening B: {:?}",
        a_mem.profile
    );
    assert!(!a_mem.profile.iter().any(|f| f == "beta-standing-fact"));
    let a_state = a_shop.state().unwrap();
    assert!(a_state.parents.contains_key("PA"));
    assert!(!a_state.parents.contains_key("PB"));
    let a_floor = a_shop.floor();
    assert_eq!(a_floor.current.as_ref().map(|c| c.id.as_str()), Some("PA"));
    assert_eq!(a_floor.current.as_ref().unwrap().state, ParentState::Held);
}

#[test]
fn reopen_same_path_switches_and_does_not_duplicate() {
    let (mut floors, _) = set();
    let a = proj("same-a");
    let b = proj("same-b");
    floors.open(&a).unwrap();
    floors
        .current_mut()
        .unwrap()
        .remember(MemoryTier::Profile, "keep-me")
        .unwrap();
    floors.open(&b).unwrap();
    assert_eq!(floors.len(), 2);

    let again = floors.open(&a).unwrap();
    assert_eq!(floors.len(), 2);
    assert_eq!(again.name, "same-a");
    assert_eq!(floors.current().unwrap().project_root(), a.as_path());
    assert!(floors
        .current()
        .unwrap()
        .memory()
        .profile
        .iter()
        .any(|f| f == "keep-me"));
}

#[test]
fn get_other_floor_without_switching_current() {
    let (mut floors, _) = set();
    let a = proj("get-a");
    let b = proj("get-b");
    floors.open(&a).unwrap();
    floors
        .current_mut()
        .unwrap()
        .remember(MemoryTier::Log, "a-log-line")
        .unwrap();
    floors.open(&b).unwrap();
    assert_eq!(floors.current().unwrap().project_root(), b.as_path());

    let a_shop = floors.get(a.to_str().unwrap()).unwrap();
    assert_eq!(a_shop.project_root(), a.as_path());
    assert!(a_shop
        .memory()
        .log
        .iter()
        .any(|line| line.fact == "a-log-line"));
    assert_eq!(floors.current().unwrap().project_root(), b.as_path());
}

#[test]
fn close_one_floor_leaves_the_other() {
    let (mut floors, _) = set();
    let a = proj("close-a");
    let b = proj("close-b");
    floors.open(&a).unwrap();
    floors
        .current_mut()
        .unwrap()
        .remember(MemoryTier::Profile, "a-stays-on-disk")
        .unwrap();
    floors.open(&b).unwrap();
    floors.close(a.to_str().unwrap()).unwrap();
    assert_eq!(floors.len(), 1);
    assert_eq!(floors.current().unwrap().project_root(), b.as_path());

    // Disk memory for A is still there after the live handle is dropped.
    let recents = tmp_dir().join("recents.json");
    let reopened = Shop::open_project_with_recents(&a, recents).unwrap();
    assert!(reopened
        .memory()
        .profile
        .iter()
        .any(|f| f == "a-stays-on-disk"));
}

#[test]
fn refuse_platform_path() {
    let (mut floors, _) = set();
    match floors.open(Path::new(r"C:\TextPCB Platform")) {
        Ok(_) => panic!("expected ForbiddenWrite"),
        Err(ShopError::ForbiddenWrite(_)) => {}
        Err(e) => panic!("expected ForbiddenWrite, got {e}"),
    }
    match floors.open(Path::new(r"C:\\TextPCB Platform")) {
        Ok(_) => panic!("expected ForbiddenWrite"),
        Err(ShopError::ForbiddenWrite(_)) => {}
        Err(e) => panic!("expected ForbiddenWrite, got {e}"),
    }
    assert!(floors.is_empty());
}

#[test]
fn empty_and_unknown_are_wait_not_pass() {
    let (mut floors, _) = set();
    match floors.open(Path::new("")) {
        Err(e) => {
            assert!(is_wait(&e), "{e}");
            match e {
                ShopError::IncompleteEvidence(_) => {}
                other => panic!("empty path must be IncompleteEvidence, got {other}"),
            }
        }
        Ok(_) => panic!("empty path must WAIT"),
    }

    match floors.switch("") {
        Err(ShopError::IncompleteEvidence(msg)) => assert!(msg.contains("WAIT")),
        other => panic!("empty switch must WAIT, got {other:?}"),
    }
    match floors.switch("never-opened") {
        Err(ShopError::Wait(msg)) => assert!(msg.contains("never-opened")),
        other => panic!("unknown switch must WAIT, got {other:?}"),
    }
    match floors.current() {
        Err(ShopError::IncompleteEvidence(msg)) => assert!(msg.contains("WAIT")),
        Err(e) => panic!("no current must WAIT, got {e}"),
        Ok(_) => panic!("no current must WAIT, got Ok(Shop)"),
    }
    match floors.get("missing") {
        Err(ShopError::Wait(_)) => {}
        Err(e) => panic!("missing get must WAIT, got {e}"),
        Ok(_) => panic!("missing get must WAIT, got Ok(Shop)"),
    }
    match floors.close("") {
        Err(ShopError::IncompleteEvidence(_)) => {}
        other => panic!("empty close must WAIT, got {other:?}"),
    }
}

#[test]
fn switch_does_not_invent_verified_or_pass() {
    let (mut floors, _) = set();
    let a = proj("wait-a");
    let b = proj("wait-b");
    floors.open(&a).unwrap();
    floors
        .current_mut()
        .unwrap()
        .open("P", "held", "no verify")
        .unwrap();
    floors.open(&b).unwrap();
    floors.switch(a.to_str().unwrap()).unwrap();
    let state = floors.current().unwrap().state().unwrap();
    let parent = state
        .parents
        .get("P")
        .expect("held parent must still be on A");
    assert_eq!(parent.state, ParentState::Held);
    assert_ne!(parent.state, ParentState::Verified);
    assert_ne!(parent.state, ParentState::Closed);
    assert!(parent.verify.is_none());
    assert_ne!(
        parent.verify.as_ref().map(|v| v.status),
        Some(EvidenceStatus::Pass)
    );
}

#[test]
fn stores_are_isolated_under_each_project() {
    let (mut floors, recents) = set();
    let a = proj("iso-a");
    let b = proj("iso-b");
    floors.open(&a).unwrap();
    floors.open(&b).unwrap();
    let ha = floors.list().into_iter().find(|h| h.name == "iso-a").unwrap();
    let hb = floors.list().into_iter().find(|h| h.name == "iso-b").unwrap();
    assert!(ha.store.starts_with(&a), "{}", ha.store.display());
    assert!(hb.store.starts_with(&b), "{}", hb.store.display());
    assert_ne!(ha.store, hb.store);
    assert!(recents.is_file());
    let rec = fs::read_to_string(&recents).unwrap();
    assert!(rec.contains("iso-a"));
    assert!(rec.contains("iso-b"));
}
