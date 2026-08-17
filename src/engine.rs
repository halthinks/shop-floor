//! Shop-floor state machine.
//!
//! Authority rules (AASM stays the authority calculus; shop only sequences):
//! - A handoff is Evidence, not truth.
//! - A CLI/test exit 0 is Evidence, not authority to CLOSE unless `shop verify` recorded it.
//! - shop never invents a child lane. Only `shop split` creates children.
//! - shop never writes C:\TextPCB Platform or any path outside the shop store
//!   and optional mailbox outbox.
//! - Incomplete evidence is WAIT, never fake PASS.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::error::{Result, ShopError};
use crate::github::{child_branch_name, GitHubRepo};
use crate::hash::{fingerprint, unix_now};
use crate::mailbox::{write_assign, AssignRecord, DEFAULT_FROM};
use crate::paths::{parse_paths, paths_overlap};
use crate::skills::{GitHubSkills, NullGitHub};
use crate::store::Store;
use crate::types::{
    Child, ChildState, Claim, EvidenceStatus, Handoff, Parent, ParentState, ReduceChild,
    ReducePackage, ShopState, VerifyRecord,
};
use crate::workers::{Backend, Intelligence, Worker, WorkerRoster};

#[derive(Clone)]
pub struct Shop {
    store: Store,
    mailbox: Option<PathBuf>,
    from: String,
    github: Arc<dyn GitHubSkills>,
}

