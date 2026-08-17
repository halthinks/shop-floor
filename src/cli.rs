//! Thin clap front-end. All sequencing lives in the library.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use crate::engine::Shop;
use crate::mailbox::DEFAULT_FROM;
use crate::types::{EvidenceStatus, ParentState};

#[derive(Parser, Debug)]
#[command(
    name = "shop",
    version,
    about = "Shop-floor hold-split-join sequencer",
    long_about = "Thin adapter: hold a parent, split claim-locked children, join, reduce, verify, close.\n\
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
    /// Create a parent in state HELD
    Open {
        #[arg(long)]
        id: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        body: String,
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
    Assign { parent: String },
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
}

pub fn run() -> Result<()> {
    run_cli(Cli::parse())
}

pub fn run_cli(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Init { dir } => {
            let root = dir.unwrap_or(cli.store);
            let shop = Shop::init(&root)?
                .with_mailbox(cli.mailbox)
                .with_from(cli.from);
            println!("initialized shop store {}", shop.store_root().display());
            Ok(())
        }
        other => {
            let mut shop = Shop::open_store(&cli.store)
                .with_context(|| format!("open store {}", cli.store.display()))?
                .with_mailbox(cli.mailbox)
                .with_from(cli.from);
            dispatch(&mut shop, other)
        }
    }
}

fn dispatch(shop: &mut Shop, command: Commands) -> Result<()> {
    match command {
        Commands::Init { .. } => unreachable!(),
        Commands::Open { id, title, body } => {
            shop.open(&id, &title, &body)?;
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
        Commands::Assign { parent } => {
            let recs = shop.assign(&parent)?;
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
    }
    Ok(())
}
