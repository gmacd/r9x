//! Integration test: timers fire on the real interrupt path.
//!
//! The unit tests drive `interrupt_handler` against a mocked counter;
//! this image is the other half: CVAL arming, the timer PPI through
//! the GIC, trap dispatch, and the level-triggered deassert on re-arm,
//! on the emulated hardware.  It carries the assertions the old main9
//! ticker demo only let a human eyeball: a fast periodic runs until a
//! one-shot cancels it, a second periodic stops itself at its limit,
//! and everything stays stopped afterwards.
#![no_std]
#![no_main]

use aarch64::reg::cnt_el0::{CntFrqEl0, CntPctEl0};
use aarch64::timer::{Timer, TimerCallback};
use aarch64::{boot, mailbox, qemu};
use core::sync::atomic::{AtomicU32, Ordering};
use core::time::Duration;
use port::println;

#[macro_use]
mod common;

/// Counts its fires; keeps its periodic timer running until the
/// `limit`th fire (0 = run until cancelled).
struct Counter {
    fires: AtomicU32,
    limit: u32,
}

impl Counter {
    const fn new(limit: u32) -> Self {
        Self { fires: AtomicU32::new(0), limit }
    }

    fn fires(&self) -> u32 {
        self.fires.load(Ordering::Relaxed)
    }
}

impl TimerCallback for Counter {
    fn fire(&self) -> bool {
        let n = self.fires.fetch_add(1, Ordering::Relaxed) + 1;
        self.limit == 0 || n < self.limit
    }
}

/// One-shot callback that cancels another timer.
struct CancelTimer {
    victim: &'static Timer,
    fires: AtomicU32,
}

impl TimerCallback for CancelTimer {
    fn fire(&self) -> bool {
        // Cancel before counting: once main sees the fire, the victim
        // is already cancelled and its count is stable.
        self.victim.cancel();
        self.fires.fetch_add(1, Ordering::Relaxed);
        false
    }
}

static FAST: Counter = Counter::new(0);
static FAST_TIMER: Timer = Timer::periodic(Duration::from_millis(5), &FAST);

static LIMITED: Counter = Counter::new(3);
static LIMITED_TIMER: Timer = Timer::periodic(Duration::from_millis(10), &LIMITED);

static STOP_FAST: CancelTimer = CancelTimer { victim: &FAST_TIMER, fires: AtomicU32::new(0) };
static STOP_FAST_TIMER: Timer = Timer::new(Duration::from_millis(40), &STOP_FAST);

/// Spin until `cond` holds or `timeout` of counter time passes;
/// returns whether it held.  Timer interrupts arrive throughout: the
/// loop body is the image's "rest of the kernel".
fn wait_until(timeout: Duration, cond: impl Fn() -> bool) -> bool {
    let freq = CntFrqEl0::read().freq();
    let ticks = (timeout.as_nanos() * freq as u128 / 1_000_000_000) as u64;
    let deadline = CntPctEl0::read().value() + ticks;
    while CntPctEl0::read().value() < deadline {
        if cond() {
            return true;
        }
        core::hint::spin_loop();
    }
    cond()
}

/// Let `timeout` of counter time pass while interrupts keep arriving.
fn settle(timeout: Duration) {
    wait_until(timeout, || false);
}

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    // The same bring-up as user_process: vectors first, then enough of
    // the kernel for the console and the interrupt machinery.
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    mailbox::init(&dt);
    boot::console(&dt);
    boot::interrupts(&dt);

    println!("running timers");

    // Three of the table's eight slots; the table is empty, so none
    // can fail.
    FAST_TIMER.start().unwrap();
    LIMITED_TIMER.start().unwrap();
    STOP_FAST_TIMER.start().unwrap();

    // The one-shot is the last scheduled event (40ms in); by the time
    // it has fired, the fast periodic has been cancelled and the
    // limited one has long hit its limit.  The generous bound is for
    // QEMU scheduling jitter, not the timers.
    let fired = wait_until(Duration::from_secs(2), || STOP_FAST.fires.load(Ordering::Relaxed) == 1);
    check!(fired, "one-shot fired and cancelled the fast timer");

    let fast = FAST.fires();
    let limited = LIMITED.fires();
    // One fire each proves the real-hardware path (CVAL -> PPI -> GIC ->
    // trap -> callback) for a periodic and for the limit timer.  A count
    // is deliberately not asserted: fires scale with the vCPU time the
    // guest got, so on a loaded host a 5ms timer can fire once and be
    // perfectly healthy -- indistinguishable from a broken re-arm.  The
    // re-arm and self-stop logic is proven deterministically by the unit
    // tests (periodic_rearm_clamps_missed_deadlines,
    // periodic_stops_when_callback_returns_false); the quiescence check
    // below proves the deassert (no interrupt storm) on hardware.
    check!(fast >= 1, "fast periodic fired via the hardware path, {fast} fires");
    check!(limited >= 1, "limited periodic fired via the hardware path, {limited} fires");

    // Quiescence: a cancelled timer, a self-stopped timer and a fired
    // one-shot must all stay stopped.  Cancel is lazy, so the hardware
    // may wake once more, but no callback may run.
    settle(Duration::from_millis(100));
    let fast_after = FAST.fires();
    let limited_after = LIMITED.fires();
    let stop_after = STOP_FAST.fires.load(Ordering::Relaxed);
    check!(fast_after == fast, "cancelled timer stayed stopped, {fast_after} fires");
    check!(limited_after == limited, "limited timer stayed stopped, {limited_after} fires");
    check!(stop_after == 1, "one-shot did not re-fire, {stop_after} fires");

    println!("timers passed");
    qemu::exit(qemu::PASS);
}
