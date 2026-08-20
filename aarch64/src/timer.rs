//! Minimal timer subsystem using the ARMv8 architectural timer.
//!
//! Timers are caller-owned: the subsystem stores only `&'static`
//! references in a small fixed table, never allocates, and the
//! interrupt handler frees nothing.  Timers therefore live in statics,
//! with interior mutability making them shareable with the handler.

use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use core::time::Duration;

use port::irq::IrqGuard;
use port::mcslock::{Lock, LockNode};

use crate::reg::cnt_el0::{CntFrqEl0, CntPctEl0, CntpCtlEl0, CntpCvalEl0};

/// Fired in interrupt context.  Return true to keep a periodic timer
/// running; the return value is ignored for one-shot timers.
pub trait TimerCallback: Send + Sync {
    fn fire(&self) -> bool;
}

/// A caller-owned timer.  `start` registers it with the subsystem,
/// which only ever borrows it.  Not designed for concurrent restarts
/// of the same timer.
pub struct Timer {
    duration: Duration,
    repeat: bool,
    deadline_ticks: AtomicU64,
    period_ticks: AtomicU64,
    active: AtomicBool,
    callback: &'static dyn TimerCallback,
}

impl Timer {
    /// A one-shot timer firing once, `relative` after `start`.
    pub const fn new(relative: Duration, callback: &'static dyn TimerCallback) -> Self {
        Self {
            duration: relative,
            repeat: false,
            deadline_ticks: AtomicU64::new(0),
            period_ticks: AtomicU64::new(0),
            active: AtomicBool::new(false),
            callback,
        }
    }

    /// A timer firing every `period` until its callback returns false
    /// or it is cancelled.
    pub const fn periodic(period: Duration, callback: &'static dyn TimerCallback) -> Self {
        let mut timer = Self::new(period, callback);
        timer.repeat = true;
        timer
    }

    /// Register and arm the timer.  Panics if the timer table is full,
    /// or if a periodic timer's period is shorter than one counter
    /// tick.
    pub fn start(&'static self) {
        let ticks = duration_to_ticks(self.duration);
        // The re-arm in interrupt_handler divides by the period, so a
        // periodic timer must be at least one tick long.  (A zero-tick
        // one-shot is fine: it just fires immediately.)
        assert!(
            !self.repeat || ticks > 0,
            "timer: periodic duration {:?} is shorter than one counter tick",
            self.duration
        );
        self.period_ticks.store(ticks, Ordering::Relaxed);
        self.deadline_ticks.store(now() + ticks, Ordering::Relaxed);
        self.active.store(true, Ordering::Release);
        let _irq = IrqGuard::new();
        register(self);
        arm_hardware();
    }

    /// Deactivate the timer.  Lazy: an already-armed hardware deadline
    /// may still cause one spurious wakeup.  Idempotent, and safe to
    /// call from any callback, including the timer's own.
    pub fn cancel(&self) {
        self.active.store(false, Ordering::Release);
    }
}

// Both write the whole of CNTP_CTL_EL0 rather than read-modify-write
// it.  IMASK must be clear or the timer counts down and sets ISTATUS
// but never raises its PPI — an armed timer that silently never fires —
// and firmware is not obliged to leave it that way.  ISTATUS is
// read-only, so writing zero to it is harmless.

fn timer_enable() {
    CntpCtlEl0(0).with_enable(true).write();
}

fn timer_disable() {
    CntpCtlEl0(0).write();
}

const MAX_TIMERS: usize = 8;

