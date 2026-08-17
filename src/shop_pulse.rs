//! Shop pulse rules. Fail-closed leftover naming and merge gate.
//!
//! This file is not a fifth product and not a `workers.rs` lane. AASM stays
//! look-only. Missing evidence is WAIT, never default PASS.
//!
//! Not declared from `src/lib.rs` (that leftover already has an owner).

/// Reserved leftover groups, in order, one owner.
pub const RESERVED: &[&[&str]] = &[
    &["src/engine.rs"],
    &["src/cli.rs"],
    &["src/ui.rs", "src/ui.html", "src/ui.css"],
    &["src/lib.rs"],
];

pub const NEVER_MERGE: &[&str] = &[
    "src/workers.rs",
    "src/runner.rs",
    "src/worktree.rs",
    "src/procwait.rs",
];

/// Historical floor leftovers. Never remmerge.
pub const NEVER_REMMERGE_MIN: u64 = 8;
pub const NEVER_REMMERGE_MAX: u64 = 18;

pub const LINUX_TESTS: &str = "Linux tests";
pub const WINDOWS_RELEASE: &str = "Windows release";

pub const HOLES: &[&str] = &[
    "engine verify/close/handoff cannot invent PASS; missing status is WAIT",
    "CLI suite reads cannot invent PASS; empty steer cannot CLOSE",
    "command-center missing counts/mailbox/boss view stay WAIT; a snapshot cannot VERIFY",
    "crate-root WAIT / missing_evidence never invent PASS, VERIFIED, or CLOSE",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckConclusion {
    Success,
    Failure,
    /// queued, in_progress, absent, or empty conclusion
    Missing,
    Other,
}

impl CheckConclusion {
    /// Missing status/conclusion is WAIT. Never default Success.
    pub fn from_github(status: Option<&str>, conclusion: Option<&str>) -> Self {
        let status = status.map(norm).unwrap_or_default();
        let conclusion = conclusion.map(norm).unwrap_or_default();
        if status.is_empty() && conclusion.is_empty() {
            return Self::Missing;
        }
        if !status.is_empty() && status != "completed" {
            return Self::Missing;
        }
        match conclusion.as_str() {
            "success" => Self::Success,
            "failure" => Self::Failure,
            "" => Self::Missing,
            _ => Self::Other,
        }
    }

    pub fn is_success(self) -> bool {
        matches!(self, Self::Success)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckRun {
    pub name: String,
    pub status: String,
    pub conclusion: CheckConclusion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PulsePr {
    pub number: u64,
    pub title: String,
    pub files: Vec<String>,
    pub checks: Vec<CheckRun>,
    pub merged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PulseFacts {
    pub roadmap: String,
    pub open_prs: Vec<PulsePr>,
    pub merged_prs: Vec<PulsePr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PulseKind {
    Merge,
    CloseWorkers,
    WriteBrief,
    Wait,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PulseDecision {
    pub kind: PulseKind,
    pub leftover: String,
    pub leftover_hole: String,
    pub pr: Option<u64>,
    pub reason: String,
}

impl PulseDecision {
    pub fn is_merge(&self) -> bool {
        self.kind == PulseKind::Merge
    }
}

fn norm(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

pub fn normalize_path(p: &str) -> String {
    p.trim().replace('\\', "/").trim_start_matches("./").to_string()
}

pub fn is_never_remmerge(number: u64) -> bool {
    (NEVER_REMMERGE_MIN..=NEVER_REMMERGE_MAX).contains(&number)
}

pub fn src_write_set(files: &[String]) -> Vec<String> {
    let mut out: Vec<String> = files
        .iter()
        .map(|f| normalize_path(f))
        .filter(|f| f.starts_with("src/"))
        .collect();
    out.sort();
    out.dedup();
    out
}

pub fn touches_never_merge(files: &[String]) -> bool {
    let set = src_write_set(files);
    NEVER_MERGE.iter().any(|p| set.iter().any(|f| f == p))
}

pub fn is_workers_pr(files: &[String]) -> bool {
    src_write_set(files).iter().any(|f| f == "src/workers.rs")
}

/// Reserved paths touched by this write set.
pub fn reserved_touched(files: &[String]) -> Vec<String> {
    let set = src_write_set(files);
    let mut out = Vec::new();
    for group in RESERVED {
        for p in *group {
            if set.iter().any(|f| f == p) {
                out.push((*p).to_string());
            }
        }
    }
    out
}

/// Index of the single reserved group that contains every reserved path in
/// the write set. None if empty, spans two groups, or includes a src/ file
/// outside that group.
pub fn reserved_group_index(files: &[String]) -> Option<usize> {
    let reserved = reserved_touched(files);
    if reserved.is_empty() {
        return None;
    }
    let mut found = None;
    for (i, group) in RESERVED.iter().enumerate() {
        if reserved.iter().all(|p| group.contains(&p.as_str())) {
            found = Some(i);
            break;
        }
    }
    let idx = found?;
    let group = RESERVED[idx];
    let src = src_write_set(files);
    if src.iter().any(|f| !group.contains(&f.as_str())) {
        return None;
    }
    Some(idx)
}

#[allow(dead_code)]
pub fn write_set_is_reserved(files: &[String]) -> bool {
    reserved_group_index(files).is_some()
}

pub fn named_check<'a>(checks: &'a [CheckRun], name: &str) -> Option<&'a CheckRun> {
    checks.iter().find(|c| c.name == name)
}

pub fn gate_conclusion(checks: &[CheckRun], name: &str) -> CheckConclusion {
    match named_check(checks, name) {
        None => CheckConclusion::Missing,
        Some(c) => c.conclusion,
    }
}

/// BOTH required checks must be recorded success. Missing is WAIT.
pub fn both_gates_green(checks: &[CheckRun]) -> bool {
    gate_conclusion(checks, LINUX_TESTS).is_success()
        && gate_conclusion(checks, WINDOWS_RELEASE).is_success()
}

pub fn roadmap_ok(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    !text.trim().is_empty()
        && t.contains("aasm")
        && (t.contains("look-only") || t.contains("look only"))
        && (t.contains("do not vendor") || t.contains("does not vendor"))
}

/// Why this open PR must not merge. None = gate open.
pub fn merge_block(pr: &PulsePr) -> Option<&'static str> {
    if pr.merged {
        return Some("already merged");
    }
    if is_never_remmerge(pr.number) {
        return Some("never remmerge PRs #8–#18");
    }
    if is_workers_pr(&pr.files) {
        return Some("never merge src/workers.rs");
    }
    if touches_never_merge(&pr.files) {
        return Some("never merge runner/worktree/procwait");
    }
    if reserved_group_index(&pr.files).is_none() {
        return Some("write set is not exactly one reserved leftover");
    }
    if !both_gates_green(&pr.checks) {
        return Some("WAIT: Linux tests and Windows release are not both success");
    }
    None
}

pub fn may_merge(pr: &PulsePr) -> bool {
    merge_block(pr).is_none()
}

pub fn unused_groups(merged: &[PulsePr]) -> Vec<usize> {
    let mut used = [false; 4];
    for pr in merged {
        if !pr.merged {
            continue;
        }
        if is_never_remmerge(pr.number) {
            continue;
        }
        if touches_never_merge(&pr.files) {
            continue;
        }
        if let Some(i) = reserved_group_index(&pr.files) {
            used[i] = true;
        }
    }
    (0..RESERVED.len()).filter(|i| !used[*i]).collect()
}

pub fn leftover_label(idx: usize) -> String {
    RESERVED[idx].join("+")
}

pub fn leftover_hole(idx: usize) -> &'static str {
    HOLES.get(idx).copied().unwrap_or("fail-closed hole unnamed")
}

/// Empty leftover is forbidden while a reserved path is unused.
pub fn leftover_name_allowed(name: &str, unused: &[usize]) -> bool {
    unused.is_empty() || !name.trim().is_empty()
}

pub fn name_leftover(unused: &[usize]) -> (String, String) {
    match unused.first().copied() {
        None => (
            String::new(),
            "no unused reserved leftover; do not invent a fifth product".into(),
        ),
        Some(i) => (leftover_label(i), leftover_hole(i).to_string()),
    }
}

pub fn decide(facts: &PulseFacts) -> PulseDecision {
    if !roadmap_ok(&facts.roadmap) {
        return PulseDecision {
            kind: PulseKind::Wait,
            leftover: "WAIT".into(),
            leftover_hole: "SUITE_ROADMAP missing or not look-only; never default PASS".into(),
            pr: None,
            reason: "WAIT: suite roadmap evidence missing; pulse will not merge".into(),
        };
    }

    let unused = unused_groups(&facts.merged_prs);
    let (leftover, leftover_hole) = name_leftover(&unused);
    debug_assert!(
        leftover_name_allowed(&leftover, &unused),
        "leftover name cannot be empty while a reserved path is unused"
    );

    if let Some(pr) = facts.open_prs.iter().find(|p| is_workers_pr(&p.files)) {
        return PulseDecision {
            kind: PulseKind::CloseWorkers,
            leftover,
            leftover_hole,
            pr: Some(pr.number),
            reason: format!(
                "close invented workers.rs PR #{}; never merge that lane",
                pr.number
            ),
        };
    }

    if let Some(idx) = unused.first().copied() {
        let open_for = facts.open_prs.iter().find(|p| {
            reserved_group_index(&p.files) == Some(idx) && !is_never_remmerge(p.number)
        });
        match open_for {
            Some(pr) if may_merge(pr) => PulseDecision {
                kind: PulseKind::Merge,
                leftover: leftover_label(idx),
                leftover_hole: leftover_hole.clone(),
                pr: Some(pr.number),
                reason: format!(
                    "lander: reserved leftover {} is green; merge PR #{} without a chat",
                    leftover_label(idx),
                    pr.number
                ),
            },
            Some(pr) => PulseDecision {
                kind: PulseKind::Wait,
                leftover,
                leftover_hole,
                pr: Some(pr.number),
                reason: format!(
                    "WAIT: reserved PR #{}: {}",
                    pr.number,
                    merge_block(pr).unwrap_or("incomplete evidence")
                ),
            },
            None => PulseDecision {
                kind: PulseKind::WriteBrief,
                leftover,
                leftover_hole,
                pr: None,
                reason: "no reserved PR open; name the next unused leftover".into(),
            },
        }
    } else {
        PulseDecision {
            kind: PulseKind::WriteBrief,
            leftover,
            leftover_hole,
            pr: None,
            reason: "all reserved leftovers have a green merged PR; do not invent a fifth product"
                .into(),
        }
    }
}

pub fn render_brief(facts: &PulseFacts, decision: &PulseDecision) -> String {
    let unused = unused_groups(&facts.merged_prs);
    let mut out = String::from("# Next leftover\n\n");
    if decision.leftover.is_empty() {
        out.push_str("Named leftover: _(none)_\n\n");
        out.push_str(
            "All reserved suite leftovers have a green merged PR. Do not invent a fifth product. Do not invent a `src/workers.rs` lane.\n\n",
        );
    } else {
        out.push_str(&format!("Named leftover: `{}`\n\n", decision.leftover));
        out.push_str(&format!("Fail-closed hole: {}\n\n", decision.leftover_hole));
    }
    out.push_str("Reserved leftover paths, in order, one owner:\n\n");
    for (i, group) in RESERVED.iter().enumerate() {
        let state = if unused.contains(&i) {
            "UNUSED"
        } else {
            "used on main"
        };
        out.push_str(&format!("1. `{}` — {state}\n", group.join("` + `")));
    }
    out.push_str("\nThe shop pulse is the lander. Orchestrator is not in the loop.\n\n");
    out.push_str("- MERGE only when both `Linux tests` and `Windows release` conclusions are success, and the `src/` write set is exactly one reserved leftover (or a subset of one reserved group).\n");
    out.push_str("- NEVER merge `src/workers.rs`, `src/runner.rs`, `src/worktree.rs`, `src/procwait.rs`.\n");
    out.push_str("- NEVER remmerge PRs #8–#18. Close invented `workers.rs` PRs.\n");
    out.push_str("- AASM stays look-only. Do not vendor AASM. Do not write `C:\\TextPCB Platform`. Do not open a TextPCB product lane.\n");
    out.push_str("- Missing evidence is WAIT, never default PASS.\n");
    out.push_str(&format!("\nPulse: {} — {}\n", kind_word(decision.kind), decision.reason));
    out
}

fn kind_word(k: PulseKind) -> &'static str {
    match k {
        PulseKind::Merge => "MERGE",
        PulseKind::CloseWorkers => "CLOSE_WORKERS",
        PulseKind::WriteBrief => "WRITE_BRIEF",
        PulseKind::Wait => "WAIT",
    }
}

pub fn kind_word_pub(k: PulseKind) -> &'static str {
    kind_word(k)
}
