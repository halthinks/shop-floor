//! Localhost command center. No Electron. No npm. One binary.
//!
//! Suite surface for awareness + mailbox + floor. Missing pieces are WAIT.
//! An unread assign is HOLD, not PASS. A mailbox handoff is Evidence and
//! cannot CLOSE. GitHub counts stay null when the list is WAIT — never invent
//! a zero. A boss view file is Evidence of what the floor said now; a picture
//! cannot VERIFY. GrokBot is a permanent steerable node on the work graph.
//! A missing `grokbot-role.json` is role=orchestrator (WAIT-safe), never an
//! error PASS, never VERIFIED. This file never writes `C:\TextPCB Platform`.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;

use serde_json::{json, Value};

use crate::engine::Shop;
use crate::error::{Result, ShopError};
use crate::project::{load_recents, recent_path_for_repo};

pub const DEFAULT_PORT: u16 = 7745;
pub const PAGE: &str = include_str!("ui.html");
pub const CSS: &str = include_str!("ui.css");

pub fn bind_local(port: u16) -> std::io::Result<TcpListener> {
    match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => Ok(l),
        Err(_) if port != 0 => TcpListener::bind(("127.0.0.1", 0)),
        Err(e) => Err(e),
    }
}

pub fn run(shop: Shop, port: u16) -> Result<()> {
    let listener = bind_local(port)?;
    let addr = listener.local_addr()?;
    println!("shop ui http://127.0.0.1:{}/", addr.port());
    serve_listener(listener, shop)
}

pub fn spawn(
    shop: Shop,
    port: u16,
) -> Result<(std::net::SocketAddr, thread::JoinHandle<Result<()>>)> {
    let listener = bind_local(port)?;
    let addr = listener.local_addr()?;
    let handle = thread::spawn(move || serve_listener(listener, shop));
    Ok((addr, handle))
}

pub fn serve_listener(listener: TcpListener, shop: Shop) -> Result<()> {
    let app = Mutex::new(shop);
    listener.set_nonblocking(false)?;
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let _ = handle_client(s, &app);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn handle_client(mut stream: TcpStream, app: &Mutex<Shop>) -> Result<()> {
    let mut buf = vec![0u8; 64 * 1024];
    let n = stream.read(&mut buf)?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let (method, path, body) = parse_request(&req);
    let (status, ctype, payload) = route(app, &method, &path, &body);
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    stream.write_all(resp.as_bytes())?;
    stream.write_all(&payload)?;
    Ok(())
}

fn parse_request(req: &str) -> (String, String, String) {
    let mut lines = req.split("\r\n");
    let start = lines.next().unwrap_or("");
    let mut parts = start.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let mut content_len = 0usize;
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            content_len = v.trim().parse().unwrap_or(0);
        }
    }
    let rest = lines.collect::<Vec<_>>().join("\r\n");
    let body = if content_len > 0 {
        rest.chars().take(content_len).collect()
    } else {
        rest
    };
    (method, path, body)
}

fn split_query(path: &str) -> (&str, Vec<(String, String)>) {
    match path.split_once('?') {
        Some((p, q)) => {
            let pairs = q
                .split('&')
                .filter_map(|part| {
                    let (k, v) = part.split_once('=')?;
                    Some((url_decode(k), url_decode(v)))
                })
                .collect();
            (p, pairs)
        }
        None => (path, Vec::new()),
    }
}

