//! TextPCB-style mailbox wire.
//!
//! Assign JSON always lands in `.shop/outbox/` (durable). If a mailbox root is
//! present and not forbidden, the same record is also written to `inbox/<peer>/`.
//! If the mailbox is down, forbidden, or inbox is missing, assign still succeeds
//! from store + outbox. One unread assign per peer inbox; a second distinct
//! assign stays outbox-only (same filename may overwrite on bounce). Unit tests
//! never need a live Windows mailbox. This adapter never writes
//! `C:\TextPCB Platform`. It is not a fifth product.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::hash::format_rfc3339;
use crate::hash::unix_now;
use crate::project::is_forbidden_project;
use crate::store::{ensure_under, write_json_atomic};
use crate::types::Child;

pub const SCHEMA_VERSION: &str = "textpcb/agent-bridge/v1";
pub const ASSIGN_TYPE: &str = "assign";
pub const STEER_TYPE: &str = "steer";
/// CONTROL plane source. Never grok-bot.
pub const DEFAULT_FROM: &str = "cursor-groksuperheavy";
/// CONTROL plane target. Never grok-bot.
pub const STEER_TO: &str = "cursor-groksuperheavy";
pub const STEER_FROM: &str = "user";

/// Claim ceiling on an assign record: assignment only. Not authority to CLOSE.
pub const CLAIM_CEILING: &str =
    "assign-only; handoff is evidence; verify required to close; no invented lanes";

/// Inbox rule. A second distinct unread assign does not land in the peer inbox.
pub const ONE_ASSIGN_IN_FLIGHT: &str =
    "one unread assign per peer inbox; second assign is outbox-only; not a fifth product";

/// This file is the mailbox adapter. It does not invent a fifth product.
pub const FIFTH_PRODUCT: bool = false;

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

/// Windows mailbox default when `SHOP_MAILBOX` is unset. Not Platform.
pub const WINDOWS_MAILBOX_DEFAULT: &str = r"C:\Users\jetga\.textpcb-agent-bridge";

fn store_mailbox(store_root: &Path) -> PathBuf {
    store_root.join("mailbox")
}

/// True when a mailbox root would write Platform or otherwise leave the adapter.
fn mailbox_forbidden(root: &Path) -> bool {
    is_forbidden_project(root)
}

/// `--mailbox` > `SHOP_MAILBOX` > Windows default > `{store}/mailbox` (tests/linux).
/// Any Platform path falls back to `{store}/mailbox`.
pub fn resolve_mailbox(explicit: Option<PathBuf>, store_root: &Path) -> PathBuf {
    let fallback = store_mailbox(store_root);
    if let Some(p) = explicit {
        if mailbox_forbidden(&p) {
            return fallback;
        }
        return p;
    }
    if let Ok(v) = std::env::var("SHOP_MAILBOX") {
        let t = v.trim();
        if !t.is_empty() {
            let p = PathBuf::from(t);
            if mailbox_forbidden(&p) {
                return fallback;
            }
            return p;
        }
    }
    #[cfg(windows)]
    {
        let win = PathBuf::from(WINDOWS_MAILBOX_DEFAULT);
        if mailbox_forbidden(&win) {
            return fallback;
        }
        return win;
    }
    #[cfg(not(windows))]
    fallback
}

/// Peer inbox is a single filename-safe token under `inbox/`. Rejects `read`,
/// `..`, separators, and anything that would walk out of the mailbox root.
fn sanitize_peer(peer: &str) -> Option<&str> {
    let t = peer.trim();
    if t.is_empty() || t == "read" || t == "." || t == ".." {
        return None;
    }
    if !t
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(t)
}

/// Inbox is usable when the mailbox root exists and is not Platform.
/// Missing inbox or a bad peer => outbox only.
fn inbox_peer_dir(root: &Path, peer: &str) -> Option<PathBuf> {
    if !root.exists() || mailbox_forbidden(root) {
        return None;
    }
    let peer = sanitize_peer(peer)?;
    Some(root.join("inbox").join(peer))
}

fn try_write_inbox(root: &Path, peer: &str, name: &str, record: &impl Serialize) -> Option<PathBuf> {
    let inbox_peer = inbox_peer_dir(root, peer)?;
    fs::create_dir_all(&inbox_peer).ok()?;
    let mail_path = inbox_peer.join(name);
    if ensure_under(&mail_path, &[root.to_path_buf()]).is_ok()
        && write_json_atomic(&mail_path, record).is_ok()
    {
        Some(mail_path)
    } else {
        None
    }
}

