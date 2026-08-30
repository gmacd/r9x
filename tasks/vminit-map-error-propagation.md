---
id: 136
status: open
wave: 1
---

# Task 136: pre-MMU `map_phys_range` swallows every `map_to` failure

## Status: open

## Problem

**B1 (all three reviewers).** `aarch64/src/pre_mmu/vminit.rs:361-369`:
on a per-page `map_to` error the code prints
`"error:vminit:map_phys_range:map_to failed"` and *continues*; the return
(`:369`) is derived only from the zero-range check. `init_vm`
(`:160-171`) treats a partially mapped range as fully mapped and enables
the MMU over it. The post-MMU twin propagates with `?`
(`aarch64/src/vm.rs:560-567`) — a line-for-line copy that does the right
thing. Swallowed failure modes: early pool exhaustion (32 pages),
`EntryIsNotTable` (a 4 KiB mapping colliding with an existing 2 MiB
block — exactly what happens if the DTB ever lands inside a kernel 2 MiB
region), the recursive-index guard. Combined with task 135, the first
touch of an unmapped kernel page is a silent wedge.

**D20.** The doc at `vminit.rs:326-329` (verbatim at `vm.rs:523-526`):
"This aligns on page size boundaries, and rounds the requested range…" —
the code (`vminit.rs:341-348`, `vm.rs:540-547`) instead returns
`PhysRangeIsNotOnPageBoundary`. A script change that breaks alignment is
silently rejected, and with B1 the rejection is swallowed.

**D15.** Pre-MMU `next_mut` (`vminit.rs:410-415`):
`next_mut<'a>(table: &mut Table, …) -> Result<&'a mut Table, _>` — `'a`
is unconstrained by any input. Inside, `&mut` is minted from raw
addresses while `table` is still borrowed; `map_to` chains them. These
are overlapping mutable borrows of the page-table pool; it works because
the pointers happen to be distinct pages and LLVM isn't currently
exploiting `noalias`. The post-MMU twin ties its lifetime to `&mut
self`.

## Design

- Propagate the first `map_to` error with `?`, mirroring `vm.rs`.
- **Recorded decision (D20):** keep the reject behaviour and correct the
  doc (rounding would silently change what gets reserved — the
  allocator contract must stay exact), or implement rounding and update
  `boot::page_allocator` to reserve the rounded range. The choice
  constrains task 139's script change and unblocks task 137; record it in
  the resolution.
- `next_mut`: return `*mut Table` (keep the walk raw) or tie the lifetime
  to `table`.

## Tests

- Host unit test: pre-MMU `map_phys_range` with a failing allocator
  returns `Err`, not `Ok` (checklist D8.3). Blocker to remove first:
  `pre_mmu/mod.rs` declares both modules *private* and every error path
  calls `putstr` → `read_volatile(0xfe215040)`, which segfaults in a host
  build — the diagnostic sink needs abstracting (D8.2).
- `map_to` with a 4 KiB-unaligned range returns
  `PhysRangeIsNotOnPageBoundary` (D8.2); the policy is pinned on both the
  pre-MMU and post-MMU sides (D8.3) so the asymmetry recorded by D20
  cannot return.
- In-image: a store to a kernel-text VA and to a DTB VA takes a
  permission fault; a call into `.data` takes an instruction abort
  (D2.12, extending the `aspace_fault.rs` scaffolding).

## Done when

- The pre-MMU mapper errors propagate exactly like the post-MMU twin.
- The doc matches the implemented alignment policy, and the policy is a
  recorded decision with both sides pinned by tests.
- The host tests run under `cargo xtask test`.
- Full `cargo xtask ci` green.

Origin: pre-`main9` boot audit, 2026-08-30
(`plans/premain-boot-review-2026-08.md`, B1, D20, D15, checklist D8).
