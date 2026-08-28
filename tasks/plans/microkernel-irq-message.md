# Stage 4 — IRQ → message routing

## Problem

The GIC IRQ handler today dispatches in-kernel: the timer PPI is handled by
`timer::interrupt_handler()`, and every other INTID is printed and disabled
(`trap.rs`, the `else` arm of the IRQ dispatch). A user-space process cannot
own a hardware interrupt: there is no path from a GIC INTID to a `Channel`.

The Amiga's model (see `amiga-inspiration.md`) is the shape this stage takes:
the custom chips generate interrupts, the interrupt handler looks up the
owning task, enqueues a pre-allocated message on its port, and wakes a
blocked receiver. The vertical blank interrupt is the heartbeat of the
graphics system; the display server (stage 5+) owns it. The kernel never
touches the display hardware — it delivers the interrupt to the process that
does.

## Design

### The routing table

A static array of `Option<IrqRoute>` (size `NIRQS = 16`). Each entry maps an
INTID to a channel handle and an owning `ProcId`:

```rust
struct IrqRoute {
    intid: u16,
    channel: ChannelHandle,
    owner: ProcId,
}
static IRQ_ROUTES: [Option<IrqRoute>; NIRQS] = [None; NIRQS];
static NIRQUEUED: AtomicUsize = AtomicUsize::new(0);
```

The table is set up by a new syscall (`SYSIRQCLAIM`): a user-space process
calls it with `(intid, channel_handle)`. The kernel checks that the INTID is
in the SPI range (32..=1019 on GICv2), not already claimed, and that the
channel handle is valid. It adds the routing table entry and enables the
interrupt at the GIC distributor.

