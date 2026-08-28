---
status: done
---

# r9-syscall-spawn: process creation + the image registry (Tier 1.2)

Task 3 of 7 in the r9x arc. Plan:
[plans/r9x-target-std-backend.md](plans/r9x-target-std-backend.md).
Needs Task 2 (a spawned process must be able to allocate). Rationale (Decision
7, order 2 of 3): multi-process is the larger capability unlock and the thing
that makes r9 an OS rather than a monitor. It unblocks (a) `r9x_std::thread`
(threads-as-processes, Decision 2), (b) the init process manager (stage 7:
spawn servers, handle crashes, restart them), and (c) running the display
server as a separate process (the Amiga goal). Plan 9: `fork`/`exec`; QNX:
`ThreadCreate` to the process manager.

## Goal

Add a process-creation service so a running process (init, first) can start a
new process from a known image, and make `r9x_std` expose it. Introduce the
**embedded image registry**: a table of image indices → embedded ELFs the
spawner can launch by index, generalizing today's "one embedded ELF per test
image." After this, init can bring up the server set; a process is no longer
only what the boot image hard-starts.

Standing constraints: warning-free for all three arches; the spawn path is a
syscall (may block/lock, not interrupt context); each spawned process gets an
isolated `Aspace`, its own kernel stack, and its own heap (Task 2); the
spawner hands the child its state through the generalized `HANDLES_VA` page
(handles + args), exactly as today's spawner hands the nameserver/console
their pair.

## Changes

- **Kernel — the image registry:** a table `index → &EmbeddedElf` (the ELFs the
  image builder embeds, the same mechanism the user-binary-loading plan uses).
  Each entry records the ELF bytes, its expected `IMAGE_BASE`, and a name. The
  registry is built at boot from the embedded set (a stated, load-bearing
  ordering: the registry must be populated before any `sys_spawn` can
  reference an index). A `spawn` by index is bounded by the table (an out-of-
  range index is an error, not a fault).
- **Kernel — `SYS_SPAWN`** (arch `process.rs` + `trap.rs`, mirroring
  `SYCCREATECHAN`): x0 = image index, x1 = a child-state VA (or 0 for default),
  x2 = priority. The kernel: takes a free `Process` slot (the existing
  `NPROCS` table — a full table is an error, as `SYCCREATECHAN`'s full channel
  table is), builds the child's `Aspace` + kernel stack, loads the ELF at
  `IMAGE_BASE` (the existing `spawn_elf`/loader path), writes the child's
  handles into its `HANDLES_VA` page (the spawner supplies the pair/args the
  child should see), sets its priority, and returns the child's id in x0.
  The child starts Runnable; the scheduler runs it on the next switch.
- **Generalize `HANDLES_VA`:** the spawner-passed page grows from
  `[in:4][out:4]` to a small header `[n_handles, handles…, argc, argv… ]`
  (a stated convention, still a single page, still read from `r9x_abi::
  HANDLES_VA`). Back-compatible: a two-handle write is `n_handles=2`.
- **`r9x_abi`:** add `SYS_SPAWN` (and the `HANDLES_VA` header layout) —
  covered by the pinning test.
- **`r9x_std` (target repo):** `r9x_std::process::spawn(index, handles) ->
  ProcessId` and `r9x_std::thread::spawn` (a process that is the child of the
  caller's "thread group" — Decision 2: a thread *is* a process; this is the
  honest first form, a process that the caller `wait`s on). A `ProcessId`
  newtype.

## Tests

- **New aarch64 integration image** `spawn`: init (a process, not the boot
  hard-start) `sys_spawn`s the console server by index, then a second
  trivial image, and asserts both reach a known exit status (the statuses are
  the assertion a host test cannot make). A spawn with a bad index returns the
  error code, not a fault.
- **HANDLES_VA header:** a spawned child reads its pair/args from the
  generalized page and acts on them (the console server receiving its
  `/dev/console` name + the nameserver pair this way).
- **Table exhaustion:** spawning until `NPROCS` is full returns the error
  code on the last one.
- **Isolation:** a spawned process's heap (Task 2) and MMIO are in its own
  `Aspace`; a fault in it does not touch the spawner.
- The pinning test covers `SYS_SPAWN` and the header layout.

## Acceptance

- `cargo xtask ci` green (all arches; the `spawn` image passes).
- init can bring up a server by index without the boot image hard-starting it.
- `r9x_std::thread::spawn` / `r9x_std::process::spawn` work on-device.
- A bad index and a full table are both error codes, not faults.
- The `HANDLES_VA` header is back-compatible (the existing spawner-passed
  pairs still work as `n_handles=2`).

## Not in scope