fn qget(q: &[(String, String)], key: &str) -> Option<String> {
    q.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

fn route(app: &Mutex<Shop>, method: &str, path: &str, body: &str) -> (String, String, Vec<u8>) {
    let (path, query) = split_query(path);
    match (method, path) {
        ("GET", "/") => (
            "200 OK".into(),
            "text/html; charset=utf-8".into(),
            PAGE.as_bytes().to_vec(),
        ),
        ("GET", "/ui.css") => (
            "200 OK".into(),
            "text/css; charset=utf-8".into(),
            CSS.as_bytes().to_vec(),
        ),
        ("GET", "/feed.xml") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            (
                "200 OK".into(),
                "application/atom+xml; charset=utf-8".into(),
                shop.feed_atom().into_bytes(),
            )
        }
        ("GET", "/rss.xml") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            (
                "200 OK".into(),
                "application/rss+xml; charset=utf-8".into(),
                shop.feed_rss().into_bytes(),
            )
        }
        ("GET", "/api/status") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            match shop.awareness(None) {
                Ok(a) => json_ok(serde_json::to_value(a).unwrap_or(json!({}))),
                Err(e) => json_err(&e),
            }
        }
        ("GET", "/api/workers") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            json_ok(workers_json(&shop))
        }
        ("GET", "/api/github") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            json_ok(github_module(&shop, qget(&query, "pull")))
        }
        ("GET", "/api/github/search") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            json_ok(github_search(
                &shop,
                qget(&query, "q").unwrap_or_default(),
                qget(&query, "kind").unwrap_or_else(|| "code".into()),
            ))
        }
        ("GET", "/api/github/file") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            json_ok(github_file(&shop, qget(&query, "path").unwrap_or_default()))
        }
        ("GET", "/api/steer") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            let _ = shop.ingest_replies();
            let mail = shop.observe_mailbox();
            let status = if mail.present && mail.wait.is_none() {
                "Live"
            } else {
                "Waiting"
            };
            json_ok(json!({
                "lines": shop.steer_transcript(),
                "peer": crate::mailbox::STEER_TO,
                "status": status,
            }))
        }
        ("GET", "/api/memory") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            json_ok(serde_json::to_value(shop.memory()).unwrap_or(json!({})))
        }
        ("GET", "/api/floor") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            json_ok(floor_payload(&shop))
        }
        ("GET", "/api/mailbox") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            json_ok(mailbox_payload(&shop))
        }
        ("GET", "/api/boss") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            json_ok(boss_payload(&shop))
        }
        ("GET", "/api/jobs") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            json_ok(jobs_payload(&shop))
        }
        ("GET", "/api/events") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            json_ok(json!({"events": shop.events()}))
        }
        ("GET", "/api/graph") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            json_ok(graph_payload(&shop))
        }
        ("GET", "/api/grokbot") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            json_ok(grokbot_payload(&shop))
        }
        ("POST", "/api/grokbot") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            match post_grokbot(&shop, body) {
                Ok(v) => json_ok(v),
                Err(e) => json_err(&e),
            }
        }
        ("GET", "/api/project") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            json_ok(project_json(&shop))
        }
        ("GET", "/api/projects") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            json_ok(projects_picker(&shop))
        }
        ("POST", "/workers") | ("POST", "/api/workers") => {
            let mut shop = app.lock().unwrap_or_else(|e| e.into_inner());
            match add_worker(&mut shop, body) {
                Ok(_) if path == "/workers" => redirect("/"),
                Ok(v) => json_ok(v),
                Err(e) => json_err(&e),
            }
        }
        (m, p) if m == "POST" && p.starts_with("/workers/") && p.ends_with("/intelligence") => {
            let peer = p
                .trim_start_matches("/workers/")
                .trim_end_matches("/intelligence");
            let mut shop = app.lock().unwrap_or_else(|e| e.into_inner());
            match set_intel(&mut shop, peer, body) {
                Ok(v) => json_ok(v),
                Err(e) => json_err(&e),
            }
        }
        (m, p) if m == "POST" && p.starts_with("/workers/") && p.ends_with("/github-skills") => {
            let peer = p
                .trim_start_matches("/workers/")
                .trim_end_matches("/github-skills");
            let mut shop = app.lock().unwrap_or_else(|e| e.into_inner());
            match set_skills(&mut shop, peer, body) {
                Ok(v) => json_ok(v),
                Err(e) => json_err(&e),
            }
        }
        ("POST", "/github/connect") | ("POST", "/api/github/connect") => {
            let mut shop = app.lock().unwrap_or_else(|e| e.into_inner());
            match connect(&mut shop, body) {
                Ok(_) if path == "/github/connect" => redirect("/"),
                Ok(v) => json_ok(v),
                Err(e) => json_err(&e),
            }
        }
        ("POST", "/github/merge") | ("POST", "/api/github/merge") => {
            let mut shop = app.lock().unwrap_or_else(|e| e.into_inner());
            match merge(&mut shop, body) {
                Ok(v) => json_ok(v),
                Err(e) => json_err(&e),
            }
        }
        ("POST", "/api/steer") => {
            let mut shop = app.lock().unwrap_or_else(|e| e.into_inner());
            let f = form(body);
            let text = f.get("text").cloned().unwrap_or_default();
            match shop.steer(&text) {
                Ok(v) => json_ok(v),
                Err(e) => json_err(&e),
            }
        }
        ("POST", "/api/memory") => {
            let mut shop = app.lock().unwrap_or_else(|e| e.into_inner());
            match memory_action(&mut shop, body) {
                Ok(v) => json_ok(v),
                Err(e) => json_err(&e),
            }
        }
        ("POST", "/api/project/open") => {
            let mut shop = app.lock().unwrap_or_else(|e| e.into_inner());
            match open_project(&mut shop, body) {
                Ok(v) => json_ok(v),
                Err(e) => json_err(&e),
            }
        }
        _ => (
            "404 Not Found".into(),
            "text/plain".into(),
            b"not found".to_vec(),
        ),
    }
}

const GROKBOT_PEER: &str = "grok-bot";
const GROKBOT_LABEL: &str = "GrokBot";
const GROKBOT_FILE: &str = "grokbot-role.json";
const GROKBOT_DEFAULT_ROLE: &str = "orchestrator";
const GROKBOT_ROLES: &[&str] = &[
    "orchestrator",
    "captain",
    "research",
    "impl",
    "builder",
    "qa",
    "lander",
];
const GROKBOT_STEER_CAP: usize = 50;

#[derive(Clone)]
struct GrokBotState {
    role: String,
    steers: Vec<Value>,
    last_steer: Option<String>,
    wait: Option<String>,
}

fn grokbot_roles_json() -> Value {
    json!(GROKBOT_ROLES)
}

fn normalize_grokbot_role(raw: &str) -> Option<&'static str> {
    let t = raw.trim();
    if t.eq_ignore_ascii_case("verified") {
        return None;
    }
    GROKBOT_ROLES.iter().copied().find(|r| *r == t)
}

