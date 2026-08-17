//! Fail-closed shop pulse: merge gate + leftover-must-name.
//! Missing evidence is WAIT, never default PASS.
//! Leftover names come from the SUITE_ROADMAP suite-modules table.

#[path = "../src/shop_pulse.rs"]
mod shop_pulse;

use shop_pulse::*;

fn roadmap() -> String {
    fs_roadmap().unwrap_or_else(|| {
        "Shop is the suite. AASM is the look-only kernel. Shop does not vendor AASM.\n\n\
         ## Suite modules (named next, disjoint)\n\n\
         | Order | Lane | Path | Do |\n\
         | --- | --- | --- | --- |\n\
         | 1 | shop-suite-roadmap | `docs/SUITE_ROADMAP.md` | This file. Suite contract only. |\n\
         | 2 | shop-aasm-map | `src/aasm_map.rs` | Keep the proving-ground map tight. Look-only. |\n\
         | 3 | shop-proving | `src/proving.rs` | Thin proving helpers only. Never invent PASS. |\n\
         | 4 | shop-awareness | `src/awareness.rs` | Missing count is None / WAIT. |\n\
         | 5 | shop-mailbox | `src/mailbox.rs` | Mailbox adapter only. |\n"
            .into()
    })
}

fn fs_roadmap() -> Option<String> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/SUITE_ROADMAP.md");
    std::fs::read_to_string(p).ok()
}

fn leftovers_of(text: &str) -> LeftoverSet {
    LeftoverSet::from_roadmap(text)
}

fn live_leftovers() -> LeftoverSet {
    leftovers_of(&roadmap())
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

/// Floor leftovers already landed. Not named suite rows.
fn landed_floor() -> Vec<PulsePr> {
    vec![
        pr(20, &["src/engine.rs"], green_checks(), true),
        pr(21, &["src/cli.rs"], green_checks(), true),
        pr(22, &["src/ui.rs", "src/ui.html", "src/ui.css"], green_checks(), true),
        pr(24, &["src/lib.rs"], green_checks(), true),
    ]
}

/// Green merged PRs outside the remmerge window for every named suite path.
fn landed_named(set: &LeftoverSet) -> Vec<PulsePr> {
    set.groups
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let files: Vec<&str> = g.paths.iter().map(String::as_str).collect();
            pr(30 + i as u64, &files, green_checks(), true)
        })
        .collect()
}

fn insert_named_suite_row(roadmap: &str, path: &str, lane: &str) -> String {
    let section = "Suite modules (named next, disjoint)";
    let start = roadmap
        .find(section)
        .expect("fixture must keep the suite-modules heading");
    let after = &roadmap[start..];
    let rel_end = after.find("\n## ").unwrap_or(after.len());
    let abs_end = start + rel_end;
    let section_body = &roadmap[start..abs_end];
    let mut last_order = 0u32;
    for line in section_body.lines() {
        let cols: Vec<&str> = line.split('|').map(str::trim).collect();
        if cols.len() < 2 {
            continue;
        }
        if let Ok(n) = cols[1].parse::<u32>() {
            last_order = last_order.max(n);
        }
    }
    let row = format!(
        "\n| {} | {lane} | `{path}` | Fixture hole; missing evidence is WAIT. |\n",
        last_order + 1
    );
    let mut out = String::new();
    out.push_str(&roadmap[..abs_end]);
    out.push_str(&row);
    out.push_str(&roadmap[abs_end..]);
    out
}

#[test]
fn green_named_leftover_pr_may_merge() {
    let set = live_leftovers();
    let open = pr(40, &["src/aasm_map.rs"], green_checks(), false);
    assert!(set.write_set_is_leftover(&open.files));
    assert_eq!(kind_word_pub(PulseKind::Merge), "MERGE");
    assert!(
        may_merge(&open, &set),
        "green named aasm_map PR must be allowed to merge"
    );
    let facts = PulseFacts {
        roadmap: roadmap(),
        open_prs: vec![open],
        merged_prs: vec![],
    };
    let d = decide(&facts);
    assert_eq!(d.kind, PulseKind::Merge);
    assert_eq!(d.pr, Some(40));
    assert_eq!(d.leftover, "src/aasm_map.rs");
    assert!(d.is_merge());
}

#[test]
fn leftover_group_subset_may_merge() {
    let mut text = roadmap();
    text = insert_named_suite_row(&text, "src/ui.rs", "shop-ui-group");
    // Make the new row a group by rewriting the last inserted path cell.
    text = text.replace(
        "| shop-ui-group | `src/ui.rs` |",
        "| shop-ui-group | `src/ui.rs` + `src/ui.html` + `src/ui.css` |",
    );
    let set = leftovers_of(&text);
    let open = pr(41, &["src/ui.rs"], green_checks(), false);
    assert!(
        set.group_index(&open.files).is_some(),
        "subset of one leftover group is allowed"
    );
    assert!(may_merge(&open, &set));
}

