//! Truth panel. Shop answers "what is happening" from store + mailbox + GitHub
//! records. Missing pieces are WAIT with the exact gap. Never invent state.
//!
//! A roster name is not a running worker. A mailbox handoff is Evidence, not a
//! live pid. `working` requires an observed Working pid. GitHub counts stay
//! `Option` — None is WAIT, never a fake zero.

use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::mailbox::MailboxObservation;
use crate::procwait::{LiveProcess, PidLiveness};
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

/// One live job: a child on an open parent, optionally bound to an observed pid.
/// Pid stays None until the ledger recorded one. Never invent a pid or a count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobView {
    pub parent_id: String,
    pub child_id: String,
    pub peer: String,
    pub parent_state: ParentState,
    pub child_state: ChildState,
    pub paths: Vec<String>,
    pub dispatched: bool,
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid_liveness: Option<PidLiveness>,
    /// assigned | working | bounced | joined | dead | wait
    pub action: String,
    pub action_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerView {
    pub peer: String,
    pub name: String,
    pub intelligence: Intelligence,
    pub github_skills: bool,
    /// idle | assigned | working | bounced | wait | live | dead
    pub action: String,
    pub action_reason: Option<String>,
    pub unread: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid_liveness: Option<PidLiveness>,
    /// Open child this worker is on, if store or ledger named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_child_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectView {
    pub name: String,
    pub path: String,
    pub store: String,
}

#[derive(Debug, Clone, Deserialize)]
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
    /// Recorded gaps from the store snapshot. Serialize merges GitHub-count
    /// and assigned-job-pid waits so the truth panel never invents a zero.
    pub waits: Vec<String>,
}

impl WorkerView {
    /// Working only when a Working pid was observed. Roster/handoff/dead are not.
    pub fn is_observed_working(&self) -> bool {
        self.pid_liveness == Some(PidLiveness::Working)
    }

    pub fn from_worker(
        w: Worker,
        parents: &[ParentView],
        mailbox: &MailboxObservation,
        processes: &[LiveProcess],
    ) -> Self {
        let (action, action_reason, unread) = classify_action(&w.peer, parents, mailbox, processes);
        let tracked = latest_process_for_peer(&w.peer, processes);
        let job = bind_job(&w.peer, parents, processes);
        Self {
            peer: w.peer,
            name: w.name,
            intelligence: w.intelligence,
            github_skills: w.github_skills,
            action,
            action_reason,
            unread,
            pid: tracked.map(|p| p.pid),
            pid_liveness: tracked.map(|p| p.liveness),
            job_parent_id: job.as_ref().map(|(p, _)| p.clone()),
            job_child_id: job.as_ref().map(|(_, c)| c.clone()),
        }
    }
}

impl JobView {
    /// Live jobs only: children on non-CLOSED parents. Empty parents → empty jobs.
    pub fn from_parents(parents: &[ParentView], processes: &[LiveProcess]) -> Vec<Self> {
        let mut jobs = Vec::new();
        for parent in parents {
            if matches!(parent.state, ParentState::Closed) {
                continue;
            }
            for child in &parent.children {
                jobs.push(Self::from_child(parent, child, processes));
            }
        }
        jobs
    }

    pub fn from_child(parent: &ParentView, child: &ChildView, processes: &[LiveProcess]) -> Self {
        let tracked = process_for_job(&parent.id, &child.id, &child.peer, processes);
        let (action, action_reason) = classify_job(child, tracked);
        Self {
            parent_id: parent.id.clone(),
            child_id: child.id.clone(),
            peer: child.peer.clone(),
            parent_state: parent.state,
            child_state: child.state,
            paths: child.paths.clone(),
            dispatched: child.dispatched,
            branch: child.branch.clone(),
            pid: tracked.map(|p| p.pid),
            pid_liveness: tracked.map(|p| p.liveness),
            action,
            action_reason,
        }
    }
}

impl GitHubAwareness {
    /// Observed count text. None is WAIT, never the digit 0.
    pub fn format_count(count: Option<u64>) -> String {
        format_observed_count(count)
    }

