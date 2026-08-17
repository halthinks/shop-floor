//! Shop is the AASM proving ground. Not a second AASM.
//!
//! AASM stays the kernel. This crate hits the laws on a real floor until they
//! break. It does not copy the reducer, the calculus, or the effect plane.
//!
//! Contact surface: `tests/aasm_proving.rs`.
//! Helpers here are predicates for that surface. They are not a second kernel.

use crate::aasm_map::{self, EvidenceClass};
use crate::types::{
    Child, ChildState, EvidenceStatus, Parent, ParentState, ReducePackage, VerifyRecord,
};

pub const KERNEL: &str = "AASM";
pub const CONTACT: &str = "tests/aasm_proving.rs";
pub const MAP: &str = "src/aasm_map.rs";

/// Recorded verify PASS: parent VERIFIED, evidence-plane PASS, exit 0.
pub fn recorded_verify_pass(parent: &Parent) -> bool {
    parent.state == ParentState::Verified
        && parent.verify.as_ref().is_some_and(|v| {
            v.status == EvidenceStatus::Pass
                && v.class == EvidenceClass::Pass
                && v.exit_code == Some(0)
        })
}

/// Law 3: a mandatory ASSIGNED child blocks complete/close.
pub fn assigned_child_open(parent: &Parent) -> bool {
    parent
        .children
        .values()
        .any(|c| c.state == ChildState::Assigned)
}

/// Close only from VERIFIED + recorded PASS + no ASSIGNED child.
pub fn close_allowed(parent: &Parent) -> bool {
    recorded_verify_pass(parent) && !assigned_child_open(parent)
}

/// Law 1: handoff / mailbox / GitHub check cannot close. Information is not authority.
pub fn information_cannot_close(parent: &Parent) -> bool {
    !close_allowed(parent)
}

/// Law 2: WAIT / gap / inconclusive / unknown is never PASS or VERIFIED.
pub fn wait_is_not_verified(record: &VerifyRecord) -> bool {
    if record.class.is_shop_wait() || record.status == EvidenceStatus::Wait {
        record.class != EvidenceClass::Pass && record.status != EvidenceStatus::Pass
    } else {
        record.class == EvidenceClass::Pass && record.status == EvidenceStatus::Pass
    }
}

/// A parent sitting on missing or WAIT evidence must not be VERIFIED or CLOSED.
pub fn parent_wait_is_not_verified(parent: &Parent) -> bool {
    match &parent.verify {
        None => parent.state != ParentState::Verified && parent.state != ParentState::Closed,
        Some(rec) if rec.class.is_shop_wait() || rec.status == EvidenceStatus::Wait => {
            parent.state != ParentState::Verified && parent.state != ParentState::Closed
        }
        Some(_) => true,
    }
}

/// Law 4: bounce keeps the same peer and paths (causal backjump, not undo).
pub fn bounce_keeps_peer_paths(before: &Child, after: &Child) -> bool {
    after.peer == before.peer && after.paths == before.paths
}

/// Law 4: JOINED siblings survive a bounce of `bounced_id`.
pub fn joined_siblings_survive(before: &Parent, after: &Parent, bounced_id: &str) -> bool {
    before.children.iter().all(|(id, child)| {
        if id == bounced_id || child.state != ChildState::Joined {
            return true;
        }
        after.children.get(id).is_some_and(|kept| {
            kept.state == ChildState::Joined
                && kept.peer == child.peer
                && kept.paths == child.paths
        })
    })
}

/// Law 5: child paths stay inside the parent claim set.
/// Empty parent scope is unconstrained here (overlap is a different engine check).
pub fn child_paths_inside_parent_claim(child_paths: &[String], parent_scope: &[String]) -> bool {
    aasm_map::paths_within_parent_scope(child_paths, parent_scope).is_none()
}

/// Law 6: reduce package is shop evidence, not an AASM recombine claim.
pub fn reduce_is_not_recombine(pkg: &ReducePackage) -> bool {
    if !pkg.aasm_note.contains("not AASM recombine") {
        return false;
    }
    match serde_json::to_value(pkg) {
        Ok(v) => v.get("recombine").is_none(),
        Err(_) => false,
    }
}

/// Law 7: merge ACK / `github_merged` is command success, not CLOSED.
pub fn merge_ack_is_not_achieved_state(parent: &Parent) -> bool {
    if parent.github_merged {
        parent.state != ParentState::Closed
    } else {
        true
    }
}

