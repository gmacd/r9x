//! Core-local interrupt masking and interrupt-context tracking.
//!
//! Any lock that is also taken in interrupt context (e.g. the console
//! lock) must be held with interrupts masked; otherwise an interrupt
//! arriving while the lock is held leaves the handler spinning on a
//! lock its own core can never release.  `IrqGuard` provides that
//! masking as an RAII guard.
//!
//! `in_interrupt` supports the complementary approach for subsystems
//! that interrupt context is simply forbidden to use (e.g. the
//! allocator): assert the invariant instead of masking around it.
//!
//! Masking is architecture-specific, so each arch registers its
//! implementation at early boot via `set_ops`, before enabling
//! interrupts (the pattern devcons uses for the Uart).  Until then, and
//! in hosted test builds where the mask instructions would be
//! privileged, `IrqGuard` is a no-op.

use core::marker::PhantomData;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::once::Once;

/// Upper bound on the cores the kernel supports.  The interrupt-depth
/// table is sized to it, and every arch's `core_id` hook must return an index
/// below it.  All current targets (Pi 4: 4 cores; QEMU `q35` and `virt`) fit
/// with headroom; raising this is the one-line change if a target with more
/// cores lands.
const MAX_CPUS: usize = 16;

// Depth rather than a flag, so nested exceptions stay counted.  Per core: the
// table answers "is *this* core in interrupt context", so a trap on one core
// must not read as interrupt context on another — the whole reason it is a
// table and not a single counter.  `core()` supplies the index.
// `AtomicUsize` is `!Copy`, so the table cannot be written with a `[x; N]`
// repeat (and `array::from_fn` is not yet const-stable); it is written out
// element-wise instead.
static INTERRUPT_DEPTH: [AtomicUsize; MAX_CPUS] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];

/// The calling core's index into `INTERRUPT_DEPTH`, from the arch's `core_id`
/// hook.  Zero when no hook is registered (hosted tests, and arches that do
/// not yet track per-core interrupt depth) — the correct answer for a single
/// boot core.
fn core() -> usize {
    let c = ops().map(|o| (o.core_id)()).unwrap_or(0);
    debug_assert!(c < MAX_CPUS, "core_id {} exceeds MAX_CPUS", c);
    c
}

/// Mark entry to interrupt context on the calling core.  Called by the arch
/// trap handler.
pub fn enter_interrupt() {
    enter_on(core());
}

/// Mark exit from interrupt context on the calling core.  Called by the arch
/// trap handler.
pub fn exit_interrupt() {
    exit_on(core());
}

/// True while the calling core is handling an interrupt or exception.
pub fn in_interrupt() -> bool {
    in_interrupt_on(core())
}

fn enter_on(core: usize) {
    INTERRUPT_DEPTH[core].fetch_add(1, Ordering::Relaxed);
}

fn exit_on(core: usize) {
    INTERRUPT_DEPTH[core].fetch_sub(1, Ordering::Relaxed);
}

fn in_interrupt_on(core: usize) -> bool {
    INTERRUPT_DEPTH[core].load(Ordering::Relaxed) > 0
}

/// Architecture hooks for the current core's interrupt context.
/// `mask` masks interrupts and returns the previous interrupt state;
/// `restore` reinstates a state previously returned by `mask`; `core_id`
/// returns the calling core's index into `INTERRUPT_DEPTH` (always below
/// `MAX_CPUS`).
pub struct IrqOps {
    pub mask: fn() -> u64,
    pub restore: fn(u64),
    pub core_id: fn() -> usize,
}

static IRQ_OPS: Once<&'static IrqOps> = Once::new();

/// Register the architecture's mask/restore implementation.  Call once
/// at early boot, before interrupts are first enabled.
pub fn set_ops(ops: &'static IrqOps) {
    let _ = IRQ_OPS.set(ops);
}

fn ops() -> Option<&'static IrqOps> {
    IRQ_OPS.get().copied()
}

/// Masks interrupts on the current core for its lifetime, restoring the
/// previous mask state on drop.  Nestable: taking a guard with
/// interrupts already masked (e.g. in interrupt context) is a no-op.
/// Create the guard before acquiring any lock shared with interrupt
/// context, and let it drop after the lock is released.
pub struct IrqGuard {
    saved: Option<u64>,
    // The saved state is core-local, so the guard must not move to
    // another core: !Send + !Sync.
    _not_send: PhantomData<*mut ()>,
}

impl IrqGuard {
    pub fn new() -> Self {
        Self { saved: ops().map(|ops| (ops.mask)()), _not_send: PhantomData }
    }
}

impl Default for IrqGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for IrqGuard {
    fn drop(&mut self) {
        if let (Some(saved), Some(ops)) = (self.saved, ops()) {
            (ops.restore)(saved);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is the fix: a trap on one core must not read as interrupt
    /// context on another.  A single shared counter (the old code) fails the
    /// first `assert!(!in_interrupt_on(1))` below.
    #[test]
    fn depth_is_per_core() {
        enter_on(0);
        assert!(in_interrupt_on(0));
        assert!(!in_interrupt_on(1), "core 1 read core 0's interrupt depth");

        enter_on(1);
        assert!(in_interrupt_on(0), "entering core 1 must not clear core 0");
        assert!(in_interrupt_on(1));

        exit_on(1);
        assert!(in_interrupt_on(0), "exiting core 1 must not clear core 0");
        assert!(!in_interrupt_on(1));

        exit_on(0);
        assert!(!in_interrupt_on(0));
        assert!(!in_interrupt_on(1));
    }
}
