---
status: done
---

# aspace-struct — the per-process address-space object

## Problem

Every process shares the single `USER_PAGETABLE` static (`vm.rs`), so a wild
write from one process corrupts the page tables every other process (and the
kernel's user-visible half) walks. Stage 3 of the microkernel substrate
(`tasks/plans/microkernel-aspace.md`) makes the address space a first-class
per-process object so that a fault in one process faults only that process's
address space.

This task builds the object and the build path; the TTBR0 switch and the fault
path are the follow-on tasks (`aspace-switch`, `aspace-fault`).

## Evidence

- `vm.rs`: `static mut USER_PAGETABLE: RootPageTable` is the one shared user
  table; `spawn` (`process.rs`) maps every process's text/stack into it.
- `vm.rs`: the recursive page-table walk (`map_to`/`next_mut`/`VmTrait`)
  already builds a table that is *not* the current TTBR by temporarily swapping
  the index-511 recursive entry — the mechanism `Aspace::new` needs to build a
  per-process root.
- `process.rs`: `Process` has `state`, `context`, `kstack`, `exit_status`,
  `prio` — no address-space field.

## Fix direction

New module `aarch64/src/aspace.rs` (gated `#[cfg(target_os = "none")]`, host
stub for the dispatch to compile):

- `struct Aspace { root: *mut RootPageTable, root_pa: PhysAddr }` — the
  process's TTBR0 root and its physical address. The root lives in a
  pagealloc'd page (one per process, bounded by `NPROCS`; not a `static`).
  The raw pointer carries the "valid for the process's life, page not freed
  this arc" lifetime honestly (the established `process.rs` note).
- `pub fn new() -> Aspace` — allocate an empty root page (cleared, index-511
  self-pointer set, as `init_empty_root_page_table` does), record its physical
  address. Panics on allocation failure (init-only context, as `spawn`'s page
  allocs do).
- `pub fn ttbr0(&self) -> PhysAddr` — the physical address to install in TTBR0.
- `pub fn map_user_page(&self, entry: Entry, va: usize) -> Result<*mut u8, PageAllocError>`
  — map a physical page into this AS at `va`, reusing
  `pagealloc::allocate_virtpage` against this AS's root (the same call
  `spawn` makes today, but against the process's own table).

`process.rs`: `Process` gains `aspace: *const Aspace` (set at `spawn`).

The `Aspace` is empty except the process's own text/stack (mapped by
`spawn` via `map_user_page`); the kernel stays in TTBR1, unreachable from EL0.
There is no copy (there is no user-visible kernel mapping to copy — the
kernel's entries are `Priv*`, and a process reaches the kernel by syscall).

## Done-when

- `aarch64::aspace` exists with the four items above; the `Process.aspace`
  field is set at `spawn` (the existing `spawn` still maps into the shared
  table this task — the switch to the per-process table is `aspace-switch`).
- `cargo xtask ci` is green (clippy ×3, dist ×3, host tests, integration
  images). The aarch64 host test build sees the stub; the target build compiles
  the real module.
- No behaviour change yet: the existing `user_process`/`ipc`/`prio` images pass
  unchanged (the `Aspace` is built but not switched in).

## Origin

Stage 3 of `tasks/plans/microkernel-substrate.md`, decomposed in
`tasks/plans/microkernel-aspace.md`. First of the three sequenced tasks
(struct → switch → fault).
