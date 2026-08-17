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

/// Pinned source read. Bump only when the dictionary is re-read from AASM.
pub const AASM_PIN: &str = "0.56.1";

/// This file is a dictionary, not a second kernel.
pub const COPIES_KERNEL: bool = false;

/// bounce is a shop word for causal backjump. AASM has no kernel type named bounce.
pub const BOUNCE_IS_KERNEL_TYPE: bool = false;

/// Shop WAIT is exactly these three AASM evidence classes. Not PASS.
pub const WAIT_CLASS_NOTE: &str = "INCONCLUSIVE | INFORMATION_GAP | UNKNOWN";

pub const WAIT_CLASSES: [EvidenceClass; 3] = [
    EvidenceClass::Inconclusive,
    EvidenceClass::InformationGap,
    EvidenceClass::Unknown,
];

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
    /// Fail-closed: any class that is not recorded PASS is WAIT.
    pub fn is_shop_wait(self) -> bool {
        !matches!(self, Self::Pass)
    }

    pub fn is_wait_class(self) -> bool {
        WAIT_CLASSES.contains(&self)
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
pub const INFORMATION_NOT_AUTHORITY: &str =
    "handoff/mailbox/github check is Evidence; cannot CLOSE or mark VERIFIED";

fn normalize_shop_word(word: &str) -> String {
    word.trim().to_ascii_lowercase().replace('_', " ")
}

/// Look-only dictionary: shop floor word → AASM meaning. Not a kernel copy.
/// bounce is causal backjump, not a kernel type named bounce.
pub fn map_shop_word(word: &str) -> Option<&'static str> {
    match normalize_shop_word(word).as_str() {
        "bounce" => Some(BACKJUMP_NOTE),
        "wait" => Some(WAIT_CLASS_NOTE),
        "held" | "parent open" => Some(LEASE_NOTE),
        "reduce" | "recombine" => Some(REDUCE_NOT_RECOMBINE),
        "verify pass" => Some(VERIFY_PASS_NOTE),
        "merge" | "merge ack" | "command ack" => Some(MERGE_ACK_NOTE),
        _ => None,
    }
}

const GAP_MARKERS: &[&str] = &[
    "missing",
    "no verify",
    "mailbox",
    "token",
    "not connected",
    "assigned",
    "gap",
    "handoff",
    "obligation",
    "check run",
    "checks pass",
    "checks passed",
    "check passed",
    "checks have pass",
    "merge ack",
    "recombine",
    "achieved state",
    "fact authority",
];

const INCONCLUSIVE_MARKERS: &[&str] = &["fail", "exit", "conflict", "inconclusive"];

/// Shop WAIT words and invented-authority text. Never PASS, even when the
/// classified wait class is UNKNOWN (Unknown must not fall through to Pass).
const WAIT_BLOCK_MARKERS: &[&str] = &[
    "wait",
    "unknown",
    "inconclusive",
    "incomplete",
    "verified",
    "bounce",
    "backjump",
];

fn has_marker(hay: &str, markers: &[&str]) -> bool {
    markers.iter().any(|m| hay.contains(m))
}

fn line_blocks_pass(reason: &str) -> bool {
    let r = reason.to_ascii_lowercase();
    has_marker(&r, GAP_MARKERS)
        || has_marker(&r, INCONCLUSIVE_MARKERS)
        || has_marker(&r, WAIT_BLOCK_MARKERS)
}

pub fn classify_wait_reason(reason: &str) -> EvidenceClass {
    let r = reason.to_ascii_lowercase();
    if has_marker(&r, GAP_MARKERS) {
        EvidenceClass::InformationGap
    } else if has_marker(&r, INCONCLUSIVE_MARKERS) {
        EvidenceClass::Inconclusive
    } else if r.contains("unknown") {
        EvidenceClass::Unknown
    } else if has_marker(&r, WAIT_BLOCK_MARKERS) {
        EvidenceClass::InformationGap
    } else {
        EvidenceClass::Unknown
    }
}

/// Fail-closed: WAIT / gap / authority-claim text / non-zero or missing exit
/// cannot become PASS. Command success is not achieved state.
/// WAIT / UNKNOWN / bounce / VERIFIED words in the log are not PASS.
pub fn classify_verify(
    status: EvidenceStatus,
    exit_code: Option<i32>,
    last_lines: &str,
) -> EvidenceClass {
    if line_blocks_pass(last_lines) {
        return classify_wait_reason(last_lines);
    }
    if matches!(exit_code, Some(code) if code != 0) {
        return EvidenceClass::Inconclusive;
    }
    if status == EvidenceStatus::Pass && exit_code == Some(0) {
        return EvidenceClass::Pass;
    }
    EvidenceClass::Unknown
}

