---
status: done
---

# Make interrupt-context tracking per-core

`INTERRUPT_DEPTH` (port/src/irq.rs:26) is one global `AtomicUsize`. The
comment says so plainly — "Core-local in spirit; needs to become per-core
state under SMP" — but two callers now treat its answer as an invariant:

- port/src/allocator.rs:518 — `assert!(!crate::irq::in_interrupt(),
  "allocation in interrupt context")`
- port/src/devcons.rs:48 — `debug_assert!(!crate::irq::in_interrupt(),
  "println in interrupt context; use iprintln")`

Once secondaries boot, core 0 taking any exception runs `enter_interrupt()`
in the trap path and the global depth is non-zero for *every* core. Core 1,
in ordinary thread context, calls `Box::new` and panics on a legal
allocation. `println!` from core 1 trips the debug assert the same way. The
window is the whole of any exception on any core, which is not rare.

A comment acknowledging a gap is fine while the gap is unreachable. This
branch made it reachable: promoting the allocator check from a debug
assert to a hard `assert!` turns a documented limitation into a guaranteed
panic, and AGENTS.md makes multi-core correctness a hard requirement rather
than a later port.

Fix direction (needs the per-core-state decision, which nothing in tree has
made yet): the depth belongs in whatever per-core block the SMP bringup
introduces — MPIDR-indexed array, TPIDR_EL1-based core block, or the
equivalent. Until that exists, the honest alternative is to weaken the
allocator back to a `debug_assert!` and say why, rather than ship an
`assert!` that is only correct on one core.

Note: gic-timer-review-nits.md #6 asks for the single-core assumption to be
*documented*. This task is the other half — the places where documenting it
is no longer enough.

Done when: `in_interrupt()` answers for the calling core only, and the
allocator and devcons asserts hold on a multi-core boot.

Origin: code review of the qemu-integration-tests branch (main...HEAD).