/// Merge ACK must not be what set VERIFIED. VERIFIED requires recorded verify PASS.
pub fn merge_ack_does_not_verify(parent: &Parent) -> bool {
    if parent.state == ParentState::Verified {
        recorded_verify_pass(parent)
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_rec(lines: &str) -> VerifyRecord {
        VerifyRecord {
            cmd: String::new(),
            exit_code: None,
            last_lines: lines.into(),
            status: EvidenceStatus::Wait,
            class: EvidenceClass::InformationGap,
            aasm_note: aasm_map::VERIFY_GAP_NOTE.into(),
            recorded_at: 1,
        }
    }

    fn pass_rec() -> VerifyRecord {
        VerifyRecord {
            cmd: "true".into(),
            exit_code: Some(0),
            last_lines: String::new(),
            status: EvidenceStatus::Pass,
            class: EvidenceClass::Pass,
            aasm_note: aasm_map::VERIFY_PASS_NOTE.into(),
            recorded_at: 1,
        }
    }

    #[test]
    fn information_and_wait_cannot_close_or_verify() {
        let mut p = Parent::new("P".into(), "t".into(), "b".into());
        p.children.insert(
            "c1".into(),
            Child::new(
                "c1".into(),
                "alice".into(),
                vec!["src/a".into()],
                "t".into(),
                "b".into(),
            ),
        );
        assert!(assigned_child_open(&p));
        assert!(information_cannot_close(&p));
        assert!(!close_allowed(&p));
        assert!(parent_wait_is_not_verified(&p));

        p.verify = Some(wait_rec("no verify command recorded"));
        p.state = ParentState::VerifyWait;
        assert!(wait_is_not_verified(p.verify.as_ref().unwrap()));
        assert!(parent_wait_is_not_verified(&p));
        assert!(!recorded_verify_pass(&p));
    }

    #[test]
    fn assigned_child_blocks_even_when_parent_is_marked_verified() {
        let mut p = Parent::new("P".into(), "t".into(), "b".into());
        p.state = ParentState::Verified;
        p.verify = Some(pass_rec());
        p.children.insert(
            "c1".into(),
            Child::new(
                "c1".into(),
                "alice".into(),
                vec!["src/a".into()],
                "t".into(),
                "b".into(),
            ),
        );
        assert!(assigned_child_open(&p));
        assert!(!close_allowed(&p));
    }

    #[test]
    fn bounce_keeps_peer_paths_and_joined_siblings() {
        let mut before = Parent::new("P".into(), "t".into(), "b".into());
        let mut c1 = Child::new(
            "c1".into(),
            "alice".into(),
            vec!["src/a".into()],
            "t".into(),
            "b".into(),
        );
        let mut c2 = Child::new(
            "c2".into(),
            "bob".into(),
            vec!["src/b".into()],
            "t".into(),
            "b".into(),
        );
        c2.state = ChildState::Joined;
        before.children.insert("c1".into(), c1.clone());
        before.children.insert("c2".into(), c2);

        let mut after = before.clone();
        c1.state = ChildState::Bounced;
        c1.backjump_note = Some(aasm_map::BACKJUMP_NOTE.into());
        after.children.insert("c1".into(), c1.clone());

        assert!(bounce_keeps_peer_paths(
            before.children.get("c1").unwrap(),
            after.children.get("c1").unwrap()
        ));
        assert!(joined_siblings_survive(&before, &after, "c1"));
    }

    #[test]
    fn child_paths_and_reduce_and_merge_ack() {
        assert!(child_paths_inside_parent_claim(
            &["src/a".into()],
            &["src/a".into(), "src/b".into()]
        ));
        assert!(!child_paths_inside_parent_claim(
            &["src/c".into()],
            &["src/a".into(), "src/b".into()]
        ));

        let pkg = ReducePackage {
            parent_id: "P".into(),
            note: "pkg".into(),
            state: ParentState::Reduced,
            children: vec![],
            github_repo: None,
            child_branches: vec![],
            pull_request_url: None,
            pull_request_number: None,
            pull_request_status: EvidenceStatus::Wait,
            pull_request_reason: None,
            aasm_note: aasm_map::REDUCE_NOT_RECOMBINE.into(),
        };
        assert!(reduce_is_not_recombine(&pkg));

        let mut p = Parent::new("P".into(), "t".into(), "b".into());
        p.state = ParentState::Verified;
        p.verify = Some(pass_rec());
        p.github_merged = true;
        p.github_merge_reason = Some(aasm_map::MERGE_ACK_NOTE.into());
        assert!(merge_ack_is_not_achieved_state(&p));
        assert!(merge_ack_does_not_verify(&p));
        assert!(close_allowed(&p));
        p.state = ParentState::Closed;
        assert!(!merge_ack_is_not_achieved_state(&p));
    }
}
