//! Fail-closed errors. Incomplete evidence is WAIT, never a fake PASS.

use std::path::PathBuf;

use crate::types::{ChildState, ParentState};

#[derive(Debug, thiserror::Error)]
pub enum ShopError {
    #[error("shop store not found at {0}")]
    StoreNotFound(PathBuf),

    #[error("invalid id '{0}': must be a non-empty filename-safe token")]
    InvalidId(String),

    #[error("parent '{0}' already exists")]
    ParentExists(String),

    #[error("parent '{0}' not found")]
    ParentNotFound(String),

    #[error("child '{0}' already exists on parent '{1}'")]
    ChildExists(String, String),

    #[error("child '{0}' not found on parent '{1}'")]
    ChildNotFound(String, String),

    #[error(
        "path claim overlap: '{path}' conflicts with in-flight child '{child}' of parent '{parent}' (path '{existing}')"
    )]
    PathOverlap {
        path: String,
        parent: String,
        child: String,
        existing: String,
    },

    #[error("join barrier: cannot reduce until every child is JOINED")]
    JoinBarrier,

    #[error("incomplete evidence is WAIT, never fake PASS: {0}")]
    IncompleteEvidence(String),

    #[error("cannot close parent '{0}': not VERIFIED (incomplete evidence is WAIT)")]
    CannotClose(String),

    #[error(
        "cannot close/complete parent '{0}': mandatory child still ASSIGNED (obligation open)"
    )]
    ObligationOpen(String),

    #[error(
        "child path '{path}' is outside parent '{parent}' claimed set (parent-subset; child cannot amplify scope)"
    )]
    ParentSubset { path: String, parent: String },

    #[error("invalid parent state {current} for {op}")]
    InvalidParentState {
        current: ParentState,
        op: &'static str,
    },

    #[error("invalid child state {current} for {op}")]
    InvalidChildState {
        current: ChildState,
        op: &'static str,
    },

    #[error("shop refuses to write outside the store or mailbox: {0}")]
    ForbiddenWrite(PathBuf),

    #[error("allowed_paths must be a non-empty comma-separated list")]
    EmptyPaths,

    #[error("unknown worker peer '{0}'; enroll capacity with shop ui or shop add first")]
    UnknownPeer(String),

    #[error("duplicate worker peer '{0}'")]
    DuplicatePeer(String),

    #[error("unknown intelligence backend '{0}' (cursor | cursor-ultra | grok-ultra | grok-bot | grok-build)")]
    UnknownBackend(String),

    #[error("unknown GitHub skill '{0}'")]
    UnknownSkill(String),

    #[error("WAIT: {0}")]
    Wait(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ShopError>;
