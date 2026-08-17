//! TextPCB-style mailbox wire.
//!
//! Assign JSON always lands in `.shop/outbox/` (durable). If a mailbox root is
//! present, the same record is also written to `inbox/<peer>/`. If the mailbox
//! is down or inbox is missing, assign still succeeds from store + outbox.
//! Unit tests never need a live Windows mailbox.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::hash::format_rfc3339;
use crate::hash::unix_now;
use crate::store::{ensure_under, write_json_atomic};
use crate::types::Child;

pub const SCHEMA_VERSION: &str = "textpcb/agent-bridge/v1";
pub const ASSIGN_TYPE: &str = "assign";
pub const STEER_TYPE: &str = "steer";
pub const DEFAULT_FROM: &str = "cursor-groksuperheavy";
/// CONTROL plane target. Never grok-bot.
pub const STEER_TO: &str = "cursor-groksuperheavy";
pub const STEER_FROM: &str = "user";

/// Claim ceiling on an assign record: assignment only. Not authority to CLOSE.
pub const CLAIM_CEILING: &str =
    "assign-only; handoff is evidence; verify required to close; no invented lanes";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignRecord {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub from: String,
    pub to: String,
    pub lane_id: String,
    pub allowed_paths: Vec<String>,
    pub body: String,
    pub claim_ceiling: String,
}

impl AssignRecord {
    pub fn from_child(from: &str, child: &Child) -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            type_: ASSIGN_TYPE.to_string(),
            from: from.to_string(),
            to: child.peer.clone(),
            lane_id: child.id.clone(),
            allowed_paths: child.paths.clone(),
            body: child.body.clone(),
            claim_ceiling: CLAIM_CEILING.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerInbox {
    pub peer: String,
    pub unread: usize,
    pub latest_assign: Option<String>,
    pub latest_handoff: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxObservation {
    pub root: Option<String>,
    pub present: bool,
    pub inbox_present: bool,
    pub wait: Option<String>,
    pub peers: Vec<PeerInbox>,
    pub unread_total: usize,
    pub outbox_count: usize,
}

/// `--mailbox` > `SHOP_MAILBOX` > `{store}/mailbox`
pub fn resolve_mailbox(explicit: Option<PathBuf>, store_root: &Path) -> PathBuf {
    if let Some(p) = explicit {
        return p;
    }
    if let Ok(v) = std::env::var("SHOP_MAILBOX") {
        let t = v.trim();
        if !t.is_empty() {
            return PathBuf::from(t);
        }
    }
    store_root.join("mailbox")
}

pub fn write_assign(
    outbox: &Path,
    mailbox: Option<&Path>,
    parent_id: &str,
    record: &AssignRecord,
) -> Result<Vec<PathBuf>> {
    let name = format!("assign-{parent_id}-{}.json", record.lane_id);
    let mut written = Vec::new();

    // Durable path: always record in the shop outbox even if the mailbox is down.
    fs::create_dir_all(outbox)?;
    let out_path = outbox.join(&name);
    ensure_under(&out_path, &[outbox.to_path_buf()])?;
    write_json_atomic(&out_path, record)?;
    written.push(out_path);

    if let Some(root) = mailbox {
        if let Some(inbox_peer) = inbox_peer_dir(root, &record.to) {
            if let Ok(()) = fs::create_dir_all(&inbox_peer) {
                let mail_path = inbox_peer.join(&name);
                if ensure_under(&mail_path, &[root.to_path_buf()]).is_ok()
                    && write_json_atomic(&mail_path, record).is_ok()
                {
                    written.push(mail_path);
                }
            }
        }
    }
    Ok(written)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteerRecord {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub from: String,
    pub to: String,
    pub body: String,
    pub created_at: String,
}

impl SteerRecord {
    pub fn new(body: &str) -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            type_: STEER_TYPE.to_string(),
            from: STEER_FROM.to_string(),
            to: STEER_TO.to_string(),
            body: body.to_string(),
            created_at: format_rfc3339(unix_now()),
        }
    }
}

/// Steer mail always lands in outbox. Inbox write only if mailbox root exists.
pub fn write_steer(
    outbox: &Path,
    mailbox: Option<&Path>,
    record: &SteerRecord,
) -> Result<Vec<PathBuf>> {
    let slug: String = record
        .created_at
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let name = format!("steer-{slug}.json");
    let mut written = Vec::new();

    fs::create_dir_all(outbox)?;
    let out_path = outbox.join(&name);
    ensure_under(&out_path, &[outbox.to_path_buf()])?;
    write_json_atomic(&out_path, record)?;
    written.push(out_path);

    if let Some(root) = mailbox {
        if let Some(inbox_peer) = inbox_peer_dir(root, STEER_TO) {
            if let Ok(()) = fs::create_dir_all(&inbox_peer) {
                let mail_path = inbox_peer.join(&name);
                if ensure_under(&mail_path, &[root.to_path_buf()]).is_ok()
                    && write_json_atomic(&mail_path, record).is_ok()
                {
                    written.push(mail_path);
                }
            }
        }
    }
    Ok(written)
}

/// Inbox is usable when the mailbox root exists. Missing inbox => outbox only.
fn inbox_peer_dir(root: &Path, peer: &str) -> Option<PathBuf> {
    if !root.exists() {
        return None;
    }
    Some(root.join("inbox").join(peer))
}

pub fn observe(mailbox: Option<&Path>, outbox: &Path) -> MailboxObservation {
    let outbox_count = count_json_files(outbox);
    let Some(root) = mailbox else {
        return MailboxObservation {
            root: None,
            present: false,
            inbox_present: false,
            wait: Some("WAIT: mailbox path unset".into()),
            peers: vec![],
            unread_total: 0,
            outbox_count,
        };
    };
    let root_s = root.display().to_string();
    if !root.exists() {
        let wait = format!("WAIT: mailbox directory missing at {root_s}");
        return MailboxObservation {
            root: Some(root_s),
            present: false,
            inbox_present: false,
            wait: Some(wait),
            peers: vec![],
            unread_total: 0,
            outbox_count,
        };
    }
    let inbox = root.join("inbox");
    if !inbox.is_dir() {
        return MailboxObservation {
            root: Some(root_s),
            present: true,
            inbox_present: false,
            wait: Some(format!(
                "WAIT: mailbox inbox missing at {}",
                inbox.display()
            )),
            peers: vec![],
            unread_total: 0,
            outbox_count,
        };
    }
    let mut peers = Vec::new();
    if let Ok(entries) = fs::read_dir(&inbox) {
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let peer = entry.file_name().to_string_lossy().into_owned();
            if peer == "read" {
                continue;
            }
            peers.push(scan_peer(&peer, &entry.path()));
        }
    }
    peers.sort_by(|a, b| a.peer.cmp(&b.peer));
    let unread_total = peers.iter().map(|p| p.unread).sum();
    MailboxObservation {
        root: Some(root_s),
        present: true,
        inbox_present: true,
        wait: None,
        peers,
        unread_total,
        outbox_count,
    }
}

