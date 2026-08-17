//! Shop floor words mapped onto AASM 0.56.1.
//!
//! Shop does **not** vendor AASM, rewrite it in Rust, or invent kernel types.
//! AASM stays the authority calculus. This module is a dictionary plus the
//! evidence classes shop records when the CLI still says `WAIT` / `bounce`.
//!
//! Dictionary (shop word → AASM, from a source read of halthinks/AASM 0.56.1):
//! - shop `bounce` → causal backjump (`compute_backjump` / `apply_backjump`).
//!   Jump to the decision that caused the conflict. Keep unrelated children.
//!   Not chronological undo. Not a kernel type named bounce.
//! - shop `WAIT` → `INCONCLUSIVE` | `INFORMATION_GAP` | `UNKNOWN`.
//!   Missing evidence is not PASS. Never store WAIT as JOINED/VERIFIED.
//! - shop `held` / parent open → AuthorityLease holder + parent-subset math.
//!   A child cannot amplify ops, bounds, depth, or scope.
//!   Lease existence is not effect permission.
//! - shop `recombine` / reduce → **not in AASM yet** (entity MERGED / U4 is NEXT).
//!   Shop reduce stays a shop-floor package. Do not claim AASM recombine.
//! - shop `verify PASS` → evidence-plane certificate only.
//!   Not COMPLETE. Not fact authority. Not achieved state.
//! - shop merge / command ACK → EffectStatus.SUCCEEDED analog.
//!   Command success is not achieved state.
//!
//! Laws (enforced in the engine, tested):
//! 1. Information does not carry authority.
//! 2. Incomplete evidence cannot CLOSE or mark VERIFIED.
//! 3. Cannot close/complete while a mandatory child is still ASSIGNED.
//! 4. Backjump keeps the same peer/paths and does not wipe unrelated JOINED children.
//! 5. Child split cannot claim paths outside the parent's claimed set.
//! 6. Reduce is shop evidence, not an AASM recombine claim.

use serde::{Deserialize, Serialize};

use crate::types::EvidenceStatus;

/// AASM evidence-plane class. Shop CLI may still print WAIT; this is what is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EvidenceClass {
    Inconclusive,
    InformationGap,
    Unknown,
    Pass,
}

impl Default for EvidenceClass {
    fn default() -> Self {
        Self::Unknown
    }
}

impl EvidenceClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inconclusive => "INCONCLUSIVE",
            Self::InformationGap => "INFORMATION_GAP",
            Self::Unknown => "UNKNOWN",
            Self::Pass => "PASS",
        }
    }

    /// Shop WAIT is these three classes. PASS is not WAIT and cannot be promoted from them.
    pub fn is_shop_wait(self) -> bool {
        !matches!(self, Self::Pass)
    }

    pub fn to_shop_status(self) -> EvidenceStatus {
        if self == Self::Pass {
            EvidenceStatus::Pass
        } else {
            EvidenceStatus::Wait
        }
    }
}

pub const BACKJUMP_NOTE: &str =
    "AASM causal backjump (compute_backjump/apply_backjump); same peer/paths; unrelated children kept";
pub const LEASE_NOTE: &str =
    "AASM AuthorityLease holder; lease existence is not effect permission; child cannot amplify scope";
pub const REDUCE_NOT_RECOMBINE: &str =
    "shop-floor reduce package; not AASM recombine (entity MERGED / U4 is NEXT)";
pub const VERIFY_PASS_NOTE: &str =
    "AASM evidence-plane certificate only; not COMPLETE; not fact authority; not achieved state";
pub const VERIFY_GAP_NOTE: &str = "AASM INFORMATION_GAP; missing evidence is not PASS";
pub const VERIFY_INCONCLUSIVE_NOTE: &str = "AASM INCONCLUSIVE; command ran; not PASS";
pub const VERIFY_UNKNOWN_NOTE: &str = "AASM UNKNOWN; incomplete evidence is not PASS";
pub const MERGE_ACK_NOTE: &str =
    "AASM EffectStatus.SUCCEEDED analog; command success is not achieved state";

pub fn classify_wait_reason(reason: &str) -> EvidenceClass {
    let r = reason.to_ascii_lowercase();
    if r.contains("missing")
        || r.contains("no verify")
        || r.contains("mailbox")
        || r.contains("token")
        || r.contains("not connected")
        || r.contains("assigned")
        || r.contains("gap")
    {
        EvidenceClass::InformationGap
    } else if r.contains("fail") || r.contains("exit") || r.contains("conflict") {
        EvidenceClass::Inconclusive
    } else {
        EvidenceClass::Unknown
    }
}

pub fn classify_verify(
    status: EvidenceStatus,
    exit_code: Option<i32>,
    last_lines: &str,
) -> EvidenceClass {
    if status == EvidenceStatus::Pass && exit_code == Some(0) {
        return EvidenceClass::Pass;
    }
    classify_wait_reason(last_lines)
}

pub fn note_for(class: EvidenceClass) -> &'static str {
    match class {
        EvidenceClass::Pass => VERIFY_PASS_NOTE,
        EvidenceClass::InformationGap => VERIFY_GAP_NOTE,
        EvidenceClass::Inconclusive => VERIFY_INCONCLUSIVE_NOTE,
        EvidenceClass::Unknown => VERIFY_UNKNOWN_NOTE,
    }
}

/// Parent-subset: child path must equal or sit under a claimed parent path.
/// A wider child path is scope amplification and is refused.
pub fn path_within_parent_scope(child: &str, parent_scope: &str) -> bool {
    let c = crate::paths::normalize_path(child);
    let p = crate::paths::normalize_path(parent_scope);
    if c.is_empty() || p.is_empty() {
        return false;
    }
    c == p || c.starts_with(&(p + "/"))
}

pub fn paths_within_parent_scope(
    child_paths: &[String],
    parent_scope: &[String],
) -> Option<String> {
    if parent_scope.is_empty() {
        return None;
    }
    for c in child_paths {
        if !parent_scope.iter().any(|p| path_within_parent_scope(c, p)) {
            return Some(c.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_classes_are_not_pass() {
        assert!(EvidenceClass::Inconclusive.is_shop_wait());
        assert!(EvidenceClass::InformationGap.is_shop_wait());
        assert!(EvidenceClass::Unknown.is_shop_wait());
        assert!(!EvidenceClass::Pass.is_shop_wait());
        assert_eq!(
            classify_verify(EvidenceStatus::Wait, None, "no verify command recorded"),
            EvidenceClass::InformationGap
        );
        assert_ne!(
            classify_verify(EvidenceStatus::Wait, Some(1), "exit 1"),
            EvidenceClass::Pass
        );
    }

    #[test]
    fn parent_subset_refuses_amplification() {
        assert!(path_within_parent_scope("src/a", "src"));
        assert!(path_within_parent_scope("src/a", "src/a"));
        assert!(!path_within_parent_scope("src", "src/a"));
        assert!(!path_within_parent_scope("other", "src"));
        assert_eq!(
            paths_within_parent_scope(&["src/c".into()], &["src/a".into(), "src/b".into()]),
            Some("src/c".into())
        );
        assert_eq!(
            paths_within_parent_scope(&["src/a/x".into()], &["src/a".into()]),
            None
        );
    }
}
