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
use shop::mailbox::STEER_TO;
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
fn ui_serves_command_center_structure() {
    assert!(PAGE.contains("id=\"command-center\""));
    assert!(PAGE.contains("id=\"workers\""));
    assert!(PAGE.contains("id=\"floor\""));
    assert!(PAGE.contains("id=\"github\""));
    assert!(PAGE.contains("id=\"mailbox\""));
    assert!(PAGE.contains("id=\"add-worker\""));
    assert!(PAGE.contains("id=\"intelligence-backend\""));
    assert!(PAGE.contains("Workers"));
    assert!(PAGE.contains("Current job"));
    assert!(PAGE.contains("GitHub"));
    assert!(PAGE.contains("Talk to the boss"));
    assert!(PAGE.contains("SuperGrokHeavy"));
    assert!(PAGE.contains("orchestrator"));
    assert!(PAGE.contains("planner"));
    assert!(PAGE.contains("What shop remembers"));
    assert!(PAGE.contains("The boss sees this every time you message him."));
    assert!(PAGE.contains("id=\"composer\"") || PAGE.contains("composer"));
    assert!(PAGE.contains("id=\"send\"") || PAGE.contains("Send"));
    assert!(PAGE.contains("Message the boss"));
    assert!(PAGE.contains("Activity"));
    assert!(PAGE.contains("Inbox"));
    assert!(PAGE.contains("This computer"));
    assert!(PAGE.contains("action-pill"));
    assert!(PAGE.contains("id=\"steer-input\""));
    assert!(PAGE.contains("id=\"project-switcher\""));
    assert!(PAGE.contains("Open folder") || PAGE.contains("open-folder"));
    assert!(PAGE.contains("Recents") || PAGE.contains("id=\"recents\""));
    assert!(!PAGE.contains("<pre"));

    let (shop, _dir) = fresh();
    let (addr, _h) = ui::spawn(shop, 0).unwrap();
    let host = format!("{}:{}", addr.ip(), addr.port());
    let html = http_get(&host, "/");
    assert!(html.contains("id=\"command-center\""));
    assert!(html.contains("id=\"workers\""));
    assert!(html.contains("id=\"floor\""));
    assert!(html.contains("id=\"github\""));
    assert!(html.contains("id=\"mailbox\""));
    assert!(html.contains("id=\"control\""));
    assert!(html.contains("id=\"feed\""));
    assert!(html.contains("SuperGrokHeavy"));
    assert!(html.contains("Talk to the boss"));
    assert!(html.contains("What shop remembers"));
    assert!(html.contains("Current job"));
    assert!(html.contains("This computer"));
    assert!(
        html.contains("intelligence-backend") || html.contains("name=\"backend\""),
        "command center must include an intelligence control"
    );
    let css = http_get(&host, "/ui.css");
    assert!(css.contains("#09090b") || css.contains("--bg"));
    let feed = http_get(&host, "/feed.xml");
    assert!(feed.contains("<feed") || feed.contains("<channel"));
}

