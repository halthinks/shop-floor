//! Launch a real worker process.
//!
//! Command source, in order:
//! 1. `SHOP_RUN_CMD` (template string, same tokens as `run.json`)
//! 2. `.shop/run.json` template for the child's intelligence backend
//!
//! Missing or empty command is [`EvidenceClass::Inconclusive`]. Shop never
//! invents a PID and never invents Grok/Cursor CLI flags. Process start and
//! exit are Evidence — they are not `VERIFIED` and never [`EvidenceClass::Pass`].
//!
//! # `.shop/run.json` schema (v1)
//!
//! ```json
//! {
//!   "schema": 1,
//!   "backends": {
//!     "<backend>": "<template>" | ["argv0", "arg", "{token}"]
//!   }
//! }
//! ```
//!
//! * `schema` — optional; `1` is the only version this runner reads.
//! * `backends` — map keyed by intelligence backend id (`cursor`,
//!   `cursor-ultra`, `grok-ultra`, `grok-bot`, `grok-build`). A missing key,
//!   empty string, or empty argv is "no command" for that backend.
//! * A template is either a whitespace-split string or an argv array.
//!   Array form is required when an argument contains spaces.
//!
//! Substitution tokens (replaced in every argv element):
//! `{peer}` `{child}` `{parent}` `{worktree}` `{model}`
//!
//! `cwd` is the worktree path when one is present. This module does not
//! create worktrees and does not wait on the child (that is other lanes).

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use serde::Deserialize;

use crate::aasm_map::EvidenceClass;
use crate::project::is_forbidden_project;

/// Environment override. When set and non-empty, it wins over `run.json`.
pub const SHOP_RUN_CMD: &str = "SHOP_RUN_CMD";

/// File name under the shop store (`<project>/.shop/run.json`).
pub const RUN_JSON: &str = "run.json";

const TOKENS: &[(&str, fn(&RunRequest) -> String)] = &[
    ("{peer}", |r| r.peer.clone()),
    ("{child}", |r| r.child.clone()),
    ("{parent}", |r| r.parent.clone()),
    ("{worktree}", |r| {
        r.worktree
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    }),
    ("{model}", |r| r.model.clone()),
];

/// v1 document stored at `.shop/run.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct RunDocument {
    #[serde(default)]
    pub schema: u32,
    #[serde(default)]
    pub backends: BTreeMap<String, BackendTemplate>,
}

/// Per-backend command template: a line or an argv array.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum BackendTemplate {
    Line(String),
    Argv(Vec<String>),
}

impl BackendTemplate {
    fn is_configured(&self) -> bool {
        match self {
            Self::Line(s) => !s.trim().is_empty(),
            Self::Argv(v) => v.iter().any(|s| !s.trim().is_empty()),
        }
    }
}

/// One launch attempt. Fields are what the caller already knows; shop
/// never fills model, backend, or worktree with invented values.
#[derive(Debug, Clone)]
pub struct RunRequest {
    pub peer: String,
    pub child: String,
    pub parent: String,
    pub backend: String,
    pub model: String,
    pub worktree: Option<PathBuf>,
    /// Shop store directory (the `.shop` folder that may contain `run.json`).
    pub shop_dir: PathBuf,
}

/// Evidence of a launch attempt. Start/exit are not verification.
#[derive(Debug)]
pub struct RunEvidence {
    pub class: EvidenceClass,
    /// Real OS pid when a process was spawned. Always `None` on Inconclusive.
    pub pid: Option<u32>,
    pub argv: Vec<String>,
    pub note: String,
    child: Option<Child>,
}

impl RunEvidence {
    /// Process start/exit never authorizes VERIFIED.
    pub fn is_verified(&self) -> bool {
        false
    }

    pub fn started(&self) -> bool {
        self.pid.is_some()
    }

    pub fn wait(&mut self) -> std::io::Result<Option<i32>> {
        match self.child.take() {
            Some(mut child) => child.wait().map(|s| s.code()),
            None => Ok(None),
        }
    }