impl Shop {
    pub fn init(store: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            store: Store::init(store)?,
            mailbox: None,
            from: DEFAULT_FROM.to_string(),
            github: Arc::new(NullGitHub),
        })
    }

    pub fn open_store(store: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            store: Store::open(store)?,
            mailbox: None,
            from: DEFAULT_FROM.to_string(),
            github: Arc::new(NullGitHub),
        })
    }

    pub fn with_github(mut self, github: Arc<dyn GitHubSkills>) -> Self {
        self.github = github;
        self
    }

    pub fn with_mailbox(mut self, mailbox: Option<PathBuf>) -> Self {
        self.mailbox = mailbox;
        self
    }

    pub fn with_from(mut self, from: impl Into<String>) -> Self {
        self.from = from.into();
        self
    }

    pub fn store_root(&self) -> &Path {
        self.store.root()
    }

    pub fn github_skills(&self) -> &dyn GitHubSkills {
        self.github.as_ref()
    }

    pub fn add_worker(
        &mut self,
        peer: &str,
        name: &str,
        backend: &str,
        model: &str,
        github_skills: bool,
    ) -> Result<Worker> {
        validate_id(peer)?;
        let backend = Backend::parse(backend)?;
        let mut roster = self.store.load_workers()?;
        if roster.get(peer).is_some() {
            return Err(ShopError::DuplicatePeer(peer.to_string()));
        }
        let worker = Worker {
            peer: peer.to_string(),
            name: if name.trim().is_empty() {
                peer.to_string()
            } else {
                name.to_string()
            },
            intelligence: Intelligence {
                backend,
                model: model.trim().to_string(),
            },
            github_skills,
        };
        roster.workers.push(worker.clone());
        self.store.save_workers(&roster)?;
        self.event(json!({
            "op": "add_worker",
            "peer": peer,
            "backend": backend.as_str(),
            "github_skills": github_skills,
            "note": "enrolls capacity only; does not create a job or lane",
        }))?;
        Ok(worker)
    }

    pub fn set_worker_intelligence(
        &mut self,
        peer: &str,
        backend: &str,
        model: &str,
    ) -> Result<Worker> {
        let backend = Backend::parse(backend)?;
        let mut roster = self.store.load_workers()?;
        let worker = roster
            .get_mut(peer)
            .ok_or_else(|| ShopError::UnknownPeer(peer.to_string()))?;
        worker.intelligence = Intelligence {
            backend,
            model: model.trim().to_string(),
        };
        let out = worker.clone();
        self.store.save_workers(&roster)?;
        self.event(json!({
            "op": "set_intelligence",
            "peer": peer,
            "backend": backend.as_str(),
        }))?;
        Ok(out)
    }

    pub fn set_worker_github_skills(&mut self, peer: &str, enabled: bool) -> Result<Worker> {
        let mut roster = self.store.load_workers()?;
        let worker = roster
            .get_mut(peer)
            .ok_or_else(|| ShopError::UnknownPeer(peer.to_string()))?;
        worker.github_skills = enabled;
        let out = worker.clone();
        self.store.save_workers(&roster)?;
        Ok(out)
    }

    pub fn workers(&self) -> Result<WorkerRoster> {
        self.store.load_workers()
    }

    pub fn worker(&self, peer: &str) -> Result<Option<Worker>> {
        Ok(self.store.load_workers()?.get(peer).cloned())
    }

    pub fn github_connect(
        &mut self,
        spec: &str,
        default_branch: Option<&str>,
        token: Option<&str>,
    ) -> Result<GitHubRepo> {
        let repo = GitHubRepo::parse(spec, default_branch)?;
        self.store.save_github(&repo)?;
        if let Some(token) = token.map(str::trim).filter(|s| !s.is_empty()) {
            self.store.write_token(token)?;
        }
        self.event(json!({
            "op": "github_connect",
            "repo": repo.slug(),
            "note": "token if any is written to .shop/github.token and is gitignored",
        }))?;
        Ok(repo)
    }

    pub fn github_repo(&self) -> Result<Option<GitHubRepo>> {
        self.store.load_github()
    }

    fn try_create_branch(&self, branch: &str) -> bool {
        let Ok(Some(repo)) = self.github_repo() else {
            return false;
        };
        if !self.github.authenticated() {
            return false;
        }
        self.github
            .invoke(
                "branches.create",
                json!({
                    "owner": repo.owner,
                    "repo": repo.name,
                    "branch": branch,
                    "from_branch": repo.default_branch,
                }),
            )
            .is_ok()
    }

    fn try_draft_pr(
        &self,
        parent_id: &str,
        note: &str,
        repo: &Option<GitHubRepo>,
        branches: &[String],
    ) -> (Option<String>, Option<u64>, EvidenceStatus, Option<String>) {
        let Some(repo) = repo else {
            return (None, None, EvidenceStatus::Wait, None);
        };
        if !self.github.authenticated() {
            return (
                None,
                None,
                EvidenceStatus::Wait,
                Some("no GitHub auth; never fake a PR URL".into()),
            );
        }
        let Some(head) = branches.first() else {
            return (
                None,
                None,
                EvidenceStatus::Wait,
                Some("no child branches to open a PR from".into()),
            );
        };
        match self.github.invoke(
            "pulls.create",
            json!({
                "owner": repo.owner,
                "repo": repo.name,
                "title": format!("shop reduce {parent_id}"),
                "head": head,
                "base": repo.default_branch.clone().unwrap_or_else(|| "main".into()),
                "draft": true,
                "body": format!("{note}\n\nchild branches:\n{}", branches.join("\n")),
            }),
        ) {
            Ok(v) => {
                let url = v
                    .get("html_url")
                    .and_then(|u| u.as_str())
                    .map(ToString::to_string);
                let number = v.get("number").and_then(|n| n.as_u64());
                if url.is_none() {
                    (
                        None,
                        None,
                        EvidenceStatus::Wait,
                        Some("PR response missing html_url; never fake a PR URL".into()),
                    )
                } else {
                    (url, number, EvidenceStatus::Pass, None)
                }
            }
            Err(e) => (None, None, EvidenceStatus::Wait, Some(e.to_string())),
        }
    }

    fn github_verify_block(&self, parent: &Parent) -> Option<String> {
        let repo = self.store.load_github().ok().flatten()?;
        match parent.github_pr_url.as_ref() {
            None => Some(format!(
                "WAIT: repo {} connected but no real PR; never fake VERIFIED",
                repo.slug()
            )),
            Some(_) => {
                if !self.github.authenticated() {
                    return Some("WAIT: no GitHub auth to read checks".into());
                }
                let n = parent.github_pr_number.unwrap_or(0);
                match self.github.invoke(
                    "pulls.read",
                    json!({
                        "owner": repo.owner,
                        "repo": repo.name,
                        "pullNumber": n,
                        "method": "get_check_runs",
                    }),
                ) {
                    Ok(v) => {
                        let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
                        let runs_pass = v
                            .get("check_runs")
                            .and_then(|a| a.as_array())
                            .map(|runs| {
                                !runs.is_empty()
                                    && runs.iter().any(|x| {
                                        x.get("conclusion").and_then(|c| c.as_str())
                                            == Some("success")
                                    })
                            })
                            .unwrap_or(false);
                        if status == "success" || runs_pass {
                            None
                        } else {
                            Some("WAIT: missing or failing GitHub checks".into())
                        }
                    }
                    Err(e) => Some(format!("WAIT: {e}")),
                }
            }
        }
    }

    pub fn open(&mut self, id: &str, title: &str, body: &str) -> Result<Parent> {
        validate_id(id)?;
        let mut state = self.store.load()?;
        if state.parents.contains_key(id) {
            return Err(ShopError::ParentExists(id.to_string()));
        }
        let parent = Parent::new(id.to_string(), title.to_string(), body.to_string());
        state.parents.insert(id.to_string(), parent.clone());
        self.store.save(&state)?;
        self.event(json!({
            "op": "open",
            "parent": id,
            "state": "HELD",
        }))?;
        Ok(parent)
    }

    pub fn split(
        &mut self,
        parent_id: &str,
        child_id: &str,
        peer: &str,
        paths_csv: &str,
        title: &str,
        body: &str,
    ) -> Result<Child> {
        validate_id(parent_id)?;
        validate_id(child_id)?;
        if peer.trim().is_empty() {
            return Err(ShopError::InvalidId(peer.to_string()));
        }
        if self.worker(peer)?.is_none() {
            return Err(ShopError::UnknownPeer(peer.to_string()));
        }
        let paths = parse_paths(paths_csv);
        if paths.is_empty() {
            return Err(ShopError::EmptyPaths);
        }

        let mut state = self.store.load()?;
        check_overlap(&state, &paths)?;

        let parent = state
            .parents
            .get_mut(parent_id)
            .ok_or_else(|| ShopError::ParentNotFound(parent_id.to_string()))?;
        if !parent.state.allows_split() {
            return Err(ShopError::InvalidParentState {
                current: parent.state,
                op: "split",
            });
        }
        if parent.children.contains_key(child_id) {
            return Err(ShopError::ChildExists(
                child_id.to_string(),
                parent_id.to_string(),
            ));
        }

        let mut child = Child::new(
            child_id.to_string(),
            peer.to_string(),
            paths.clone(),
            title.to_string(),
            body.to_string(),
        );
        let branch = child_branch_name(parent_id, child_id);
        let created = self.try_create_branch(&branch);
        child.branch = Some(branch);
        child.branch_created = created;
        parent.children.insert(child_id.to_string(), child.clone());
        parent.state = ParentState::InFlight;
        parent.touch();

        state.claims.push(Claim {
            parent_id: parent_id.to_string(),
            child_id: child_id.to_string(),
            peer: peer.to_string(),
            paths,
        });
        self.store.save(&state)?;
        self.event(json!({
            "op": "split",
            "parent": parent_id,
            "child": child_id,
            "peer": peer,
            "paths": child.paths,
            "note": "shop never invents a child lane; only split creates children",
        }))?;
        Ok(child)
    }

    pub fn assign(&mut self, parent_id: &str) -> Result<Vec<AssignRecord>> {
        let mut state = self.store.load()?;
        let parent = state
            .parents
            .get_mut(parent_id)
            .ok_or_else(|| ShopError::ParentNotFound(parent_id.to_string()))?;
        if !parent.state.allows_assign() {
            return Err(ShopError::InvalidParentState {
                current: parent.state,
                op: "assign",
            });
        }

        let mut records = Vec::new();
        for child in parent.children.values_mut() {
            let bounce_reassign = child.state == ChildState::Bounced;
            let fresh = child.state == ChildState::Assigned && !child.dispatched;
            if !fresh && !bounce_reassign {
                continue;
            }
            if bounce_reassign {
                child.state = ChildState::Assigned;
            }
            let rec = AssignRecord::from_child(&self.from, child);
            child.dispatched = true;
            records.push(rec);
        }
        if !parent.children.is_empty() {
            parent.state = ParentState::InFlight;
        }
        parent.touch();
        self.store.save(&state)?;

        for rec in &records {
            write_assign(
                &self.store.outbox_dir(),
                self.mailbox.as_deref(),
                parent_id,
                rec,
            )?;
        }
        self.event(json!({
            "op": "assign",
            "parent": parent_id,
            "count": records.len(),
        }))?;
        Ok(records)
    }

    pub fn accept(
        &mut self,
        parent_id: &str,
        child_id: &str,
        handoff_spec: &str,
    ) -> Result<Handoff> {
        let handoff = load_handoff(handoff_spec)?;
        let mut state = self.store.load()?;
        {
            let parent = state
                .parents
                .get_mut(parent_id)
                .ok_or_else(|| ShopError::ParentNotFound(parent_id.to_string()))?;
            if !parent.state.allows_child_update() {
                return Err(ShopError::InvalidParentState {
                    current: parent.state,
                    op: "accept",
                });
            }
            let child = parent.children.get_mut(child_id).ok_or_else(|| {
                ShopError::ChildNotFound(child_id.to_string(), parent_id.to_string())
            })?;
            if child.state != ChildState::Assigned {
                return Err(ShopError::InvalidChildState {
                    current: child.state,
                    op: "accept",
                });
            }
            child.handoff = Some(handoff.clone());
            child.state = ChildState::Joined;
            parent.touch();
        }
        state
            .claims
            .retain(|c| !(c.parent_id == parent_id && c.child_id == child_id));
        self.store.save(&state)?;
        self.event(json!({
            "op": "accept",
            "parent": parent_id,
            "child": child_id,
            "handoff_hash": handoff.hash,
            "note": "handoff is Evidence, not truth",
        }))?;
        Ok(handoff)
    }

    pub fn bounce(&mut self, parent_id: &str, child_id: &str, reason: &str) -> Result<Child> {
        let mut state = self.store.load()?;
        let parent = state
            .parents
            .get_mut(parent_id)
            .ok_or_else(|| ShopError::ParentNotFound(parent_id.to_string()))?;
        if !parent.state.allows_child_update() {
            return Err(ShopError::InvalidParentState {
                current: parent.state,
                op: "bounce",
            });
        }
        {
            let child = parent.children.get_mut(child_id).ok_or_else(|| {
                ShopError::ChildNotFound(child_id.to_string(), parent_id.to_string())
            })?;
            if child.state == ChildState::Joined {
                return Err(ShopError::InvalidChildState {
                    current: child.state,
                    op: "bounce",
                });
            }
            child.state = ChildState::Bounced;
            child.dispatched = false;
            child.bounce_reason = Some(reason.to_string());
        }
        parent.state = ParentState::InFlight;
        parent.touch();
        let bounced = parent.children.get(child_id).unwrap().clone();
        self.store.save(&state)?;
        self.event(json!({
            "op": "bounce",
            "parent": parent_id,
            "child": child_id,
            "peer": bounced.peer,
            "paths": bounced.paths,
            "reason": reason,
            "note": "bounce stays tied to the same peer and paths",
        }))?;
        Ok(bounced)
    }

    pub fn join(&mut self, parent_id: &str) -> Result<ParentState> {
        let mut state = self.store.load()?;
        let parent = state
            .parents
            .get_mut(parent_id)
            .ok_or_else(|| ShopError::ParentNotFound(parent_id.to_string()))?;
        if matches!(
            parent.state,
            ParentState::Reduced
                | ParentState::VerifyWait
                | ParentState::Verified
                | ParentState::Closed
        ) {
            return Err(ShopError::InvalidParentState {
                current: parent.state,
                op: "join",
            });
        }

        // Never skip the join barrier. Never invent a child lane.
        if parent.all_children_joined() {
            parent.state = ParentState::ReduceReady;
        } else if parent.children.is_empty() {
            parent.state = ParentState::Held;
        } else {
            parent.state = ParentState::InFlight;
        }
        parent.touch();
        let next = parent.state;
        self.store.save(&state)?;
        self.event(json!({
            "op": "join",
            "parent": parent_id,
            "state": next.to_string(),
        }))?;
        Ok(next)
    }

    pub fn reduce(&mut self, parent_id: &str, note: &str) -> Result<ReducePackage> {
        let mut state = self.store.load()?;
        let parent = state
            .parents
            .get_mut(parent_id)
            .ok_or_else(|| ShopError::ParentNotFound(parent_id.to_string()))?;
        if parent.state != ParentState::ReduceReady || !parent.all_children_joined() {
            return Err(ShopError::JoinBarrier);
        }
        parent.state = ParentState::Reduced;
        parent.reduce_note = Some(note.to_string());
        parent.touch();

        let child_branches: Vec<String> = parent
            .children
            .values()
            .filter_map(|c| c.branch.clone())
            .collect();
        let repo = self.store.load_github().ok().flatten();
        let (pr_url, pr_number, pr_status, pr_reason) =
            self.try_draft_pr(parent_id, note, &repo, &child_branches);
        parent.github_pr_url = pr_url.clone();
        parent.github_pr_number = pr_number;
        parent.github_pr_status = Some(pr_status);
        parent.github_pr_reason = pr_reason.clone();

        let package = ReducePackage {
            parent_id: parent_id.to_string(),
            note: note.to_string(),
            state: ParentState::Reduced,
            children: parent
                .children
                .values()
                .map(|c| ReduceChild {
                    id: c.id.clone(),
                    peer: c.peer.clone(),
                    paths: c.paths.clone(),
                    handoff_hash: c.handoff.as_ref().map(|h| h.hash.clone()),
                    handoff_status: c.handoff.as_ref().map(|h| h.status.clone()),
                    state: c.state,
                })
                .collect(),
            github_repo: repo.as_ref().map(|r| r.slug()),
            child_branches,
            pull_request_url: pr_url,
            pull_request_number: pr_number,
            pull_request_status: pr_status,
            pull_request_reason: pr_reason,
        };
        self.store.save(&state)?;
        let bytes = serde_json::to_vec_pretty(&package)?;
        self.store
            .write_under(format!("reduce/{parent_id}.json"), &bytes)?;
        self.event(json!({
            "op": "reduce",
            "parent": parent_id,
            "note": "reduce package is Evidence listing; does not land onto a canonical repo",
        }))?;
        Ok(package)
    }

    pub fn verify(&mut self, parent_id: &str, cmd: Option<&str>) -> Result<VerifyRecord> {
        let mut state = self.store.load()?;
        let parent = state
            .parents
            .get_mut(parent_id)
            .ok_or_else(|| ShopError::ParentNotFound(parent_id.to_string()))?;
        if !parent.state.allows_verify() {
            return Err(ShopError::InvalidParentState {
                current: parent.state,
                op: "verify",
            });
        }

        let cmd = match cmd.map(str::trim).filter(|s| !s.is_empty()) {
            Some(c) => Some(c.to_string()),
            None => parent.verify_cmd.clone(),
        };

        let mut record = match cmd {
            None => VerifyRecord {
                cmd: String::new(),
                exit_code: None,
                last_lines: "no verify command recorded".into(),
                status: EvidenceStatus::Wait,
                recorded_at: unix_now(),
            },
            Some(cmd) => {
                parent.verify_cmd = Some(cmd.clone());
                run_verify_command(&cmd)
            }
        };
        if let Some(reason) = self.github_verify_block(parent) {
            record.status = EvidenceStatus::Wait;
            if !record.last_lines.is_empty() {
                record.last_lines.push('\n');
            }
            record.last_lines.push_str(&reason);
        }
        parent.verify = Some(record.clone());
        parent.state = match record.status {
            EvidenceStatus::Pass => ParentState::Verified,
            EvidenceStatus::Wait => ParentState::VerifyWait,
        };
        parent.touch();
        self.store.save(&state)?;
        let bytes = serde_json::to_vec_pretty(&record)?;
        self.store
            .write_under(format!("verify/{parent_id}.json"), &bytes)?;
        self.event(json!({
            "op": "verify",
            "parent": parent_id,
            "status": record.status.to_string(),
            "exit_code": record.exit_code,
            "note": "exit 0 is Evidence; CLOSE requires this recorded PASS",
        }))?;
        Ok(record)
    }

    pub fn close(&mut self, parent_id: &str) -> Result<Parent> {
        let mut state = self.store.load()?;
        let verified = {
            let parent = state
                .parents
                .get(parent_id)
                .ok_or_else(|| ShopError::ParentNotFound(parent_id.to_string()))?;
            parent.state == ParentState::Verified
                && parent
                    .verify
                    .as_ref()
                    .is_some_and(|v| v.status == EvidenceStatus::Pass && v.exit_code == Some(0))
        };
        if !verified {
            return Err(ShopError::CannotClose(parent_id.to_string()));
        }
        let parent = state.parents.get_mut(parent_id).unwrap();
        parent.state = ParentState::Closed;
        parent.touch();
        let closed = parent.clone();
        state.claims.retain(|c| c.parent_id != parent_id);
        self.store.save(&state)?;
        self.event(json!({
            "op": "close",
            "parent": parent_id,
            "state": "CLOSED",
        }))?;
        Ok(closed)
    }

    pub fn status(&self, parent_id: Option<&str>) -> Result<String> {
        let state = self.store.load()?;
        let mut out = format_status(&state, parent_id)?;
        out.push_str("WORKERS\n");
        let roster = self.store.load_workers()?;
        if roster.workers.is_empty() {
            out.push_str("  (none; add via shop ui)\n");
        } else {
            for w in &roster.workers {
                out.push_str(&format!(
                    "  {}  name={}  backend={}  model={}  github_skills={}\n",
                    w.peer, w.name, w.intelligence.backend, w.intelligence.model, w.github_skills
                ));
            }
        }
        out.push_str("GITHUB\n");
        match self.store.load_github()? {
            Some(r) => out.push_str(&format!(
                "  connected {} default_branch={}\n",
                r.slug(),
                r.default_branch.as_deref().unwrap_or("(unset)")
            )),
            None => out.push_str("  (not connected)\n"),
        }
        Ok(out)
    }

    pub fn state(&self) -> Result<ShopState> {
        self.store.load()
    }

    fn event(&self, event: Value) -> Result<()> {
        self.store.append_event(&event)
    }
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.contains("..")
        || id.chars().any(|c| c.is_whitespace() || c == '\0')
    {
        return Err(ShopError::InvalidId(id.to_string()));
    }
    Ok(())
}