    /// Exact gaps for unobserved GitHub counts. Does not invent zeros.
    pub fn count_waits(&self) -> Vec<String> {
        let mut waits = Vec::new();
        if self.issue_count.is_none() {
            waits.push("WAIT: GitHub issue_count not observed; not invented as 0".into());
        }
        if self.pr_count.is_none() {
            waits.push("WAIT: GitHub pr_count not observed; not invented as 0".into());
        }
        if self.check_count.is_none() {
            waits.push("WAIT: GitHub check_count not observed; not invented as 0".into());
        }
        waits
    }
}

impl Awareness {
    /// Jobs from store parents plus the caller-supplied ledger. No invented pids.
    pub fn jobs(&self, processes: &[LiveProcess]) -> Vec<JobView> {
        JobView::from_parents(&self.parents, processes)
    }

    /// Jobs from store parents only. Pids stay None until a ledger is supplied.
    pub fn jobs_from_store(&self) -> Vec<JobView> {
        JobView::from_parents(&self.parents, &[])
    }

    pub fn github_count_waits(&self) -> Vec<String> {
        self.github.count_waits()
    }

    /// Assigned jobs with no ledger pid. Missing pid is WAIT, never a fake working.
    pub fn job_waits(&self, processes: &[LiveProcess]) -> Vec<String> {
        job_pid_waits(&self.jobs(processes))
    }

    /// Observed working workers. Roster names and dead pids do not count.
    pub fn working_worker_count(&self) -> usize {
        observed_working_workers(&self.workers)
    }

    /// Store waits plus exact GitHub-count and assigned-job-pid gaps.
    /// Does not invent zeros. Does not drop an already-recorded WAIT.
    pub fn merge_waits(&self, processes: &[LiveProcess]) -> Vec<String> {
        merge_truth_waits(&self.waits, &self.github, &self.jobs(processes))
    }

    /// Live jobs from store parents. A worker pid attaches only when that
    /// worker is already bound to the exact parent/child/peer. Never steal.
    pub fn jobs_from_workers(&self) -> Vec<JobView> {
        let mut jobs = Vec::new();
        for parent in &self.parents {
            if matches!(parent.state, ParentState::Closed) {
                continue;
            }
            for child in &parent.children {
                let tracked = self.workers.iter().find(|w| {
                    w.peer == child.peer
                        && w.job_parent_id.as_deref() == Some(parent.id.as_str())
                        && w.job_child_id.as_deref() == Some(child.id.as_str())
                        && w.pid.is_some()
                });
                let proc = tracked.and_then(live_process_from_bound_worker);
                let processes = proc.as_ref().map(std::slice::from_ref).unwrap_or(&[]);
                jobs.push(JobView::from_child(parent, child, processes));
            }
        }
        jobs
    }

    /// Recorded waits plus GitHub-count and assigned-job-pid gaps from
    /// store + bound worker pids. Missing count stays WAIT, never 0.
    pub fn truth_waits(&self) -> Vec<String> {
        merge_truth_waits(&self.waits, &self.github, &self.jobs_from_workers())
    }
}

impl Serialize for Awareness {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let jobs = self.jobs_from_workers();
        let waits = merge_truth_waits(&self.waits, &self.github, &jobs);
        let mut s = serializer.serialize_struct("Awareness", 10)?;
        s.serialize_field("store_ok", &self.store_ok)?;
        s.serialize_field("project", &self.project)?;
        s.serialize_field("workers", &self.workers)?;
        s.serialize_field("claims", &self.claims)?;
        s.serialize_field("parents", &self.parents)?;
        s.serialize_field("jobs", &jobs)?;
        s.serialize_field("mailbox", &self.mailbox)?;
        s.serialize_field("github", &self.github)?;
        s.serialize_field("events", &self.events)?;
        s.serialize_field("waits", &waits)?;
        s.end()
    }
}

/// Observed live-job count. Length of the observed vec, never a guess.
pub fn observed_job_count(jobs: &[JobView]) -> usize {
    jobs.len()
}

/// Workers whose latest observed pid is Working. A roster name is not working.
pub fn observed_working_workers(workers: &[WorkerView]) -> usize {
    workers
        .iter()
        .filter(|w| w.is_observed_working())
        .count()
}

/// Jobs whose bound pid is Working. Assigned-without-pid is not working.
pub fn observed_working_jobs(jobs: &[JobView]) -> usize {
    jobs.iter()
        .filter(|j| j.pid_liveness == Some(PidLiveness::Working))
        .count()
}

