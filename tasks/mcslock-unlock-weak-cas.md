---
id: 108
status: open
wave: 0
---

# Task 108: MCS unlock uses compare_exchange_weak with no retry loop

## Status: open — wave 0

## Problem

`port/src/mcslock.rs:69`:

```rust
if self.queue.compare_exchange_weak(p, ptr::null_mut(), AcqRel, Relaxed).is_ok() {
    return;
}
while node.next.load(Acquire).is_null() { hint::spin_loop(); }
```

`compare_exchange_weak` is permitted to fail spuriously even when
`queue == p`.  On aarch64 it lowers to LDAXR/STLXR, and the exclusive
monitor is cleared by exception entry and return as well as by
cache-line events.

So: a core releases the uncontended lock, a timer IRQ lands between the
LDAXR and the STLXR, the STLXR fails, the CAS returns `Err`, there is no
successor — and the loop at `:74` waits forever for a `next` that will
never be written.  The lock is now permanently held and the kernel wedges
on the next acquisition, with no diagnostic.

The tick fires every 100 ms and `TABLE.lock` is taken on most syscalls,
so the window is small but continuously sampled.

The canonical MCS release (Mellor-Crummey & Scott; Linux's `osq_lock`)
uses a strong compare-exchange here, or loops on the weak one.

## Design

- Use `compare_exchange` (strong).  A retry loop on the weak form is also
  correct but strictly worse — there is nothing to gain from retrying a
  CAS whose only failure mode we care about is spurious.
- Audit the other atomics in the file for the same shape.
- `:98` and `:124` materialise `&mut *self.lock.get()` from an
  `UnsafeCell` that every other core is concurrently doing the same to.
  The operations inside are atomic so it works, but it is an aliasing
  violation LLVM is entitled to exploit under `noalias`.  Both
  `MCSLock::lock` and `unlock` take `&self`, so `&*self.lock.get()`
  suffices.  Fix in the same change.

## Tests

- Host: task 97's loom/miri model of the lock is exactly the gate that
  catches this — land the two together.  A loom run with a spurious-CAS
  model should fail before the fix.
- Integration: task 124's SMP soak image.

## Done when

- The release path cannot spin on a successor that will never arrive.
- No `&mut` is materialised from the shared `UnsafeCell`.
- Task 97's tests cover the release path.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