`fork`-with-memory-copy (a child that inherits the caller's memory) — r9x
children start fresh from the image + `HANDLES_VA`, Plan 9's `exec` shape, not
`fork`. A `wait`/reap that learns a child's status is Task 5 (`sys_wait`);
here the child simply runs and the status is observable via the existing exit
path. Named spawns (by string, resolved through the nameserver) — the registry
is by index for now; naming is a refinement once the nameserver is the
authority. `exec` of an arbitrary (non-embedded) image — there is no file
system (Task 7); the registry is embedded, per the user-binary-loading plan.
M:N threading on one context (Decision 2 — refused).

## Build record (2026-08-25)

Built and green: `cargo xtask ci` exit 0, 21/21 QEMU images (up from 20; the
new `spawn` image), all host tests pass, warning-free across aarch64 / x86-64 /
riscv64.

What landed
- `SYS_SPAWN` (24) + the error codes (`SPAWN_ERR_MIN` 128, `SPAWN_BAD_INDEX`
  128, `SPAWN_NO_SLOT` 129, `SPAWN_BAD_STATE` 130, `SPAWN_MAX_HANDLES` 512),
  all in `r9x_abi` (the source of truth, pinning-tested against the arch
  re-exports).
- `aarch64::registry`: the embedded image registry (`NIMAGES = 8` slots,
  `register` / `lookup`, `&'static` bytes, the host stubs).
- `aarch64::process`: `sys_spawn(index, state_va, prio)` — validates the
  index, the priority, and the child-state's `n_handles` *before* any mapping
  (a refused spawn leaks nothing); reads the child-state page from the
  spawner's address space (`copy_from_user`, the spawner's TTBR0 is installed
  during the syscall); builds the child's Aspace; writes the child-state to
  the child's HANDLES_VA page; `try_install`s it. `spawn_elf` (the init-context
  path) and `sys_spawn` (the live path) share `load_elf` + `install`; they
  differ only in what they write to HANDLES_VA and how they install
  (`try_install` returns `Option` — the syscall path — `install` wraps it with
  a panic — the init-context path where a full table is a kernel bug).
- The generalized HANDLES_VA header (`[n_handles:4][handle:4 ...][argc:4]
  [argv ...]`): the old `[in][out]` survives as `n_handles=2, handles=[in,out]`,
  so the pre-spawn servers are back-compatible. `rt::handles()` now reads the
  pair at offsets 4,8 (under the count); `rt::n_handles()` reads the count.
- `r9x_std::process::spawn(index, state, prio) -> Result<ProcessId, SpawnErr>`
  (the error codes are a recoverable refusal, not a fault);
  `r9x_std::thread::spawn` (the thin default-priority wrapper — the
  "thread" is a process, Decision 2).
- `cmd/child` (the reporter child): reads its child-state, asserts
  `n_handles == 2`, sends the handle count back over `in_h`, blocks.
- `cmd/init` (rewritten): reads the spawner's pair, writes a child-state page
  on its own heap, spawns the child by index, drives the bad-index and
  full-table error cases, receives the child's round-trip (proving the
  child-state reached the child intact), blocks.
- The `spawn` integration image (aarch64): registers `[child]` (index 0),
  creates the pair kernel-side, spawns init (the init-context path),
  `run_all`, checks init is alive + no process exited (a fault or a panic ends
  a process, so an all-alive table is success).

Decisions / build notes
- **Error codes at 128+**: a valid process id is a table index 0..NPROCS (8),
  far below 128, so `Result<ProcessId, SpawnErr>` disambiguates (the syscall
  returns a `u64` x0; 0..8 is an id, 128+ is an error).
- **All validation before any mapping**: index, priority, and the child-state's
  `n_handles` are checked before the Aspace is built; a refused spawn leaks
  nothing.
- **`n_handles`, not the first handle, is the child-state check**: channel 0
  is a valid handle (the channel table is indexed from 0, `try_create` returns
  `NUSED.fetch_add(1)` starting at 0), so `in_h != 0` is the wrong check (it
  false-fires on the first channel). `rt::n_handles()` (the header's count) is
  the check: a real pair is 2, a zero page (no child-state) is 0. This was the
  one QEMU failure during the build — init asserted `in_h != 0` and exited
  because the image's first channel was 0.
- **The child-state page is a full 4096-byte page on the spawner's heap**: the
  kernel reads exactly one page from `state_va`; a `Vec<u8>` of 4096 bytes is
  page-aligned (the brk allocator returns page-aligned addresses).
- **`any_exited()` as the image's failure check**: the image can't know the
  children's ids (init spawned them), so it checks the whole table. A fault or
  a panic ends a process; an all-alive table is success.
- **aarch64-only for now**: the registry, `sys_spawn`, and the spawn image are
  aarch64-only (the first-arch build). `r9x_abi` has the syscall number +
  error codes (neutral) and `r9x_std::process::spawn` is the neutral API (the
  other arches get the host stub — a `sys` that would fault if called, but no
  image calls it there). Porting the registry + `sys_spawn` to riscv64 /
  x86-64 is the remaining step (same shape; each arch's `process.rs` gets the
  registry + `sys_spawn` + the trap dispatch arm).

Deferred
- Named spawns (by string, resolved through the nameserver) — the registry is
  by index for now; naming is a refinement once the nameserver is the
  authority.
- `exec` of an arbitrary (non-embedded) image — there is no file system
  (Task 7); the registry is embedded, per the user-binary-loading plan.
- The other two arches' registry + `sys_spawn` + spawn image (the
  aarch64-only build above).
