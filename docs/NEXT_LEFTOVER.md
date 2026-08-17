# Next leftover

Named leftover: `src/aasm_map.rs`

Fail-closed hole: Keep the proving-ground map tight. Look-only. Do not copy the AASM kernel. Do not touch engine/cli/ui.

Named leftover paths from `docs/SUITE_ROADMAP.md` suite modules, in order, one owner:

1. `src/aasm_map.rs` — UNUSED
1. `src/proving.rs` — UNUSED
1. `src/awareness.rs` — UNUSED
1. `src/mailbox.rs` — UNUSED

The shop pulse is the lander. Orchestrator is not in the loop.

- MERGE only when both `Linux tests` and `Windows release` conclusions are success, and the `src/` write set is exactly one leftover path (or a subset of one leftover group).
- NEVER merge `src/workers.rs`, `src/runner.rs`, `src/worktree.rs`, `src/procwait.rs`.
- NEVER remmerge PRs #8–#29. Close invented `workers.rs` PRs.
- AASM stays look-only. Do not vendor AASM. Do not write `C:\TextPCB Platform`. Do not open a TextPCB product lane.
- Missing evidence is WAIT, never default PASS.

Pulse: MERGE — lander: leftover src/aasm_map.rs is green; merge PR #31 without a chat
