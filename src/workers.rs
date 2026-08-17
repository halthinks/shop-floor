//! Worker roster. Enrolls capacity only — never a job, never a lane.
//!
//! ROADMAP 1: launch the assigned AI (Grok Build / Cursor / tagged backend).
//! A name on this roster is not a running worker. Isolation is a child
//! worktree. A running worker is an observed pid, never invented here.

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
    /// Tagged launch backends for ROADMAP 1. These are roster tags, not pids.
    pub const LAUNCH_TAGS: &'static [Self] = &[
        Self::Cursor,
        Self::CursorUltra,
        Self::GrokUltra,
        Self::GrokBot,
        Self::GrokBuild,
    ];

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

    /// Every known backend is a launch tag. Unknown strings never parse here.
    pub fn is_tagged_launch_backend(self) -> bool {
        Self::LAUNCH_TAGS.contains(&self)
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

impl Worker {
    /// Build enrolled capacity from a seed row. Still not a running worker.
    pub fn from_seed(seed: SeededWorker, github_skills: bool) -> Result<Self> {
        Ok(Self {
            peer: seed.peer.to_string(),
            name: seed.title.to_string(),
            intelligence: Intelligence {
                backend: Backend::parse(seed.backend)?,
                model: seed.model.to_string(),
            },
            github_skills,
        })
    }

    /// Tagged backend this capacity would launch if assigned and configured.
    pub fn tagged_backend(&self) -> Backend {
        self.intelligence.backend
    }

    /// Roster rows have no pid. Never invent one from the name.
    pub fn roster_pid(&self) -> Option<u32> {
        None
    }

    /// A name on the roster is capacity, not a live process.
    pub fn is_running_worker(&self) -> bool {
        false
    }

    /// Worktrees are cut per assigned child, never owned by a roster name.
    pub fn owns_child_worktree(&self) -> bool {
        false
    }

    /// Tagged backend + model the runner would look up. Not a pid.
    pub fn launch_identity(&self) -> LaunchIdentity {
        LaunchIdentity {
            peer: self.peer.clone(),
            backend: self.intelligence.backend,
            model: self.intelligence.model.clone(),
        }
    }
}

/// Observed live worker. Construct only from a real pid — never from a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveWorker {
    pub peer: String,
    pub backend: Backend,
    pub pid: u32,
}

impl LiveWorker {
    /// Pid 0 is not a process. A roster name cannot fill this in.
    pub fn observed(peer: impl Into<String>, backend: Backend, pid: u32) -> Option<Self> {
        if pid == 0 {
            None
        } else {
            Some(Self {
                peer: peer.into(),
                backend,
                pid,
            })
        }
    }

    /// Observed pid is the running worker. Name/backend never suffice.
    pub fn is_running_worker(&self) -> bool {
        self.pid != 0
    }

    /// Isolation is a child worktree. Live observation here is pid only.
    pub fn owns_child_worktree(&self) -> bool {
        false
    }
}

/// What the roster would launch if assigned and configured.
/// Not a pid. Not a worktree. Runner looks up the command; this file does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchIdentity {
    pub peer: String,
    pub backend: Backend,
    pub model: String,
}

impl LaunchIdentity {
    /// A launch tag is capacity. It is not a running worker.
    pub fn is_running_worker(&self) -> bool {
        false
    }

    /// Isolation is a child worktree. This identity does not own one.
    pub fn owns_child_worktree(&self) -> bool {
        false
    }

    /// Backend id `run.json` would key. Roster never invents the command.
    pub fn backend_id(&self) -> &'static str {
        self.backend.as_str()
    }

    /// Pid is observed elsewhere. Identity cannot invent one.
    pub fn pid(&self) -> Option<u32> {
        None
    }
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

impl SeededWorker {
    /// Seeds enroll names. They do not start a process.
    pub fn is_running_worker(self) -> bool {
        false
    }
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

    /// Presence here is enrolled capacity, not liveness.
    pub fn is_enrolled(&self, peer: &str) -> bool {
        self.get(peer).is_some()
    }

    /// A roster cannot report running workers. Pid comes from procwait.
    pub fn live_from_roster(&self) -> Vec<LiveWorker> {
        Vec::new()
    }