    pub fn kill(&mut self) -> std::io::Result<()> {
        match self.child.as_mut() {
            Some(child) => child.kill(),
            None => Ok(()),
        }
    }

    pub fn take_child(&mut self) -> Option<Child> {
        self.child.take()
    }
}

/// Read `SHOP_RUN_CMD` or the backend template from `shop_dir/run.json`,
/// substitute tokens, and spawn. No command → Inconclusive, no PID.
pub fn launch(req: &RunRequest) -> RunEvidence {
    if let Some(path) = forbidden_path(req) {
        return inconclusive(
            Vec::new(),
            format!("refused Platform path: {}", path.display()),
        );
    }

    let Some(template) = resolve_template(req) else {
        return inconclusive(
            Vec::new(),
            "no command configured (SHOP_RUN_CMD or .shop/run.json backend template)".into(),
        );
    };

    let argv = expand_template(&template, req);
    if argv.is_empty() || argv.iter().all(|s| s.is_empty()) {
        return inconclusive(argv, "command template expanded to an empty argv".into());
    }

    match spawn_worker(&argv, req.worktree.as_deref()) {
        Ok(child) => {
            let pid = child.id();
            RunEvidence {
                class: EvidenceClass::Unknown,
                pid: Some(pid),
                argv,
                note: "process start is Evidence, not VERIFIED".into(),
                child: Some(child),
            }
        }
        Err(e) => inconclusive(argv, format!("process failed to start: {e}")),
    }
}

/// Resolve the template without spawning. `None` means unconfigured.
pub fn resolve_template(req: &RunRequest) -> Option<BackendTemplate> {
    if let Some(line) = env_run_cmd() {
        return Some(BackendTemplate::Line(line));
    }
    load_run_document(&req.shop_dir)
        .and_then(|doc| doc.backends.get(&req.backend).cloned())
        .filter(BackendTemplate::is_configured)
}

/// Load `.shop/run.json` from a shop store directory. Invalid JSON is
/// treated as missing — the runner does not invent a command from junk.
pub fn load_run_document(shop_dir: &Path) -> Option<RunDocument> {
    let path = run_json_path(shop_dir);
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn run_json_path(shop_dir: &Path) -> PathBuf {
    if shop_dir.join(RUN_JSON).is_file() || shop_dir.ends_with(".shop") {
        return shop_dir.join(RUN_JSON);
    }
    shop_dir.join(".shop").join(RUN_JSON)
}

pub fn expand_template(template: &BackendTemplate, req: &RunRequest) -> Vec<String> {
    match template {
        BackendTemplate::Line(line) => split_argv(&substitute(line, req)),
        BackendTemplate::Argv(parts) => parts.iter().map(|p| substitute(p, req)).collect(),
    }
}

pub fn substitute(template: &str, req: &RunRequest) -> String {
    let mut out = template.to_string();
    for (token, value) in TOKENS {
        if out.contains(token) {
            out = out.replace(token, &value(req));
        }
    }
    out
}

fn env_run_cmd() -> Option<String> {
    match env::var(SHOP_RUN_CMD) {
        Ok(v) => {
            let t = v.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Err(_) => None,
    }
}

fn split_argv(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_string).collect()
}

fn spawn_worker(argv: &[String], worktree: Option<&Path>) -> std::io::Result<Child> {
    let bin = &argv[0];
    let mut cmd = Command::new(bin);
    if argv.len() > 1 {
        cmd.args(&argv[1..]);
    }
    if let Some(dir) = worktree {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

fn forbidden_path(req: &RunRequest) -> Option<PathBuf> {
    if is_forbidden_project(&req.shop_dir) {
        return Some(req.shop_dir.clone());
    }
    if let Some(wt) = &req.worktree {
        if is_forbidden_project(wt) {
            return Some(wt.clone());
        }
    }
    None
}

fn inconclusive(argv: Vec<String>, note: String) -> RunEvidence {
    RunEvidence {
        class: EvidenceClass::Inconclusive,
        pid: None,
        argv,
        note,
        child: None,
    }
}
