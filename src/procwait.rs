//! Live process wait/stop state.
//!
//! Shop sequences work. AASM stays the authority calculus. Waiting on a pid
//! records Evidence only — never VERIFIED, never CLOSED, never a fake PASS.
//! Incomplete evidence (dead pid, no exit) is WAIT / UNKNOWN.
//!
//! `shop stop` is an explicit operator action. Waiting is not stopping.
//! An alive pid is a working status pill. A dead pid is not working.

use std::fs;
use std::path::Path;
use std::process::{Child, Command};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::aasm_map::EvidenceClass;
use crate::error::{Result, ShopError};
use crate::hash::unix_now;
use crate::types::EvidenceStatus;

pub const WAIT_EVIDENCE_NOTE: &str =
    "wait exit is Evidence only; not VERIFIED; not CLOSE; shop verify is the only VERIFIED path";
pub const WAIT_UNKNOWN_NOTE: &str = "AASM UNKNOWN; dead pid with no exit; not PASS; not VERIFIED";
pub const STOP_NOTE: &str = "explicit operator stop; waiting is not stopping";

/// Alive pid is working. Dead pid is not working. Never fake working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PidLiveness {
    Working,
    Dead,
}

impl std::fmt::Display for PidLiveness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Working => "WORKING",
            Self::Dead => "DEAD",
        })
    }
}

impl PidLiveness {
    pub fn status_pill(self) -> &'static str {
        status_pill(self)
    }

    pub fn is_working(self) -> bool {
        matches!(self, Self::Working)
    }
}

/// Status-pill word. Alive → working. Dead → dead (never working).
pub fn status_pill(liveness: PidLiveness) -> &'static str {
    match liveness {
        PidLiveness::Working => "working",
        PidLiveness::Dead => "dead",
    }
}

/// Evidence recorded by `shop wait`. Not a verify record. Never promotes a parent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitEvidence {
    pub pid: u32,
    pub exit_code: Option<i32>,
    pub status: EvidenceStatus,
    pub class: EvidenceClass,
    pub aasm_note: String,
    pub recorded_at: u64,
}

impl WaitEvidence {
    pub fn is_unknown(&self) -> bool {
        self.class == EvidenceClass::Unknown && self.exit_code.is_none()
    }
}

/// Explicit operator stop. Distinct from wait.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopRecord {
    pub pid: u32,
    pub explicit: bool,
    pub liveness: PidLiveness,
    pub note: String,
    pub recorded_at: u64,
}

/// Optional bind of a pid onto a parent/child/peer. Bind is context, not authority.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessBind {
    pub parent_id: Option<String>,
    pub child_id: Option<String>,
    pub peer: Option<String>,
}

impl ProcessBind {
    pub fn new(parent_id: Option<&str>, child_id: Option<&str>, peer: Option<&str>) -> Self {
        Self {
            parent_id: parent_id.map(|s| s.to_string()).filter(|s| !s.is_empty()),
            child_id: child_id.map(|s| s.to_string()).filter(|s| !s.is_empty()),
            peer: peer.map(|s| s.to_string()).filter(|s| !s.is_empty()),
        }
    }
}

/// One tracked pid. Stored apart from `Parent.verify` so wait cannot CLOSE.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveProcess {
    pub pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
    pub liveness: PidLiveness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait: Option<WaitEvidence>,
    #[serde(default)]
    pub explicit_stop: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_note: Option<String>,
    pub updated_at: u64,
}

impl LiveProcess {
    pub fn observed(pid: u32, bind: ProcessBind) -> Self {
        Self {
            pid,
            parent_id: bind.parent_id,
            child_id: bind.child_id,
            peer: bind.peer,
            liveness: pid_liveness(pid),
            exit_code: None,
            wait: None,
            explicit_stop: false,
            stop_note: None,
            updated_at: unix_now(),
        }
    }

