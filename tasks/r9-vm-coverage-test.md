---
id: 91
status: open
---

# Task 91: Full VM integration test with good coverage of the VM codebase

## Status: open

## Problem

The VM code (`aarch64/src/vm.rs`, 900 lines, plus the pre-MMU duplicate in
`aarch64/src/pre_mmu/vminit.rs`) is among the most correctness-critical
surfaces in the kernel, and its current tests leave two structural gaps:

1. **The production recursive walk is never exercised by a host test.** The
   only host unit test that drives `map_phys_range`
   (`map_phys_range_test`) uses a `TestVmTrait` whose `resolve_entry_mut`
   dereferences the entry's PA directly. It bypasses `recursive_table_addr` /
   `VmTraitImpl::resolve_entry_mut` — the recursive page-table trick that the
   *real* `map_to`/`next_mut` depend on. So the code path that actually runs
   on hardware has no host-level coverage at all.

2. **The live per-process ASpace dynamic-mapping path is only covered
   indirectly.** `aspace.rs` (isolation) and `aspace_fault.rs` (fault
   isolation) spawn raw programs at fixed VAs; neither maps a page into a
   *fresh* `Aspace`'s TTBR0 and verifies a value round-trips through the new
   mapping. There is no test that calls `map_user_page` /
   `map_user_data_page` / `map_mmio` and then reads the result back through
   the live MMU. There is no test of the 2 MiB or 1 GiB page sizes, of
   overwriting an existing mapping, or of a mapping at a VA that forces all
   four walk levels to be created.

Honesty note (2026-08-27 audit): this task was motivated by task 87, but
87 turned out to be a *bus error from a wrong physical address*
(`r9-mailbox-mmio-fix.md`), not an invalid PTE — the mapping was valid,
and rows 11/14 as originally imagined would have passed without catching
it. The tests still earn their keep: they pin the recursive walk, the
descriptor encodings, and the live-ASpace mapping path against future
regressions, and the matrix below now also covers what the audit showed
nothing covers (W^X, live permissions, the map_to error path, SMP).

## What "good coverage" means here

`vm.rs` is `no_std` aarch64 target code — it cannot run under the host
toolchain, so a line-coverage percentage is not the bar. The bar is
**assertion coverage**: every public VM behaviour in the matrix below has at
least one assertion (host unit test for pure functions, QEMU image assertion
for live-MMU behaviour) that **fails if that behaviour is broken**. Each row
is a checkbox; the task is done when every row has a named assertion that
passes — with two documented exceptions: row 19's fault half and row 2's
`rw_user_text` sub-test track task 96 (they pin the *corrected* W^X
encoding, so they land with 96 or are written against it), and row 11's
Mailbox variant lands after task 87's mailbox fix (its PA/offsets come
from that fix).

### Coverage matrix

