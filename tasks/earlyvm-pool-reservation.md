---
id: 139
status: open
wave: 4
---

# Task 139: the live page tables are reserved from the allocator by section-ordering accident

## Status: open

## Problem

**D6 (all three, in pieces).** `aarch64/lib/kernel.ld:67-70`:

```ld
/* TODO Should this be here???  Better to put into the bss section later */
.end : ALIGN(2097152) {}
PROVIDE(bss_pa  = LOADADDR(.bss));
PROVIDE(ebss_pa = LOADADDR(.end));
```

`ebss_pa` is the LMA of an **empty** section (objdump: `.end` LMA == VMA
== `0xffff800002200000`; `LOADADDR` currently evaluates to the physical
`0x2200000` — `nm` verified). `.earlyvm_pagetables` sits *between*
`bss_pa` and `ebss_pa`, so `data_physrange().add(&bss_physrange())` in
`boot::page_allocator` (`aarch64/src/boot.rs:36-41`) swallows the early
pool — the **only** reason the allocator doesn't hand out TTBR1's own
tables. Compounding:

- `PhysRange::add` (`core/src/addr.rs:197-199`) is a **hull**
  (min-start/max-end), not a union — the reservation silently covers any
  gap between data and bss, and the fact that they abut is asserted
  nowhere;
- `kmem.rs` doesn't declare the `earlyvm_*` symbols (only
  `pre_mmu/vminit.rs`'s private copy does), so the pool is invisible to
  the module that owns the allocator contract;
- acting on the script's own TODO (move the pool, or change `ebss_pa`)
  *without* an explicit reservation makes the allocator hand out the
  live page tables. Nothing in the tree catches any of it.

**D14 (all three, in pieces).** `kmem.rs` and `pre_mmu/vminit.rs`
duplicate the `_pa` symbol block and the range list, and they have
drifted: `vminit` has the `earlyvm_*` symbols, `kmem` doesn't; `kmem` has
`total_kernel_physrange()`, `vminit` doesn't. The *range list* also
exists twice: `init_vm`'s `custom_map` (`vminit.rs:127-134`) and
`boot::page_allocator`'s `physranges` (`boot.rs:36-42`) must agree or the
allocator hands out mapped kernel memory (or the kernel runs on allocated
memory). They agree today; nothing enforces it. The copy is pure — the
pre-MMU module only reads linker symbols.

**C11.** `ebss_pa`'s comment claims "end of bss"; it is the LMA of an
empty 2 MiB-aligned section that happens to sit after `.bss`.

## Design

- Reserve the pool **by name** in `boot::page_allocator`: add the
  `earlyvm_*` symbols to `kmem.rs` and reserve the range explicitly.
- Stop `ebss_pa` pretending to be end-of-bss (the script change is
  constrained by task 136's alignment decision: the real end of `.bss` is
  4 KiB- but **not** 2 MiB-aligned, so what `ebss_pa` may become depends
  on round-vs-reject).
- One home for the `_pa` symbol block and the range list (`kmem.rs`),
  with the `custom_map` list derived from it so the two copies cannot
  drift.
- Assert adjacency where it is load-bearing.

## Tests

- D4.2: `bss_physrange()` contains `earlyvm_pagetables_pa..eearlyvm_pagetables_pa`.
- D4.3: `TTBR1_EL1` and `TTBR0_EL1` both fall inside the reserved ranges.
- D4.4: allocate pages in a bounded loop until failure; no PA falls in
  the DTB (4K-rounded) or kernel ranges.
- D4.5: `boottext.end == text.start`, `text.end == rodata.start`,
  `rodata.end == data.start`, `data.end == bss.start` — *equality* (all
  four hold in the current ELF), not the `<=` used in
  `aarch64/tests/pagetables.rs:59-62`.
- D4.11: `ebss_pa < 0x1_0000_0000` — it must be a physical address, not
  a KZERO offset (today `0x2200000`; a KZERO-valued `LOADADDR(.end)`
  would be `0xffff800002200000` and break D4.7's alignment check in a
  confusing way).
- D4.7: the `_pa` symbols keep their alignment (2 MiB for the block
  ends, 4 KiB for the page-granular ones).
- D4.10 (post task 142): the pool contributes no bytes to the flat
  image — compare `objcopy -O binary` size against
  `LOADADDR`/`SIZEOF(.earlyvm_pagetables)`.

## Done when

- The reservation is by name, the contract has one home, the equalities
  and the physical-address invariant are asserted in tests.
- Moving or re-aligning `.earlyvm_pagetables` in the script is caught by
  CI, not by the allocator handing out live tables.
- Full `cargo xtask ci` green.

Origin: pre-`main9` boot audit, 2026-08-30
(`plans/premain-boot-review-2026-08.md`, D6, D14, C11, checklist D4).
