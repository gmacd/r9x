//! Process control: the syscalls a process issues about itself and to launch
//! another.

use r9x_abi::{
    KILL_BAD_ID, SETPRIO_BAD_PRIO, SPAWN_BAD_INDEX, SPAWN_BAD_STATE, SPAWN_ERR_MIN, SPAWN_NO_SLOT,
    SYS_KILL, SYS_SETPRIO, SYS_SPAWN, SYS_WAIT, SYSEXIT, SYSYIELD, WAIT_BAD_ID, WAIT_TIMEOUT,
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

/// The result of a [`wait`](ProcessId::wait) call.
pub enum WaitResult {
    /// A child was reaped: its id and exit status.
    Reaped(ProcessId, u64),
    /// No child was available (timeout or no matching zombie).
    Timeout,
    /// The specified child id is not a zombie.
    BadId,
}

impl ProcessId {
    /// Wait for this child to finish.  Returns its exit status on success,
    /// or an error.  The deadline is in counter ticks (0 = block forever;
    /// the current implementation always returns immediately).
    pub fn wait(self, deadline: u64) -> Result<u64, WaitError> {
        let (id, status, _) = unsafe { sys(SYS_WAIT, self.0 as u64, deadline, 0, 0, 0) };
        if id == WAIT_BAD_ID {
            Err(WaitError::BadId)
        } else if id == WAIT_TIMEOUT {
            Err(WaitError::Timeout)
        } else {
            Ok(status)
        }
    }

    /// Terminate the process with this id.  Returns `Ok(())` on success,
    /// `Err(KillError::BadId)` if the id is not a live process.
    pub fn kill(self) -> Result<(), KillError> {
        let (result, _, _) = unsafe { sys(SYS_KILL, self.0 as u64, 0, 0, 0, 0) };
        if result == KILL_BAD_ID { Err(KillError::BadId) } else { Ok(()) }
    }
}

/// Wait for any child to finish.  Returns the reaped child's id and exit
/// status, or an error.
pub fn wait_any(deadline: u64) -> Result<(ProcessId, u64), WaitError> {
    let (id, status, _) = unsafe { sys(SYS_WAIT, 0, deadline, 0, 0, 0) };
    if id == WAIT_BAD_ID {
        Err(WaitError::BadId)
    } else if id == WAIT_TIMEOUT {
        Err(WaitError::Timeout)
    } else {
        Ok((ProcessId(id as usize), status))
    }
}

/// An error from [`wait`](ProcessId::wait).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WaitError {
    /// No zombie was available (timeout).
    Timeout,
    /// The specified child id is not a zombie.
    BadId,
}

/// An error from [`kill`](ProcessId::kill).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KillError {
    /// The target id is not a live or zombie process.
    BadId,
}

/// A process's priority: 0 is most urgent, 254 is the least urgent
/// schedulable level (255 is the idle sentinel, not a settable priority).
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub struct Priority(u8);

impl Priority {
    /// Most urgent (level 0).
    pub const MIN: Priority = Priority(0);
    /// Least urgent schedulable level (254).
    pub const MAX: Priority = Priority(254);
    /// The idle sentinel (not settable; a process at this level is never
    /// scheduled).
    pub const IDLE: Priority = Priority(255);

    /// Create a priority from a level (0-254; 255 is the idle sentinel and
    /// is refused by `set_priority`).
    pub const fn new(level: u8) -> Priority {
        Priority(level)
    }

    /// The numeric level.
    pub const fn level(self) -> u8 {
        self.0
    }
}

/// An error from [`set_priority`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SetPrioError {
    /// The target id is not a live process.
    BadId,
    /// The priority is the idle sentinel (255).
    BadPrio,
}

/// Set a process's priority.  `target` 0 means self.  Returns `Ok(())` on
/// success, `Err(BadId)` if the target is not live, `Err(BadPrio)` if the
/// priority is the idle sentinel.
pub fn set_priority(target: u64, prio: Priority) -> Result<(), SetPrioError> {
    let result = unsafe { sys(SYS_SETPRIO, target, prio.level() as u64, 0, 0, 0) };
    match result.0 {
        0 => Ok(()),
        SETPRIO_BAD_PRIO => Err(SetPrioError::BadPrio),
        _ => Err(SetPrioError::BadId),
    }
}
