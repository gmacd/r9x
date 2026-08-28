---
status: open
---

# Rename PhysRange::add and document what it actually does

Paths refreshed 2026-08-27: the type moved to `r9x-core` (commit
e4a403a); `port/src/mem.rs` is now a re-export shim.

`PhysRange::add` (`core/src/addr.rs:197-199`) is not arithmetic. It
returns the range spanning both operands — `min(starts)..max(ends)` —
and therefore silently swallows any gap between two disjoint ranges. The
name reads as `ops::Add` to anyone skimming — and `PhysAddr`, two screens
up in the same file, implements *real* `ops::Add` (addr.rs:109), which
makes the confusion sharper. The function carries no doc comment.

All four call sites combine adjacent *linker sections*:
`boottext_physrange().add(&text_physrange())` and
`data_physrange().add(&bss_physrange())` at `aarch64/src/boot.rs:38,40`
and `aarch64/src/pre_mmu/vminit.rs:123,125`. They depend on the
gap-swallowing behaviour erring toward marking *more* memory used, and on
the linker placing those sections adjacently. That invariant is recorded
nowhere. (No userland/cmd callers — verified repo-wide; the edit is in a
crate more crates depend on, but all callers are aarch64.)

Fix direction: rename to `span`, and add a doc comment stating that the
result covers any gap between the two ranges and why that is the desired
direction of error here. Rejected names: `hull` (computational-geometry
vocabulary, appears nowhere in r9 or the Plan 9 tree); `union` (actively
wrong — the result is not the union when the ranges are disjoint);
`merge` (implies adjacency, which is an assumption rather than a
guarantee).

The rename without the comment just relocates the trap.

Done when: `PhysRange::span` replaces `add` at all four call sites; the
doc comment states the gap behaviour; gates clean on all three
architectures.

Sequencing: was "after `range-by-value-sweep.md`, which changes this
method's signature" — the sweep has since landed, so that constraint is
lifted. Optional extra scope while touching it: the signature is still
`add(&self, other: &PhysRange)`; by-value (`self, other: PhysRange` —
the type is `Copy`) would match the sweep's direction. Separate commit —
it is a different logical change.

Origin: plan `tasks/plans/range-by-value.md`, decision record 4 (naming
and the missing invariant both raised by the clarity and whole-system
lenses).
