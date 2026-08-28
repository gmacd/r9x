---
status: done
---

# Timer queue lock can deadlock against the interrupt handler

**Severity: blocking**
**Status: done** (verified 2026-07-24)
Originally: **FIXED (uncommitted)** — `port/src/irq.rs` IrqGuard + arch hooks in
`aarch64/src/irq.rs`; guards applied in timer.rs (`with_timers`), gic.rs,
devcons.rs, allocator.rs. The timer subsystem was subsequently redesigned to
caller-owned static timers (task 07), so the interrupt handler no longer
allocates or frees at all. Verified: all-arch clippy/dist/test green; QEMU
raspi4b boot runs the timer demo to completion (cancel included).

`Timer::start` (`aarch64/src/timer.rs:95-101`) takes `TIMER_QUEUE` with IRQs
enabled. If the timer IRQ fires on the same core while the lock is held,
`timer::interrupt_handler` (`aarch64/src/timer.rs:210`) spins on the same lock
in trap context and the interrupted holder never resumes — a hard single-core
deadlock. Exception entry masks IRQs *inside* the handler, but nothing protects
thread-context acquisitions.

The same class of problem applies to any lock shared between thread and IRQ
context:

- The heap allocator: `drain_due` allocates a `Vec` and the handler drops
  `Box`es in IRQ context. If the allocator lock is held by the interrupted
  thread, same deadlock.
- The console lock: callbacks `println!` in IRQ context while `main9` also
  prints.
- The GIC lock: `gic::init` holds it while a timer IRQ can already be pending
  (see task 04), and `gic::try_ack_interrupt` would spin on it in trap context.

## Fix

Introduce a spin_lock_irqsave-style primitive: mask IRQs (DAIF.I) before
acquiring any lock that is also taken in interrupt context, restore the
previous mask state on release. Apply to the timer queue lock, the GIC lock,
and audit the allocator and console paths.

Currently latent only because the demo timers are sparse enough that an IRQ
landing inside `start()`'s critical section is unlikely; it is a real hang when
it hits.

Update: the allocator case is worse than a deadlock.
`GlobalQuickAlloc::with_allocator` (`port/src/allocator.rs:509-518`) swaps the
`QuickFit` pointer to null during an allocation and asserts non-null on entry,
so an IRQ-context alloc/free landing inside a thread-context allocation panics
("global allocator is nil") rather than hanging.

## Proposed implementation

1. Add `port/src/irq.rs` with an RAII `IrqGuard` (mask IRQs on `new`, restore
   previous state on `Drop`; nestable; `cfg!(test)` no-op following the
   `reg/cnt_el0.rs` idiom). Per-arch inner asm via `cfg(target_arch)`:
   aarch64 `mrs daif` / `msr daifset, #2`; riscv64 `csrrc sstatus` (SIE);
   x86-64 `pushfq` + `cli`.

2. Wrap every acquisition of a lock shared with IRQ context:
   - `timer.rs`: a `with_timer_queue(f)` helper that takes `IrqGuard` + a
     fresh `LockNode::new()` (also resolves task 02) + the queue lock; used by
     `Timer::start` and all three lock sections of `interrupt_handler`.
     Delete `TIMER_LOCK_NODE`.
   - `gic.rs`: `IrqGuard` in `init`, `try_ack_interrupt`, `end_interrupt`
     (no-op in trap context, closes the `init` window).
   - `devcons.rs`: `IrqGuard` before the `CONS.lock` in the print path.
   - `allocator.rs`: `IrqGuard` at the top of `with_allocator` (fixes the
     null-swap panic).

3. Optional follow-up: remove allocation from the handler entirely —
   `drain_due` can cut the intrusive list at the first non-due node and
   return the old head instead of collecting into a `Vec`.

Same-core reentrancy is the only deadlock possible today; on SMP the MCS lock
handles cross-core contention and composes correctly with the masking
(spin_lock_irqsave semantics).
