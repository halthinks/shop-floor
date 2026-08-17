# Shop pulse

The shop pulse is the lander. Orchestrator is not in the loop. Brock is not in the loop. A chat babysitter is not in the loop.

`.github/workflows/shop-pulse.yml` runs on a schedule and on `workflow_dispatch`. It reads `docs/SUITE_ROADMAP.md` and live GitHub facts (open PRs, check runs), then either merges a green leftover or names the next unused named suite path in `docs/NEXT_LEFTOVER.md`.

Leftover discovery is derived at runtime from the table under **Suite modules (named next, disjoint)** in `docs/SUITE_ROADMAP.md`. The first unused named `src/` module path is the leftover. A later named row is picked up with no pulse code change. `_(none)_` is only legal when every named suite path already has a green merged PR.

Rules (fail-closed; missing evidence is WAIT, never default PASS):

- MERGE only when both `Linux tests` and `Windows release` conclusions are success, and the `src/` write set is exactly one leftover path (or a subset of one leftover group).
- NEVER merge `src/workers.rs`, `src/runner.rs`, `src/worktree.rs`, `src/procwait.rs`.
- NEVER remmerge PRs #8–#29. Close invented `workers.rs` PRs.
- Leftover naming is mandatory while a named unused SUITE_ROADMAP path exists.
- Do not invent a fifth product outside the roadmap table. Do not invent a workers.rs lane.
- AASM stays look-only. Do not vendor AASM. Do not write `C:\TextPCB Platform`. Do not open a TextPCB product lane.

`cargo test --test pulse` proves the merge gate and leftover-must-name. `cargo run --bin shop-pulse -- live --apply` is what the workflow runs.
