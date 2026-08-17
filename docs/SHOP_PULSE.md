# Shop pulse

The shop pulse is the lander. Orchestrator is not in the loop. Brock is not in the loop. A chat babysitter is not in the loop.

`.github/workflows/shop-pulse.yml` runs on a schedule and on `workflow_dispatch`. It reads `docs/SUITE_ROADMAP.md` and live GitHub facts (open PRs, check runs), then either merges a green reserved leftover or names the next unused reserved path in `docs/NEXT_LEFTOVER.md`.

Reserved leftovers, in order, one owner: `src/engine.rs`, `src/cli.rs`, `src/ui.rs`+`src/ui.html`+`src/ui.css`, `src/lib.rs`.

Rules (fail-closed; missing evidence is WAIT, never default PASS):

- MERGE only when both `Linux tests` and `Windows release` conclusions are success, and the `src/` write set is exactly one reserved leftover (or a subset of one reserved group).
- NEVER merge `src/workers.rs`, `src/runner.rs`, `src/worktree.rs`, `src/procwait.rs`.
- NEVER remmerge PRs #8–#18. Close invented `workers.rs` PRs.
- Leftover naming is mandatory while a reserved path is still unmerged or has no green PR.
- Do not invent a fifth product. Do not invent a workers.rs lane.
- AASM stays look-only. Do not vendor AASM. Do not write `C:\TextPCB Platform`. Do not open a TextPCB product lane.

`cargo test --test pulse` proves the merge gate and leftover-must-name. `cargo run --bin shop-pulse -- live --apply` is what the workflow runs.
