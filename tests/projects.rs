//! Multiple open floors: switch without losing the other project's memory/floor.
//! Incomplete evidence is WAIT, never a fake PASS.

#[path = "../src/projects.rs"]
mod projects;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use shop::memory::MemoryTier;
use shop::types::{EvidenceStatus, ParentState};
use shop::{Shop, ShopError};

use projects::FloorSet;

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp_dir() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("shop-projects-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn proj(name: &str) -> PathBuf {
    let p = tmp_dir().join(name);
    fs::create_dir_all(&p).unwrap();
    p
}

fn set() -> (FloorSet, PathBuf) {
    let recents = tmp_dir().join("recents.json");
    (FloorSet::with_recents(&recents), recents)
}

fn is_wait(err: &ShopError) -> bool {
    match err {
        ShopError::IncompleteEvidence(_) | ShopError::Wait(_) | ShopError::ForbiddenWrite(_) => {
            true
        }
        _ => false,
    }
}

#[test]
fn open_two_floors_keeps_both_live() {
    let (mut floors, _) = set();
    let a = proj("alpha");
    let b = proj("beta");

    let ha = floors.open(&a).unwrap();
    assert_eq!(floors.len(), 1);
    assert_eq!(ha.name, "alpha");
    assert_eq!(floors.current().unwrap().project_root(), a.as_path());

    let hb = floors.open(&b).unwrap();
    assert_eq!(floors.len(), 2);
    assert_eq!(hb.name, "beta");
    assert_eq!(floors.current().unwrap().project_root(), b.as_path());

    let listed: Vec<String> = floors.list().into_iter().map(|h| h.name).collect();
    assert!(listed.contains(&"alpha".into()));
    assert!(listed.contains(&"beta".into()));
}

#[test]
fn switch_preserves_other_project_memory_and_floor() {
    let (mut floors, _) = set();
    let a = proj("mem-a");
    let b = proj("mem-b");

    floors.open(&a).unwrap();
    floors
        .current_mut()
        .unwrap()
        .remember(MemoryTier::Profile, "alpha-standing-fact")
        .unwrap();
    floors
        .current_mut()
        .unwrap()
        .open("PA", "alpha parent", "alpha body")
        .unwrap();

    floors.open(&b).unwrap();
    floors
        .current_mut()
        .unwrap()
        .remember(MemoryTier::Profile, "beta-standing-fact")
        .unwrap();
    floors
        .current_mut()
        .unwrap()
        .open("PB", "beta parent", "beta body")
        .unwrap();

    // B is current: must not see A's floor or memory.
    let b_shop = floors.current().unwrap();
    let b_mem = b_shop.memory();
    assert!(b_mem.profile.iter().any(|f| f == "beta-standing-fact"));
    assert!(!b_mem.profile.iter().any(|f| f == "alpha-standing-fact"));
    let b_state = b_shop.state().unwrap();
    assert!(b_state.parents.contains_key("PB"));
    assert!(!b_state.parents.contains_key("PA"));
    let b_floor = b_shop.floor();
    assert_eq!(b_floor.current.as_ref().map(|c| c.id.as_str()), Some("PB"));

    // Switch back to A without re-opening from scratch (path or unique name).
    floors.switch("mem-a").unwrap();
    assert_eq!(floors.current().unwrap().project_root(), a.as_path());
    floors.switch(b.to_str().unwrap()).unwrap();
    floors.switch(a.to_str().unwrap()).unwrap();
    assert_eq!(floors.len(), 2, "switch must not drop the other floor");
    let a_shop = floors.current().unwrap();
    assert_eq!(a_shop.project_root(), a.as_path());
    let a_mem = a_shop.memory();
    assert!(
        a_mem.profile.iter().any(|f| f == "alpha-standing-fact"),
        "A memory must survive opening B: {:?}",
        a_mem.profile
    );
    assert!(!a_mem.profile.iter().any(|f| f == "beta-standing-fact"));
    let a_state = a_shop.state().unwrap();
    assert!(a_state.parents.contains_key("PA"));
    assert!(!a_state.parents.contains_key("PB"));
    let a_floor = a_shop.floor();
    assert_eq!(a_floor.current.as_ref().map(|c| c.id.as_str()), Some("PA"));
    assert_eq!(a_floor.current.as_ref().unwrap().state, ParentState::Held);
}

#[test]
fn reopen_same_path_switches_and_does_not_duplicate() {
    let (mut floors, _) = set();
    let a = proj("same-a");
    let b = proj("same-b");
    floors.open(&a).unwrap();
    floors
        .current_mut()
        .unwrap()
        .remember(MemoryTier::Profile, "keep-me")
        .unwrap();
    floors.open(&b).unwrap();
    assert_eq!(floors.len(), 2);

    let again = floors.open(&a).unwrap();
    assert_eq!(floors.len(), 2);
    assert_eq!(again.name, "same-a");
    assert_eq!(floors.current().unwrap().project_root(), a.as_path());
    assert!(floors
        .current()
        .unwrap()
        .memory()
        .profile
        .iter()
        .any(|f| f == "keep-me"));
}

