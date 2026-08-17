# Next leftover

Named leftover: _(none)_

All reserved suite leftovers have a green merged PR. Do not invent a fifth product. Do not invent a `src/workers.rs` lane.

Reserved leftover paths, in order, one owner:

1. `src/engine.rs` — used on main (#20 / #23)
2. `src/cli.rs` — used on main (#21, follow-up #25)
3. `src/ui.rs` + `src/ui.html` + `src/ui.css` — used on main (#22)
4. `src/lib.rs` — used on main (#24)

The shop pulse is the lander. Orchestrator is not in the loop.

- MERGE only when both `Linux tests` and `Windows release` conclusions are success, and the `src/` write set is exactly one reserved leftover (or a subset of one reserved group).
- NEVER merge `src/workers.rs`, `src/runner.rs`, `src/worktree.rs`, `src/procwait.rs`.
- NEVER remmerge PRs #8–#18. Close invented `workers.rs` PRs.
- AASM stays look-only. Do not vendor AASM. Do not write `C:\TextPCB Platform`. Do not open a TextPCB product lane.
- Missing evidence is WAIT, never default PASS.

Live facts at write: open reserved PRs = 0. #24 landed green. No unused reserved path remains.
