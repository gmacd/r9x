//! Threads, in the honest first form (Decision 2: a thread is a process).
//!
//! A "thread" here is a *process* the caller spawned and will `wait` on (Task
//! 5) — not a light-weight execution context in the caller's address space.
//! That is what r9 can be today: the process is the scheduling unit, and a
//! process runs an *image* (an ELF), not a closure.  The closure form (a
//! thread that runs the caller's code in the caller's address space) is
//! impossible until the kernel can hand a process a function rather than an
//! image — so `spawn` is by image index, the same primitive
//! [`crate::process::spawn`] is, at the default priority.

use crate::process::{ProcessId, SpawnErr, spawn as process_spawn};

/// Spawn a "thread" (a process, the child of the caller) from the image
/// registry by `index`, passing it the child-state page at `state` and the
/// default priority.  The child is a process the caller `waits` on (Task 5);
/// the closure form is impossible — a process runs an image, not a closure —
/// so this is by index, the honest first form.
pub fn spawn(index: u64, state: usize) -> Result<ProcessId, SpawnErr> {
    process_spawn(index, state, 128)
}
