//! Localhost shop UI. No Electron. No npm. One binary.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;

use serde_json::{json, Value};

use crate::engine::Shop;
use crate::error::{Result, ShopError};

pub const DEFAULT_PORT: u16 = 7745;
pub const PAGE: &str = include_str!("ui.html");

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
    listener.set_nonblocking(false)?;
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let shop = shop.clone();
                let _ = handle_client(s, &shop);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn handle_client(mut stream: TcpStream, shop: &Shop) -> Result<()> {
    let mut buf = vec![0u8; 64 * 1024];
    let n = stream.read(&mut buf)?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let (method, path, body) = parse_request(&req);
    let (status, ctype, payload) = route(shop, &method, &path, &body);
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

fn route(shop: &Shop, method: &str, path: &str, body: &str) -> (String, String, Vec<u8>) {
    match (method, path) {
        ("GET", "/") => (
            "200 OK".into(),
            "text/html; charset=utf-8".into(),
            PAGE.as_bytes().to_vec(),
        ),
        ("GET", "/api/workers") => json_ok(workers_json(shop)),
        ("GET", "/api/github") => json_ok(github_panel(shop)),
        ("POST", "/workers") | ("POST", "/api/workers") => match add_worker(shop, body) {
            Ok(_) if path == "/workers" => redirect("/"),
            Ok(v) => json_ok(v),
            Err(e) => json_err(&e),
        },
        (m, p) if m == "POST" && p.starts_with("/workers/") && p.ends_with("/intelligence") => {
            let peer = p
                .trim_start_matches("/workers/")
                .trim_end_matches("/intelligence");
            match set_intel(shop, peer, body) {
                Ok(_) => redirect("/"),
                Err(e) => json_err(&e),
            }
        }
        (m, p) if m == "POST" && p.starts_with("/workers/") && p.ends_with("/github-skills") => {
            let peer = p
                .trim_start_matches("/workers/")
                .trim_end_matches("/github-skills");
            match set_skills(shop, peer, body) {
                Ok(_) => redirect("/"),
                Err(e) => json_err(&e),
            }
        }
        ("POST", "/github/connect") | ("POST", "/api/github/connect") => {
            match connect(shop, body) {
                Ok(_) if path == "/github/connect" => redirect("/"),
                Ok(v) => json_ok(v),
                Err(e) => json_err(&e),
            }
        }
        ("POST", "/github/merge") | ("POST", "/api/github/merge") => match merge(shop, body) {
            Ok(v) => json_ok(v),
            Err(e) => json_err(&e),
        },
        _ => (
            "404 Not Found".into(),
            "text/plain".into(),
            b"not found".to_vec(),
        ),
    }
}

fn workers_json(shop: &Shop) -> Value {
    match shop.workers() {
        Ok(r) => serde_json::to_value(r).unwrap_or(json!({"workers":[]})),
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn github_panel(shop: &Shop) -> Value {
    let repo = shop.github_repo().ok().flatten();
    let authed = shop.github_skills().authenticated();
    let mut issues = json!("WAIT: not connected or not authenticated");
    let mut pulls = json!("WAIT: not connected or not authenticated");
    let mut checks = json!("WAIT: missing PR or checks");
    if let Some(r) = &repo {
        if authed {
            issues = shop
                .github_skills()
                .invoke("issues.list", json!({"owner": r.owner, "repo": r.name}))
                .unwrap_or_else(|e| json!({"WAIT": e.to_string()}));
            pulls = shop
                .github_skills()
                .invoke("pulls.list", json!({"owner": r.owner, "repo": r.name}))
                .unwrap_or_else(|e| json!({"WAIT": e.to_string()}));
            checks = json!("WAIT: open a reduce PR before checks are evidence");
        }
    }
    json!({
        "connected": repo.is_some(),
        "repo": repo.as_ref().map(|r| r.slug()),
        "authenticated": authed,
        "reason": if authed { None } else { Some("no GitHub auth; never fake") },
        "issues": issues,
        "pulls": pulls,
        "checks": checks,
        "skills": crate::skills::SKILL_PACK,
    })
}

fn add_worker(shop: &Shop, body: &str) -> Result<Value> {
    let f = form(body);
    let peer = f.get("peer").cloned().unwrap_or_default();
    let name = f.get("name").cloned().unwrap_or_default();
    let backend = f.get("backend").cloned().unwrap_or_default();
    let model = f.get("model").cloned().unwrap_or_default();
    let skills = f
        .get("github_skills")
        .map(|s| s != "0" && s != "false")
        .unwrap_or(true);
    let mut shop = shop.clone();
    let w = shop.add_worker(&peer, &name, &backend, &model, skills)?;
    Ok(serde_json::to_value(w)?)
}

fn set_intel(shop: &Shop, peer: &str, body: &str) -> Result<Value> {
    let f = form(body);
    let backend = f.get("backend").cloned().unwrap_or_default();
    let model = f.get("model").cloned().unwrap_or_default();
    let mut shop = shop.clone();
    let w = shop.set_worker_intelligence(peer, &backend, &model)?;
    Ok(serde_json::to_value(w)?)
}

fn set_skills(shop: &Shop, peer: &str, body: &str) -> Result<Value> {
    let f = form(body);
    let enabled = f
        .get("enabled")
        .map(|s| s == "1" || s == "true")
        .unwrap_or(false);
    let mut shop = shop.clone();
    let w = shop.set_worker_github_skills(peer, enabled)?;
    Ok(serde_json::to_value(w)?)
}

fn connect(shop: &Shop, body: &str) -> Result<Value> {
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
    let mut shop = shop.clone();
    let repo = shop.github_connect(&spec, branch.as_deref(), token.as_deref())?;
    Ok(json!({"repo": repo.slug()}))
}

fn merge(shop: &Shop, body: &str) -> Result<Value> {
    let f = form(body);
    let confirm = f.get("confirm").cloned().unwrap_or_default();
    if confirm != "MERGE" {
        return Err(ShopError::Wait(
            "merge is explicit only; type MERGE to confirm".into(),
        ));
    }
    let repo = shop
        .github_repo()?
        .ok_or_else(|| ShopError::Wait("no connected repo".into()))?;
    let n: u64 = f
        .get("number")
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| ShopError::IncompleteEvidence("PR number required".into()))?;
    shop.github_skills().invoke(
        "pulls.merge",
        json!({"owner": repo.owner, "repo": repo.name, "pullNumber": n}),
    )
}

fn form(body: &str) -> std::collections::BTreeMap<String, String> {
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        if let Some(obj) = v.as_object() {
            return obj
                .iter()
                .filter_map(|(k, val)| Some((k.clone(), val.as_str()?.to_string())))
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

/// Used by CLI when the store is missing.
pub fn ensure_store(dir: PathBuf) -> Result<Shop> {
    match Shop::open_store(&dir) {
        Ok(s) => Ok(s),
        Err(_) => Shop::init(dir),
    }
}
