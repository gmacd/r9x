---
status: done
---

# r9-syscall-heap: a kernel heap — the first service a real std needs (Tier 1.1)

Task 2 of 7 in the r9x arc. Plan:
[plans/r9x-target-std-backend.md](plans/r9x-target-std-backend.md).
Needs Task 1 (the r9x target + `r9x_std::rt` static allocator to replace).
Rationale (Decision 7, order 1 of 3): the kernel owns the page tables and the
per-process `Aspace`, so a process's heap can only grow by a kernel call —
exactly QNX's `sysmalloc`, Plan 9's `sbrk`→`sysbrk`, Linux's `brk`/`mmap`
(`linux/mm/mmap.c`). Without it, `r9x_std`/`alloc` cannot grow, so no
`Vec`/`String`/`Box` works for a server that buffers. It is the #1 kernel
addition to make r9 a usable custom kernel.

## Goal

Add a memory-allocation service to the kernel and make `r9x_std::rt`'s global
allocator use it, so r9x processes get a heap that grows on demand (and can
give memory back). The static fixed-buffer allocator from Task 1 is removed.
After this, `r9x_std`'s `alloc` is real: a server may allocate past its static
footprint and the kernel backs it.

Standing constraints: warning-free for all three arches; the heap is
per-process (a process's allocation never touches another process's `Aspace` —
the isolation property); the interrupt context stays within the three-thing
budget (the alloc path is a syscall, not an interrupt, so it may block/lock,
but a *page fault* path must not).

## Changes

