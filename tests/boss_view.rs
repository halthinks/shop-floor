//! Live floor view for SuperGrokHeavy.
//! Incomplete evidence is WAIT, never fake PASS. A picture is not authority.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use shop::floor::{write_floor, FloorChild, FloorCurrent, FloorHandoff, FloorMemory};
use shop::memory::ShopMemory;
use shop::project::is_forbidden_project;
use shop::types::{ChildState, Claim, EvidenceStatus, ParentState};
use shop::EvidenceClass;
use shop::STEER_TO;

#[path = "../src/boss_view.rs"]
mod boss_view;

use boss_view::{
    capture, capture_and_write, json_is_live_view, json_is_memory_pack, load_view, render_ppm,
    write_view, BossView, IMAGE_HEIGHT, IMAGE_WIDTH, IMAGE_REL, KIND, SCHEMA, SCREEN_REL,
    VIEW_REL,
};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp_dir() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("shop-boss-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn store() -> PathBuf {
    let p = tmp_dir().join(".shop");
    fs::create_dir_all(&p).unwrap();
    p
}

fn child(
    id: &str,
    parent: &str,
    peer: &str,
    state: ChildState,
    handoff: Option<FloorHandoff>,
) -> FloorChild {
    FloorChild {
        id: id.into(),
        parent_id: parent.into(),
        peer: peer.into(),
        paths: vec!["src/a.rs".into()],
        state,
        bounce_reason: None,
        backjump_note: None,
        handoff,
    }
}

fn live_floor() -> FloorMemory {
    FloorMemory {
        current: Some(FloorCurrent {
            id: "P".into(),
            state: ParentState::InFlight,
            title: "run workers".into(),
            body: "do the work".into(),
        }),
        children: vec![child(
            "c1",
            "P",
            "alice",
            ChildState::Assigned,
            None,
        )],
        claims: vec![Claim {
            parent_id: "P".into(),
            child_id: "c1".into(),
            peer: "alice".into(),
            paths: vec!["src/a.rs".into()],
        }],
        history: Vec::new(),
    }
}

fn assert_not_authority(view: &BossView) {
    assert_eq!(view.status, EvidenceStatus::Wait);
    assert_ne!(view.status, EvidenceStatus::Pass);
    assert_ne!(view.class, EvidenceClass::Pass);
    assert!(!view.is_pass());
    assert!(view.class.is_shop_wait());
}

#[test]
fn missing_floor_is_wait_no_invented_parent() {
    let store = store();
    let view = capture(&store);
    assert_not_authority(&view);
    assert_eq!(view.class, EvidenceClass::InformationGap);
    assert!(view.current.is_none(), "must not invent a parent");
    assert!(view.children.is_empty(), "must not invent children");
    assert!(view.wait.as_deref().unwrap_or("").contains("no floor"));
    assert_eq!(view.kind, KIND);
    assert_eq!(view.to, STEER_TO);
}

#[test]
fn live_snapshot_is_not_a_memory_pack_or_steer() {
    let store = store();
    write_floor(&store, &live_floor()).unwrap();

    let view = capture(&store);
    assert_not_authority(&view);
    assert_eq!(view.kind, KIND);
    assert_eq!(view.schema, SCHEMA);
    assert_eq!(view.to, STEER_TO);
    assert_eq!(view.current.as_ref().map(|c| c.id.as_str()), Some("P"));
    assert_eq!(
        view.current.as_ref().map(|c| c.state.as_str()),
        Some("IN_FLIGHT")
    );
    assert_eq!(view.children.len(), 1);
    assert_eq!(view.children[0].state, "ASSIGNED");
    assert!(view.screen.contains("SHOP FLOOR — LIVE VIEW"));
    assert!(view.screen.contains("PARENT  P  IN_FLIGHT"));
    assert!(view.screen.contains("c1"));

    let json = view.to_json();
    assert!(json_is_live_view(&json), "must be a live floor view");
    assert!(
        !json_is_memory_pack(&json),
        "must not be the steer memory pack"
    );
    assert!(!json.contains("\"profile\""));
    assert!(!json.contains("\"log\""));
    assert!(!json.contains("\"memory\""));
    assert!(!json.contains("\"type\": \"steer\""));
    assert!(json.contains(&format!("\"kind\": \"{KIND}\"")));

    let pack = ShopMemory::default().format();
    assert!(
        pack.contains("PROFILE") || pack.contains("LOG"),
        "memory pack text stays a memory pack, not a live view"
    );
    assert!(!json_is_live_view(&pack));
}

#[test]
fn joined_without_handoff_is_wait_not_shown_as_joined() {
    let store = store();
    let mut floor = live_floor();
    floor.children = vec![child("c1", "P", "alice", ChildState::Joined, None)];
    write_floor(&store, &floor).unwrap();

    let view = capture(&store);
    assert_not_authority(&view);
    assert_eq!(view.class, EvidenceClass::InformationGap);
    assert_eq!(view.children.len(), 1);
    assert_eq!(view.children[0].state, "ASSIGNED");
    assert!(view.children[0].joined_without_handoff);
    assert_ne!(view.children[0].state, "JOINED");
    assert!(view
        .wait
        .as_deref()
        .unwrap_or("")
        .contains("JOINED without handoff"));
}

