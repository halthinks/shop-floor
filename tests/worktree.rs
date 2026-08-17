//! Isolated git worktrees for assigned children.
//! Incomplete evidence is WAIT, never fake PASS. A worktree is not authority.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use shop::types::{ChildState, EvidenceStatus, ParentState};
use shop::worktree::{
    child_worktree_path, ensure_child_worktree, project_is_git_repo, ChildWorktree,
};
use shop::{EvidenceClass, Shop};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp_dir() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("shop-wt-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn git(cwd: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git")
}

fn git_ok(cwd: &Path, args: &[&str]) {
    let out = git(cwd, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git_stdout(cwd: &Path, args: &[&str]) -> String {
    let out = git(cwd, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn init_git_project() -> PathBuf {
    let dir = tmp_dir();
    git_ok(&dir, &["init"]);
    git_ok(&dir, &["config", "user.email", "shop@test"]);
    git_ok(&dir, &["config", "user.name", "shop-test"]);
    fs::write(dir.join(".gitignore"), ".shop/\n").unwrap();
    git_ok(&dir, &["add", ".gitignore"]);
    git_ok(&dir, &["commit", "-m", "init"]);
    dir
}

fn open_shop(project: &Path) -> Shop {
    let recents = tmp_dir().join("recents.json");
    Shop::open_project_with_recents(project, recents).unwrap()
}

#[test]
fn git_repo_assign_creates_worktree_on_child_branch() {
    let project = init_git_project();
    let mut shop = open_shop(&project);
    assert_eq!(shop.project_root(), project.as_path());
    shop.open("P", "parent", "body").unwrap();
    shop.split("P", "c1", "cursor", "src/a", "one", "do a")
        .unwrap();
    shop.assign("P").unwrap();

    let dest = child_worktree_path(&project, "P", "c1");
    assert!(
        dest.is_dir(),
        "expected real worktree at {}",
        dest.display()
    );
    assert_eq!(
        git_stdout(&dest, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "shop/P/c1"
    );
    assert!(project_is_git_repo(&dest));

    let isolated = ensure_child_worktree(&project, "P", "c1").expect("reuse existing worktree");
    assert_eq!(
        isolated,
        ChildWorktree {
            path: dest,
            branch: "shop/P/c1".into(),
        }
    );
}

#[test]
fn non_git_project_is_inconclusive_wait_no_fake_worktree() {
    let project = tmp_dir();
    assert!(!project_is_git_repo(&project));
    let dest = child_worktree_path(&project, "P", "c1");
    let err = ensure_child_worktree(&project, "P", "c1").expect_err("non-git must WAIT");
    assert_eq!(err.class, EvidenceClass::Inconclusive);
    assert_eq!(err.status, EvidenceStatus::Wait);
    assert!(err.class.is_shop_wait());
    assert_ne!(err.class, EvidenceClass::Pass);
    assert!(!dest.exists(), "must not mkdir a fake worktree");
    assert!(!project.join(".shop/worktrees").exists());
}

#[test]
fn worktree_existence_is_not_authority() {
    let project = init_git_project();
    let mut shop = open_shop(&project);
    shop.open("P", "parent", "body").unwrap();
    shop.split("P", "c1", "cursor", "src/a", "one", "do a")
        .unwrap();
    shop.assign("P").unwrap();

    assert!(child_worktree_path(&project, "P", "c1").is_dir());

    let parent = shop.state().unwrap().parents["P"].clone();
    let child = &parent.children["c1"];
    assert_eq!(child.state, ChildState::Assigned);
    assert_ne!(child.state, ChildState::Joined);
    assert_ne!(parent.state, ParentState::Verified);
    assert_ne!(parent.state, ParentState::Closed);
    assert_ne!(parent.state, ParentState::ReduceReady);
    assert!(parent.verify.is_none());
    assert_ne!(
        parent.verify.as_ref().map(|v| v.status),
        Some(EvidenceStatus::Pass)
    );
}

#[test]
fn empty_git_repo_worktree_add_failure_is_wait() {
    let project = tmp_dir();
    git_ok(&project, &["init"]);
    assert!(project_is_git_repo(&project));
    let dest = child_worktree_path(&project, "P", "c1");
    let err = ensure_child_worktree(&project, "P", "c1").expect_err("no HEAD is WAIT");
    assert_eq!(err.class, EvidenceClass::Inconclusive);
    assert_eq!(err.status, EvidenceStatus::Wait);
    assert!(!dest.exists(), "failed add must not leave a fake worktree");
}
