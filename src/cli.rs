//! Thin clap front-end. All sequencing lives in the library.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use crate::engine::Shop;
use crate::mailbox::{resolve_mailbox, DEFAULT_FROM};
use crate::skills_live::LiveGitHub;
use crate::types::{EvidenceStatus, ParentState};
use crate::ui::{self, DEFAULT_PORT};

#[derive(Parser, Debug)]
#[command(
    name = "shop",
    version,
    about = "Shop-floor hold-split-join sequencer",
    long_about = "Open shop ui, add a worker, assign AI intelligence, then hold-split-join.\n\
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
        } => {
            let h = shop.accept(&parent, &child, &handoff)?;
            println!(
                "accepted child {child} -> JOINED (handoff {} {})",
                h.hash, h.status
            );
        }
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
            Err(e) => bail!("{e}"),
        },
        Commands::Merge { parent } => match shop.merge_verified(&parent) {
            Ok(_) => println!("merged {parent} (verify PASS recorded)"),
            Err(e) => bail!("{e}"),
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
