# shop is the AASM proving ground

AASM is the kernel. Shop is the floor that hits the laws until they break.

Shop does not copy the reducer, the calculus, or the effect plane. It does not vendor AASM. Floor words stay shop words (`bounce`, `WAIT`, `held`, reduce, verify). The map is `src/aasm_map.rs`.

The contact surface is `tests/aasm_proving.rs`. Those tests must fail closed:

- a handoff, mailbox reply, or GitHub check is Evidence — it cannot CLOSE a parent
- missing evidence is INCONCLUSIVE / INFORMATION_GAP / UNKNOWN, never VERIFIED
- a parent cannot complete while a mandatory child is still ASSIGNED
- bounce is a causal backjump: same peer/paths; JOINED siblings survive
- child paths stay inside the parent claim set (no amplification)
- reduce does not claim AASM recombine
- a command or merge ACK is not achieved state (no auto VERIFIED from a merge record)

If a test here goes green by inventing authority, the floor is lying.
