//! ROADMAP 1 leftover: a name on the roster is not a running worker.
//!
//! Seed / add_worker enroll capacity only. Assign writes a job. A running
//! worker is a real launched process with a tracked pid. Incomplete evidence
//! is WAIT. Start/exit are Evidence, never VERIFIED or PASS.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use shop::runner::SHOP_RUN_CMD;
use shop::types::{ChildState, ParentState};
use shop::workers::SEEDED_WORKERS;
use shop::{EvidenceClass, PidLiveness, Shop};

static SEQ: AtomicU64 = AtomicU64::new(0);
static ENV: Mutex<()> = Mutex::new(());

fn lock_env() -> MutexGuard<'static, ()> {
    ENV.lock().unwrap_or_else(|e| e.into_inner())
}

fn tmp_store() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("shop-run-workers-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn isolated() -> (MutexGuard<'static, ()>, Shop, PathBuf) {
    let guard = lock_env();
    env::remove_var("SHOP_RUN");
    env::remove_var(SHOP_RUN_CMD);
    let dir = tmp_store();
    (guard, Shop::init(&dir).unwrap(), dir)
}

fn enroll(shop: &mut Shop, peer: &str) {
    shop.add_worker(peer, peer, "grok-bot", "", true).unwrap();
}

fn view_for<'a>(shop: &'a Shop, peer: &str) -> shop::awareness::WorkerView {
    shop.awareness(None)
        .unwrap()
        .workers
        .into_iter()
        .find(|w| w.peer == peer)
        .unwrap_or_else(|| panic!("missing roster peer {peer}"))
}

fn noop_cmd() -> &'static str {
    if cfg!(windows) {
        "cmd /c exit 0"
    } else {
        "true"
    }
}

#[test]
fn seeded_roster_name_is_not_a_running_worker() {
    let (_lock, shop, _dir) = isolated();
    let roster = shop.workers().unwrap();
    assert!(
        !roster.workers.is_empty(),
        "init seeds roster names as capacity"
    );
    for seed in SEEDED_WORKERS {
        let row = roster
            .get(seed.peer)
            .unwrap_or_else(|| panic!("missing seeded roster name {}", seed.peer));
        assert_eq!(row.name, seed.title);
        assert_eq!(row.intelligence.backend.as_str(), seed.backend);
        assert_eq!(row.intelligence.model, seed.model);
    }

    let ledger = shop.procwait().unwrap();
    assert!(
        ledger.processes.is_empty(),
        "seeded roster must not invent a live process"
    );
    assert!(ledger.latest_for_peer("cursor").is_none());
    assert!(ledger.latest_for_peer("cursor-groksuperheavy").is_none());

    for w in shop.awareness(None).unwrap().workers {
        assert_eq!(w.pid, None, "roster name {} is not a pid", w.peer);
        assert_eq!(w.pid_liveness, None);
        assert_ne!(
            w.action, "working",
            "roster name {} is not a running worker",
            w.peer
        );
        assert_eq!(w.action, "idle");
    }

    let status = shop.status(None).unwrap();
    assert!(status.contains("WORKERS"));
    assert!(status.contains("PROCESSES"));
    assert!(
        !status.contains("pid="),
        "status must not pretend a roster name has a pid: {status}"
    );
}

#[test]
fn add_worker_enrolls_capacity_without_a_process() {
    let (_lock, mut shop, dir) = isolated();
    let before = shop.procwait().unwrap().processes.len();
    let w = shop
        .add_worker("alice", "Alice", "grok-bot", "", true)
        .unwrap();
    assert_eq!(w.peer, "alice");
    assert_eq!(w.name, "Alice");
    assert!(dir.join("workers.json").is_file());

    assert!(
        shop.procwait().unwrap().processes.len() == before
            && shop.procwait().unwrap().latest_for_peer("alice").is_none()
    );
    let view = view_for(&shop, "alice");
    assert_eq!(view.pid, None);
    assert_ne!(view.action, "working");
    assert_eq!(view.action, "idle");
}

