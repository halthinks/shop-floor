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

use crate::awareness::{
    Awareness, ChildView, GitHubAwareness, ParentView, ProjectView, WorkerView,
};
use crate::error::{Result, ShopError};
use crate::floor::FloorMemory;
use crate::github::{child_branch_name, GitHubRepo};
use crate::hash::{fingerprint, unix_now};
use crate::mailbox::{observe, write_assign, AssignRecord, MailboxObservation, DEFAULT_FROM};
use crate::memory::{MemoryTier, ShopMemory};
use crate::paths::{parse_paths, paths_overlap};
use crate::project::{
    default_recents_path, format_recents, load_recents, origin_github_slug, project_name,
    project_root_from_store, recent_path_for_repo, record_recent, refuse_platform,
    slugs_from_github_search, store_dir_for, touch_recent_github,
};
use crate::skills::{GitHubSkills, NullGitHub};
use crate::steer::{parse_steer, steer_to_supergrok, SteerLine, SteerVerb};
use crate::store::Store;
use crate::types::{
    Child, ChildState, Claim, EvidenceStatus, Handoff, Parent, ParentState, ReduceChild,
    ReducePackage, ShopState, VerifyRecord,
};
use crate::workers::{Backend, Intelligence, Worker, WorkerRoster};

#[derive(Clone)]
pub struct Shop {
    store: Store,
    project_root: PathBuf,
    recents_path: PathBuf,
    mailbox: Option<PathBuf>,
    from: String,
    github: Arc<dyn GitHubSkills>,
}

impl Shop {
    fn from_store(store: Store) -> Self {
        let root = store.root().to_path_buf();
        Self {
            project_root: project_root_from_store(&root),
            recents_path: root.join("recents.json"),
            mailbox: Some(root.join("mailbox")),
            store,
            from: DEFAULT_FROM.to_string(),
            github: Arc::new(NullGitHub),
        }
    }

    pub fn init(store: impl AsRef<Path>) -> Result<Self> {
        let store = Store::init(store)?;
        let mut shop = Self::from_store(store);
        shop.ensure_seeded_workers()?;
        shop.ensure_default_github()?;
        Ok(shop)
    }

    pub fn open_store(store: impl AsRef<Path>) -> Result<Self> {
        let store = Store::open(store)?;
        let mut shop = Self::from_store(store);
        shop.ensure_seeded_workers()?;
        shop.ensure_default_github()?;
        Ok(shop)
    }

    pub fn with_recents_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.recents_path = path.into();
        self
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub fn recents_path(&self) -> &Path {
        &self.recents_path
    }

    /// Open a project folder. Creates `.shop` if missing. Refuses Platform paths.
    pub fn open_project(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_project_with_recents(path, default_recents_path())
    }

    pub fn open_project_with_recents(
        path: impl AsRef<Path>,
        recents_path: impl AsRef<Path>,
    ) -> Result<Self> {
        let path = path.as_ref();
        refuse_platform(path)?;
        refuse_platform(recents_path.as_ref())?;
        let store_dir = store_dir_for(path)?;
        refuse_platform(&store_dir)?;
        if let Some(parent) = store_dir.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut shop =
            if store_dir.join("meta.json").exists() || store_dir.join("snapshot.json").exists() {
                Shop::open_store(&store_dir)?
            } else {
                Shop::init(&store_dir)?
            };
        shop.project_root = project_root_from_store(shop.store.root());
        if shop.project_root.as_os_str().is_empty() {
            shop.project_root = path.to_path_buf();
        }
        shop.recents_path = recents_path.as_ref().to_path_buf();
        let slug = shop.github_repo().ok().flatten().map(|r| r.slug());
        record_recent(&shop.recents_path, &shop.project_root, slug.as_deref())?;
        if let Some(origin) = origin_github_slug(&shop.project_root) {
            let _ = shop.github_connect(&origin, None, None);
        }
        Ok(shop)
    }