/// True when `peer` already has a different unread assign file.
/// Same `name` (bounce / re-dispatch) is not a second assign.
fn inbox_has_other_assign(root: &Path, peer: &str, name: &str) -> bool {
    let Some(dir) = inbox_peer_dir(root, peer) else {
        return false;
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        let path = e.path();
        if !path.is_file() {
            return false;
        }
        let n = e.file_name().to_string_lossy().into_owned();
        if n.starts_with('.') || n == name {
            return false;
        }
        n.to_ascii_lowercase().contains("assign")
    })
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
        // One assign in flight: a distinct unread assign stays outbox-only.
        if !inbox_has_other_assign(root, &record.to, &name) {
            if let Some(mail_path) = try_write_inbox(root, &record.to, &name, record) {
                written.push(mail_path);
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
    #[serde(default)]
    pub memory: serde_json::Value,
}

impl SteerRecord {
    pub fn new(body: &str) -> Self {
        Self::with_memory(body, serde_json::json!({}))
    }

    pub fn with_memory(body: &str, memory: serde_json::Value) -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            type_: STEER_TYPE.to_string(),
            from: STEER_FROM.to_string(),
            to: STEER_TO.to_string(),
            body: body.to_string(),
            created_at: format_rfc3339(unix_now()),
            memory,
        }
    }
}

/// Steer mail always lands in outbox. Inbox write only if mailbox root exists
/// and is not Platform. Steer target is CONTROL (`STEER_TO`), never grok-bot.
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
        if let Some(mail_path) = try_write_inbox(root, STEER_TO, &name, record) {
            written.push(mail_path);
        }
    }
    Ok(written)
}

fn wait_obs(root: Option<String>, present: bool, wait: String, outbox_count: usize) -> MailboxObservation {
    MailboxObservation {
        root,
        present,
        inbox_present: false,
        wait: Some(wait),
        peers: vec![],
        unread_total: 0,
        outbox_count,
    }
}

