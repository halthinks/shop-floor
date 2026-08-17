//! Worker roster. Enrolls capacity only — never a job, never a lane.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{Result, ShopError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backend {
    #[serde(rename = "cursor")]
    Cursor,
    #[serde(rename = "cursor-ultra")]
    CursorUltra,
    #[serde(rename = "grok-ultra")]
    GrokUltra,
    #[serde(rename = "grok-bot")]
    GrokBot,
    #[serde(rename = "grok-build")]
    GrokBuild,
}

impl Backend {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim() {
            "cursor" => Ok(Self::Cursor),
            "cursor-ultra" => Ok(Self::CursorUltra),
            "grok-ultra" => Ok(Self::GrokUltra),
            "grok-bot" => Ok(Self::GrokBot),
            "grok-build" => Ok(Self::GrokBuild),
            other => Err(ShopError::UnknownBackend(other.to_string())),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cursor => "cursor",
            Self::CursorUltra => "cursor-ultra",
            Self::GrokUltra => "grok-ultra",
            Self::GrokBot => "grok-bot",
            Self::GrokBuild => "grok-build",
        }
    }

    pub fn allows_empty_model(self) -> bool {
        matches!(self, Self::GrokUltra | Self::GrokBot | Self::GrokBuild)
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intelligence {
    pub backend: Backend,
    /// User-typed Cursor Ultra / custom model id. Shop never invents one.
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worker {
    pub peer: String,
    pub name: String,
    pub intelligence: Intelligence,
    /// Grant or revoke the whole GitHub skill pack. Default on.
    #[serde(default = "default_github_skills")]
    pub github_skills: bool,
}

fn default_github_skills() -> bool {
    true
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerRoster {
    #[serde(default)]
    pub workers: Vec<Worker>,
}

/// One seeded roster row. Capacity only — not a job, not a lane.
#[derive(Debug, Clone, Copy)]
pub struct SeededWorker {
    pub peer: &'static str,
    pub backend: &'static str,
    pub model: &'static str,
    pub title: &'static str,
}

/// Floor roster shown on first init. Capacity only — not jobs, not lanes.
pub const SEEDED_WORKERS: &[SeededWorker] = &[
    SeededWorker {
        peer: "cursor",
        backend: "cursor",
        model: "",
        title: "Cursor",
    },
    SeededWorker {
        peer: "grok-ultra",
        backend: "cursor-ultra",
        model: "grok",
        title: "Grok (Cursor Ultra)",
    },
    SeededWorker {
        peer: "grok-bot",
        backend: "grok-bot",
        model: "",
        title: "Orchestrator / Grok Bot",
    },
    SeededWorker {
        peer: "cursor-groksuperheavy",
        backend: "grok-build",
        model: "",
        title: "SuperGrokHeavy",
    },
];

impl WorkerRoster {
    pub fn get(&self, peer: &str) -> Option<&Worker> {
        self.workers.iter().find(|w| w.peer == peer)
    }

    pub fn get_mut(&mut self, peer: &str) -> Option<&mut Worker> {
        self.workers.iter_mut().find(|w| w.peer == peer)
    }
}