    /// Live GitHub repo list. Public recorded slugs without a token. Never invent private slugs.
    pub fn github_repo_catalog(&self) -> Value {
        match self
            .github
            .invoke("repos.search", json!({"query": "user:halthinks"}))
        {
            Ok(v) => json!(slugs_from_github_search(&v)),
            Err(e) => {
                if !self.github.authenticated() {
                    json!(crate::skills::PUBLIC_REPOS
                        .iter()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>())
                } else {
                    json!(format!("WAIT: {e}"))
                }
            }
        }
    }

    fn ensure_seeded_workers(&mut self) -> Result<()> {
        if !self.store.load_workers()?.workers.is_empty() {
            return Ok(());
        }
        for seed in crate::workers::SEEDED_WORKERS {
            self.add_worker(seed.peer, seed.title, seed.backend, seed.model, true)?;
        }
        Ok(())
    }

    /// Record shop's own public repo. No clone, no lane, no Platform write.
    fn ensure_default_github(&mut self) -> Result<()> {
        if self.store.load_github()?.is_some() {
            return Ok(());
        }
        self.github_connect(crate::github::DEFAULT_REPO, Some("main"), None)?;
        Ok(())
    }

    pub fn with_github(mut self, github: Arc<dyn GitHubSkills>) -> Self {
        self.github = github;
        self
    }

    pub fn with_mailbox(mut self, mailbox: Option<PathBuf>) -> Self {
        self.mailbox = Some(mailbox.unwrap_or_else(|| self.store.root().join("mailbox")));
        self
    }

