# Stage 3: per-process `Aspace` — isolated address spaces

## Problem and constraints

Stage 2 gave the kernel its IPC primitive and proved the priority-inheritance
guarantee is deterministic. The microkernel's next load-bearing property is
**isolation**: a fault in one server must fault only that server's address
space, not the kernel or its peers. Today the property is false by
construction — every process shares the single `USER_PAGETABLE` static
(`vm.rs`), so a wild write from one process corrupts the page tables every
other process (and the kernel's user-visible mappings) walks. This stage makes
the address space a **first-class per-process object** (`Aspace`) and gives the
scheduler the one new duty that comes with it: switch the process's TTBR0 in
with its context.

Standing constraints: warning-free across aarch64 / x86-64 / riscv64
(`cargo xtask ci`); minimal scoped change (this is a *turn* of the existing
`vm`/`process` code, not a rewrite); Plan 9 shape; aarch64 is the reference
implementation (the other two arches keep gate-green — they do not yet have a
user-process baseline, so `Aspace` lands on aarch64 only this arc). The
determinism story (stages 1–2) is unchanged: `Aspace` adds no scheduling
policy, only the TTBR0 write on the switch path.

## Prior art

**r9 already has the page-table machinery.** `vm.rs` is a recursive page-table
walk: the root table's last entry (index 511) points at the root table itself,
so a walk can address every intermediate table through a *recursive VA*
(`recursive_table_addr`) regardless of which TTBR the table is installed in.
`map_phys_range`/`map_to`/`next_mut` build or extend the tree; `VmTrait` is
the seam that makes the walk testable (the host tests resolve entries through
the physical address directly). `pagealloc::allocate_virtpage` allocates a
physical page and maps it into a given `RootPageTable`. `pre_mmu/vminit.rs`
builds the *kernel* table (TTBR1) and an identity-mapped boot table (TTBR0)
before the MMU is on; after boot the kernel user table (`USER_PAGETABLE`) is
installed in TTBR0 and `spawn` maps each process's text/stack into it. What is
missing is exactly the per-process-ness: the table is one static, not one per
process, and there is no fault path.

**QNX** (`microkernel-and-firmware` corpus): a per-process address space is a
first-class kernel object (a page-table root + the process's mapping), switched
in with the process's context on every `swtch`. A fault in a process's AS is
delivered to that process (or kills it); the kernel's AS is separate and a
user fault can never corrupt it. The kernel does *not* demand-page by default
— a fault that cannot be fixed is a process death. That is exactly the shape
this stage takes: isolation first, demand paging later (and later, it is a
policy over the `Aspace` fault hook, not a new primitive).

**Plan 9** (`/Volumes/Code/repos/plan9`, 9port design): 9port runs *all*
processes in a **shared** kernel address space — a server's fault is the
kernel's fault. This is the anti-example: the design doc explicitly *refuses*
"a shared address space for all processes" (Not building), because isolation is
the point. What Plan 9 gives us instead is the *server* model: each server is
a process with its own `Aspace`, and a server's death is a clean event the
nameserver notices (stage 6), not a kernel panic.

**Linux** (witness only): `mm_struct` is the per-process AS (a page-table root
+ VMA list + the `pgd` pointer); `switch_mm` installs it on context switch;
the fault handler (`do_page_fault`) walks the *current* `mm_struct`'s tables
and either fixes the fault (demand page, COW) or kills the process
(`send_signal(SIGSEGV)`). The essential parts: per-process `mm_struct`,
`switch_mm` on the hot path, a fault handler that resolves against the current
AS. The accreted parts (VMAs, COW, mprotect, overcommit) are refused — r9's
`Aspace` is a page-table root plus the kernel-mapping copy, not a VMA
allocator.

## Hardware assumptions (required)

Target: aarch64 on QEMU `raspi4b` (the integration path) and Pi 4 hardware.
x86-64/riscv64: **nothing new this arc** — they have no user-process baseline,
so `Aspace` is aarch64-only and the other arches' gates keep compiling exactly
what they do today.

- **TTBR0 / TTBR1 split (Arm ARM DDI 0487, "Translation regime" / TCR_EL1.T0SZ):**
  TTBR1 (the kernel table) covers `[KZERO, 2^48)`; TTBR0 (the user table)
  covers `[0, KZERO)`. When EL0 runs, *both* tables are live: an EL0 access
  to a VA in `[0, KZERO)` walks TTBR0, an access to `[KZERO, 2^48)` walks
  TTBR1. This is why the process's TTBR0 only needs the *user* half — the
  kernel half is always TTBR1 and is never switched. **This is the load-bearing
  fact of the whole design**: switching TTBR0 per process leaves every kernel
  mapping (code, the interrupt stack, the console, the page tables themselves)
  reachable through the *unchanged* TTBR1, so a context switch is one register
  write, not a remap.
- **The recursive walk works on a non-live table.** `vm.rs`'s recursive
  mechanism addresses intermediate tables through the root table's index-511
  self-pointer, reached via a recursive VA computed from the table's own
  physical address. `map_to` temporarily swaps the recursive entry to the
  table being modified, so the walk can build a table that is *not* the current
  TTBR. This is what makes per-process tables buildable: `Aspace::new` can
  build a process's table while a *different* process's table is live in
  TTBR0.
- **TLBI on switch.** Switching TTBR0 requires a `TLBI vmalle1is` + `DSB
  ISH` + `ISB` so the new translations take effect before the process's next
  EL0 access (the same sequence `vm::switch` already does; Arm ARM DDI 0487,
  "TLB maintenance instructions"). A per-process TLBI (`tlbi vae1is`) would be
  cheaper but needs the VA and is a later optimisation; the all-EL1 invalidation
  is correct and matches the existing code.
- **Fault delivery.** A data abort or instruction abort taken while EL0 runs
  lands in the `SyncInvalidEL0_64` vector with `ESR_EL1.EC` = 0x24/0x25
  (Arm ARM DDI 0487, ESR_EL1 exception codes) and `FAR_EL1` = the faulting VA.
  The handler is in EL1 with TTBR1 live, so it can walk the *faulting process's*
  table (through the recursive mechanism) to decide whether the fault is
  fixable or a kill.
- **Memory ordering**: the TTBR0 write is followed by `DSB ISH` before `ISB`
  (Arm ARM DDI 0487, "Memory barriers"); the fault handler's read of
  `FAR_EL1`/`ESR_EL1` is a plain read (they are written by hardware on the
  exception entry, no barrier needed). No new ordering beyond what `vm::switch`
  and the vector preamble already establish.
- **Firmware co-tenancy**: unchanged — the process's TTBR0 maps only its text,
  its stack, and the kernel image's *user-visible* mappings (the console
  device mapping, the mailbox). The kernel's private mappings (the page
  tables, the interrupt stack) are in TTBR1 and are not in the process's TTBR0.
  A process cannot reach the kernel's private memory through its TTBR0.

