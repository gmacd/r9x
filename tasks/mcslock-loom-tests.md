---
id: 97
status: open
---

# Task 97: MCS lock and allocator concurrency tests

## Status: open

## Problem

AGENTS.md mandates multi-core correctness ("never assume single-core"),
but the tree has **no concurrency gate at all**: `port/src/mcslock.rs`
and `port/src/allocator.rs` have zero `#[test]`s (port's 55 tests live
in bitmapalloc/irq/elf/mem/ipc/maths/lib/once). Gate 49 (miri) was
specced as covering "the MCS lock, atomics, and allocator" — coverage
`cargo miri test -p port` cannot provide while those tests don't exist.

## Design

Host tests, in two tiers:

1. **Plain host tests** (land first, unblock gate 49): acquire/release,
   nested/contended two-thread acquire, allocator alloc/free patterns,
   allocator reuse-after-free of the same block. These give miri's
   provenance and data-race checking something to chew on;
   `-Zmiri-many-seeds` then explores interleavings.
2. **loom** (the stronger tool for schedule exploration, tokio's
   practice): model the MCS lock's atomics under loom's exhaustive
   scheduler behind a `#[cfg(loom)]` shim for the atomics imports.
   Firecracker/tokio show the pattern; kernels proper rarely carry it,
   which is an argument for it, not against — the lock is the piece
   everything else's correctness stands on.

If the loom shim turns out to be invasive for `port`'s atomics usage,
land tier 1 + miri many-seeds and record the decision here — but the
default is both tiers.

## Done when

- `mcslock.rs` and `allocator.rs` have host tests covering the cases
  above; `cargo xtask test` runs them.
- Either loom tests exist behind `cfg(loom)` with a CI invocation, or a
  recorded decision says why many-seeds miri suffices.
- Gate 49's acceptance ("miri exercises the lock and allocator") is
  actually true.

Origin: backlog audit 2026-08-27 (gates group — the plan's biggest
coverage gap against the project's own SMP charter).