fn scan_peer(peer: &str, dir: &Path) -> PeerInbox {
    let mut latest_assign = None;
    let mut latest_handoff = None;
    let mut unread = 0usize;
    if let Ok(entries) = fs::read_dir(dir) {
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        files.sort();
        for path in files {
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.starts_with('.') {
                continue;
            }
            unread += 1;
            let lower = name.to_ascii_lowercase();
            if lower.contains("assign") {
                latest_assign = Some(name);
            } else if lower.contains("handoff") || lower.contains("accept") {
                latest_handoff = Some(name);
            }
        }
    }
    PeerInbox {
        peer: peer.to_string(),
        unread,
        latest_assign,
        latest_handoff,
    }
}

fn count_json_files(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("json"))
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assign_record_json_shape() {
        let child = Child::new(
            "lane-1".into(),
            "peer-a".into(),
            vec!["src/a".into(), "src/b".into()],
            "t".into(),
            "do the work".into(),
        );
        let rec = AssignRecord::from_child(DEFAULT_FROM, &child);
        let v = serde_json::to_value(&rec).unwrap();
        assert_eq!(v["schema_version"], SCHEMA_VERSION);
        assert_eq!(v["type"], "assign");
        assert_eq!(v["from"], DEFAULT_FROM);
        assert_eq!(v["to"], "peer-a");
        assert_eq!(v["lane_id"], "lane-1");
        assert_eq!(v["allowed_paths"], serde_json::json!(["src/a", "src/b"]));
        assert_eq!(v["body"], "do the work");
        assert_eq!(v["claim_ceiling"], CLAIM_CEILING);
        assert!(v.get("pcb").is_none());
        assert!(v.get("cad").is_none());
    }
}
