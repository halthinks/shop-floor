//! Shop pulse runner. Reads suite roadmap + live GitHub facts, then lands
//! or names the next reserved leftover. Orchestrator is not in the loop.

#[path = "../shop_pulse.rs"]
mod shop_pulse;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use serde_json::{json, Value};
use shop_pulse::{
    decide, kind_word_pub, render_brief, CheckConclusion, CheckRun, PulseFacts, PulseKind, PulsePr,
};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        eprintln!(
            "shop-pulse live [--apply] [--repo OWNER/NAME] [--roadmap PATH] [--brief PATH]\n\
             shop-pulse decide --facts FACTS.json [--brief PATH]"
        );
        return ExitCode::SUCCESS;
    }
    match args[0].as_str() {
        "decide" => run_decide(&args[1..]),
        "live" => run_live(&args[1..]),
        other => {
            eprintln!("WAIT: unknown shop-pulse command {other}; never default PASS");
            ExitCode::SUCCESS
        }
    }
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == name {
            return args.get(i + 1).map(|s| s.as_str());
        }
        i += 1;
    }
    None
}

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn run_decide(args: &[String]) -> ExitCode {
    let Some(path) = flag(args, "--facts") else {
        eprintln!("WAIT: --facts missing; never default PASS");
        return ExitCode::SUCCESS;
    };
    let raw = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("WAIT: facts unreadable: {e}");
            return ExitCode::SUCCESS;
        }
    };
    let v: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("WAIT: facts JSON: {e}");
            return ExitCode::SUCCESS;
        }
    };
    let facts = match facts_from_json(&v) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("WAIT: {e}");
            return ExitCode::SUCCESS;
        }
    };
    finish(&facts, has(args, "--apply"), flag(args, "--brief"))
}

fn run_live(args: &[String]) -> ExitCode {
    let repo = flag(args, "--repo")
        .map(str::to_string)
        .or_else(|| env::var("GITHUB_REPOSITORY").ok())
        .unwrap_or_else(|| "halthinks/shop-floor".into());
    let roadmap_path = flag(args, "--roadmap").unwrap_or("docs/SUITE_ROADMAP.md");
    let brief_path = flag(args, "--brief").unwrap_or("docs/NEXT_LEFTOVER.md");
    let roadmap = match fs::read_to_string(roadmap_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("WAIT: cannot read {roadmap_path}: {e}; never default PASS");
            return ExitCode::SUCCESS;
        }
    };
    let facts = match gather_live(&repo, roadmap) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("WAIT: live GitHub facts: {e}; never default PASS");
            return ExitCode::SUCCESS;
        }
    };
    finish(&facts, has(args, "--apply"), Some(brief_path))
}

fn finish(facts: &PulseFacts, apply: bool, brief: Option<&str>) -> ExitCode {
    let decision = decide(facts);
    let payload = json!({
        "kind": kind_word_pub(decision.kind),
        "leftover": decision.leftover,
        "leftover_hole": decision.leftover_hole,
        "pr": decision.pr,
        "reason": decision.reason,
    });
    println!("{}", serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into()));
    if decision.is_merge() {
        eprintln!("pulse decision: MERGE");
    }

    if decision.kind == PulseKind::WriteBrief || brief.is_some() {
        if let Some(path) = brief {
            let text = render_brief(facts, &decision);
            if let Err(e) = write_brief(path, &text) {
                eprintln!("WAIT: leftover brief: {e}");
            } else {
                eprintln!("wrote leftover brief {path}");
            }
        }
    }

    if !apply {
        return ExitCode::SUCCESS;
    }
    match decision.kind {
        PulseKind::Merge => {
            let Some(n) = decision.pr else {
                eprintln!("WAIT: merge without PR number");
                return ExitCode::SUCCESS;
            };
            apply_merge(n);
        }
        PulseKind::CloseWorkers => {
            let Some(n) = decision.pr else {
                eprintln!("WAIT: close without PR number");
                return ExitCode::SUCCESS;
            };
            apply_close_workers(n);
        }
        PulseKind::WriteBrief | PulseKind::Wait => {}
    }
    ExitCode::SUCCESS
}

fn write_brief(path: &str, text: &str) -> Result<(), String> {
    if let Some(parent) = PathBuf::from(path).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    fs::write(path, text).map_err(|e| e.to_string())
}

fn apply_merge(n: u64) {
    let status = Command::new("gh")
        .args(["pr", "merge", &n.to_string(), "--merge"])
        .status();
    match status {
        Ok(s) if s.success() => eprintln!("merged PR #{n}"),
        Ok(s) => eprintln!("WAIT: gh pr merge #{n} exit {s}"),
        Err(e) => eprintln!("WAIT: gh missing for merge: {e}"),
    }
}