    pub fn status_pill(&self) -> &'static str {
        self.liveness.status_pill()
    }

    pub fn is_working(&self) -> bool {
        self.liveness.is_working()
    }

    pub fn refresh(&mut self) {
        self.liveness = pid_liveness(self.pid);
        self.updated_at = unix_now();
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcWaitLedger {
    #[serde(default)]
    pub processes: Vec<LiveProcess>,
}

impl ProcWaitLedger {
    pub fn get(&self, pid: u32) -> Option<&LiveProcess> {
        self.processes.iter().find(|p| p.pid == pid)
    }

    pub fn get_mut(&mut self, pid: u32) -> Option<&mut LiveProcess> {
        self.processes.iter_mut().find(|p| p.pid == pid)
    }

    pub fn latest_for_peer(&self, peer: &str) -> Option<&LiveProcess> {
        self.processes
            .iter()
            .filter(|p| p.peer.as_deref() == Some(peer))
            .max_by_key(|p| p.updated_at)
    }

    pub fn upsert(&mut self, proc: LiveProcess) {
        if let Some(existing) = self.get_mut(proc.pid) {
            if existing.parent_id.is_none() {
                existing.parent_id = proc.parent_id;
            }
            if existing.child_id.is_none() {
                existing.child_id = proc.child_id;
            }
            if existing.peer.is_none() {
                existing.peer = proc.peer;
            }
            existing.liveness = proc.liveness;
            existing.updated_at = proc.updated_at;
            if proc.exit_code.is_some() {
                existing.exit_code = proc.exit_code;
            }
            if proc.wait.is_some() {
                existing.wait = proc.wait;
            }
            if proc.explicit_stop {
                existing.explicit_stop = true;
                existing.stop_note = proc.stop_note;
            }
        } else {
            self.processes.push(proc);
        }
    }

    pub fn refresh_liveness(&mut self) {
        for p in &mut self.processes {
            p.refresh();
        }
    }
}

pub fn load_ledger(root: &Path) -> Result<ProcWaitLedger> {
    let path = root.join("procwait.json");
    if !path.exists() {
        return Ok(ProcWaitLedger::default());
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

pub fn save_ledger(root: &Path, ledger: &ProcWaitLedger) -> Result<()> {
    crate::store::write_json_atomic(&root.join("procwait.json"), ledger)
}

/// Classify a wait observation. Exit is Evidence. Missing exit is UNKNOWN.
/// Never PASS — wait is not `shop verify`.
pub fn evidence_for_exit(pid: u32, exit_code: Option<i32>) -> WaitEvidence {
    let (class, note) = match exit_code {
        Some(_) => (EvidenceClass::Inconclusive, WAIT_EVIDENCE_NOTE),
        None => (EvidenceClass::Unknown, WAIT_UNKNOWN_NOTE),
    };
    WaitEvidence {
        pid,
        exit_code,
        status: EvidenceStatus::Wait,
        class,
        aasm_note: note.to_string(),
        recorded_at: unix_now(),
    }
}

pub fn pid_liveness(pid: u32) -> PidLiveness {
    if is_working_pid(pid) {
        PidLiveness::Working
    } else {
        PidLiveness::Dead
    }
}

fn is_working_pid(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(s) => s,
            Err(_) => return false,
        };
        match proc_state_char(&stat) {
            Some('Z') | Some('X') | Some('x') => false,
            Some(_) => true,
            None => false,
        }
    }
    #[cfg(not(unix))]
    {
        // Cannot confirm a live pid. Never fake working.
        let _ = pid;
        false
    }
}

fn proc_state_char(stat: &str) -> Option<char> {
    let rest = stat.rsplit_once(')')?.1;
    rest.split_whitespace().next()?.chars().next()
}

pub fn refuse_self_or_init(pid: u32, op: &str) -> Result<()> {
    if pid == 0 || pid == 1 || pid == std::process::id() {
        return Err(ShopError::Wait(format!(
            "refusing to {op} pid 0, 1, or the shop process itself"
        )));
    }
    Ok(())
}

/// Block until the pid is dead. Does not reap a child and does not yield an exit.
pub fn wait_until_dead(pid: u32) {
    while pid_liveness(pid) == PidLiveness::Working {
        thread::sleep(Duration::from_millis(20));
    }
}

/// Wait on an owned child so the exit code can be recorded as Evidence.
pub fn wait_owned_child(child: &mut Child) -> Option<i32> {
    match child.wait() {
        Ok(status) => status.code(),
        Err(_) => None,
    }
}

/// Explicit stop. Waiting must not call this.
pub fn signal_stop(pid: u32) -> Result<()> {
    refuse_self_or_init(pid, "stop")?;
    if pid_liveness(pid) == PidLiveness::Dead {
        return Ok(());
    }
    let _ = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status();
    for _ in 0..40 {
        if pid_liveness(pid) == PidLiveness::Dead {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    }
    if pid_liveness(pid) == PidLiveness::Working {
        let _ = Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status();
        for _ in 0..20 {
            if pid_liveness(pid) == PidLiveness::Dead {
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
    Ok(())
}

pub fn apply_wait(ledger: &mut ProcWaitLedger, pid: u32, exit_code: Option<i32>) -> WaitEvidence {
    let evidence = evidence_for_exit(pid, exit_code);
    let now = unix_now();
    if let Some(proc) = ledger.get_mut(pid) {
        proc.liveness = PidLiveness::Dead;
        proc.exit_code = exit_code;
        proc.wait = Some(evidence.clone());
        proc.updated_at = now;
        // Waiting is not stopping. Leave explicit_stop untouched.
    } else {
        ledger.processes.push(LiveProcess {
            pid,
            parent_id: None,
            child_id: None,
            peer: None,
            liveness: PidLiveness::Dead,
            exit_code,
            wait: Some(evidence.clone()),
            explicit_stop: false,
            stop_note: None,
            updated_at: now,
        });
    }
    evidence
}

pub fn apply_stop(ledger: &mut ProcWaitLedger, pid: u32) -> StopRecord {
    let now = unix_now();
    let liveness = pid_liveness(pid);
    let rec = StopRecord {
        pid,
        explicit: true,
        liveness,
        note: STOP_NOTE.to_string(),
        recorded_at: now,
    };
    if let Some(proc) = ledger.get_mut(pid) {
        proc.liveness = liveness;
        proc.explicit_stop = true;
        proc.stop_note = Some(STOP_NOTE.to_string());
        proc.updated_at = now;
    } else {
        ledger.processes.push(LiveProcess {
            pid,
            parent_id: None,
            child_id: None,
            peer: None,
            liveness,
            exit_code: None,
            wait: None,
            explicit_stop: true,
            stop_note: Some(STOP_NOTE.to_string()),
            updated_at: now,
        });
    }
    rec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_exit_is_evidence_never_pass() {
        let ev = evidence_for_exit(42, Some(0));
        assert_eq!(ev.exit_code, Some(0));
        assert_eq!(ev.status, EvidenceStatus::Wait);
        assert_ne!(ev.class, EvidenceClass::Pass);
        assert!(ev.aasm_note.contains("Evidence"));
        assert!(!ev.aasm_note.contains("VERIFIED") || ev.aasm_note.contains("not VERIFIED"));
    }

    #[test]
    fn dead_pid_no_exit_is_unknown() {
        let ev = evidence_for_exit(99, None);
        assert!(ev.is_unknown());
        assert_eq!(ev.class, EvidenceClass::Unknown);
        assert_eq!(ev.status, EvidenceStatus::Wait);
        assert_ne!(ev.class, EvidenceClass::Pass);
    }

    #[test]
    fn pills_distinguish_working_from_dead() {
        assert_eq!(status_pill(PidLiveness::Working), "working");
        assert_eq!(status_pill(PidLiveness::Dead), "dead");
        assert_ne!(status_pill(PidLiveness::Dead), "working");
        assert!(PidLiveness::Working.is_working());
        assert!(!PidLiveness::Dead.is_working());
    }

    #[test]
    fn wait_does_not_mark_explicit_stop() {
        let mut ledger = ProcWaitLedger::default();
        apply_wait(&mut ledger, 7, Some(1));
        let p = ledger.get(7).unwrap();
        assert!(!p.explicit_stop);
        assert!(p.wait.is_some());
        apply_stop(&mut ledger, 7);
        let p = ledger.get(7).unwrap();
        assert!(p.explicit_stop);
    }

    #[test]
    fn unknown_note_is_not_verify_pass() {
        assert_eq!(evidence_for_exit(1, None).aasm_note, WAIT_UNKNOWN_NOTE);
        assert_eq!(evidence_for_exit(1, Some(2)).aasm_note, WAIT_EVIDENCE_NOTE);
    }
}
