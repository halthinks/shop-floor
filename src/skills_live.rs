//! Live GitHub backend: prefer `gh`, else token + curl. Never used by unit tests.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

use crate::error::{Result, ShopError};
use crate::skills::{local_secret_scan, require_no_secrets, GitHubSkills, SKILL_PACK};

pub struct LiveGitHub {
    token: Option<String>,
    have_gh: bool,
}

impl LiveGitHub {
    /// Detect credentials from env, `.shop/github.token`, and `gh`.
    /// Does not log the token.
    pub fn detect(store_root: &Path) -> Self {
        let token = read_token(store_root);
        let have_gh = Command::new("gh")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        Self { token, have_gh }
    }

    fn request(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        if self.have_gh {
            return gh_api(method, path, body, self.token.as_deref());
        }
        if let Some(token) = &self.token {
            return curl_api(method, path, body, token);
        }
        Err(ShopError::Wait(
            "no GitHub auth (gh missing and no GITHUB_TOKEN/GH_TOKEN/.shop/github.token)".into(),
        ))
    }
}

impl GitHubSkills for LiveGitHub {
    fn authenticated(&self) -> bool {
        self.have_gh || self.token.is_some()
    }

    fn invoke(&self, skill: &str, args: Value) -> Result<Value> {
        if skill == "secret_scan" {
            return local_secret_scan(&args);
        }
        if skill == "repos.connect" {
            return Err(ShopError::Wait(
                "repos.connect is a local store write; use shop github connect".into(),
            ));
        }
        if !SKILL_PACK.contains(&skill) {
            return Err(ShopError::UnknownSkill(skill.to_string()));
        }
        if !self.authenticated() {
            return Err(ShopError::Wait(format!(
                "no GitHub auth for {skill}; never fake a result"
            )));
        }
        dispatch_live(self, skill, &args)
    }
}

