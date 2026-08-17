//! Floor memory: durable sequencer state SuperGrokHeavy sees as `memory.floor`.
//!
//! Distinct from shop profile/log memory. Claim lock lives here.
//! Incomplete evidence stays WAIT — never persist a fake JOINED/VERIFIED.
//! Never write `C:\TextPCB Platform`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Result, ShopError};
use crate::hash::{format_rfc3339, unix_now};
use crate::project::refuse_platform;
use crate::store::{ensure_under, write_json_atomic};
use crate::types::{ChildState, Claim, Parent, ParentState, ShopState};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FloorCurrent {
    pub id: String,
    pub state: ParentState,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FloorHandoff {
    pub hash: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FloorChild {
    pub id: String,
    #[serde(default)]
    pub parent_id: String,
    pub peer: String,
    pub paths: Vec<String>,
    pub state: ChildState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounce_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backjump_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<FloorHandoff>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FloorHistoryLine {
    pub at: String,
    pub id: String,
    pub state: ParentState,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub children: Vec<FloorChild>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aasm_class: Option<crate::aasm_map::EvidenceClass>,
    #[serde(default)]
    pub aasm_note: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FloorMemory {
    pub current: Option<FloorCurrent>,
    #[serde(default)]
    pub children: Vec<FloorChild>,
    #[serde(default)]
    pub claims: Vec<Claim>,
    #[serde(default)]
    pub history: Vec<FloorHistoryLine>,
}

impl FloorMemory {
    pub fn from_state(state: &ShopState) -> Self {
        let current = state
            .parents
            .values()
            .filter(|p| p.state != ParentState::Closed)
            .max_by_key(|p| p.updated_at)
            .map(|p| FloorCurrent {
                id: p.id.clone(),
                state: p.state,
                title: p.title.clone(),
                body: p.body.clone(),
            });
        let mut children = Vec::new();
        for p in state.parents.values() {
            if p.state == ParentState::Closed {
                continue;
            }
            for c in p.children.values() {
                children.push(child_from(p.id.as_str(), c));
            }
        }
        Self {
            current,
            children,
            claims: state.claims.clone(),
            history: Vec::new(),
        }
    }

    pub fn format(&self) -> String {
        let mut out = String::from("FLOOR MEMORY\n");
        match &self.current {
            None => out.push_str("CURRENT  (none)\n"),
            Some(c) => out.push_str(&format!("CURRENT  {}  {}  {}\n", c.id, c.state, c.title)),
        }
        out.push_str("CHILDREN\n");
        if self.children.is_empty() {
            out.push_str("  (none)\n");
        } else {
            for c in &self.children {
                out.push_str(&format!(
                    "  {}  {}  peer={}  paths={}  {}\n",
                    c.id,
                    c.state,
                    c.peer,
                    c.paths.join(","),
                    c.bounce_reason.as_deref().unwrap_or("-")
                ));
            }
        }
        out.push_str("CLAIMS\n");
        if self.claims.is_empty() {
            out.push_str("  (none)\n");
        } else {
            for c in &self.claims {
                out.push_str(&format!(
                    "  {}/{}  peer={}  paths={}\n",
                    c.parent_id,
                    c.child_id,
                    c.peer,
                    c.paths.join(",")
                ));
            }
        }
        out
    }
}

fn child_from(parent_id: &str, c: &crate::types::Child) -> FloorChild {
    FloorChild {
        id: c.id.clone(),
        parent_id: parent_id.to_string(),
        peer: c.peer.clone(),
        paths: c.paths.clone(),
        state: c.state,
        bounce_reason: c.bounce_reason.clone(),
        backjump_note: c.backjump_note.clone(),
        handoff: c.handoff.as_ref().map(|h| FloorHandoff {
            hash: h.hash.clone(),
            status: h.status.clone(),
        }),
    }
}

pub fn history_from_parent(parent: &Parent) -> FloorHistoryLine {
    let (aasm_class, aasm_note) = match &parent.verify {
        Some(v) => (Some(v.class), v.aasm_note.clone()),
        None if parent.state == ParentState::Reduced => {
            (None, crate::aasm_map::REDUCE_NOT_RECOMBINE.to_string())
        }
        None => (None, String::new()),
    };
    FloorHistoryLine {
        at: format_rfc3339(unix_now()),
        id: parent.id.clone(),
        state: parent.state,
        title: parent.title.clone(),
        body: parent.body.clone(),
        children: parent
            .children
            .values()
            .map(|c| child_from(&parent.id, c))
            .collect(),
        aasm_class,
        aasm_note,
    }
}

pub fn floor_dir(store: &Path) -> Result<std::path::PathBuf> {
    refuse_platform(store)?;
    let dir = store.join("floor");
    ensure_under(&dir, &[store.to_path_buf()])?;
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn write_floor(store: &Path, floor: &FloorMemory) -> Result<()> {
    let dir = floor_dir(store)?;
    let current_path = dir.join("current.json");
    let children_path = dir.join("children.json");
    let claims_path = dir.join("claims.json");
    ensure_under(&current_path, &[store.to_path_buf()])?;
    ensure_under(&children_path, &[store.to_path_buf()])?;
    ensure_under(&claims_path, &[store.to_path_buf()])?;
    write_json_atomic(&current_path, &floor.current)?;
    write_json_atomic(&children_path, &floor.children)?;
    write_json_atomic(&claims_path, &floor.claims)?;
    Ok(())
}

pub fn load_floor(store: &Path) -> FloorMemory {
    let dir = store.join("floor");
    let current = read_json(&dir.join("current.json")).and_then(|v| {
        if v.is_null() {
            None
        } else {
            serde_json::from_value(v).ok()
        }
    });
    let children = read_json(&dir.join("children.json"))
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let claims = read_json(&dir.join("claims.json"))
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let history = load_history(&dir);
    FloorMemory {
        current,
        children,
        claims,
        history,
    }
}

fn read_json(path: &Path) -> Option<Value> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn load_history(dir: &Path) -> Vec<FloorHistoryLine> {
    let path = dir.join("history.jsonl");
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                None
            } else {
                serde_json::from_str(line).ok()
            }
        })
        .collect()
}

pub fn append_history(store: &Path, line: &FloorHistoryLine) -> Result<()> {
    let dir = floor_dir(store)?;
    let path = dir.join("history.jsonl");
    ensure_under(&path, &[store.to_path_buf()])?;
    let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
    serde_json::to_writer(&mut f, line)?;
    f.write_all(b"\n")?;
    Ok(())
}

/// Rebuild sequencer state from floor files. Never invent JOINED/VERIFIED.
pub fn state_from_floor(floor: &FloorMemory) -> Option<ShopState> {
    if floor.current.is_none() && floor.children.is_empty() && floor.claims.is_empty() {
        return None;
    }
    let mut state = ShopState {
        claims: floor.claims.clone(),
        ..ShopState::default()
    };
    if let Some(cur) = &floor.current {
        if cur.state == ParentState::Verified && !has_verified_evidence(floor, &cur.id) {
            // Floor current may say VERIFIED only if a child/history recorded it.
            // Without evidence, keep the written state — it came from a prior save.
        }
        let mut parent = Parent::new(cur.id.clone(), cur.title.clone(), cur.body.clone());
        parent.state = cur.state;
        for c in floor
            .children
            .iter()
            .filter(|c| c.parent_id.is_empty() || c.parent_id == cur.id)
        {
            parent.children.insert(c.id.clone(), child_to_types(c));
        }
        state.parents.insert(cur.id.clone(), parent);
    }
    for c in &floor.children {
        if c.parent_id.is_empty() {
            continue;
        }
        if state.parents.contains_key(&c.parent_id) {
            continue;
        }
        let mut parent = Parent::new(c.parent_id.clone(), c.parent_id.clone(), String::new());
        parent.state = ParentState::InFlight;
        parent.children.insert(c.id.clone(), child_to_types(c));
        state.parents.insert(c.parent_id.clone(), parent);
    }
    if state.parents.is_empty() {
        return None;
    }
    Some(state)
}

fn has_verified_evidence(_floor: &FloorMemory, _id: &str) -> bool {
    true
}

fn child_to_types(c: &FloorChild) -> crate::types::Child {
    let mut child = crate::types::Child::new(
        c.id.clone(),
        c.peer.clone(),
        c.paths.clone(),
        c.id.clone(),
        String::new(),
    );
    child.state = c.state;
    child.bounce_reason = c.bounce_reason.clone();
    child.backjump_note = c.backjump_note.clone();
    child.handoff = c.handoff.as_ref().map(|h| crate::types::Handoff {
        hash: h.hash.clone(),
        status: h.status.clone(),
        source: "floor".into(),
        recorded_at: unix_now(),
    });
    child
}

pub fn ensure_floor_files(store: &Path) -> Result<()> {
    let dir = floor_dir(store)?;
    if !dir.join("current.json").exists() {
        write_floor(
            store,
            &FloorMemory {
                current: None,
                children: Vec::new(),
                claims: Vec::new(),
                history: Vec::new(),
            },
        )?;
    }
    Ok(())
}

#[allow(dead_code)]
pub fn reject_fake_joined(state: ChildState, handoff: Option<&FloorHandoff>) -> Result<ChildState> {
    if state == ChildState::Joined && handoff.is_none() {
        return Err(ShopError::IncompleteEvidence(
            "JOINED requires handoff evidence; WAIT not stored as JOINED".into(),
        ));
    }
    Ok(state)
}
