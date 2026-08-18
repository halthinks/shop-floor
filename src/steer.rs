//! CONTROL plane. Local shop verbs run here. Free text goes to SuperGrokHeavy
//! (`cursor-groksuperheavy`), never to grok-bot. Shop never invents a lane.
//!
//! A steer record is Evidence. It cannot CLOSE a parent or mint VERIFIED.
//! Missing mailbox / inbox / forbidden root is WAIT, never default PASS.
//! AASM stays look-only. This file does not vendor the kernel.
//! This file is not a fifth product. It never writes `C:\TextPCB Platform`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{Result, ShopError};
use crate::hash::{format_rfc3339, unix_now};
use crate::mailbox::{write_steer, SteerRecord};
use crate::memory::MemoryTier;
use crate::project::is_forbidden_project;

/// AASM stays look-only. This file does not vendor the kernel.
pub const COPIES_KERNEL: bool = false;

/// This file cannot CLOSE a parent. A steer record is Evidence, not authority.
pub const CAN_CLOSE: bool = false;

/// This file cannot mint VERIFIED. A steer record is not a verify certificate.
pub const CAN_MINT_VERIFIED: bool = false;

/// This file is the control-plane steer parser. It does not invent a fifth product.
pub const FIFTH_PRODUCT: bool = false;

/// A steer record is Evidence. This file cannot CLOSE a parent.
pub const STEER_IS_EVIDENCE: &str = "steer record is Evidence; cannot CLOSE or mark VERIFIED";

/// Missing mailbox/inbox/forbidden root. Recorded locally. No reply faked. Never PASS.
pub const WAIT_MAILBOX: &str =
    "WAIT: mailbox missing; recorded locally; no SuperGrokHeavy reply faked";