fn dispatch_live(gh: &LiveGitHub, skill: &str, args: &Value) -> Result<Value> {
    let owner = args.get("owner").and_then(|v| v.as_str());
    let repo = args.get("repo").and_then(|v| v.as_str());
    match skill {
        "me" | "whoami" => gh.request("GET", "user", None),
        "repos.search" => {
            let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            gh.request("GET", &format!("search/repositories?q={q}"), None)
        }
        "repos.create" => {
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let private = args
                .get("private")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let body = json!({"name": name, "private": private, "auto_init": args.get("autoInit")});
            if let Some(org) = args.get("organization").and_then(|v| v.as_str()) {
                gh.request("POST", &format!("orgs/{org}/repos"), Some(&body))
            } else {
                gh.request("POST", "user/repos", Some(&body))
            }
        }
        "files.get" => {
            let (o, r) = owner_repo(owner, repo)?;
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let mut url = format!("repos/{o}/{r}/contents/{path}");
            if let Some(refer) = args.get("ref").and_then(|v| v.as_str()) {
                url.push_str(&format!("?ref={refer}"));
            }
            gh.request("GET", &url, None)
        }
        "files.put" => {
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
            require_no_secrets(content)?;
            let (o, r) = owner_repo(owner, repo)?;
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            gh.request("PUT", &format!("repos/{o}/{r}/contents/{path}"), Some(args))
        }
        "files.delete" => {
            let (o, r) = owner_repo(owner, repo)?;
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            gh.request(
                "DELETE",
                &format!("repos/{o}/{r}/contents/{path}"),
                Some(args),
            )
        }
        "files.push" => {
            if let Some(files) = args.get("files").and_then(|v| v.as_array()) {
                for f in files {
                    if let Some(c) = f.get("content").and_then(|v| v.as_str()) {
                        require_no_secrets(c)?;
                    }
                }
            }
            Err(ShopError::Wait(
                "files.push via live API requires a git tree commit; use gh or record-only mock in tests"
                    .into(),
            ))
        }
        "branches.list" => {
            let (o, r) = owner_repo(owner, repo)?;
            gh.request("GET", &format!("repos/{o}/{r}/branches"), None)
        }
        "branches.create" => {
            let (o, r) = owner_repo(owner, repo)?;
            let branch = args.get("branch").and_then(|v| v.as_str()).unwrap_or("");
            let from = args
                .get("from_branch")
                .and_then(|v| v.as_str())
                .unwrap_or("HEAD");
            let sha_json = gh.request("GET", &format!("repos/{o}/{r}/git/ref/heads/{from}"), None);
            let sha = match sha_json {
                Ok(v) => v
                    .pointer("/object/sha")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                Err(_) => {
                    let repo_json = gh.request("GET", &format!("repos/{o}/{r}"), None)?;
                    repo_json
                        .get("default_branch")
                        .and_then(|x| x.as_str())
                        .unwrap_or("main")
                        .to_string()
                }
            };
            if sha.is_empty() || sha.chars().all(|c| c.is_ascii_alphabetic()) && sha.len() < 7 {
                let repo_json = gh.request("GET", &format!("repos/{o}/{r}"), None)?;
                let def = repo_json
                    .get("default_branch")
                    .and_then(|x| x.as_str())
                    .unwrap_or("main");
                let head =
                    gh.request("GET", &format!("repos/{o}/{r}/git/ref/heads/{def}"), None)?;
                let sha = head
                    .pointer("/object/sha")
                    .and_then(|x| x.as_str())
                    .ok_or_else(|| {
                        ShopError::Wait("could not resolve default branch sha".into())
                    })?;
                let body = json!({"ref": format!("refs/heads/{branch}"), "sha": sha});
                return gh.request("POST", &format!("repos/{o}/{r}/git/refs"), Some(&body));
            }
            let body = json!({"ref": format!("refs/heads/{branch}"), "sha": sha});
            gh.request("POST", &format!("repos/{o}/{r}/git/refs"), Some(&body))
        }
        "commits.list" => {
            let (o, r) = owner_repo(owner, repo)?;
            gh.request("GET", &format!("repos/{o}/{r}/commits"), None)
        }
        "commits.get" => {
            let (o, r) = owner_repo(owner, repo)?;
            let sha = args.get("sha").and_then(|v| v.as_str()).unwrap_or("");
            gh.request("GET", &format!("repos/{o}/{r}/commits/{sha}"), None)
        }
        "commits.search" => {
            let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            gh.request("GET", &format!("search/commits?q={q}"), None)
        }
        "issues.list" => {
            let (o, r) = owner_repo(owner, repo)?;
            gh.request("GET", &format!("repos/{o}/{r}/issues"), None)
        }
        "issues.read" => {
            let (o, r) = owner_repo(owner, repo)?;
            let n = args
                .get("issue_number")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            gh.request("GET", &format!("repos/{o}/{r}/issues/{n}"), None)
        }
        "issues.write" => {
            let (o, r) = owner_repo(owner, repo)?;
            let method = args
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or("create");
            if method == "update" {
                let n = args
                    .get("issue_number")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                gh.request("PATCH", &format!("repos/{o}/{r}/issues/{n}"), Some(args))
            } else {
                gh.request("POST", &format!("repos/{o}/{r}/issues"), Some(args))
            }
        }
        "issues.comment" => {
            let (o, r) = owner_repo(owner, repo)?;
            let n = args
                .get("issue_number")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            gh.request(
                "POST",
                &format!("repos/{o}/{r}/issues/{n}/comments"),
                Some(args),
            )
        }
        "issues.sub" => {
            let (o, r) = owner_repo(owner, repo)?;
            let n = args
                .get("issue_number")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            gh.request(
                "POST",
                &format!("repos/{o}/{r}/issues/{n}/sub_issues"),
                Some(args),
            )
        }
        "pulls.list" => {
            let (o, r) = owner_repo(owner, repo)?;
            gh.request("GET", &format!("repos/{o}/{r}/pulls"), None)
        }
        "pulls.read" => pull_read(gh, owner, repo, args),
        "pulls.create" => {
            let (o, r) = owner_repo(owner, repo)?;
            let mut body = args.clone();
            if body.get("draft").is_none() {
                body["draft"] = json!(true);
            }
            gh.request("POST", &format!("repos/{o}/{r}/pulls"), Some(&body))
        }
        "pulls.update" => {
            let (o, r) = owner_repo(owner, repo)?;
            let n = pull_number(args);
            gh.request("PATCH", &format!("repos/{o}/{r}/pulls/{n}"), Some(args))
        }
        "pulls.update_branch" => {
            let (o, r) = owner_repo(owner, repo)?;
            let n = pull_number(args);
            gh.request(
                "PUT",
                &format!("repos/{o}/{r}/pulls/{n}/update-branch"),
                None,
            )
        }
        "pulls.merge" => {
            let (o, r) = owner_repo(owner, repo)?;
            let n = pull_number(args);
            gh.request("PUT", &format!("repos/{o}/{r}/pulls/{n}/merge"), Some(args))
        }
        "reviews.write" => review_write(gh, owner, repo, args),
        "reviews.comment" => {
            let (o, r) = owner_repo(owner, repo)?;
            let n = pull_number(args);
            gh.request(
                "POST",
                &format!("repos/{o}/{r}/pulls/{n}/comments"),
                Some(args),
            )
        }
        "reviews.reply" => {
            let (o, r) = owner_repo(owner, repo)?;
            let n = pull_number(args);
            let cid = args.get("commentId").and_then(|v| v.as_u64()).unwrap_or(0);
            gh.request(
                "POST",
                &format!("repos/{o}/{r}/pulls/{n}/comments/{cid}/replies"),
                Some(args),
            )
        }
        "search.code" => search(gh, "code", args),
        "search.issues" => search(gh, "issues", args),
        "search.pulls" => {
            let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            gh.request("GET", &format!("search/issues?q={q}+is:pr"), None)
        }
        "search.commits" => search(gh, "commits", args),
        "search.users" => search(gh, "users", args),
        "releases.list" => {
            let (o, r) = owner_repo(owner, repo)?;
            gh.request("GET", &format!("repos/{o}/{r}/releases"), None)
        }
        "releases.get" => {
            let (o, r) = owner_repo(owner, repo)?;
            if let Some(tag) = args.get("tag").and_then(|v| v.as_str()) {
                gh.request("GET", &format!("repos/{o}/{r}/releases/tags/{tag}"), None)
            } else {
                gh.request("GET", &format!("repos/{o}/{r}/releases/latest"), None)
            }
        }
        "tags.list" => {
            let (o, r) = owner_repo(owner, repo)?;
            gh.request("GET", &format!("repos/{o}/{r}/tags"), None)
        }
        "tags.get" => {
            let (o, r) = owner_repo(owner, repo)?;
            let tag = args.get("tag").and_then(|v| v.as_str()).unwrap_or("");
            gh.request("GET", &format!("repos/{o}/{r}/git/ref/tags/{tag}"), None)
        }
        "collaborators.list" => {
            let (o, r) = owner_repo(owner, repo)?;
            gh.request("GET", &format!("repos/{o}/{r}/collaborators"), None)
        }
        "teams.list" => {
            if let Some(org) = args.get("org").and_then(|v| v.as_str()) {
                gh.request("GET", &format!("orgs/{org}/teams"), None)
            } else {
                gh.request("GET", "user/teams", None)
            }
        }
        "fork" => {
            let (o, r) = owner_repo(owner, repo)?;
            gh.request("POST", &format!("repos/{o}/{r}/forks"), None)
        }
        other => Err(ShopError::UnknownSkill(other.to_string())),
    }
}