| # | Behaviour (API) | Kind | Assertion must prove |
|---|-----------------|------|----------------------|
| 1 | `Entry::empty()` is all-zero | host | `entry.0 == 0`, `valid()==false` |
| 2 | Every `Entry` constructor's encoding | host | raw `u64` has the exact Valid/Type/MAIR/AP/Shareable/Accessed/PXN/UXN bits for `rw_kernel_data`, `ro_kernel_data`, `ro_kernel_text`, `rw_device`, `rw_user_mmio`, `rw_user_text`, `rw_user_data` (one subtest each). **Caution:** `rw_user_text` is W+X today (`vm.rs:181-190`) — do not snapshot-bless it; that sub-test asserts task 96's corrected RO+X encoding and lands with 96 |
| 3 | `with_phys_addr` / `phys_addr` round-trip | host | `(e.with_phys_addr(p)).phys_addr() == p` for a page-aligned `p` |
| 4 | `is_table(level)` | host | true for a table descriptor at its level, false for a page descriptor; a 4K page entry is `is_table(Level::Level3)==false` |
| 5 | Table-descriptor encoding pinned to the ARM spec | host | a **passing** snapshot test records the current `next_mut` bytes (regression guard), and a **passing** spec assertion checks `bits[1:0] == 0b11` (the VMSAv8-64 TABLE type at L0–L2) and that the hierarchical controls `[63:59]` are deliberately zero — see the corrected note below |
| 6 | `va_index` / `va_indices` | host | correct for `0x0`, `0x10_0000`, `0x7000_0000` (→(0,1,384,0)), `0x8000_0000` (→(0,2,0,0)), and the kernel high-half sample already covered |
| 7 | `recursive_table_addr` for **User** type | host | the (511,…) pattern holds for `RootPageTableType::User` at each level (today only Kernel is asserted) |
| 8 | `map_phys_range` produces a **structurally correct** table | host | walk the resulting table: each L0/L1/L2 slot points at the next allocated table; each L3 slot has the right PA and the requested MAIR/AP/page bit — not just the returned `VirtRange` |
| 9 | `Aspace::new` + `install` + text round-trip | QEMU | a fresh AS's TTBR0, a `map_user_page` of known bytes, a spawned program reads them back and exits with a status derived from the bytes |
| 10 | `map_user_data_page` round-trip | QEMU | process writes a canary to its mapped data page and reads it back |
| 11 | `map_mmio` round-trip | QEMU | map the PL011 (`0xFE20_1000`) into the process's TTBR0; the process performs a **non-faulting read** (FR/LSR) and reaches its sentinel exit — proof the Device mapping works, no console write. A second variant maps the **real** Mailbox page (`0xFE00_B000` @ `0x7000_0000` — the old spec's `0xFE00_0000` is an unassigned bus hole that faults regardless of any VM code, see `r9-mailbox-mmio-fix.md`) and reads STATUS at page offset `0x898` — deterministic, non-faulting, and it exercises Device attributes + the `0x7000_0000` walk; lands after 87's mailbox fix |
| 12 | Page sizes 2 MiB (kernel) and 1 GiB (host) | QEMU + host | 12a: a kernel section's L1 slot is a 2 MiB **block** descriptor, not a 4 K chain (the production 2 M path, `vminit.rs`). 12b: `map_phys_range` with `Page1G` via `TestVmTrait` yields a 1 GiB block descriptor — **1 G has no production caller; this covers a latent feature, not a live path** |
| 13 | Overwrite an existing mapping | QEMU | map VA `v` to page A, then to page B; a read back yields B's canary |
| 14 | Non-trivial 4-level walk VA | QEMU | map at `0x7000_0000` (L2 index 384, forces a fresh L2/L3 chain) and round-trip — the task-87 VA |
| 15 | Misaligned range rejected | QEMU or host | `map_phys_range` returns `PhysRangeIsNotOnPageBoundary` for a range off a page boundary |
| 16 | Zero-length range rejected | host | `map_phys_range` returns `PhysRangeIsZero` |
| 17 | Pre-MMU kernel table is walkable + recursive slot valid | QEMU | extend `pagetables.rs`: a known data symbol resolves to its physical page via a manual `kernel_pagetable()` walk, and entry 511 is a valid table descriptor |
| 18 | Kernel-side walk after map (Zircon Map→Query) | QEMU | after each row-9/10/11/13/14 mapping, a `vmdebug` helper walks TTBR0 and asserts the leaf PA **and attribute bits** — not just the user-visible round-trip (this is the read-back task 94 adds to `map_to` itself, asserted independently) |
| 19 | Live permission enforcement | QEMU | a process jumping into its `rw_user_data` page (UXN=1) is killed with FAULT_STATUS; the store-to-own-text half lands with task 96 (text is W+X today) |
| 20 | `map_to` error path restores the recursive entry | host | `TestPageAllocator` runs dry mid-walk: the error is returned **and** the root's entry 511 still holds its original value (task 94's regression test; lives here in the matrix) |
| 21 | No W+X user page | host | walk a built table and assert no entry is user-writable and user-executable (the `ptdump_check_wx` idea) — lands with task 96, which removes the one offender |

