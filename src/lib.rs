//! `shop` — thin shop-floor hold-split-join sequencer with a local worker UI.
//!
//! This is not AASM and not T3 Code. AASM stays the authority calculus.
//! shop holds a parent job, splits finite children, joins, bounces failures
//! to the originating lane, reduces, and verifies. Incomplete evidence is
//! WAIT, never a fake PASS.
//!
//! Authority rules (enforced):
//! - A handoff is Evidence, not truth.
//! - A CLI/test exit 0 is Evidence, not authority to CLOSE unless `shop verify` recorded it.
//! - shop never invents a child lane. Only `shop split` creates children.
//! - shop never writes outside the shop store and optional mailbox outbox.
//!
//! Crate-root surface is fail-closed. [`WAIT`] / [`missing_evidence`] never
//! invent PASS, VERIFIED, or CLOSE. [`Handoff`] is Evidence
//! ([`HANDOFF_IS_EVIDENCE`]) and cannot CLOSE. Unread assign is HOLD
//! ([`HOLD_UNREAD`]). [`WaitEvidence`] stays visible. This file does not
//! invent a workers product.

pub mod aasm_map;
pub mod awareness;
pub mod cli;
pub mod engine;
pub mod error;
pub mod feed;
pub mod floor;
pub mod github;
pub mod hash;
pub mod mailbox;
pub mod memory;
pub mod paths;
pub mod procwait;
pub mod project;
pub mod proving;
pub mod replies;
pub mod runner;
pub mod secret_scan;
pub mod skills;
pub mod skills_live;
pub mod skills_mock;
pub mod steer;
pub mod store;
pub mod types;
pub mod ui;
pub mod workers;
pub mod worktree;

pub use aasm_map::EvidenceClass;
pub use awareness::Awareness;
pub use engine::Shop;
pub use error::{Result, ShopError};
pub use floor::FloorMemory;
pub use github::GitHubRepo;
pub use mailbox::{
    AssignRecord, MailboxObservation, CLAIM_CEILING, DEFAULT_FROM, HANDOFF_IS_EVIDENCE,
    HOLD_UNREAD, SCHEMA_VERSION, STEER_TO,
};
pub use memory::{MemoryTier, ShopMemory};
pub use procwait::{
    status_pill, LiveProcess, PidLiveness, ProcWaitLedger, StopRecord, WaitEvidence,
    WAIT_EVIDENCE_NOTE, WAIT_UNKNOWN_NOTE,
};
pub use skills::{GitHubSkills, NullGitHub, SKILL_PACK};
pub use skills_mock::MockGitHub;
pub use types::{
    Child, ChildState, Claim, EvidenceStatus, Handoff, Parent, ParentState, ReducePackage,
    VerifyRecord,
};
pub use workers::{Backend, Intelligence, Worker};

/// Crate-root shop word for missing evidence. Never PASS.
pub const WAIT: EvidenceStatus = EvidenceStatus::Wait;

/// Fail-closed crate-root status. Missing evidence is WAIT.
///
/// This surface does not mint PASS, VERIFIED, or CLOSE. A recorded verify
/// PASS stays on [`VerifyRecord`] after `shop verify`, not here. A stuffed
/// `"PASS"` word is still not authority.
pub fn missing_evidence() -> EvidenceStatus {
    EvidenceStatus::Wait
}