The lookup is a linear scan over `NIRQS` entries (16 comparisons per IRQ —
acceptable: the IRQ handler is not the display server's hot path; that is the
server's main loop).

### The non-blocking send

`port::ipc` gains a `try_send` function: a `send` that does not block when
the queue is full. Instead of blocking the sender (the interrupt handler
cannot block — it is in interrupt context and has no process to switch), it
returns `Err(IpcErr::Full)` and the message is lost. This is the Amiga's
answer: a lost display-refresh interrupt is acceptable; a lost input
interrupt is not (the input server will re-read the device on the next
poll).

```rust
pub fn try_send<S: IpcScheduler>(
    sched: &S,
    ch: &Channel,
    msg: Message,
) -> Result<(), IpcErr>
```

The fast path (a receiver is blocked) is the same as `send`: hand the message
to the receiver and wake it. No PI in `try_send`: the sender is the kernel
(not a process), so there is no client priority to inherit. The slow path
(no receiver blocked, room in the queue) enqueues and returns `Ok(())`. The
full-queue path returns `Err(IpcErr::Full)` (the message is lost).

### The IRQ handler

The trap handler's IRQ dispatch changes from:

```rust
if intid == timer::intid() {
    timer::interrupt_handler();
} else {
    iprintln!("Unhandled GIC IRQ {intid}");
    gic::disable_interrupt(intid);
}
```

to:

```rust
if intid == timer::intid() {
    timer::interrupt_handler();
} else if let Some(ch) = irq::route(intid) {
    // Enqueue a pre-allocated message on the owning channel.
    // The message is a fixed opcode (the INTID), no payload: the server
    // knows which device fired because it claimed the INTID.
    let msg = Message { opcode: intid as u16, tag: 0, len: 0, buf: [0; MSG_MAX] };
    let _ = ipc::try_send(&KernSched, ch, msg);  // Full → lost (Amiga's answer)
} else {
    iprintln!("Unhandled GIC IRQ {intid}");
    gic::disable_interrupt(intid);
}
```

The `let _ =` is deliberate: a full queue means the server is not keeping up,
and the interrupt is lost. No print (it would be on the hot path). The
`process::irq_resched()` at the trap tail is unchanged (it handles the timer
tick's resched flag).

### The syscall

`SYSIRQCLAIM = 19`: x0 = INTID, x1 = channel handle. The kernel:
1. Checks the INTID is in the SPI range (32..=1019 on GICv2).
2. Checks the channel handle is valid (created via `ipc::create()`).
3. Checks the INTID is not already claimed.
4. Adds the routing table entry (`IrqRoute { intid, channel, owner }`).
5. Enables the interrupt at the GIC distributor (`gic::enable_interrupt`).
6. Returns 0 on success, an error code on failure.

The owner is the current process (`process::current_id()`). The close-on-
owner-death hook is not wired this arc (same as the channel table).

### What this stage refuses to build

- **An in-kernel interrupt handler for a device**: the kernel never handles a
  device IRQ in-kernel. The timer PPI is the exception (it is the scheduler's
  heartbeat, not a device). Every other IRQ is routed to a user-space process.
- **A priority for IRQ messages**: the interrupt handler does not boost the
  receiver. The server runs at its base priority; if it needs a higher
  priority, it sets it via the existing `set_priority` path. The PI
  mechanism (stage 1) is for request/reply, not for IRQ delivery.
- **A per-IRQ priority at the GIC**: all SPIs are enabled at the default
  priority (0xa0). A server that needs a higher-priority interrupt is a
  later concern (the GIC's priority register is a knob, not a decision).
- **Input re-read on lost interrupt**: the Amiga's answer is that a lost
  input interrupt is the server's problem (it re-reads the device on the
  next poll). The kernel does not re-queue or retry.

### Hardware assumptions

- **aarch64 (Pi 4 / QEMU `raspi4b`)**: GICv2. INTID space 0..1019 (SGIs
  0..15, PPIs 16..31, SPIs 32..1019). The SPI range is 32..=1019 (988
  interrupts). The VideoCore VI GPU's HBLANK/VBLANK interrupt is an SPI
  (the exact INTID depends on the DT; the test image uses a known SPI).
  The Pi's firmware owns the UART and mailbox — the early console stays
  in-kernel (the timer PPI is the other in-kernel interrupt).
- **x86-64 (QEMU `q35`)**: APIC. Vector space 0..255. The VirtIO GPU's
  config change interrupt is a vector. This arch is gate-green only (the
  aarch64 arc is the reference implementation).
- **riscv64 (QEMU `virt`, `nezha`)**: PLIC/CLINT. The VirtIO GPU's config
  change interrupt is an IRQ. This arch is gate-green only.

### Init/bringup order

1. GIC init (existing): the distributor and CPU interface are brought up.
2. Channel table (existing): `ipc::create()` allocates channels.
3. `SYSIRQCLAIM` (new): a user-space process claims an INTID. The kernel
   adds the routing table entry and enables the interrupt at the distributor.
4. IRQ delivery (new): the GIC fires, the trap handler looks up the routing
   table, calls `try_send`, and wakes a blocked receiver.

The order is load-bearing: the routing table entry must exist before the
GIC enables the interrupt (otherwise the interrupt fires with no route and
is disabled). The `SYSIRQCLAIM` syscall does both (add the entry, then
enable the interrupt) in one critical section.

### Failure policy

- A `SYSIRQCLAIM` with a bad INTID (out of SPI range) returns an error.
- A `SYSIRQCLAIM` with a bad channel handle returns an error.
- A `SYSIRQCLAIM` with an already-claimed INTID returns an error.
- A `try_send` with a full queue returns `Err(IpcErr::Full)` (the message
  is lost; no print, no retry).
- An IRQ with no routing table entry is printed and disabled (the existing
  path).

## Amiga design questions

- **Vertical blank**: on the Pi 4, the VideoCore VI GPU's HBLANK/VBLANK
  interrupt is the heartbeat. The period is ~16.7 ms at 60 Hz. The test
  image uses a known SPI (the PL011 UART's interrupt, which is available
  on QEMU `raspi4b`), not the GPU's (the GPU is a stage 5+ concern).
- **Interrupt context budget**: the IRQ handler does three things: lookup
  (linear scan over 16 entries), enqueue (`try_send` on the channel), wake
  (mark the receiver Runnable). No allocation, no lock held across a switch
  (the `wake` doesn't switch). The budget is met.
- **Per-IRQ message pool**: the channel's bounded queue (QUEUE_CAP = 8) is
  the per-IRQ message pool. If the queue is full, the message is lost
  (the Amiga's answer).
- **Display server ownership**: not this stage (stage 5+). The kernel never
  touches the GPU's MMIO.
- **Boot to graphics**: not this stage (stage 5+). The console server is
  the first step; the display server is a later stage.

## Panel critique

### Simplicity and interfaces

The routing table is a small static array (16 entries). The `try_send`
function is a variant of `send` (the same fast/slow/full paths, but the full
path returns an error instead of blocking). The `SYSIRQCLAIM` syscall is a
single atomic operation (add the entry, enable the interrupt). The concept
count is low: one new struct (`IrqRoute`), one new function (`try_send`),
one new syscall (`SYSIRQCLAIM`).

The interface is narrow: the user-space process claims an INTID, and the
kernel delivers messages to its channel. The protocol is the server's
concern (the kernel is opaque to it).

### Microkernel and firmware

The interrupt context budget is met: the IRQ handler does lookup, enqueue,
wake — three things, no allocation, no lock held across a switch. The
`try_send` function takes the channel lock (the MCS lock), but it does not
hold it across a switch (the `wake` doesn't switch; it just marks the
process Runnable).

The init/bringup order is stated: the routing table entry is added before
the GIC enables the interrupt (in one critical section, in `SYSIRQCLAIM`).

The firmware co-tenancy is unchanged: the Pi's firmware owns the UART and
mailbox (the early console stays in-kernel). The timer PPI is the other
in-kernel interrupt (the scheduler's heartbeat).

### Kernel taste

The routing table is a small static array (16 entries), not a hash table or
a B-tree. The lookup is a linear scan (16 comparisons per IRQ). This is the
right shape for a small, bounded table: the linear scan is simpler than a
hash table, and the 16 comparisons are cheap (the IRQ handler is not the
display server's hot path).

The `try_send` function is a variant of `send`, not a separate mechanism.
The code is shared (the fast/slow/full paths are the same; only the full
path's behaviour differs). This is the right shape: the kernel does not
duplicate the send logic.

### Hardware truth

The INTID range check (32..=1019) is the GICv2 SPI range (Arm GICv2
Architecture Specification, section 3.4.1: SPIs are INTIDs 32..1019). The
test image uses a known SPI (the PL011 UART's interrupt, INTID 129 on QEMU
`raspi4b`), which is in the SPI range.

The `try_send` function does not allocate (the message is a stack-allocated
`Message` struct, and the queue is a fixed array). The MCS lock is the same
lock `send` uses (no new lock).

The memory ordering is the same as `send`: the MCS lock provides the
acquire/release for the channel's state. The `wake` function (which marks
the process Runnable) uses the same atomics as the existing `wake` (no new
ordering requirement).

### Whole system

The new concepts are: one struct (`IrqRoute`), one function (`try_send`),
one syscall (`SYSIRQCLAIM`). The existing concepts (the channel, the process
table, the GIC driver) are unchanged. The net concept count is +3.

The metaphor is extended, not introduced: the IPC channel is the existing
metaphor. The IRQ routing table is a new table, but it's a simple mapping
(INTID → channel handle). The `try_send` function is a variant of `send`,
not a new mechanism.

After this lands, one person can still hold the subsystem in their head: the
IRQ handler looks up the routing table, calls `try_send`, and wakes a
blocked receiver. Three things.

### Clarity and composition

The `try_send` function is a variant of `send`, and the code is shared (the
fast/slow/full paths are the same; only the full path's behaviour differs).
The `SYSIRQCLAIM` syscall is a single atomic operation (add the entry,
enable the interrupt). The routing table lookup is a linear scan (16
comparisons per IRQ).

The code is boring: the routing table is a small static array, the `try_send`
function is a variant of `send`, and the IRQ handler is a three-line
dispatch (timer, route, disable).

### Amiga shape

The vertical blank interrupt is the heartbeat of the graphics system. The
IRQ handler routes it to the display server's channel (a stage 5+ concern).
The interrupt context budget is met: lookup, enqueue, wake — three things,
no allocation, no lock held across a switch.

The per-IRQ message pool is the channel's bounded queue (QUEUE_CAP = 8). If
the queue is full, the message is lost (the Amiga's answer: acceptable for a
display refresh, not for input).

The display server (stage 5+) owns the GPU MMIO via a `map_mmio` verb. The
kernel never touches the GPU's MMIO.

## Decision records

1. **`try_send` is a variant of `send`, not a separate mechanism.** The
   fast/slow/full paths are shared; only the full path's behaviour differs
   (return `Err(IpcErr::Full)` instead of blocking). The whole-system lens argued for
   a separate `IrqMessage` type (to make the "no allocation" invariant
   explicit); we chose the variant because the code is shared and the
   invariant is stated in the doc comment.

2. **No PI in `try_send`.** The sender is the kernel (not a process), so
   there is no client priority to inherit. The kernel-taste lens argued that
   the display server should run at the client's priority (the vertical
   blank is urgent); we chose no PI because the server sets its own priority
   via `set_priority` (the PI mechanism is for request/reply, not for IRQ
   delivery).

3. **The routing table is a linear scan, not a hash table.** The table has
   16 entries; a linear scan is 16 comparisons per IRQ. The hardware-truth lens argued
   that a hash table is O(1); we chose the linear scan because 16
   comparisons is cheap (the IRQ handler is not the display server's hot
   path) and the linear scan is simpler.

4. **The message is a fixed opcode (the INTID), no payload.** The server
   knows which device fired because it claimed the INTID. The simplicity lens
   argued for a payload (to carry device-specific data); we chose no payload
   because the server can read the device's state directly (it owns the
   MMIO via a `map_mmio` verb, stage 5+).

5. **A lost interrupt is not retried.** The Amiga's answer: a lost display-
   refresh interrupt is acceptable; a lost input interrupt is the server's
   problem (it re-reads the device on the next poll). The microkernel lens argued
   for a retry (to avoid lost input); we chose no retry because the retry
   would require a per-IRQ state machine (a later concern) and the server
   can poll the device.

## Tasks

- [irq-route.md](../irq-route.md) — the routing table, `try_send`, and the
  `SYSIRQCLAIM` syscall. The aarch64 reference implementation.
- [irq-integration.md](../irq-integration.md) — the integration image: a
  user-space process claims a SPI, the GIC fires, the process receives a
  message on its channel.
