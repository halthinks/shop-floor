//! Isolated local git worktrees for assigned children.
//!
//! A worktree is Evidence of isolation, not authority to JOIN, VERIFY, or CLOSE.
//! Incomplete isolation is WAIT / INCONCLUSIVE — never a fake directory, fake
//! branch, or fake PASS. Shop does not vendor AASM.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::aasm_map::EvidenceClass;
use crate::github::child_branch_name;
use crate::types::EvidenceStatus;

/// Local isolation checkout. Existence is not JOINED / VERIFIED / PASS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildWorktree {
    pub path: PathBuf,
    pub branch: String,
}

/// Inconclusive isolation attempt. Shop WAIT — never promoted to PASS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeWait {
    pub class: EvidenceClass,
    pub status: EvidenceStatus,
    pub reason: String,
}

impl WorktreeWait {
    fn inconclusive(reason: impl Into<String>) -> Self {
        Self {
            class: EvidenceClass::Inconclusive,
            status: EvidenceStatus::Wait,
            reason: reason.into(),
        }
    }
}

/// `<project>/.shop/worktrees/<parent>/<child>`
pub fn child_worktree_path(project_root: &Path, parent: &str, child: &str) -> PathBuf {
    project_root
        .join(".shop")
        .join("worktrees")
        .join(parent)
        .join(child)
}

/// True only when `git rev-parse` says this directory is a work tree.
pub fn project_is_git_repo(project_root: &Path) -> bool {
    git_stdout(project_root, &["rev-parse", "--is-inside-work-tree"])
        .map(|s| s == "true")
        .unwrap_or(false)
}

/// Create or reuse a real git worktree for one assigned child.
///
/// On success the checkout is at [`child_worktree_path`] on
/// [`child_branch_name`]. That is isolation evidence only.
///
/// Not a git repo, or `git worktree add` failure: [`WorktreeWait`] with
/// [`EvidenceClass::Inconclusive`] / WAIT. Does not mkdir a fake worktree
/// and does not invent a branch.
pub fn ensure_child_worktree(
    project_root: &Path,
    parent: &str,
    child: &str,
) -> std::result::Result<ChildWorktree, WorktreeWait> {
    if !id_safe(parent) || !id_safe(child) {
        return Err(WorktreeWait::inconclusive(
            "parent/child id is not filename-safe; no fake worktree",
        ));
    }

    let dest = child_worktree_path(project_root, parent, child);
    let branch = child_branch_name(parent, child);

    if !project_is_git_repo(project_root) {
        return Err(WorktreeWait::inconclusive(
            "project is not a git repo; worktree add would fail; no fake worktree or branch",
        ));
    }

    if dest.exists() {
        if worktree_on_branch(&dest, &branch) {
            return Ok(ChildWorktree { path: dest, branch });
        }
        return Err(WorktreeWait::inconclusive(format!(
            "path {} exists but is not a git worktree on {branch}; no fake success",
            dest.display()
        )));
    }

    if let Some(parent_dir) = dest.parent() {
        fs::create_dir_all(parent_dir).map_err(|e| {
            WorktreeWait::inconclusive(format!(
                "could not create worktree parent dir: {e}; worktree add would fail"
            ))
        })?;
    }

    if !add_worktree(project_root, &dest, &branch) {
        // git may leave an empty dest; do not treat that as isolation.
        let _ = remove_if_empty_dir(&dest);
        return Err(WorktreeWait::inconclusive(format!(
            "git worktree add failed for {branch} at {}; no fake worktree",
            dest.display()
        )));
    }

    if !worktree_on_branch(&dest, &branch) {
        return Err(WorktreeWait::inconclusive(format!(
            "worktree add exited but {} is not on {branch}; incomplete evidence is WAIT",
            dest.display()
        )));
    }

    Ok(ChildWorktree { path: dest, branch })
}

fn id_safe(id: &str) -> bool {
    !id.is_empty()
        && !id.contains('/')
        && !id.contains('\\')
        && !id.contains("..")
        && !id.chars().any(|c| c.is_whitespace() || c == '\0')
}

fn worktree_on_branch(path: &Path, branch: &str) -> bool {
    git_stdout(path, &["rev-parse", "--is-inside-work-tree"])
        .map(|s| s == "true")
        .unwrap_or(false)
        && git_stdout(path, &["rev-parse", "--abbrev-ref", "HEAD"])
            .map(|s| s == branch)
            .unwrap_or(false)
}

fn add_worktree(project: &Path, dest: &Path, branch: &str) -> bool {
    let dest_s = dest.to_string_lossy();
    if branch_exists(project, branch) {
        git_ok(project, &["worktree", "add", dest_s.as_ref(), branch])
    } else {
        git_ok(project, &["worktree", "add", "-b", branch, dest_s.as_ref()])
    }
}

fn branch_exists(project: &Path, branch: &str) -> bool {
    let spec = format!("refs/heads/{branch}");
    git_ok(project, &["show-ref", "--verify", "--quiet", &spec])
}

fn remove_if_empty_dir(path: &Path) {
    if path.is_dir()
        && fs::read_dir(path)
            .map(|mut i| i.next().is_none())
            .unwrap_or(false)
    {
        let _ = fs::remove_dir(path);
    }
}

fn git_ok(cwd: &Path, args: &[&str]) -> bool {
    git(cwd, args).map(|o| o.status.success()).unwrap_or(false)
}

fn git_stdout(cwd: &Path, args: &[&str]) -> Option<String> {
    let out = git(cwd, args).ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn git(cwd: &Path, args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
}