fn default_grokbot() -> GrokBotState {
    GrokBotState {
        role: GROKBOT_DEFAULT_ROLE.to_string(),
        steers: Vec::new(),
        last_steer: None,
        wait: None,
    }
}

fn grokbot_path(shop: &Shop) -> PathBuf {
    shop.store_root().join(GROKBOT_FILE)
}

fn load_grokbot(shop: &Shop) -> GrokBotState {
    let path = grokbot_path(shop);
    if !path.is_file() {
        // Missing file is orchestrator. Not an error. Not PASS. Not VERIFIED.
        return default_grokbot();
    }
    match fs::read_to_string(&path) {
        Ok(raw) => match serde_json::from_str::<Value>(&raw) {
            Ok(v) if v.is_object() => parse_grokbot_file(&v),
            Ok(_) => {
                let mut s = default_grokbot();
                s.wait = Some("WAIT: grokbot-role.json is not an object".into());
                s
            }
            Err(e) => {
                let mut s = default_grokbot();
                s.wait = Some(format!("WAIT: grokbot-role.json unreadable: {e}"));
                s
            }
        },
        Err(e) => {
            let mut s = default_grokbot();
            s.wait = Some(format!("WAIT: grokbot-role.json unreadable: {e}"));
            s
        }
    }
}

fn parse_grokbot_file(v: &Value) -> GrokBotState {
    let role = v
        .get("role")
        .and_then(|x| x.as_str())
        .and_then(normalize_grokbot_role)
        .unwrap_or(GROKBOT_DEFAULT_ROLE)
        .to_string();
    let steers = v
        .get("steers")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let last = v
        .get("steer")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            steers.last().and_then(|s| {
                s.get("text")
                    .and_then(|t| t.as_str())
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(ToString::to_string)
            })
        });
    GrokBotState {
        role,
        steers,
        last_steer: last,
        wait: None,
    }
}

fn save_grokbot(shop: &Shop, state: &GrokBotState) -> Result<()> {
    let path = grokbot_path(shop);
    crate::store::ensure_under(&path, &[shop.store_root().to_path_buf()])?;
    let v = json!({
        "peer": GROKBOT_PEER,
        "role": state.role,
        "steer": state.last_steer,
        "steers": state.steers,
    });
    crate::store::write_json_atomic(&path, &v)
}

fn grokbot_json(state: &GrokBotState) -> Value {
    let mut v = json!({
        "peer": GROKBOT_PEER,
        "role": state.role,
        "roles": grokbot_roles_json(),
        "steers": state.steers,
    });
    if let Some(obj) = v.as_object_mut() {
        if let Some(w) = &state.wait {
            obj.insert("wait".into(), json!(w));
        }
        obj.remove("VERIFIED");
        obj.remove("verified");
    }
    v
}

fn grokbot_payload(shop: &Shop) -> Value {
    grokbot_json(&load_grokbot(shop))
}

fn post_grokbot(shop: &Shop, body: &str) -> Result<Value> {
    let f = form(body);
    let mut state = load_grokbot(shop);
    state.wait = None;
    let mut changed = false;
    if let Some(role) = f.get("role") {
        let Some(ok) = normalize_grokbot_role(role) else {
            return Err(ShopError::IncompleteEvidence(format!(
                "unknown grokbot role '{role}'"
            )));
        };
        state.role = ok.to_string();
        changed = true;
    }
    if let Some(steer) = f.get("steer") {
        let text = steer.trim();
        if text.is_empty() {
            return Err(ShopError::IncompleteEvidence("empty steer".into()));
        }
        state.last_steer = Some(text.to_string());
        state.steers.push(json!({
            "text": text,
            "at": crate::hash::format_rfc3339(crate::hash::unix_now()),
            "role": state.role,
        }));
        if state.steers.len() > GROKBOT_STEER_CAP {
            let drop = state.steers.len() - GROKBOT_STEER_CAP;
            state.steers.drain(0..drop);
        }
        changed = true;
    }
    if !changed {
        return Err(ShopError::IncompleteEvidence(
            "POST /api/grokbot needs role or steer".into(),
        ));
    }
    save_grokbot(shop, &state)?;
    Ok(grokbot_json(&state))
}

/// Work-graph steer target: SuperGrok (`STEER_TO`). kimi-code is demoted —
/// if that peer is still on the roster it is a worker, not captain.
fn steer_target_id(_shop: &Shop) -> String {
    crate::mailbox::STEER_TO.into()
}