pub fn observe(mailbox: Option<&Path>, outbox: &Path) -> MailboxObservation {
    let outbox_count = count_json_files(outbox);
    let Some(root) = mailbox else {
        return wait_obs(None, false, "WAIT: mailbox path unset".into(), outbox_count);
    };
    let root_s = root.display().to_string();
    if mailbox_forbidden(root) {
        return wait_obs(
            Some(root_s),
            false,
            "WAIT: mailbox path is forbidden (not Platform)".into(),
            outbox_count,
        );
    }
    if !root.exists() {
        return wait_obs(
            Some(root_s),
            false,
            format!("WAIT: mailbox directory missing at {root_s}"),
            outbox_count,
        );
    }
    let inbox = root.join("inbox");
    if !inbox.is_dir() {
        return wait_obs(
            Some(root_s),
            true,
            format!("WAIT: mailbox inbox missing at {}", inbox.display()),
            outbox_count,
        );
    }
    let mut peers = Vec::new();
    if let Ok(entries) = fs::read_dir(&inbox) {
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let peer = entry.file_name().to_string_lossy().into_owned();
            if sanitize_peer(&peer).is_none() {
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

    fn child(id: &str, peer: &str) -> Child {
        Child::new(
            id.into(),
            peer.into(),
            vec!["src/mailbox.rs".into()],
            "t".into(),
            "do the work".into(),
        )
    }

    fn scratch(label: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "shop-mailbox-{}-{}-{label}",
            std::process::id(),
            unix_now()
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn assign_record_json_shape() {
        let rec = AssignRecord::from_child(DEFAULT_FROM, &child("lane-1", "peer-a"));
        let v = serde_json::to_value(&rec).unwrap();
        assert_eq!(v["schema_version"], SCHEMA_VERSION);
        assert_eq!(v["type"], "assign");
        assert_eq!(v["from"], DEFAULT_FROM);
        assert_eq!(v["to"], "peer-a");
        assert_eq!(v["lane_id"], "lane-1");
        assert_eq!(v["allowed_paths"], serde_json::json!(["src/mailbox.rs"]));
        assert_eq!(v["body"], "do the work");
        assert_eq!(v["claim_ceiling"], CLAIM_CEILING);
        assert!(v.get("pcb").is_none());
        assert!(v.get("cad").is_none());
        assert_ne!(DEFAULT_FROM, "grok-bot");
        assert_ne!(STEER_TO, "grok-bot");
    }

    #[test]
    fn resolve_mailbox_prefers_explicit_then_refuses_platform() {
        assert_eq!(
            WINDOWS_MAILBOX_DEFAULT,
            r"C:\Users\jetga\.textpcb-agent-bridge"
        );
        assert!(!WINDOWS_MAILBOX_DEFAULT.contains("TextPCB Platform"));
        assert!(!is_forbidden_project(Path::new(WINDOWS_MAILBOX_DEFAULT)));
        let store = PathBuf::from("/tmp/shop-store-mb");
        let explicit = resolve_mailbox(Some(PathBuf::from("/tmp/bridge")), &store);
        assert_eq!(explicit, PathBuf::from("/tmp/bridge"));
        let forbidden = PathBuf::from(r"C:\TextPCB Platform");
        assert_eq!(
            resolve_mailbox(Some(forbidden), &store),
            store.join("mailbox")
        );
        #[cfg(not(windows))]
        {
            let fallback = resolve_mailbox(Some(store.join("mailbox")), &store);
            assert_eq!(fallback, store.join("mailbox"));
        }
    }

    #[test]
    fn write_assign_outbox_only_when_mailbox_missing() {
        let dir = scratch("missing");
        let outbox = dir.join("outbox");
        let mailbox = dir.join("missing-mailbox");
        let rec = AssignRecord::from_child(DEFAULT_FROM, &child("c1", "alice"));
        let written = write_assign(&outbox, Some(&mailbox), "P", &rec).unwrap();
        assert_eq!(written.len(), 1);
        assert!(outbox.join("assign-P-c1.json").is_file());
        assert!(!mailbox.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_assign_mirrors_inbox_when_root_present() {
        let dir = scratch("present");
        let outbox = dir.join("outbox");
        let mailbox = dir.join("bridge");
        fs::create_dir_all(mailbox.join("inbox")).unwrap();
        let rec = AssignRecord::from_child(DEFAULT_FROM, &child("c1", "alice"));
        let written = write_assign(&outbox, Some(&mailbox), "P", &rec).unwrap();
        assert_eq!(written.len(), 2);
        let inbox = mailbox.join("inbox/alice/assign-P-c1.json");
        assert!(inbox.is_file());
        let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&inbox).unwrap()).unwrap();
        assert_eq!(v["type"], "assign");
        assert_eq!(v["from"], DEFAULT_FROM);
        assert_eq!(v["to"], "alice");
        assert_eq!(v["lane_id"], "c1");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_assign_never_writes_platform() {
        let dir = scratch("platform");
        let outbox = dir.join("outbox");
        let mailbox = dir.join("TextPCB Platform");
        fs::create_dir_all(&mailbox).unwrap();
        let rec = AssignRecord::from_child(DEFAULT_FROM, &child("c1", "alice"));
        let written = write_assign(&outbox, Some(&mailbox), "P", &rec).unwrap();
        assert_eq!(written.len(), 1);
        assert!(outbox.join("assign-P-c1.json").is_file());
        assert!(!mailbox.join("inbox").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_assign_rejects_peer_path_escape() {
        let dir = scratch("escape");
        let outbox = dir.join("outbox");
        let mailbox = dir.join("bridge");
        fs::create_dir_all(mailbox.join("inbox")).unwrap();
        let rec = AssignRecord::from_child(DEFAULT_FROM, &child("c1", "../escape"));
        let written = write_assign(&outbox, Some(&mailbox), "P", &rec).unwrap();
        assert_eq!(written.len(), 1);
        assert!(!dir.join("escape").exists());
        assert!(!mailbox.join("escape").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_steer_targets_control_never_grok_bot() {
        let dir = scratch("steer");
        let outbox = dir.join("outbox");
        let mailbox = dir.join("bridge");
        fs::create_dir_all(mailbox.join("inbox")).unwrap();
        let rec = SteerRecord::new("hello");
        assert_eq!(rec.to, STEER_TO);
        assert_ne!(rec.to, "grok-bot");
        let written = write_steer(&outbox, Some(&mailbox), &rec).unwrap();
        assert_eq!(written.len(), 2);
        assert!(mailbox
            .join("inbox")
            .join(STEER_TO)
            .join(format!(
                "steer-{}.json",
                rec.created_at
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .collect::<String>()
            ))
            .is_file());
        assert!(!mailbox.join("inbox/grok-bot").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn observe_wait_when_unset_missing_or_forbidden() {
        let dir = scratch("obs");
        let outbox = dir.join("outbox");
        fs::create_dir_all(&outbox).unwrap();
        fs::write(outbox.join("assign-P-c1.json"), "{}").unwrap();

        let unset = observe(None, &outbox);
        assert!(!unset.present);
        assert_eq!(unset.wait.as_deref(), Some("WAIT: mailbox path unset"));
        assert_eq!(unset.outbox_count, 1);

        let missing = dir.join("no-root");
        let miss = observe(Some(&missing), &outbox);
        assert!(!miss.present);
        assert!(miss.wait.unwrap().contains("missing"));

        let forbidden = dir.join("TextPCB Platform");
        let hold = observe(Some(&forbidden), &outbox);
        assert!(!hold.present);
        assert!(hold.wait.unwrap().contains("forbidden"));
        assert!(!forbidden.exists());

        let root = dir.join("bridge");
        fs::create_dir_all(&root).unwrap();
        let no_inbox = observe(Some(&root), &outbox);
        assert!(no_inbox.present);
        assert!(!no_inbox.inbox_present);
        assert!(no_inbox.wait.unwrap().contains("inbox missing"));

        fs::create_dir_all(root.join("inbox/alice")).unwrap();
        fs::write(root.join("inbox/alice/assign-P-c1.json"), "{}").unwrap();
        fs::create_dir_all(root.join("inbox/read")).unwrap();
        let ok = observe(Some(&root), &outbox);
        assert!(ok.present && ok.inbox_present);
        assert!(ok.wait.is_none());
        assert_eq!(ok.unread_total, 1);
        assert_eq!(ok.peers.len(), 1);
        assert_eq!(ok.peers[0].peer, "alice");
        assert_eq!(ok.peers[0].latest_assign.as_deref(), Some("assign-P-c1.json"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn adapter_is_not_a_fifth_product() {
        assert!(!FIFTH_PRODUCT);
        assert!(ONE_ASSIGN_IN_FLIGHT.contains("one unread assign"));
        assert!(!ONE_ASSIGN_IN_FLIGHT.contains("workers.rs"));
    }

    #[test]
    fn write_assign_second_to_same_peer_stays_outbox_only() {
        let dir = scratch("one-flight");
        let outbox = dir.join("outbox");
        let mailbox = dir.join("bridge");
        fs::create_dir_all(mailbox.join("inbox")).unwrap();
        let first = AssignRecord::from_child(DEFAULT_FROM, &child("c1", "alice"));
        let second = AssignRecord::from_child(DEFAULT_FROM, &child("c2", "alice"));
        assert_eq!(
            write_assign(&outbox, Some(&mailbox), "P", &first)
                .unwrap()
                .len(),
            2
        );
        let written = write_assign(&outbox, Some(&mailbox), "P", &second).unwrap();
        assert_eq!(written.len(), 1);
        assert!(outbox.join("assign-P-c2.json").is_file());
        assert!(mailbox.join("inbox/alice/assign-P-c1.json").is_file());
        assert!(!mailbox.join("inbox/alice/assign-P-c2.json").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_assign_same_name_may_overwrite_on_bounce() {
        let dir = scratch("bounce");
        let outbox = dir.join("outbox");
        let mailbox = dir.join("bridge");
        fs::create_dir_all(mailbox.join("inbox")).unwrap();
        let rec = AssignRecord::from_child(DEFAULT_FROM, &child("c1", "alice"));
        assert_eq!(
            write_assign(&outbox, Some(&mailbox), "P", &rec)
                .unwrap()
                .len(),
            2
        );
        let again = write_assign(&outbox, Some(&mailbox), "P", &rec).unwrap();
        assert_eq!(again.len(), 2);
        assert!(mailbox.join("inbox/alice/assign-P-c1.json").is_file());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_assign_two_peers_each_get_one_inbox() {
        let dir = scratch("two-peers");
        let outbox = dir.join("outbox");
        let mailbox = dir.join("bridge");
        fs::create_dir_all(mailbox.join("inbox")).unwrap();
        let a = AssignRecord::from_child(DEFAULT_FROM, &child("c1", "alice"));
        let b = AssignRecord::from_child(DEFAULT_FROM, &child("c2", "bob"));
        assert_eq!(write_assign(&outbox, Some(&mailbox), "P", &a).unwrap().len(), 2);
        assert_eq!(write_assign(&outbox, Some(&mailbox), "P", &b).unwrap().len(), 2);
        assert!(mailbox.join("inbox/alice/assign-P-c1.json").is_file());
        assert!(mailbox.join("inbox/bob/assign-P-c2.json").is_file());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_assign_skips_reserved_read_peer() {
        let dir = scratch("read-peer");
        let outbox = dir.join("outbox");
        let mailbox = dir.join("bridge");
        fs::create_dir_all(mailbox.join("inbox")).unwrap();
        let rec = AssignRecord::from_child(DEFAULT_FROM, &child("c1", "read"));
        let written = write_assign(&outbox, Some(&mailbox), "P", &rec).unwrap();
        assert_eq!(written.len(), 1);
        assert!(!mailbox.join("inbox/read").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
