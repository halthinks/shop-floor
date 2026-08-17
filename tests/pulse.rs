//! Fail-closed shop pulse: merge gate + leftover-must-name.
//! Missing evidence is WAIT, never default PASS.

#[path = "../src/shop_pulse.rs"]
mod shop_pulse;

use shop_pulse::*;

fn roadmap() -> String {
    fs_roadmap().unwrap_or_else(|| {
        "Shop is the suite. AASM is the look-only kernel. Shop does not vendor AASM."
            .into()
    })
}

fn fs_roadmap() -> Option<String> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/SUITE_ROADMAP.md");
    std::fs::read_to_string(p).ok()
}

fn green_checks() -> Vec<CheckRun> {
    vec![
        CheckRun {
            name: LINUX_TESTS.into(),
            status: "completed".into(),
            conclusion: CheckConclusion::Success,
        },
        CheckRun {
            name: WINDOWS_RELEASE.into(),
            status: "completed".into(),
            conclusion: CheckConclusion::Success,
        },
    ]
}

fn pr(number: u64, files: &[&str], checks: Vec<CheckRun>, merged: bool) -> PulsePr {
    PulsePr {
        number,
        title: format!("pr-{number}"),
        files: files.iter().map(|s| (*s).to_string()).collect(),
        checks,
        merged,
    }
}

fn landed_reserved() -> Vec<PulsePr> {
    vec![
        pr(20, &["src/engine.rs"], green_checks(), true),
        pr(21, &["src/cli.rs"], green_checks(), true),
        pr(22, &["src/ui.rs", "src/ui.html", "src/ui.css"], green_checks(), true),
        pr(24, &["src/lib.rs"], green_checks(), true),
    ]
}

#[test]
fn green_reserved_pr_may_merge() {
    let open = pr(30, &["src/engine.rs"], green_checks(), false);
    assert!(write_set_is_reserved(&open.files));
    assert_eq!(kind_word_pub(PulseKind::Merge), "MERGE");
    assert!(may_merge(&open), "green reserved engine PR must be allowed to merge");
    let facts = PulseFacts {
        roadmap: roadmap(),
        open_prs: vec![open],
        merged_prs: vec![],
    };
    let d = decide(&facts);
    assert_eq!(d.kind, PulseKind::Merge);
    assert_eq!(d.pr, Some(30));
    assert_eq!(d.leftover, "src/engine.rs");
    assert!(d.is_merge());
}

#[test]
fn ui_subset_is_one_reserved_group() {
    let open = pr(31, &["src/ui.rs"], green_checks(), false);
    assert_eq!(reserved_group_index(&open.files), Some(2));
    assert!(may_merge(&open));
    let facts = PulseFacts {
        roadmap: roadmap(),
        open_prs: vec![open],
        merged_prs: vec![
            pr(20, &["src/engine.rs"], green_checks(), true),
            pr(21, &["src/cli.rs"], green_checks(), true),
        ],
    };
    let d = decide(&facts);
    assert_eq!(d.kind, PulseKind::Merge);
    assert_eq!(d.leftover, "src/ui.rs+src/ui.html+src/ui.css");
}

#[test]
fn ungated_incomplete_ci_must_not_merge() {
    let queued = vec![
        CheckRun {
            name: LINUX_TESTS.into(),
            status: "queued".into(),
            conclusion: CheckConclusion::Missing,
        },
        CheckRun {
            name: WINDOWS_RELEASE.into(),
            status: "queued".into(),
            conclusion: CheckConclusion::Missing,
        },
    ];
    let open = pr(32, &["src/lib.rs"], queued, false);
    assert!(!both_gates_green(&open.checks));
    assert!(!may_merge(&open));
    assert_eq!(
        merge_block(&open),
        Some("WAIT: Linux tests and Windows release are not both success")
    );
    let facts = PulseFacts {
        roadmap: roadmap(),
        open_prs: vec![open],
        merged_prs: vec![
            pr(20, &["src/engine.rs"], green_checks(), true),
            pr(21, &["src/cli.rs"], green_checks(), true),
            pr(22, &["src/ui.rs", "src/ui.html", "src/ui.css"], green_checks(), true),
        ],
    };
    let d = decide(&facts);
    assert_eq!(d.kind, PulseKind::Wait);
    assert!(!d.is_merge());
    assert_eq!(d.leftover, "src/lib.rs");
}