fn check_overlap(state: &ShopState, paths: &[String]) -> Result<()> {
    for claim in &state.claims {
        let open = state
            .parents
            .get(&claim.parent_id)
            .map(|p| p.state.is_open())
            .unwrap_or(true);
        if !open {
            continue;
        }
        for new_p in paths {
            for old_p in &claim.paths {
                if paths_overlap(new_p, old_p) {
                    return Err(ShopError::PathOverlap {
                        path: new_p.clone(),
                        parent: claim.parent_id.clone(),
                        child: claim.child_id.clone(),
                        existing: old_p.clone(),
                    });
                }
            }
        }
    }
    Ok(())
}

fn load_handoff(spec: &str) -> Result<Handoff> {
    let path = Path::new(spec);
    let (bytes, source) = if path.is_file() {
        (fs::read(path)?, spec.to_string())
    } else {
        (spec.as_bytes().to_vec(), "inline".to_string())
    };
    if bytes.is_empty() {
        return Err(ShopError::IncompleteEvidence(
            "empty handoff; accept refuses to fake PASS".into(),
        ));
    }
    let status = handoff_status(&bytes);
    Ok(Handoff {
        hash: fingerprint(&bytes),
        status,
        source,
        recorded_at: unix_now(),
    })
}

fn handoff_status(bytes: &[u8]) -> String {
    if let Ok(v) = serde_json::from_slice::<Value>(bytes) {
        if let Some(s) = v
            .get("status")
            .or_else(|| v.get("result"))
            .and_then(|x| x.as_str())
        {
            let up = s.to_ascii_uppercase();
            if up == "PASS" || up == "DONE" {
                return up;
            }
        }
    }
    "PASS".to_string()
}