#[test]
fn assigned_roster_peer_is_not_a_running_worker() {
    let (_lock, mut shop, dir) = isolated();
    enroll(&mut shop, "alice");
    shop.open("P", "t", "b").unwrap();
    shop.split("P", "c1", "alice", "src/a", "one", "do a")
        .unwrap();
    fs::write(
        dir.join("run.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": 1,
            "backends": { "grok-bot": noop_cmd() }
        }))
        .unwrap(),
    )
    .unwrap();

    let recs = shop.assign("P").unwrap();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].to, "alice");

    let child = shop.state().unwrap().parents["P"].children["c1"].clone();
    assert_eq!(child.state, ChildState::Assigned);
    assert!(child.dispatched);
    assert!(
        shop.procwait().unwrap().processes.is_empty(),
        "default assign must not launch"
    );

    let view = view_for(&shop, "alice");
    assert_eq!(view.pid, None);
    assert_eq!(view.action, "assigned");
    assert_ne!(view.action, "working");
}

#[test]
fn run_child_unconfigured_leaves_roster_not_running() {
    let (_lock, mut shop, _dir) = isolated();
    enroll(&mut shop, "alice");
    shop.open("P", "t", "b").unwrap();
    shop.split("P", "c1", "alice", "src/a", "one", "do a")
        .unwrap();
    shop.assign("P").unwrap();

    let ev = shop.run_child("P", "c1").unwrap();
    assert_eq!(ev.class, EvidenceClass::Inconclusive);
    assert_eq!(ev.pid, None, "unconfigured run must not invent a PID");
    assert!(!ev.started());
    assert!(!ev.is_verified());
    assert_ne!(ev.class, EvidenceClass::Pass);

    let child = shop.state().unwrap().parents["P"].children["c1"].clone();
    assert_eq!(child.state, ChildState::Assigned);
    assert_ne!(
        shop.state().unwrap().parents["P"].state,
        ParentState::Verified
    );
    assert!(shop.procwait().unwrap().processes.is_empty());

    let view = view_for(&shop, "alice");
    assert_eq!(view.pid, None);
    assert_ne!(view.action, "working");
}

#[test]
fn run_child_real_process_is_the_running_worker() {
    let (_lock, mut shop, dir) = isolated();
    enroll(&mut shop, "alice");
    shop.open("P", "t", "b").unwrap();
    shop.split("P", "c1", "alice", "src/a", "one", "do a")
        .unwrap();
    shop.assign("P").unwrap();
    fs::write(
        dir.join("run.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": 1,
            "backends": { "grok-bot": noop_cmd() }
        }))
        .unwrap(),
    )
    .unwrap();

    let ev = shop.run_child("P", "c1").unwrap();
    let pid = ev
        .pid
        .expect("configured run.json must spawn a real process");
    assert!(pid > 0);
    assert!(ev.started());
    assert!(!ev.is_verified());
    assert_ne!(ev.class, EvidenceClass::Pass);

    let ledger = shop.procwait().unwrap();
    let tracked = ledger
        .latest_for_peer("alice")
        .expect("running worker is a tracked pid, not a roster name");
    assert_eq!(tracked.pid, pid);
    assert_eq!(tracked.peer.as_deref(), Some("alice"));
    assert_eq!(tracked.parent_id.as_deref(), Some("P"));
    assert_eq!(tracked.child_id.as_deref(), Some("c1"));

    let view = view_for(&shop, "alice");
    assert_eq!(view.pid, Some(pid));
    assert!(view.pid_liveness.is_some());
    match view.pid_liveness {
        Some(PidLiveness::Working) | Some(PidLiveness::Dead) => {}
        None => panic!("running worker must have observed liveness"),
    }
    assert_ne!(
        shop.state().unwrap().parents["P"].state,
        ParentState::Verified
    );
}