#[test]
fn joined_with_handoff_is_shown_joined_but_still_not_pass() {
    let store = store();
    let mut floor = live_floor();
    floor.children = vec![child(
        "c1",
        "P",
        "alice",
        ChildState::Joined,
        Some(FloorHandoff {
            hash: "fnv1a64:deadbeef:len=1".into(),
            status: "DONE".into(),
        }),
    )];
    write_floor(&store, &floor).unwrap();

    let view = capture(&store);
    assert_not_authority(&view);
    assert_eq!(view.children[0].state, "JOINED");
    assert!(!view.children[0].joined_without_handoff);
    assert_eq!(
        view.children[0].handoff_hash.as_deref(),
        Some("fnv1a64:deadbeef:len=1")
    );
}

#[test]
fn writes_snapshot_and_image_under_boss_not_steer() {
    let store = store();
    write_floor(&store, &live_floor()).unwrap();

    let (view, written) = capture_and_write(&store).expect("write live view");
    assert_not_authority(&view);
    assert!(written.view.ends_with(Path::new(VIEW_REL)));
    assert!(written.screen.ends_with(Path::new(SCREEN_REL)));
    assert!(written.image.ends_with(Path::new(IMAGE_REL)));
    assert!(written.view.is_file());
    assert!(written.screen.is_file());
    assert!(written.image.is_file());
    assert!(!store.join("outbox").exists() || fs::read_dir(store.join("outbox")).map(|d| d.count()).unwrap_or(0) == 0);

    let disk = fs::read_to_string(&written.view).unwrap();
    assert!(json_is_live_view(&disk));
    assert!(!json_is_memory_pack(&disk));

    let screen = fs::read_to_string(&written.screen).unwrap();
    assert!(screen.contains("LIVE VIEW"));
    assert_eq!(screen, view.screen);

    let ppm = fs::read(&written.image).unwrap();
    let header = String::from_utf8_lossy(&ppm);
    assert!(header.starts_with("P3\n"), "PPM image for the boss");
    assert!(header.contains(&format!("{IMAGE_WIDTH} {IMAGE_HEIGHT}")));
    assert!(ppm.len() > 64, "image must have pixels, not just a header");

    let loaded = load_view(&store).expect("load written view");
    assert_eq!(loaded.kind, KIND);
    assert_not_authority(&loaded);
}

#[test]
fn ppm_render_is_a_real_image() {
    let store = store();
    write_floor(&store, &live_floor()).unwrap();
    let view = capture(&store);
    let ppm = render_ppm(&view);
    let text = String::from_utf8(ppm).expect("P3 is ASCII");
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("P3"));
    assert_eq!(
        lines.next(),
        Some(format!("{IMAGE_WIDTH} {IMAGE_HEIGHT}").as_str())
    );
    assert_eq!(lines.next(), Some("255"));
}

#[test]
fn platform_path_is_refused_no_write() {
    let platform = PathBuf::from(r"C:\TextPCB Platform");
    assert!(is_forbidden_project(&platform));
    let view = capture(&platform);
    assert_not_authority(&view);
    assert_eq!(view.class, EvidenceClass::Inconclusive);
    assert!(view.current.is_none());
    assert!(view
        .wait
        .as_deref()
        .unwrap_or("")
        .contains("TextPCB Platform"));

    let err = write_view(&platform, &view).expect_err("must not write Platform");
    let msg = err.to_string();
    assert!(
        msg.contains("TextPCB") || msg.contains("refuses"),
        "forbidden write: {msg}"
    );
}

#[test]
fn missing_on_disk_view_is_wait_not_invented() {
    let store = store();
    let err = load_view(&store).expect_err("no file");
    let msg = err.to_string();
    assert!(msg.contains("WAIT"), "{msg}");
    assert!(!store.join(VIEW_REL).exists());
}

#[test]
fn bounced_child_is_shown_bounced_still_wait() {
    let store = store();
    let mut floor = live_floor();
    let mut bounced = child("c1", "P", "alice", ChildState::Bounced, None);
    bounced.bounce_reason = Some("path overlap".into());
    floor.children = vec![bounced];
    write_floor(&store, &floor).unwrap();

    let view = capture(&store);
    assert_not_authority(&view);
    assert_eq!(view.children.len(), 1);
    assert_eq!(view.children[0].state, "BOUNCED");
    assert_eq!(
        view.children[0].bounce_reason.as_deref(),
        Some("path overlap")
    );
    assert!(!view.children[0].joined_without_handoff);
    assert!(view.screen.contains("BOUNCED"));
    assert!(view.screen.contains("bounce=path overlap"));
}

#[test]
fn complete_snapshot_is_unknown_never_pass() {
    let store = store();
    write_floor(&store, &live_floor()).unwrap();
    let view = capture(&store);
    assert_not_authority(&view);
    assert_eq!(view.class, EvidenceClass::Unknown);
    assert_eq!(view.status, EvidenceStatus::Wait);
    assert!(view.wait.is_none());
    assert!(view.screen.contains("live view is Evidence, not PASS"));
}

#[test]
fn verified_floor_current_does_not_become_view_pass() {
    let store = store();
    let mut floor = live_floor();
    floor.current = Some(FloorCurrent {
        id: "P".into(),
        state: ParentState::Verified,
        title: "done?".into(),
        body: String::new(),
    });
    write_floor(&store, &floor).unwrap();
    let view = capture(&store);
    assert_not_authority(&view);
    assert_eq!(
        view.current.as_ref().map(|c| c.state.as_str()),
        Some("VERIFIED")
    );
    assert_eq!(view.class, EvidenceClass::InformationGap);
    assert!(view
        .wait
        .as_deref()
        .unwrap_or("")
        .contains("not verify authority"));
}
