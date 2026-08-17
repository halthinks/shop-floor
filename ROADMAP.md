# shop roadmap

This PR is the floor: hold-split-join, workers as capacity, GitHub (public now, private with token), project picker, Talk to the boss, shop memory, floor memory, file-based reply ingest.

Shop is the AASM proving ground. It does not copy the kernel. See `src/aasm_map.rs` and `docs/AASM_PROVING_GROUND.md`. Proving-ground tests in `tests/aasm_proving.rs` stay required.

Next, in order:

1. Run workers
   Launch the assigned AI (Grok Build / Cursor / tagged backend). Cut an isolated git worktree per child. Wait on a real process. A name on the roster is not a running worker.

2. Windows binary + CI
   `cargo build --release` artifact people can run on Windows. GitHub Actions: test on Linux, release a Windows binary. `shop ui` has to be a thing you start, not a cloud-agent myth. GitHub Actions CI now exists for this item.

3. More than one project
   Multiple open floors. Switch without losing the other project's memory/floor.

4. Boss sees the screen
   SuperGrokHeavy gets a live floor view (status snapshot or image), not only the memory pack in a steer JSON.

Later: this floor is what lets a TextPCB shop exist. Not in this PR.