/// Recorded waits plus GitHub-count and assigned-job-pid gaps. Never invents 0.
pub fn merge_truth_waits(
    recorded: &[String],
    github: &GitHubAwareness,
    jobs: &[JobView],
) -> Vec<String> {
    let mut waits = recorded.to_vec();
    for wait in github
        .count_waits()
        .into_iter()
        .chain(job_pid_waits(jobs))
    {
        if !waits.iter().any(|existing| existing == &wait) {
            waits.push(wait);
        }
    }
    waits
}

/// Assigned jobs with no observed pid. Does not invent a working count.
pub fn job_pid_waits(jobs: &[JobView]) -> Vec<String> {
    jobs.iter()
        .filter(|j| matches!(j.child_state, ChildState::Assigned) && j.pid.is_none())
        .map(|j| {
            format!(
                "WAIT: job {}/{} has no observed pid; not invented",
                j.parent_id, j.child_id
            )
        })
        .collect()
}

/// Observed count text. None is WAIT, never a fake zero.
pub fn format_observed_count(count: Option<u64>) -> String {
    match count {
        Some(n) => n.to_string(),
        None => "WAIT".into(),
    }
}

/// Live jobs on open parents. Count is the observed vec length, not a guess.
pub fn live_jobs(parents: &[ParentView], processes: &[LiveProcess]) -> Vec<JobView> {
    JobView::from_parents(parents, processes)
}

/// Reconstruct a ledger row only when the worker already named this job.
/// Parent-less pids are refused so they cannot fall through onto another child.
fn live_process_from_bound_worker(w: &WorkerView) -> Option<LiveProcess> {
    let pid = w.pid?;
    let liveness = w.pid_liveness?;
    let parent_id = w.job_parent_id.as_deref()?;
    let child_id = w.job_child_id.as_deref()?;
    let mut proc = LiveProcess::observed(
        pid,
        crate::procwait::ProcessBind::new(Some(parent_id), Some(child_id), Some(w.peer.as_str())),
    );
    proc.liveness = liveness;
    Some(proc)
}

fn latest_process_for_peer<'a>(peer: &str, processes: &'a [LiveProcess]) -> Option<&'a LiveProcess> {
    processes
        .iter()
        .filter(|p| p.peer.as_deref() == Some(peer))
        .max_by_key(|p| p.updated_at)
}

fn process_for_job<'a>(
    parent_id: &str,
    child_id: &str,
    peer: &str,
    processes: &'a [LiveProcess],
) -> Option<&'a LiveProcess> {
    let exact = processes
        .iter()
        .filter(|p| {
            p.peer.as_deref() == Some(peer)
                && p.parent_id.as_deref() == Some(parent_id)
                && p.child_id.as_deref() == Some(child_id)
        })
        .max_by_key(|p| p.updated_at);
    if exact.is_some() {
        return exact;
    }
    // Fallback only when the ledger named no parent. Never steal another job's pid.
    processes
        .iter()
        .filter(|p| {
            p.peer.as_deref() == Some(peer)
                && p.child_id.as_deref() == Some(child_id)
                && p.parent_id.is_none()
        })
        .max_by_key(|p| p.updated_at)
}

fn is_open_job(parent_id: &str, child_id: &str, parents: &[ParentView]) -> bool {
    parents.iter().any(|parent| {
        !matches!(parent.state, ParentState::Closed)
            && parent.id == parent_id
            && parent.children.iter().any(|child| child.id == child_id)
    })
}

fn open_children_for_peer<'a>(
    peer: &str,
    parents: &'a [ParentView],
) -> Vec<(&'a ParentView, &'a ChildView)> {
    let mut out = Vec::new();
    for parent in parents {
        if matches!(parent.state, ParentState::Closed) {
            continue;
        }
        for child in &parent.children {
            if child.peer == peer {
                out.push((parent, child));
            }
        }
    }
    out
}