static TIMERS: Lock<[Option<&'static Timer>; MAX_TIMERS]> = Lock::new("timers", [None; MAX_TIMERS]);

/// Run `f` with the timer table locked and IRQs masked.  The lock is
/// shared with the interrupt handler, so it must never be held with
/// IRQs enabled: a timer interrupt arriving mid-hold would spin on it
/// forever.  (In the handler itself the masking is a harmless no-op.)
fn with_timers<R>(f: impl FnOnce(&mut [Option<&'static Timer>; MAX_TIMERS]) -> R) -> R {
    let _irq = IrqGuard::new();
    let node = LockNode::new();
    let mut guard = TIMERS.lock(&node);
    f(&mut guard)
}

fn register(timer: &'static Timer) {
    with_timers(|timers| {
        // Already registered (a restart)?
        if timers.iter().flatten().any(|t| ptr::eq(*t, timer)) {
            return;
        }
        // Take a free slot, or one whose timer is no longer active.
        for slot in timers.iter_mut() {
            if slot.is_none_or(|t| !t.active.load(Ordering::Acquire)) {
                *slot = Some(timer);
                return;
            }
        }
        panic!("timer table full");
    })
}

/// Arm the hardware for the earliest active deadline, or disarm if
/// there is none.  Call with IRQs masked.
fn arm_hardware() {
    let timers = with_timers(|timers| *timers);
    let next = timers
        .into_iter()
        .flatten()
        .filter(|t| t.active.load(Ordering::Acquire))
        .map(|t| t.deadline_ticks.load(Ordering::Relaxed))
        .min();
    match next {
        Some(deadline) => {
            CntpCvalEl0::write(deadline);
            timer_enable();
        }
        None => timer_disable(),
    }
}

pub fn init() {
    // duration_to_ticks re-checks this on every start; checking here
    // too fails at boot rather than at the first timer.
    if CntFrqEl0::read().freq() == 0 {
        panic!("timer: CNTFRQ_EL0=0: counter frequency not programmed by firmware");
    }

    timer_disable();
}

fn now() -> u64 {
    CntPctEl0::read().value()
}

/// Convert a duration to hardware ticks.  Reads CNTFRQ_EL0 directly —
/// firmware programs it before we ever run, so unlike a cached copy
/// there is no ordering dependency on `init`.
fn duration_to_ticks(dur: Duration) -> u64 {
    let freq = CntFrqEl0::read().freq();
    assert!(freq != 0, "timer: CNTFRQ_EL0=0: counter frequency not programmed by firmware");
    ((dur.as_nanos() * freq as u128) / 1_000_000_000) as u64
}

/// Hardware interrupt handler — called from the trap handler.
///
/// Fires all due timers (outside the table lock, so a callback may
/// start or cancel timers) and arms the next deadline, which is also
/// what deasserts the level-triggered timer interrupt: the new CVAL is
/// in the future, or the timer is disabled.
pub fn interrupt_handler() {
    // 1. Copy out the table so callbacks run outside the lock.
    let timers = with_timers(|timers| *timers);

    // 2. Fire due timers.
    let now = now();
    for timer in timers.into_iter().flatten() {
        if !timer.active.load(Ordering::Acquire) {
            continue;
        }
        let deadline = timer.deadline_ticks.load(Ordering::Relaxed);
        if deadline > now {
            continue;
        }
        if !timer.repeat {
            // Deactivate before firing so the callback may restart it.
            timer.active.store(false, Ordering::Release);
            timer.callback.fire();
        } else if timer.callback.fire() {
            // Advance the deadline; if the callback cancelled its own
            // timer the cleared active flag still stops it.
            // If the callback ran longer than the period, advance the
            // deadline repeatedly until it is in the future to avoid
            // an interrupt storm.  `start` guarantees a periodic
            // timer's period is at least one tick, so the division is
            // safe.
            let period = timer.period_ticks.load(Ordering::Relaxed);
            let next = deadline + ((now - deadline) / period + 1) * period;
            timer.deadline_ticks.store(next, Ordering::Relaxed);
        } else {
            timer.active.store(false, Ordering::Release);
        }
    }

    // 3. Arm next timer or disarm, deasserting the interrupt.
    arm_hardware();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reg::cnt_el0::{set_mock_count, set_mock_freq};
    use std::sync::{Mutex, MutexGuard};

    // The tests drive the real start/cancel/interrupt_handler paths
    // (IrqGuard is a no-op on hosted builds), so they share the global
    // timer table and the mocked counter and frequency and must not
    // interleave.  `reset` serialises them; a should_panic test
    // poisons the mutex, which the lock recovery shrugs off.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// 1000Hz: one tick per millisecond, so Duration::from_millis(n)
    /// converts to n ticks.
    const FREQ_1MS_PER_TICK: u64 = 1000;

    fn reset(freq: u64) -> MutexGuard<'static, ()> {
        let guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_mock_freq(freq);
        set_mock_count(0);
        with_timers(|timers| *timers = [None; MAX_TIMERS]);
        guard
    }

    /// Counts its fires; keeps a periodic timer running until the
    /// `limit`th fire.
    struct CountingCallback {
        fires: AtomicU64,
        limit: u64,
    }

    impl CountingCallback {
        const fn new(limit: u64) -> Self {
            Self { fires: AtomicU64::new(0), limit }
        }

        fn fires(&self) -> u64 {
            self.fires.load(Ordering::Relaxed)
        }
    }

    impl TimerCallback for CountingCallback {
        fn fire(&self) -> bool {
            let n = self.fires.fetch_add(1, Ordering::Relaxed) + 1;
            n < self.limit
        }
    }

    #[test]
    fn periodic_rearm_clamps_missed_deadlines() {
        let _guard = reset(FREQ_1MS_PER_TICK);
        static CALLBACK: CountingCallback = CountingCallback::new(u64::MAX);
        static TIMER: Timer = Timer::periodic(Duration::from_millis(10), &CALLBACK);
        TIMER.start();
        assert_eq!(TIMER.deadline_ticks.load(Ordering::Relaxed), 10);

        // t=15, past the deadline: fires and re-arms in the future.
        // next = 10 + ((15-10)/10 + 1)*10 = 20.
        set_mock_count(15);
        interrupt_handler();
        assert_eq!(CALLBACK.fires(), 1);
        assert_eq!(TIMER.deadline_ticks.load(Ordering::Relaxed), 20);

        // A slow callback: time jumps a whole period past the
        // deadline.  next = 20 + ((35-20)/10 + 1)*10 = 40, skipping
        // the missed t=30 expiry rather than firing immediately again.
        set_mock_count(35);
        interrupt_handler();
        assert_eq!(CALLBACK.fires(), 2);
        assert_eq!(TIMER.deadline_ticks.load(Ordering::Relaxed), 40);
    }

    #[test]
    fn one_shot_fires_once_and_ignores_return_value() {
        let _guard = reset(FREQ_1MS_PER_TICK);
        // The callback returns true ("keep running"); a one-shot must
        // deactivate anyway.
        static CALLBACK: CountingCallback = CountingCallback::new(u64::MAX);
        static TIMER: Timer = Timer::new(Duration::from_millis(10), &CALLBACK);
        TIMER.start();

        set_mock_count(15);
        interrupt_handler();
        assert_eq!(CALLBACK.fires(), 1);
        assert!(!TIMER.active.load(Ordering::Acquire));

        set_mock_count(100);
        interrupt_handler();
        assert_eq!(CALLBACK.fires(), 1);
    }

    #[test]
    fn periodic_stops_when_callback_returns_false() {
        let _guard = reset(FREQ_1MS_PER_TICK);
        static CALLBACK: CountingCallback = CountingCallback::new(2);
        static TIMER: Timer = Timer::periodic(Duration::from_millis(10), &CALLBACK);
        TIMER.start();

        set_mock_count(10);
        interrupt_handler();
        assert_eq!(CALLBACK.fires(), 1);
        assert!(TIMER.active.load(Ordering::Acquire));

        set_mock_count(20);
        interrupt_handler();
        assert_eq!(CALLBACK.fires(), 2);
        assert!(!TIMER.active.load(Ordering::Acquire));

        set_mock_count(30);
        interrupt_handler();
        assert_eq!(CALLBACK.fires(), 2);
    }

    #[test]
    fn cancelled_timer_does_not_fire() {
        let _guard = reset(FREQ_1MS_PER_TICK);
        static CALLBACK: CountingCallback = CountingCallback::new(u64::MAX);
        static TIMER: Timer = Timer::periodic(Duration::from_millis(10), &CALLBACK);
        TIMER.start();
        TIMER.cancel();

        set_mock_count(50);
        interrupt_handler();
        assert_eq!(CALLBACK.fires(), 0);
    }

    /// The handler deactivates a one-shot before firing exactly so the
    /// callback may restart it.  This pins that contract, and with it
    /// that callbacks run outside the table lock: the restart's
    /// re-registration would deadlock inside the handler otherwise.
    #[test]
    fn one_shot_callback_can_restart_its_timer() {
        let _guard = reset(FREQ_1MS_PER_TICK);

        struct RestartOnce {
            fires: AtomicU64,
        }
        impl TimerCallback for RestartOnce {
            fn fire(&self) -> bool {
                if self.fires.fetch_add(1, Ordering::Relaxed) == 0 {
                    TIMER.start();
                }
                false
            }
        }
        static CALLBACK: RestartOnce = RestartOnce { fires: AtomicU64::new(0) };
        static TIMER: Timer = Timer::new(Duration::from_millis(10), &CALLBACK);

        TIMER.start();
        set_mock_count(15);
        interrupt_handler();
        assert_eq!(CALLBACK.fires.load(Ordering::Relaxed), 1);
        assert!(TIMER.active.load(Ordering::Acquire));
        assert_eq!(TIMER.deadline_ticks.load(Ordering::Relaxed), 25);

        set_mock_count(30);
        interrupt_handler();
        assert_eq!(CALLBACK.fires.load(Ordering::Relaxed), 2);
        assert!(!TIMER.active.load(Ordering::Acquire));
    }

    #[test]
    #[should_panic(expected = "CNTFRQ_EL0=0")]
    fn start_with_unprogrammed_frequency_panics() {
        let _guard = reset(0);
        static CALLBACK: CountingCallback = CountingCallback::new(u64::MAX);
        static TIMER: Timer = Timer::periodic(Duration::from_millis(10), &CALLBACK);
        TIMER.start();
    }

    #[test]
    #[should_panic(expected = "shorter than one counter tick")]
    fn sub_tick_periodic_period_panics() {
        let _guard = reset(FREQ_1MS_PER_TICK);
        static CALLBACK: CountingCallback = CountingCallback::new(u64::MAX);
        static TIMER: Timer = Timer::periodic(Duration::from_micros(500), &CALLBACK);
        TIMER.start();
    }
}