**Note on row 5 (corrected 2026-08-27 — the original note had the spec
inverted):** per the Arm ARM (VMSAv8-64 descriptor formats), at levels
0–2 **`bits[1:0] = 0b11` is the TABLE descriptor and `0b01` is the BLOCK
descriptor**; `0b11` at level 3 is the page descriptor. So
`next_mut`'s / `init_empty_root_page_table`'s
`Entry::rw_kernel_data().with_phys_addr(pa).with_page_or_table(true)`
already produces the architecturally **correct** type (double-checked
2026-08-27 against Linux `pgtable-hwdef.h`: `P4D/PUD/PMD_TYPE_TABLE =
0b11`, `PMD_TYPE_SECT` (block) = `0b01`, `PTE_TYPE_PAGE = 0b11`). The
extra attribute bits it sets are architecturally harmless: per the
table-descriptor format, **bits[11:2] and bits[58:52] are IGNORED** —
the PE makes no use of them — so MAIR idx (2-4), AP (6-7), SH (8-9),
AF (10), and the leaf PXN/UXN positions (53/54) are all in IGNORED
ranges; the *defined* high fields are the hierarchical controls
PXNTable(59)/XNTable(60)/APTable(62:61)/NSTable(63), and RES0 is only
[51:48], which this construction leaves zero. (A prior review pass
re-labelled MAIR/AP/SH as RES0 and AF/PXN/UXN as "defined fields" —
both wrong: IGNORED is an architectural guarantee, so the current
bytes are safe on strictly-conformant silicon, not just under QEMU's
tolerance. Linux zeroes those bits by construction, which proves
nothing either way.) This also resolves the note's old mystery: the
earlier "clean descriptor" attempt that "crashed the kernel in QEMU"
installed `0b01` — a *block* descriptor — at L0–L2; of course it
crashed. Row 5 is therefore two **passing** tests: the snapshot
regression guard, and a spec assertion that `bits[1:0] == 0b11`,
hierarchical controls `[63:59]` deliberately zero, and RES0 `[51:48]`
zero. There is no discrepancy to document, no `#[ignore]`, and nothing
routes to task 87 (which was a wrong-PA bus error, unrelated to
descriptor encoding). Optional tidy-up, separate commit: construct
table descriptors without the ignored attribute bits purely for
readability.

**Unmap:** no unmap exists anywhere in `vm.rs` — teardown is explicitly
deferred (`aspace.rs:38-42`) — so this matrix has no unmap row *by
necessity, not omission*. Add one when the teardown arc lands.

**Break-before-make (note, no assertable row yet):** `write_entry`
(`vm.rs:705-710`) replaces a live valid leaf directly with the new valid
entry, then TLBIs. ARM requires invalid→TLBI→new when changing a live
entry (Linux enforces the safe subset with `pgattr_change_is_safe`
BUG_ONs, `mmu.c:120-153`). QEMU's forgiving TLB means no test can catch
a violation from the outside; when BBM is implemented, add a row
asserting the code takes the invalid-first path. Related: overwrite
policy is currently *silent replace* — row 13 pins that consciously
(Plan 9's `mmukmap` refuses overwrite; xv6 panics; either policy is
fine, the point is a test pinning whichever is chosen).

**SMP (note):** page-table mutation is completely unlocked — two cores
racing `map_phys_range` on intermediate-table creation, or the shared
entry-511 swap (`vm.rs:712-727`), is undefined today despite the
project's SMP charter. A locking decision precedes any assertable test;
record it here when made.

## Design

### A. New QEMU integration image — `aarch64/tests/vm.rs`

