# Microkernel substrate: QNX mechanism under a Plan 9 file namespace

## Problem and constraints

r9 is a monolith-in-formation. It has a real scheduler (round-robin, preemption),
a 9nen-style context switch, and EL0 user-process entry — but no IPC, no
per-process address space, no 9P, no namespace, and no user-space servers.
Every subsystem that in Plan 9 is a *file server* (filesystem, network, device)
would land **in the kernel**, which is where the monolith grows and where the
TCB stops being auditable by one person.

This plan defines the **end state**: a small kernel that does message passing,
scheduling, address spaces, and nothing else, with Plan 9's file-server
namespace built on top in user space. It then gives a **staged path** from the
current tree to that state.

Standing constraints: warning-free across aarch64 / x86-64 / riscv64 (`cargo
xtask ci`); minimal scoped changes (this is a *turn*, not a rewrite — it builds
on the done preemption arc); Plan 9 shape; and the standing correctness
prerequisite that **multi-core must be right first** (tier-1 tasks), because the
end state's determinism story (priority inheritance) is meaningless on a
scheduler that races itself.

## Prior art

**r9 already has the hardest 20%.** `aarch64::process` is the scheduler: a proc
table with a round-robin cursor, tick preemption (`irq_resched`), per-process
64 KiB kstacks, `TPIDR_EL1` as the current-process pointer, and the MCS lock
discipline documented in its header. `swtch.S` is the context switch. The
`first-user-process` arc proved EL0 entry, a live user page table, and `svc`
dispatch with an exit path. What is missing is exactly the substrate this plan
adds: IPC, per-process address spaces, IRQ-to-message routing, 9P, and the
namespace.

**Plan 9** (`/Volumes/Code/repos/plan9`, partial checkout — `sys/include/9p.h`
and `src/cmd`; from the published 9port design). `9p.h` defines the in-kernel
9P server's data structures — `Fid`, `Req`, `Fidpool`, `Reqpool`, `File`,
`Tree` — which is direct evidence that Plan 9's file server is a self-contained
unit with its own pools, not a kernel monolith. The 9P wire protocol itself is a
self-describing message protocol: every request is `size | type | tag | payload`
(length-prefixed, tagged for request/reply correlation, network-runnable). That
is the key fact this plan leans on: **9P is already a message protocol**, so it
rides on a message-passing kernel without an adapter. What Plan 9 *as shipped*
does not give us: per-process address spaces (9port runs all processes in a
shared kernel AS), and any determinism guarantee. Essential parts we reuse: the
9P protocol, the Fid/Req server model, the namespace/bind tree. Accreted /
refused: a shared address space, an in-kernel 9P server.

**Linux** (witness only). `mm_struct` (per-process address space), the
SysV/futex IPC paths, and — decisively — the **futex wake with priority
inversion** machinery (`rt_mutex` PI, `SCHED_FIFO`) confirm that (a) a small
in-kernel message/queue primitive plus a priority-inheriting scheduler is the
established shape for a determinism claim, and (b) per-process AS is a
first-class kernel object, not an afterthought. Everything else (cred, cgroup,
the VFS layering) is accretion.

## Hardware assumptions (required)

The substrate is **arch-agnostic in `port`**; each arch contributes exactly two
things: (1) "block this thread and wake it" (reuses the existing scheduler
states + `resched`, so it is *not* new per-arch code in any meaningful sense),
and (2) "deliver a hardware interrupt to a named process" (the IRQ→message
routing, which genuinely differs per controller).

- **aarch64 (Pi 4 / QEMU `raspi4b`)** — GICv2 routes IRQs; a single
  per-core generic timer; the early console is the VideoCore miniuart/pl011
  MMIO. IRQ→message: the GIC's IRQ handler (today `gic.rs` dispatches in-kernel
  to handler code) instead looks up the owning `ProcId` for that SPI/PPI and
  enqueues a pre-allocated per-IRQ message. The Pi's firmware owns the UART and
  mailbox — the **early console stays in-kernel** (see Not building); after
  bringup the *console server* owns the MMIO and the kernel's raw path is retired.
  Coherency: a single coherent domain on these targets; the message handoff
  needs no cache maintenance, only the release/acquire implied by the channel
  queue lock.
- **x86-64 (QEMU `q35`)** — APIC (local + I/O) for IRQs, per-core TSC/LAPIC
  timer. The x86 crate already has `proc.rs` and `syscall.rs`, so its EL0
  entry is at or near the aarch64 baseline; the substrate grafts onto it the
  same way. IRQ→message: the APIC EOI path carries the vector to the owning
  `ProcId`.
