# shop

Open the floor, add a worker, assign AI intelligence, then sequence work.

```text
shop ui
```

That starts a localhost page (default port **7745**, so it does not collide with a live room on 7744). In the form: name/peer, intelligence backend (`cursor` | `cursor-ultra` | `grok-bot` | `grok-build`), and an optional model id you already have. Shop never invents Cursor model IDs. Adding a worker enrolls **capacity only** — it does not create a job, invent a lane, write `C:\TextPCB Platform`, or send mailbox assigns.

`shop add --peer alice --name Alice --backend grok-bot` is the same store write for tests and scripts. Roster lives in `.shop/workers.json`.

`shop` is not [AASM](https://github.com/halthinks/AASM) and not T3 Code. AASM stays the authority calculus. This crate keeps durable parent-job state so isolated workers can be sequenced without overlapping `allowed_paths`.

## Hold-split-join

```text
shop open --id PARENT --title "..." --body "..."
shop split PARENT --child C1 --peer alice --paths a,b --title "..." --body "..."
shop assign PARENT
shop accept PARENT --child C1 --handoff ./handoff.json
shop join PARENT
shop reduce PARENT --note "..."
shop verify PARENT --cmd "cargo test"
shop close PARENT
```

Parent: `HELD → IN_FLIGHT → REDUCE_READY → REDUCED → VERIFY_WAIT|VERIFIED → CLOSED`

`split` rejects unknown peers and overlapping in-flight `allowed_paths`. Bounce stays on the same peer/paths. Incomplete evidence is **WAIT**, never a fake PASS. `close` only from `VERIFIED`.

## GitHub

`shop github connect owner/name` (or the GitHub panel in the UI) records the repo in `.shop/github.json`. An optional token field writes `.shop/github.token` (gitignored; never committed). Each child gets an intended branch `shop/<parent>/<child>` — the branch is created only when `gh` or a token actually works; otherwise the ref is recorded and stays WAIT. `reduce` lists repo + child branches and opens a **draft** PR only if authenticated; if not, the package stays WAIT and shop never fakes a PR URL. `verify` may treat PR checks as evidence when a token works; missing checks or no real PR = WAIT, never VERIFIED. Merge is an explicit danger control, never automatic.

The GitHub skill pack (shop wrappers of the official MCP surface, via `gh` or `GITHUB_TOKEN`, mock in tests): me/whoami; repos search/connect/create (create is explicit, default private); files get/put/delete/push; branches list/create; commits list/get/search; issues list/read/write/comment/sub-issues; pulls list/read (diff/files/commits/reviews/comments/status/check_runs)/create/update/update-branch/merge (explicit only); reviews write/pending comments/reply; search code/issues/pulls/commits/users; releases/tags; collaborators/teams (read); fork; secret scan of provided content (kind+offset only, no secret dumps). A worker toggle grants or revokes the whole pack.

## Store

Durable JSON under `.shop/` (gitignored): snapshot, parents, claims, workers, github, outbox, reduce packages. Writes are temp + rename. Shop writes only inside the store and an optional mailbox outbox.

## Build

```bash
cargo test
cargo build --release
```

Apache-2.0.