fn graph_payload(shop: &Shop) -> Value {
    let gb = load_grokbot(shop);
    let target = steer_target_id(shop);
    let roster = shop.workers().ok();
    let workers = roster.as_ref().map(|r| r.workers.as_slice()).unwrap_or(&[]);
    let target_label = workers
        .iter()
        .find(|w| w.peer == target)
        .map(|w| w.name.clone())
        .unwrap_or_else(|| "SuperGrokHeavy".into());
    let mut nodes = vec![
        json!({
            "id": GROKBOT_PEER,
            "label": GROKBOT_LABEL,
            "role": gb.role,
            "steerable": true,
            "kind": "grokbot",
        }),
        json!({
            "id": target,
            "label": target_label,
            "kind": "peer",
            "steerable": false,
        }),
    ];
    for w in workers {
        if w.peer == GROKBOT_PEER || w.peer == target {
            continue;
        }
        nodes.push(json!({
            "id": w.peer,
            "label": w.name,
            "kind": "worker",
            "backend": w.intelligence.backend.as_str(),
            "steerable": false,
        }));
    }
    if let Ok(jobs) = local_live_jobs(shop) {
        for job in jobs {
            let id = format!("job:{}:{}", job.parent_id, job.child_id);
            nodes.push(json!({
                "id": id,
                "label": format!("{}/{}", job.parent_id, job.child_id),
                "kind": "job",
                "peer": job.peer,
                "action": job.action,
                "steerable": false,
            }));
        }
    }
    let mut graph = json!({
        "nodes": nodes,
        "edges": [{
            "from": GROKBOT_PEER,
            "to": target,
            "kind": "steer",
        }],
    });
    if let Some(w) = gb.wait {
        if let Some(obj) = graph.as_object_mut() {
            obj.insert("wait".into(), json!(w));
        }
    }
    graph
}