A whole-kernel image in the shape of `aspace.rs` (link the real library, `l.S`
runs early boot, then `main9`). Bring up only what is needed:
`boot::page_allocator`, `mailbox::init`, `boot::console`, `boot::interrupts`,
and `vm::init_user_page_tables`.

Core technique — **canary round-trip**, two modes depending on whether the page
is kernel-reachable:
- **Kernel-stamped** (page in TTBR1, via `map_user_page`, which returns the
  kernel identity pointer): the kernel writes a known canary, installs the AS,
  and spawns a program that reads the page into `x8` and exits with it. `status
  == canary` proves the *right* physical page; `status == FAULT_STATUS` (killed,
  not `canary`) proves the mapping is broken. Used for rows 9, 13, 14.
- **Process-self** (page only in TTBR0, via `map_user_data_page`, which is
  deliberately *not* in TTBR1 so the kernel cannot write it): the program
  stores a canary to the page and reads it back into `x8`, exiting with it.
  Reaching the sentinel (not `FAULT_STATUS`) proves the RW data mapping works.
  Used for row 10.

The identity-sensitive rows (12a, 13) use a **distinct canary per mapping** so
the assertion is "B not A" / "2 M block, not a 4 K chain". Reuse
`process::Image::Raw` and the `mov x8, #N; svc #0` programs from `aspace.rs` as
the skeleton.

Cover the user-ASpace rows 9, 10, 11, 13, 14 in this image. (The 2 M
kernel-descriptor row 12a and the kernel-table row 17 live in `pagetables.rs`,
since both inspect the kernel table — see C.)

For row 11 (MMIO), do **not write to the PL011** — a write corrupts the kernel
console the test itself prints through, and a DR *read* returns RX data, not
the TX you wrote. The proof is a **non-faulting read**: the program reads the
mapped PL011 LSR and then exits with its sentinel; reaching the sentinel (not
`FAULT_STATUS`) proves the Device mapping works. The second variant maps the
**real** Mailbox page (`0xFE00_B000` @ `0x7000_0000` — see row 11 and
`r9-mailbox-mmio-fix.md`; the old `0xFE00_0000` is a bus hole) and reads
STATUS at page offset `0x898` — deterministic and non-faulting; it lands
after 87's mailbox fix supplies the constants.

Register the image: add `aarch64/tests/vm.rs` **and** a `[[test]]` entry in
`aarch64/Cargo.toml` with `harness = false` and
`required-features = ["qemu-test"]` (a file in `tests/` without an entry is
reported, not run).

### B. Host unit tests — extend `mod tests` in `aarch64/src/vm.rs`

Cover matrix rows 1–8, 12b, 15, 16. These run under `cargo xtask test` on the
host.

- Rows 1–4, 6, 7: direct `assert_eq!` on the raw `u64` / index tuples.
- Row 8: keep the existing `TestVmTrait` (it is the only way to run `map_to`
  off-target), but add a helper that walks the produced table and asserts the
  structure, not just the returned `VirtRange`.
- Row 5: two **passing** tests. (1) A snapshot test that builds the descriptor
  the way `next_mut` does today and asserts the recorded raw bytes (a regression
  guard — it flips if anyone changes the construction). (2) A spec assertion:
  `bits[1:0] == 0b11` (the L0–L2 TABLE type) and hierarchical controls `[63:59]`
  zero. See the corrected note — the current construction is already
  spec-correct; the extra attribute bits sit in IGNORED fields.
- Row 20: the allocator-runs-dry error-path test (task 94's regression
  guard) — assert error returned + entry 511 restored.
- Row 21: the no-W+X walk (with task 96).
- Row 12b: `map_phys_range` with `PageSize::Page1G` via `TestVmTrait`; assert
  the L1 slot is a 1 GiB block descriptor. Note in the test that 1 G has no
  production caller.
- Row 15: `map_phys_range` with a range off a page boundary →
  `PhysRangeIsNotOnPageBoundary` (a pure precondition check, so host, not QEMU).
