//! Required shop-floor gates. Incomplete evidence is WAIT, never fake PASS.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use shop::error::ShopError;
use shop::mailbox::{AssignRecord, CLAIM_CEILING, DEFAULT_FROM, SCHEMA_VERSION};
use shop::skills::GitHubSkills;
use shop::types::{Child, ChildState, EvidenceStatus, ParentState};
use shop::ui::{self, PAGE};
use shop::{MockGitHub, Shop, SKILL_PACK};

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

fn enroll(shop: &mut Shop, peers: &[&str]) {
    for p in peers {
        shop.add_worker(p, p, "grok-bot", "", true).unwrap();
    }
}

fn ready_two_children(shop: &mut Shop) {
    enroll(shop, &["alice", "bob"]);
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
    enroll(&mut shop, &["alice", "bob", "carol"]);
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
    enroll(&mut shop, &["alice", "bob"]);
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
    enroll(&mut shop, &["alice"]);
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
    enroll(&mut shop, &["alice", "bob"]);
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
    enroll(&mut shop, &["alice"]);
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

#[test]
fn add_worker_and_assign_intelligence_via_library() {
    let (mut shop, dir) = fresh();
    let w = shop
        .add_worker("alice", "Alice", "cursor-ultra", "my-ultra-id", true)
        .unwrap();
    assert_eq!(w.peer, "alice");
    assert_eq!(w.intelligence.backend.as_str(), "cursor-ultra");
    assert_eq!(w.intelligence.model, "my-ultra-id");
    assert!(w.github_skills);
    assert!(dir.join("workers.json").is_file());

    let w = shop
        .set_worker_intelligence("alice", "grok-bot", "")
        .unwrap();
    assert_eq!(w.intelligence.backend.as_str(), "grok-bot");
    assert_eq!(w.intelligence.model, "");
}

#[test]
fn reject_duplicate_peer() {
    let (mut shop, _dir) = fresh();
    shop.add_worker("alice", "A", "cursor", "m", true).unwrap();
    let err = shop
        .add_worker("alice", "B", "grok-bot", "", true)
        .unwrap_err();
    assert!(matches!(err, ShopError::DuplicatePeer(p) if p == "alice"));
}

#[test]
fn reject_unknown_backend() {
    let (mut shop, _dir) = fresh();
    let err = shop
        .add_worker("alice", "A", "gpt-99", "x", true)
        .unwrap_err();
    assert!(matches!(err, ShopError::UnknownBackend(b) if b == "gpt-99"));
}

#[test]
fn split_cannot_target_unknown_peer() {
    let (mut shop, _dir) = fresh();
    shop.open("P", "t", "b").unwrap();
    let err = shop
        .split("P", "c1", "ghost", "src/a", "t", "b")
        .unwrap_err();
    assert!(matches!(err, ShopError::UnknownPeer(p) if p == "ghost"));
}

#[test]
fn ui_page_serves_add_worker_form_and_intelligence_control() {
    assert!(PAGE.contains("id=\"add-worker\""));
    assert!(PAGE.contains("id=\"intelligence-backend\"") || PAGE.contains("name=\"backend\""));
    assert!(PAGE.contains("id=\"intelligence-model\"") || PAGE.contains("name=\"model\""));

    let (shop, _dir) = fresh();
    let (addr, _h) = ui::spawn(shop, 0).unwrap();
    let html = http_get(&format!("{}:{}", addr.ip(), addr.port()), "/");
    assert!(
        html.contains("id=\"add-worker\""),
        "served HTML must include the add-worker form"
    );
    assert!(
        html.contains("intelligence-backend") || html.contains("name=\"backend\""),
        "served HTML must include an intelligence control"
    );
    assert!(html.contains("github-panel") || html.contains("GitHub"));
}

#[test]
fn connect_repo_record_and_child_branch_name() {
    let (mut shop, dir) = fresh();
    enroll(&mut shop, &["alice"]);
    let repo = shop
        .github_connect("halthinks/shop-floor", Some("main"), None)
        .unwrap();
    assert_eq!(repo.slug(), "halthinks/shop-floor");
    assert!(dir.join("github.json").is_file());

    shop.open("P", "t", "b").unwrap();
    let child = shop.split("P", "c1", "alice", "src/a", "t", "b").unwrap();
    assert_eq!(child.branch.as_deref(), Some("shop/P/c1"));
    assert!(!child.branch_created);
}

#[test]
fn reduce_package_lists_repo_and_branches_without_faking_pr() {
    let (mut shop, _dir) = fresh();
    enroll(&mut shop, &["alice", "bob"]);
    shop.github_connect("acme/widget", Some("main"), None)
        .unwrap();
    shop.open("P", "t", "b").unwrap();
    shop.split("P", "c1", "alice", "a", "t", "b").unwrap();
    shop.split("P", "c2", "bob", "b", "t", "b").unwrap();
    shop.accept("P", "c1", r#"{"status":"PASS"}"#).unwrap();
    shop.accept("P", "c2", r#"{"status":"PASS"}"#).unwrap();
    shop.join("P").unwrap();
    let pkg = shop.reduce("P", "note").unwrap();
    assert_eq!(pkg.github_repo.as_deref(), Some("acme/widget"));
    assert!(pkg.child_branches.contains(&"shop/P/c1".into()));
    assert!(pkg.child_branches.contains(&"shop/P/c2".into()));
    assert!(pkg.pull_request_url.is_none());
    assert_eq!(pkg.pull_request_status, EvidenceStatus::Wait);
}

#[test]
fn verify_close_without_real_pr_stays_wait() {
    let (mut shop, _dir) = fresh();
    enroll(&mut shop, &["alice", "bob"]);
    shop.github_connect("acme/widget", Some("main"), None)
        .unwrap();
    shop.open("P", "t", "b").unwrap();
    shop.split("P", "c1", "alice", "a", "t", "b").unwrap();
    shop.split("P", "c2", "bob", "b", "t", "b").unwrap();
    shop.accept("P", "c1", r#"{"status":"PASS"}"#).unwrap();
    shop.accept("P", "c2", r#"{"status":"PASS"}"#).unwrap();
    shop.join("P").unwrap();
    shop.reduce("P", "note").unwrap();

    let rec = shop.verify("P", Some("true")).unwrap();
    assert_eq!(rec.status, EvidenceStatus::Wait);
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
fn github_skills_mock_never_hits_live_api() {
    let mock = MockGitHub::authed();
    let me = mock.invoke("me", json!({})).unwrap();
    assert_eq!(me["login"], "mock-user");
    assert!(SKILL_PACK.contains(&"pulls.merge"));
    let unauth = MockGitHub::unauth();
    assert!(matches!(
        unauth.invoke("me", json!({})).unwrap_err(),
        ShopError::Wait(_)
    ));
    let scan = unauth
        .invoke(
            "secret_scan",
            json!({"files": "hello ghp_abcdefghijklmnop"}),
        )
        .unwrap();
    assert_eq!(scan["clean"], false);
    assert!(!scan.to_string().contains("ghp_abcdefghijklmnop"));
}

#[test]
fn reduce_never_auto_merges() {
    let mock = Arc::new(MockGitHub::authed().with_create_pr(true));
    let dir = tmp_store();
    let mut shop = Shop::init(&dir).unwrap().with_github(mock.clone());
    enroll(&mut shop, &["alice"]);
    shop.github_connect("acme/widget", Some("main"), None)
        .unwrap();
    shop.open("P", "t", "b").unwrap();
    shop.split("P", "c1", "alice", "a", "t", "b").unwrap();
    shop.accept("P", "c1", r#"{"status":"PASS"}"#).unwrap();
    shop.join("P").unwrap();
    let pkg = shop.reduce("P", "note").unwrap();
    assert!(pkg.pull_request_url.is_some());
    assert!(!mock.calls().iter().any(|c| c == "pulls.merge"));
    assert!(!mock.merged());
}

fn http_get(addr: &str, path: &str) -> String {
    let mut last = String::new();
    for _ in 0..40 {
        if let Ok(mut s) = TcpStream::connect(addr) {
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
            if s.write_all(req.as_bytes()).is_ok() {
                let mut buf = Vec::new();
                let _ = s.read_to_end(&mut buf);
                last = String::from_utf8_lossy(&buf).into_owned();
                if last.contains("id=\"add-worker\"") || last.starts_with("HTTP/1.1 200") {
                    return last;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    last
}
