//! Process control: the syscalls a process issues about itself and to launch
//! another.

use r9x_abi::{
    SPAWN_BAD_INDEX, SPAWN_BAD_STATE, SPAWN_ERR_MIN, SPAWN_NO_SLOT, SYS_SPAWN, SYSEXIT, SYSYIELD,
};

use crate::sys::sys;

/// A process id: an index into the kernel's process table.  The kernel hands
/// it back on a successful [`spawn`]; it names the child in the table (the
/// kernel's `status` and the spawner's `wait` — Task 5 — key on it).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ProcessId(usize);

impl ProcessId {
    /// The id as an index: for kernel-side status lookups (the test images)
    /// and `wait` (Task 5).
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

/// An error from [`spawn`]: a value the kernel returns that is not a process
/// id (at or above [`SPAWN_ERR_MIN`]).  The spawner recovers from all of these
/// — they are not faults.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpawnErr {
    /// The image index is not in the registry (out of range, or the registry
    /// empty).
    BadIndex,
    /// The process table is full (no free slot).
    NoSlot,
    /// The child-state or the priority is malformed (too many handles, or the
    /// priority is the idle sentinel or above).
    BadState,
}

/// Spawn a process from the image registry by `index`, handing it the
/// child-state page at `state` (a page in *this* process's address space, laid
/// out as `[n_handles, handles..., argc, argv...]`, or 0 for none) and the
/// `prio` priority (0 most urgent; 255, the idle sentinel, is refused).
/// Returns the child's id, or the error (a bad index, a full table, or a
/// malformed state/priority — all recoverable, not faults).
pub fn spawn(index: u64, state: usize, prio: u64) -> Result<ProcessId, SpawnErr> {
    let (id, _, _) = unsafe { sys(SYS_SPAWN, index, state as u64, prio, 0, 0) };
    match id {
        SPAWN_BAD_INDEX => Err(SpawnErr::BadIndex),
        SPAWN_NO_SLOT => Err(SpawnErr::NoSlot),
        SPAWN_BAD_STATE => Err(SpawnErr::BadState),
        // A value below the error bound is a process id (a table index).
        _ if id < SPAWN_ERR_MIN => Ok(ProcessId(id as usize)),
        // An unknown code at or above the bound: treat as a bad state rather
        // than a bogus id (defensive — the kernel returns only the three).
        _ => Err(SpawnErr::BadState),
    }
}

/// End this process.  The kernel records the svc number as the exit status,
/// so `code` is carried for the ABI's shape and is not a distinguishable
/// code.
#[inline(never)]
pub fn exit(code: u64) -> ! {
    let _ = unsafe { sys(SYSEXIT, code, 0, 0, 0, 0) };
    loop {
        core::hint::spin_loop();
    }
}

/// Voluntarily yield the CPU to other ready processes.
pub fn yield_now() {
    let _ = unsafe { sys(SYSYIELD, 0, 0, 0, 0, 0) };
}