    pub fn mailbox_root(&self) -> Option<&Path> {
        self.mailbox.as_deref()
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

    pub fn github_arc(&self) -> Arc<dyn GitHubSkills> {
        self.github.clone()
    }

    pub fn from_name(&self) -> &str {
        &self.from
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
        self.note_memory(&format!("worker added {peer}"))?;
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
        let _ = touch_recent_github(&self.recents_path, &self.project_root, &repo.slug());
        self.event(json!({
            "op": "github_connect",
            "repo": repo.slug(),
            "note": "token if any is written to .shop/github.token and is gitignored",
        }))?;
        self.note_memory(&format!("github connected {}", repo.slug()))?;
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
        // Default shop-floor record is a catalog pointer, not a PR claim.
        if repo.slug() == crate::github::DEFAULT_REPO
            && parent.github_pr_url.is_none()
            && parent.github_pr_number.is_none()
        {
            return None;
        }
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
        self.open_with_scope(id, title, body, "")
    }

    /// Open a parent lease. Optional claimed paths are parent-subset scope.
    /// Empty paths: children define claims; overlap is still rejected.
    pub fn open_with_scope(
        &mut self,
        id: &str,
        title: &str,
        body: &str,
        paths_csv: &str,
    ) -> Result<Parent> {
        validate_id(id)?;
        let mut state = self.store.load()?;
        if state.parents.contains_key(id) {
            return Err(ShopError::ParentExists(id.to_string()));
        }
        let mut parent = Parent::new(id.to_string(), title.to_string(), body.to_string());
        parent.claimed_paths = crate::paths::parse_paths(paths_csv);
        parent.lease_note = crate::aasm_map::LEASE_NOTE.to_string();
        state.parents.insert(id.to_string(), parent.clone());
        self.store.save(&state)?;
        self.event(json!({
            "op": "open",
            "parent": id,
            "state": "HELD",
            "aasm_note": crate::aasm_map::LEASE_NOTE,
        }))?;
        self.note_memory(&format!("job opened {id} HELD"))?;
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
        if let Some(path) =
            crate::aasm_map::paths_within_parent_scope(&paths, &parent.claimed_paths)
        {
            return Err(ShopError::ParentSubset {
                path,
                parent: parent_id.to_string(),
            });
        }
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
        self.note_memory(&format!("split {parent_id}/{child_id} peer={peer}"))?;
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
        if !records.is_empty() {
            if let Some(w) = self.observe_mailbox().wait {
                self.event(json!({
                    "op": "mailbox_wait",
                    "parent": parent_id,
                    "note": w,
                }))?;
            }
        }
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
            child.backjump_note = Some(crate::aasm_map::BACKJUMP_NOTE.to_string());
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
            "note": crate::aasm_map::BACKJUMP_NOTE,
        }))?;
        self.note_memory(&format!(
            "bounce {parent_id}/{child_id} stays peer={} paths={}",
            bounced.peer,
            bounced.paths.join(",")
        ))?;
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
            aasm_note: crate::aasm_map::REDUCE_NOT_RECOMBINE.to_string(),
        };
        self.store.save(&state)?;
        if let Some(p) = state.parents.get(parent_id) {
            self.store.append_floor_history(p)?;
        }
        let bytes = serde_json::to_vec_pretty(&package)?;
        self.store
            .write_under(format!("reduce/{parent_id}.json"), &bytes)?;
        self.event(json!({
            "op": "reduce",
            "parent": parent_id,
            "note": "reduce package is Evidence listing; does not land onto a canonical repo",
        }))?;
        self.note_memory(&format!("reduce {parent_id} REDUCED"))?;
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

        let assigned_open = parent
            .children
            .values()
            .any(|c| c.state == ChildState::Assigned);
        let mut record = match cmd {
            None => VerifyRecord {
                cmd: String::new(),
                exit_code: None,
                last_lines: "no verify command recorded".into(),
                status: EvidenceStatus::Wait,
                class: Default::default(),
                aasm_note: String::new(),
                recorded_at: unix_now(),
            }
            .classify(),
            Some(cmd) => {
                parent.verify_cmd = Some(cmd.clone());
                run_verify_command(&cmd)
            }
        };
        if assigned_open && record.status == EvidenceStatus::Pass {
            record.status = EvidenceStatus::Wait;
            record.last_lines = format!(
                "{}\nmandatory child still ASSIGNED (obligation open)",
                record.last_lines
            );
        }
        if let Some(reason) = self.github_verify_block(parent) {
            record.status = EvidenceStatus::Wait;
            if !record.last_lines.is_empty() {
                record.last_lines.push('\n');
            }
            record.last_lines.push_str(&reason);
        }
        record = record.classify();
        parent.verify = Some(record.clone());
        parent.state = match record.class {
            crate::aasm_map::EvidenceClass::Pass => ParentState::Verified,
            _ => ParentState::VerifyWait,
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
        match record.status {
            EvidenceStatus::Wait => {
                self.note_memory(&format!("verify {parent_id} WAIT"))?;
            }
            EvidenceStatus::Pass => {
                self.note_memory(&format!("verify {parent_id} PASS"))?;
            }
        }
        Ok(record)
    }

    /// Merge is allowed only after verify recorded PASS / parent VERIFIED.
    /// Missing PR, auth, or verify is WAIT — never a fake merge or PR URL.
    pub fn merge_verified(&mut self, parent_id: &str) -> Result<Value> {
        let repo = self.store.load_github()?;
        let mut state = self.store.load()?;
        let parent = state
            .parents
            .get_mut(parent_id)
            .ok_or_else(|| ShopError::ParentNotFound(parent_id.to_string()))?;
        let verified = parent.state == ParentState::Verified
            && parent
                .verify
                .as_ref()
                .is_some_and(|v| v.status == EvidenceStatus::Pass);
        if !verified {
            let reason =
                "merge blocked until shop verify recorded PASS / parent VERIFIED".to_string();
            parent.github_merge_status = Some(EvidenceStatus::Wait);
            parent.github_merge_reason = Some(reason.clone());
            self.store.save(&state)?;
            self.event(json!({
                "op": "merge_blocked",
                "parent": parent_id,
                "reason": reason,
            }))?;
            return Err(ShopError::Wait(reason));
        }
        let Some(n) = parent.github_pr_number else {
            let reason = "no real PR; never fake a merge or a PR URL".to_string();
            parent.github_merge_status = Some(EvidenceStatus::Wait);
            parent.github_merge_reason = Some(reason.clone());
            self.store.save(&state)?;
            self.event(json!({
                "op": "merge_blocked",
                "parent": parent_id,
                "reason": reason,
            }))?;
            return Err(ShopError::Wait(reason));
        };
        if !self.github.authenticated() {
            let reason = "WAIT: GitHub token missing; never fake a merge".to_string();
            parent.github_merge_status = Some(EvidenceStatus::Wait);
            parent.github_merge_reason = Some(reason.clone());
            self.store.save(&state)?;
            self.event(json!({
                "op": "merge_blocked",
                "parent": parent_id,
                "reason": reason,
            }))?;
            return Err(ShopError::Wait(reason));
        }
        let Some(repo) = repo else {
            let reason = "WAIT: GitHub repo not connected; never fake a merge".to_string();
            parent.github_merge_status = Some(EvidenceStatus::Wait);
            parent.github_merge_reason = Some(reason.clone());
            self.store.save(&state)?;
            self.event(json!({
                "op": "merge_blocked",
                "parent": parent_id,
                "reason": reason,
            }))?;
            return Err(ShopError::Wait(reason));
        };
        match self.github.invoke(
            "pulls.merge",
            json!({
                "owner": repo.owner,
                "repo": repo.name,
                "pullNumber": n,
            }),
        ) {
            Ok(v) => {
                parent.github_merged = true;
                parent.github_merge_status = Some(EvidenceStatus::Pass);
                parent.github_merge_reason = Some(crate::aasm_map::MERGE_ACK_NOTE.to_string());
                self.store.save(&state)?;
                self.event(json!({
                    "op": "merge",
                    "parent": parent_id,
                    "pull": n,
                    "note": crate::aasm_map::MERGE_ACK_NOTE,
                }))?;
                Ok(v)
            }
            Err(e) => {
                parent.github_merged = false;
                parent.github_merge_status = Some(EvidenceStatus::Wait);
                parent.github_merge_reason = Some(e.to_string());
                self.store.save(&state)?;
                Err(e)
            }
        }
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
        let assigned_open = state
            .parents
            .get(parent_id)
            .map(|p| p.children.values().any(|c| c.state == ChildState::Assigned))
            .unwrap_or(false);
        if assigned_open {
            return Err(ShopError::ObligationOpen(parent_id.to_string()));
        }
        if !verified {
            return Err(ShopError::CannotClose(parent_id.to_string()));
        }
        let parent = state.parents.get_mut(parent_id).unwrap();
        parent.state = ParentState::Closed;
        parent.touch();
        let closed = parent.clone();
        state.claims.retain(|c| c.parent_id != parent_id);
        self.store.append_floor_history(&closed)?;
        self.store.save(&state)?;
        self.event(json!({
            "op": "close",
            "parent": parent_id,
            "state": "CLOSED",
        }))?;
        self.note_memory(&format!("closed {parent_id}"))?;
        Ok(closed)
    }

    pub fn observe_mailbox(&self) -> MailboxObservation {
        observe(self.mailbox.as_deref(), &self.store.outbox_dir())
    }

    pub fn awareness(&self, parent_id: Option<&str>) -> Result<Awareness> {
        let _ = self.ingest_replies();
        let state = self.store.load()?;
        let roster = self.store.load_workers()?;
        let mailbox = self.observe_mailbox();
        let repo = self.store.load_github()?;
        let authed = self.github.authenticated();
        let public_repo = repo
            .as_ref()
            .map(|r| crate::skills::is_recorded_public(&r.slug()))
            .unwrap_or(false);
        let github_wait = if repo.is_none() {
            Some("WAIT: GitHub repo not connected".to_string())
        } else if !authed && !public_repo {
            Some("WAIT: private repo needs gh or token".to_string())
        } else {
            None
        };
        let mut checks = json!("WAIT: missing PR or checks");
        let mut issue_count = None;
        let mut pr_count = None;
        let mut check_count = None;
        if let (Some(r), true) = (&repo, authed) {
            if let Ok(v) = self
                .github
                .invoke("issues.list", json!({"owner": r.owner, "repo": r.name}))
            {
                if let Some(arr) = v.as_array() {
                    issue_count = Some(arr.len() as u64);
                }
            }
            if let Ok(v) = self
                .github
                .invoke("pulls.list", json!({"owner": r.owner, "repo": r.name}))
            {
                if let Some(arr) = v.as_array() {
                    pr_count = Some(arr.len() as u64);
                }
            }
            if let Some(parent) = parent_id.and_then(|id| state.parents.get(id)) {
                if let Some(n) = parent.github_pr_number {
                    checks = self
                        .github
                        .invoke(
                            "pulls.read",
                            json!({
                                "owner": r.owner,
                                "repo": r.name,
                                "pullNumber": n,
                                "method": "get_check_runs",
                            }),
                        )
                        .unwrap_or_else(|e| json!({"WAIT": e.to_string()}));
                    if let Some(arr) = checks.get("check_runs").and_then(|a| a.as_array()) {
                        check_count = Some(arr.len() as u64);
                    }
                }
            }
        }
        let parents: Vec<ParentView> = state
            .parents
            .values()
            .filter(|p| parent_id.map(|id| p.id == id).unwrap_or(true))
            .map(|p| ParentView {
                id: p.id.clone(),
                title: p.title.clone(),
                state: p.state,
                children: p
                    .children
                    .values()
                    .map(|c| ChildView {
                        id: c.id.clone(),
                        peer: c.peer.clone(),
                        state: c.state,
                        paths: c.paths.clone(),
                        branch: c.branch.clone(),
                        dispatched: c.dispatched,
                    })
                    .collect(),
                verify: p.verify.as_ref().map(|v| v.status),
                github_pr_url: p.github_pr_url.clone(),
                github_pr_status: p.github_pr_status,
                github_merged: p.github_merged,
                github_merge_status: p.github_merge_status,
                github_merge_reason: p.github_merge_reason.clone(),
            })
            .collect();
        let mut waits = Vec::new();
        if let Some(w) = &mailbox.wait {
            waits.push(w.clone());
        }
        if let Some(w) = &github_wait {
            waits.push(w.clone());
        }
        for p in &parents {
            if p.github_merge_status == Some(EvidenceStatus::Wait) {
                if let Some(r) = &p.github_merge_reason {
                    waits.push(format!("WAIT: merge {}: {r}", p.id));
                }
            }
            if p.verify == Some(EvidenceStatus::Wait) {
                waits.push(format!("WAIT: verify {} not PASS", p.id));
            }
        }
        let workers: Vec<WorkerView> = roster
            .workers
            .into_iter()
            .map(|w| WorkerView::from_worker(w, &parents, &mailbox))
            .collect();
        let mut events = self.store.load_events();
        events.reverse();
        if events.len() > 80 {
            events.truncate(80);
        }
        Ok(Awareness {
            store_ok: true,
            project: ProjectView {
                name: project_name(&self.project_root),
                path: self.project_root.display().to_string(),
                store: self.store.root().display().to_string(),
            },
            workers,
            claims: state.claims,
            parents,
            mailbox,
            github: GitHubAwareness {
                repo: repo.as_ref().map(|r| r.slug()),
                default_branch: repo.and_then(|r| r.default_branch),
                authenticated: authed,
                token_present: self.store.has_token_file() || authed,
                wait: github_wait,
                checks,
                issue_count,
                pr_count,
                check_count,
            },
            events,
            waits,
        })
    }

    pub fn status(&self, parent_id: Option<&str>) -> Result<String> {
        let a = self.awareness(parent_id)?;
        let mut out = String::new();
        out.push_str("STATUS (truth panel; missing pieces are WAIT, never invented)\n");
        if !a.waits.is_empty() {
            out.push_str("WAITS\n");
            for w in &a.waits {
                out.push_str(&format!("  {w}\n"));
            }
        }
        out.push_str("WORKERS\n");
        if a.workers.is_empty() {
            out.push_str("  (none; add via shop ui)\n");
        } else {
            for w in &a.workers {
                out.push_str(&format!(
                    "  {}  name={}  backend={}  model={}  github_skills={}  action={}\n",
                    w.peer,
                    w.name,
                    w.intelligence.backend,
                    w.intelligence.model,
                    w.github_skills,
                    w.action
                ));
            }
        }
        out.push_str("PARENTS\n");
        if a.parents.is_empty() {
            out.push_str("  (none)\n");
        } else {
            for p in &a.parents {
                out.push_str(&format!("  {}  {}\n", p.id, p.state));
                for c in &p.children {
                    out.push_str(&format!(
                        "    child {}  {}  peer={}  paths={}\n",
                        c.id,
                        c.state,
                        c.peer,
                        c.paths.join(",")
                    ));
                }
            }
        }
        out.push_str("CLAIMS\n");
        if a.claims.is_empty() {
            out.push_str("  (none)\n");
        } else {
            for claim in &a.claims {
                out.push_str(&format!(
                    "  {}/{}  peer={}  paths={}\n",
                    claim.parent_id,
                    claim.child_id,
                    claim.peer,
                    claim.paths.join(",")
                ));
            }
        }
        out.push_str("MAILBOX\n");
        if let Some(w) = &a.mailbox.wait {
            out.push_str(&format!("  {w}\n"));
        } else {
            out.push_str(&format!(
                "  unread={}  outbox={}  root={}\n",
                a.mailbox.unread_total,
                a.mailbox.outbox_count,
                a.mailbox.root.as_deref().unwrap_or("?")
            ));
            for p in &a.mailbox.peers {
                out.push_str(&format!(
                    "  peer={} unread={} latest_assign={} latest_handoff={}\n",
                    p.peer,
                    p.unread,
                    p.latest_assign.as_deref().unwrap_or("-"),
                    p.latest_handoff.as_deref().unwrap_or("-")
                ));
            }
        }
        out.push_str("GITHUB\n");
        if let Some(w) = &a.github.wait {
            out.push_str(&format!("  {w}\n"));
        }
        if let Some(r) = &a.github.repo {
            out.push_str(&format!(
                "  repo={} auth={} token_present={}\n",
                r, a.github.authenticated, a.github.token_present
            ));
        }
        Ok(out)
    }

    pub fn state(&self) -> Result<ShopState> {
        self.store.load()
    }

    fn event(&self, mut event: Value) -> Result<()> {
        if event.get("title").is_none() {
            if let Some(op) = event.get("op").and_then(|v| v.as_str()) {
                event["title"] = json!(crate::feed::event_title(op));
            }
        }
        self.store.append_event(&event)
    }

    pub fn events(&self) -> Vec<Value> {
        self.store.load_events()
    }

    pub fn event_log(&self, n: usize) -> String {
        crate::feed::format_log(&self.store.load_events(), n)
    }

    pub fn feed_atom(&self) -> String {
        crate::feed::atom(&self.store.load_events())
    }

    pub fn feed_rss(&self) -> String {
        crate::feed::rss(&self.store.load_events())
    }

    pub fn steer_transcript(&self) -> Vec<Value> {
        let _ = self.ingest_replies();
        self.store.load_steer()
    }

    pub fn remember(&mut self, tier: MemoryTier, fact: &str) -> Result<ShopMemory> {
        crate::memory::remember(self.store.root(), tier, fact)
    }

    pub fn forget(&mut self, fact: &str) -> Result<ShopMemory> {
        crate::memory::forget(self.store.root(), fact)
    }

    pub fn memory(&self) -> ShopMemory {
        self.store.load_memory()
    }

    pub fn memory_text(&self) -> String {
        self.memory().format()
    }

    pub fn floor(&self) -> FloorMemory {
        self.store.load_floor()
    }

    pub fn floor_text(&self) -> String {
        self.floor().format()
    }

    pub fn memory_pack(&self) -> Value {
        self.memory()
            .pack_json(serde_json::to_value(self.floor()).unwrap_or(json!({})))
    }

    fn note_memory(&self, fact: &str) -> Result<()> {
        crate::memory::append_log(self.store.root(), fact)
    }

    /// Read SuperGrokHeavy replies into steer.jsonl. Never invent a line.
    pub fn ingest_replies(&self) -> Result<Vec<Value>> {
        let found = crate::replies::scan_replies(self.store.root(), self.mailbox.as_deref());
        if found.is_empty() {
            return Ok(Vec::new());
        }
        let mut seen = crate::replies::load_seen(self.store.root());
        let mut added = Vec::new();
        for reply in found {
            if seen.iter().any(|s| s == &reply.key) {
                continue;
            }
            let line = SteerLine::new("orchestrator", &reply.body, None);
            self.store.append_steer(&line)?;
            seen.push(reply.key);
            added.push(serde_json::to_value(&line).unwrap_or(json!({})));
        }
        if !added.is_empty() {
            crate::replies::save_seen(self.store.root(), &seen)?;
        }
        Ok(added)
    }

    pub fn recents(&self) -> crate::project::Recents {
        load_recents(&self.recents_path)
    }

    pub fn recents_text(&self) -> String {
        format_recents(&self.recents_path)
    }

    /// CONTROL plane. Local verbs run here. Free text goes to SuperGrokHeavy.
    pub fn steer(&mut self, text: &str) -> Result<Value> {
        let _ = self.ingest_replies();
        let text = text.trim();
        if text.is_empty() {
            return Err(ShopError::IncompleteEvidence("empty steer".into()));
        }
        let user = SteerLine::new("user", text, None);
        self.store.append_steer(&user)?;

        let mut remembered = false;
        if let Some(SteerVerb::Remember { tier, fact }) = parse_steer(text) {
            self.remember(tier, &fact)?;
            remembered = true;
        } else if let Some(verb) = parse_steer(text) {
            let (body, wait) = self.run_steer_verb(verb)?;
            let shop_line = SteerLine::new("shop", body, wait);
            self.store.append_steer(&shop_line)?;
            return Ok(json!({
                "kind": "shop",
                "line": shop_line,
                "transcript": self.store.load_steer(),
            }));
        }

        let pack = self.memory_pack();
        let (rec, _written, mut wait) = steer_to_supergrok(
            &self.store.outbox_dir(),
            self.mailbox.as_deref(),
            text,
            pack,
        )?;
        if let Some(boss_wait) = crate::replies::boss_cmd_wait() {
            wait = Some(wait.unwrap_or(boss_wait));
        }
        let note = if remembered {
            format!("remembered; to SuperGrokHeavy ({})", rec.to)
        } else {
            format!("to SuperGrokHeavy ({})", rec.to)
        };
        let mut line = SteerLine::new("shop", note, wait.clone());
        line.to = Some(rec.to.clone());
        self.store.append_steer(&line)?;
        Ok(json!({
            "kind": "steer",
            "to": rec.to,
            "type": rec.type_,
            "wait": wait,
            "line": line,
            "memory": rec.memory,
            "transcript": self.store.load_steer(),
        }))
    }

    fn run_steer_verb(&mut self, verb: SteerVerb) -> Result<(String, Option<String>)> {
        match verb {
            SteerVerb::Status => Ok((self.status(None)?, None)),
            SteerVerb::Workers => {
                let roster = self.workers()?;
                let mut s = String::from("WORKERS\n");
                for w in roster.workers {
                    s.push_str(&format!(
                        "  {}  {}  {}\n",
                        w.peer, w.intelligence.backend, w.intelligence.model
                    ));
                }
                Ok((s, None))
            }
            SteerVerb::Github => {
                let repo = self.github_repo()?.map(|r| r.slug());
                let wait = if self.github.authenticated() {
                    None
                } else {
                    Some("WAIT: GitHub token missing".into())
                };
                Ok((
                    format!(
                        "github {}  auth={}",
                        repo.as_deref().unwrap_or("not connected"),
                        self.github.authenticated()
                    ),
                    wait,
                ))
            }
            SteerVerb::AddWorker {
                peer,
                backend,
                model,
            } => {
                let w = self.add_worker(&peer, &peer, &backend, &model, true)?;
                Ok((
                    format!("added worker {} backend={}", w.peer, w.intelligence.backend),
                    None,
                ))
            }
            SteerVerb::OpenParent { id, title } => {
                self.open(&id, &title, "")?;
                Ok((format!("opened parent {id} HELD"), None))
            }
            SteerVerb::Split {
                parent,
                child,
                peer,
                paths,
            } => {
                self.split(&parent, &child, &peer, &paths, &child, "")?;
                Ok((
                    format!("split {parent}/{child} peer={peer} paths={paths}"),
                    None,
                ))
            }
            SteerVerb::Assign { parent } => {
                let n = self.assign(&parent)?.len();
                Ok((format!("assign {parent} wrote {n} record(s)"), None))
            }
            SteerVerb::Join { parent } => {
                let st = self.join(&parent)?;
                Ok((format!("join {parent} -> {st}"), None))
            }
            SteerVerb::Reduce { parent, note } => {
                self.reduce(&parent, &note)?;
                Ok((format!("reduce {parent} -> REDUCED"), None))
            }
            SteerVerb::Verify { parent, cmd } => {
                let rec = self.verify(&parent, cmd.as_deref())?;
                Ok((format!("verify {parent} {}", rec.status), None))
            }
            SteerVerb::Bounce {
                parent,
                child,
                reason,
            } => {
                self.bounce(&parent, &child, &reason)?;
                Ok((format!("bounce {parent}/{child}"), None))
            }
            SteerVerb::OpenProject { path } => {
                let next = Shop::open_project_with_recents(&path, &self.recents_path)?
                    .with_github(self.github.clone())
                    .with_from(self.from.clone())
                    .with_recents_path(self.recents_path.clone());
                let shown = next.project_root.display().to_string();
                *self = next;
                Ok((format!("opened project {shown}"), None))
            }
            SteerVerb::Remember { tier, fact } => {
                Ok((format!("remembered {} ({})", fact, tier.as_str()), None))
            }
            SteerVerb::OpenRepo { spec } => {
                let repo = self.github_connect(&spec, None, None)?;
                if let Some(known) = recent_path_for_repo(&self.recents_path, &repo.slug()) {
                    if known != self.project_root {
                        let next = Shop::open_project_with_recents(&known, &self.recents_path)?
                            .with_github(self.github.clone())
                            .with_from(self.from.clone())
                            .with_recents_path(self.recents_path.clone());
                        *self = next;
                        self.github_connect(&spec, None, None)?;
                    }
                }
                Ok((
                    format!(
                        "connected repo {} (no clone, no invented lane)",
                        repo.slug()
                    ),
                    None,
                ))
            }
        }
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
            class: Default::default(),
            aasm_note: String::new(),
            recorded_at,
        }
        .classify(),
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
                class: Default::default(),
                aasm_note: String::new(),
                recorded_at,
            }
            .classify()
        }
    }
}

fn last_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

#[allow(dead_code)]
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
