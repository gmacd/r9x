---
id: 142
status: open
wave: 7
---

# Task 142: early table lifetime and layout hygiene

## Status: open

## Problem

Five findings about the early tables' lifetimes and the layout they sit
in (audit D4, D5, D10, D12, D17):

**D4 (all three, in pieces).** The TTBR0 identity block (1 GiB at PA 0,
see plan ruling 1) outlives its transition, and its comments say
otherwise: `vminit.rs:187-188` "Once we've jumped to the higher half,
this will no longer be used"; `vminit.rs:283-286` "…during the transition
until the PC is also in the higher half … Once we enter rust-land, we
can define a new set of tables". Neither is true: `l.S` never touches
TTBR0 after `init_vm`; `main.rs:47-50` replaces it only in
`vm::switch(user_pagetable(), User)`, which the *full* kernel reaches
after irq_ops, DTB parse, page allocator, mailbox, console and
interrupts — and **none of the short test images calls it at all** (21 of
28 images call `vm::switch`; none of `pagetables`, `miniuart`, `pl011`,
`clock`, `irq`, `timers`, `allocate` do), so they run to completion with
the block live. While live, PA `[0, 1 GiB)` — the whole kernel image and
the QEMU DTB at `0x8000000`, the entire low GiB on the metal — is
PrivRw (read/write/EL1-executable) at low VAs: a stray low pointer
reads/writes/`exec`s real memory instead of faulting.

**D5 (opus).** `vm.rs:689-701` (`init_empty_root_page_table`) documents
the invariant that every root page table carries a self-pointer at entry
511 (what `replace_recursive_entry` and the recursive walk rely on).
`init_vm` writes it for the kernel root (`vminit.rs:146-150`) but not for
`physicalpt4` (`vminit.rs:208-217`; audit dump: `[511]` is zero), yet
`physicalpt4` is installed in TTBR0. Between MMU-enable and
`vm::switch`, any `RootPageTableType::User` operation works on a root
that breaks the invariant; a test image that builds an `Aspace` without
switching would.

**D10 (fable, opus).** Boot stack: 16 KiB (`l.S:3`), no guard, abutting
the page-table pool — the build-independent invariant is
`stack + STACKSZ == earlyvm_pagetables_pa`, exactly, with zero gap. A 16
KiB overflow silently walks down through `.bss` statics. Plausible in
debug builds: `init_vm` alone takes a ~0x260-byte frame and the DTB
parser recurses.

**D12 (opus).** `.earlyvm_pagetables` is emitted as CONTENTS, not NOBITS
(objdump: type DATA, no NOBITS), so `objcopy -O binary` materialises the
128 KiB of zeros — the flat image is 33,968,128 bytes (stale `.gz` build)
ending exactly at `eearlyvm_pagetables_pa`. The zeros are redundant
(`l.S` and the allocator both clear the region) and they make the image
size depend on the pool size.

**D17 (opus).** Nothing checks that the kernel image does not overlap the
DTB. The debug image spans `0x80000..0x20e4000` (~33 MiB; `.rodata` alone
is 30 MiB) and the QEMU DTB sits at PA `0x8000000` — a quarter of the way
there. If the image ever reaches `0x8000000`, the loader overwrites the
DTB and `DeviceTree::from_usize(…).unwrap()` panics silently (task 135's
window, before the fix).

## Design

- TTBR0 block: correct the comments and state the lifetime; make the
  block PrivRo (the transition only needs EL1 execute); have the test
  images switch TTBR0 to a null root at the end of their boot prefix.
- `physicalpt4`: write the recursive slot too (keeps the invariant
  universal), or scope the invariant to "the kernel root" and make
  `replace_recursive_entry(User, …)` refuse the early root.
- Stack: a guard page or a bigger `STACKSZ`.
- Pool: emit `.earlyvm_pagetables` as NOBITS (pairs with task 139's
  reservation-by-name and unblocks checklist D4.10).
- DTB overlap: assert `total_kernel_physrange().end <= dtb_pa` in an
  image (and in the loader, once the flat-image size stops including the
  pool).

## Tests

- D3.1: at `main9` entry, `TTBR0_EL1 != TTBR1_EL1` and TTBR0 translates
  VA 0 to PA 0 (documents the window; makes any change visible).
- D3.2: `physicalpt4.entries[511]` carries the intended contract
  (post-fix: self-pointer, per D5).
- D3.3/D3.4: after `init_user_page_tables()` + `switch`, VA 0 and
  VA `0x80000` unmapped; and for an image that *never* switches, the
  low-VA behaviour matches the documented policy.
- D3.6: the identity block is `PXN == false` (required for the
  transition) and `UXN == true`; post-fix, AP per the documented policy.
- D4.9: SP at the `bl main9` handoff satisfies
  `stack <= sp <= stack + STACKSZ` — at the handoff SP *equals* the
  exclusive end (the pool's first byte), so the assertion must be
  inclusive.
- D2.14: enumerate every table reachable from the root (visited set):
  phys addrs inside the earlyvm pool; count as expected; headroom within
  margin of 32.
- D4.6: `total_kernel_physrange().end <= dtb_pa`.
- D4.10 (post NOBITS): the pool contributes no bytes to the flat image.

## Done when

- The block's lifetime, permissions and recursive slot match the
  comments; the test images switch to a null root.
- The stack has a guard (or headroom) and the layout pin passes.
- The flat image no longer includes the pool; the DTB-overlap assertion
  passes.
- Full `cargo xtask ci` green.

Origin: pre-`main9` boot audit, 2026-08-30
(`plans/premain-boot-review-2026-08.md`, D4, D5, D10, D12, D17,
checklist D3/D2.14/D4.6/D4.9/D4.10).
