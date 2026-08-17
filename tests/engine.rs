//! Engine fail-closed: missing handoff evidence is WAIT, never a fake PASS.
//! Unread assign is HOLD. A still-ASSIGNED child cannot be treated as PASS.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use shop::error::ShopError;
use shop::mailbox::{AssignRecord, ASSIGN_TYPE, DEFAULT_FROM, HOLD_UNREAD};
use shop::types::{ChildState, EvidenceStatus, ParentState, VerifyRecord};
use shop::{EvidenceClass, Shop};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp_store() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("shop-engine-{}-{n}", std::process::id()));
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

fn assigned_child(shop: &mut Shop) {
    enroll(shop, &["alice"]);
    shop.open("P", "t", "b").unwrap();
    shop.split("P", "c1", "alice", "src/a", "t", "b").unwrap();
}

fn assert_accept_refuses(shop: &mut Shop, spec: &str) {
    let err = shop
        .accept("P", "c1", spec)
        .expect_err("incomplete handoff must refuse accept");
    match err {
        ShopError::IncompleteEvidence(msg) => {
            assert!(
                msg.contains("WAIT") || msg.contains("HOLD") || msg.contains("empty"),
                "refuse must stay WAIT/HOLD, got {msg}"
            );
            assert!(
                !msg.contains("PASS") || msg.contains("invent") || msg.contains("fake"),
                "error must not claim PASS: {msg}"
            );
        }
        other => panic!("expected IncompleteEvidence, got {other}"),
    }
    let child = shop.state().unwrap().parents["P"].children["c1"].clone();
    assert_eq!(child.state, ChildState::Assigned);
    assert!(child.handoff.is_none());
}

#[test]
fn handoff_without_status_refuses_accept_and_does_not_invent_pass() {
    let (mut shop, _dir) = fresh();
    assigned_child(&mut shop);

    assert_accept_refuses(&mut shop, r#"{"note":"done"}"#);
    assert_accept_refuses(&mut shop, r#"{}"#);
    assert_accept_refuses(&mut shop, r#"{"status":"WAIT"}"#);
    assert_accept_refuses(&mut shop, r#"{"result":"FAIL"}"#);
    assert_accept_refuses(&mut shop, "not-json");
    assert_accept_refuses(&mut shop, "");

    assert_eq!(shop.join("P").unwrap(), ParentState::InFlight);
    assert!(matches!(
        shop.close("P").unwrap_err(),
        ShopError::ObligationOpen(_)
    ));
}

#[test]
fn unread_assign_json_is_hold_not_pass() {
    let (mut shop, _dir) = fresh();
    assigned_child(&mut shop);
    let rec = AssignRecord::from_child(
        DEFAULT_FROM,
        &shop.state().unwrap().parents["P"].children["c1"],
    );
    let mut v = serde_json::to_value(&rec).unwrap();
    v["type"] = serde_json::json!(ASSIGN_TYPE);
    // Even a stuffed PASS on an assign record is HOLD, not accept evidence.
    v["status"] = serde_json::json!("PASS");
    let spec = serde_json::to_string(&v).unwrap();

    let err = shop.accept("P", "c1", &spec).unwrap_err();
    match err {
        ShopError::IncompleteEvidence(msg) => {
            assert_eq!(msg, HOLD_UNREAD);
            assert!(msg.contains("HOLD"));
            assert!(msg.contains("not PASS"));
        }
        other => panic!("unread assign must be HOLD, got {other}"),
    }
    let child = shop.state().unwrap().parents["P"].children["c1"].clone();
    assert_eq!(child.state, ChildState::Assigned);
    assert_eq!(shop.join("P").unwrap(), ParentState::InFlight);
}

#[test]
fn explicit_pass_and_done_still_accept() {
    let (mut shop, _dir) = fresh();
    enroll(&mut shop, &["alice", "bob"]);
    shop.open("P", "t", "b").unwrap();
    shop.split("P", "c1", "alice", "src/a", "t", "b").unwrap();
    shop.split("P", "c2", "bob", "src/b", "t", "b").unwrap();

    let pass = shop.accept("P", "c1", r#"{"status":"PASS"}"#).unwrap();
    assert_eq!(pass.status, "PASS");
    let done = shop.accept("P", "c2", r#"{"result":"done"}"#).unwrap();
    assert_eq!(done.status, "DONE");
    assert_eq!(shop.join("P").unwrap(), ParentState::ReduceReady);
}

#[test]
fn assigned_child_blocks_join_verify_and_close() {
    let (mut shop, dir) = fresh();
    assigned_child(&mut shop);

    assert_eq!(shop.join("P").unwrap(), ParentState::InFlight);
    assert!(matches!(
        shop.reduce("P", "nope").unwrap_err(),
        ShopError::JoinBarrier
    ));
    assert!(matches!(
        shop.verify("P", Some("true")).unwrap_err(),
        ShopError::InvalidParentState { .. }
    ));
    assert!(matches!(
        shop.close("P").unwrap_err(),
        ShopError::ObligationOpen(_)
    ));

    // Tamper: parent marked VERIFIED while the child is still ASSIGNED.
    // Close must stay ObligationOpen. Verify must demote PASS to WAIT.
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

    let rec = shop.verify("P", Some("true")).unwrap();
    assert_eq!(rec.status, EvidenceStatus::Wait);
    assert_ne!(rec.class, EvidenceClass::Pass);
    assert_eq!(
        shop.state().unwrap().parents["P"].state,
        ParentState::VerifyWait
    );
    assert!(matches!(
        shop.close("P").unwrap_err(),
        ShopError::ObligationOpen(_)
    ));
}
