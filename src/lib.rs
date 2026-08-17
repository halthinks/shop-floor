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

pub use aasm_map::EvidenceClass;
pub use awareness::Awareness;
pub use engine::Shop;
pub use error::{Result, ShopError};
pub use floor::FloorMemory;
pub use github::GitHubRepo;
pub use mailbox::{
    AssignRecord, MailboxObservation, CLAIM_CEILING, DEFAULT_FROM, SCHEMA_VERSION, STEER_TO,
};
pub use memory::{MemoryTier, ShopMemory};
pub use procwait::{
    status_pill, LiveProcess, PidLiveness, ProcWaitLedger, StopRecord, WaitEvidence,
};
pub use skills::{GitHubSkills, NullGitHub, SKILL_PACK};
pub use skills_mock::MockGitHub;
pub use types::{
    Child, ChildState, Claim, EvidenceStatus, Parent, ParentState, ReducePackage, VerifyRecord,
};
pub use workers::{Backend, Intelligence, Worker};