#[test]
fn one_green_check_is_wait_never_pass() {
    let only_linux = vec![CheckRun {
        name: LINUX_TESTS.into(),
        status: "completed".into(),
        conclusion: CheckConclusion::Success,
    }];
    assert_eq!(
        gate_conclusion(&only_linux, WINDOWS_RELEASE),
        CheckConclusion::Missing
    );
    assert!(!both_gates_green(&only_linux));
    let open = pr(33, &["src/cli.rs"], only_linux, false);
    assert!(!may_merge(&open));
}

#[test]
fn missing_check_never_defaults_to_pass() {
    assert_eq!(
        CheckConclusion::from_github(None, None),
        CheckConclusion::Missing
    );
    assert!(!CheckConclusion::from_github(Some("completed"), None).is_success());
    assert!(!CheckConclusion::from_github(Some("in_progress"), Some("success")).is_success());
    assert_eq!(
        CheckConclusion::from_github(Some("COMPLETED"), Some("SUCCESS")),
        CheckConclusion::Success
    );
    assert!(!both_gates_green(&[]));
}

#[test]
fn workers_rs_must_not_merge_and_is_closed() {
    let open = pr(40, &["src/workers.rs"], green_checks(), false);
    assert!(is_workers_pr(&open.files));
    assert!(!may_merge(&open));
    assert_eq!(merge_block(&open), Some("never merge src/workers.rs"));
    let facts = PulseFacts {
        roadmap: roadmap(),
        open_prs: vec![open],
        merged_prs: landed_reserved(),
    };
    let d = decide(&facts);
    assert_eq!(d.kind, PulseKind::CloseWorkers);
    assert_eq!(d.pr, Some(40));
    assert!(!d.is_merge());
}

#[test]
fn refill_paths_must_not_merge() {
    for path in ["src/runner.rs", "src/worktree.rs", "src/procwait.rs"] {
        let open = pr(41, &[path], green_checks(), false);
        assert!(touches_never_merge(&open.files), "{path}");
        assert!(!may_merge(&open), "{path} must not merge");
        assert_eq!(
            merge_block(&open),
            Some("never merge runner/worktree/procwait")
        );
    }
}

#[test]
fn remmerge_8_through_18_must_not_merge() {
    for n in 8..=18 {
        let open = pr(n, &["src/engine.rs"], green_checks(), false);
        assert!(is_never_remmerge(n));
        assert!(!may_merge(&open), "PR #{n} must not remmerge");
        assert_eq!(merge_block(&open), Some("never remmerge PRs #8–#18"));
    }
}

#[test]
fn leftover_name_cannot_be_empty_while_reserved_unused() {
    let unused = unused_groups(&[]);
    assert_eq!(unused, vec![0, 1, 2, 3]);
    let (name, hole) = name_leftover(&unused);
    assert!(!name.is_empty(), "must name src/engine.rs");
    assert_eq!(name, "src/engine.rs");
    assert!(!hole.is_empty());
    assert!(leftover_name_allowed(&name, &unused));
    assert!(
        !leftover_name_allowed("", &unused),
        "empty leftover while unused reserved path is forbidden"
    );
    assert!(
        !leftover_name_allowed("   ", &unused),
        "whitespace leftover is still empty"
    );
}

#[test]
fn leftover_empty_only_after_all_reserved_used() {
    let unused = unused_groups(&landed_reserved());
    assert!(unused.is_empty());
    let (name, hole) = name_leftover(&unused);
    assert!(name.is_empty());
    assert!(hole.contains("do not invent a fifth product"));
    assert!(leftover_name_allowed(&name, &unused));
}

