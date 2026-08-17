//! Truth panel. Shop answers "what is happening" from store + mailbox + GitHub
//! records. Missing pieces are WAIT with the exact gap. Never invent state.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::mailbox::MailboxObservation;
use crate::types::{ChildState, Claim, EvidenceStatus, ParentState};
use crate::workers::{Intelligence, Worker};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildView {
    pub id: String,
    pub peer: String,
    pub state: ChildState,
    pub paths: Vec<String>,
    pub branch: Option<String>,
    pub dispatched: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParentView {
    pub id: String,
    pub title: String,
    pub state: ParentState,
    pub children: Vec<ChildView>,
    pub verify: Option<EvidenceStatus>,
    pub github_pr_url: Option<String>,
    pub github_pr_status: Option<EvidenceStatus>,
    pub github_merged: bool,
    pub github_merge_status: Option<EvidenceStatus>,
    pub github_merge_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubAwareness {
    pub repo: Option<String>,
    pub default_branch: Option<String>,
    pub authenticated: bool,
    pub token_present: bool,
    pub wait: Option<String>,
    pub checks: Value,
    /// None means WAIT — never invent a zero.
    pub issue_count: Option<u64>,
    pub pr_count: Option<u64>,
    pub check_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerView {
    pub peer: String,
    pub name: String,
    pub intelligence: Intelligence,
    pub github_skills: bool,
    /// idle | assigned | working | bounced | wait | live
    pub action: String,
    pub action_reason: Option<String>,
    pub unread: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectView {
    pub name: String,
    pub path: String,
    pub store: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Awareness {
    pub store_ok: bool,
    pub project: ProjectView,
    pub workers: Vec<WorkerView>,
    pub claims: Vec<Claim>,
    pub parents: Vec<ParentView>,
    pub mailbox: MailboxObservation,
    pub github: GitHubAwareness,
    /// Newest first. Only events that were written to `.shop/events.jsonl`.
    pub events: Vec<Value>,
    /// Exact missing pieces. Empty only when nothing is WAIT.
    pub waits: Vec<String>,
}

impl WorkerView {
    pub fn from_worker(w: Worker, parents: &[ParentView], mailbox: &MailboxObservation) -> Self {
        let (action, action_reason, unread) = classify_action(&w.peer, parents, mailbox);
        Self {
            peer: w.peer,
            name: w.name,
            intelligence: w.intelligence,
            github_skills: w.github_skills,
            action,
            action_reason,
            unread,
        }
    }
}

/// Derive the live action pill from store + mailbox observe.
/// `working` only when a handoff is observed while the child is still ASSIGNED.
/// Unknown is WAIT. Never fake working.
pub fn classify_action(
    peer: &str,
    parents: &[ParentView],
    mailbox: &MailboxObservation,
) -> (String, Option<String>, usize) {
    let mut has_assigned = false;
    let mut has_bounced = false;
    for p in parents {
        if matches!(p.state, ParentState::Closed) {
            continue;
        }
        for c in &p.children {
            if c.peer != peer {
                continue;
            }
            match c.state {
                ChildState::Bounced => has_bounced = true,
                ChildState::Assigned => has_assigned = true,
                ChildState::Joined => {}
            }
        }
    }
    let inbox = mailbox.peers.iter().find(|p| p.peer == peer);
    let unread = inbox.map(|p| p.unread).unwrap_or(0);
    let has_handoff = inbox.and_then(|p| p.latest_handoff.as_ref()).is_some();

    if has_bounced {
        return (
            "bounced".into(),
            Some("child bounced; same peer/paths".into()),
            unread,
        );
    }
    if has_assigned && has_handoff {
        return (
            "working".into(),
            Some("handoff observed while still ASSIGNED".into()),
            unread,
        );
    }
    if has_assigned {
        return ("assigned".into(), Some("child ASSIGNED".into()), unread);
    }
    if unread > 0 {
        return (
            "wait".into(),
            Some("unread mailbox; no assigned child in store".into()),
            unread,
        );
    }
    if inbox.is_some() && mailbox.present && mailbox.inbox_present {
        return ("live".into(), None, unread);
    }
    ("idle".into(), None, unread)
}