- Row 16: `map_phys_range` with a zero-length `PhysRange` → `PhysRangeIsZero`.

Do **not** try to run the real `VmTraitImpl` on the host — it reads
`ttbr0_el1()` and KZERO-offset pointers that do not exist off-target. That is
exactly why rows 9–14 live in the QEMU image.

### C. Kernel-table assertions — extend `pagetables.rs`

`vminit.rs` carries a separate `map_to`/`next_mut`/`map_phys_range`/`entry_mut`
that is exercised only by whole boot; the kernel's own table (TTBR1) is the
only place 2 M descriptors appear. Both kernel-table rows live in the existing
`pagetables` image (it already walks kernel section ranges):
- Row 17: a known data symbol resolves to its physical page via a manual
  `kernel_pagetable()` walk, and the kernel root's entry 511 is a valid table
  descriptor (the self-pointer, written by `vminit.rs`).
- Row 12a: a kernel section mapped at 2 M (Kernel Text/RO/Data) actually
  produces a 2 M **block** descriptor at its L1 slot, not a chain of 4 K pages.

Do **not** attempt to unify the two `map_to`/`next_mut` copies in this task —
that is a separate refactor (see Related).

## Files

- `aarch64/tests/vm.rs` — new QEMU image (user-ASpace rows 9, 10, 11, 13, 14).
- `aarch64/Cargo.toml` — `[[test]]` entry for `vm`.
- `aarch64/src/vm.rs` — extend `mod tests` (rows 1–8, 12b, 15, 16, 20, 21).
  Row 5 adds assertions only; it does **not** touch `next_mut` (the current
  construction is spec-correct — see the row-5 note).
- `aarch64/tests/pagetables.rs` — kernel-table rows 17 and 12a (walk + entry 511
  + 2 M block descriptor).

## Definition of done

- Every row in the coverage matrix has a named assertion; the matrix is updated
  in this file to tick each box. Rows 1–10, 11 (PL011 variant), 12a/b, 13–18,
  20 tick as *passing*; row 11's Mailbox variant lands after 87's mailbox fix;
  rows 2 (`rw_user_text` sub-test), 19 (store-to-text half), and 21 land with
  task 96.
- `cargo xtask qemu --arch aarch64 --image vm` passes (the row-11 PL011 variant
  and rows 9, 10, 13, 14, 18, 19).
- `cargo xtask test` passes (new host unit tests, all un-ignored).
- `cargo xtask ci` is green across aarch64, riscv64, x86-64 (the new image is
  aarch64-gated like its siblings; the host unit tests run on the host).
- No new warnings on any arch.

## Related

- **Task 87** (`r9-mailbox-mmio-fix.md`) — the original motivation, since
  rediagnosed as a wrong-PA bus error: no VM assertion here would have caught
  it (the mapping was valid), which is why row 18 (kernel-side walk asserting
  PA + attributes) and task 93 (DFSC decode) were added — together they make
  "mapping wrong" and "PA wrong" distinguishable at a glance. Row 11's mailbox
  variant uses 87's corrected constants. (The real indices: `0x7000_0000`→L1
  1/L2 384 and `0x8000_0000`→L1 2/L2 0 — the two differ at L2, not L1.)
- **Tasks 94/96** — row 20 is 94's regression test; rows 2 (`rw_user_text`),
  19 (store-to-text), and 21 pin 96's corrected encoding.
- Existing tests this extends, not duplicates: `pagetables.rs` (kernel table
  walkable), `aspace.rs` (per-process isolation), `aspace_fault.rs` (fault
  isolation), `map_phys_range_test` (host, fake `VmTrait`).
- Follow-up (out of scope here): unify the `vminit.rs` duplicate with `vm.rs`
  so there is one `map_to`/`next_mut`; the coverage added by this task makes
  that refactor safe to attempt.