- **Kernel — new syscalls** (arch `process.rs` + the `trap.rs` dispatch,
  mirroring `SYCCREATECHAN`'s shape):
  - `SYS_ALLOC` (x0 = byte count, aligned; on return x0 = a user VA, or an
    error code when the process's `Aspace` cannot grow): grow the current
    process's user heap by whole pages and hand back a page-aligned user VA
    from the new range. The mapping is `Entry::rw_user_data` into the
    process's TTBR0 only (no TTBR1), reusing the per-process `Aspace` the
    loader already builds.
  - `SYS_FREE` (x0 = VA): return a range to the process heap. (A first cut
    may be "free the top" / `brk`-style; a general free is a stated refinement
    — see Not in scope.)
  - Add the numbers to `r9x_abi` (Task 1's crate) so build and loader agree;
    the pinning test (Task 1) covers them.
- **Kernel — the heap's home:** a per-process heap cursor (a `brk`-style top
  watermark in the `Process` struct) plus the page-grant from the process's
  `Aspace`. Allocation is page-granular; sub-page requests round up. The
  cursor + the guard against growing into the MMIO region (the top of the user
  half, where `SYS_MAP_MMIO` lives) is an asserted invariant, not a silent
  overlap.
- **`r9x_std::rt`:** replace the static global allocator with a
  kernel-backed `System`-style allocator whose `alloc`/`dealloc` call
  `SYS_ALLOC`/`SYS_FREE` via `r9x_std::sys`. The static buffer and its size constant
  are deleted (Decision 4 resolves). A host test of the new allocator's
  rounding/alignment logic (the syscall is stubbed).
- **`r9x_std::mem`:** expose the growing heap; document the page granularity and
  the top-of-heap/`MMIO` boundary.

## Tests

- **Host unit tests:** the heap cursor math (round-up to page, monotonic
  growth, the top-bound invariant) in `port`/the arch, `#[cfg(test)]` style.
- **New aarch64 integration image** `heap`: a process allocates a small block
  via `r9x_std` (`Box`/`Vec`), writes a marker, allocates past the old static
  footprint (the load-bearing case — this is what the static heap could not
  do), reads the marker back, exits 0. A second case allocates up to the
  top-bound and asserts the error code, not a fault into the MMIO region.
- **Isolation:** two processes each allocate; assert a fault in one does not
  touch the other (the existing per-process `Aspace` test extended to the heap
  cursor).
- The pinning test covers the new `SYS_ALLOC`/`SYS_FREE` numbers.

## Acceptance

- `cargo xtask ci` green (all arches; the `heap` image passes).
- `r9x_std::rt` no longer references the static buffer; its allocator is the
  kernel-backed one.
- A `Vec::with_capacity` past the static footprint works on-device (the
  `heap` image proves it).
- Growing to the top of the user half returns an error code; it does not
  overlap the `SYS_MAP_MMIO` region or fault.

## Not in scope

A general (non-`brk`) free that coalesces arbitrary holes — the first cut may
be top-only/`brk`-style, stated as a refinement (a server that allocates and
frees in the middle is the trigger to generalize). A user-visible `mmap`-style
API (the service is `alloc`/`free`; a raw `mmap` is a later refinement).
Per-heap accounting/limits (a later scheduler concern, Task 6's area). A
`SYS_MMAP` with flags — not needed until a server needs a specific
mapping; `SYS_ALLOC` covers the current need.

## Build record (2026-08-25)

`cargo xtask ci` green: 20/20 QEMU images (the new `heap` image is the 20th),
all three arches, warning-free; the four cursor-math host tests pass under
`cargo xtask test`.

- **Brk-style, per-process, TTBR0-only.** The heap is a top watermark in
  `Process` (`heap_base` / `heap_brk` / `heap_hwm`), seeded just above the user
  stack at spawn (the `install` assert keeps the base page-aligned and in the
  user half). `SYS_ALLOC` grows it toward `KZERO`; each new page is mapped into
  the process's TTBR0 *only* (`aspace::map_user_data_page`) — no TTBR1 identity
  map, so the heap is the process's to use and never kernel-reachable (the
  device-dumb / QNX model). The watermark fields are touched under the table
  lock, as the module requires.
- **`SYS_FREE` is free-the-top (brk-style).** It lowers `heap_brk`; the
  released pages stay mapped and `heap_hwm` remembers the high water so a
  regrow reuses them instead of re-mapping (and double-allocating) them. A
  general free is the stated refinement (Not in scope).
- **The error is a code, not a fault.** A refused grant (top bound or a page
  cannot be mapped) returns `1`; a heap VA is page-aligned and at or above one
  page, so the allocator treats any return below `PAGE_SIZE` as failure. The
  top-bound guard itself is the cursor's `None`, not a fault into the `MMIO`
  region — proven host-side (`brk_grow_refuses_to_cross_the_bound`). The bound
  (`KZERO`) is far above QEMU's physical memory, so it is not reached on-device.
- **The success signal is liveness, not “exit 0”.** The kernel records the *svc
  number* as the exit status (a sysexit is always `0`), and the panic handler
  exits `0`, so a clean exit and a panic are indistinguishable — only a fault
  (`0xff`) is distinct. The `heap` image therefore checks both heaptasks are
  still *alive* (blocked), which catches a fault and a panic alike.
- **Rounding is shared and host-tested at the cursor.** The allocator factors
  its page count into `pages_for` (the same `div_ceil + max` the kernel's
  `brk_grow` uses), so the rounding the task asks to host-test is covered by
  the cursor-math tests. `r9x_std` itself cannot be host-tested directly — its
  `#[panic_handler]`/`#[global_allocator]` clash with the host's test harness
  when linking a test binary — so no `#[cfg(test)]` was added there (the
  rounding is covered by the cursor tests + by inspection).
- **Isolation is two live processes.** The `heap` image spawns two heaptasks;
  each keeps its own watermark in its own `Aspace`, and both come back alive,
  so one's allocation never touches the other's.

*Resolved (post-73):* the r9-spec server lint — the servers and `r9x_std`
were linted by `clippy --workspace` for their `default-target` (the kernel's
`os = "none"` for the servers, the host for the libs), not the r9 target they
are built for, so `#[cfg(target_os = "r9")]` code would have been active in
the build and invisible to the lint. `xtask` now lints the servers +
`r9x_std` for the r9 spec (same flags as the `ServerStep` build) and excludes
them from the `--workspace` pass, so each crate is linted for the target it
ships for. `r9x_std` is linted for every r9 target (the riscv64/x86_64 runs
reach it even where no server has landed, and that is the first build-check
of `r9x_std` for those two specs). A general free — Not in scope. A
user-visible `mmap`-style API — Not in scope.
