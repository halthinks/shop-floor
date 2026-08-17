//! Required shop-floor gates. Incomplete evidence is WAIT, never fake PASS.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use shop::error::ShopError;
use shop::mailbox::{AssignRecord, CLAIM_CEILING, DEFAULT_FROM, SCHEMA_VERSION};
use shop::types::{Child, ChildState, EvidenceStatus, ParentState};
use shop::Shop;

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp_store() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("shop-test-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn fresh() -> (Shop, PathBuf) {
    let dir = tmp_store();
    (Shop::init(&dir).unwrap(), dir)
}

fn ready_two_children(shop: &mut Shop) {
    shop.open("P", "parent", "body").unwrap();
    shop.split("P", "c1", "alice", "src/a,src/b", "one", "do a")
        .unwrap();
    shop.split("P", "c2", "bob", "src/c", "two", "do c")
        .unwrap();
    shop.accept("P", "c1", r#"{"status":"PASS"}"#).unwrap();
    shop.accept("P", "c2", r#"{"status":"DONE"}"#).unwrap();
}

#[test]
fn overlapping_allowed_paths_on_split_is_rejected() {
    let (mut shop, _dir) = fresh();
    shop.open("P1", "t", "b").unwrap();
    shop.split("P1", "c1", "alice", "src/foo,src/extra", "t", "b")
        .unwrap();

    shop.open("P2", "t", "b").unwrap();
    let err = shop
        .split("P2", "c2", "bob", "src/foo/bar", "t", "b")
        .unwrap_err();
    match err {
        ShopError::PathOverlap {
            path,
            parent,
            child,
            existing,
        } => {
            assert_eq!(parent, "P1");
            assert_eq!(child, "c1");
            assert!(path.contains("src/foo") || existing.contains("src/foo"));
        }
        other => panic!("expected PathOverlap, got {other}"),
    }

    let err = shop
        .split("P1", "c3", "carol", "src/foo", "t", "b")
        .unwrap_err();
    assert!(matches!(err, ShopError::PathOverlap { .. }));
}

#[test]
fn join_barrier_cannot_reduce_until_all_children_joined() {
    let (mut shop, _dir) = fresh();
    shop.open("P", "t", "b").unwrap();
    shop.split("P", "c1", "alice", "a", "t", "b").unwrap();
    shop.split("P", "c2", "bob", "b", "t", "b").unwrap();
    shop.accept("P", "c1", r#"{"status":"PASS"}"#).unwrap();

    let state = shop.join("P").unwrap();
    assert_eq!(state, ParentState::InFlight);
    assert!(matches!(
        shop.reduce("P", "nope").unwrap_err(),
        ShopError::JoinBarrier
    ));

    shop.accept("P", "c2", r#"{"status":"PASS"}"#).unwrap();
    assert_eq!(shop.join("P").unwrap(), ParentState::ReduceReady);
    shop.reduce("P", "ok").unwrap();
}

#[test]
fn bounce_keeps_the_same_peer_and_paths() {
    let (mut shop, _dir) = fresh();
    shop.open("P", "t", "b").unwrap();
    shop.split("P", "c1", "alice", "src/lane,src/other", "t", "work")
        .unwrap();
    let bounced = shop.bounce("P", "c1", "failed closed").unwrap();
    assert_eq!(bounced.state, ChildState::Bounced);
    assert_eq!(bounced.peer, "alice");
    assert_eq!(bounced.paths, vec!["src/lane", "src/other"]);

    let recs = shop.assign("P").unwrap();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].to, "alice");
    assert_eq!(recs[0].allowed_paths, vec!["src/lane", "src/other"]);
    assert_eq!(recs[0].lane_id, "c1");

    let child = shop.state().unwrap().parents["P"].children["c1"].clone();
    assert_eq!(child.state, ChildState::Assigned);
    assert_eq!(child.peer, "alice");
    assert_eq!(child.paths, vec!["src/lane", "src/other"]);
}

#[test]
fn verify_without_successful_recorded_command_cannot_close() {
    let (mut shop, _dir) = fresh();
    ready_two_children(&mut shop);
    shop.join("P").unwrap();
    shop.reduce("P", "pkg").unwrap();

    assert!(matches!(
        shop.close("P").unwrap_err(),
        ShopError::CannotClose(_)
    ));

    let wait = shop.verify("P", None).unwrap();
    assert_eq!(wait.status, EvidenceStatus::Wait);
    assert_eq!(
        shop.state().unwrap().parents["P"].state,
        ParentState::VerifyWait
    );
    assert!(matches!(
        shop.close("P").unwrap_err(),
        ShopError::CannotClose(_)
    ));
}

#[test]
fn init_open_two_children_accept_join_reduce_verify_true_close() {
    let (mut shop, dir) = fresh();
    shop.open("P", "title", "body").unwrap();
    assert_eq!(shop.state().unwrap().parents["P"].state, ParentState::Held);
    assert!(shop.state().unwrap().parents["P"].children.is_empty());

    shop.split("P", "c1", "alice", "src/a", "one", "a").unwrap();
    shop.split("P", "c2", "bob", "src/b", "two", "b").unwrap();
    shop.assign("P").unwrap();
    assert!(dir.join("outbox/assign-P-c1.json").is_file());
    assert!(dir.join("outbox/assign-P-c2.json").is_file());

    shop.accept("P", "c1", r#"{"status":"PASS"}"#).unwrap();
    shop.accept("P", "c2", r#"{"status":"DONE"}"#).unwrap();
    assert_eq!(shop.join("P").unwrap(), ParentState::ReduceReady);

    let pkg = shop.reduce("P", "integrate").unwrap();
    assert_eq!(pkg.children.len(), 2);
    assert!(dir.join("reduce/P.json").is_file());
    assert_eq!(
        shop.state().unwrap().parents["P"].state,
        ParentState::Reduced
    );

    let rec = shop.verify("P", Some("true")).unwrap();
    assert_eq!(rec.status, EvidenceStatus::Pass);
    assert_eq!(rec.exit_code, Some(0));
    assert_eq!(
        shop.state().unwrap().parents["P"].state,
        ParentState::Verified
    );

    shop.close("P").unwrap();
    assert_eq!(
        shop.state().unwrap().parents["P"].state,
        ParentState::Closed
    );
}

#[test]
fn verify_false_stays_wait_not_closed() {
    let (mut shop, _dir) = fresh();
    ready_two_children(&mut shop);
    shop.join("P").unwrap();
    shop.reduce("P", "pkg").unwrap();

    let rec = shop.verify("P", Some("false")).unwrap();
    assert_eq!(rec.status, EvidenceStatus::Wait);
    assert_ne!(rec.exit_code, Some(0));
    let parent = shop.state().unwrap().parents["P"].clone();
    assert_eq!(parent.state, ParentState::VerifyWait);
    assert_ne!(parent.state, ParentState::Closed);
    assert!(matches!(
        shop.close("P").unwrap_err(),
        ShopError::CannotClose(_)
    ));
}

#[test]
fn assign_writes_outbox_when_mailbox_dir_missing() {
    let dir = tmp_store();
    let mailbox = dir.join("missing-mailbox");
    let mut shop = Shop::init(dir.join("store"))
        .unwrap()
        .with_mailbox(Some(mailbox.clone()));
    shop.open("P", "t", "b").unwrap();
    shop.split("P", "c1", "alice", "only", "t", "body").unwrap();
    shop.assign("P").unwrap();
    assert!(!mailbox.exists());
    assert!(dir.join("store/outbox/assign-P-c1.json").is_file());
}

#[test]
fn assign_record_json_shape() {
    let child = Child::new(
        "lane-1".into(),
        "peer-a".into(),
        vec!["src/a".into()],
        "t".into(),
        "work".into(),
    );
    let rec = AssignRecord::from_child(DEFAULT_FROM, &child);
    let v = serde_json::to_value(&rec).unwrap();
    assert_eq!(v["schema_version"], SCHEMA_VERSION);
    assert_eq!(v["type"], "assign");
    assert_eq!(v["from"], DEFAULT_FROM);
    assert_eq!(v["to"], "peer-a");
    assert_eq!(v["lane_id"], "lane-1");
    assert_eq!(v["allowed_paths"], serde_json::json!(["src/a"]));
    assert_eq!(v["body"], "work");
    assert_eq!(v["claim_ceiling"], CLAIM_CEILING);
}