#[test]
fn get_other_floor_without_switching_current() {
    let (mut floors, _) = set();
    let a = proj("get-a");
    let b = proj("get-b");
    floors.open(&a).unwrap();
    floors
        .current_mut()
        .unwrap()
        .remember(MemoryTier::Log, "a-log-line")
        .unwrap();
    floors.open(&b).unwrap();
    assert_eq!(floors.current().unwrap().project_root(), b.as_path());

    let a_shop = floors.get(a.to_str().unwrap()).unwrap();
    assert_eq!(a_shop.project_root(), a.as_path());
    assert!(a_shop
        .memory()
        .log
        .iter()
        .any(|line| line.fact == "a-log-line"));
    assert_eq!(floors.current().unwrap().project_root(), b.as_path());
}

#[test]
fn close_one_floor_leaves_the_other() {
    let (mut floors, _) = set();
    let a = proj("close-a");
    let b = proj("close-b");
    floors.open(&a).unwrap();
    floors
        .current_mut()
        .unwrap()
        .remember(MemoryTier::Profile, "a-stays-on-disk")
        .unwrap();
    floors.open(&b).unwrap();
    floors.close(a.to_str().unwrap()).unwrap();
    assert_eq!(floors.len(), 1);
    assert_eq!(floors.current().unwrap().project_root(), b.as_path());

    // Disk memory for A is still there after the live handle is dropped.
    let recents = tmp_dir().join("recents.json");
    let reopened = Shop::open_project_with_recents(&a, recents).unwrap();
    assert!(reopened
        .memory()
        .profile
        .iter()
        .any(|f| f == "a-stays-on-disk"));
}

#[test]
fn refuse_platform_path() {
    let (mut floors, _) = set();
    match floors.open(Path::new(r"C:\TextPCB Platform")) {
        Ok(_) => panic!("expected ForbiddenWrite"),
        Err(ShopError::ForbiddenWrite(_)) => {}
        Err(e) => panic!("expected ForbiddenWrite, got {e}"),
    }
    match floors.open(Path::new(r"C:\\TextPCB Platform")) {
        Ok(_) => panic!("expected ForbiddenWrite"),
        Err(ShopError::ForbiddenWrite(_)) => {}
        Err(e) => panic!("expected ForbiddenWrite, got {e}"),
    }
    assert!(floors.is_empty());
}

#[test]
fn empty_and_unknown_are_wait_not_pass() {
    let (mut floors, _) = set();
    match floors.open(Path::new("")) {
        Err(e) => {
            assert!(is_wait(&e), "{e}");
            match e {
                ShopError::IncompleteEvidence(_) => {}
                other => panic!("empty path must be IncompleteEvidence, got {other}"),
            }
        }
        Ok(_) => panic!("empty path must WAIT"),
    }

    match floors.switch("") {
        Err(ShopError::IncompleteEvidence(msg)) => assert!(msg.contains("WAIT")),
        other => panic!("empty switch must WAIT, got {other:?}"),
    }
    match floors.switch("never-opened") {
        Err(ShopError::Wait(msg)) => assert!(msg.contains("never-opened")),
        other => panic!("unknown switch must WAIT, got {other:?}"),
    }
    match floors.current() {
        Err(ShopError::IncompleteEvidence(msg)) => assert!(msg.contains("WAIT")),
        other => panic!("no current must WAIT, got {other:?}"),
    }
    match floors.get("missing") {
        Err(ShopError::Wait(_)) => {}
        other => panic!("missing get must WAIT, got {other:?}"),
    }
    match floors.close("") {
        Err(ShopError::IncompleteEvidence(_)) => {}
        other => panic!("empty close must WAIT, got {other:?}"),
    }
}

#[test]
fn switch_does_not_invent_verified_or_pass() {
    let (mut floors, _) = set();
    let a = proj("wait-a");
    let b = proj("wait-b");
    floors.open(&a).unwrap();
    floors
        .current_mut()
        .unwrap()
        .open("P", "held", "no verify")
        .unwrap();
    floors.open(&b).unwrap();
    floors.switch(a.to_str().unwrap()).unwrap();
    let parent = &floors.current().unwrap().state().unwrap().parents["P"];
    assert_eq!(parent.state, ParentState::Held);
    assert_ne!(parent.state, ParentState::Verified);
    assert_ne!(parent.state, ParentState::Closed);
    assert!(parent.verify.is_none());
    assert_ne!(
        parent.verify.as_ref().map(|v| v.status),
        Some(EvidenceStatus::Pass)
    );
}

#[test]
fn stores_are_isolated_under_each_project() {
    let (mut floors, recents) = set();
    let a = proj("iso-a");
    let b = proj("iso-b");
    floors.open(&a).unwrap();
    floors.open(&b).unwrap();
    let ha = floors.list().into_iter().find(|h| h.name == "iso-a").unwrap();
    let hb = floors.list().into_iter().find(|h| h.name == "iso-b").unwrap();
    assert!(ha.store.starts_with(&a), "{}", ha.store.display());
    assert!(hb.store.starts_with(&b), "{}", hb.store.display());
    assert_ne!(ha.store, hb.store);
    assert!(recents.is_file());
    let rec = fs::read_to_string(&recents).unwrap();
    assert!(rec.contains("iso-a"));
    assert!(rec.contains("iso-b"));
}
