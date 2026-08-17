//! Live process wait/stop. Exit is Evidence. Never VERIFIED/CLOSE.
//! Dead pid + no exit is UNKNOWN. Stop is explicit. Alive = working; dead is not.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use shop::awareness::classify_action;
use shop::mailbox::MailboxObservation;
use shop::procwait::{
    evidence_for_exit, status_pill, LiveProcess, PidLiveness, ProcWaitLedger, ProcessBind,
};
use shop::types::{EvidenceStatus, ParentState};
use shop::{EvidenceClass, Shop};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp_store() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("shop-procwait-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn fresh() -> (Shop, PathBuf) {
    let dir = tmp_store();
    (Shop::init(&dir).unwrap(), dir)
}

fn empty_mail() -> MailboxObservation {
    MailboxObservation {
        root: None,
        present: false,
        inbox_present: false,
        wait: None,
        peers: Vec::new(),
        unread_total: 0,
        outbox_count: 0,
    }
}

fn enroll(shop: &mut Shop, peer: &str) {
    shop.add_worker(peer, peer, "grok-bot", "", true).unwrap();
}

#[test]
fn wait_records_exit_as_evidence_never_verified_or_closed() {
    let (mut shop, _dir) = fresh();
    enroll(&mut shop, "alice");
    shop.open("P", "parent", "body").unwrap();
    shop.split("P", "c1", "alice", "src/a", "one", "do a")
        .unwrap();
    shop.accept("P", "c1", r#"{"status":"PASS"}"#).unwrap();
    shop.join("P").unwrap();
    shop.reduce("P", "pkg").unwrap();
    assert_eq!(
        shop.state().unwrap().parents["P"].state,
        ParentState::Reduced
    );
    assert!(shop.state().unwrap().parents["P"].verify.is_none());

    let mut child = Command::new("true").spawn().unwrap();
    let ev = shop
        .wait_child(&mut child, Some("P"), Some("c1"), Some("alice"))
        .unwrap();

    assert_eq!(ev.exit_code, Some(0));
    assert_eq!(ev.status, EvidenceStatus::Wait);
    assert_eq!(ev.class, EvidenceClass::Inconclusive);
    assert_ne!(ev.class, EvidenceClass::Pass);
    assert!(ev.aasm_note.contains("Evidence"));
    assert!(ev.aasm_note.contains("not VERIFIED"));

    let parent = shop.state().unwrap().parents["P"].clone();
    assert_eq!(parent.state, ParentState::Reduced);
    assert_ne!(parent.state, ParentState::Verified);
    assert_ne!(parent.state, ParentState::Closed);
    assert!(
        parent.verify.is_none(),
        "wait must not write Parent.verify or reuse shop.verify"
    );

    let tracked = shop.procwait().unwrap().get(ev.pid).cloned().unwrap();
    assert!(!tracked.explicit_stop, "waiting is not stopping");
    assert_eq!(tracked.wait.as_ref().map(|w| w.exit_code), Some(Some(0)));
}

#[test]
fn dead_pid_without_exit_is_unknown() {
    let (mut shop, _dir) = fresh();
    let ev = shop
        .wait_pid(999_999_991, None, None, None)
        .expect("dead pid wait records Unknown, does not invent PASS");
    assert!(ev.exit_code.is_none());
    assert!(ev.is_unknown());
    assert_eq!(ev.class, EvidenceClass::Unknown);
    assert_eq!(ev.status, EvidenceStatus::Wait);
    assert_ne!(ev.class, EvidenceClass::Pass);
    assert!(ev.aasm_note.contains("UNKNOWN") || ev.aasm_note.contains("Unknown"));

    let classified = evidence_for_exit(8, None);
    assert_eq!(classified.class, EvidenceClass::Unknown);
    assert_ne!(classified.status, EvidenceStatus::Pass);
}

#[test]
fn stop_is_explicit_waiting_is_not_stopping() {
    let (mut shop, _dir) = fresh();
    enroll(&mut shop, "alice");

    let mut child = Command::new("true").spawn().unwrap();
    let waited = shop
        .wait_child(&mut child, None, None, Some("alice"))
        .unwrap();
    let after_wait = shop.procwait().unwrap().get(waited.pid).cloned().unwrap();
    assert!(!after_wait.explicit_stop);
    assert!(after_wait.wait.is_some());

    let mut live = Command::new("sleep").arg("60").spawn().unwrap();
    let pid = live.id();
    shop.track_pid(pid, None, None, Some("alice")).unwrap();
    assert!(
        shop.procwait().unwrap().get(pid).unwrap().is_working(),
        "alive pid must be working before stop"
    );
    let rec = shop.stop_pid(pid).unwrap();
    assert!(rec.explicit);
    assert!(rec.note.contains("explicit"));
    assert!(rec.note.contains("waiting is not stopping"));
    let after_stop = shop.procwait().unwrap().get(pid).cloned().unwrap();
    assert!(after_stop.explicit_stop);
    assert!(!after_stop.is_working());
    assert_ne!(after_stop.status_pill(), "working");
    let _ = live.wait();
}

#[test]
fn alive_pid_is_working_dead_pid_is_not() {
    assert_eq!(status_pill(PidLiveness::Working), "working");
    assert_eq!(status_pill(PidLiveness::Dead), "dead");
    assert_ne!(status_pill(PidLiveness::Dead), "working");

    let (mut shop, _dir) = fresh();
    enroll(&mut shop, "alice");
    let mut live = Command::new("sleep").arg("60").spawn().unwrap();
    let pid = live.id();
    let tracked = shop.track_pid(pid, None, None, Some("alice")).unwrap();
    assert_eq!(tracked.status_pill(), "working");
    assert!(tracked.is_working());

    let awareness = shop.awareness(None).unwrap();
    let alice = awareness
        .workers
        .iter()
        .find(|w| w.peer == "alice")
        .expect("alice enrolled");
    assert_eq!(alice.action, "working");
    assert_eq!(alice.pid, Some(pid));
    assert_eq!(alice.pid_liveness, Some(PidLiveness::Working));

    let (action, reason, _) = classify_action(
        "alice",
        &[],
        &empty_mail(),
        &shop.procwait().unwrap().processes,
    );
    assert_eq!(action, "working");
    assert!(reason.unwrap().contains("working"));

    shop.stop_pid(pid).unwrap();
    let dead = shop.procwait().unwrap().get(pid).cloned().unwrap();
    assert_eq!(dead.status_pill(), "dead");
    assert!(!dead.is_working());

    let awareness = shop.awareness(None).unwrap();
    let alice = awareness
        .workers
        .iter()
        .find(|w| w.peer == "alice")
        .unwrap();
    assert_eq!(alice.action, "dead");
    assert_ne!(alice.action, "working");
    assert_eq!(alice.pid_liveness, Some(PidLiveness::Dead));

    let (action, reason, _) = classify_action(
        "alice",
        &[],
        &empty_mail(),
        &shop.procwait().unwrap().processes,
    );
    assert_eq!(action, "dead");
    assert_ne!(action, "working");
    assert!(reason.unwrap().contains("dead"));

    let _ = live.wait();
}

#[test]
fn ledger_pills_never_call_verify_path() {
    let mut ledger = ProcWaitLedger::default();
    let mut working =
        LiveProcess::observed(std::process::id(), ProcessBind::new(None, None, Some("w")));
    working.liveness = PidLiveness::Working;
    ledger.upsert(working);
    let mut dead = LiveProcess::observed(1, ProcessBind::new(None, None, Some("d")));
    dead.liveness = PidLiveness::Dead;
    ledger.upsert(dead);
    assert_eq!(
        ledger.latest_for_peer("w").unwrap().status_pill(),
        "working"
    );
    assert_eq!(ledger.latest_for_peer("d").unwrap().status_pill(), "dead");
    assert_ne!(
        ledger.latest_for_peer("d").unwrap().status_pill(),
        "working"
    );
}