fn pull_read(
    gh: &LiveGitHub,
    owner: Option<&str>,
    repo: Option<&str>,
    args: &Value,
) -> Result<Value> {
    let (o, r) = owner_repo(owner, repo)?;
    let n = pull_number(args);
    let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("get");
    match method {
        "get" => gh.request("GET", &format!("repos/{o}/{r}/pulls/{n}"), None),
        "get_diff" => gh.request("GET", &format!("repos/{o}/{r}/pulls/{n}"), None),
        "get_files" => gh.request("GET", &format!("repos/{o}/{r}/pulls/{n}/files"), None),
        "get_commits" => gh.request("GET", &format!("repos/{o}/{r}/pulls/{n}/commits"), None),
        "get_comments" => gh.request("GET", &format!("repos/{o}/{r}/issues/{n}/comments"), None),
        "get_reviews" => gh.request("GET", &format!("repos/{o}/{r}/pulls/{n}/reviews"), None),
        "get_review_comments" => {
            gh.request("GET", &format!("repos/{o}/{r}/pulls/{n}/comments"), None)
        }
        "get_status" | "get_check_runs" => {
            let pr = gh.request("GET", &format!("repos/{o}/{r}/pulls/{n}"), None)?;
            let sha = pr
                .pointer("/head/sha")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if method == "get_status" {
                gh.request("GET", &format!("repos/{o}/{r}/commits/{sha}/status"), None)
            } else {
                gh.request(
                    "GET",
                    &format!("repos/{o}/{r}/commits/{sha}/check-runs"),
                    None,
                )
            }
        }
        other => Err(ShopError::UnknownSkill(format!("pulls.read:{other}"))),
    }
}

