# shop

```text
shop ui
```

Opens the **command center** on localhost (default port **7745**). One surface: project picker, workers, floor, GitHub, CONTROL, feed, mailbox.

The **CONTROL** plane steers **SuperGrokHeavy** (`cursor-groksuperheavy`). Local shop verbs run immediately. Free text is a mailbox `steer` to SuperGrokHeavy, never to grok-bot.

Open projects like Cursor / Grok Bot: **LOCAL** folder or **GITHUB** repo (`shop project open PATH`). Store is `<project>/.shop`. Recents live in `~/.shop/recents.json`.

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

`shop github connect owner/name` (or Connect in the GitHub module) records the repo in `.shop/github.json`. An optional token writes `.shop/github.token` (gitignored). Each child gets intended branch `shop/<parent>/<child>`. `reduce` lists repo + branches and opens a draft PR only if authenticated — never a fake PR URL. Merge stays disabled until `shop verify` recorded PASS / parent VERIFIED. Missing checks = WAIT.

Skill pack (via `gh` or `GITHUB_TOKEN`, mock in tests): me/whoami; repos search/connect/create (explicit, default private); files get/put/delete/push; branches list/create; commits list/get/search; issues list/read/write/comment/sub-issues; pulls list/read/create/update/update-branch/merge; reviews; search; releases/tags; collaborators/teams (read); fork; secret scan (kind+offset only).

## Mailbox

`--mailbox DIR`, else `SHOP_MAILBOX`, else `.shop/mailbox`. Assign JSON (`textpcb/agent-bridge/v1`) writes to `inbox/<peer>/` when that inbox exists, and always to `.shop/outbox/`. Steer mail goes to `inbox/cursor-groksuperheavy/`. If the mailbox is down, store + outbox still record. Status shows unread by peer.

## Store

Durable JSON under `.shop/` (gitignored). Events: `.shop/events.jsonl` (also `GET /feed.xml` and `shop log`). If the UI process dies, the CLI still works on the same store. One path failing does not blank the others.

```bash
cargo test
cargo build --release
```

Apache-2.0.
