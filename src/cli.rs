//! Thin clap front-end. All sequencing lives in the library.
//!
//! Suite reads (`awareness`, `mailbox`, `prove`, `workers`) print store truth.
//! Suite waits (unread assign, missing pid, missing GitHub counts) stay WAIT.
//! Close / merge / accept never invent PASS. Handoff is Evidence, not CLOSE.
//! Missing evidence is WAIT. This file cannot CLOSE a parent or invent PASS.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use crate::awareness::Awareness;
use crate::engine::Shop;
use crate::mailbox::{resolve_mailbox, MailboxObservation, DEFAULT_FROM};
use crate::procwait::ProcWaitLedger;
use crate::proving;
use crate::skills_live::LiveGitHub;
use crate::types::{EvidenceStatus, Parent, ParentState, ReducePackage};
use crate::ui::{self, DEFAULT_PORT};
use crate::workers::WorkerRoster;

#[derive(Parser, Debug)]
#[command(
    name = "shop",
    version,
    about = "Shop-floor hold-split-join sequencer",
    long_about = "Open shop ui, add a worker, assign AI intelligence, then hold-split-join.\n\
                  shop awareness / mailbox / prove / workers print store truth.\n\
                  Not AASM. Not T3 Code. Incomplete evidence is WAIT, never fake PASS."
)]
#[command(arg_required_else_help = true)]
pub struct Cli {
    /// Shop store directory
    #[arg(long, global = true, default_value = ".shop")]
    pub store: PathBuf,

    /// Optional TextPCB-style mailbox directory
    #[arg(long, global = true)]
    pub mailbox: Option<PathBuf>,

