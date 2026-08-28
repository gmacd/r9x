---
status: done
---

# Verbose QEMU runs can deadlock on a full stderr pipe

In verbose mode the qemu runner drains the child's stdout to EOF before
touching stderr (`xtask/src/main.rs:1383` area — single reader thread,
sequential streams). Pipes buffer ~64KB: if QEMU emits more stderr than
that (repeated device warnings, guest-triggered error reports) while its
stdout is still open, QEMU blocks in a write to stderr, never exits, and
the 60s deadline kills it — the harness then reports TIMED OUT for a run
that was passing. CI is exposed: `arch-ci` runs
`cargo xtask integration-test --arch <arch> --verbose`.

Classic two-pipe deadlock; the non-verbose path is not immune in
principle either wherever one stream is drained to EOF before the other
is opened.

Fix direction: one drain thread per stream (or poll both), joining both
before waiting on the child.

Done when: a verbose integration-test run whose QEMU writes >64KB to
stderr completes and reports the guest's real pass/fail, not TIMED OUT.

Origin: code review of xtask/CI (2026-08-20, high effort) — PLAUSIBLE
(mechanism verified; needs a chatty guest to trigger).

## Status: done (r9x2 working tree, 2026-08-20)

`qemu()` now spawns one `filter_and_print` thread per stream and joins
both after the child exits (or is killed at the deadline), so a full
stderr pipe can no longer block QEMU and the tail of output lands before
the result line. Verified: `cargo xtask integration-test --arch aarch64
--verbose` passes with QEMU's stderr warnings visibly interleaved; fmt
and clippy clean.
