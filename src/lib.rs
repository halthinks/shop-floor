//! `shop` — thin shop-floor hold-split-join sequencer.
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

pub mod cli;
pub mod engine;
pub mod error;
pub mod hash;
pub mod mailbox;
pub mod paths;
pub mod store;
pub mod types;

pub use engine::Shop;
pub use error::{Result, ShopError};
pub use mailbox::{AssignRecord, CLAIM_CEILING, DEFAULT_FROM, SCHEMA_VERSION};
pub use types::{
    Child, ChildState, Claim, EvidenceStatus, Parent, ParentState, ReducePackage, VerifyRecord,
};