#[test]
fn ungated_incomplete_ci_must_not_merge() {
    let set = live_leftovers();
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
    let open = pr(42, &["src/aasm_map.rs"], queued, false);
    assert!(!both_gates_green(&open.checks));
    assert!(!may_merge(&open, &set));
    assert_eq!(
        merge_block(&open, &set),
        Some("WAIT: Linux tests and Windows release are not both success")
    );
    let facts = PulseFacts {
        roadmap: roadmap(),
        open_prs: vec![open],
        merged_prs: vec![],
    };
    let d = decide(&facts);
    assert_eq!(d.kind, PulseKind::Wait);
    assert!(!d.is_merge());
    assert_eq!(d.leftover, "src/aasm_map.rs");
}

#[test]
fn one_green_check_is_wait_never_pass() {
    let set = live_leftovers();
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
    let open = pr(43, &["src/proving.rs"], only_linux, false);
    assert!(!may_merge(&open, &set));
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
    let set = live_leftovers();
    let open = pr(50, &["src/workers.rs"], green_checks(), false);
    assert!(is_workers_pr(&open.files));
    assert!(!may_merge(&open, &set));
    assert_eq!(merge_block(&open, &set), Some("never merge src/workers.rs"));
    let facts = PulseFacts {
        roadmap: roadmap(),
        open_prs: vec![open],
        merged_prs: landed_floor(),
    };
    let d = decide(&facts);
    assert_eq!(d.kind, PulseKind::CloseWorkers);
    assert_eq!(d.pr, Some(50));
    assert!(!d.is_merge());
    assert!(
        !d.leftover.is_empty(),
        "closing workers.rs must still name the unused suite leftover"
    );
}

#[test]
fn refill_paths_must_not_merge() {
    let set = live_leftovers();
    for path in ["src/runner.rs", "src/worktree.rs", "src/procwait.rs"] {
        let open = pr(51, &[path], green_checks(), false);
        assert!(touches_never_merge(&open.files), "{path}");
        assert!(!may_merge(&open, &set), "{path} must not merge");
        assert_eq!(
            merge_block(&open, &set),
            Some("never merge runner/worktree/procwait")
        );
    }
}

#[test]
fn remmerge_8_through_29_must_not_merge() {
    let set = live_leftovers();
    for n in 8..=29 {
        let open = pr(n, &["src/aasm_map.rs"], green_checks(), false);
        assert!(is_never_remmerge(n));
        assert!(!may_merge(&open, &set), "PR #{n} must not remmerge");
        assert_eq!(merge_block(&open, &set), Some("never remmerge PRs #8–#29"));
    }
}

#[test]
fn leftover_name_cannot_be_empty_while_named_unused() {
    let set = live_leftovers();
    let unused = set.unused(&[]);
    assert!(
        !unused.is_empty(),
        "live suite table must still have unused named paths"
    );
    let (name, hole) = set.name_leftover(&unused);
    assert!(
        !name.is_empty(),
        "leftover name cannot be empty while a named unused SUITE_ROADMAP path exists"
    );
    assert_eq!(name, set.label(unused[0]));
    assert_eq!(name, "src/aasm_map.rs");
    assert!(!hole.is_empty());
    assert!(leftover_name_allowed(&name, &unused));
    assert!(
        !leftover_name_allowed("", &unused),
        "empty leftover while unused named path is forbidden"
    );
    assert!(
        !leftover_name_allowed("   ", &unused),
        "whitespace leftover is still empty"
    );
}

#[test]
fn leftover_empty_only_after_all_named_used() {
    let set = live_leftovers();
    let unused = set.unused(&landed_named(&set));
    assert!(unused.is_empty());
    let (name, hole) = set.name_leftover(&unused);
    assert!(name.is_empty());
    assert!(hole.contains("do not invent a fifth product"));
    assert!(leftover_name_allowed(&name, &unused));
}

#[test]
fn two_leftover_groups_in_one_pr_cannot_merge() {
    let set = live_leftovers();
    let open = pr(
        60,
        &["src/aasm_map.rs", "src/proving.rs"],
        green_checks(),
        false,
    );
    assert!(set.group_index(&open.files).is_none());
    assert!(!may_merge(&open, &set));
}

