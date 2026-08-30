---
id: 143
status: open
wave: 8
---

# Task 143: pre-`main9` comment and dead-code sweep, plus two recorded decisions

## Status: open

## Problem

Eighteen comment findings (C1–C18, full text in the audit) plus seven
small design risks that are mechanical or decision-only. The comments
matter here because the boot path is the one place in the tree where a
wrong comment is a *lie about hardware state* rather than a stale note.

Highlights (the audit carries all of them with both sides quoted):

- **C1:** `MAIR_EL1` attr1 is `0x04` = Device **nGnRE**, not the
  nGnRnE the comment claims (`0b01` in bits [3:2] is nGnRE).
- **C2:** all four GPIO-funcsel mask comments in `pre_mmu/util.rs:51,56,63,68`
  describe the *inverse* of their masks (the code is right).
- **C4:** `l.S:40` "throught", `:130`/`:137` "be be seen"; the header
  register note at `l.S:42` is wrong on x28 (dead), x20 (clobbered,
  unlisted); `mov x28, x4` at `l.S:50` is a dead register with a wrong
  comment; "secrtion" at `vm.rs:101`.
- **C13:** `current_core`'s doc (`irq.rs:91`) names QEMU machines aarch64
  never runs on (`q35` is the x86-64 machine, `virt` is the riscv64 one in
  this tree). The mock's `DAIF_IRQ_BIT = 1 << 7` is **correct** (as read
  by `mrs`, DAIF.I is bit 7); only the target list needs fixing.
- **C14:** `xtask/src/main.rs:754` calls the PL011 the early console; the
  early console is the mini-UART (serial_hd(1)), which xtask sends to
  `null` (task 135 fixes the sink).
- **C17:** stale `.quad (0*2*GiB) + (PT_BLOCK|…)` assembler fragments in
  `vminit.rs` referencing macros no longer in the tree (the value 0
  matches the entry's PA, but the implied 2 GiB stride doesn't match the
  level-1 1 GiB stride it annotates), commented-out debug blocks, dead
  `puttable`; `entry_mut` is `pub` in a private module and its `Result`
  has no `Err` arm; `PhysPage4K::clear` passes `count = 1` to
  `volatile_set_memory` over a 4 KiB buffer — correct, but reads as a
  one-byte write.

Design risks folded in:

- **D9 (opus):** KZERO is defined three times, in three languages, with
  no cross-check (`kernel.ld:6`, `l.S:35`, `param.rs:2`); a mismatch dies
  silently at `msr sctlr_el1`.
- **D11 (fable, opus):** the BSS-clear loops are do-while (`l.S:110-114`
  and `:116-120`) — an empty or inverted range clears memory until
  wraparound; currently unreachable, guarded only by linker order, which
  task 139 shows is soft. Also: the second loop re-zeroes the pool the
  first just zeroed, and the allocator zeros it again — the pool is
  cleared up to five times for one invariant.
- **D13 (recorded decision):** SError (DAIF.A) is masked at reset and
  never unmasked (`irq.rs:42`, `DAIFClr, #2`); the SError handlers in the
  vector table are unreachable and RAS is suppressed for the life of the
  kernel with no log line. Decide: unmask A once the vectors are
  installed and make the handler ack RAS, or document running RAS-off.
- **D16:** the mini-UART clock is hard-coded to 500 MHz
  (`pre_mmu/util.rs:33-35`) — it is the VPU *core* clock, which
  `core_freq`/`force_turbo`/`arm_boost`/throttling all move; the
  canonical mitigation (`core_freq_min=500` / `enable_uart=1` in
  config.txt) is documented nowhere. Also `init_early_uart_putc` spins on
  LSR bit 5 unbounded. Cross-ref task 127 (metal).
- **D18:** the self-pointer entries are PrivRw, Normal, `PXN=0`
  (executable) and non-shareable, while the same physical pages are
  mapped Inner-shareable by the kernel data blocks. The `SH`/`AP`/`PXN`
  bits are ignored when the descriptor is walked *as a table* but apply
  when it is reached *as a leaf* through the recursive alias (audit dump:
  root self-pointer `0x00000000020c5403` → PrivRw, Non-shareable, PXN=0,
  UXN=0, while the same page is covered by a 2 MiB block at SH=Inner).
  Build them from the same attribute set as `rw_kernel_data()` — still
  required for the root after task 138 removes the L1/L2/L3 copies.
- **D22:** `.bootdata` is lumped into the RO+X `.text.boot` mapping
  (`kernel.ld:14-15`), and the section is declared `"awx"` (`l.S:37`) —
  write permission the mapping does not grant. Same mismatch from both
  sides.
- **D23 / C17 remainder:** the commented-out debug block and `.quad`
  fragments in `init_vm`'s table-construction path are dead code dressed
  as documentation.

## Design

- Fix the comments to match the code (or the code to match the comments,
  per finding — the audit records which side is right in each case).
- D9: single-source KZERO where possible, and pin the rest with the
  in-image check in task 137's D4.8 (linker-placed symbol VA vs its
  `_pa` twin).
- D11: make the BSS loops while-style (test-before-store); drop the
  redundant pool re-zeros once task 139's reservation is explicit.
- D13: record the decision in the resolution and in `docs/lessons.md`.
- D18: derive the self-pointer attributes from `rw_kernel_data()` (or an
  explicit equivalent) rather than hand-set bits.
- D22: separate `.bootdata` (or give `.text.boot` honest attributes).
- D16: document the config.txt mitigation in `docs/`; note the unbounded
  spin; metal validation in task 127.

## Tests

- `gate-typos`-style: no new typos; the existing ones are gone.
- D8.5 (host): `Entry` bitstruct round-trip against hand-computed
  constants using the real descriptor values confirmed live in the guest
  (`0x0040000000000781` text 2M block, `0x0060000000200781` rodata 2M
  block, `0x0060000008000783` DTB 4K leaf) — the cheapest guard against a
  silent `Entry` field-offset change.
- D2.13: the root self-pointer matches the derived attributes (pairs
  with task 138's D2.13).
- Full `cargo xtask ci` green across the three architectures (the comment
  fixes must not touch behaviour).

## Done when

- Every C1–C18 site matches its code (or the code matches the comment,
  per the audit); the two decisions (D13, and the D9 single-sourcing
  approach) are recorded.
- The do-while loops are while-loops; the dead code is gone.
- Full `cargo xtask ci` green.

Origin: pre-`main9` boot audit, 2026-08-30
(`plans/premain-boot-review-2026-08.md`, C1–C18, D9, D11, D13, D16, D18,
D22, D23, checklist D8.5).
