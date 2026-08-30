---
id: 137
status: open
wave: 2
depends-on: 136
---

# Task 137: "Kernel Text" maps and reserves PA `[0, etext)` — the null page does not fault

## Status: open — wave 2.  Depends on 136's alignment decision

## Problem

**B5 (all three, in pieces).** `aarch64/src/pre_mmu/vminit.rs:37-39,85-87`:
`boottext_physrange = PhysRange::new(base_physaddr() /* = 0 */, eboottext)`,
so "Kernel Text" is PA `0x0..0x200000` (debug build) and its first 2 MiB
block maps `KZERO + 0`. What is *behind* that page is not kernel code:
the LMA is `0x80000`, so PA 0 holds QEMU's machine/boot data on QEMU and
firmware structures on the metal (audit: `pmemsave 0 0x100` shows
machine data; `l.S`'s first word `0xaa0003fb` lives at `0x80000`).
Consequences:

- the `TODO leave the first page unmapped to catch null pointer
  dereferences` (`vminit.rs:117`) is violated: a kernel-mode null *read*
  or *jump* through `KZERO + 0` hits mapped memory instead of faulting
  (writes do fault — PrivRo);
- PA `[0, 0x80000)` — below the LMA, firmware territory on the metal —
  is claimed as kernel text in the mapping **and** in
  `boot::page_allocator`'s reserved list;
- in the debug build a full 2 MiB of RAM is permanently reserved for
  non-kernel pages.

**Why this is not a one-line fix** (audit final gate): "start the
boottext range at `boottext_pa` (`0x80000`)" is rejected as stated — the
range is mapped with `PageSize::Page2M` (`vminit.rs:129`) and
`map_phys_range` rejects non-2 MiB-aligned ranges (task 136's D20), so
`0x80000` panics `init_vm`. And "leave page 0 unmapped" needs 4 KiB
granularity across the first 2 MiB, which collides with the existing 2 MiB
block (`EntryIsNotTable`).

## Design

A mapping-strategy decision, recorded in the resolution:

- split the first 2 MiB into 4 KiB leaves (page 0 left unmapped, the rest
  of the block mapped as today), or
- map the boottext at its true start with a different page size, or
- keep the 2 MiB block and accept the null page mapped PrivRo (document
  the decision and close the `vminit.rs:117` TODO with a rationale).

Whichever is chosen: the allocator's reserved list must match the
mapping (task 139 gives both one home), and the policy is pinned by a
test.

## Tests

- D3.5: `KZERO + 0` is / is not mapped — the pin, so acting on the policy
  is a one-line test change.
- D7.3: a read of an unmapped kernel VA in the KZERO window faults (if
  the policy unmaps page 0).
- The golden transcript (task 135) is updated to the new "Kernel Text"
  start; D1.6's alignment assertion follows the chosen page size.
- D4.8: `param::KZERO` matches the linker (a linker-placed symbol's VA
  versus its `_pa` twin — the circular `va - phys(va)` derivation is
  uninformative; the `l.S` copy is not observable at `main9`).

## Done when

- The null-page policy is a recorded decision, pinned by D3.5.
- The mapping and the allocator reservation agree.
- Full `cargo xtask ci` green.

Origin: pre-`main9` boot audit, 2026-08-30
(`plans/premain-boot-review-2026-08.md`, B5, checklist D3.5/D7.3/D4.8).