- **riscv64 (QEMU `virt`, `nezha`)** — SBI for firmware calls, PLIC/CLINT for
  IRQs/timer, already split into `platform/{virt,nezha}`. IRQ→message routes the
  PLIC claim to the owning `ProcId`. This arch is the furthest behind on
  user-process entry; the substrate is defined for it but **lands last** (the
  aarch64 arc is the reference implementation).
- **Firmware co-tenancy (all)**: firmware owns early-boot peripherals. The only
  permanent kernel-resident device is the early console; every other peripheral
  is owned by a user-space server. No design here assumes the kernel is alone
  on the machine during bringup.
- **Memory ordering (all)**: the message buffer handoff is protected by the
  channel queue lock (a `SeqCst`/acquire-release pair is sufficient; the fast
  path — direct handoff to a blocked receiver — reuses the scheduler's existing
  wake barriers). No new ordering requirement beyond what `swtch` and the MCS
  lock already establish. *Stated, not silent.*
- **Interrupt context budget**: an IRQ handler may do exactly three things —
  look up the owning process, enqueue a *pre-allocated* per-IRQ message, wake a
  waiter. No allocation, no unbounded work, no lock held across a switch. This
  is the determinism contract made concrete (see Failure policy).

## Design

### Data structures

The central structure is the **Channel**; everything else is a value that flows
through it. The message is a *bounded, typed, tagged* value — the "strict
interface" the design is named for.

```text
// port::ipc  (arch-agnostic)

struct Channel {
    queue:    BoundedQueue<Message>,  // fixed capacity, pre-allocated slots; no alloc on the hot path
    waiters:  WaitList,              // processes blocked in receive(); reuses the scheduler's block/resume
    owner:    ProcId,                // the creator (a server); a channel dies with its owner
}

struct Message {
    opcode: u16,                     // server-defined — the strict, enumerable API
    tag:    u32,                     // request/reply correlation (9P rides its tag here)
    buf:    [u8; MSG_MAX],           // bounded payload; MSG_MAX is a const (fast path ~ 256 B)
    len:    u16,
}
```

- **Why the special cases disappear.** A send has two shapes and nothing else:
  *fast* (a receiver is blocked → hand the buffer to it directly, wake it) and
  *slow* (no receiver blocked → copy into a queue slot, wake if it was empty).
  There is no "shared memory" case, no "callback" case, no "file descriptor"
  case in the kernel. Ownership is always the buffer: the sender moves it; the
  receiver owns it after dequeue. One owner at a time, always — the
  "share memory by communicating" proverb, enforced by the type system
  (`Message` is `Send` and moves across the boundary; it is never aliased).
- **Priority inheritance lives on the channel, not the scheduler — QNX's
  server-at-client rule.** A *server* blocked in `receive` runs at the priority
  of the *client* that sends to it: when a send wakes a blocked receiver, the
  receiver is boosted to the sender's priority for the duration of the exchange
  and unboosted when it next blocks. A high-priority client's request is
  therefore never made to wait behind lower-priority work in its own server. This
  is exactly QNX's mechanism: its kernel threads start at the *lowest* priority
  (255) and, once blocked in `MsgReceive`, operate at the priority of whoever
  sends to them. It is ~a field on `Channel` plus the scheduler's priority
  compare and the existing `boost`/`unboost`.
- **Priority is a QNX-shaped range, not a level pair.** A process's priority is
  an index into 256 levels (0 most urgent, 255 the idle thread's slot — which a
  runnable process never occupies, so it is kept out of the live 0–254 range),
  mirroring QNX's numbering from the start. The kernel uses only a few of the
  levels, but the *range* is what makes the PI guarantee load-bearing: with a
  bare `User`/`Kernel` pair a `Kernel` server outranks everything and the boost
  has nothing to do.
- **Per-process address space** is a first-class object (`Aspace`): a page-table
  root + a map of pinned/mapped ranges + the fault policy. Generalizing
  `first-user-process` (whose process *shared* the kernel user table) into a
  real per-process AS is what makes a server *isolated*: a fault in one server
  faults only that `Aspace`.

