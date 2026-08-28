---
id: 94
status: done
---

# Task 94: map_to error path leaks the recursive entry; no read-back

## Status: open

Two latent defects found while auditing the (wrongly) blamed recursive
walk for task 87.

## Problem 1 — Err arm leaves TTBR0[511] clobbered

`map_to` (`aarch64/src/vm.rs`) swaps the **live** root's entry 511 for
the target table's recursive entry (`vm.rs:438-440`), but the `Err` arm
returns (`vm.rs:456-465`) without calling
`write_recursive_entry(pgtype, old_recursive_entry)` — only the success
path restores it (`vm.rs:473`).

Harmless when mapping the live table (temp == old). But when mapping a
**non-live** root — exactly what `sys_spawn`'s `map_user_page` into a
child `Aspace` does (`process.rs:600-620`, `aspace.rs:141-176`) — an
allocation failure mid-walk leaves the *caller's* TTBR0[511] pointing at
the *child's* root table. Every later recursive access from the parent
then walks the wrong tree.

Fix: restore the old recursive entry on every exit path (a scope guard,
or restructure so the swap/restore brackets the walk unconditionally).

## Problem 2 — Ok without verification

`map_to` returns `Ok(())` after `write_entry` (`vm.rs:472`) without
reading the entry back. This was the one accurate observation in the old
task-87 file. A read-back assertion (walk the just-written slot, check
valid + PA) is cheap and is exactly Zircon's `ArchVmAspace` Map→Query
pattern; Plan 9's `mmukmap` goes further and refuses to overwrite a
live entry at all (`sys/src/9/bcm/mmu.c:288`).

## Tests

A host test using the existing `TestVmTrait` with a page allocator that
**runs dry mid-walk**: assert (a) the error is returned, (b) the root's
entry 511 still holds its original value afterward. This row also
belongs in task 91's matrix (section B).

## Done when

- Both exit paths restore the recursive entry; the read-back assertion
  is in place.
- The allocator-runs-dry host test passes.
- Full `cargo xtask ci` green.

Origin: backlog audit 2026-08-27 (VM group, spun off the task 87
rediagnosis).
