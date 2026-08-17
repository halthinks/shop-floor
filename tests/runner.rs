//! Runner gates: a real process from SHOP_RUN_CMD or `.shop/run.json`,
//! and Inconclusive with no fake PID when nothing is configured.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use shop::runner::{
    expand_template, launch, load_run_document, resolve_template, substitute, BackendTemplate,
    RunRequest, SHOP_RUN_CMD,
};
use shop::EvidenceClass;

static ENV: Mutex<()> = Mutex::new(());

fn lock_env() -> MutexGuard<'static, ()> {
    ENV.lock().unwrap_or_else(|e| e.into_inner())
}

fn tmp_shop() -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = env::temp_dir().join(format!("shop-runner-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn isolated() -> (MutexGuard<'static, ()>, PathBuf) {
    let guard = lock_env();
    env::remove_var(SHOP_RUN_CMD);
    (guard, tmp_shop())
}

fn request(shop_dir: PathBuf, worktree: Option<PathBuf>) -> RunRequest {
    RunRequest {
        peer: "alice".into(),
        child: "c1".into(),
        parent: "P".into(),
        backend: "grok-bot".into(),
        model: "kept-empty-by-caller".into(),
        worktree,
        shop_dir,
    }
}

#[test]
fn unconfigured_is_inconclusive_without_fake_pid() {
    let (_lock, shop_dir) = isolated();
    let req = request(shop_dir, None);

    assert!(resolve_template(&req).is_none());
    let ev = launch(&req);

    assert_eq!(ev.class, EvidenceClass::Inconclusive);
    assert_eq!(ev.pid, None, "unconfigured launch must not invent a PID");
    assert!(!ev.started());
    assert!(!ev.is_verified());
    assert_ne!(ev.class, EvidenceClass::Pass);
    assert!(ev.note.contains("no command configured"));
}

#[test]
fn shop_run_cmd_starts_a_real_process() {
    let (_lock, shop_dir) = isolated();
    let worktree = shop_dir.join("wt");
    fs::create_dir_all(&worktree).unwrap();
    let marker = worktree.join("started");

    env::set_var(SHOP_RUN_CMD, "touch {worktree}/started");

    let req = request(shop_dir, Some(worktree.clone()));
    let mut ev = launch(&req);

    assert!(ev.started(), "configured SHOP_RUN_CMD must spawn");
    let pid = ev.pid.expect("real pid");
    assert!(pid > 0);
    assert_ne!(ev.class, EvidenceClass::Pass);
    assert!(!ev.is_verified(), "process start is Evidence, not VERIFIED");
    assert_eq!(ev.class, EvidenceClass::Unknown);

    let code = ev.wait().unwrap();
    assert_eq!(code, Some(0));
    assert!(
        marker.is_file(),
        "worker process must have created {}",
        marker.display()
    );
}

#[test]
fn run_json_backend_template_starts_a_real_process() {
    let (_lock, shop_dir) = isolated();
    let worktree = shop_dir.join("tree");
    fs::create_dir_all(&worktree).unwrap();

    let doc = serde_json::json!({
        "schema": 1,
        "backends": {
            "grok-bot": [
                "sh",
                "-c",
                "printf '%s' '{peer}|{child}|{parent}|{model}' > out.txt"
            ]
        }
    });
    fs::write(
        shop_dir.join("run.json"),
        serde_json::to_vec_pretty(&doc).unwrap(),
    )
    .unwrap();

    let loaded = load_run_document(&shop_dir).expect("run.json");
    assert_eq!(loaded.schema, 1);
    assert!(loaded.backends.contains_key("grok-bot"));

    let req = request(shop_dir, Some(worktree.clone()));
    let tmpl = resolve_template(&req).expect("backend template");
    let argv = expand_template(&tmpl, &req);
    assert_eq!(argv[0], "sh");
    assert!(argv[2].contains("alice|c1|P|kept-empty-by-caller"));
    assert!(!argv[2].contains("{peer}"));

    let mut ev = launch(&req);
    assert!(
        ev.pid.is_some(),
        "run.json template must spawn a real process"
    );
    assert_ne!(ev.class, EvidenceClass::Pass);
    assert!(!ev.is_verified());
    ev.wait().unwrap();

    let out = fs::read_to_string(worktree.join("out.txt")).unwrap();
    assert_eq!(out, "alice|c1|P|kept-empty-by-caller");
}

#[test]
fn empty_backend_template_is_inconclusive() {
    let (_lock, shop_dir) = isolated();
    let doc = serde_json::json!({
        "schema": 1,
        "backends": {
            "grok-bot": "",
            "cursor": []
        }
    });
    fs::write(
        shop_dir.join("run.json"),
        serde_json::to_vec_pretty(&doc).unwrap(),
    )
    .unwrap();

    let ev = launch(&request(shop_dir, None));
    assert_eq!(ev.class, EvidenceClass::Inconclusive);
    assert_eq!(ev.pid, None);
}

#[test]
fn substitute_replaces_every_token() {
    let req = RunRequest {
        peer: "p".into(),
        child: "c".into(),
        parent: "par".into(),
        backend: "cursor".into(),
        model: "m".into(),
        worktree: Some(PathBuf::from("/wt")),
        shop_dir: PathBuf::from("/tmp/shop"),
    };
    assert_eq!(
        substitute("{peer} {child} {parent} {worktree} {model}", &req),
        "p c par /wt m"
    );
    match expand_template(&BackendTemplate::Line("bin {peer} {child}".into()), &req).as_slice() {
        [bin, peer, child] => {
            assert_eq!(bin, "bin");
            assert_eq!(peer, "p");
            assert_eq!(child, "c");
        }
        other => panic!("unexpected argv {other:?}"),
    }
}
