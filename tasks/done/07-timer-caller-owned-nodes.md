---
status: done
---

# Consider caller-owned (intrusive) timer nodes

**Severity: design / future**
**Status: done** (2026-07-24; the `with_allocator` check is now a hard
`assert!` in all builds, not the `debug_assert!` described below.)
Originally: **LARGELY DONE (uncommitted)** — timer.rs rewritten: `Timer` is a
caller-owned `static` (`start(&'static self)`), the sorted heap-node queue is
replaced by a fixed table of `&'static Timer` (MAX_TIMERS = 8, panics when
full), cancellation is `Timer::cancel(&self)` on the owned timer, and the
handler neither allocates nor frees. The module is now entirely safe Rust.
Also done: the `with_allocator` IrqGuard is replaced by
`debug_assert!(!port::irq::in_interrupt())`, backed by an interrupt-depth
counter bracketing `trap_unsafe`. "Interrupt context never allocates" is now
a checked invariant rather than a masked-around possibility. Also done:
`iprint`/`iprintln` (Plan 9 style: IRQ-masked, best-effort interlock, direct
polled UART write) now handles all interrupt/panic-context printing, and
`devcons::putstr` dropped its IrqGuard for the matching
`debug_assert!(!in_interrupt())` — so `println!` no longer masks IRQs for
the duration of UART output.

The current timer API transfers ownership of a heap-allocated node to the
queue (`Timer::start(self: Box<Self>)`); the interrupt handler frees fired
one-shots and cancelled nodes. That is why the handler needs the allocator at
all, and why `with_allocator` masks IRQs (task 01).

The Linux/Plan 9 design is caller-owned intrusive nodes: the timer node is
embedded in the structure that cares about the timeout (proc, device); the
timer core never allocates or frees; arming links the node, expiry unlinks it.
Realistic upcoming timers fit this naturally — a `&'static` node for the
scheduler tick, per-process sleep timeouts embedded in the proc structure.

Trade-off to resolve before switching: a queued caller-owned node must not be
dropped/moved/reused while the handler can still reach it. In C this is a
documented contract (and a classic Linux use-after-free bug class, policed by
debugobjects and timer_delete_sync semantics). In Rust it means `Pin` +
unsafe contracts, or `&'static` nodes. The Box-transfer design exists because
it makes fire/cancel races memory-safe without unsafe caller obligations —
not just to serve the demo code in main.rs.

If adopted:
- The handler becomes allocation- and deallocation-free by construction.
- Replace the IrqGuard in `with_allocator` with enforcement: an
  `in_interrupt` flag set by the trap handler, asserted in `with_allocator`,
  making "handlers never touch the allocator" a checked invariant.
- Cancellation becomes a state flag on the caller-owned node (the
  `Arc<TimerShared>` handle machinery goes away).
- The queue/GIC/console IrqGuards are unaffected — they fix lock deadlocks,
  not allocation.
