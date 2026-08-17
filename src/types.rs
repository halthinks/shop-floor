//! Durable shop-floor types.
//!
//! Parent: HELD -> IN_FLIGHT -> REDUCE_READY -> REDUCED -> VERIFY_WAIT|VERIFIED -> CLOSED
//! Child:  ASSIGNED -> JOINED | BOUNCED (bounce may return to ASSIGNED on the same peer)
//!
//! AASM stays the authority calculus. These states sequence work; they are not truth.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::aasm_map::EvidenceClass;
use crate::hash::unix_now;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ParentState {
    Held,
    InFlight,
    ReduceReady,
    Reduced,
    VerifyWait,
    Verified,
    Closed,
}

impl fmt::Display for ParentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Held => "HELD",
            Self::InFlight => "IN_FLIGHT",
            Self::ReduceReady => "REDUCE_READY",
            Self::Reduced => "REDUCED",
            Self::VerifyWait => "VERIFY_WAIT",
            Self::Verified => "VERIFIED",
            Self::Closed => "CLOSED",
        })
    }
}

impl ParentState {
    pub fn is_open(self) -> bool {
        !matches!(self, Self::Closed)
    }

    pub fn allows_split(self) -> bool {
        matches!(self, Self::Held | Self::InFlight)
    }

    pub fn allows_assign(self) -> bool {
        matches!(self, Self::Held | Self::InFlight)
    }

    pub fn allows_child_update(self) -> bool {
        matches!(self, Self::Held | Self::InFlight)
    }

    pub fn allows_verify(self) -> bool {
        matches!(self, Self::Reduced | Self::VerifyWait | Self::Verified)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ChildState {
    Assigned,
    Joined,
    Bounced,
}

impl fmt::Display for ChildState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Assigned => "ASSIGNED",
            Self::Joined => "JOINED",
            Self::Bounced => "BOUNCED",
        })
    }
}

impl ChildState {
    pub fn holds_claim(self) -> bool {
        matches!(self, Self::Assigned | Self::Bounced)
    }
}

/// Shop CLI word. Internally paired with [`crate::aasm_map::EvidenceClass`].
/// WAIT is INCONCLUSIVE | INFORMATION_GAP | UNKNOWN — never promoted to PASS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EvidenceStatus {
    Pass,
    Wait,
}

impl Default for EvidenceStatus {
    fn default() -> Self {
        Self::Wait
    }
}

impl fmt::Display for EvidenceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pass => "PASS",
            Self::Wait => "WAIT",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handoff {
    /// Content fingerprint. Evidence identity, not authority.
    pub hash: String,
    /// Worker-claimed PASS or DONE. Evidence, not truth.
    pub status: String,
    pub source: String,
    pub recorded_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Child {
    pub id: String,
    pub peer: String,
    pub paths: Vec<String>,
    pub title: String,
    pub body: String,
    pub state: ChildState,
    pub dispatched: bool,
    pub handoff: Option<Handoff>,
    pub bounce_reason: Option<String>,
    /// Causal backjump note. Not an AASM type named bounce.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backjump_note: Option<String>,
    /// Intended child ref `shop/<parent>/<child>`. Recorded even when not created.
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub branch_created: bool,
}

impl Child {
    pub fn new(id: String, peer: String, paths: Vec<String>, title: String, body: String) -> Self {
        Self {
            id,
            peer,
            paths,
            title,
            body,
            state: ChildState::Assigned,
            dispatched: false,
            handoff: None,
            bounce_reason: None,
            backjump_note: None,
            branch: None,
            branch_created: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyRecord {
    pub cmd: String,
    pub exit_code: Option<i32>,
    pub last_lines: String,
    pub status: EvidenceStatus,
    #[serde(default)]
    pub class: EvidenceClass,
    #[serde(default)]
    pub aasm_note: String,
    pub recorded_at: u64,
}

impl VerifyRecord {
    pub fn classify(mut self) -> Self {
        self.class =
            crate::aasm_map::classify_verify(self.status, self.exit_code, &self.last_lines);
        if self.status == EvidenceStatus::Pass && self.class != EvidenceClass::Pass {
            self.status = EvidenceStatus::Wait;
        }
        if self.class != EvidenceClass::Pass {
            self.status = EvidenceStatus::Wait;
        }
        self.aasm_note = crate::aasm_map::note_for(self.class).to_string();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parent {
    pub id: String,
    pub title: String,
    pub body: String,
    pub state: ParentState,
    pub children: BTreeMap<String, Child>,
    pub verify_cmd: Option<String>,
    pub verify: Option<VerifyRecord>,
    pub reduce_note: Option<String>,
    #[serde(default)]
    pub github_pr_url: Option<String>,
    #[serde(default)]
    pub github_pr_number: Option<u64>,
    #[serde(default)]
    pub github_pr_status: Option<EvidenceStatus>,
    #[serde(default)]
    pub github_pr_reason: Option<String>,
    #[serde(default)]
    pub github_merged: bool,
    #[serde(default)]
    pub github_merge_status: Option<EvidenceStatus>,
    #[serde(default)]
    pub github_merge_reason: Option<String>,
    /// AuthorityLease scope. Empty means children define claims; overlap still rejected.
    #[serde(default)]
    pub claimed_paths: Vec<String>,
    #[serde(default)]
    pub lease_note: String,
    pub opened_at: u64,
    pub updated_at: u64,
}

impl Parent {
    pub fn new(id: String, title: String, body: String) -> Self {
        let now = unix_now();
        Self {
            id,
            title,
            body,
            state: ParentState::Held,
            children: BTreeMap::new(),
            verify_cmd: None,
            verify: None,
            reduce_note: None,
            github_pr_url: None,
            github_pr_number: None,
            github_pr_status: None,
            github_pr_reason: None,
            github_merged: false,
            github_merge_status: None,
            github_merge_reason: None,
            claimed_paths: Vec::new(),
            lease_note: String::new(),
            opened_at: now,
            updated_at: now,
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = unix_now();
    }

    pub fn all_children_joined(&self) -> bool {
        !self.children.is_empty()
            && self
                .children
                .values()
                .all(|c| c.state == ChildState::Joined)
    }
}

/// Global in-flight allowed_paths claim. One row per in-flight child.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub parent_id: String,
    pub child_id: String,
    pub peer: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReduceChild {
    pub id: String,
    pub peer: String,
    pub paths: Vec<String>,
    pub handoff_hash: Option<String>,
    pub handoff_status: Option<String>,
    pub state: ChildState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReducePackage {
    pub parent_id: String,
    pub note: String,
    pub state: ParentState,
    pub children: Vec<ReduceChild>,
    #[serde(default)]
    pub github_repo: Option<String>,
    #[serde(default)]
    pub child_branches: Vec<String>,
    #[serde(default)]
    pub pull_request_url: Option<String>,
    #[serde(default)]
    pub pull_request_number: Option<u64>,
    #[serde(default)]
    pub pull_request_status: EvidenceStatus,
    #[serde(default)]
    pub pull_request_reason: Option<String>,
    /// Shop evidence listing. Not an AASM recombine claim.
    #[serde(default)]
    pub aasm_note: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShopState {
    pub parents: BTreeMap<String, Parent>,
    pub claims: Vec<Claim>,
}
