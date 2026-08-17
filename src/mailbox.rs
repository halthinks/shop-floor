//! File-based TextPCB agent-bridge assign adapter.
//!
//! This is an outbox shape, not AASM and not a live Windows mailbox requirement.
//! Unit-test the JSON. If `--mailbox` is missing, `shop assign` still writes
//! `.shop/outbox/`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::store::{ensure_under, write_json_atomic};
use crate::types::Child;

pub const SCHEMA_VERSION: &str = "textpcb/agent-bridge/v1";
pub const ASSIGN_TYPE: &str = "assign";
pub const DEFAULT_FROM: &str = "cursor-groksuperheavy";

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

pub fn write_assign(
    outbox: &Path,
    mailbox: Option<&Path>,
    parent_id: &str,
    record: &AssignRecord,
) -> Result<Vec<PathBuf>> {
    let name = format!("assign-{parent_id}-{}.json", record.lane_id);
    let mut written = Vec::new();

    fs::create_dir_all(outbox)?;
    let out_path = outbox.join(&name);
    ensure_under(&out_path, &[outbox.to_path_buf()])?;
    write_json_atomic(&out_path, record)?;
    written.push(out_path);

    if let Some(dir) = mailbox {
        if dir.is_dir() {
            let mail_path = dir.join(&name);
            ensure_under(&mail_path, &[dir.to_path_buf()])?;
            write_json_atomic(&mail_path, record)?;
            written.push(mail_path);
        }
    }
    Ok(written)
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