Who owns what: the **kernel** owns channels (while live), message queue slots,
`Aspace` structures, and the per-IRQ message pool. A **server** owns its
channels' *content* (the messages) and its `Aspace`'s mapping. A **client** owns
nothing in the kernel — it owns its `Aspace` and the replies it receives. No
fact is stored twice: a message is in exactly one place (a queue slot or a
receiver's hands); a process's register state is in its `Context` on its kstack
as today.

### Interfaces

Three layers, each a strict interface (this is the graft of the QNX *and* Plan
9 candidates — see Decision records):

1. **Kernel IPC** (`port::ipc`) — the mechanism. `send(ch, &Message)`,
   `receive(ch) -> Result<Message, IpcErr>`, `reply(ch, tag, &Message)`, plus
   `Channel::create()`. Day-one users: the console server (stage 5) and the
   two test images in stage 2. The kernel message is **opaque and bounded** — it
   carries no protocol; 9P is a payload, not a kernel type.
2. **Server API** — the policy. The *default* server interface is the **9P
   file protocol** over a channel (open/read/write/ctl/walk/stat — file-server-
   shaped, as the Plan 9 network papers demand). A narrow **native opcode API** is permitted for
   the few servers where the file model is a poor fit (the raw console); even
   those are addressable through the namespace, so the metaphor does not fork.
3. **Namespace** (`nameserver`) — the policy made navigable. A user-space
   process owns the bind tree; it maps *names* (`/dev/console`, `/mnt/...`) to
   *channels*. A client resolves a name to a channel and then speaks 9P (or the
   native opcode) over it. This is where Plan 9's "everything is a file"
   returns: the client never sees a channel ID, only a path.

The public surface of `port::ipc` is its specification: `Channel`, `Message`,
the three verbs, and `IpcErr`. Nothing else. `MSG_MAX` and the queue capacity
are `const`s, not knobs.

### Init and bringup order

The kernel comes up exactly as it does today (early console, MMU, pagealloc,
FDT, scheduler, IRQs) and then stops doing *things* and starts doing
*delivering*. Load-bearing orderings, stated:

1. **Priority+PI scheduler before IPC** (stage 1 before stage 2). A channel's
   PI field is meaningless on a round-robin scheduler; IPC built on round-robin
   is a monolith with RPC and no latency guarantee. (Decision record.)
2. **Per-IRQ message pool allocated at boot, before any IRQ→message routing is
   armed** (no allocation in IRQ context, ever).
3. **The first user process is a server, not a shell.** Kernel brings up
   `init` (a process manager) → `init` starts the **nameserver** → the
   nameserver starts/registers the **file servers** (console, then dev/mnt/net)
   → the namespace composes → a shell runs. The kernel is *done* at the moment
   `init` is running; everything after is user space. This inverts the monolith
   order (where the kernel starts all its subsystems) — the kernel's only job
   is to start one process and then deliver messages.
4. **Early console is the only device the kernel touches after this point**, and
   only until the console server is up, at which point the kernel's raw path is
   retired. Stated so it is not silently kept forever (Not building).

### Failure policy

- **A server crash kills only its `Aspace`.** The kernel faults the `Aspace`,
  marks the `ProcId` dead, and the nameserver marks its name down. Clients get a
  clean `IpcErr::Eio`/`ENode` and may retry. The kernel never crashes from a
  server fault — this is the payoff for the whole design and the reason
  `panic = "abort"` (already set) is safe in user space.
- **IPC errors are `Result`s, not panics**, from server context: `IpcErr`
  covers channel-closed, queue-full (sender blocked, then… see below), and
  bad-tag. A server is ordinary safe Rust.
- **Queue-full policy**: a `send` to a full channel *blocks the sender* (it
  joins the queue's back). This is a *decision*, not a knob — dropping a message
  silently would hide a stuck server; blocking surfaces it. (If a specific
  server needs drop-on-full later, that is that server's policy over the
  primitive, not a new primitive.)
- **Interrupt context**: the IRQ handler cannot panic, allocate, or hold a lock
  across a switch. Its only failure is "owning process is dead" → the message
  is dropped and the IRQ is re-acknowledged (loud: a counter is bumped, surfaced
  through a control file). No spin.
- **Init-only panics** are acceptable where the existing code already panics
  (boot-time pool allocation, `Aspace` creation during bringup) — matching
  `main9`'s established style. No panic is reachable from a live server or from
  trap/IRQ context.

## Path from here (staged)

Seven stages. Each is independently useful and gate-green on its own; 1–2 are
filed as tasks now, 3–7 are epics filed as the preceding stage lands (their
detail is only actionable once the substrate exists — filing them now would be
speculation, the same discipline `first-user-process.md` applies to the proc
redesign).

| # | Stage | One-line scope | Lands when |
|---|-------|----------------|------------|
| 1 | **Priority + PI scheduler** | extend `aarch64::process` from round-robin to priority + priority inheritance | tier-1 SMP tasks done |
| 2 | **IPC core** (`port::ipc`) | `Channel` + bounded `Message`, send/receive/reply, fast+slow path, PI | stage 1 |
| 3 | **Per-process `Aspace`** | generalize the shared user table into isolated address spaces + fault | stage 2 |
| 4 | **IRQ → message** | kernel routes a hardware IRQ to its owning server process (per-arch) | stage 2 (parallel to 3) |
| 5 | **Console server** | the UART driver moves to a user process; kernel keeps early console only | stages 3+4 |
| 6 | **Nameserver + namespace** | the bind tree becomes a process; names → channels; the file metaphor returns | stage 5 |
| 7 | **9P file servers** (fs/dev/net) | 9P over channels using the `Fid`/`Req` model; `/net` is the network stack | stage 6 |

Sequencing notes: 3 and 4 are independent and can interleave (both need only
stage 2). Stage 5 is the **proof of concept** — one driver across the IPC
boundary is what proves the whole model before the heavier 6–7. Stage 7 is where
Plan 9 becomes recognizably Plan 9: at that point the architecture is unchanged
from today's goal, it just runs on a substrate with a tiny auditable TCB and a
real determinism guarantee.

## Not building

Considered and refused, so it is not re-proposed in six months:

- **In-kernel filesystems / network / device drivers** (except the early
  console). The moment a task says "add the sd driver in-kernel because it's
  easier," that is the microkernel dying.
- **A shared address space for all processes** (Plan 9-as-shipped / 9port).
  Rejected: isolation is the point. Every server gets its own `Aspace`.
- **A generic "message router" in the kernel.** The kernel does *one* thing with
  a message: put it on a channel's queue and wake a waiter. Name→channel and
  IRQ→channel resolution live in the nameserver and the IRQ-owner table, not in
  a kernel routing abstraction (a midlayer tax — see Decision records).
- **A `Message` with a dozen optional fields / a trait-based "protocol" type.**
  The message is `opcode | tag | bounded buf | len`. Anything richer is a
  server's encoding of its own payload, not a kernel concept.
- **Exokernel / user-space IPC** (kernel does only traps, IPC is a library).
  Rejected for this project — see Decision record 4.
- **A kernel heap for user data.** The kernel allocates its own structures
  (channels, queue slots, `Aspace`s); servers allocate their own in their own
  `Aspace`.
- **Determinism over the network.** The bounded-latency guarantee holds for
  *local* IPC only; 9P over the network is not deterministic. Stated, not
  papered over.

## Decision records

- **Decision: QNX mechanism (microkernel IPC + per-process AS) as the substrate,
  Plan 9's 9P/namespace as the interface built on top — not either/or.**
  - Alternatives: **(A) Plan 9 as shipped** — in-kernel 9P server, shared
    address space, no IPC primitive; **(C) Oberon** — a single address space of
    lean modules whose narrow `pub` interfaces *are* the message passing (no
    processes, no isolation, no dynamic namespace).
  - **(A) lost** because it forfeits the user's stated goals — bounded
    interfaces, isolation, determinism — and grows the TCB back into a monolith.
    But its **9P protocol, Fid/Req server model, and namespace are grafted
    wholesale** onto the winning substrate (they are the interface layer, which
    A got right). **(C) lost** because it refuses processes and address-space
    isolation, which are the entire payoff; but its **concept-lean discipline is
    grafted** — the kernel's internal modules (ipc, sched, aspace) are
    Oberon-lean, and the "strict interface" is enforced the Oberon way (enum
    opcodes, const sizes, a module's `pub` surface is its spec).
  - Dissent: the whole-system lens objects to *two* message ideas
    in flight (kernel message vs 9P message). Resolved by layering: the kernel
    message is the mechanism, 9P is a *payload protocol* over it — one
    mechanism, one namespace, the metaphor does not fork.
- **Decision: the scheduler becomes priority-based with priority inheritance, as
  a prerequisite to IPC.**
  - Alternatives: keep round-robin and add IPC on top.
  - Lost: round-robin cannot bound priority inversion; a "deterministic" IPC on
    a fair-share scheduler is a false claim (PI *is* the reason IPC
    exists in QNX).
  - Dissent: the kernel-taste lens objects to changing a scheduler
    that works (round-robin is simpler). We accept the change because PI is
    load-bearing, not speculative — without it stage 2's determinism test cannot
    pass, so the work is forced, not preferred.
- **Decision: the kernel message is opaque and bounded; 9P is a user-space
  protocol riding on it; a narrow native opcode API is the documented exception
  for non-file servers (e.g. raw console).**
  - Alternatives: make 9P the *kernel* type (no native opcode) — rejected: it
    would force the file model onto things that are not files and bloat the TCB
    with 9P semantics; make *everything* native opcodes — rejected: it forks the
    uniform file metaphor (the simplicity/whole-system failure the design exists
    to avoid).
  - Dissent: the simplicity lens wants *only* files, no native
    opcode at all. We allow the exception because the raw console genuinely is
    not a file (it is a polled MMIO char device during early bringup), and even
    it is name-addressable so the client still sees a path. Recorded, not
    averaged away.
- **Decision: in-kernel IPC (QNX), not exokernel / user-space IPC.**
  - Alternatives: an exokernel where the kernel handles only traps and a
    userspace library does IPC (the hardware-truth pole: "rediscover
    hardware", the Barrelfish multikernel).
  - Lost: exokernel removes the *kernel* but not the *coordination* problem —
    someone still arbitrates priorities and delivers IRQs, and distributing that
    across userspace is a larger, less-auditable TCB for a hobby OS that one
    person must hold in their head (personal mastery). A small in-kernel IPC
    keeps the TCB *small and auditable in one place*.
  - Dissent: the hardware lens is the exokernel's natural
    advocate and the objection is real — a small kernel is still a kernel. We
    chose the QNX shape over the exo shape because *auditable-small* beats
    *empty-but-distributed* at this project's scale; if r9 ever scales to a
    multikernel, that is a re-opening, not a bug.
- **Decision: the early console is the single permanent kernel-resident device,
  and it is *early* — retired once the console server runs.**
  - Alternatives: move even the early console to a server (impossible — a
    server must exist before any server can run); keep the kernel UART driver
    permanently (an unjustified residence the microkernel lens must flag).
  - Dissent: the firmware lens wants the kernel device-free. We keep
    the raw char-out because it is the one thing that must exist before IPC, the
    nameserver, or any `Aspace`; the commitment is that it is the *early* path
    and stage 5 retires it. Stated so it does not silently become permanent.
- **Decision: `send` to a full channel blocks the sender; there is no drop mode.**
  - Alternatives: drop-on-full (a "nonblocking" flag), or a per-channel policy
    knob.
  - Lost: a flag makes one function do two things and a knob is a
    decision refused and exported as interface; dropping silently hides
    a stuck server.
  - Dissent: the whole-system lens notes a future server may want
    drop-on-full. It gets it by *retrying with a short block timeout* in
    user space over the primitive — the primitive stays total.

## Tasks

Filed now (actionable):
- [microkernel-priority-pi.md](../microkernel-priority-pi.md) — stage 1:
  priority + priority inheritance in `aarch64::process`. First, because stage 2
  stands on the scheduler being able to keep a determinism claim.
- [microkernel-ipc-core.md](../microkernel-ipc-core.md) — stage 2: `port::ipc`
  (`Channel`, bounded `Message`, send/receive/reply, fast+slow path, PI on the
  channel). The central primitive; aarch64 reference, x86-64/riscv64 gate-green
  but not yet exercised.

Sequenced, filed as the preceding stage lands (epics, not yet decomposed):
- Stage 3 — per-process `Aspace` (isolation + fault).
- Stage 4 — IRQ → message (per-arch GIC/APIC/PLIC routing to an owning `ProcId`).
- Stage 5 — console server (the proof of concept: one driver across the
  boundary; retires the kernel's raw console path).
- Stage 6 — nameserver + namespace (names → channels; the file metaphor returns).
- Stage 7 — 9P file servers: fs/dev/net, using the `Fid`/`Req` model from
  Plan 9's `9p.h`; `/net` is the network stack.

Dependency: stage 1 assumes the **tier-1 SMP correctness tasks** are done —
PI and per-core IRQ routing are incoherent on a scheduler that still races
itself. If they are not done, stage 1 is blocked on them, not ahead of them.