## Design

### Data structures

The central structure is the **`Aspace`**: one per process, a page-table root
plus the physical address of that root. Everything else is a value that flows
through the existing machinery.

```text
// aarch64::aspace  (new module)

struct Aspace {
    root: *mut RootPageTable,   // the process's TTBR0 root, in a pagealloc page
    root_pa: PhysAddr,          // the physical address (what TTBR0 holds)
}
```

- **Why a raw pointer, not a `&'static mut`.** The root table lives in a
  pagealloc'd page (it cannot be a `static` — there is one per process and the
  count is bounded by `NPROCS`, not by the type system). The pointer is valid
  for the process's life (the page is not freed this arc, matching the
  established "pages are not freed" note in `process.rs`). A raw pointer
  carries that lifetime honestly; a `&'static mut` would be a lie.
- **The kernel mappings are copied in, not shared.** `Aspace::new` builds an
  empty root and copies the *user-visible* kernel mappings from the current
  `USER_PAGETABLE` (the console device page, the mailbox page, the kernel
  text/data the user half sees) into the new table. The copy is a shallow
  copy of the table entries (the leaf pages are shared — they are read-only or
  device mappings, not per-process scratch). The process's own text/stack
  pages are mapped into its *own* table by `spawn` (as today, but into the
  process's `Aspace` rather than the shared `USER_PAGETABLE`). **The copy is
  what makes the isolation real**: a fault in one process walks its *own*
  tables, and a wild write to a process's text/stack page corrupts only that
  page, not the shared table.
- **No VMA list, no map of pinned ranges (yet).** The design doc's "a map of
  pinned/mapped ranges" is the *end state*; this arc builds only the page-table
  root plus the kernel-mapping copy. A VMA list is a later concern (it is what
  a fault handler needs to *demand-page* or to *tear down* a range, neither of
  which this arc does). Keeping the `Aspace` to two fields now is the Oberon-lean
  discipline: the structure is as small as the isolation property needs.

The `Process` struct gains one field:

```text
aspace: *const Aspace   // the process's address space (set at spawn)
```

The `Aspace` is owned by the process (created at `spawn`, lives for the
process's life). The kernel owns the pagealloc page backing the root table.
No fact is stored twice: the root table's physical address is in the `Aspace`
and in the page table's own index-511 entry (the recursive self-pointer); the
`Aspace` is the *authoritative* copy, the table entry is the walk's
convenience.

### Interfaces

One new module, `aarch64::aspace`, with three verbs (day-one user:
`process::spawn`; the fault handler is the second user, same arc):

```rust
// aarch64/src/aspace.rs
/// Create an address space: allocate the root table, copy the
/// user-visible kernel mappings from the current user table.  Panics on
/// allocation failure (init-only context, as spawn's page allocs do).
pub fn new() -> Aspace;

/// The physical address to install in TTBR0 for this AS.
pub fn ttbr0(&self) -> PhysAddr;

/// Map a physical page into this AS at `va` (the process's own text/stack).
/// Reuses pagealloc::allocate_virtpage against this AS's root.
pub fn map_user_page(&self, entry: Entry, va: usize) -> Result<*mut u8, PageAllocError>;
```

The fault hook is a method on the *process* side (it needs the `Aspace` and
the decision to kill):

```rust
// aarch64::process
/// Handle a synchronous fault from `id`'s EL0.  Inspects the fault against
/// `id`'s Aspace; if it is not fixable (this arc: nothing is), kills the
/// process with a fault status.  Returns true if the process should be
/// rescheduled (it died), false if the fault was benign (unused this arc).
fn fault(far: u64, esr: EsrEl1, id: ProcessId) -> bool;
```

The `switch` path gains one line: `switch_out` installs the next process's
TTBR0 before the `swtch` (and the current process's TTBR0 is implicit — it is
already live). The write is guarded by the same `DSB/ISB` `vm::switch` uses.

### Init and bringup order

Load-bearing orderings, stated:

1. **The kernel table (TTBR1) and the shared user table (`USER_PAGETABLE`)
   are set up at boot, before any process exists** — unchanged. The
   `USER_PAGETABLE` is the *template* `Aspace::new` copies the kernel
   mappings from; it stays live as TTBR0 while the kernel runs (no process),
   so a process's first TTBR0 switch is from the shared table to its own.
2. **`Aspace::new` copies the kernel mappings from the *current*
   `USER_PAGETABLE`** — so `spawn` must call it after the user table is live
   (a precondition `spawn` already has, checked by the existing "TTBR0 is the
   user table" invariant).
3. **The TTBR0 write on switch is after the TPIDR write and before the
   `swtch`** — the process's table must be live before its first EL0
   instruction, and the TPIDR (the current-process pointer) must be set before
   the fault handler can find the process if the very first instruction
   faults. The order is TPIDR → TTBR0 → `swtch`.
4. **The fault handler reads the current process from TPIDR** (as `exit_current`
   does) — so a fault with a null TPIDR is a kernel fault (it happened while
   the kernel ran, in TTBR1), not a process fault, and is reported loudly and
   spins (the existing unknown-exception path).

### Failure policy

- **A fault that cannot be fixed kills the process.** This arc has no
  demand-paging, so *every* EL0 data/instruction abort is a kill: the handler
  prints the FAR/ESR and the process id, marks the process `Exited` with a
  fault status (distinct from a clean exit status), and reschedules. The
  isolation guarantee is the point: the kill is *of the process*, and the
  kernel and its peers are untouched.
- **A fault with no current process** (null TPIDR) is a kernel fault: print
  and spin (loud, debuggable), as the existing unknown-exception path does.
  A kernel fault is not this arc's problem to recover from.
- **Allocation failure in `Aspace::new`**: `panic!` (init-only context, as
  `spawn`'s page allocs already do). No panic is reachable from a live
  process or from trap context: the fault handler only prints and marks the
  process dead.

## Not building

- **Demand paging.** A fault that *fixes* itself (a missing page that gets
  allocated on the fault) is the next arc, and it is a policy over the `Aspace`
  fault hook, not a new primitive. This arc's fault hook returns "kill" for
  everything; the fix path is a later change to that one function.
- **A VMA list / a map of pinned ranges.** The `Aspace` is a page-table root
  plus the kernel-mapping copy. A VMA list is what teardown and demand-paging
  need; neither is this arc. The two-field `Aspace` is as small as isolation
  needs.
- **Per-process TLBI (`tlbi vae1is`).** The all-EL1 invalidation is correct
  and matches the existing `vm::switch`; the per-process form is a later
  optimisation with a measurement to justify it.
- **Freeing the `Aspace`'s root page on exit.** Pages are not freed this arc
  (the established `process.rs` note); the root page is leaked with the
  process. A free API in `pagealloc`/`vm` is a prerequisite for teardown,
  which is not this arc.
- **x86-64/riscv64 `Aspace`.** Out of scope by the arch baseline (they have no
  user-process entry); the design keeps every change inside the `aarch64`
  crate so the gates stay green without cross-arch surface.
- **A shared `Aspace` for the kernel's own EL0-visible mappings.** The kernel
  runs in EL1 with TTBR1; it has no `Aspace` of its own. The `USER_PAGETABLE`
  is the template and the no-process TTBR0, not an `Aspace`.

## Decision records

- **Decision: the `Aspace` is a page-table root plus a shallow copy of the
  user-visible kernel mappings, not a VMA list.**
  - Alternatives: a VMA list (Linux's `mm_struct` shape) — rejected: it is
    what demand-paging and teardown need, and neither is this arc; a two-field
    struct is the Oberon-lean shape the isolation property actually needs.
  - Dissent: the kernel-taste lens notes the VMA list will be
    needed the moment a fault *fixes* a page or a range is torn down; we
    accept the refactor then rather than carrying an empty list now (the same
    "second user earns the abstraction" call as the `first-user-process`
    `Process`-struct refusal).
- **Decision: the process's TTBR0 is empty except its own text/stack; the
  kernel stays in TTBR1, unreachable from EL0.**
  - Alternatives: copy the kernel's user-visible mappings into each `Aspace` —
    rejected on inspection: there are none. The kernel's mappings are `Priv*`
    (TTBR1), unreachable from EL0, and the current `USER_PAGETABLE` holds only
    process text/stack. A process reaches the kernel by syscall; a mapped
    device page is stage 5's `map_mmio` (the console server's capability), not
    part of isolation. An empty per-process root is the minimal table the
    isolation property needs, and it is what makes a wild write corrupt only
    the faulting process's page.
  - Dissent: the hardware lens notes a process that cannot reach
    any device mapping cannot be a device *server* (stage 5's console server
    must poll the UART); agreed — that is exactly why `map_mmio` is the
    stage-5 verb on the `Aspace`, and it is refused here (building it now is
    the "filing stage 5's detail before stage 3" mistake). The empty root is
    the isolation; the device capability is a later, separate verb.
- **Decision: the fault handler kills on every EL0 fault; there is no fix
  path this arc.**
  - Alternatives: a no-op fault handler (print and return) — rejected: a
    return to the faulting instruction is a livelock (the process re-faults
    immediately). A demand-paging handler — rejected: it is the next arc, and
    building it now is the "filing stage 7's detail before stage 2" mistake
    the plan avoids. Kill is the total function: it is the isolation
    guarantee, and the fix path is a later change to the one function.
  - Dissent: the microkernel lens wants the fault *delivered* to the
    process (a signal) so a server can catch its own faults; we kill instead
    because r9 has no signal machinery and a server that catches its own
    faults is a later arc. The kill is the Plan 9 shape: a server's death is
    a clean event (stage 6's nameserver notices), not a signal.
- **Decision: the TTBR0 switch is in `switch_out`, after TPIDR and before
  `swtch`, using the all-EL1 TLBI.**
  - Alternatives: switch TTBR0 in the vector tail (the EL0 return path) —
    rejected: the tail is the single EL0 return path and adding a per-process
    TTBR0 write there couples the return path to the AS; the switch belongs in
    `switch_out`, where the context is already being moved. A per-process TLBI
    — rejected: it is a later optimisation with a measurement to justify it;
    the all-EL1 form is correct and matches the existing `vm::switch`.
  - Dissent: the kernel-taste lens notes the all-EL1 TLBI on every
    context switch is a hot-path cost (it flushes the whole TLB, including the
    neighbour's entries); we accept it because it is correct and matches the
    existing code, and the per-process form is filed as the optimisation the
    measurement will decide. The hot-path cost is real but bounded (a TLBI +
    DSB is a few cycles on QEMU; on the Pi 4 it is measurable but not the
    scheduler's bottleneck at this process count).

## Tasks

Filed (actionable, sequenced):

1. `tasks/aspace-struct.md` — the `aarch64::aspace` module: `Aspace` (two
   fields), `new` (allocate an empty root), `ttbr0`, `map_user_page`.
   The `Process.aspace` field. No scheduler or trap change yet: this is the
   structure and the build path. First, because the switch and the fault
   path stand on the `Aspace` existing.
2. `tasks/aspace-switch.md` — `switch_out` installs the next process's
   TTBR0 (TPIDR → TTBR0 → `swtch`); `spawn` builds the process's `Aspace` and
   maps its text/stack into it (not the shared `USER_PAGETABLE`). The
   integration image: two processes with the same text/stack VAs but
   *different* physical pages, both running, asserting they see their own
   data (the isolation proof at the scheduling level).
3. `tasks/aspace-fault.md` — the EL0 fault path in `trap.rs`: a
   data/instruction abort with a current process is `process::fault`, which
   kills the process with a fault status; a fault with no current process
   spins. The integration image: a process that writes to an unmapped VA
   dies with a fault status; its peer (and the kernel) survive. The
   isolation proof at the fault level.

Sequenced: 1 → 2 → 3. Stage 4 (IRQ→message) is independent of 3 and can
interleave with 2 (it needs only stage 2).
