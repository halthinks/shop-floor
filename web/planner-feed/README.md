# planner-feed

`planner-feed.json` is the poll target for `web/planner-feed/index.html`.
The top-of-page box reads this file. It does not invent planner thoughts.

## Schema

```json
{
  "updated_at": "ISO-8601 Z",
  "planner": "cursor-groksuperheavy",
  "thoughts": [{"t": "ISO-8601 Z", "text": "..."}],
  "actions": [{"t": "ISO-8601 Z", "kind": "assign|handoff|status|merge", "text": "..."}]
}
```

`kind` is one of `assign`, `handoff`, `status`, `merge`.
Seeded `text` values start with `EXAMPLE:` and are not shop metrics.

## How the live room writes it

A live room / `worker_runtime` overwrites `web/planner-feed/planner-feed.json` from:

1. SuperGrok mailbox — `inbox/cursor-groksuperheavy` (and shop ingest of `steer-reply` | `handoff` | `reply` from `cursor-groksuperheavy`)
2. The out log — `.shop/outbox/` assign/steer records plus `.shop/events.jsonl`

Write the whole object atomically. Set `updated_at` to the write time (ISO-8601 Z).
`planner` stays `cursor-groksuperheavy`.

If mailbox or out log is missing or incomplete, keep the last good file or write WAIT in `text`. Never invent a thought, action, PASS, TextPCB lane, or `C:\TextPCB Platform`.
