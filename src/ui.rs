//! Localhost command center. No Electron. No npm. One binary.

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
            json_ok(json!({"lines": shop.steer_transcript()}))
        }
        ("GET", "/api/events") => {
            let shop = app.lock().unwrap_or_else(|e| e.into_inner());
            json_ok(json!({"events": shop.events()}))
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
    let wait = if repo.is_none() {
        Some("WAIT: GitHub repo not connected")
    } else if !authed {
        Some("WAIT: GitHub token missing; never fake")
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
        if authed {
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
        "skills": crate::skills::SKILL_PACK,
    })
}

fn github_search(shop: &Shop, q: String, kind: String) -> Value {
    let Some(r) = shop.github_repo().ok().flatten() else {
        return json!({"WAIT": "GitHub repo not connected"});
    };
    if !shop.github_skills().authenticated() {
        return json!({"WAIT": "GitHub token missing; never fake"});
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
    if !shop.github_skills().authenticated() {
        return json!({"WAIT": "GitHub token missing; never fake"});
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