#[test]
fn seeded_workers_are_on_the_roster() {
    let (shop, _dir) = fresh();
    let peers: Vec<String> = shop
        .workers()
        .unwrap()
        .workers
        .into_iter()
        .map(|w| w.peer)
        .collect();
    for need in ["cursor", "grok-ultra", "grok-bot", "cursor-groksuperheavy"] {
        assert!(
            peers.iter().any(|p| p == need),
            "missing seeded worker {need}"
        );
    }
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
fn mailbox_assign_writes_inbox_json_shape() {
    let dir = tmp_store();
    let mailbox = dir.join("bridge");
    fs::create_dir_all(mailbox.join("inbox")).unwrap();
    let mut shop = Shop::init(dir.join("store"))
        .unwrap()
        .with_mailbox(Some(mailbox.clone()));
    enroll(&mut shop, &["alice"]);
    shop.open("P", "t", "b").unwrap();
    shop.split("P", "c1", "alice", "only", "t", "body").unwrap();
    shop.assign("P").unwrap();
    let path = mailbox.join("inbox/alice/assign-P-c1.json");
    assert!(path.is_file());
    let v: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(v["schema_version"], SCHEMA_VERSION);
    assert_eq!(v["type"], "assign");
    assert_eq!(v["to"], "alice");
    assert_eq!(v["lane_id"], "c1");
}

#[test]
fn status_reports_unread_and_wait() {
    let (shop, _dir) = fresh();
    let text = shop.status(None).unwrap();
    assert!(text.contains("WAIT") || text.contains("unread"));
    let a = shop.awareness(None).unwrap();
    assert!(a.mailbox.wait.is_some() || a.github.wait.is_some() || !a.waits.is_empty());

    let dir = tmp_store();
    let mailbox = dir.join("bridge");
    fs::create_dir_all(mailbox.join("inbox/alice")).unwrap();
    fs::write(mailbox.join("inbox/alice/assign-P-c1.json"), "{}").unwrap();
    let mut shop = Shop::init(dir.join("store"))
        .unwrap()
        .with_mailbox(Some(mailbox));
    enroll(&mut shop, &["alice"]);
    let a = shop.awareness(None).unwrap();
    assert!(a.mailbox.unread_total >= 1);
    let text = shop.status(None).unwrap();
    assert!(text.contains("unread"));
}

#[test]
fn verify_fail_blocks_merge() {
    let mock = Arc::new(
        MockGitHub::authed()
            .with_create_pr(true)
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
    let rec = shop.verify("P", Some("false")).unwrap();
    assert_eq!(rec.status, EvidenceStatus::Wait);
    assert!(matches!(
        shop.merge_verified("P").unwrap_err(),
        ShopError::Wait(_)
    ));
    assert!(!mock.merged());
}

#[test]
fn verify_pass_allows_merge_record() {
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
    let rec = shop.verify("P", Some("true")).unwrap();
    assert_eq!(rec.status, EvidenceStatus::Pass);
    shop.merge_verified("P").unwrap();
    assert!(mock.merged() || shop.state().unwrap().parents["P"].github_merged);
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

#[test]
fn events_append_on_add_worker() {
    let (mut shop, dir) = fresh();
    shop.add_worker("alice", "Alice", "grok-bot", "", true)
        .unwrap();
    let log = fs::read_to_string(dir.join("events.jsonl")).unwrap();
    assert!(log.contains("add_worker"));
    assert!(log.contains("alice"));
    assert!(log.contains("cursor-groksuperheavy"));
}

#[test]
fn steer_status_is_a_shop_line() {
    let (mut shop, _dir) = fresh();
    let v = shop.steer("status").unwrap();
    assert_eq!(v["kind"], "shop");
    let text = shop.steer_transcript();
    let joined = text.iter().map(|l| l.to_string()).collect::<String>();
    assert!(joined.contains("status") || joined.contains("WORKERS") || joined.contains("shop"));
}

#[test]
fn steer_free_text_goes_to_supergrokheavy_not_grok_bot() {
    let dir = tmp_store();
    let mailbox = dir.join("bridge");
    fs::create_dir_all(mailbox.join("inbox")).unwrap();
    let mut shop = Shop::init(dir.join("store"))
        .unwrap()
        .with_mailbox(Some(mailbox.clone()));
    let v = shop.steer("please look at the floor").unwrap();
    assert_eq!(v["to"], STEER_TO);
    assert_eq!(STEER_TO, "cursor-groksuperheavy");
    assert_ne!(STEER_TO, "grok-bot");
    let log = fs::read_to_string(dir.join("store/steer.jsonl")).unwrap();
    assert!(log.contains("please look at the floor"));
    let inbox = mailbox.join("inbox/cursor-groksuperheavy");
    let found = fs::read_dir(&inbox)
        .unwrap()
        .flatten()
        .any(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"));
    assert!(found, "steer JSON must land in SuperGrokHeavy inbox");
    let rec: serde_json::Value = fs::read_dir(&inbox)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .map(|p| serde_json::from_slice(&fs::read(p).unwrap()).unwrap())
        .unwrap();
    assert_eq!(rec["type"], "steer");
    assert_eq!(rec["to"], "cursor-groksuperheavy");
    assert_eq!(rec["from"], "user");
    assert!(rec["memory"]["profile"].is_array());
    assert!(rec["memory"]["floor"].is_object());
    assert!(rec["memory"]["floor"].get("children").is_some());
    assert!(rec["memory"]["floor"].get("claims").is_some());
}

#[test]
fn steer_free_text_does_not_invent_a_split() {
    let (mut shop, _dir) = fresh();
    shop.steer("split the work please").unwrap();
    assert!(shop.state().unwrap().parents.is_empty());
}

#[test]
fn open_project_a_then_b_switches_store() {
    let recents = tmp_store().join("recents.json");
    let a = tmp_store().join("proj-a");
    let b = tmp_store().join("proj-b");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    let mut shop = Shop::open_project_with_recents(&a, &recents).unwrap();
    shop.open("PA", "a", "a").unwrap();
    assert!(shop.store_root().starts_with(&a) || shop.project_root() == a);
    let shop_b = Shop::open_project_with_recents(&b, &recents).unwrap();
    assert!(shop_b.state().unwrap().parents.get("PA").is_none());
    assert_eq!(shop_b.project_root(), b.as_path());
    let rec = fs::read_to_string(&recents).unwrap();
    assert!(rec.contains("proj-a"));
    assert!(rec.contains("proj-b"));
}

#[test]
fn open_github_repo_sets_record_without_a_lane() {
    let (mut shop, dir) = fresh();
    shop.steer("open repo halthinks/shop-floor").unwrap();
    let repo = shop.github_repo().unwrap().unwrap();
    assert_eq!(repo.slug(), "halthinks/shop-floor");
    assert!(dir.join("github.json").is_file());
    assert!(shop.state().unwrap().parents.is_empty());
}

#[test]
fn refuse_platform_project_path() {
    let recents = tmp_store().join("recents.json");
    match Shop::open_project_with_recents(r"C:\TextPCB Platform", &recents) {
        Ok(_) => panic!("expected ForbiddenWrite"),
        Err(ShopError::ForbiddenWrite(_)) => {}
        Err(e) => panic!("expected ForbiddenWrite, got {e}"),
    }
    match Shop::open_project_with_recents(r"C:\\TextPCB Platform", &recents) {
        Ok(_) => panic!("expected ForbiddenWrite"),
        Err(ShopError::ForbiddenWrite(_)) => {}
        Err(e) => panic!("expected ForbiddenWrite, got {e}"),
    }
}

#[test]
fn remember_and_forget_profile_facts() {
    let (mut shop, dir) = fresh();
    shop.remember(shop::MemoryTier::Profile, "we ship shop-floor")
        .unwrap();
    let mem = shop.memory();
    assert!(mem.profile.iter().any(|f| f == "we ship shop-floor"));
    assert!(dir.join("memory/profile.json").is_file());
    shop.forget("we ship shop-floor").unwrap();
    assert!(!shop
        .memory()
        .profile
        .iter()
        .any(|f| f == "we ship shop-floor"));
    match shop.remember(shop::MemoryTier::Profile, r"C:\TextPCB Platform") {
        Err(ShopError::ForbiddenWrite(_)) => {}
        other => panic!("expected ForbiddenWrite, got {other:?}"),
    }
}

#[test]
fn add_worker_grows_memory_log() {
    let (mut shop, _dir) = fresh();
    let before = shop.memory().log.len();
    shop.add_worker("alice", "Alice", "grok-bot", "", true)
        .unwrap();
    assert!(shop.memory().log.len() > before);
    let joined = shop
        .memory()
        .log
        .iter()
        .map(|l| l.fact.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("alice"));
}

#[test]
fn floor_memory_persists_across_open_store() {
    let dir = tmp_store();
    let mut shop = Shop::init(&dir).unwrap();
    enroll(&mut shop, &["alice"]);
    shop.open("P", "title", "body").unwrap();
    shop.split("P", "c1", "alice", "src/a,src/b", "one", "do a")
        .unwrap();
    assert!(dir.join("floor/current.json").is_file());
    assert!(dir.join("floor/children.json").is_file());
    assert!(dir.join("floor/claims.json").is_file());
    let floor = shop.floor();
    assert_eq!(floor.current.as_ref().map(|c| c.id.as_str()), Some("P"));
    assert_eq!(floor.children.len(), 1);
    assert_eq!(floor.children[0].state, ChildState::Assigned);
    assert_eq!(floor.claims.len(), 1);
    drop(shop);

    let shop2 = Shop::open_store(&dir).unwrap();
    let floor2 = shop2.floor();
    assert_eq!(floor2.current.as_ref().map(|c| c.id.as_str()), Some("P"));
    assert_eq!(floor2.children.len(), 1);
    assert_eq!(floor2.children[0].id, "c1");
    assert_eq!(floor2.children[0].peer, "alice");
    assert_eq!(floor2.claims.len(), 1);
    assert_eq!(floor2.claims[0].paths, vec!["src/a", "src/b"]);
}

#[test]
fn floor_memory_records_bounce_and_close_clears_current() {
    let (mut shop, dir) = fresh();
    enroll(&mut shop, &["alice", "bob"]);
    shop.open("P", "title", "body").unwrap();
    shop.split("P", "c1", "alice", "src/a", "one", "a").unwrap();
    shop.split("P", "c2", "bob", "src/b", "two", "b").unwrap();
    let bounced = shop.bounce("P", "c1", "failed closed").unwrap();
    assert_eq!(bounced.state, ChildState::Bounced);
    let floor = shop.floor();
    let c1 = floor.children.iter().find(|c| c.id == "c1").unwrap();
    assert_eq!(c1.state, ChildState::Bounced);
    assert_eq!(c1.bounce_reason.as_deref(), Some("failed closed"));
    assert_ne!(c1.state, ChildState::Joined);

    shop.accept("P", "c1", r#"{"status":"PASS"}"#).unwrap_err();
    shop.assign("P").unwrap();
    shop.accept("P", "c1", r#"{"status":"PASS"}"#).unwrap();
    shop.accept("P", "c2", r#"{"status":"DONE"}"#).unwrap();
    shop.join("P").unwrap();
    shop.reduce("P", "pkg").unwrap();
    shop.verify("P", Some("true")).unwrap();
    shop.close("P").unwrap();
    let floor = shop.floor();
    assert!(floor.current.is_none());
    assert!(dir.join("floor/history.jsonl").is_file());
    let hist = fs::read_to_string(dir.join("floor/history.jsonl")).unwrap();
    assert!(hist.contains("\"P\"") || hist.contains("P"));
}

#[test]
fn steer_json_includes_memory_floor_and_profile() {
    let dir = tmp_store();
    let mailbox = dir.join("bridge");
    fs::create_dir_all(mailbox.join("inbox")).unwrap();
    let mut shop = Shop::init(dir.join("store"))
        .unwrap()
        .with_mailbox(Some(mailbox.clone()));
    shop.remember(shop::MemoryTier::Profile, "standing fact")
        .unwrap();
    enroll(&mut shop, &["alice"]);
    shop.open("P", "t", "b").unwrap();
    shop.split("P", "c1", "alice", "src/a", "t", "b").unwrap();
    let v = shop.steer("please look at the floor").unwrap();
    assert_eq!(v["to"], "cursor-groksuperheavy");
    assert!(v["memory"]["profile"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x == "standing fact"));
    assert!(v["memory"]["floor"]["children"].as_array().unwrap().len() >= 1);
    assert!(v["memory"]["floor"]["claims"].as_array().unwrap().len() >= 1);
    let inbox = mailbox.join("inbox/cursor-groksuperheavy");
    let rec: serde_json::Value = fs::read_dir(&inbox)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .map(|p| serde_json::from_slice(&fs::read(p).unwrap()).unwrap())
        .unwrap();
    assert_eq!(rec["to"], "cursor-groksuperheavy");
    assert!(rec["memory"]["floor"]["children"].is_array());
    assert!(rec["memory"]["profile"].is_array());
}

#[test]
fn steer_reply_file_is_ingested_into_api_steer() {
    let dir = tmp_store();
    let mailbox = dir.join("bridge");
    fs::create_dir_all(mailbox.join("inbox/shop")).unwrap();
    let shop = Shop::init(dir.join("store"))
        .unwrap()
        .with_mailbox(Some(mailbox.clone()));
    fs::write(
        mailbox.join("inbox/shop/reply-1.json"),
        r#"{"type":"steer-reply","from":"cursor-groksuperheavy","to":"user","body":"I see the floor."}"#,
    )
    .unwrap();
    let (addr, _h) = ui::spawn(shop, 0).unwrap();
    let host = format!("{}:{}", addr.ip(), addr.port());
    let body = http_get(&host, "/api/steer");
    assert!(body.contains("I see the floor."), "{body}");
    assert!(body.contains("orchestrator"), "{body}");
}

#[test]
fn no_fake_orchestrator_line_when_mailbox_empty() {
    let (shop, _dir) = fresh();
    shop.ingest_replies().unwrap();
    let text = shop.steer_transcript();
    assert!(!text.iter().any(|l| l["role"] == "orchestrator"));
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
                if last.contains("id=\"command-center\"")
                    || last.contains("id=\"add-worker\"")
                    || last.contains("--bg")
                    || last.starts_with("HTTP/1.1 200")
                {
                    return last;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    last
}