/// Refused Platform write. Missing evidence is WAIT, not a fake PASS.
pub const WAIT_PLATFORM: &str =
    "WAIT: refused C:\\TextPCB Platform; recorded nothing; no SuperGrokHeavy reply faked";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SteerVerb {
    Status,
    Workers,
    Github,
    AddWorker {
        peer: String,
        backend: String,
        model: String,
    },
    OpenParent {
        id: String,
        title: String,
    },
    Split {
        parent: String,
        child: String,
        peer: String,
        paths: String,
    },
    Assign {
        parent: String,
    },
    Join {
        parent: String,
    },
    Reduce {
        parent: String,
        note: String,
    },
    Verify {
        parent: String,
        cmd: Option<String>,
    },
    Bounce {
        parent: String,
        child: String,
        reason: String,
    },
    OpenProject {
        path: String,
    },
    OpenRepo {
        spec: String,
    },
    Remember {
        tier: MemoryTier,
        fact: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteerLine {
    pub role: String,
    pub text: String,
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

impl SteerLine {
    pub fn new(role: &str, text: impl Into<String>, wait: Option<String>) -> Self {
        Self {
            role: role.to_string(),
            text: text.into(),
            at: format_rfc3339(unix_now()),
            wait,
            to: None,
        }
    }

    /// This file never mints PASS. A missing wait is not PASS.
    pub fn is_pass(&self) -> bool {
        false
    }

    /// A steer line cannot CLOSE a parent.
    pub fn can_close(&self) -> bool {
        CAN_CLOSE
    }

    /// A steer line cannot mint VERIFIED.
    pub fn mints_verified(&self) -> bool {
        CAN_MINT_VERIFIED
    }
}

/// This file never mints PASS.
pub fn is_pass() -> bool {
    false
}

/// A steer record cannot CLOSE a parent.
pub fn can_close() -> bool {
    CAN_CLOSE
}

/// A steer record cannot mint VERIFIED.
pub fn mints_verified() -> bool {
    CAN_MINT_VERIFIED
}

/// Fail-closed: missing mailbox/inbox/forbidden root is WAIT. Present mailbox is not PASS.
pub fn steer_wait(mailbox: Option<&Path>) -> Option<String> {
    if mailbox_ready(mailbox) {
        None
    } else {
        Some(WAIT_MAILBOX.into())
    }
}

fn mailbox_ready(mailbox: Option<&Path>) -> bool {
    let Some(root) = mailbox else {
        return false;
    };
    if is_forbidden_project(root) {
        return false;
    }
    root.is_dir() && root.join("inbox").is_dir()
}

fn is_platform_text(s: &str) -> bool {
    is_forbidden_project(Path::new(s.trim()))
}

/// Parse a local steer verb. Incomplete `split` is not a verb — free text, no lane.
pub fn parse_steer(text: &str) -> Option<SteerVerb> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if let Some(rest) = strip_prefix_ci(t, "remember ") {
        let rest = rest.trim();
        if rest.is_empty() {
            return None;
        }
        if let Some(fact) = strip_prefix_ci(rest, "log ") {
            let fact = fact.trim();
            if fact.is_empty() {
                return None;
            }
            return Some(SteerVerb::Remember {
                tier: MemoryTier::Log,
                fact: fact.to_string(),
            });
        }
        if let Some(fact) = strip_prefix_ci(rest, "profile ") {
            let fact = fact.trim();
            if fact.is_empty() {
                return None;
            }
            return Some(SteerVerb::Remember {
                tier: MemoryTier::Profile,
                fact: fact.to_string(),
            });
        }
        return Some(SteerVerb::Remember {
            tier: MemoryTier::Profile,
            fact: rest.to_string(),
        });
    }
    if lower == "status" {
        return Some(SteerVerb::Status);
    }
    if lower == "workers" {
        return Some(SteerVerb::Workers);
    }
    if lower == "github" {
        return Some(SteerVerb::Github);
    }
    if let Some(rest) = strip_prefix_ci(t, "add worker ") {
        let mut parts = rest.split_whitespace();
        let peer = parts.next()?.to_string();
        let backend = parts.next()?.to_string();
        let model = parts.collect::<Vec<_>>().join(" ");
        return Some(SteerVerb::AddWorker {
            peer,
            backend,
            model,
        });
    }
    if let Some(rest) = strip_prefix_ci(t, "open local ") {
        let path = rest.trim();
        if path.is_empty() || is_platform_text(path) {
            return None;
        }
        return Some(SteerVerb::OpenProject {
            path: path.to_string(),
        });
    }
    if let Some(rest) = strip_prefix_ci(t, "open project ") {
        let path = rest.trim();
        if path.is_empty() || is_platform_text(path) {
            return None;
        }
        return Some(SteerVerb::OpenProject {
            path: path.to_string(),
        });
    }
    if let Some(rest) = strip_prefix_ci(t, "open repo ") {
        let spec = rest.trim();
        if !spec.contains('/') {
            return None;
        }
        return Some(SteerVerb::OpenRepo {
            spec: spec.to_string(),
        });
    }
    if let Some(rest) = strip_prefix_ci(t, "split ") {
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.len() < 4 {
            return None;
        }
        return Some(SteerVerb::Split {
            parent: parts[0].to_string(),
            child: parts[1].to_string(),
            peer: parts[2].to_string(),
            paths: parts[3].to_string(),
        });
    }
    if let Some(rest) = strip_prefix_ci(t, "assign ") {
        let parent = rest.split_whitespace().next()?;
        return Some(SteerVerb::Assign {
            parent: parent.to_string(),
        });
    }
    if let Some(rest) = strip_prefix_ci(t, "join ") {
        let parent = rest.split_whitespace().next()?;
        return Some(SteerVerb::Join {
            parent: parent.to_string(),
        });
    }
    if let Some(rest) = strip_prefix_ci(t, "reduce ") {
        let mut parts = rest.split_whitespace();
        let parent = parts.next()?.to_string();
        let note = parts.collect::<Vec<_>>().join(" ");
        return Some(SteerVerb::Reduce { parent, note });
    }
    if let Some(rest) = strip_prefix_ci(t, "verify ") {
        let mut parts = rest.split_whitespace();
        let parent = parts.next()?.to_string();
        let cmd = {
            let c = parts.collect::<Vec<_>>().join(" ");
            if c.is_empty() {
                None
            } else {
                Some(c)
            }
        };
        return Some(SteerVerb::Verify { parent, cmd });
    }
    if let Some(rest) = strip_prefix_ci(t, "bounce ") {
        let mut parts = rest.split_whitespace();
        let parent = parts.next()?.to_string();
        let child = parts.next()?.to_string();
        let reason = parts.collect::<Vec<_>>().join(" ");
        if reason.is_empty() {
            return None;
        }
        return Some(SteerVerb::Bounce {
            parent,
            child,
            reason,
        });
    }
    if let Some(rest) = strip_prefix_ci(t, "open ") {
        let mut parts = rest.split_whitespace();
        let id = parts.next()?.to_string();
        if id.eq_ignore_ascii_case("project") || id.eq_ignore_ascii_case("repo") {
            return None;
        }
        let title = parts.collect::<Vec<_>>().join(" ");
        if title.is_empty() {
            return None;
        }
        return Some(SteerVerb::OpenParent { id, title });
    }
    None
}

fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    if text.len() < prefix.len() {
        return None;
    }
    if text.get(..prefix.len())?.eq_ignore_ascii_case(prefix) {
        Some(&text[prefix.len()..])
    } else {
        None
    }
}

pub fn steer_to_supergrok(
    outbox: &Path,
    mailbox: Option<&Path>,
    body: &str,
    memory: serde_json::Value,
) -> Result<(SteerRecord, Vec<std::path::PathBuf>, Option<String>)> {
    if is_forbidden_project(outbox) {
        return Err(ShopError::IncompleteEvidence(WAIT_PLATFORM.into()));
    }
    let rec = SteerRecord::with_memory(body, memory);
    let written = write_steer(outbox, mailbox, &rec)?;
    let wait = steer_wait(mailbox);
    Ok((rec, written, wait))
}

pub fn line_json(line: &SteerLine) -> Value {
    json!(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mailbox::STEER_TO;
    use std::fs;
    use std::path::PathBuf;

    fn scratch(label: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "shop-steer-{}-{}-{label}",
            std::process::id(),
            unix_now()
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn assert_never_pass(line: &SteerLine, wait: Option<&str>) {
        assert!(!line.is_pass());
        assert!(!line.can_close());
        assert!(!line.mints_verified());
        assert!(!is_pass());
        assert!(!can_close());
        assert!(!mints_verified());
        assert!(!CAN_CLOSE);
        assert!(!CAN_MINT_VERIFIED);
        let v = line_json(line);
        assert!(v.get("pass").is_none());
        assert!(v.get("verified").is_none());
        assert!(v.get("VERIFIED").is_none());
        assert!(v.get("CLOSE").is_none());
        assert_ne!(v.get("wait").and_then(|w| w.as_str()).unwrap_or(""), "PASS");
        if let Some(word) = wait.or(line.wait.as_deref()) {
            assert!(
                word.contains("WAIT"),
                "missing evidence must be WAIT, got {word}"
            );
            assert!(!word.contains(" is PASS"));
            assert_ne!(word, "PASS");
            assert_ne!(word, "VERIFIED");
        }
    }

    #[test]
    fn free_text_is_not_split() {
        assert!(parse_steer("please look at the floor").is_none());
        assert!(parse_steer("split the work").is_none());
        assert!(parse_steer("split P").is_none());
    }

    #[test]
    fn split_requires_ids_and_paths() {
        assert_eq!(
            parse_steer("split P c1 alice src/a,src/b"),
            Some(SteerVerb::Split {
                parent: "P".into(),
                child: "c1".into(),
                peer: "alice".into(),
                paths: "src/a,src/b".into(),
            })
        );
    }

    #[test]
    fn control_plane_is_not_a_fifth_product_or_kernel() {
        assert!(!FIFTH_PRODUCT);
        assert!(!COPIES_KERNEL);
        assert!(!CAN_CLOSE);
        assert!(!CAN_MINT_VERIFIED);
        assert!(!is_pass());
        assert!(STEER_IS_EVIDENCE.contains("Evidence"));
        assert!(STEER_IS_EVIDENCE.contains("cannot CLOSE"));
        assert!(STEER_IS_EVIDENCE.contains("VERIFIED"));
        assert!(!STEER_IS_EVIDENCE.contains("PASS"));
        assert!(WAIT_MAILBOX.contains("WAIT"));
        assert!(!WAIT_MAILBOX.contains("PASS"));
        assert!(!WAIT_PLATFORM.contains("PASS"));
        assert!(parse_steer("pcb").is_none());
        assert!(parse_steer("cad").is_none());
        assert!(parse_steer("textpcb").is_none());
        assert!(parse_steer("close P").is_none());
        assert!(parse_steer("verified P").is_none());
        assert!(parse_steer("PASS").is_none());
    }

    #[test]
    fn parse_verify_is_a_verb_not_minted_verified() {
        assert_eq!(
            parse_steer("verify P"),
            Some(SteerVerb::Verify {
                parent: "P".into(),
                cmd: None,
            })
        );
        assert!(!CAN_MINT_VERIFIED);
        assert!(!mints_verified());
        assert!(!can_close());
    }

    #[test]
    fn open_project_refuses_platform_as_a_verb() {
        assert!(parse_steer(r"open project C:\TextPCB Platform").is_none());
        assert!(parse_steer(r"open local C:\TextPCB Platform").is_none());
        assert!(parse_steer("open project TextPCB Platform").is_none());
        assert_eq!(
            parse_steer("open project /tmp/shop-floor"),
            Some(SteerVerb::OpenProject {
                path: "/tmp/shop-floor".into(),
            })
        );
    }

    #[test]
    fn missing_mailbox_is_wait_never_pass() {
        let dir = scratch("missing");
        let outbox = dir.join("outbox");
        let (rec, written, wait) =
            steer_to_supergrok(&outbox, None, "look at the floor", json!({})).unwrap();
        assert_eq!(wait.as_deref(), Some(WAIT_MAILBOX));
        assert_eq!(steer_wait(None).as_deref(), Some(WAIT_MAILBOX));
        assert_eq!(rec.to, STEER_TO);
        assert_ne!(rec.to, "grok-bot");
        assert_eq!(written.len(), 1);
        assert!(outbox.is_dir());
        let line = SteerLine::new("shop", "to SuperGrokHeavy", wait.clone());
        assert_never_pass(&line, wait.as_deref());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mailbox_file_or_missing_inbox_is_wait_never_pass() {
        let dir = scratch("inbox-gap");
        let outbox = dir.join("outbox");

        let missing = dir.join("no-root");
        let (_, _, wait) = steer_to_supergrok(&outbox, Some(&missing), "hello", json!({})).unwrap();
        assert_eq!(wait.as_deref(), Some(WAIT_MAILBOX));
        assert_eq!(steer_wait(Some(&missing)).as_deref(), Some(WAIT_MAILBOX));

        let as_file = dir.join("bridge-file");
        fs::write(&as_file, b"not-a-dir").unwrap();
        let (_, _, wait) = steer_to_supergrok(&outbox, Some(&as_file), "hello", json!({})).unwrap();
        assert_eq!(wait.as_deref(), Some(WAIT_MAILBOX));
        assert_never_pass(
            &SteerLine::new("shop", "recorded locally", wait.clone()),
            wait.as_deref(),
        );

        let root = dir.join("bridge");
        fs::create_dir_all(&root).unwrap();
        let (_, written, wait) =
            steer_to_supergrok(&outbox, Some(&root), "hello", json!({})).unwrap();
        assert_eq!(wait.as_deref(), Some(WAIT_MAILBOX));
        assert_eq!(written.len(), 1);
        assert!(!root.join("inbox").exists());
        assert_never_pass(
            &SteerLine::new("shop", "recorded locally", wait.clone()),
            wait.as_deref(),
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ready_mailbox_is_not_pass_and_cannot_close_or_verify() {
        let dir = scratch("ready");
        let outbox = dir.join("outbox");
        let mailbox = dir.join("bridge");
        fs::create_dir_all(mailbox.join("inbox")).unwrap();
        let (rec, written, wait) =
            steer_to_supergrok(&outbox, Some(&mailbox), "hello", json!({})).unwrap();
        assert!(wait.is_none());
        assert!(steer_wait(Some(&mailbox)).is_none());
        assert_eq!(written.len(), 2);
        assert_eq!(rec.type_, "steer");
        assert_eq!(rec.to, STEER_TO);
        assert_ne!(rec.to, "grok-bot");
        let v = serde_json::to_value(&rec).unwrap();
        assert!(v.get("pass").is_none());
        assert!(v.get("verified").is_none());
        assert!(v.get("VERIFIED").is_none());
        assert!(v.get("CLOSE").is_none());
        assert_ne!(v["type"], "VERIFIED");
        let line = SteerLine::new("shop", "to SuperGrokHeavy", wait);
        assert_never_pass(&line, None);
        assert!(!line.is_pass());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn steer_never_writes_platform() {
        let dir = scratch("platform");
        let outbox = dir.join("outbox");
        let platform = PathBuf::from(r"C:\TextPCB Platform");
        assert!(is_forbidden_project(&platform));
        let (rec, written, wait) =
            steer_to_supergrok(&outbox, Some(&platform), "hello", json!({})).unwrap();
        assert_eq!(wait.as_deref(), Some(WAIT_MAILBOX));
        assert_eq!(written.len(), 1);
        assert_eq!(rec.to, STEER_TO);
        assert!(!platform.exists());
        assert_never_pass(
            &SteerLine::new("shop", "refused platform", wait.clone()),
            wait.as_deref(),
        );

        let err = steer_to_supergrok(&platform, None, "hello", json!({})).unwrap_err();
        match err {
            ShopError::IncompleteEvidence(msg) => {
                assert_eq!(msg, WAIT_PLATFORM);
                assert!(msg.contains("WAIT"));
                assert!(!msg.contains("PASS"));
            }
            other => panic!("Platform outbox must WAIT, got {other:?}"),
        }
        assert!(!platform.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
