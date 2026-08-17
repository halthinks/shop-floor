//! In-memory GitHub skill backend. Unit tests must use this — never the live API.

use std::sync::Mutex;

use serde_json::{json, Value};

use crate::error::{Result, ShopError};
use crate::skills::{local_secret_scan, GitHubSkills, SKILL_PACK};

#[derive(Default)]
struct MockInner {
    calls: Vec<String>,
    create_pr: bool,
    create_branch: bool,
    checks_pass: bool,
    merged: bool,
}

pub struct MockGitHub {
    authed: bool,
    inner: Mutex<MockInner>,
}

impl MockGitHub {
    pub fn unauth() -> Self {
        Self {
            authed: false,
            inner: Mutex::new(MockInner::default()),
        }
    }

    pub fn authed() -> Self {
        Self {
            authed: true,
            inner: Mutex::new(MockInner::default()),
        }
    }

    pub fn with_create_pr(self, yes: bool) -> Self {
        self.inner.lock().unwrap().create_pr = yes;
        self
    }

    pub fn with_create_branch(self, yes: bool) -> Self {
        self.inner.lock().unwrap().create_branch = yes;
        self
    }

    pub fn with_checks_pass(self, yes: bool) -> Self {
        self.inner.lock().unwrap().checks_pass = yes;
        self
    }

    pub fn calls(&self) -> Vec<String> {
        self.inner.lock().unwrap().calls.clone()
    }

    pub fn merged(&self) -> bool {
        self.inner.lock().unwrap().merged
    }

    fn record(&self, skill: &str) {
        self.inner.lock().unwrap().calls.push(skill.to_string());
    }
}

impl GitHubSkills for MockGitHub {
    fn authenticated(&self) -> bool {
        self.authed
    }

    fn invoke(&self, skill: &str, args: Value) -> Result<Value> {
        if skill == "secret_scan" {
            self.record(skill);
            return local_secret_scan(&args);
        }
        if !SKILL_PACK.contains(&skill) {
            return Err(ShopError::UnknownSkill(skill.to_string()));
        }
        if skill == "repos.connect" {
            return Err(ShopError::Wait(
                "repos.connect is a local store write; use shop github connect".into(),
            ));
        }
        if !self.authed {
            return Err(ShopError::Wait(format!(
                "no GitHub auth for {skill}; never fake a result"
            )));
        }
        self.record(skill);
        match skill {
            "me" | "whoami" => Ok(json!({"login": "mock-user", "id": 1})),
            "repos.search" => Ok(json!({"items": []})),
            "repos.create" => {
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("new");
                let private = args
                    .get("private")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                Ok(json!({"full_name": format!("mock-user/{name}"), "private": private}))
            }
            "files.get" | "files.put" | "files.delete" | "files.push" => {
                if let Some(c) = args.get("content").and_then(|v| v.as_str()) {
                    crate::skills::require_no_secrets(c)?;
                }
                if let Some(files) = args.get("files").and_then(|v| v.as_array()) {
                    for f in files {
                        if let Some(c) = f.get("content").and_then(|v| v.as_str()) {
                            crate::skills::require_no_secrets(c)?;
                        }
                    }
                }
                Ok(json!({"ok": true}))
            }
            "branches.list" => Ok(json!([])),
            "branches.create" => {
                if !self.inner.lock().unwrap().create_branch {
                    return Err(ShopError::Wait(
                        "branch not created; intended ref recorded only".into(),
                    ));
                }
                let branch = args.get("branch").and_then(|v| v.as_str()).unwrap_or("");
                Ok(json!({"ref": format!("refs/heads/{branch}")}))
            }
            "commits.list" | "commits.search" => Ok(json!([])),
            "commits.get" => Ok(json!({"sha": "abc123"})),
            "issues.list" => Ok(json!([])),
            "issues.read" | "issues.write" | "issues.comment" | "issues.sub" => {
                Ok(json!({"ok": true}))
            }
            "pulls.list" => Ok(json!([])),
            "pulls.read" => {
                let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("get");
                if method == "get_check_runs" || method == "get_status" {
                    let pass = self.inner.lock().unwrap().checks_pass;
                    return Ok(json!({
                        "status": if pass { "success" } else { "pending" },
                        "check_runs": if pass {
                            json!([{"name": "ci", "conclusion": "success"}])
                        } else {
                            json!([])
                        },
                    }));
                }
                Ok(json!({"number": args.get("pullNumber").cloned().unwrap_or(json!(1))}))
            }
            "pulls.create" => {
                if !self.inner.lock().unwrap().create_pr {
                    return Err(ShopError::Wait(
                        "no real PR created; never fake a PR URL".into(),
                    ));
                }
                Ok(json!({
                    "number": 7,
                    "html_url": "https://example.test/mock/pull/7",
                    "draft": true,
                }))
            }
            "pulls.update" | "pulls.update_branch" => Ok(json!({"ok": true})),
            "pulls.merge" => {
                self.inner.lock().unwrap().merged = true;
                Ok(json!({"merged": true, "note": "explicit merge only"}))
            }
            "reviews.write" | "reviews.comment" | "reviews.reply" => Ok(json!({"ok": true})),
            "search.code" | "search.issues" | "search.pulls" | "search.commits"
            | "search.users" => Ok(json!({"items": []})),
            "releases.list" | "tags.list" | "collaborators.list" | "teams.list" => Ok(json!([])),
            "releases.get" | "tags.get" => Ok(json!({"ok": true})),
            "fork" => Ok(json!({"full_name": "mock-user/fork"})),
            other => Err(ShopError::UnknownSkill(other.to_string())),
        }
    }
}