fn apply_close_workers(n: u64) {
    let status = Command::new("gh")
        .args([
            "pr",
            "close",
            &n.to_string(),
            "--comment",
            "shop pulse: invented workers.rs lane; never merge. Orchestrator is not in the loop.",
        ])
        .status();
    match status {
        Ok(s) if s.success() => eprintln!("closed invented workers.rs PR #{n}"),
        Ok(s) => eprintln!("WAIT: gh pr close #{n} exit {s}"),
        Err(e) => eprintln!("WAIT: gh missing for close: {e}"),
    }
}

fn gather_live(repo: &str, roadmap: String) -> Result<PulseFacts, String> {
    let open_raw = gh_json(&[
        "pr",
        "list",
        "--repo",
        repo,
        "--state",
        "open",
        "--limit",
        "50",
        "--json",
        "number,title,files,statusCheckRollup,state",
    ])?;
    let merged_raw = gh_json(&[
        "pr",
        "list",
        "--repo",
        repo,
        "--state",
        "merged",
        "--limit",
        "50",
        "--json",
        "number,title,files,state",
    ])?;
    let mut facts = PulseFacts {
        roadmap,
        open_prs: prs_from_list(&open_raw, false)?,
        merged_prs: prs_from_list(&merged_raw, true)?,
    };
    for pr in facts.open_prs.iter_mut() {
        if pr.files.is_empty() || pr.checks.is_empty() {
            if let Ok(extra) = view_pr(repo, pr.number) {
                if pr.files.is_empty() {
                    pr.files = extra.files;
                }
                if pr.checks.is_empty() {
                    pr.checks = extra.checks;
                }
            }
        }
    }
    for pr in facts.merged_prs.iter_mut() {
        if pr.files.is_empty() {
            if let Ok(extra) = view_pr(repo, pr.number) {
                pr.files = extra.files;
            }
        }
    }
    Ok(facts)
}

fn view_pr(repo: &str, number: u64) -> Result<PulsePr, String> {
    let raw = gh_json(&[
        "pr",
        "view",
        &number.to_string(),
        "--repo",
        repo,
        "--json",
        "number,title,files,statusCheckRollup,state,mergedAt",
    ])?;
    pr_from_value(&raw, raw.get("mergedAt").map(|v| !v.is_null()).unwrap_or(false))
}

fn gh_json(args: &[&str]) -> Result<Value, String> {
    let out = Command::new("gh")
        .args(args)
        .output()
        .map_err(|e| format!("gh not available: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "gh {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("gh JSON: {e}"))
}

fn prs_from_list(v: &Value, merged: bool) -> Result<Vec<PulsePr>, String> {
    let arr = v
        .as_array()
        .ok_or_else(|| "WAIT: PR list is not an array".to_string())?;
    let mut out = Vec::new();
    for item in arr {
        out.push(pr_from_value(item, merged)?);
    }
    Ok(out)
}

fn facts_from_json(v: &Value) -> Result<PulseFacts, String> {
    let roadmap = v
        .get("roadmap")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let open_prs = v
        .get("open_prs")
        .map(|x| prs_from_list(x, false))
        .transpose()?
        .unwrap_or_default();
    let merged_prs = v
        .get("merged_prs")
        .map(|x| prs_from_list(x, true))
        .transpose()?
        .unwrap_or_default();
    Ok(PulseFacts {
        roadmap,
        open_prs,
        merged_prs,
    })
}

fn pr_from_value(v: &Value, merged: bool) -> Result<PulsePr, String> {
    let number = v
        .get("number")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| "WAIT: PR number missing".to_string())?;
    let title = v
        .get("title")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let files = files_from(v);
    let checks = checks_from(v);
    Ok(PulsePr {
        number,
        title,
        files,
        checks,
        merged,
    })
}

fn files_from(v: &Value) -> Vec<String> {
    let Some(arr) = v.get("files").and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|f| {
            f.as_str()
                .map(|s| s.to_string())
                .or_else(|| {
                    f.get("path")
                        .and_then(|p| p.as_str())
                        .map(|s| s.to_string())
                })
                .or_else(|| {
                    f.get("filename")
                        .and_then(|p| p.as_str())
                        .map(|s| s.to_string())
                })
        })
        .collect()
}

fn checks_from(v: &Value) -> Vec<CheckRun> {
    let arr = v
        .get("statusCheckRollup")
        .or_else(|| v.get("check_runs"))
        .or_else(|| v.get("checks"))
        .and_then(|x| x.as_array());
    let Some(arr) = arr else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|c| {
            let name = c.get("name").and_then(|x| x.as_str())?.to_string();
            let status = c
                .get("status")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let conclusion = c.get("conclusion").and_then(|x| x.as_str());
            Some(CheckRun {
                name,
                conclusion: CheckConclusion::from_github(Some(&status), conclusion),
                status,
            })
        })
        .collect()
}