    /// Bind an observed pid to enrolled capacity. Unknown peer or pid 0 is None.
    pub fn bind_live(&self, peer: &str, pid: u32) -> Option<LiveWorker> {
        let worker = self.get(peer)?;
        LiveWorker::observed(peer, worker.tagged_backend(), pid)
    }

    /// Launch tag for an enrolled peer. Missing peer is None, never invented.
    pub fn launch_identity(&self, peer: &str) -> Option<LaunchIdentity> {
        self.get(peer).map(Worker::launch_identity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_names_are_capacity_not_running() {
        assert!(!SEEDED_WORKERS.is_empty());
        for seed in SEEDED_WORKERS {
            assert!(!seed.is_running_worker());
            let w = Worker::from_seed(*seed, true).unwrap();
            assert_eq!(w.peer, seed.peer);
            assert_eq!(w.name, seed.title);
            assert_eq!(w.tagged_backend().as_str(), seed.backend);
            assert!(w.tagged_backend().is_tagged_launch_backend());
            assert_eq!(w.roster_pid(), None);
            assert!(!w.is_running_worker());
            assert!(!w.owns_child_worktree());
        }
    }

    #[test]
    fn roster_cannot_invent_a_live_worker() {
        let mut roster = WorkerRoster::default();
        for seed in SEEDED_WORKERS {
            roster.workers.push(Worker::from_seed(*seed, true).unwrap());
        }
        assert!(roster.is_enrolled("cursor"));
        assert!(roster.is_enrolled("cursor-groksuperheavy"));
        assert!(!roster.is_enrolled("alice"));
        assert!(roster.live_from_roster().is_empty());
        assert_eq!(roster.bind_live("cursor", 0), None);
        assert_eq!(roster.bind_live("missing", 4242), None);

        let live = roster
            .bind_live("cursor-groksuperheavy", 4242)
            .expect("enrolled peer + real pid is a running worker");
        assert_eq!(live.peer, "cursor-groksuperheavy");
        assert_eq!(live.backend, Backend::GrokBuild);
        assert_eq!(live.pid, 4242);
        assert!(live.backend.is_tagged_launch_backend());
        assert!(live.is_running_worker());
        assert!(!live.owns_child_worktree());
    }

    #[test]
    fn launch_identity_is_not_a_running_worker() {
        let w = Worker::from_seed(SEEDED_WORKERS[3], true).unwrap();
        let id = w.launch_identity();
        assert_eq!(id.peer, "cursor-groksuperheavy");
        assert_eq!(id.backend, Backend::GrokBuild);
        assert_eq!(id.backend_id(), "grok-build");
        assert_eq!(id.model, "");
        assert_eq!(id.pid(), None);
        assert!(!id.is_running_worker());
        assert!(!id.owns_child_worktree());
        assert!(id.backend.is_tagged_launch_backend());

        let mut roster = WorkerRoster::default();
        roster.workers.push(w);
        assert_eq!(roster.launch_identity("missing"), None);
        let again = roster
            .launch_identity("cursor-groksuperheavy")
            .expect("enrolled peer has a launch tag");
        assert_eq!(again.backend, Backend::GrokBuild);
        assert!(!again.is_running_worker());
        assert_eq!(again.pid(), None);
    }

    #[test]
    fn launch_tags_are_the_known_backends() {
        for tag in Backend::LAUNCH_TAGS {
            assert!(tag.is_tagged_launch_backend());
            assert_eq!(Backend::parse(tag.as_str()).unwrap(), *tag);
        }
        assert!(Backend::parse("not-a-backend").is_err());
        assert_eq!(LiveWorker::observed("alice", Backend::GrokBot, 0), None);
    }

    #[test]
    fn roster_json_stays_capacity_only() {
        let w = Worker::from_seed(SEEDED_WORKERS[0], true).unwrap();
        let v = serde_json::to_value(&w).unwrap();
        assert_eq!(v["peer"], "cursor");
        assert_eq!(v["name"], "Cursor");
        assert_eq!(v["intelligence"]["backend"], "cursor");
        assert!(v.get("pid").is_none());
        assert!(v.get("worktree").is_none());
        assert!(v.get("lane").is_none());
    }
}
