//! AASM proving-ground contact surface. Must fail closed.
//!
//! Shop is the floor. AASM is the kernel. These tests are not a second AASM.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use shop::aasm_map::MERGE_ACK_NOTE;
use shop::error::ShopError;
use shop::types::{ChildState, EvidenceStatus, ParentState, VerifyRecord};
use shop::{EvidenceClass, MockGitHub, Shop};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp_store() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("shop-prove-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn fresh() -> (Shop, PathBuf) {
    let dir = tmp_store();
    (Shop::init(&dir).unwrap(), dir)
}

fn enroll(shop: &mut Shop, peers: &[&str]) {
    for p in peers {
        shop.add_worker(p, p, "grok-bot", "", true).unwrap();
    }
}

fn ready_two(shop: &mut Shop) {
    enroll(shop, &["alice", "bob"]);
    shop.open("P", "parent", "body").unwrap();
    shop.split("P", "c1", "alice", "src/a", "one", "a").unwrap();
    shop.split("P", "c2", "bob", "src/b", "two", "b").unwrap();
    shop.accept("P", "c1", r#"{"status":"PASS"}"#).unwrap();
    shop.accept("P", "c2", r#"{"status":"DONE"}"#).unwrap();
}

#[test]
fn handoff_mailbox_github_check_cannot_close_parent() {
    let dir = tmp_store();
    let mailbox = dir.join("bridge");
    fs::create_dir_all(mailbox.join("inbox/shop")).unwrap();
    let mock = Arc::new(MockGitHub::authed().with_checks_pass(true));
    let mut shop = Shop::init(dir.join("store"))
        .unwrap()
        .with_mailbox(Some(mailbox.clone()))
        .with_github(mock);
    enroll(&mut shop, &["alice"]);
    shop.github_connect("acme/widget", Some("main"), None)
        .unwrap();
    shop.open("P", "t", "b").unwrap();
    shop.split("P", "c1", "alice", "src/a", "t", "b").unwrap();
    shop.accept("P", "c1", r#"{"status":"PASS"}"#).unwrap();
    let _ = shop.github_skills().invoke(
        "pulls.read",
        serde_json::json!({
            "owner": "acme",
            "repo": "widget",
            "pullNumber": 1,
            "method": "get_check_runs",
        }),
    );
    fs::write(
        mailbox.join("inbox/shop/r.json"),
        r#"{"type":"steer-reply","from":"cursor-groksuperheavy","body":"checks passed"}"#,
    )
    .unwrap();
    shop.ingest_replies().unwrap();
    let p = shop.state().unwrap().parents["P"].clone();
    assert_ne!(p.state, ParentState::Closed, "information is not authority");
    assert_ne!(p.state, ParentState::Verified);
    assert!(matches!(
        shop.close("P").unwrap_err(),
        ShopError::CannotClose(_)
    ));
}

#[test]
fn missing_evidence_is_gap_never_verified() {
    let (mut shop, _dir) = fresh();
    ready_two(&mut shop);
    shop.join("P").unwrap();
    shop.reduce("P", "pkg").unwrap();
    let rec = shop.verify("P", None).unwrap();
    assert_eq!(rec.status, EvidenceStatus::Wait);
    assert!(rec.class.is_shop_wait());
    assert_ne!(rec.class, EvidenceClass::Pass);
    assert!(matches!(
        rec.class,
        EvidenceClass::InformationGap | EvidenceClass::Inconclusive | EvidenceClass::Unknown
    ));
    assert_ne!(
        shop.state().unwrap().parents["P"].state,
        ParentState::Verified
    );
    let rec = shop.verify("P", Some("false")).unwrap();
    assert_eq!(rec.status, EvidenceStatus::Wait);
    assert!(rec.class.is_shop_wait());
    assert_ne!(
        shop.state().unwrap().parents["P"].state,
        ParentState::Verified
    );
}

#[test]
fn cannot_complete_parent_with_assigned_child() {
    let (mut shop, dir) = fresh();
    enroll(&mut shop, &["alice"]);
    shop.open("P", "t", "b").unwrap();
    shop.split("P", "c1", "alice", "src/a", "t", "b").unwrap();
    let mut state = shop.state().unwrap();
    {
        let p = state.parents.get_mut("P").unwrap();
        p.state = ParentState::Verified;
        p.verify = Some(
            VerifyRecord {
                cmd: "true".into(),
                exit_code: Some(0),
                last_lines: String::new(),
                status: EvidenceStatus::Pass,
                class: EvidenceClass::Pass,
                aasm_note: String::new(),
                recorded_at: 1,
            }
            .classify(),
        );
    }
    fs::write(
        dir.join("snapshot.json"),
        serde_json::to_vec_pretty(&state).unwrap(),
    )
    .unwrap();
    drop(shop);
    let mut shop = Shop::open_store(&dir).unwrap();
    assert_eq!(
        shop.state().unwrap().parents["P"].children["c1"].state,
        ChildState::Assigned
    );
    assert!(matches!(
        shop.close("P").unwrap_err(),
        ShopError::ObligationOpen(_)
    ));
}

#[test]
fn bounce_is_backjump_siblings_survive() {
    let (mut shop, _dir) = fresh();
    enroll(&mut shop, &["alice", "bob"]);
    shop.open("P", "t", "b").unwrap();
    shop.split("P", "c1", "alice", "src/a", "t", "b").unwrap();
    shop.split("P", "c2", "bob", "src/b", "t", "b").unwrap();
    shop.accept("P", "c2", r#"{"status":"DONE"}"#).unwrap();
    let bounced = shop.bounce("P", "c1", "conflict").unwrap();
    assert_eq!(bounced.peer, "alice");
    assert_eq!(bounced.paths, vec!["src/a"]);
    assert!(bounced
        .backjump_note
        .as_deref()
        .unwrap_or("")
        .contains("backjump"));
    let p = shop.state().unwrap().parents["P"].clone();
    assert_eq!(p.children["c2"].state, ChildState::Joined);
    assert_eq!(p.children["c2"].paths, vec!["src/b"]);
    assert_eq!(p.children["c1"].state, ChildState::Bounced);
}

#[test]
fn child_paths_stay_inside_parent_claim_set() {
    let (mut shop, _dir) = fresh();
    enroll(&mut shop, &["alice", "bob"]);
    shop.open_with_scope("P", "t", "b", "src/a,src/b").unwrap();
    assert!(matches!(
        shop.split("P", "c1", "alice", "src/c", "t", "b")
            .unwrap_err(),
        ShopError::ParentSubset { .. }
    ));
    shop.split("P", "c1", "alice", "src/a", "t", "b").unwrap();
    assert!(matches!(
        shop.split("P", "c2", "bob", "src/a", "t", "b").unwrap_err(),
        ShopError::PathOverlap { .. }
    ));
}

#[test]
fn reduce_does_not_claim_aasm_recombine() {
    let (mut shop, _dir) = fresh();
    ready_two(&mut shop);
    shop.join("P").unwrap();
    let pkg = shop.reduce("P", "pkg").unwrap();
    let v = serde_json::to_value(&pkg).unwrap();
    assert!(v.get("recombine").is_none());
    assert!(pkg.aasm_note.contains("not AASM recombine"));
}

#[test]
fn merge_ack_is_not_achieved_state() {
    let mock = Arc::new(
        MockGitHub::authed()
            .with_create_pr(true)
            .with_create_branch(true)
            .with_checks_pass(true),
    );
    let dir = tmp_store();
    let mut shop = Shop::init(&dir).unwrap().with_github(mock.clone());
    enroll(&mut shop, &["alice"]);
    shop.github_connect("acme/widget", Some("main"), None)
        .unwrap();
    shop.open("P", "t", "b").unwrap();
    shop.split("P", "c1", "alice", "a", "t", "b").unwrap();
    shop.accept("P", "c1", r#"{"status":"PASS"}"#).unwrap();
    shop.join("P").unwrap();
    shop.reduce("P", "note").unwrap();
    assert_eq!(
        shop.state().unwrap().parents["P"].state,
        ParentState::Reduced
    );
    assert!(matches!(
        shop.merge_verified("P").unwrap_err(),
        ShopError::Wait(_)
    ));
    assert_ne!(
        shop.state().unwrap().parents["P"].state,
        ParentState::Verified,
        "merge ACK must not auto-VERIFIED"
    );
    let rec = shop.verify("P", Some("true")).unwrap();
    assert_eq!(rec.status, EvidenceStatus::Pass);
    shop.merge_verified("P").unwrap();
    let p = shop.state().unwrap().parents["P"].clone();
    assert!(p.github_merged);
    assert_eq!(p.state, ParentState::Verified);
    assert_ne!(p.state, ParentState::Closed);
    assert_eq!(p.github_merge_reason.as_deref(), Some(MERGE_ACK_NOTE));
}
