# shop

```text
shop ui
```

Opens the **command center** on localhost (default port **7745**). One surface: project picker, workers, current job, GitHub, Talk to the boss, activity, inbox.

**Talk to the boss** steers **SuperGrokHeavy** (`cursor-groksuperheavy`), the orchestrator and planner. Local shop verbs run immediately and show as system notes. Free text is a mailbox `steer` to SuperGrokHeavy, never to grok-bot. Every steer includes `memory.profile`, the last 20 `memory.log` lines, and the full `memory.floor` object.

Open projects like Cursor / Grok Bot: **This computer** folder or **GitHub** repo (`shop project open PATH`). Store is `<project>/.shop`. Recents live in `~/.shop/recents.json`.

Add a worker and assign intelligence in the Workers column (backend + model). Seeded roster: `cursor`, `grok-ultra`, `grok-bot`, `cursor-groksuperheavy`. Enrolling capacity does not create a job, invent a lane, write `C:\TextPCB Platform`, or send mailbox assigns.

`shop add --peer alice --name Alice --backend grok-bot` is the same store write for tests and scripts.

`shop` is not [AASM](https://github.com/halthinks/AASM) and not T3 Code. AASM stays the authority calculus.

## Hold-split-join

```text
shop open --id PARENT --title "..." --body "..."
shop split PARENT --child C1 --peer cursor --paths a,b --title "..." --body "..."
shop assign PARENT
shop accept PARENT --child C1 --handoff ./handoff.json
shop join PARENT
shop reduce PARENT --note "..."
shop verify PARENT --cmd "cargo test"
shop close PARENT
```

Parent: `HELD → IN_FLIGHT → REDUCE_READY → REDUCED → VERIFY_WAIT|VERIFIED → CLOSED`

`split` rejects unknown peers and overlapping in-flight `allowed_paths`. Bounce stays on the same peer/paths. Incomplete evidence is **WAIT**, never a fake PASS. `close` only from `VERIFIED`. No approval gates on add/split/assign/accept/bounce/join/reduce/verify. Merge is explicit and blocked until VERIFIED.

## GitHub

`shop github connect owner/name` (or Connect in the GitHub module) records the repo in `.shop/github.json`. Public recorded repos (`halthinks/shop-floor`, `halthinks/AASM`) are readable without a token. Private repos need `gh` or a token from day one. An optional token writes `.shop/github.token` (gitignored). Each child gets intended branch `shop/<parent>/<child>`. `reduce` lists repo + branches and opens a draft PR only if authenticated — never a fake PR URL. Merge stays disabled until `shop verify` recorded PASS / parent VERIFIED. Missing checks = WAIT.

Skill pack (via `gh` or `GITHUB_TOKEN`, mock in tests): me/whoami; repos search/connect/create (explicit, default private); files get/put/delete/push; branches list/create; commits list/get/search; issues list/read/write/comment/sub-issues; pulls list/read/create/update/update-branch/merge; reviews; search; releases/tags; collaborators/teams (read); fork; secret scan (kind+offset only).

## Mailbox

`--mailbox DIR`, else `SHOP_MAILBOX`, else `.shop/mailbox`. Assign JSON (`textpcb/agent-bridge/v1`) writes to `inbox/<peer>/` when that inbox exists, and always to `.shop/outbox/`. Steer mail goes to `inbox/cursor-groksuperheavy/`. If the mailbox is down, store + outbox still record. Status shows unread by peer.

## Shop memory

Standing facts and dated episodes, distinct from the floor.

```text
shop remember --tier profile "we ship shop-floor"
shop remember --tier log "opened parent P"
shop forget "we ship shop-floor"
shop memory
```

Stored under `.shop/memory/profile.json` and `.shop/memory/log.jsonl`. Saying `remember …` in Talk to the boss stores the fact and still steers SuperGrokHeavy. Shop never invents a fact and never writes `C:\TextPCB Platform`.

## Floor memory

Durable sequencer state. Current job, children, claim lock, and closed/reduced history.

```text
shop floor
```

Files: `.shop/floor/current.json`, `children.json`, `claims.json`, `history.jsonl`. Restart via `Shop::open_store` restores children and claims. Bounce stays on the same peer/paths. Incomplete evidence stays WAIT — never stored as JOINED or VERIFIED. Closing a parent appends history and clears current.

## Reply loop

Shop reads SuperGrokHeavy answers back into the transcript. It does not invent them. On the same 2s refresh it watches mailbox `inbox/shop`, `.shop/inbox/`, and `inbox/cursor-groksuperheavy` for JSON with `from: cursor-groksuperheavy` and `type` `steer-reply` | `handoff` | `reply`. New bodies append as `role=orchestrator` on `steer.jsonl`. If nothing has replied, the chat shows Waiting.

`shop listen` stays running with the UI and performs this poll.

To launch SuperGrokHeavy / Grok Build on a steer, set `SHOP_BOSS_CMD` to a command (no Platform path). If the command is missing or fails, shop records WAIT `boss process not launched` and still keeps the steer file.

## Store

Durable JSON under `.shop/` (gitignored). Events: `.shop/events.jsonl` (also `GET /feed.xml` and `shop log`). Floor and shop memory live beside the snapshot. If the UI process dies, the CLI still works on the same store. One path failing does not blank the others.

```bash
cargo test
cargo build --release
```

Apache-2.0.
