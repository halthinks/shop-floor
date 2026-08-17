# shop

Thin shop-floor sequencer: **hold → split → join → reduce → verify → close**.

`shop` is not [AASM](https://github.com/halthinks/AASM) and not T3 Code. AASM stays the authority calculus. This crate only keeps durable parent-job state so isolated workers can be sequenced without overlapping `allowed_paths`. Quantity of workers is not the problem. Sequencing is.

## What it is not

- Not an authority/solver kernel. Do not treat shop state as truth.
- Not a PCB/CAD model. No domain types live here.
- Not a merge bot. `shop reduce` writes a package under `.shop/`; it does not land onto a canonical repo.

## Authority (enforced)

These rules come from AASM's evidence/authority split and are enforced in code:

- A handoff is **Evidence**, not truth.
- A CLI/test exit 0 is **Evidence**, not authority to `close` unless `shop verify` recorded a PASS.
- `shop` never invents a child lane. Only `shop split` (human/SuperGrok) creates children.
- Incomplete evidence is **WAIT**, never a fake PASS.
- `shop` writes only inside the shop store and an optional mailbox outbox.

## Loop

```text
shop init
shop open --id PARENT --title "..." --body "..."
shop split PARENT --child C1 --peer PEER --paths a,b --title "..." --body "..."
shop assign PARENT
# workers run; they do not share paths
shop accept PARENT --child C1 --handoff ./handoff.json
# or: shop bounce PARENT --child C1 --reason "..."   (same peer/paths only)
shop join PARENT          # all JOINED -> REDUCE_READY; else stay put
shop reduce PARENT --note "..."
shop verify PARENT --cmd "cargo test"
shop close PARENT         # only from VERIFIED
```

Parent: `HELD → IN_FLIGHT → REDUCE_READY → REDUCED → VERIFY_WAIT|VERIFIED → CLOSED`

Child: `ASSIGNED → JOINED | BOUNCED` (bounce may return to `ASSIGNED` on the same peer)

### Claim lock

`split` rejects any `allowed_paths` overlap with an in-flight child of any open parent. That is the collision ledger.

### Bounce

A bounced child stays tied to the same peer and paths. `shop assign` may return it to `ASSIGNED` on that lane only.

### WAIT, not fake PASS

`verify` records exit code and last lines. A recorded exit 0 → `VERIFIED`. Failure, missing command, or missing evidence → `WAIT`. `close` is refused unless verify recorded PASS.

## Mailbox

`--mailbox DIR` writes TextPCB agent-bridge v1 assign JSON (`schema_version=textpcb/agent-bridge/v1`, `type=assign`, `from=cursor-groksuperheavy` by default). If that directory is missing, `assign` still writes `.shop/outbox/`. Tests do not need a live Windows mailbox.

## Store

Durable JSON under `.shop/` (or `shop init DIR`): snapshot, per-parent files, claim ledger, outbox, reduce packages. Writes are temp + rename. No database.

## Build

```bash
cargo test
cargo build --release
```

Apache-2.0.