fn run_verify_command(cmd: &str) -> VerifyRecord {
    let recorded_at = unix_now();
    match Command::new("sh").arg("-c").arg(cmd).output() {
        Err(e) => VerifyRecord {
            cmd: cmd.to_string(),
            exit_code: None,
            last_lines: format!("command failed to execute: {e}"),
            status: EvidenceStatus::Wait,
            recorded_at,
        },
        Ok(out) => {
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            if !out.stderr.is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&String::from_utf8_lossy(&out.stderr));
            }
            let last_lines = last_lines(&text, 20);
            let exit_code = out.status.code();
            // Exit 0 is Evidence. Missing exit code is WAIT. Never mark VERIFIED
            // without a recorded successful command.
            let status = if exit_code == Some(0) {
                EvidenceStatus::Pass
            } else {
                EvidenceStatus::Wait
            };
            VerifyRecord {
                cmd: cmd.to_string(),
                exit_code,
                last_lines,
                status,
                recorded_at,
            }
        }
    }
}

fn last_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

fn format_status(state: &ShopState, parent_id: Option<&str>) -> Result<String> {
    let mut out = String::new();
    let parents: Vec<&Parent> = if let Some(id) = parent_id {
        match state.parents.get(id) {
            Some(p) => vec![p],
            None => return Err(ShopError::ParentNotFound(id.to_string())),
        }
    } else {
        state.parents.values().collect()
    };

    if parents.is_empty() {
        out.push_str("no parents\n");
    }
    for parent in parents {
        out.push_str(&format!("PARENT {}  {}\n", parent.id, parent.state));
        out.push_str(&format!("  title: {}\n", parent.title));
        out.push_str(&format!("  body: {}\n", parent.body));
        if let Some(cmd) = &parent.verify_cmd {
            out.push_str(&format!("  verify_cmd: {cmd}\n"));
        }
        if let Some(v) = &parent.verify {
            out.push_str(&format!("  verify: {} exit={:?}\n", v.status, v.exit_code));
        }
        if parent.children.is_empty() {
            out.push_str("  children: (none; shop never invents a lane)\n");
        } else {
            out.push_str("  children:\n");
            for child in parent.children.values() {
                let handoff = child
                    .handoff
                    .as_ref()
                    .map(|h| format!("  handoff={} {}", h.hash, h.status))
                    .unwrap_or_default();
                let branch = child
                    .branch
                    .as_ref()
                    .map(|b| format!("  branch={} created={}", b, child.branch_created))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "    - {}  {}  peer={}  dispatched={}  paths={}{}{}\n",
                    child.id,
                    child.state,
                    child.peer,
                    child.dispatched,
                    child.paths.join(","),
                    handoff,
                    branch
                ));
            }
        }
    }
    out.push_str("CLAIMS\n");
    if state.claims.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for claim in &state.claims {
            out.push_str(&format!(
                "  {}/{}  peer={}  paths={}\n",
                claim.parent_id,
                claim.child_id,
                claim.peer,
                claim.paths.join(",")
            ));
        }
    }
    Ok(out)
}