/// Bind the worker to one open job. Prefer a working pid's bind, then store child.
/// A closed parent is not a live job. Never invent a bind.
fn bind_job(
    peer: &str,
    parents: &[ParentView],
    processes: &[LiveProcess],
) -> Option<(String, String)> {
    if let Some(proc) = latest_process_for_peer(peer, processes) {
        if proc.liveness == PidLiveness::Working {
            if let (Some(parent_id), Some(child_id)) = (&proc.parent_id, &proc.child_id) {
                if is_open_job(parent_id, child_id, parents) {
                    return Some((parent_id.clone(), child_id.clone()));
                }
            }
        }
    }
    let open = open_children_for_peer(peer, parents);
    let assigned = open
        .iter()
        .find(|(_, c)| matches!(c.state, ChildState::Assigned));
    if let Some((parent, child)) = assigned {
        return Some((parent.id.clone(), child.id.clone()));
    }
    let bounced = open
        .iter()
        .find(|(_, c)| matches!(c.state, ChildState::Bounced));
    if let Some((parent, child)) = bounced {
        return Some((parent.id.clone(), child.id.clone()));
    }
    if let Some(proc) = latest_process_for_peer(peer, processes) {
        if let (Some(parent_id), Some(child_id)) = (&proc.parent_id, &proc.child_id) {
            if is_open_job(parent_id, child_id, parents) {
                return Some((parent_id.clone(), child_id.clone()));
            }
        }
    }
    open.first()
        .map(|(parent, child)| (parent.id.clone(), child.id.clone()))
}

fn classify_job(child: &ChildView, tracked: Option<&LiveProcess>) -> (String, Option<String>) {
    if let Some(proc) = tracked {
        if proc.liveness == PidLiveness::Working {
            return (
                "working".into(),
                Some(format!("live pid {} is working", proc.pid)),
            );
        }
    }
    match child.state {
        ChildState::Bounced => (
            "bounced".into(),
            Some("child bounced; same peer/paths".into()),
        ),
        ChildState::Assigned => {
            if let Some(proc) = tracked {
                (
                    "assigned".into(),
                    Some(format!(
                        "child ASSIGNED; pid {} is dead; not working",
                        proc.pid
                    )),
                )
            } else {
                ("assigned".into(), Some("child ASSIGNED".into()))
            }
        }
        ChildState::Joined => {
            if let Some(proc) = tracked {
                (
                    "dead".into(),
                    Some(format!("pid {} is dead; not working", proc.pid)),
                )
            } else {
                ("joined".into(), Some("child JOINED; no live pid".into()))
            }
        }
    }
}

