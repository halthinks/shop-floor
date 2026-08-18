# Next leftover

Named leftover: _(none)_

All named suite leftovers have a green merged PR. Do not invent a fifth product. Do not invent a `src/workers.rs` lane.

Named leftover paths from `docs/SUITE_ROADMAP.md` suite modules, in order, one owner:

1. `src/aasm_map.rs` — used on main
1. `src/proving.rs` — used on main
1. `src/awareness.rs` — used on main
1. `src/mailbox.rs` — used on main
1. `src/steer.rs` — used on main

The shop pulse is the lander. Orchestrator is not in the loop.

- MERGE only when both `Linux tests` and `Windows release` conclusions are success, and the `src/` write set is exactly one leftover path (or a subset of one leftover group).
- NEVER merge `src/workers.rs`, `src/runner.rs`, `src/worktree.rs`, `src/procwait.rs`.
- NEVER remmerge PRs #8–#29. Close invented `workers.rs` PRs.
- AASM stays look-only. Do not vendor AASM. Do not write `C:\TextPCB Platform`. Do not open a TextPCB product lane.
- Missing evidence is WAIT, never default PASS.

Pulse: WRITE_BRIEF — all named suite leftovers have a green merged PR; do not invent a fifth product
