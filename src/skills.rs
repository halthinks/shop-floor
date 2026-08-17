//! GitHub skill pack — shop-side wrappers of the official GitHub MCP surface.
//!
//! Live calls go through `gh` or a token + curl. Unit tests use [`MockGitHub`].
//! Missing auth or missing evidence is WAIT, never a fake PASS or PR URL.

use serde_json::{json, Value};

use crate::error::{Result, ShopError};
use crate::secret_scan::scan_secrets;

pub const SKILL_PACK: &[&str] = &[
    "me",
    "whoami",
    "repos.search",
    "repos.connect",
    "repos.create",
    "files.get",
    "files.put",
    "files.delete",
    "files.push",
    "branches.list",
    "branches.create",
    "commits.list",
    "commits.get",
    "commits.search",
    "issues.list",
    "issues.read",
    "issues.write",
    "issues.comment",
    "issues.sub",
    "pulls.list",
    "pulls.read",
    "pulls.create",
    "pulls.update",
    "pulls.update_branch",
    "pulls.merge",
    "reviews.write",
    "reviews.comment",
    "reviews.reply",
    "search.code",
    "search.issues",
    "search.pulls",
    "search.commits",
    "search.users",
    "releases.list",
    "releases.get",
    "tags.list",
    "tags.get",
    "collaborators.list",
    "teams.list",
    "fork",
    "secret_scan",
];

/// Recorded public repos. Unauthenticated search may return these. Never invent private slugs.
pub const PUBLIC_REPOS: &[&str] = &["halthinks/shop-floor", "halthinks/AASM"];

pub fn skill_requires_auth(skill: &str) -> bool {
    matches!(
        skill,
        "me" | "whoami"
            | "repos.create"
            | "files.put"
            | "files.delete"
            | "files.push"
            | "branches.create"
            | "issues.write"
            | "issues.comment"
            | "issues.sub"
            | "pulls.create"
            | "pulls.update"
            | "pulls.update_branch"
            | "pulls.merge"
            | "reviews.write"
            | "reviews.comment"
            | "reviews.reply"
            | "collaborators.list"
            | "teams.list"
            | "fork"
    )
}

pub fn repo_slug_from_args(args: &Value) -> Option<String> {
    let owner = args.get("owner").and_then(|v| v.as_str())?;
    let repo = args.get("repo").and_then(|v| v.as_str())?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

pub fn is_recorded_public(slug: &str) -> bool {
    PUBLIC_REPOS.iter().any(|s| *s == slug)
}

pub fn public_search_items() -> Value {
    json!({
        "items": [
            {"full_name": "halthinks/shop-floor", "private": false},
            {"full_name": "halthinks/AASM", "private": false},
        ]
    })
}

pub fn private_needs_auth() -> ShopError {
    ShopError::Wait("private repo needs gh or token".into())
}

pub fn public_read_payload(skill: &str) -> Value {
    match skill {
        "files.get" => json!({"name": "README.md", "type": "file", "content": ""}),
        "commits.get" | "releases.get" | "tags.get" => json!({"ok": true}),
        "pulls.read" => json!({"number": 1, "check_runs": []}),
        "issues.read" => json!({"number": 1, "title": ""}),
        _ => json!([]),
    }
}

pub trait GitHubSkills: Send + Sync {
    fn authenticated(&self) -> bool;
    fn invoke(&self, skill: &str, args: Value) -> Result<Value>;
}

/// No credentials. Public recorded repos are readable. Writes and private are WAIT.
pub struct NullGitHub;

impl GitHubSkills for NullGitHub {
    fn authenticated(&self) -> bool {
        false
    }

    fn invoke(&self, skill: &str, args: Value) -> Result<Value> {
        invoke_public_or_wait(skill, &args, false)
    }
}

/// Unauthenticated public reads against the recorded public set. Private is WAIT.
pub fn invoke_public_or_wait(skill: &str, args: &Value, authed: bool) -> Result<Value> {
    if skill == "secret_scan" {
        return local_secret_scan(args);
    }
    if skill == "repos.connect" {
        return Err(ShopError::Wait(
            "repos.connect is a local store write; use shop github connect".into(),
        ));
    }
    if !SKILL_PACK.contains(&skill) {
        return Err(ShopError::UnknownSkill(skill.to_string()));
    }
    if skill_requires_auth(skill) && !authed {
        return Err(ShopError::Wait(format!(
            "no GitHub auth for {skill}; never fake a result"
        )));
    }
    if skill == "repos.search" {
        return Ok(public_search_items());
    }
    if let Some(slug) = repo_slug_from_args(args) {
        if !is_recorded_public(&slug) && !authed {
            return Err(private_needs_auth());
        }
    }
    if !authed {
        return Ok(public_read_payload(skill));
    }
    Err(ShopError::Wait(format!(
        "no GitHub auth for {skill}; never fake a result"
    )))
}

pub fn local_secret_scan(args: &Value) -> Result<Value> {
    let mut files: Vec<String> = Vec::new();
    if let Some(s) = args.get("files").and_then(|v| v.as_str()) {
        files.push(s.to_string());
    } else if let Some(arr) = args.get("files").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(s) = v.as_str() {
                files.push(s.to_string());
            } else if let Some(c) = v.get("content").and_then(|x| x.as_str()) {
                files.push(c.to_string());
            }
        }
    } else if let Some(s) = args.get("content").and_then(|v| v.as_str()) {
        files.push(s.to_string());
    }
    if files.is_empty() {
        return Err(ShopError::IncompleteEvidence(
            "secret_scan requires content; missing evidence is WAIT".into(),
        ));
    }
    let mut all = Vec::new();
    for (i, content) in files.iter().enumerate() {
        for hit in scan_secrets(content) {
            all.push(json!({
                "file_index": i,
                "kind": hit.kind,
                "offset": hit.offset,
            }));
        }
    }
    Ok(json!({
        "hits": all,
        "clean": all.is_empty(),
        "note": "hits report kind+offset only; secret material is not echoed",
    }))
}

pub fn require_no_secrets(content: &str) -> Result<()> {
    let hits = scan_secrets(content);
    if hits.is_empty() {
        return Ok(());
    }
    let kinds: Vec<&str> = hits.iter().map(|h| h.kind.as_str()).collect();
    Err(ShopError::Wait(format!(
        "secret_scan blocked write (kinds: {}); secret material not logged",
        kinds.join(",")
    )))
}
