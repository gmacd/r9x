//! The clock: the arch's monotonic counter, in ticks.  A `SYS_CLOCK` read —
//! the primitive a user-space server needs to pace itself (the display's
//! 60 Hz, a timer wheel's tick) without polling.  The wait is [`crate::ipc::receive_at`],
//! a bounded receive on the server's own channel.

use r9x_abi::SYS_CLOCK;

use crate::sys::sys;

/// The counter frequency (ticks per second).  A hardware constant the user
/// reads from `CNTFRQ_EL0` (EL0 opt-in at boot); on QEMU it is 1 GHz.  A
/// compile-time constant: a hardware change is a one-line update here.
pub const COUNTER_FREQ: u64 = 1_000_000_000;

/// The monotonic clock's value, in ticks.  A `SYS_CLOCK` read — a register
/// read in the kernel, so no lock, no allocation.  The caller converts ticks
/// to a duration with [`COUNTER_FREQ`].
pub fn now() -> u64 {
    unsafe { sys(SYS_CLOCK, 0, 0, 0, 0, 0).0 }
}

/// A duration, in ticks.  Convert to/from seconds with [`COUNTER_FREQ`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Duration {
    /// The duration, in ticks.
    pub ticks: u64,
}

impl Duration {
    /// A duration of `secs` seconds.
    pub fn from_secs(secs: u64) -> Self {
        Self { ticks: secs * COUNTER_FREQ }
    }

    /// A duration of `n` ticks.
    pub fn from_ticks(ticks: u64) -> Self {
        Self { ticks }
    }

    /// The deadline `d` ticks from now (saturating: no overflow).
    pub fn deadline_from(d: Duration) -> u64 {
        now().saturating_add(d.ticks)
    }
}
