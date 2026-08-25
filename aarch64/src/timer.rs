//! Minimal timer subsystem using the ARMv8 architectural timer.
//!
//! Timers are caller-owned: the subsystem stores only `&'static`
//! references in a small fixed table, never allocates, and the
//! interrupt handler frees nothing.  Timers therefore live in statics,
//! with interior mutability making them shareable with the handler.
//!
//! The PPI's INTID is not a constant: it is parsed from the devicetree
//! at init (`timer_intid_from_dt`), because the number is a property of
//! the machine's interrupt wiring, not of the kernel.

use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use core::time::Duration;

use port::Result;
use port::irq::IrqGuard;
use port::mcslock::{Lock, LockNode};
use port::once::Once;
use r9x_core::fdt::DeviceTree;

use crate::gic;
use crate::reg::cnt_el0::{CntFrqEl0, CntKctlEl1, CntPctEl0, CntpCtlEl0, CntpCvalEl0};

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

    /// Register and arm the timer.  Fails if the timer table is full —
    /// eight *active* timers, since a slot whose timer is no longer
    /// active is reclaimed — and panics if a periodic timer's period is
    /// shorter than one counter tick.
    pub fn start(&'static self) -> Result<()> {
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
        if let Err(e) = register(self) {
            // Leave the timer as it was found: not in the table,
            // inactive.  IRQs are masked, so nothing observes the
            // transient state.
            self.active.store(false, Ordering::Release);
            return Err(e);
        }
        arm_hardware();
        Ok(())
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

fn register(timer: &'static Timer) -> Result<()> {
    with_timers(|timers| {
        // Already registered (a restart)?
        if timers.iter().flatten().any(|t| ptr::eq(*t, timer)) {
            return Ok(());
        }
        // Take a free slot, or one whose timer is no longer active.
        for slot in timers.iter_mut() {
            if slot.is_none_or(|t| !t.active.load(Ordering::Acquire)) {
                *slot = Some(timer);
                return Ok(());
            }
        }
        Err("timer table full: eight active timers")
    })
}

/// Arm the hardware for the earliest active deadline, or disarm if
/// there is none.  Call with IRQs masked.
///
/// `TIMERS` is one table shared by every core, but `CNTP_CVAL_EL0` has a
/// separate copy in each PE (Arm ARM DDI 0487, CNTP_CVAL_EL0): each core
/// writes its own, so after any second core has run this, every core holds
/// the same global minimum deadline in its own CVAL, and each core's timer
/// fires its own PPI when that deadline is reached.  `now` is not banked
/// (CNTPCT_EL0 is one global count), so every core compares the same
/// deadline against the same time and all of them see the expiry as due.
/// Per-core CVALs are the architecture, not an oversight
/// (gic-timer-review-nits.md #6 asked for this to be written down; it is
/// now).
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

/// The INTID of the timer PPI on this machine, published by `init`
/// from the devicetree (`timer_intid_from_dt`).  IRQs are masked until
/// after init, so the first read is always after the publish.
static INTID: Once<u16> = Once::new();

/// The INTID of the timer PPI on this machine.
pub fn intid() -> u16 {
    *INTID.get().expect("timer: not initialised")
}

/// Parse the INTID of the EL1 non-secure physical timer PPI from the
/// devicetree.
///
/// The generic-timer PPIs sit at the low end of the PPI range by Arm
/// Base System Architecture convention (SBSA; the GIC-400 TRM agrees):
/// INTID 26 = non-secure EL2 physical, 27 = virtual (CNTV), 28 = EL2
/// virtual (VHE), 29 = secure EL1 physical (CNTPS), 30 = non-secure
/// EL1 physical (CNTP); DT PPI numbers map to INTIDs as 16 + n.  The
/// convention — not the architecture — is what makes the number a
/// property of the machine's wiring, which is why it is parsed rather
/// than assumed.  The `arm,armv8-timer` node's `interrupts` list is a
/// series of (type, PPI, flags) triplets ordered [0] EL1 secure
/// physical, [1] EL1 non-secure physical, [2] EL1 virtual, [3] EL2
/// physical (`arm,arch_timer.yaml`).  r9 guarantees non-secure EL1
/// handoff on every supported target (QEMU `virt` boots a 64-bit guest
/// at EL1 non-secure; the BCM firmware hands the OS off at EL1
/// non-secure) and arms the CNTP, so it takes entry [1].
///
/// The positional read of entry [1] assumes the secure entry occupies
/// [0]; a list that names its entries (`interrupt-names`, used by some
/// hypervisor-generated DTs) is a different convention and is refused
/// rather than guessed at.
///
/// Verified on both supported machines: QEMU virt (live DTB) and
/// bcm2711 (Pi 4) list PPI 14 at entry [1] — INTID 30 — and on QEMU
/// virt arming CNTP in fact delivers INTID 30.
///
/// A board whose timer node is `arm,armv7-timer` parented to a local
/// interrupt controller (the Pi 3's bcm2837: its timer PPIs are
/// local-intc IRQs, not GIC PPIs) has no `arm,armv8-timer` node and
/// fails here.  That is the loud way to say the board is out of scope.
fn timer_intid_from_dt(dt: &DeviceTree) -> Result<u16> {
    let node = dt.find_compatible("arm,armv8-timer").next().ok_or(
        "no arm,armv8-timer node in devicetree: the timer PPI is not GIC-routed (on the Pi 3's bcm2837 it goes through the local interrupt controller; the Pi 3 is out of scope — see AGENTS.md)",
    )?;
    if dt.property(&node, "interrupt-names").is_some() {
        return Err(
            "arm,armv8-timer node has interrupt-names; r9 takes the positional entry [1] and does not parse named lists",
        );
    }
    let prop = dt
        .property(&node, "interrupts")
        .ok_or("arm,armv8-timer node has no interrupts property")?;
    let cells = dt.property_value_as_u32_iter(&prop);
    // Entry [1]'s PPI is the 5th cell: triplet [0] is cells 0-2,
    // triplet [1] is cells 3-5, the PPI is cell 4.  Cell 3 must be the
    // GIC_PPI type (1); anything else means the specifiers are not the
    // 3-cell GIC form this positional read assumes.
    let mut ppi = None;
    for (i, cell) in cells.enumerate() {
        match i {
            3 => {
                if cell != 1 {
                    return Err("arm,armv8-timer interrupts entry [1] is not a GIC PPI specifier");
                }
            }
            4 => ppi = Some(cell),
            _ => {}
        }
    }
    let ppi =
        ppi.ok_or("arm,armv8-timer interrupts list is too short for the non-secure EL1 PPI")?;
    if ppi > 15 {
        return Err("arm,armv8-timer interrupts entry [1] is not a local PPI number");
    }
    // ppi <= 15, so 16 + ppi fits a u16.
    Ok(16 + ppi as u16)
}

/// Initialise the timer subsystem.  Requires `gic::init`: delivery of
/// the PPI is enabled at the distributor as the last step, and only
/// after the hardware is disarmed, so an inherited armed timer cannot
/// be admitted pending.
pub fn init(dt: &DeviceTree) {
    let intid = timer_intid_from_dt(dt).unwrap_or_else(|msg| {
        panic!("timer: {msg:?}: refusing to boot without a GIC-routed timer")
    });
    let Ok(intid) = INTID.set(intid) else {
        panic!("timer: initialised twice");
    };

    // duration_to_ticks re-checks this on every start; checking here
    // too fails at boot rather than at the first timer.
    if CntFrqEl0::read().freq() == 0 {
        panic!("timer: CNTFRQ_EL0=0: counter frequency not programmed by firmware");
    }

    // Opt EL0 into reading the counter and its frequency: a process
    // that can pace itself against the real clock (the preempt image
    // does) instead of against a machine-speed-dependent instruction
    // count.  EL0 access to the timer stays trapped.
    CntKctlEl1::el0_counter_access().write();

    timer_disable();
    gic::enable_interrupt(*intid);
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
        // The due-check is a claim, not a load: once two cores each arm
        // their banked CVAL to the global minimum (see `arm_hardware`),
        // both take the PPI at the same expiry and both see it as due.  The
        // re-arm in step 3 runs on every core and deasserts each one's own
        // PPI, so only the *fire* must be singular: exactly one core claims
        // the expiry and fires, the rest skip.  One expiry thus yields one
        // `fire()` and one deadline advance.
        if !timer.repeat {
            // A one-shot claims by clearing `active`: the core whose swap
            // finds it set owns the fire.  Clearing before firing also
            // lets the callback restart the timer.
            if timer.active.swap(false, Ordering::AcqRel) {
                timer.callback.fire();
            }
        } else {
            // A periodic timer claims by advancing the deadline: the core
            // whose compare-exchange lands owns the fire, and the loser
            // sees a deadline in the future and skips.  `next` is the
            // clamp, computed from the *claimed* deadline (the value the
            // exchange succeeded on), never a stale separately-loaded one.
            // If the callback ran longer than a period the clamp skips the
            // missed expiries rather than re-firing in a storm.  `start`
            // guarantees a periodic period is at least one tick, so the
            // division is safe.
            let period = timer.period_ticks.load(Ordering::Relaxed);
            let next = deadline + ((now - deadline) / period + 1) * period;
            let claimed = timer
                .deadline_ticks
                .compare_exchange(deadline, next, Ordering::Acquire, Ordering::Relaxed)
                .is_ok();
            if !claimed {
                continue;
            }
            if !timer.callback.fire() {
                timer.active.store(false, Ordering::Release);
            }
        }
    }

    // 3. Arm next timer or disarm, deasserting the interrupt.
    arm_hardware();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reg::cnt_el0::{mock_kctl, set_mock_count, set_mock_freq, set_mock_kctl};
    use r9x_core::fdt::DeviceTree;
    use std::sync::{Mutex, MutexGuard};

    /// The Pi 4's DTB — the same file the aarch64 QEMU run uses —
    /// carries the `arm,armv8-timer` node `init` parses.
    static RPI4_DTB: &[u8] = include_bytes!("../lib/bcm2711-rpi-4-b.dtb");

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
        set_mock_kctl(0);
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
        TIMER.start().unwrap();
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
        TIMER.start().unwrap();

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
        TIMER.start().unwrap();

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
        TIMER.start().unwrap();
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
                    TIMER.start().unwrap();
                }
                false
            }
        }
        static CALLBACK: RestartOnce = RestartOnce { fires: AtomicU64::new(0) };
        static TIMER: Timer = Timer::new(Duration::from_millis(10), &CALLBACK);

        TIMER.start().unwrap();
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

    /// A full table is eight *active* timers: a slot whose timer is no
    /// longer active is reclaimed, and a start that fails leaves its
    /// timer out of the table and inactive.
    #[test]
    fn a_full_table_rejects_the_ninth_active_timer() {
        let _guard = reset(FREQ_1MS_PER_TICK);
        static CALLBACK: CountingCallback = CountingCallback::new(u64::MAX);
        static T0: Timer = Timer::new(Duration::from_millis(100), &CALLBACK);
        static T1: Timer = Timer::new(Duration::from_millis(101), &CALLBACK);
        static T2: Timer = Timer::new(Duration::from_millis(102), &CALLBACK);
        static T3: Timer = Timer::new(Duration::from_millis(103), &CALLBACK);
        static T4: Timer = Timer::new(Duration::from_millis(104), &CALLBACK);
        static T5: Timer = Timer::new(Duration::from_millis(105), &CALLBACK);
        static T6: Timer = Timer::new(Duration::from_millis(106), &CALLBACK);
        static T7: Timer = Timer::new(Duration::from_millis(107), &CALLBACK);
        static NINTH: Timer = Timer::new(Duration::from_millis(110), &CALLBACK);

        T0.start().unwrap();
        T1.start().unwrap();
        T2.start().unwrap();
        T3.start().unwrap();
        T4.start().unwrap();
        T5.start().unwrap();
        T6.start().unwrap();
        T7.start().unwrap();
        assert!(NINTH.start().is_err());
        // The failed start left the timer as it was found.
        assert!(!NINTH.active.load(Ordering::Acquire));

        // Reclaim one slot and the ninth fits.
        T0.cancel();
        NINTH.start().unwrap();
        assert!(NINTH.active.load(Ordering::Acquire));
    }

    #[test]
    fn init_parses_the_timer_intid_and_opts_el0_into_the_counter() {
        let _guard = reset(FREQ_1MS_PER_TICK);
        assert_eq!(mock_kctl(), 0, "reset clears the opt-in");
        let dt = DeviceTree::new(RPI4_DTB).unwrap();
        init(&dt);
        // The Pi 4's DTB lists PPI 14 at entry [1]: INTID 30.
        assert_eq!(intid(), 30);
        assert_eq!(
            mock_kctl(),
            CntKctlEl1::el0_counter_access().bits(),
            "init enables EL0 reads of CNTPCT_EL0 and CNTFRQ_EL0, nothing else"
        );
    }

    #[test]
    #[should_panic(expected = "CNTFRQ_EL0=0")]
    fn start_with_unprogrammed_frequency_panics() {
        let _guard = reset(0);
        static CALLBACK: CountingCallback = CountingCallback::new(u64::MAX);
        static TIMER: Timer = Timer::periodic(Duration::from_millis(10), &CALLBACK);
        TIMER.start().unwrap();
    }

    #[test]
    #[should_panic(expected = "shorter than one counter tick")]
    fn sub_tick_periodic_period_panics() {
        let _guard = reset(FREQ_1MS_PER_TICK);
        static CALLBACK: CountingCallback = CountingCallback::new(u64::MAX);
        static TIMER: Timer = Timer::periodic(Duration::from_micros(500), &CALLBACK);
        TIMER.start().unwrap();
    }
}