    /// Assign-record `from` field
    #[arg(long, global = true, default_value = DEFAULT_FROM)]
    pub from: String,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Create a shop store (default `.shop/` in cwd)
    Init { dir: Option<PathBuf> },
    /// Open the command center
    Ui {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    /// Alias for `shop ui`
    Serve {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    /// Enroll a worker (thin CLI for the same store write as the UI)
    Add {
        #[arg(long)]
        peer: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        backend: String,
        #[arg(long)]
        model: Option<String>,
        /// Grant the GitHub skill pack (default on)
        #[arg(long, default_value_t = true)]
        github_skills: bool,
        #[arg(long)]
        no_github_skills: bool,
    },
    /// Connect a GitHub repo (local record; token stays in .shop/)
    Github {
        #[command(subcommand)]
        cmd: GithubCmd,
    },
    /// Create a parent in state HELD
    Open {
        #[arg(long)]
        id: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        body: String,
        /// Optional claimed paths (parent-subset lease). Empty: children define claims.
        #[arg(long)]
        paths: Option<String>,
    },
    /// Add a child. REJECT overlapping in-flight allowed_paths.
    Split {
        parent: String,
        #[arg(long)]
        child: String,
        #[arg(long)]
        peer: String,
        /// Comma-separated allowed_paths
        #[arg(long)]
        paths: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        body: String,
    },
    /// Emit mailbox-shaped assign records for undispatched ASSIGNED children
    Assign {
        parent: String,
        /// Launch configured processes after the worktree hook
        #[arg(long)]
        run: bool,
    },
    /// Launch the configured process for an assigned child
    Run {
        parent: String,
        #[arg(long)]
        child: String,
    },
    /// Record a child handoff (PASS/DONE). Child -> JOINED
    Accept {
        parent: String,
        #[arg(long)]
        child: String,
        #[arg(long)]
        handoff: String,
    },
    /// Fail closed. Child -> BOUNCED, same peer and paths
    Bounce {
        parent: String,
        #[arg(long)]
        child: String,
        #[arg(long)]
        reason: String,
    },
    /// Print parent + children + claims
    Status { parent: Option<String> },
    /// Truth panel. Missing pieces are WAIT, never invented zeros.
    Awareness {
        parent: Option<String>,
        /// Emit JSON (jobs + merged waits). Default is text.
        #[arg(long)]
        json: bool,
    },
    /// List enrolled workers. Capacity only; a roster name is not a running worker.
    Workers,
    /// Mailbox observation. Missing inbox is WAIT. Unread assign is HOLD, not PASS.
    Mailbox,
    /// Fail-closed proving-ground readout. Missing parent is WAIT, never PASS.
    Prove { parent: String },
    /// Set worker intelligence (capacity only; does not launch a process)
    Intel {
        #[arg(long)]
        peer: String,
        #[arg(long)]
        backend: String,
        #[arg(long)]
        model: Option<String>,
    },
    /// Steer SuperGrokHeavy. Empty text is WAIT. Does not CLOSE a parent.
    Steer {
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
    },
    /// Print the process ledger. A roster name is not a live pid.
    Processes,
    /// Join barrier: all JOINED -> REDUCE_READY; else stay HELD/IN_FLIGHT
    Join { parent: String },
    /// Parent -> REDUCED only from REDUCE_READY. Writes a reduce package.
    Reduce {
        parent: String,
        #[arg(long)]
        note: String,
    },
    /// Run a command; PASS -> VERIFIED. Fail or missing evidence -> WAIT
    Verify {
        parent: String,
        #[arg(long)]
        cmd: Option<String>,
    },
    /// Close only from VERIFIED
    Close { parent: String },
    /// Merge the reduce PR. Blocked until verify PASS / VERIFIED. No prompt.
    Merge { parent: String },
    /// Wait on a live process. Exit is Evidence only — never VERIFIED/CLOSED.
    Wait {
        pid: u32,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        child: Option<String>,
        #[arg(long)]
        peer: Option<String>,
    },
    /// Explicit operator stop. Waiting is not stopping.
    Stop { pid: u32 },
    /// Open a local project folder or list recents
    Project {
        #[command(subcommand)]
        cmd: ProjectCmd,
    },
    /// Print the event tail (same events as /feed.xml)
    Log {
        #[arg(long, default_value_t = 40)]
        n: usize,
    },
    /// Store a standing fact or a dated episode
    Remember {
        #[arg(long, default_value = "profile")]
        tier: String,
        fact: String,
    },
    /// Remove an exact profile fact
    Forget { fact: String },
    /// Print shop profile + last log lines
    Memory,
    /// Print floor memory (current job, children, claims)
    Floor,
    /// Stay running with the UI and ingest SuperGrokHeavy replies
    Listen {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub enum GithubCmd {
    /// Record owner/name (+ optional token file)
    Connect {
        /// owner/name
        repo: String,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ProjectCmd {
    /// Open a local folder as the shop project (`<path>/.shop`)
    Open { path: PathBuf },
    /// Print user-level recents (`~/.shop/recents.json`)
    Recent,
}

pub fn run() -> Result<()> {
    run_cli(Cli::parse())
}

pub fn run_cli(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Init { dir } => {
            let root = dir.unwrap_or(cli.store);
            let mailbox = resolve_mailbox(cli.mailbox, &root);
            let shop = Shop::init(&root)?
                .with_mailbox(Some(mailbox))
                .with_from(cli.from);
            println!("initialized shop store {}", shop.store_root().display());
            Ok(())
        }
        Commands::Project { cmd } => match cmd {
            ProjectCmd::Open { path } => {
                let shop = Shop::open_project(&path)?;
                println!(
                    "opened project {} store {}",
                    shop.project_root().display(),
                    shop.store_root().display()
                );
                Ok(())
            }
            ProjectCmd::Recent => {
                print!(
                    "{}",
                    crate::project::format_recents(&crate::project::default_recents_path())
                );
                Ok(())
            }
        },
        Commands::Ui { port } | Commands::Serve { port } | Commands::Listen { port } => {
            let mailbox = resolve_mailbox(cli.mailbox, &cli.store);
            let shop = ui::ensure_store(cli.store.clone())?
                .with_mailbox(Some(mailbox))
                .with_from(cli.from);
            let live = std::sync::Arc::new(LiveGitHub::detect(shop.store_root()));
            let shop = shop.with_github(live);
            ui::run(shop, port).map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(())
        }
        other => {
            let mailbox = resolve_mailbox(cli.mailbox, &cli.store);
            let mut shop = Shop::open_store(&cli.store)
                .with_context(|| format!("open store {}", cli.store.display()))?
                .with_mailbox(Some(mailbox))
                .with_from(cli.from);
            let live = std::sync::Arc::new(LiveGitHub::detect(shop.store_root()));
            shop = shop.with_github(live);
            dispatch(&mut shop, other)
        }
    }
}

fn dispatch(shop: &mut Shop, command: Commands) -> Result<()> {
    match command {
        Commands::Init { .. }
        | Commands::Ui { .. }
        | Commands::Serve { .. }
        | Commands::Listen { .. }
        | Commands::Project { .. } => unreachable!(),
        Commands::Log { n } => {
            print!("{}", shop.event_log(n));
        }
        Commands::Remember { tier, fact } => {
            let tier = crate::memory::MemoryTier::parse(&tier)?;
            shop.remember(tier, &fact)?;
            println!("remembered ({}) {fact}", tier.as_str());
        }
        Commands::Forget { fact } => {
            shop.forget(&fact)?;
            println!("forgot {fact}");
        }
        Commands::Memory => {
            print!("{}", shop.memory_text());
        }
        Commands::Floor => {
            print!("{}", shop.floor_text());
        }
        Commands::Add {
            peer,
            name,
            title,
            backend,
            model,
            github_skills,
            no_github_skills,
        } => {
            let skills = github_skills && !no_github_skills;
            let shown = title.as_deref().or(name.as_deref()).unwrap_or(&peer);
            let w = shop.add_worker(
                &peer,
                shown,
                &backend,
                model.as_deref().unwrap_or(""),
                skills,
            )?;
            println!(
                "added worker {} backend={} model={} github_skills={} (capacity only)",
                w.peer, w.intelligence.backend, w.intelligence.model, w.github_skills
            );
        }
        Commands::Github { cmd } => match cmd {
            GithubCmd::Connect {
                repo,
                branch,
                token,
            } => {
                let r = shop.github_connect(&repo, branch.as_deref(), token.as_deref())?;
                println!("connected GitHub repo {}", r.slug());
            }
        },
        Commands::Open {
            id,
            title,
            body,
            paths,
        } => {
            shop.open_with_scope(&id, &title, &body, paths.as_deref().unwrap_or(""))?;
            println!("opened parent {id} in HELD");
        }
        Commands::Split {
            parent,
            child,
            peer,
            paths,
            title,
            body,
        } => {
            shop.split(&parent, &child, &peer, &paths, &title, &body)?;
            println!("split child {child} on {parent} -> ASSIGNED (peer={peer} paths={paths})");
        }
        Commands::Assign { parent, run } => {
            let recs = if run {
                shop.assign_and_run(&parent)?
            } else {
                shop.assign(&parent)?
            };
            if recs.is_empty() {
                println!("no undispatched ASSIGNED children on {parent}");
            } else {
                println!(
                    "assigned {} record(s) for {parent} -> {}/outbox",
                    recs.len(),
                    shop.store_root().display()
                );
            }
        }
        Commands::Run { parent, child } => {
            let ev = shop.run_child(&parent, &child)?;
            match ev.pid {
                Some(pid) => println!(
                    "run {parent}/{child}: pid {pid} (start is Evidence, not VERIFIED); {}",
                    ev.note
                ),
                None => println!(
                    "run {parent}/{child}: {} ({}); assign kept, no fake pid",
                    ev.class.as_str(),
                    ev.note
                ),
            }
        }
        Commands::Accept {
            parent,
            child,
            handoff,
        } => match shop.accept(&parent, &child, &handoff) {
            Ok(h) => println!(
                "accepted child {child} -> JOINED (handoff {} {} is Evidence; cannot CLOSE; never invent PASS)",
                h.hash, h.status
            ),
            Err(e) => bail!("{}", wait_op("accept", &format!("{parent}/{child}"), e)),
        },
        Commands::Bounce {
            parent,
            child,
            reason,
        } => {
            let c = shop.bounce(&parent, &child, &reason)?;
            println!(
                "bounced child {child} -> BOUNCED (peer={} paths={}; same lane only)",
                c.peer,
                c.paths.join(",")
            );
        }
        Commands::Status { parent } => {
            print!("{}", shop.status(parent.as_deref())?);
        }
        Commands::Awareness { parent, json } => {
            let a = shop.awareness(parent.as_deref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&a)?);
            } else {
                print!("{}", format_awareness(&a));
            }
        }
        Commands::Workers => {
            let roster = shop.workers()?;
            let awareness = shop.awareness(None).ok();
            print!("{}", format_workers(&roster, awareness.as_ref()));
        }
        Commands::Mailbox => {
            print!("{}", format_mailbox(&shop.observe_mailbox()));
        }
        Commands::Prove { parent } => {
            let state = shop.state()?;
            let pkg = load_reduce_package(shop.store_root(), &parent);
            print!("{}", format_prove(&parent, state.parents.get(&parent), pkg.as_ref()));
        }
        Commands::Intel {
            peer,
            backend,
            model,
        } => {
            let w = shop.set_worker_intelligence(&peer, &backend, model.as_deref().unwrap_or(""))?;
            println!(
                "intel {} backend={} model={} (capacity only; not a running worker)",
                w.peer, w.intelligence.backend, w.intelligence.model
            );
        }
        Commands::Steer { text } => {
            let body = text.join(" ");
            if body.trim().is_empty() {
                println!("WAIT: empty steer; parent not CLOSED; never PASS");
                return Ok(());
            }
            let v = shop.steer(&body)?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        Commands::Processes => {
            print!("{}", format_processes(&shop.procwait()?));
        }
        Commands::Join { parent } => {
            let state = shop.join(&parent)?;
            match state {
                ParentState::ReduceReady => println!("join {parent}: REDUCE_READY"),
                other => println!(
                    "join {parent}: barrier not met (still {other}; never skip the join barrier)"
                ),
            }
        }
        Commands::Reduce { parent, note } => {
            shop.reduce(&parent, &note)?;
            println!(
                "reduced {parent} -> REDUCED (package {}/reduce/{parent}.json)",
                shop.store_root().display()
            );
        }
        Commands::Verify { parent, cmd } => {
            let rec = shop.verify(&parent, cmd.as_deref())?;
            match rec.status {
                EvidenceStatus::Pass => {
                    println!(
                        "verify {parent}: PASS -> VERIFIED (exit {:?})",
                        rec.exit_code
                    );
                }
                EvidenceStatus::Wait => {
                    println!(
                        "verify {parent}: WAIT (exit {:?}); parent not VERIFIED",
                        rec.exit_code
                    );
                    if rec.last_lines.is_empty() {
                        println!("  (no command output recorded)");
                    } else {
                        println!("{}", rec.last_lines);
                    }
                }
            }
        }
        Commands::Close { parent } => match shop.close(&parent) {
            Ok(_) => println!("closed {parent}"),
            Err(e) => bail!("{}", wait_op("close", &parent, e)),
        },
        Commands::Merge { parent } => match shop.merge_verified(&parent) {
            Ok(_) => println!(
                "merged {parent} (verify PASS recorded; merge ACK is not CLOSE)"
            ),
            Err(e) => bail!("{}", wait_op("merge", &parent, e)),
        },
        Commands::Wait {
            pid,
            parent,
            child,
            peer,
        } => {
            let rec = shop.wait_pid(pid, parent.as_deref(), child.as_deref(), peer.as_deref())?;
            let parent_state = parent.as_deref().and_then(|id| {
                shop.state()
                    .ok()
                    .and_then(|s| s.parents.get(id).map(|p| p.state))
            });
            match rec.class {
                crate::aasm_map::EvidenceClass::Unknown => {
                    println!(
                        "wait {pid}: UNKNOWN (dead pid, no exit); not PASS; parent not VERIFIED"
                    );
                }
                _ => {
                    println!(
                        "wait {pid}: exit {:?} as Evidence ({}); parent not VERIFIED",
                        rec.exit_code,
                        rec.class.as_str()
                    );
                }
            }
            if let Some(state) = parent_state {
                println!("  parent still {state} (wait does not verify or close)");
            }
        }
        Commands::Stop { pid } => {
            let rec = shop.stop_pid(pid)?;
            println!(
                "stop {pid}: explicit ({}); waiting is not stopping; pill={}",
                rec.liveness,
                rec.liveness.status_pill()
            );
        }
    }
    Ok(())
}

fn load_reduce_package(store: &Path, parent: &str) -> Option<ReducePackage> {
    let path = store.join("reduce").join(format!("{parent}.json"));
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn law_word(ok: bool) -> &'static str {
    if ok {
        "yes"
    } else {
        "WAIT"
    }
}

/// Fail-closed operator line. Errors stay WAIT and never print PASS.
fn wait_op(op: &str, target: &str, err: impl std::fmt::Display) -> String {
    format!("{op} {target}: WAIT ({err}); never PASS")
}

/// Fail-closed proving readout. Missing parent cannot print close PASS.
fn format_prove(id: &str, parent: Option<&Parent>, reduce: Option<&ReducePackage>) -> String {
    let mut out = format!("PROVE parent={id}\n");
    let Some(parent) = parent else {
        out.push_str("  WAIT: parent not found; close is WAIT; never PASS\n");
        out.push_str(&format!(
            "  kernel={} contact={}\n",
            proving::KERNEL,
            proving::CONTACT
        ));
        return out;
    };
    out.push_str(&format!("  state={}\n", parent.state));
    let close = proving::close_allowed(parent);
    if close {
        out.push_str("  close: allowed (recorded verify PASS; no ASSIGNED child)\n");
    } else {
        out.push_str("  close: WAIT (not VERIFIED or assigned child still open)\n");
    }
    out.push_str("  handoff_mailbox_github: Evidence; cannot CLOSE\n");
    out.push_str(&format!(
        "  recorded_verify_pass: {}\n",
        law_word(proving::recorded_verify_pass(parent))
    ));
    out.push_str(&format!(
        "  assigned_child_open: {}\n",
        if proving::assigned_child_open(parent) {
            "yes (blocks close)"
        } else {
            "no"
        }
    ));
    out.push_str(&format!(
        "  information_cannot_close: {}\n",
        law_word(proving::information_cannot_close(parent))
    ));
    out.push_str(&format!(
        "  parent_wait_is_not_verified: {}\n",
        law_word(proving::parent_wait_is_not_verified(parent))
    ));
    out.push_str(&format!(
        "  merge_ack_is_not_achieved_state: {}\n",
        law_word(proving::merge_ack_is_not_achieved_state(parent))
    ));
    out.push_str(&format!(
        "  merge_ack_does_not_verify: {}\n",
        law_word(proving::merge_ack_does_not_verify(parent))
    ));
    let mut path_ok = true;
    for child in parent.children.values() {
        if !proving::child_paths_inside_parent_claim(&child.paths, &parent.claimed_paths) {
            path_ok = false;
            out.push_str(&format!(
                "  WAIT: child {} path outside parent claim\n",
                child.id
            ));
        }
    }
    out.push_str(&format!(
        "  child_paths_inside_parent_claim: {}\n",
        law_word(path_ok)
    ));
    match reduce {
        Some(pkg) => out.push_str(&format!(
            "  reduce_is_not_recombine: {}\n",
            law_word(proving::reduce_is_not_recombine(pkg))
        )),
        None => out.push_str("  reduce_is_not_recombine: WAIT (no reduce package)\n"),
    }
    out.push_str(&format!(
        "  kernel={} contact={}\n",
        proving::KERNEL,
        proving::CONTACT
    ));
    out
}

fn format_mailbox(obs: &MailboxObservation) -> String {
    let mut out = String::from("MAILBOX (handoff is Evidence; unread assign is HOLD, not PASS)\n");
    if let Some(w) = &obs.wait {
        out.push_str(&format!("  {w}\n"));
        return out;
    }
    out.push_str(&format!(
        "  unread={}  outbox={}  root={}  hold_in_flight={}\n",
        obs.unread_total,
        obs.outbox_count,
        obs.root.as_deref().unwrap_or("?"),
        obs.hold_in_flight()
    ));
    if obs.peers.is_empty() {
        out.push_str("  (no peer inboxes)\n");
    } else {
        for p in &obs.peers {
            out.push_str(&format!(
                "  peer={} unread={} latest_assign={} latest_handoff={}\n",
                p.peer,
                p.unread,
                p.latest_assign.as_deref().unwrap_or("-"),
                p.latest_handoff.as_deref().unwrap_or("-")
            ));
        }
    }
    if obs.hold_in_flight() {
        out.push_str("  HOLD: unread assign in flight; not PASS; cannot CLOSE\n");
    }
    out
}

fn format_workers(roster: &WorkerRoster, awareness: Option<&Awareness>) -> String {
    let mut out = String::from(
        "WORKERS (capacity only; a roster name is not a running worker)\n",
    );
    if roster.workers.is_empty() {
        out.push_str("  (none; add via shop add or shop ui)\n");
        return out;
    }
    for w in &roster.workers {
        let view = awareness.and_then(|a| a.workers.iter().find(|v| v.peer == w.peer));
        let (action, pid, working) = match view {
            Some(v) => {
                let pid = v
                    .pid
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "WAIT".into());
                let working = if v.is_observed_working() { "yes" } else { "no" };
                (v.action.as_str(), pid, working)
            }
            None => ("WAIT", "WAIT".into(), "no"),
        };
        out.push_str(&format!(
            "  {}  name={}  backend={}  model={}  github_skills={}  action={}  pid={}  working={}\n",
            w.peer,
            w.name,
            w.intelligence.backend,
            w.intelligence.model,
            w.github_skills,
            action,
            pid,
            working
        ));
    }
    if let Some(a) = awareness {
        out.push_str(&format!(
            "  observed_working={} (roster names do not count)\n",
            a.working_worker_count()
        ));
    }
    out
}

fn format_awareness(a: &Awareness) -> String {
    let jobs = a.jobs_from_workers();
    let waits = a.truth_waits();
    let mut out = String::from(
        "AWARENESS (truth panel; missing pieces are WAIT, never invented; cannot CLOSE)\n",
    );
    out.push_str(&format!(
        "PROJECT  name={}  path={}  store={}\n",
        a.project.name, a.project.path, a.project.store
    ));
    out.push_str("WAITS\n");
    if waits.is_empty() {
        out.push_str("  (none recorded)\n");
    } else {
        for w in &waits {
            out.push_str(&format!("  {w}\n"));
        }
    }
    out.push_str("WORKERS\n");
    if a.workers.is_empty() {
        out.push_str("  (none; add via shop add or shop ui)\n");
    } else {
        for w in &a.workers {
            let pid = match (w.pid, w.pid_liveness) {
                (Some(pid), Some(live)) => format!("  pid={pid}  liveness={live}"),
                _ => "  pid=WAIT".into(),
            };
            out.push_str(&format!(
                "  {}  action={}  working={}{pid}\n",
                w.peer,
                w.action,
                if w.is_observed_working() { "yes" } else { "no" }
            ));
        }
    }
    out.push_str(&format!(
        "  observed_working_workers={}\n",
        a.working_worker_count()
    ));
    out.push_str("JOBS\n");
    if jobs.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for j in &jobs {
            let pid = j
                .pid
                .map(|p| p.to_string())
                .unwrap_or_else(|| "WAIT".into());
            out.push_str(&format!(
                "  {}/{}  peer={}  child={}  action={}  pid={}\n",
                j.parent_id, j.child_id, j.peer, j.child_state, j.action, pid
            ));
        }
    }
    out.push_str(&format!(
        "  observed_jobs={}  observed_working_jobs={}\n",
        crate::awareness::observed_job_count(&jobs),
        crate::awareness::observed_working_jobs(&jobs)
    ));
    out.push_str("GITHUB\n");
    if let Some(w) = &a.github.wait {
        out.push_str(&format!("  {w}\n"));
    }
    out.push_str(&format!(
        "  repo={}  issues={}  prs={}  checks={}\n",
        a.github.repo.as_deref().unwrap_or("WAIT"),
        crate::awareness::format_observed_count(a.github.issue_count),
        crate::awareness::format_observed_count(a.github.pr_count),
        crate::awareness::format_observed_count(a.github.check_count)
    ));
    out.push_str("MAILBOX\n");
    if let Some(w) = &a.mailbox.wait {
        out.push_str(&format!("  {w}\n"));
    } else {
        out.push_str(&format!(
            "  unread={}  outbox={}  hold_in_flight={}\n",
            a.mailbox.unread_total,
            a.mailbox.outbox_count,
            a.mailbox.hold_in_flight()
        ));
        if a.mailbox.hold_in_flight() {
            out.push_str("  HOLD: unread assign in flight; not PASS; cannot CLOSE\n");
        }
    }
    out
}

fn format_processes(ledger: &ProcWaitLedger) -> String {
    let mut out =
        String::from("PROCESSES (wait/stop track real pids; a roster name is not a process)\n");
    if ledger.processes.is_empty() {
        out.push_str("  (none)\n");
        return out;
    }
    for p in &ledger.processes {
        out.push_str(&format!(
            "  pid={}  {}  pill={}\n",
            p.pid,
            p.liveness,
            p.status_pill()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aasm_map::{self, EvidenceClass};
    use crate::awareness::{GitHubAwareness, ProjectView};
    use crate::mailbox::PeerInbox;
    use crate::types::{Child, ChildState, EvidenceStatus, VerifyRecord};
    use clap::Parser;
    use serde_json::json;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("shop").chain(args.iter().copied())).expect("parse")
    }

    #[test]
    fn suite_verbs_parse() {
        assert!(matches!(
            parse(&["awareness"]).command,
            Commands::Awareness {
                parent: None,
                json: false
            }
        ));
        assert!(matches!(
            parse(&["awareness", "P", "--json"]).command,
            Commands::Awareness {
                parent: Some(ref id),
                json: true
            } if id == "P"
        ));
        assert!(matches!(parse(&["workers"]).command, Commands::Workers));
        assert!(matches!(parse(&["mailbox"]).command, Commands::Mailbox));
        assert!(matches!(
            parse(&["prove", "P"]).command,
            Commands::Prove { parent: ref id } if id == "P"
        ));
        assert!(matches!(
            parse(&["intel", "--peer", "alice", "--backend", "grok-bot"]).command,
            Commands::Intel { ref peer, ref backend, model: None }
                if peer == "alice" && backend == "grok-bot"
        ));
        assert!(matches!(
            parse(&["steer", "remember", "we", "ship"]).command,
            Commands::Steer { ref text } if text == &["remember", "we", "ship"]
        ));
        assert!(matches!(
            parse(&["processes"]).command,
            Commands::Processes
        ));
    }

    #[test]
    fn missing_parent_prove_is_wait_never_pass() {
        let text = format_prove("P", None, None);
        assert!(text.contains("WAIT: parent not found"));
        assert!(text.contains("close is WAIT"));
        assert!(!text.to_uppercase().contains("CLOSE: ALLOWED"));
        assert!(!text.contains("close: allowed"));
    }

    #[test]
    fn assigned_child_prove_cannot_close() {
        let mut p = Parent::new("P".into(), "t".into(), "b".into());
        p.children.insert(
            "c1".into(),
            Child::new(
                "c1".into(),
                "alice".into(),
                vec!["src/a".into()],
                "t".into(),
                "b".into(),
            ),
        );
        let text = format_prove("P", Some(&p), None);
        assert!(text.contains("close: WAIT"));
        assert!(text.contains("assigned_child_open: yes"));
        assert!(text.contains("handoff_mailbox_github: Evidence; cannot CLOSE"));
        assert!(text.contains("reduce_is_not_recombine: WAIT (no reduce package)"));
        assert!(!text.contains("close: allowed"));
    }

    #[test]
    fn mailbox_hold_is_not_pass() {
        let obs = MailboxObservation {
            root: Some("/tmp/mail".into()),
            present: true,
            inbox_present: true,
            wait: None,
            peers: vec![PeerInbox {
                peer: "alice".into(),
                unread: 1,
                latest_assign: Some("assign.json".into()),
                latest_handoff: None,
            }],
            unread_total: 1,
            outbox_count: 1,
        };
        let text = format_mailbox(&obs);
        assert!(text.contains("HOLD: unread assign"));
        assert!(text.contains("not PASS"));
        assert!(text.contains("hold_in_flight=true"));
        assert!(!text.contains("PASS\n"));
    }

    #[test]
    fn mailbox_gap_is_wait() {
        let obs = MailboxObservation {
            root: None,
            present: false,
            inbox_present: false,
            wait: Some("WAIT: mailbox path unset".into()),
            peers: vec![],
            unread_total: 0,
            outbox_count: 0,
        };
        let text = format_mailbox(&obs);
        assert!(text.contains("WAIT: mailbox path unset"));
        assert!(!text.contains("unread=0"));
    }

    #[test]
    fn empty_roster_is_not_working() {
        let text = format_workers(&WorkerRoster::default(), None);
        assert!(text.contains("capacity only"));
        assert!(text.contains("(none;"));
        assert!(!text.contains("working=yes"));
    }

    #[test]
    fn awareness_missing_github_counts_are_wait_not_zero() {
        let a = Awareness {
            store_ok: true,
            project: ProjectView {
                name: "demo".into(),
                path: "/tmp/demo".into(),
                store: "/tmp/demo/.shop".into(),
            },
            workers: vec![],
            claims: vec![],
            parents: vec![],
            mailbox: MailboxObservation {
                root: None,
                present: false,
                inbox_present: false,
                wait: Some("WAIT: mailbox path unset".into()),
                peers: vec![],
                unread_total: 0,
                outbox_count: 0,
            },
            github: GitHubAwareness {
                repo: None,
                default_branch: None,
                authenticated: false,
                token_present: false,
                wait: Some("WAIT: GitHub not connected".into()),
                checks: json!({}),
                issue_count: None,
                pr_count: None,
                check_count: None,
            },
            events: vec![],
            waits: vec!["WAIT: no live pid".into()],
        };
        let text = format_awareness(&a);
        assert!(text.contains("issues=WAIT"));
        assert!(text.contains("prs=WAIT"));
        assert!(text.contains("checks=WAIT"));
        assert!(text.contains("repo=WAIT"));
        assert!(text.contains("WAIT: GitHub not connected"));
        assert!(text.contains("WAIT: mailbox path unset"));
        assert!(text.contains("observed_working_workers=0"));
        assert!(!text.contains("issues=0"));
        assert!(!text.contains("prs=0"));
        assert!(!text.contains("checks=0"));
        assert!(!text.contains("PASS"));
    }

    #[test]
    fn empty_process_ledger_is_not_a_live_pid() {
        let text = format_processes(&ProcWaitLedger::default());
        assert!(text.contains("roster name is not a process"));
        assert!(text.contains("(none)"));
        assert!(!text.contains("pill=working"));
    }

    #[test]
    fn prove_verified_close_allowed_only_with_recorded_pass() {
        let mut p = Parent::new("P".into(), "t".into(), "b".into());
        p.state = crate::types::ParentState::Verified;
        p.verify = Some(VerifyRecord {
            cmd: "true".into(),
            exit_code: Some(0),
            last_lines: String::new(),
            status: EvidenceStatus::Pass,
            class: EvidenceClass::Pass,
            aasm_note: aasm_map::VERIFY_PASS_NOTE.into(),
            recorded_at: 1,
        });
        let mut joined = Child::new(
            "c1".into(),
            "alice".into(),
            vec!["src/a".into()],
            "t".into(),
            "b".into(),
        );
        joined.state = ChildState::Joined;
        p.children.insert("c1".into(), joined);
        let text = format_prove("P", Some(&p), None);
        assert!(text.contains("close: allowed"));
        assert!(text.contains("recorded_verify_pass: yes"));
        assert!(text.contains("handoff_mailbox_github: Evidence; cannot CLOSE"));
    }

    #[test]
    fn wait_op_is_wait_never_pass() {
        let text = wait_op("close", "P", "not VERIFIED");
        assert!(text.contains("close P: WAIT"));
        assert!(text.contains("never PASS"));
        assert!(!text.contains("closed P"));
        let merge = wait_op("merge", "P", "merge blocked until shop verify recorded PASS");
        assert!(merge.contains("merge P: WAIT"));
        assert!(merge.contains("never PASS"));
    }

    #[test]
    fn awareness_hold_cannot_close() {
        let a = Awareness {
            store_ok: true,
            project: ProjectView {
                name: "demo".into(),
                path: "/tmp/demo".into(),
                store: "/tmp/demo/.shop".into(),
            },
            workers: vec![],
            claims: vec![],
            parents: vec![],
            mailbox: MailboxObservation {
                root: Some("/tmp/mail".into()),
                present: true,
                inbox_present: true,
                wait: None,
                peers: vec![PeerInbox {
                    peer: "alice".into(),
                    unread: 1,
                    latest_assign: Some("assign.json".into()),
                    latest_handoff: None,
                }],
                unread_total: 1,
                outbox_count: 0,
            },
            github: GitHubAwareness {
                repo: None,
                default_branch: None,
                authenticated: false,
                token_present: false,
                wait: Some("WAIT: GitHub not connected".into()),
                checks: json!({}),
                issue_count: None,
                pr_count: None,
                check_count: None,
            },
            events: vec![],
            waits: vec![],
        };
        let text = format_awareness(&a);
        assert!(text.contains("cannot CLOSE"));
        assert!(text.contains("HOLD: unread assign in flight"));
        assert!(text.contains("not PASS"));
        assert!(text.contains("hold_in_flight=true"));
        assert!(!text.contains("issues=0"));
    }

    #[test]
    fn missing_parent_prove_names_handoff_as_evidence() {
        let text = format_prove("P", None, None);
        assert!(text.contains("WAIT: parent not found"));
        assert!(!text.contains("handoff_mailbox_github: Evidence"));
        assert!(!text.contains("close: allowed"));
    }
}
