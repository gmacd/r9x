---
status: done
commit: 4a74cda
---

# kernel.ld is written with unwrap + ignored write_all

The generated linker script is written with `File::create(...).unwrap()`
followed by a discarded `let _ = file.write_all(...)`
(`xtask/src/config.rs:231`). A failed write — disk full, permissions —
is silently ignored, leaving a truncated `kernel.ld` that the subsequent
cargo build links against, producing a mislaid or unbootable image with
no error pointing at the cause. This is the worst failure shape: not a
crash but a silently wrong artifact.

Fix direction: propagate both the create and the write errors through
the crate's Result type like every step in main.rs already does.

Done when: a failed kernel.ld write fails the xtask step with an error
naming the path, before cargo ever runs.

Origin: code review of xtask/CI (2026-08-20, high effort) — CONFIRMED.
Same file and theme as [xtask-config-link-script-panic.md]; land
together.

## Status: done (4a74cda)

`File::create` + `write_all` are chained and propagated with the path in
the message ("could not write `<path>/kernel.ld`"); the ignored
`create_dir_all` on the same path is propagated too. A failed write now
fails the step before cargo runs.
