# Plan: architecture review, 2026-08

Audit of the tree at `f76d96a`, cross-checked against QNX Neutrino,
Plan 9, seL4, Zircon and Minix 3.  Opened tasks 102–127.

Two rulings were taken during the review and are recorded here because
several tasks below branch on them:

1. **`SYS_MAP_MMIO` is to be gated / permissioned.**  Physical-memory
   access becomes a capability, not an open syscall.  Task 99 (PA
   validation) is the first half; task 120 (device capabilities) is the
   authorization half it explicitly deferred.  The "device-dumb, the QNX
   model" comment at `aarch64/src/ipc.rs:392` asserts an isolation
   property the code does not provide, and is now wrong rather than
   incomplete.
2. **Multi-core is coming sooner rather than later.**  Every race the
   review found is therefore a live defect, not a deferral.  Tasks that
   were "SMP-latent" are filed at their true severity, and task 124
   (bring the secondaries up) moves ahead of the IPC rework rather than
   behind it.

## What r9x is

A microkernel in the QNX mould wearing Plan 9 clothes.  The kernel
brokers messages and owns as little else as it can; drivers are ordinary
user processes; names, not handles, are meant to be how one process
finds another.  Four kernel objects, all fixed static tables, no
allocation, no reclamation: process (`NPROCS = 8`), channel
(`NCHANNELS = 16`), message (`MSG_MAX = 256`, queue depth 8), IRQ route
(`NIRQS = 16`).  The bind table (`NENT = 8`) lives in the nameserver's
own memory, which is the one piece that is already in the right place.

## The seven structural problems

Ranked by how much each costs per server added.  The first three rewrite
other people's code if deferred.

1. **Reply is not a kernel concept** — task 118.  A channel is a one-way
   queue; reply is a convention carried in the payload, so each server
   invented its own.  The nameserver reads the reply handle from the last
   four bytes, the console from the first four, the mailbox uses none and
   replies on a shared outbound channel two clients would race on.  The
   `tag` field already exists for correlation and now duplicates the job.
   A request/reply costs three channels instead of one, which is why the
   display image's budget is exactly at the 16-channel limit.

2. **Handles are global integers, not capabilities** — task 119.
   `channel()` accepts any index below `NUSED` from any process and
   `Channel::owner` is always 0, so any process can receive on the
   console's inbound channel.  This also blocks the Plan 9 half of the
   project outright: per-process namespaces are impossible while a handle
   is a global integer.

3. **Every server speaks an ad-hoc protocol** — task 122.  `OP_BIND`,
   `OP_WRITE`, `OP_CONFIGURE_FB` and `R_OK` are all 0 — opcode zero means
   four different things depending on the channel, by stated convention.
   That is the property 9P exists to remove.  Every protocol written
   before 9P lands gets rewritten when it does.

4. **`map_mmio` is an unrestricted physical capability** — tasks 99, 120.
   Ruled: gate it.

5. **Fixed 256-byte messages, no bulk path** — task 121.  The console
   cannot atomically write a line over 252 bytes and says so in its own
   docs; 9P `Tread`/`Twrite` will force chunking into every layer.

6. **No server lifecycle, no in-process threads** — tasks 98, 125.  A
   faulted server is gone permanently and its names stay bound to dead
   channels; the panic handler exits 0, so every crash looks clean.  And
   `recv_waiter` is a single slot with a thread being a whole process, so
   every server is a serialized state machine — a constraint that is
   currently an accident reading as a design choice.

7. **Error space overlaps value space** — task 126.  `ERR_NO_SLOT = 8` is
   also a valid channel handle; `RECEIVE_CLOSED = 2` collides with
   `OP_UNBIND` and `R_EFULL`; `SYSEXIT`'s number doubles as its status.
   `RECEIVE_TIMEOUT = 0xffff` is the one that got it right.

## The eighth, now that multi-core is near

Every kernel structure is either a single global under one lock (the
process table, `CURSOR`, `NEED_RESCHED`, `STARTER_CTX`) or a single-slot
field guarded by nothing (`send_waiter`, `recv_waiter`, the IRQ route
cells).  Coherent single-core, incoherent multi-core.  This is the
constraint that shapes tasks 118 and 119: both add a new kernel table,
and both must be born concurrent rather than retrofitted.

## Landing order

- **Wave 0 — ground truth.**  102, 103 (release-build landmines), 106,
  107 (allocator arithmetic), 104, 105 (page-table walk), 108, 109
  (locks).  These are silent corruption and undiagnosable hangs; every
  measurement taken before they land is measuring through them.
- **Wave 1 — make the races visible.**  124.  Deliberately small, and the
  highest value-to-effort change on the list: until a second core runs,
  every concurrency defect here is invisible to the whole test suite,
  which is how a project with an explicit SMP charter accumulated a
  dozen.  Expect it to fail loudly; that is the deliverable.  Task 97's
  loom/miri tests are the host-side companion.
- **Wave 2 — SMP foundation.**  117, 110, 113, plus the per-CPU
  restructure inside 124.
- **Wave 3 — ABI and hardening, parallel with 1–2.**  126, 115, 116, 111,
  112, 114.  None of it collides with the scheduler work; this is the
  clean split if two implementers are available.
- **Wave 4 — the primitives.**  118, then 119.  Both rewrite the IPC core
  and everything after depends on them.
- **Wave 5 — the boundary.**  99 + 120, 121, 98.
- **Wave 6 — protocol and scale.**  122, 123, 78, 125, 127.

## Note on the existing backlog

Several open tasks are the same work seen from a different angle and
should be landed together rather than separately: 99 with 120 (validation
then authorization), 98 with 114 (supervision needs the closed-channel
arms first), 78 with 122 (the 9P client wants the codec the server side
also needs), 97 with 108 (the loom tests are what catch the weak-CAS
bug), 91 with 104/105 (the VM matrix's BBM and SMP rows are these fixes'
acceptance evidence), 9 with 107 (both are `mark_range`/bitmap
arithmetic).

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