#[test]
fn write_brief_names_first_unused_named_path() {
    let facts = PulseFacts {
        roadmap: roadmap(),
        open_prs: vec![],
        merged_prs: landed_floor(),
    };
    let d = decide(&facts);
    assert_eq!(d.kind, PulseKind::WriteBrief);
    assert_eq!(d.leftover, "src/aasm_map.rs");
    assert!(!d.leftover.is_empty());
    let brief = render_brief(&facts, &d);
    assert!(brief.contains("src/aasm_map.rs"));
    assert!(brief.contains("UNUSED"));
    assert!(brief.contains("Orchestrator is not in the loop"));
    assert!(!brief.contains("Named leftover: _(none)_"));
}

#[test]
fn green_named_pr_still_merges_after_path_already_used() {
    let set = live_leftovers();
    let open = pr(70, &["src/aasm_map.rs"], green_checks(), false);
    assert!(may_merge(&open, &set));
    let facts = PulseFacts {
        roadmap: roadmap(),
        open_prs: vec![open],
        merged_prs: landed_named(&set),
    };
    let d = decide(&facts);
    assert_eq!(d.kind, PulseKind::Merge);
    assert_eq!(d.pr, Some(70));
    assert!(
        d.leftover.is_empty(),
        "do not invent a leftover name outside the table"
    );
}

#[test]
fn live_suite_after_floor_land_names_first_unused_module() {
    let facts = PulseFacts {
        roadmap: roadmap(),
        open_prs: vec![],
        merged_prs: landed_floor(),
    };
    let d = decide(&facts);
    assert_eq!(d.kind, PulseKind::WriteBrief);
    assert_eq!(d.leftover, "src/aasm_map.rs");
    assert!(!d.leftover.is_empty());
    let brief = render_brief(&facts, &d);
    assert!(brief.contains("src/aasm_map.rs"));
    assert!(brief.contains("workers.rs"));
    assert!(!brief.contains("C:\\TextPCB Platform") || brief.contains("Do not write"));
}

#[test]
fn leftover_follows_new_roadmap_row_without_reserved_array() {
    let base = roadmap();
    let base_set = leftovers_of(&base);
    assert!(
        !base_set.groups.iter().any(|g| g.contains("src/future_suite.rs")),
        "future path must not be hardcoded"
    );
    let used = landed_named(&base_set);
    let before = decide(&PulseFacts {
        roadmap: base.clone(),
        open_prs: vec![],
        merged_prs: used.clone(),
    });
    assert!(
        before.leftover.is_empty(),
        "_(none)_ is legal only when every named suite path has a green merged PR"
    );

    let extended = insert_named_suite_row(&base, "src/future_suite.rs", "shop-future");
    let after_set = leftovers_of(&extended);
    assert!(
        after_set.groups.iter().any(|g| g.contains("src/future_suite.rs")),
        "new named row must be discovered from the roadmap fixture"
    );
    let after = decide(&PulseFacts {
        roadmap: extended,
        open_prs: vec![],
        merged_prs: used,
    });
    assert_eq!(after.kind, PulseKind::WriteBrief);
    assert_eq!(after.leftover, "src/future_suite.rs");
    assert!(!after.leftover.is_empty());
}

#[test]
fn workers_named_in_roadmap_still_cannot_merge() {
    let poisoned = insert_named_suite_row(&roadmap(), "src/workers.rs", "shop-workers");
    let set = leftovers_of(&poisoned);
    assert!(
        !set.groups.iter().any(|g| g.contains("src/workers.rs")),
        "workers.rs is forbidden, not a leftover"
    );
    let open = pr(80, &["src/workers.rs"], green_checks(), false);
    assert!(!may_merge(&open, &set));
    assert_eq!(merge_block(&open, &set), Some("never merge src/workers.rs"));
}

#[test]
fn missing_roadmap_is_wait_never_merge() {
    let facts = PulseFacts {
        roadmap: String::new(),
        open_prs: vec![pr(40, &["src/aasm_map.rs"], green_checks(), false)],
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
    assert!(suite_table_ok(&text));
    assert!(
        !text.to_ascii_lowercase().contains("vendor aasm")
            || text.contains("does not vendor")
            || text.contains("Do not vendor")
    );
    let set = leftovers_of(&text);
    let names: Vec<String> = set.groups.iter().map(LeftoverGroup::label).collect();
    assert!(names.contains(&"src/aasm_map.rs".into()));
    assert!(names.contains(&"src/proving.rs".into()));
    assert!(names.contains(&"src/awareness.rs".into()));
    assert!(names.contains(&"src/mailbox.rs".into()));
    assert!(!names.iter().any(|n| n.contains("workers.rs")));
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