fn review_write(
    gh: &LiveGitHub,
    owner: Option<&str>,
    repo: Option<&str>,
    args: &Value,
) -> Result<Value> {
    let (o, r) = owner_repo(owner, repo)?;
    let n = pull_number(args);
    let method = args
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("create");
    match method {
        "create" => gh.request(
            "POST",
            &format!("repos/{o}/{r}/pulls/{n}/reviews"),
            Some(args),
        ),
        "submit_pending" => {
            let reviews = gh.request("GET", &format!("repos/{o}/{r}/pulls/{n}/reviews"), None)?;
            Err(ShopError::Wait(format!(
                "submit_pending needs a pending review id (have {})",
                reviews
            )))
        }
        "delete_pending" => Err(ShopError::Wait(
            "delete_pending requires a pending review id".into(),
        )),
        "resolve_thread" | "unresolve_thread" => Err(ShopError::Wait(
            "thread resolve uses GraphQL; missing evidence is WAIT".into(),
        )),
        other => Err(ShopError::UnknownSkill(format!("reviews.write:{other}"))),
    }
}

fn search(gh: &LiveGitHub, kind: &str, args: &Value) -> Result<Value> {
    let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
    gh.request("GET", &format!("search/{kind}?q={q}"), None)
}

fn owner_repo<'a>(owner: Option<&'a str>, repo: Option<&'a str>) -> Result<(&'a str, &'a str)> {
    match (owner, repo) {
        (Some(o), Some(r)) if !o.is_empty() && !r.is_empty() => Ok((o, r)),
        _ => Err(ShopError::IncompleteEvidence(
            "owner and repo required".into(),
        )),
    }
}

fn pull_number(args: &Value) -> u64 {
    args.get("pullNumber")
        .or_else(|| args.get("pull_number"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}

fn read_token(store_root: &Path) -> Option<String> {
    for key in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if let Ok(v) = std::env::var(key) {
            let t = v.trim().to_string();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    let path = store_root.join("github.token");
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn gh_api(method: &str, path: &str, body: Option<&Value>, token: Option<&str>) -> Result<Value> {
    let mut cmd = Command::new("gh");
    cmd.arg("api").arg("-X").arg(method).arg(path);
    if let Some(b) = body {
        cmd.arg("--input").arg("-");
        if let Some(t) = token {
            cmd.env("GH_TOKEN", t);
            cmd.env("GITHUB_TOKEN", t);
        }
        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        use std::io::Write;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin.write_all(b.to_string().as_bytes())?;
        }
        let out = child.wait_with_output()?;
        return parse_cmd_json("gh", &out);
    }
    if let Some(t) = token {
        cmd.env("GH_TOKEN", t);
        cmd.env("GITHUB_TOKEN", t);
    }
    let out = cmd.output()?;
    parse_cmd_json("gh", &out)
}

fn curl_api(method: &str, path: &str, body: Option<&Value>, token: &str) -> Result<Value> {
    let url = format!("https://api.github.com/{path}");
    let tmp = header_file(token)?;
    let mut cmd = Command::new("curl");
    cmd.arg("-sS")
        .arg("-X")
        .arg(method)
        .arg("-H")
        .arg("Accept: application/vnd.github+json")
        .arg("-H")
        .arg("X-GitHub-Api-Version: 2022-11-28")
        .arg("-H")
        .arg("User-Agent: shop-floor")
        .arg("-H")
        .arg(format!("@{}", tmp.display()))
        .arg(&url);
    if let Some(b) = body {
        cmd.arg("-H").arg("Content-Type: application/json");
        cmd.arg("-d").arg(b.to_string());
    }
    let out = cmd.output()?;
    let _ = std::fs::remove_file(&tmp);
    parse_cmd_json("curl", &out)
}

fn header_file(token: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("shop-gh-auth-{}.txt", std::process::id()));
    // Authorization header only; file is temp and removed after the call.
    std::fs::write(&path, format!("Authorization: Bearer {token}\n"))?;
    Ok(path)
}

fn parse_cmd_json(tool: &str, out: &std::process::Output) -> Result<Value> {
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let _ = stderr;
        return Err(ShopError::Wait(format!(
            "{tool} failed (exit {:?}); missing evidence is WAIT",
            out.status.code()
        )));
    }
    if stdout.trim().is_empty() {
        return Ok(json!({"ok": true}));
    }
    serde_json::from_str(&stdout).map_err(|_| {
        ShopError::Wait(format!(
            "{tool} returned non-JSON; missing evidence is WAIT"
        ))
    })
}
