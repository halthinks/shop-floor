# shop suite roadmap

Shop is the suite. AASM is the look-only kernel. This file is the T3-class
suite contract after leftovers **#8–#13** landed on `main`. It does not
rewrite the floor, remmerge those PRs, or open a TextPCB product lane.

Shop is not [AASM](https://github.com/halthinks/AASM) and not T3 Code.
AASM stays the authority calculus. Shop obeys it through a dictionary.
It does not vendor the reducer, the calculus, or the effect plane.

`ROADMAP.md` is the floor (hold-split-join, run, Windows, multi-project,
boss screen). This file is the suite that sits on that floor.

## T3-class (what this word means here)

T3-class is the harness class: a real control surface over real workers,
not a cloud-agent myth and not a second kernel.

| Law | Shop meaning |
| --- | --- |
| Real process | A roster name is not a running worker. Launch waits on a live pid. |
| Isolated child | Each assigned child gets its own git worktree and `allowed_paths`. |
| One owner per path | Two in-flight children cannot claim the same path. |
| Look-only kernel | `src/aasm_map.rs` is a dictionary. Do not copy AASM types into shop. |
| Fail closed | Missing evidence is WAIT (`INCONCLUSIVE` / `INFORMATION_GAP` / `UNKNOWN`). Never fake PASS. |
| Evidence ≠ authority | Handoff, mailbox reply, CI check, merge ACK cannot CLOSE a parent. |
| No invented lane | Only a named captain assign creates work. `src/workers.rs` stays unassigned until named. |
| No Platform write | Never write `C:\TextPCB Platform`. No TextPCB product lane from this suite. |

Shop is the suite that must hold those laws. T3 Code is a different
product. Do not port it. Do not claim to be it.

## Kernel boundary (look-only)

| Piece | Role | Not allowed |
| --- | --- | --- |
| [halthinks/AASM](https://github.com/halthinks/AASM) 0.56.1 | Kernel. Authority calculus. | Shop does not vendor it. |
| `src/aasm_map.rs` | Shop word → AASM class. EvidenceClass only. | No reducer. No effect plane. No invented kernel type named bounce. |
| `src/proving.rs` | Thin proving helpers. Points at the contact tests. | Not a second `tests/aasm_proving.rs`. |
| `tests/aasm_proving.rs` | Contact surface. Must fail closed. | Green-by-invented-authority is a lie. |
| `docs/AASM_PROVING_GROUND.md` | Floor statement of the laws. | Do not weaken. |

Dictionary (shop word → AASM), already on main:

- `bounce` → causal backjump (`compute_backjump` / `apply_backjump`). Same peer/paths. JOINED siblings survive. Not chronological undo.
- `WAIT` → `INCONCLUSIVE` \| `INFORMATION_GAP` \| `UNKNOWN`. Missing evidence is not PASS.
- `held` / parent open → AuthorityLease holder + parent-subset. Child cannot amplify scope. Lease existence is not effect permission.
- `reduce` → shop package. **Not** AASM recombine (entity MERGED / U4 is NEXT).
- `verify PASS` → evidence-plane certificate only. Not COMPLETE. Not fact authority. Not achieved state.
- merge / command ACK → EffectStatus.SUCCEEDED analog. Command success is not achieved state.

Do not rewrite `src/engine.rs`, `src/cli.rs`, `src/ui.rs`, `src/ui.html`,
`src/ui.css`, or `src/lib.rs` from this lane. The suite names modules.
Those files stay someone else's assign.

## Floor leftovers — on main. Do not remmerge.

Independent `main` at assign time: `a5af69f183f2a56538989d79326792f73783a465`
(Merge pull request **#13**). Open PRs: **0**.

| PR | Lane | What landed | Do not |
| --- | --- | --- | --- |
| #3 | SHOP-WORKFLOWS | GitHub Actions CI + Windows release skeleton | refill |
| #4 | SHOP-RUNNER | Launch a real process from `SHOP_RUN_CMD` / `run.json` | refill |
| #5 | SHOP-PROCWAIT | Live wait/stop process state; not a verify promotion | refill |
| #6 | SHOP-WORKTREE | Isolated git worktree per assigned child | refill |
| #7 | SHOP-GLUE | Runner glue recut on main | refill |
| #8 | SHOP-MULTI-PROJECT | `src/projects.rs` + `tests/projects.rs` — more than one open floor | remmerge |
| #9 | SHOP-WINDOWS-BINARY | `docs/WINDOWS.md` + `scripts/shop-ui.ps1` — `shop ui` is a startable binary | remmerge |
| #10 | SHOP-BOSS-SCREEN | `src/boss_view.rs` + `tests/boss_view.rs` — SuperGrokHeavy live floor view | remmerge |
| #11 | SHOP-ROSTER-NOT-RUNNING | `tests/run_workers.rs` — a roster name is not a running worker | remmerge |
| #12 | planner-feed JSON | `web/planner-feed` poll target + README | remmerge |
| #13 | planner-feed box | Captain train-of-thought feed box | remmerge |

`ROADMAP.md` items 1–4 are the floor leftovers above. They are done on
`main`. This suite file does not reopen them.

## Suite modules (named next, disjoint)

Captain names these. One owner per path. Copy grok-qa. Incomplete
evidence is WAIT. Do not invent a fifth product.

| Order | Lane | Path | Do |
| --- | --- | --- | --- |
| 1 | shop-suite-roadmap | `docs/SUITE_ROADMAP.md` | This file. Suite contract only. |
| 2 | shop-aasm-map | `src/aasm_map.rs` | Keep the proving-ground map tight. Look-only. Do not copy the AASM kernel. Do not touch engine/cli/ui. |
| 3 | shop-proving | `src/proving.rs` | Thin proving helpers only. Do not rewrite `tests/aasm_proving.rs` unless that file is also owned later. |
| 4 | shop-awareness | `src/awareness.rs` | Live awareness of workers/jobs from store + mailbox + GitHub records. No fake numbers. Missing count is `None` / WAIT. Do not touch `src/workers.rs` unless assigned. |
| 5 | shop-mailbox | `src/mailbox.rs` | Mailbox adapter only. One assign in flight. Outbox is durable if the inbox is down. Do not invent a fifth product. |

Hold, not a suite row:

- `src/workers.rs` exists on main (`03dbe459` at #13). Unassigned. Do not invent that lane.
- `web/planner-feed` is on main via #12/#13. Forbidden to this lane. Do not refill.
- `src/projects.rs`, `src/boss_view.rs`, `tests/run_workers.rs`, `docs/WINDOWS.md`, `scripts/shop-ui.ps1` — leftover owners stay. Do not remmerge.

## Suite shape

```text
AASM 0.56.1          look-only kernel (do not vendor)
        ^
        | dictionary only
src/aasm_map.rs      shop word → AASM class
src/proving.rs       thin helpers → tests/aasm_proving.rs
        ^
shop suite           this file names the modules
        |
   +----+----+----+----+
   |         |         |
awareness  mailbox   floor leftovers already on main
workers/   assign    projects / boss_view / run_workers
jobs/      one in    WINDOWS / worktree / runner / procwait
GitHub     flight    planner-feed (#12/#13)
```

Awareness reads. Mailbox writes assigns and steers. Neither may mint
VERIFIED. Neither may write Platform. Neither may copy AASM.

## Fail-closed (suite-wide)

Copied from the proving-ground contact. The suite does not relax them.

1. A handoff, mailbox reply, or GitHub check is Evidence. It cannot CLOSE a parent.
2. Missing evidence is `INCONCLUSIVE` / `INFORMATION_GAP` / `UNKNOWN`, never VERIFIED.
3. A parent cannot complete while a mandatory child is still ASSIGNED.
4. Bounce is a causal backjump: same peer/paths; JOINED siblings survive.
5. Child paths stay inside the parent claim set (no amplification).
6. Reduce does not claim AASM recombine.
7. A command or merge ACK is not achieved state (no auto VERIFIED from a merge record).

If a suite module goes green by inventing authority, the suite is lying.

## This PR

- Adds `docs/SUITE_ROADMAP.md` only.
- Does not rewrite engine / cli / ui / lib.
- Does not remmerge #8 / #9 / #10 / #11 / #12 / #13.
- Does not merge.
- Does not write `C:\TextPCB Platform`.
- Does not open a TextPCB product lane.

Later: this suite is what lets a TextPCB shop exist. Not in this PR.
