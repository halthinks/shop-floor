//! Shop memory: standing facts and dated episodes.
//!
//! Distinct from floor memory (the sequencer). Never invent a fact.
//! Incomplete evidence is not stored as truth. Never write Platform paths.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{Result, ShopError};
use crate::hash::{format_rfc3339, unix_now};
use crate::project::refuse_platform;
use crate::store::{ensure_under, write_json_atomic};

pub const LOG_PACK: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryTier {
    Profile,
    Log,
}

impl MemoryTier {
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "profile" => Ok(Self::Profile),
            "log" => Ok(Self::Log),
            other => Err(ShopError::IncompleteEvidence(format!(
                "unknown memory tier '{other}' (profile|log)"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Log => "log",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryLogLine {
    pub at: String,
    pub fact: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShopMemory {
    #[serde(default)]
    pub profile: Vec<String>,
    #[serde(default)]
    pub log: Vec<MemoryLogLine>,
}

impl ShopMemory {
    pub fn pack_json(&self, floor: Value) -> Value {
        let log: Vec<&MemoryLogLine> = self.log.iter().rev().take(LOG_PACK).collect();
        let log: Vec<&MemoryLogLine> = log.into_iter().rev().collect();
        json!({
            "profile": self.profile,
            "log": log,
            "floor": floor,
        })
    }

    pub fn format(&self) -> String {
        let mut out = String::from("PROFILE\n");
        if self.profile.is_empty() {
            out.push_str("  (none)\n");
        } else {
            for f in &self.profile {
                out.push_str(&format!("  {f}\n"));
            }
        }
        out.push_str("LOG\n");
        if self.log.is_empty() {
            out.push_str("  (none)\n");
        } else {
            for line in self
                .log
                .iter()
                .rev()
                .take(40)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
            {
                out.push_str(&format!("  {}  {}\n", line.at, line.fact));
            }
        }
        out
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ProfileFile {
    #[serde(default)]
    facts: Vec<String>,
}

pub fn refuse_memory_text(text: &str) -> Result<()> {
    let t = text.trim();
    if t.is_empty() {
        return Err(ShopError::IncompleteEvidence(
            "empty fact is not stored".into(),
        ));
    }
    if t.contains("TextPCB Platform") || t.contains("C:\\TextPCB") || t.contains(r"C:\TextPCB") {
        return Err(ShopError::ForbiddenWrite(std::path::PathBuf::from(t)));
    }
    Ok(())
}

pub fn memory_dir(store: &Path) -> Result<std::path::PathBuf> {
    refuse_platform(store)?;
    let dir = store.join("memory");
    ensure_under(&dir, &[store.to_path_buf()])?;
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn load_memory(store: &Path) -> ShopMemory {
    let dir = store.join("memory");
    let profile = load_profile(&dir);
    let log = load_log(&dir);
    ShopMemory { profile, log }
}

fn load_profile(dir: &Path) -> Vec<String> {
    let path = dir.join("profile.json");
    let Ok(bytes) = fs::read(&path) else {
        return Vec::new();
    };
    let Ok(file) = serde_json::from_slice::<ProfileFile>(&bytes) else {
        if let Ok(arr) = serde_json::from_slice::<Vec<String>>(&bytes) {
            return arr;
        }
        return Vec::new();
    };
    file.facts
}

fn load_log(dir: &Path) -> Vec<MemoryLogLine> {
    let path = dir.join("log.jsonl");
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

pub fn remember(store: &Path, tier: MemoryTier, fact: &str) -> Result<ShopMemory> {
    refuse_memory_text(fact)?;
    let dir = memory_dir(store)?;
    let fact = fact.trim().to_string();
    match tier {
        MemoryTier::Profile => {
            let mut facts = load_profile(&dir);
            if !facts.iter().any(|f| f == &fact) {
                facts.push(fact);
            }
            let path = dir.join("profile.json");
            ensure_under(&path, &[store.to_path_buf()])?;
            write_json_atomic(&path, &ProfileFile { facts })?;
        }
        MemoryTier::Log => {
            append_log(store, &fact)?;
        }
    }
    Ok(load_memory(store))
}

pub fn append_log(store: &Path, fact: &str) -> Result<()> {
    refuse_memory_text(fact)?;
    let dir = memory_dir(store)?;
    let path = dir.join("log.jsonl");
    ensure_under(&path, &[store.to_path_buf()])?;
    let line = MemoryLogLine {
        at: format_rfc3339(unix_now()),
        fact: fact.trim().to_string(),
    };
    let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
    serde_json::to_writer(&mut f, &line)?;
    f.write_all(b"\n")?;
    Ok(())
}

pub fn forget(store: &Path, fact: &str) -> Result<ShopMemory> {
    refuse_memory_text(fact)?;
    let dir = memory_dir(store)?;
    let fact = fact.trim();
    let facts: Vec<String> = load_profile(&dir)
        .into_iter()
        .filter(|f| f != fact)
        .collect();
    let path = dir.join("profile.json");
    ensure_under(&path, &[store.to_path_buf()])?;
    write_json_atomic(&path, &ProfileFile { facts })?;
    Ok(load_memory(store))
}