#[test]
fn two_reserved_groups_in_one_pr_cannot_merge() {
    let open = pr(
        50,
        &["src/engine.rs", "src/lib.rs"],
        green_checks(),
        false,
    );
    assert!(reserved_group_index(&open.files).is_none());
    assert!(!may_merge(&open));
}

#[test]
fn write_brief_when_no_reserved_pr_open() {
    let facts = PulseFacts {
        roadmap: roadmap(),
        open_prs: vec![],
        merged_prs: vec![
            pr(20, &["src/engine.rs"], green_checks(), true),
            pr(21, &["src/cli.rs"], green_checks(), true),
        ],
    };
    let d = decide(&facts);
    assert_eq!(d.kind, PulseKind::WriteBrief);
    assert_eq!(d.leftover, "src/ui.rs+src/ui.html+src/ui.css");
    assert!(!d.leftover.is_empty());
    let brief = render_brief(&facts, &d);
    assert!(brief.contains("src/ui.rs"));
    assert!(brief.contains("UNUSED"));
    assert!(brief.contains("Orchestrator is not in the loop"));
    assert!(brief.contains("fail-closed") || brief.contains("WAIT"));
}

#[test]
fn green_reserved_pr_still_merges_after_path_already_used() {
    let open = pr(25, &["src/cli.rs"], green_checks(), false);
    assert!(may_merge(&open));
    let facts = PulseFacts {
        roadmap: roadmap(),
        open_prs: vec![open],
        merged_prs: landed_reserved(),
    };
    let d = decide(&facts);
    assert_eq!(d.kind, PulseKind::Merge);
    assert_eq!(d.pr, Some(25));
    assert!(d.leftover.is_empty(), "do not invent a fifth leftover name");
}

#[test]
fn live_suite_after_lib_land_does_not_invent_fifth() {
    let facts = PulseFacts {
        roadmap: roadmap(),
        open_prs: vec![],
        merged_prs: landed_reserved(),
    };
    let d = decide(&facts);
    assert_eq!(d.kind, PulseKind::WriteBrief);
    assert!(d.leftover.is_empty());
    assert!(d.leftover_hole.contains("do not invent a fifth product"));
    let brief = render_brief(&facts, &d);
    assert!(brief.contains("Do not invent a fifth product"));
    assert!(brief.contains("workers.rs"));
    assert!(!brief.contains("C:\\TextPCB Platform") || brief.contains("Do not write"));
}

#[test]
fn missing_roadmap_is_wait_never_merge() {
    let facts = PulseFacts {
        roadmap: String::new(),
        open_prs: vec![pr(30, &["src/engine.rs"], green_checks(), false)],
        merged_prs: vec![],
    };
    let d = decide(&facts);
    assert_eq!(d.kind, PulseKind::Wait);
    assert!(!d.is_merge());
    assert!(d.reason.contains("WAIT"));
}

#[test]
fn suite_roadmap_on_disk_is_look_only() {
    let text = fs_roadmap().expect("docs/SUITE_ROADMAP.md must exist");
    assert!(roadmap_ok(&text));
    assert!(!text.to_ascii_lowercase().contains("vendor aasm") || text.contains("does not vendor") || text.contains("Do not vendor"));
}

#[test]
fn pulse_workflow_is_scheduled() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".github/workflows/shop-pulse.yml");
    let yml = std::fs::read_to_string(&path).expect("shop-pulse workflow must exist");
    assert!(yml.contains("schedule:"), "workflow must be scheduled");
    assert!(yml.contains("workflow_dispatch"), "workflow must be dispatchable");
    assert!(yml.contains("shop-pulse") || yml.contains("shop pulse"));
    assert!(yml.contains("Linux tests") || yml.contains("shop-pulse"));
}