fn workers_json(shop: &Shop) -> Value {
    match shop.workers() {
        Ok(r) => serde_json::to_value(r).unwrap_or(json!({"workers": []})),
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn project_json(shop: &Shop) -> Value {
    json!({
        "name": crate::project::project_name(shop.project_root()),
        "path": shop.project_root().display().to_string(),
        "store": shop.store_root().display().to_string(),
        "recents": load_recents(shop.recents_path()).projects,
    })
}

fn projects_picker(shop: &Shop) -> Value {
    json!({
        "local_recents": load_recents(shop.recents_path()).projects,
        "github": shop.github_repo_catalog(),
        "authenticated": shop.github_skills().authenticated(),
        "current": {
            "name": crate::project::project_name(shop.project_root()),
            "path": shop.project_root().display().to_string(),
            "repo": shop.github_repo().ok().flatten().map(|r| r.slug()),
        }
    })
}

fn list_or_wait(v: Result<Value>) -> Value {
    match v {
        Ok(val) => val,
        Err(e) => json!({"WAIT": e.to_string()}),
    }
}

fn github_module(shop: &Shop, pull: Option<String>) -> Value {
    let repo = shop.github_repo().ok().flatten();
    let authed = shop.github_skills().authenticated();
    let public_repo = repo
        .as_ref()
        .map(|r| crate::skills::is_recorded_public(&r.slug()))
        .unwrap_or(false);
    let wait = if repo.is_none() {
        Some("WAIT: GitHub repo not connected")
    } else if !authed && !public_repo {
        Some("WAIT: private repo needs gh or token")
    } else {
        None
    };
    let mut issues = json!("WAIT");
    let mut pulls = json!("WAIT");
    let mut checks = json!("WAIT");
    let mut reviews = json!("WAIT");
    let mut comments = json!("WAIT");
    let mut selected = pull.clone();
    if let Some(r) = &repo {
        if authed || public_repo {
            issues = list_or_wait(
                shop.github_skills()
                    .invoke("issues.list", json!({"owner": r.owner, "repo": r.name})),
            );
            pulls = list_or_wait(
                shop.github_skills()
                    .invoke("pulls.list", json!({"owner": r.owner, "repo": r.name})),
            );
            if selected.is_none() {
                if let Ok(state) = shop.state() {
                    selected = state
                        .parents
                        .values()
                        .find_map(|p| p.github_pr_number.map(|n| n.to_string()));
                }
            }
            if let Some(n) = selected.clone() {
                if let Ok(num) = n.parse::<u64>() {
                    checks = list_or_wait(shop.github_skills().invoke(
                        "pulls.read",
                        json!({
                            "owner": r.owner,
                            "repo": r.name,
                            "pullNumber": num,
                            "method": "get_check_runs",
                        }),
                    ));
                    reviews = list_or_wait(shop.github_skills().invoke(
                        "pulls.read",
                        json!({
                            "owner": r.owner,
                            "repo": r.name,
                            "pullNumber": num,
                            "method": "get_reviews",
                        }),
                    ));
                    comments = list_or_wait(shop.github_skills().invoke(
                        "pulls.read",
                        json!({
                            "owner": r.owner,
                            "repo": r.name,
                            "pullNumber": num,
                            "method": "get_comments",
                        }),
                    ));
                }
            } else {
                checks = json!("WAIT: no PR selected");
                reviews = json!("WAIT: no PR selected");
                comments = json!("WAIT: no PR selected");
            }
        }
    }
    let verified = shop.state().ok().and_then(|s| {
        s.parents
            .values()
            .find(|p| p.state == crate::types::ParentState::Verified)
            .map(|p| p.id.clone())
    });
    json!({
        "connected": repo.is_some(),
        "repo": repo.as_ref().map(|r| r.slug()),
        "owner": repo.as_ref().map(|r| r.owner.clone()),
        "name": repo.as_ref().map(|r| r.name.clone()),
        "authenticated": authed,
        "wait": wait,
        "issues": issues,
        "pulls": pulls,
        "checks": checks,
        "reviews": reviews,
        "comments": comments,
        "selected_pull": selected,
        "merge_parent": verified,
        "merge_enabled": verified.is_some(),
        "issue_count": list_count(&issues),
        "pr_count": list_count(&pulls),
        "check_count": list_count(&checks),
        "count_note": "null count is WAIT; one API page is not the total; never invent a zero",
        "skills": crate::skills::SKILL_PACK,
    })
}

fn list_count(v: &Value) -> Value {
    // One GitHub API page is not the repository total. Only an
    // authoritative total_count is a count. Missing total is WAIT.
    if let Some(n) = authoritative_total(v) {
        return json!(n);
    }
    json!(null)
}

fn authoritative_total(v: &Value) -> Option<u64> {
    if v.as_str().is_some() {
        return None;
    }
    if v.get("WAIT").is_some() || v.get("error").is_some() {
        return None;
    }
    v.get("total_count")
        .and_then(|x| x.as_u64())
        .or_else(|| v.get("total").and_then(|x| x.as_u64()))
}

fn floor_payload(shop: &Shop) -> Value {
    let mut v = serde_json::to_value(shop.floor()).unwrap_or(json!({}));
    if let Some(obj) = v.as_object_mut() {
        if obj.get("current").map(|c| c.is_null()).unwrap_or(true) {
            obj.insert("wait".into(), json!("WAIT: no job open"));
        }
        obj.insert(
            "evidence_note".into(),
            json!("floor memory is Evidence; not VERIFIED; not CLOSE"),
        );
        match jobs_payload(shop) {
            jobs if jobs.get("WAIT").is_some() => {
                obj.insert("jobs".into(), jobs);
            }
            jobs => {
                obj.insert(
                    "jobs".into(),
                    jobs.get("jobs").cloned().unwrap_or(json!("WAIT")),
                );
            }
        }
    }
    v
}

fn jobs_payload(shop: &Shop) -> Value {
    match local_live_jobs(shop) {
        Ok(jobs) => json!({
            "jobs": jobs,
            "count": jobs.len() as u64,
            "working": crate::awareness::observed_working_jobs(&jobs) as u64,
            "count_note": "count is observed length; missing pid is WAIT; never invent working",
        }),
        Err(e) => json!({
            "WAIT": e.to_string(),
            "jobs": "WAIT",
            "count": null,
            "working": null,
            "count_note": "null count is WAIT; never invent a zero",
        }),
    }
}

fn local_live_jobs(shop: &Shop) -> Result<Vec<crate::awareness::JobView>> {
    let state = shop.state()?;
    let ledger = shop.procwait()?;
    let parents: Vec<crate::awareness::ParentView> = state
        .parents
        .values()
        .map(|p| crate::awareness::ParentView {
            id: p.id.clone(),
            title: p.title.clone(),
            state: p.state,
            children: p
                .children
                .values()
                .map(|c| crate::awareness::ChildView {
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
    Ok(crate::awareness::live_jobs(&parents, &ledger.processes))
}

fn mailbox_payload(shop: &Shop) -> Value {
    let observation = shop.observe_mailbox();
    let hold_in_flight = observation.hold_in_flight();
    json!({
        "observation": observation,
        "hold_in_flight": hold_in_flight,
        "one_assign_in_flight": crate::mailbox::ONE_ASSIGN_IN_FLIGHT,
        "handoff_is_evidence": crate::mailbox::HANDOFF_IS_EVIDENCE,
        "hold_unread": crate::mailbox::HOLD_UNREAD,
        "fifth_product": crate::mailbox::FIFTH_PRODUCT,
    })
}

fn boss_payload(shop: &Shop) -> Value {
    let path = shop.store_root().join("boss").join("view.json");
    if !path.is_file() {
        return json!({
            "WAIT": "boss view not written",
            "kind": "live_floor_view",
            "status": "WAIT",
            "class": "INFORMATION_GAP",
            "note": "a picture is Evidence; cannot VERIFY or CLOSE",
        });
    }
    match fs::read_to_string(&path) {
        Ok(raw) => match serde_json::from_str::<Value>(&raw) {
            Ok(mut v) if v.is_object() => {
                if let Some(obj) = v.as_object_mut() {
                    if obj.get("WAIT").is_none() {
                        if let Some(w) = obj.get("wait").cloned() {
                            if !w.is_null() {
                                obj.insert("WAIT".into(), w);
                            }
                        }
                    }
                }
                v
            }
            Ok(_) => json!({"WAIT": "boss view is not an object"}),
            Err(e) => json!({"WAIT": format!("boss view unreadable: {e}")}),
        },
        Err(e) => json!({"WAIT": format!("boss view unreadable: {e}")}),
    }
}

fn github_search(shop: &Shop, q: String, kind: String) -> Value {
    let Some(r) = shop.github_repo().ok().flatten() else {
        return json!({"WAIT": "GitHub repo not connected"});
    };
    let public_repo = crate::skills::is_recorded_public(&r.slug());
    if !shop.github_skills().authenticated() && !public_repo {
        return json!({"WAIT": "private repo needs gh or token"});
    }
    if q.trim().is_empty() {
        return json!({"WAIT": "empty search"});
    }
    let skill = match kind.as_str() {
        "issues" => "search.issues",
        "pulls" => "search.pulls",
        _ => "search.code",
    };
    let query = if kind == "code" {
        format!("repo:{}/{} {}", r.owner, r.name, q)
    } else {
        q
    };
    list_or_wait(shop.github_skills().invoke(skill, json!({"query": query})))
}

fn github_file(shop: &Shop, path: String) -> Value {
    let Some(r) = shop.github_repo().ok().flatten() else {
        return json!({"WAIT": "GitHub repo not connected"});
    };
    let public_repo = crate::skills::is_recorded_public(&r.slug());
    if !shop.github_skills().authenticated() && !public_repo {
        return json!({"WAIT": "private repo needs gh or token"});
    }
    list_or_wait(shop.github_skills().invoke(
        "files.get",
        json!({"owner": r.owner, "repo": r.name, "path": path}),
    ))
}

fn add_worker(shop: &mut Shop, body: &str) -> Result<Value> {
    let f = form(body);
    let peer = f
        .get("peer")
        .cloned()
        .filter(|s| !s.is_empty())
        .or_else(|| f.get("name").cloned())
        .unwrap_or_default();
    let name = f.get("name").cloned().unwrap_or_else(|| peer.clone());
    let backend = f.get("backend").cloned().unwrap_or_default();
    let model = f.get("model").cloned().unwrap_or_default();
    let skills = f
        .get("github_skills")
        .map(|s| s != "0" && s != "false")
        .unwrap_or(true);
    let w = shop.add_worker(&peer, &name, &backend, &model, skills)?;
    Ok(serde_json::to_value(w)?)
}

fn set_intel(shop: &mut Shop, peer: &str, body: &str) -> Result<Value> {
    let f = form(body);
    let backend = f.get("backend").cloned().unwrap_or_default();
    let model = f.get("model").cloned().unwrap_or_default();
    let w = shop.set_worker_intelligence(peer, &backend, &model)?;
    Ok(serde_json::to_value(w)?)
}

fn set_skills(shop: &mut Shop, peer: &str, body: &str) -> Result<Value> {
    let f = form(body);
    let enabled = f
        .get("enabled")
        .map(|s| s == "1" || s == "true")
        .unwrap_or(false);
    let w = shop.set_worker_github_skills(peer, enabled)?;
    Ok(serde_json::to_value(w)?)
}

fn connect(shop: &mut Shop, body: &str) -> Result<Value> {
    let f = form(body);
    let owner = f.get("owner").cloned().unwrap_or_default();
    let name = f.get("name").cloned().unwrap_or_default();
    let branch = f.get("branch").cloned();
    let token = f.get("token").cloned();
    let spec = if owner.is_empty() {
        f.get("repo").cloned().unwrap_or_default()
    } else {
        format!("{owner}/{name}")
    };
    let repo = shop.github_connect(&spec, branch.as_deref(), token.as_deref())?;
    Ok(json!({"repo": repo.slug()}))
}

fn merge(shop: &mut Shop, body: &str) -> Result<Value> {
    let f = form(body);
    let parent = f
        .get("parent")
        .cloned()
        .ok_or_else(|| ShopError::IncompleteEvidence("parent required".into()))?;
    shop.merge_verified(&parent)
}

fn memory_action(shop: &mut Shop, body: &str) -> Result<Value> {
    let f = form(body);
    let action = f.get("action").map(String::as_str).unwrap_or("");
    let fact = f.get("fact").cloned().unwrap_or_default();
    match action {
        "remember" => {
            let tier = crate::memory::MemoryTier::parse(
                f.get("tier").map(String::as_str).unwrap_or("profile"),
            )?;
            let mem = shop.remember(tier, &fact)?;
            Ok(serde_json::to_value(mem)?)
        }
        "forget" => {
            let mem = shop.forget(&fact)?;
            Ok(serde_json::to_value(mem)?)
        }
        _ => Err(ShopError::IncompleteEvidence(
            "action must be remember or forget".into(),
        )),
    }
}

fn open_project(shop: &mut Shop, body: &str) -> Result<Value> {
    let f = form(body);
    let kind = f.get("kind").map(String::as_str).unwrap_or("");
    if kind == "github" || f.get("repo").is_some() && kind != "local" {
        let spec = f
            .get("repo")
            .cloned()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ShopError::IncompleteEvidence("repo required".into()))?;
        return open_github_project(shop, spec);
    }
    let path = f
        .get("path")
        .cloned()
        .ok_or_else(|| ShopError::IncompleteEvidence("path required".into()))?;
    let gh = shop.github_arc();
    let from = shop.from_name().to_string();
    let recents = shop.recents_path().to_path_buf();
    let mailbox = shop.mailbox_root().map(PathBuf::from);
    let mut next = Shop::open_project_with_recents(&path, &recents)?
        .with_github(gh)
        .with_from(from)
        .with_recents_path(recents);
    if let Some(mb) = mailbox {
        next = next.with_mailbox(Some(mb));
    }
    let shown = next.project_root().display().to_string();
    let repo = next.github_repo().ok().flatten().map(|r| r.slug());
    *shop = next;
    Ok(json!({"kind": "local", "path": shown, "repo": repo}))
}

fn open_github_project(shop: &mut Shop, spec: String) -> Result<Value> {
    shop.github_connect(&spec, None, None)?;
    if let Some(known) = recent_path_for_repo(shop.recents_path(), &spec) {
        let gh = shop.github_arc();
        let from = shop.from_name().to_string();
        let recents = shop.recents_path().to_path_buf();
        let mut next = Shop::open_project_with_recents(&known, &recents)?
            .with_github(gh)
            .with_from(from)
            .with_recents_path(recents);
        next.github_connect(&spec, None, None)?;
        *shop = next;
    }
    Ok(json!({
        "kind": "github",
        "path": shop.project_root().display().to_string(),
        "repo": spec,
    }))
}

fn form(body: &str) -> std::collections::BTreeMap<String, String> {
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        if let Some(obj) = v.as_object() {
            return obj
                .iter()
                .filter_map(|(k, val)| {
                    Some((
                        k.clone(),
                        val.as_str()
                            .map(ToString::to_string)
                            .or_else(|| val.as_bool().map(|b| b.to_string()))
                            .or_else(|| val.as_i64().map(|n| n.to_string()))?,
                    ))
                })
                .collect();
        }
    }
    let mut out = std::collections::BTreeMap::new();
    for part in body.split('&') {
        if let Some((k, v)) = part.split_once('=') {
            out.insert(url_decode(k), url_decode(v));
        }
    }
    out
}

fn url_decode(s: &str) -> String {
    let s = s.replace('+', " ");
    let mut out = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(c) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(c as char);
                i += 3;
                continue;
            }
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

fn json_ok(v: Value) -> (String, String, Vec<u8>) {
    (
        "200 OK".into(),
        "application/json".into(),
        serde_json::to_vec(&v).unwrap_or_else(|_| b"{}".to_vec()),
    )
}

fn json_err(e: &ShopError) -> (String, String, Vec<u8>) {
    (
        "400 Bad Request".into(),
        "application/json".into(),
        serde_json::to_vec(&json!({"error": e.to_string()})).unwrap_or_default(),
    )
}

fn redirect(to: &str) -> (String, String, Vec<u8>) {
    let body = format!("<a href=\"{to}\">redirect</a>");
    (
        format!("303 See Other\r\nLocation: {to}"),
        "text/html".into(),
        body.into_bytes(),
    )
}

pub fn ensure_store(dir: PathBuf) -> Result<Shop> {
    match Shop::open_store(&dir) {
        Ok(s) => Ok(s),
        Err(_) => Shop::init(dir),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn tmp_store() -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!("shop-ui-grokbot-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn http(addr: &str, method: &str, path: &str, body: &str) -> String {
        let mut last = String::new();
        for _ in 0..40 {
            if let Ok(mut s) = TcpStream::connect(addr) {
                let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
                let mut req =
                    format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
                if !body.is_empty() {
                    req.push_str("Content-Type: application/json\r\n");
                    req.push_str(&format!("Content-Length: {}\r\n", body.len()));
                }
                req.push_str("\r\n");
                req.push_str(body);
                if s.write_all(req.as_bytes()).is_ok() {
                    let mut buf = Vec::new();
                    let _ = s.read_to_end(&mut buf);
                    last = String::from_utf8_lossy(&buf).into_owned();
                    if last.starts_with("HTTP/1.1 200") || last.starts_with("HTTP/1.1 400") {
                        return last;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        last
    }

    fn json_body(resp: &str) -> Value {
        let body = resp.split("\r\n\r\n").nth(1).unwrap_or(resp);
        serde_json::from_str(body).unwrap_or_else(|_| json!({"raw": body}))
    }

    fn spawn_shop(shop: Shop) -> String {
        let (addr, _h) = spawn(shop, 0).unwrap();
        std::mem::forget(_h);
        format!("{}:{}", addr.ip(), addr.port())
    }

    #[test]
    fn missing_role_file_is_orchestrator_not_error() {
        let dir = tmp_store();
        let shop = Shop::init(&dir).unwrap();
        assert!(!grokbot_path(&shop).is_file());
        let host = spawn_shop(shop);
        let resp = http(&host, "GET", "/api/grokbot", "");
        assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
        let v = json_body(&resp);
        assert_eq!(v["peer"], GROKBOT_PEER);
        assert_eq!(v["role"], GROKBOT_DEFAULT_ROLE);
        assert!(v.get("error").is_none(), "{v}");
        assert_ne!(v["role"], "VERIFIED");
        let roles = v["roles"].as_array().expect("roles");
        for want in GROKBOT_ROLES {
            assert!(
                roles.iter().any(|r| r == *want),
                "missing role {want} in {roles:?}"
            );
        }
        assert_eq!(v["steers"].as_array().map(|a| a.len()).unwrap_or(99), 0);
    }

    #[test]
    fn post_role_and_steer_persist_on_disk() {
        let dir = tmp_store();
        let shop = Shop::init(&dir).unwrap();
        let store = shop.store_root().to_path_buf();
        let host = spawn_shop(shop);
        let role_resp = http(&host, "POST", "/api/grokbot", r#"{"role":"research"}"#);
        assert!(role_resp.starts_with("HTTP/1.1 200"), "{role_resp}");
        let role_v = json_body(&role_resp);
        assert_eq!(role_v["role"], "research");

        let steer_resp = http(
            &host,
            "POST",
            "/api/grokbot",
            r#"{"steer":"look at the floor"}"#,
        );
        assert!(steer_resp.starts_with("HTTP/1.1 200"), "{steer_resp}");
        let steer_v = json_body(&steer_resp);
        assert_eq!(steer_v["role"], "research");
        let steers = steer_v["steers"].as_array().expect("steers");
        assert_eq!(steers.len(), 1);
        assert_eq!(steers[0]["text"], "look at the floor");

        let disk = fs::read_to_string(store.join(GROKBOT_FILE)).unwrap();
        let saved: Value = serde_json::from_str(&disk).unwrap();
        assert_eq!(saved["role"], "research");
        assert_eq!(saved["steer"], "look at the floor");
        assert_ne!(saved["role"], "VERIFIED");

        let get = json_body(&http(&host, "GET", "/api/grokbot", ""));
        assert_eq!(get["role"], "research");
        assert_eq!(get["steers"][0]["text"], "look at the floor");
    }

    #[test]
    fn graph_includes_steerable_grokbot_and_steer_edge() {
        let dir = tmp_store();
        let shop = Shop::init(&dir).unwrap();
        let host = spawn_shop(shop);
        let resp = http(&host, "GET", "/api/graph", "");
        assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
        let v = json_body(&resp);
        let nodes = v["nodes"].as_array().expect("nodes");
        let grok = nodes
            .iter()
            .find(|n| n["id"] == GROKBOT_PEER)
            .expect("grok-bot node");
        assert_eq!(grok["label"], GROKBOT_LABEL);
        assert_eq!(grok["role"], GROKBOT_DEFAULT_ROLE);
        assert_eq!(grok["steerable"], true);
        let edges = v["edges"].as_array().expect("edges");
        assert!(
            edges.iter().any(|e| {
                e["from"] == GROKBOT_PEER
                    && e["to"] == crate::mailbox::STEER_TO
                    && e["kind"] == "steer"
            }),
            "missing grok-bot -> SuperGrok steer edge: {edges:?}"
        );
        assert!(nodes.iter().any(|n| n["id"] == crate::mailbox::STEER_TO));
        assert!(!nodes.iter().any(|n| n["id"] == "kimi-code"));
    }

    #[test]
    fn graph_steers_supergrok_even_when_kimi_code_exists() {
        let dir = tmp_store();
        let mut shop = Shop::init(&dir).unwrap();
        shop.add_worker("kimi-code", "Kimi", "cursor", "", true)
            .unwrap();
        let host = spawn_shop(shop);
        let v = json_body(&http(&host, "GET", "/api/graph", ""));
        let edges = v["edges"].as_array().expect("edges");
        assert!(
            edges.iter().any(|e| {
                e["from"] == GROKBOT_PEER
                    && e["to"] == crate::mailbox::STEER_TO
                    && e["kind"] == "steer"
            }),
            "missing grok-bot -> SuperGrok steer edge: {edges:?}"
        );
        assert!(!edges.iter().any(|e| {
            e["from"] == GROKBOT_PEER && e["to"] == "kimi-code" && e["kind"] == "steer"
        }));
        let nodes = v["nodes"].as_array().unwrap();
        assert!(nodes.iter().any(|n| n["id"] == crate::mailbox::STEER_TO));
        assert!(nodes.iter().any(|n| n["id"] == "kimi-code"));
        assert!(!nodes.iter().any(|n| n["id"] == "captain"));
    }

    #[test]
    fn unknown_role_is_wait_not_verified() {
        let dir = tmp_store();
        let shop = Shop::init(&dir).unwrap();
        let host = spawn_shop(shop);
        let resp = http(&host, "POST", "/api/grokbot", r#"{"role":"VERIFIED"}"#);
        assert!(resp.starts_with("HTTP/1.1 400"), "{resp}");
        let v = json_body(&resp);
        assert!(v.get("error").is_some(), "{v}");
        let get = json_body(&http(&host, "GET", "/api/grokbot", ""));
        assert_eq!(get["role"], GROKBOT_DEFAULT_ROLE);
        assert_ne!(get["role"], "VERIFIED");
    }

    #[test]
    fn page_embeds_grokbot_on_the_graph() {
        assert!(PAGE.contains("id=\"work-graph\""));
        assert!(PAGE.contains("id=\"grok-bot\"") || PAGE.contains("grok-bot"));
        assert!(PAGE.contains("GrokBot"));
        assert!(PAGE.contains("id=\"grokbot-dock\""));
        assert!(PAGE.contains("id=\"grokbot-steer\"") || PAGE.contains("Steer"));
        assert!(CSS.contains("#work-graph") || CSS.contains("work-graph"));
    }
}