/// Fail-closed crate-root status from an optional recorded word.
///
/// `None` or anything other than an already-recorded [`EvidenceStatus::Wait`]
/// or [`EvidenceStatus::Pass`] cannot occur; `None` is WAIT. A `Some(Pass)`
/// is passed through as already-recorded evidence, not invented here.
pub fn recorded_or_wait(status: Option<EvidenceStatus>) -> EvidenceStatus {
    status.unwrap_or(WAIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_root_does_not_invent_pass_verified_or_close() {
        assert_eq!(WAIT, EvidenceStatus::Wait);
        assert_eq!(WAIT.to_string(), "WAIT");
        assert_eq!(missing_evidence(), EvidenceStatus::Wait);
        assert_eq!(missing_evidence().to_string(), "WAIT");
        assert_ne!(missing_evidence(), EvidenceStatus::Pass);
        assert_eq!(recorded_or_wait(None), EvidenceStatus::Wait);
        assert_eq!(
            recorded_or_wait(Some(EvidenceStatus::Wait)),
            EvidenceStatus::Wait
        );
        assert_eq!(
            recorded_or_wait(Some(EvidenceStatus::Pass)),
            EvidenceStatus::Pass,
            "already-recorded PASS is not invented at crate root"
        );
        assert_eq!(EvidenceStatus::default(), EvidenceStatus::Wait);
        assert_eq!(EvidenceClass::default(), EvidenceClass::Unknown);
        assert!(EvidenceClass::Unknown.is_shop_wait());
        assert!(EvidenceClass::Inconclusive.is_shop_wait());
        assert!(EvidenceClass::InformationGap.is_shop_wait());
        assert!(!EvidenceClass::Pass.is_shop_wait());
    }

    #[test]
    fn wait_types_stay_visible_on_crate_root() {
        let _wait: EvidenceStatus = WAIT;
        let _class_wait = EvidenceClass::Unknown;
        let _note = WAIT_EVIDENCE_NOTE;
        let _unknown = WAIT_UNKNOWN_NOTE;
        assert!(WAIT_EVIDENCE_NOTE.contains("Evidence"));
        assert!(
            WAIT_EVIDENCE_NOTE.contains("not VERIFIED") && WAIT_EVIDENCE_NOTE.contains("not CLOSE"),
            "wait note must not promote to VERIFIED/CLOSE: {WAIT_EVIDENCE_NOTE}"
        );
        assert!(WAIT_UNKNOWN_NOTE.contains("not PASS"));
        assert!(WAIT_UNKNOWN_NOTE.contains("not VERIFIED"));
        let _ledger_ty: Option<ProcWaitLedger> = None;
        let _wait_ev_name = std::any::type_name::<WaitEvidence>();
        assert!(
            _wait_ev_name.contains("WaitEvidence"),
            "WaitEvidence must stay on the crate root"
        );
    }

    #[test]
    fn handoff_is_evidence_and_cannot_close() {
        let handoff = Handoff {
            hash: "abc".into(),
            status: "PASS".into(),
            source: "unit".into(),
            recorded_at: 0,
        };
        assert_eq!(handoff.status, "PASS");
        assert!(HANDOFF_IS_EVIDENCE.contains("Evidence"));
        assert!(HANDOFF_IS_EVIDENCE.contains("cannot CLOSE"));
        assert!(
            HANDOFF_IS_EVIDENCE.contains("VERIFIED") && HANDOFF_IS_EVIDENCE.contains("cannot"),
            "handoff must not be authority to VERIFIED: {HANDOFF_IS_EVIDENCE}"
        );
        assert!(!HANDOFF_IS_EVIDENCE.contains("is PASS"));
        assert!(HOLD_UNREAD.contains("HOLD"));
        assert!(HOLD_UNREAD.contains("not PASS"));
        assert!(HOLD_UNREAD.contains("cannot CLOSE"));
        assert!(CLAIM_CEILING.contains("handoff is evidence"));
        assert!(CLAIM_CEILING.contains("no invented lanes"));
    }

    #[test]
    fn workers_reexport_is_not_a_new_product() {
        let _backend = Backend::GrokBot;
        let _intel = Intelligence {
            backend: Backend::GrokBot,
            model: String::new(),
        };
        let _worker = Worker {
            peer: "alice".into(),
            name: "Alice".into(),
            intelligence: _intel,
            github_skills: true,
        };
        assert_eq!(Backend::GrokBot.as_str(), "grok-bot");
    }
}
