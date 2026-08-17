//! Ingest SuperGrokHeavy replies into the steer transcript.
//!
//! Shop never fakes a boss line. Only written mailbox/store files with
//! type `steer-reply` | `handoff` | `reply` from `cursor-groksuperheavy`.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::Result;
use crate::hash::fingerprint;
use crate::mailbox::STEER_TO;
use crate::project::refuse_platform;
use crate::store::{ensure_under, write_json_atomic};

const REPLY_TYPES: &[&str] = &["steer-reply", "handoff", "reply"];

#[derive(Debug, Clone)]
pub struct IngestedReply {
    pub key: String,
    pub body: String,
}

pub fn scan_replies(store: &Path, mailbox: Option<&Path>) -> Vec<IngestedReply> {
    let mut out = Vec::new();
    let mut dirs = Vec::new();
    dirs.push(store.join("inbox"));
    dirs.push(store.join("inbox").join("shop"));
    dirs.push(store.join("inbox").join(STEER_TO));
    if let Some(root) = mailbox {
        dirs.push(root.join("inbox"));
        dirs.push(root.join("inbox").join("shop"));
        dirs.push(root.join("inbox").join(STEER_TO));
    }
    for dir in dirs {
        if refuse_platform(&dir).is_err() {
            continue;
        }
        push_dir_replies(&dir, &mut out);
    }
    out
}

fn push_dir_replies(dir: &Path, out: &mut Vec<IngestedReply>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        if let Some(body) = extract_reply(&v) {
            let key = fingerprint(&bytes);
            out.push(IngestedReply { key, body });
        }
    }
}

fn extract_reply(v: &Value) -> Option<String> {
    let type_ = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if !REPLY_TYPES.contains(&type_) {
        return None;
    }
    let from = v.get("from").and_then(|t| t.as_str()).unwrap_or("");
    if !from.is_empty() && from != STEER_TO && !from.eq_ignore_ascii_case("SuperGrokHeavy") {
        return None;
    }
    let body = v
        .get("body")
        .and_then(|t| t.as_str())
        .or_else(|| v.get("text").and_then(|t| t.as_str()))
        .unwrap_or("")
        .trim();
    if body.is_empty() {
        return None;
    }
    Some(body.to_string())
}

pub fn load_seen(store: &Path) -> Vec<String> {
    let path = store.join("replies-seen.json");
    let Ok(bytes) = fs::read(path) else {
        return Vec::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub fn save_seen(store: &Path, seen: &[String]) -> Result<()> {
    if refuse_platform(store).is_err() {
        return Ok(());
    }
    let path = store.join("replies-seen.json");
    ensure_under(&path, &[store.to_path_buf()])?;
    write_json_atomic(&path, &seen)?;
    Ok(())
}

pub fn boss_cmd_wait() -> Option<String> {
    match std::env::var("SHOP_BOSS_CMD") {
        Ok(cmd) if !cmd.trim().is_empty() => {
            let cmd = cmd.trim();
            if crate::project::is_forbidden_project(Path::new(cmd)) {
                return Some("WAIT: boss process not launched".into());
            }
            let mut parts = cmd.split_whitespace();
            let Some(bin) = parts.next() else {
                return Some("WAIT: boss process not launched".into());
            };
            let args: Vec<&str> = parts.collect();
            match std::process::Command::new(bin).args(args).spawn() {
                Ok(_) => None,
                Err(_) => Some("WAIT: boss process not launched".into()),
            }
        }
        _ => Some("WAIT: boss process not launched".into()),
    }
}

pub fn listen_roots(store: &Path, mailbox: Option<&Path>) -> Vec<PathBuf> {
    let mut v = vec![store.join("inbox")];
    if let Some(m) = mailbox {
        v.push(m.join("inbox"));
    }
    v
}