/// Derive the live action pill from store + mailbox + live pid observe.
/// An alive pid is working. A dead pid is not working. Never fake working.
/// A mailbox handoff is Evidence, not a running worker.
pub fn classify_action(
    peer: &str,
    parents: &[ParentView],
    mailbox: &MailboxObservation,
    processes: &[LiveProcess],
) -> (String, Option<String>, usize) {
    let open = open_children_for_peer(peer, parents);
    let has_assigned = open
        .iter()
        .any(|(_, c)| matches!(c.state, ChildState::Assigned));
    let has_bounced = open
        .iter()
        .any(|(_, c)| matches!(c.state, ChildState::Bounced));
    let inbox = mailbox.peers.iter().find(|p| p.peer == peer);
    let unread = inbox.map(|p| p.unread).unwrap_or(0);
    let latest_pid = latest_process_for_peer(peer, processes);

    // Observed Working pid wins. Bounce is store state, not a live process.
    if let Some(proc) = latest_pid {
        if proc.liveness == PidLiveness::Working {
            return (
                "working".into(),
                Some(format!("live pid {} is working", proc.pid)),
                unread,
            );
        }
    }
    if has_bounced {
        return (
            "bounced".into(),
            Some("child bounced; same peer/paths".into()),
            unread,
        );
    }
    if has_assigned {
        return ("assigned".into(), Some("child ASSIGNED".into()), unread);
    }
    if let Some(proc) = latest_pid {
        return (
            "dead".into(),
            Some(format!("pid {} is dead; not working", proc.pid)),
            unread,
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mailbox::PeerInbox;
    use crate::procwait::ProcessBind;
    use crate::workers::{Backend, Intelligence};

    fn mail(peers: Vec<PeerInbox>) -> MailboxObservation {
        MailboxObservation {
            root: None,
            present: !peers.is_empty(),
            inbox_present: !peers.is_empty(),
            wait: None,
            peers,
            unread_total: 0,
            outbox_count: 0,
        }
    }

    fn empty_mail() -> MailboxObservation {
        MailboxObservation {
            root: None,
            present: false,
            inbox_present: false,
            wait: None,
            peers: Vec::new(),
            unread_total: 0,
            outbox_count: 0,
        }
    }

    fn worker(peer: &str) -> Worker {
        Worker {
            peer: peer.into(),
            name: peer.into(),
            intelligence: Intelligence {
                backend: Backend::GrokBot,
                model: String::new(),
            },
            github_skills: true,
        }
    }

    fn parent_with_child(state: ChildState) -> ParentView {
        ParentView {
            id: "P".into(),
            title: "job".into(),
            state: ParentState::InFlight,
            children: vec![ChildView {
                id: "c1".into(),
                peer: "alice".into(),
                state,
                paths: vec!["src/awareness.rs".into()],
                branch: None,
                dispatched: true,
            }],
            verify: None,
            github_pr_url: None,
            github_pr_status: None,
            github_merged: false,
            github_merge_status: None,
            github_merge_reason: None,
        }
    }

    fn proc(peer: &str, liveness: PidLiveness, updated_at: u64) -> LiveProcess {
        let mut p = LiveProcess::observed(42, ProcessBind::new(Some("P"), Some("c1"), Some(peer)));
        p.liveness = liveness;
        p.updated_at = updated_at;
        p
    }

    #[test]
    fn roster_name_is_idle_not_working() {
        let (action, _, unread) = classify_action("alice", &[], &empty_mail(), &[]);
        assert_eq!(action, "idle");
        assert_ne!(action, "working");
        assert_eq!(unread, 0);
        let view = WorkerView::from_worker(worker("alice"), &[], &empty_mail(), &[]);
        assert_eq!(view.pid, None);
        assert_eq!(view.job_parent_id, None);
        assert_eq!(view.action, "idle");
    }

    #[test]
    fn assigned_without_pid_is_assigned_not_working() {
        let parents = vec![parent_with_child(ChildState::Assigned)];
        let (action, reason, _) = classify_action("alice", &parents, &empty_mail(), &[]);
        assert_eq!(action, "assigned");
        assert_ne!(action, "working");
        assert!(reason.unwrap().contains("ASSIGNED"));
        let jobs = live_jobs(&parents, &[]);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].pid, None);
        assert_eq!(jobs[0].action, "assigned");
        assert_ne!(jobs[0].action, "working");
    }

    #[test]
    fn handoff_without_pid_is_not_working() {
        let parents = vec![parent_with_child(ChildState::Assigned)];
        let mailbox = mail(vec![PeerInbox {
            peer: "alice".into(),
            unread: 0,
            latest_assign: Some("assign.json".into()),
            latest_handoff: Some("handoff.json".into()),
        }]);
        let (action, _, _) = classify_action("alice", &parents, &mailbox, &[]);
        assert_eq!(action, "assigned");
        assert_ne!(action, "working");
    }

    #[test]
    fn working_requires_observed_working_pid() {
        let parents = vec![parent_with_child(ChildState::Assigned)];
        let processes = vec![proc("alice", PidLiveness::Working, 2)];
        let (action, reason, _) = classify_action("alice", &parents, &empty_mail(), &processes);
        assert_eq!(action, "working");
        assert!(reason.unwrap().contains("live pid 42"));
        let jobs = live_jobs(&parents, &processes);
        assert_eq!(jobs[0].pid, Some(42));
        assert_eq!(jobs[0].pid_liveness, Some(PidLiveness::Working));
        assert_eq!(jobs[0].action, "working");
    }

    #[test]
    fn dead_pid_is_not_working() {
        let processes = vec![proc("alice", PidLiveness::Dead, 2)];
        let (action, reason, _) = classify_action("alice", &[], &empty_mail(), &processes);
        assert_eq!(action, "dead");
        assert_ne!(action, "working");
        assert!(reason.unwrap().contains("dead"));
    }

    #[test]
    fn dead_pid_does_not_hide_assigned_job() {
        let parents = vec![parent_with_child(ChildState::Assigned)];
        let processes = vec![proc("alice", PidLiveness::Dead, 2)];
        let (action, _, _) = classify_action("alice", &parents, &empty_mail(), &processes);
        assert_eq!(action, "assigned");
        assert_ne!(action, "working");
        let jobs = live_jobs(&parents, &processes);
        assert_eq!(jobs[0].action, "assigned");
        assert_eq!(jobs[0].pid, Some(42));
        assert_eq!(jobs[0].pid_liveness, Some(PidLiveness::Dead));
    }

    #[test]
    fn closed_parent_is_not_a_live_job() {
        let mut parent = parent_with_child(ChildState::Assigned);
        parent.state = ParentState::Closed;
        let jobs = live_jobs(&[parent], &[]);
        assert!(jobs.is_empty());
    }

    #[test]
    fn missing_github_counts_are_wait_not_zero() {
        assert_eq!(format_observed_count(None), "WAIT");
        assert_eq!(format_observed_count(Some(0)), "0");
        assert_eq!(format_observed_count(Some(3)), "3");
        assert_ne!(format_observed_count(None), "0");
        let github = GitHubAwareness {
            repo: None,
            default_branch: None,
            authenticated: false,
            token_present: false,
            wait: Some("WAIT: GitHub repo not connected".into()),
            checks: Value::Null,
            issue_count: None,
            pr_count: None,
            check_count: None,
        };
        let waits = github.count_waits();
        assert_eq!(waits.len(), 3);
        assert!(waits.iter().all(|w| w.contains("not invented as 0")));
    }

    #[test]
    fn worker_view_binds_open_job_without_inventing_pid() {
        let parents = vec![parent_with_child(ChildState::Assigned)];
        let view = WorkerView::from_worker(worker("alice"), &parents, &empty_mail(), &[]);
        assert_eq!(view.job_parent_id.as_deref(), Some("P"));
        assert_eq!(view.job_child_id.as_deref(), Some("c1"));
        assert_eq!(view.pid, None);
        assert_eq!(view.action, "assigned");
        let waits = job_pid_waits(&live_jobs(&parents, &[]));
        assert_eq!(waits.len(), 1);
        assert!(waits[0].contains("P/c1"));
        assert_eq!(observed_job_count(&live_jobs(&parents, &[])), 1);
        assert_eq!(observed_working_workers(&[view]), 0);
    }

    #[test]
    fn pid_from_another_parent_is_not_this_job() {
        let parents = vec![parent_with_child(ChildState::Assigned)];
        let mut other = LiveProcess::observed(
            99,
            ProcessBind::new(Some("Q"), Some("c1"), Some("alice")),
        );
        other.liveness = PidLiveness::Working;
        other.updated_at = 9;
        let jobs = live_jobs(&parents, &[other]);
        assert_eq!(jobs[0].pid, None);
        assert_eq!(jobs[0].action, "assigned");
        assert_ne!(jobs[0].action, "working");
    }

    #[test]
    fn closed_parent_does_not_bind_even_with_working_pid() {
        let mut parent = parent_with_child(ChildState::Assigned);
        parent.state = ParentState::Closed;
        let processes = vec![proc("alice", PidLiveness::Working, 2)];
        let jobs = live_jobs(&[parent.clone()], &processes);
        assert!(jobs.is_empty());
        assert_eq!(observed_job_count(&jobs), 0);
        let view = WorkerView::from_worker(worker("alice"), &[parent], &empty_mail(), &processes);
        assert_eq!(view.job_parent_id, None);
        assert_eq!(view.job_child_id, None);
        assert_eq!(view.pid, Some(42));
        assert_eq!(view.action, "working");
        assert_eq!(observed_working_workers(&[view]), 1);
    }

    #[test]
    fn bounced_without_pid_is_bounced_not_working() {
        let parents = vec![parent_with_child(ChildState::Bounced)];
        let (action, reason, _) = classify_action("alice", &parents, &empty_mail(), &[]);
        assert_eq!(action, "bounced");
        assert_ne!(action, "working");
        assert!(reason.unwrap().contains("bounced"));
        let jobs = live_jobs(&parents, &[]);
        assert_eq!(jobs[0].action, "bounced");
        assert_eq!(jobs[0].pid, None);
    }

    #[test]
    fn working_pid_wins_over_bounced_child() {
        let parents = vec![parent_with_child(ChildState::Bounced)];
        let processes = vec![proc("alice", PidLiveness::Working, 2)];
        let (action, reason, _) = classify_action("alice", &parents, &empty_mail(), &processes);
        assert_eq!(action, "working");
        assert!(reason.unwrap().contains("live pid 42"));
        let jobs = live_jobs(&parents, &processes);
        assert_eq!(jobs[0].action, "working");
        assert_eq!(jobs[0].pid, Some(42));
        assert_eq!(observed_working_jobs(&jobs), 1);
        let view = WorkerView::from_worker(worker("alice"), &parents, &empty_mail(), &processes);
        assert!(view.is_observed_working());
    }

    #[test]
    fn unread_mailbox_without_job_is_wait_not_working() {
        let mailbox = mail(vec![PeerInbox {
            peer: "alice".into(),
            unread: 2,
            latest_assign: None,
            latest_handoff: Some("handoff.json".into()),
        }]);
        let (action, reason, unread) = classify_action("alice", &[], &mailbox, &[]);
        assert_eq!(action, "wait");
        assert_ne!(action, "working");
        assert_eq!(unread, 2);
        assert!(reason.unwrap().contains("unread mailbox"));
    }

    #[test]
    fn present_inbox_without_unread_is_live_not_working() {
        let mailbox = mail(vec![PeerInbox {
            peer: "alice".into(),
            unread: 0,
            latest_assign: None,
            latest_handoff: None,
        }]);
        let (action, _, unread) = classify_action("alice", &[], &mailbox, &[]);
        assert_eq!(action, "live");
        assert_ne!(action, "working");
        assert_eq!(unread, 0);
    }

    #[test]
    fn merge_waits_keeps_gaps_and_never_invents_zero() {
        let parents = vec![parent_with_child(ChildState::Assigned)];
        let github = GitHubAwareness {
            repo: None,
            default_branch: None,
            authenticated: false,
            token_present: false,
            wait: Some("WAIT: GitHub repo not connected".into()),
            checks: Value::Null,
            issue_count: None,
            pr_count: None,
            check_count: None,
        };
        let awareness = Awareness {
            store_ok: true,
            project: ProjectView {
                name: "shop".into(),
                path: ".".into(),
                store: ".shop".into(),
            },
            workers: Vec::new(),
            claims: Vec::new(),
            parents: parents.clone(),
            mailbox: empty_mail(),
            github,
            events: Vec::new(),
            waits: vec!["WAIT: mailbox down".into()],
        };
        let merged = awareness.merge_waits(&[]);
        assert!(merged.iter().any(|w| w.contains("mailbox down")));
        assert!(merged.iter().any(|w| w.contains("issue_count")));
        assert!(merged.iter().any(|w| w.contains("P/c1")));
        assert!(merged.iter().all(|w| w.contains("WAIT")));
        assert!(merged.iter().any(|w| w.contains("not invented as 0")));
        assert_eq!(format_observed_count(None), "WAIT");
        assert_ne!(format_observed_count(None), "0");
        assert_eq!(observed_working_jobs(&awareness.jobs_from_store()), 0);
    }

    #[test]
    fn assigned_job_preferred_over_joined_sibling() {
        let mut parent = parent_with_child(ChildState::Joined);
        parent.children.push(ChildView {
            id: "c2".into(),
            peer: "alice".into(),
            state: ChildState::Assigned,
            paths: vec!["src/awareness.rs".into()],
            branch: None,
            dispatched: true,
        });
        let view = WorkerView::from_worker(worker("alice"), &[parent], &empty_mail(), &[]);
        assert_eq!(view.job_parent_id.as_deref(), Some("P"));
        assert_eq!(view.job_child_id.as_deref(), Some("c2"));
        assert_eq!(view.pid, None);
        assert_eq!(view.action, "assigned");
    }

    fn panel(parents: Vec<ParentView>, workers: Vec<WorkerView>, github: GitHubAwareness) -> Awareness {
        Awareness {
            store_ok: true,
            project: ProjectView {
                name: "shop".into(),
                path: ".".into(),
                store: ".shop".into(),
            },
            workers,
            claims: Vec::new(),
            parents,
            mailbox: empty_mail(),
            github,
            events: Vec::new(),
            waits: Vec::new(),
        }
    }

    fn unobserved_github() -> GitHubAwareness {
        GitHubAwareness {
            repo: None,
            default_branch: None,
            authenticated: false,
            token_present: false,
            wait: Some("WAIT: GitHub repo not connected".into()),
            checks: Value::Null,
            issue_count: None,
            pr_count: None,
            check_count: None,
        }
    }

    #[test]
    fn truth_panel_json_includes_jobs_and_does_not_invent_zero() {
        let parents = vec![parent_with_child(ChildState::Assigned)];
        let workers = vec![WorkerView::from_worker(
            worker("alice"),
            &parents,
            &empty_mail(),
            &[],
        )];
        let awareness = panel(parents, workers, unobserved_github());
        let json = serde_json::to_value(&awareness).unwrap();
        assert!(json.get("jobs").and_then(|j| j.as_array()).is_some());
        assert_eq!(json["jobs"].as_array().unwrap().len(), 1);
        assert_eq!(json["jobs"][0]["pid"], Value::Null);
        assert_eq!(json["jobs"][0]["action"], "assigned");
        assert_ne!(json["jobs"][0]["action"], "working");
        assert_eq!(json["github"]["issue_count"], Value::Null);
        assert_eq!(json["github"]["pr_count"], Value::Null);
        assert_eq!(json["github"]["check_count"], Value::Null);
        assert_ne!(json["github"]["issue_count"], 0);
        assert_ne!(json["github"]["issue_count"], "0");
        let waits = json["waits"].as_array().expect("waits");
        assert!(waits.iter().any(|w| w.as_str().unwrap().contains("issue_count")));
        assert!(waits.iter().any(|w| w.as_str().unwrap().contains("P/c1")));
        assert!(waits.iter().all(|w| w.as_str().unwrap().contains("WAIT")));
        assert_eq!(format_observed_count(None), "WAIT");
        assert_ne!(format_observed_count(None), "0");
        assert_eq!(observed_working_jobs(&awareness.jobs_from_workers()), 0);
        assert_eq!(observed_job_count(&awareness.jobs_from_workers()), 1);
    }

    #[test]
    fn jobs_from_workers_binds_only_the_exact_open_job() {
        let parents = vec![parent_with_child(ChildState::Assigned)];
        let processes = vec![proc("alice", PidLiveness::Working, 2)];
        let workers = vec![WorkerView::from_worker(
            worker("alice"),
            &parents,
            &empty_mail(),
            &processes,
        )];
        let awareness = panel(parents, workers, unobserved_github());
        let jobs = awareness.jobs_from_workers();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].pid, Some(42));
        assert_eq!(jobs[0].pid_liveness, Some(PidLiveness::Working));
        assert_eq!(jobs[0].action, "working");
        assert_eq!(observed_working_jobs(&jobs), 1);
        assert_eq!(awareness.working_worker_count(), 1);
    }

    #[test]
    fn jobs_from_workers_does_not_steal_another_parent_pid() {
        let parents = vec![parent_with_child(ChildState::Assigned)];
        let mut other = WorkerView::from_worker(worker("alice"), &parents, &empty_mail(), &[]);
        other.pid = Some(99);
        other.pid_liveness = Some(PidLiveness::Working);
        other.job_parent_id = Some("Q".into());
        other.job_child_id = Some("c1".into());
        other.action = "working".into();
        let awareness = panel(parents, vec![other], unobserved_github());
        let jobs = awareness.jobs_from_workers();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].pid, None);
        assert_eq!(jobs[0].action, "assigned");
        assert_ne!(jobs[0].action, "working");
        assert_eq!(observed_working_jobs(&jobs), 0);
        let waits = awareness.truth_waits();
        assert!(waits.iter().any(|w| w.contains("P/c1")));
    }

    #[test]
    fn closed_parent_is_absent_from_truth_panel_jobs() {
        let mut parent = parent_with_child(ChildState::Assigned);
        parent.state = ParentState::Closed;
        let processes = vec![proc("alice", PidLiveness::Working, 2)];
        let workers = vec![WorkerView::from_worker(
            worker("alice"),
            &[parent.clone()],
            &empty_mail(),
            &processes,
        )];
        let awareness = panel(vec![parent], workers, unobserved_github());
        assert!(awareness.jobs_from_workers().is_empty());
        let json = serde_json::to_value(&awareness).unwrap();
        assert_eq!(json["jobs"].as_array().unwrap().len(), 0);
        assert_eq!(json["workers"][0]["job_parent_id"], Value::Null);
        assert_eq!(json["workers"][0]["action"], "working");
        assert_eq!(awareness.working_worker_count(), 1);
    }
}