pub fn note_for(class: EvidenceClass) -> &'static str {
    match class {
        EvidenceClass::Pass => VERIFY_PASS_NOTE,
        EvidenceClass::InformationGap => VERIFY_GAP_NOTE,
        EvidenceClass::Inconclusive => VERIFY_INCONCLUSIVE_NOTE,
        EvidenceClass::Unknown => VERIFY_UNKNOWN_NOTE,
    }
}

/// Absolute, UNC, drive-letter, or control bytes. Not a lease path.
/// `normalize_path` would strip a leading `/`, which is amplification.
fn lease_path_refused_raw(raw: &str) -> bool {
    if raw.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return true;
    }
    let t = raw.trim();
    if t.is_empty() {
        return true;
    }
    if t.starts_with('/') || t.starts_with('\\') {
        return true;
    }
    let b = t.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// Collapse `.` / `..` for lease-subset math only. Escape above the claim root
/// or an empty result is refused. This is not an AASM path type.
fn collapse_lease_path(raw: &str) -> Option<Vec<String>> {
    if lease_path_refused_raw(raw) {
        return None;
    }
    let n = crate::paths::normalize_path(raw);
    if n.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for part in n.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            if out.pop().is_none() {
                return None;
            }
            continue;
        }
        if part.contains(':') {
            return None;
        }
        out.push(part.to_string());
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Parent-subset: child path must equal or sit under a claimed parent path.
/// A wider child path, a `..` walk out of the claim, or an empty path is
/// scope amplification and is refused.
pub fn path_within_parent_scope(child: &str, parent_scope: &str) -> bool {
    let Some(c) = collapse_lease_path(child) else {
        return false;
    };
    let Some(p) = collapse_lease_path(parent_scope) else {
        return false;
    };
    c.starts_with(p.as_slice())
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
    fn map_is_not_a_kernel_copy() {
        assert!(!COPIES_KERNEL);
        assert!(!BOUNCE_IS_KERNEL_TYPE);
        assert_eq!(AASM_PIN, "0.56.1");
        match EvidenceClass::Unknown {
            EvidenceClass::Inconclusive
            | EvidenceClass::InformationGap
            | EvidenceClass::Unknown
            | EvidenceClass::Pass => {}
        }
    }

    #[test]
    fn look_only_dictionary_maps_shop_words() {
        let bounce = map_shop_word("bounce").expect("bounce is a shop word");
        assert!(bounce.contains("backjump"), "{bounce}");
        assert!(!bounce
            .to_ascii_lowercase()
            .contains("kernel type named bounce"));
        assert_eq!(map_shop_word("WAIT"), Some(WAIT_CLASS_NOTE));
        assert_eq!(map_shop_word("held"), Some(LEASE_NOTE));
        assert_eq!(map_shop_word("parent_open"), Some(LEASE_NOTE));
        assert_eq!(map_shop_word("reduce"), Some(REDUCE_NOT_RECOMBINE));
        assert!(map_shop_word("reduce")
            .unwrap()
            .contains("not AASM recombine"));
        assert_eq!(map_shop_word("verify pass"), Some(VERIFY_PASS_NOTE));
        assert_eq!(map_shop_word("merge ack"), Some(MERGE_ACK_NOTE));
        assert_eq!(map_shop_word("invented-lane"), None);
        assert_eq!(
            WAIT_CLASSES,
            [
                EvidenceClass::Inconclusive,
                EvidenceClass::InformationGap,
                EvidenceClass::Unknown,
            ]
        );
        for class in WAIT_CLASSES {
            assert!(class.is_shop_wait());
            assert!(class.is_wait_class());
            assert_ne!(class, EvidenceClass::Pass);
            assert_eq!(class.to_shop_status(), EvidenceStatus::Wait);
        }
    }

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
        assert_eq!(
            classify_verify(EvidenceStatus::Pass, Some(0), "no verify command recorded"),
            EvidenceClass::InformationGap
        );
        assert_eq!(
            classify_verify(
                EvidenceStatus::Pass,
                Some(0),
                "mandatory child still ASSIGNED (obligation open)"
            ),
            EvidenceClass::InformationGap
        );
        assert_eq!(
            classify_verify(EvidenceStatus::Pass, Some(0), "mailbox reply / checks pass"),
            EvidenceClass::InformationGap
        );
        assert_eq!(
            classify_verify(EvidenceStatus::Pass, Some(0), "All checks have passed"),
            EvidenceClass::InformationGap
        );
        assert_eq!(
            classify_verify(EvidenceStatus::Pass, Some(0), "merge ack recorded"),
            EvidenceClass::InformationGap
        );
        assert_eq!(
            classify_verify(EvidenceStatus::Pass, Some(0), "claimed AASM recombine"),
            EvidenceClass::InformationGap
        );
        assert_eq!(
            classify_verify(EvidenceStatus::Wait, Some(0), ""),
            EvidenceClass::Unknown
        );
        assert_eq!(
            classify_verify(EvidenceStatus::Wait, Some(0), "ok"),
            EvidenceClass::Unknown
        );
        assert_eq!(
            classify_verify(EvidenceStatus::Pass, Some(0), ""),
            EvidenceClass::Pass
        );
        assert_eq!(
            classify_verify(EvidenceStatus::Pass, None, ""),
            EvidenceClass::Unknown
        );
        assert_eq!(
            classify_verify(EvidenceStatus::Pass, Some(1), "ok"),
            EvidenceClass::Inconclusive
        );
        assert_eq!(
            classify_verify(EvidenceStatus::Pass, Some(0), "WAIT"),
            EvidenceClass::InformationGap
        );
        assert_eq!(
            classify_verify(EvidenceStatus::Pass, Some(0), "UNKNOWN"),
            EvidenceClass::Unknown
        );
        assert_eq!(
            classify_verify(EvidenceStatus::Pass, Some(0), "INCONCLUSIVE"),
            EvidenceClass::Inconclusive
        );
        assert_eq!(
            classify_verify(EvidenceStatus::Pass, Some(0), "incomplete evidence"),
            EvidenceClass::InformationGap
        );
        assert_eq!(
            classify_verify(EvidenceStatus::Pass, Some(0), "VERIFIED"),
            EvidenceClass::InformationGap
        );
        assert_eq!(
            classify_verify(EvidenceStatus::Pass, Some(0), "bounce / backjump"),
            EvidenceClass::InformationGap
        );
        assert_ne!(
            classify_verify(EvidenceStatus::Pass, Some(0), "WAIT"),
            EvidenceClass::Pass
        );
        assert_ne!(
            classify_verify(EvidenceStatus::Pass, None, "missing exit"),
            EvidenceClass::Pass
        );
        assert_eq!(
            EvidenceClass::InformationGap.to_shop_status(),
            EvidenceStatus::Wait
        );
        assert_eq!(EvidenceClass::Pass.to_shop_status(), EvidenceStatus::Pass);
    }

    #[test]
    fn parent_subset_refuses_amplification() {
        assert!(path_within_parent_scope("src/a", "src"));
        assert!(path_within_parent_scope("src/a", "src/a"));
        assert!(path_within_parent_scope("src/a/./x", "src/a"));
        assert!(!path_within_parent_scope("src", "src/a"));
        assert!(!path_within_parent_scope("other", "src"));
        assert!(!path_within_parent_scope("src/foo", "src/foobar"));
        assert!(!path_within_parent_scope("src/a/../../secret", "src/a"));
        assert!(!path_within_parent_scope("src/a/../b", "src/a"));
        assert!(!path_within_parent_scope("../src/a", "src/a"));
        assert!(!path_within_parent_scope("C:/src/a", "src/a"));
        assert!(!path_within_parent_scope("/src/a", "src"));
        assert!(!path_within_parent_scope("\\src\\a", "src"));
        assert!(!path_within_parent_scope("//src/a", "src"));
        assert!(!path_within_parent_scope("src/a\0x", "src/a"));
        assert_eq!(
            paths_within_parent_scope(&["src/c".into()], &["src/a".into(), "src/b".into()]),
            Some("src/c".into())
        );
        assert_eq!(
            paths_within_parent_scope(&["src/a".into()], &[]),
            None,
            "empty parent claim: children define claims (shop open-without-scope)"
        );
        assert_eq!(
            paths_within_parent_scope(&["src/a/x".into()], &["src/a".into()]),
            None
        );
        assert_eq!(
            paths_within_parent_scope(&["src/a/../c".into()], &["src/a".into()]),
            Some("src/a/../c".into())
        );
    }
}
