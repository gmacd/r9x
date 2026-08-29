use crate::Result;
use crate::irq::IrqGuard;
use crate::mcslock::{Lock, LockNode};
use crate::once::Once;
use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};

const fn ctrl(b: u8) -> u8 {
    b - b'@'
}

#[allow(dead_code)]
const BACKSPACE: u8 = ctrl(b'H');
#[allow(dead_code)]
const DELETE: u8 = 0x7F;
#[allow(dead_code)]
const CTLD: u8 = ctrl(b'D');
#[allow(dead_code)]
const CTLP: u8 = ctrl(b'P');
#[allow(dead_code)]
const CTLU: u8 = ctrl(b'U');

pub trait Uart {
    fn putb(&self, b: u8);
}

static CONS: Lock<Option<&'static dyn Uart>> = Lock::new("cons", None);

/// Once the console server is up, the kernel stops using the PL011 for normal
/// output.  This flag gates the `println!` path (the locked `Console`);
/// the `iprint` path (direct polled write, no lock) is the debug backstop and
/// is never gated.  Set once during bringup, never cleared (one-way): the
/// console server does not die in this design (no death hook; stage 7's
/// concern if that changes).
static CONSOLE_LIVE: AtomicBool = AtomicBool::new(false);

/// Set the console-live gate: from this point on, `println!` output is
/// dropped (the console server owns the UART for normal output).  The
/// `iprint` path is unaffected.  Call once during bringup, after the console
/// server has been spawned.
pub fn set_console_live() {
    CONSOLE_LIVE.store(true, Ordering::Release);
}

/// Console is what should be used in almost all cases, as it ensures threadsafe
/// use of the console.
pub struct Console;

impl Console {
    pub fn set_uart<F>(uart_fn: F)
    where
        F: FnOnce() -> Result<&'static dyn Uart>,
    {
        let node = LockNode::new();
        let mut cons = CONS.lock(&node);
        *cons = uart_fn().ok();
    }

    pub fn putstr(&mut self, s: &str) {
        // The console lock is thread-context only; interrupt context
        // must use iprint, which bypasses it.
        debug_assert!(!crate::irq::in_interrupt(), "println in interrupt context; use iprintln");

        // Once the console server is up, normal kernel output is dropped:
        // the server owns the UART for user-facing text.  The `iprint` path
        // (direct polled write, no lock) is the debug backstop and is never
        // gated.
        if CONSOLE_LIVE.load(Ordering::Acquire) {
            return;
        }

        let node = LockNode::new();
        let uart_guard = CONS.lock(&node);
        if let Some(uart) = *uart_guard {
            for b in s.bytes() {
                putb(uart, b);
            }
        }
    }
}

impl fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.putstr(s);
        Ok(())
    }
}

pub fn print(args: fmt::Arguments) {
    // XXX: Just for testing.
    use fmt::Write;
    let mut cons: Console = Console {};
    cons.write_fmt(args).unwrap();
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

#[macro_export]
macro_rules! print {
    ($($args:tt)*) => {{
        $crate::devcons::print(format_args!($($args)*))
    }};
}

/// Log at boot whether this release image carries the extra checks that
/// `[profile.release]` currently enables (task 102): while the kernel is
/// young, a release build keeps debug-assertions and overflow-checks on.
/// Gated on `release_profile` (a marker cfg the build sets for release
/// images) and `debug_assertions` — `overflow-checks` is a codegen option,
/// not a cfg, so it is named in the text rather than tested.  The line is
/// the reminder to drop those profile overrides once speed is actually
/// wanted — it compiles away the day `debug-assertions` does.
pub fn release_checks_note() {
    #[cfg(all(release_profile, debug_assertions))]
    {
        println!(
            "release image: debug-assertions + overflow-checks enabled (task 102); drop from [profile.release] once speed is measured"
        );
    }
}

fn putb(uart: &dyn Uart, b: u8) {
    if b == b'\n' {
        uart.putb(b'\r');
    } else if b == BACKSPACE {
        uart.putb(b);
        uart.putb(b' ');
    }
    uart.putb(b);
}

// iprint: the interrupt- and panic-safe print, in the tradition of
// Plan 9's iprint.  It masks IRQs, takes only a best-effort interlock,
// and writes polled bytes directly to the hardware, bypassing the
// console lock — so it works in interrupt context, in panic, and while
// debugging the console or locks themselves.  Output may interleave
// with a concurrent print; that is the accepted price of never
// blocking.

/// Direct console byte writer, registered by arch code at boot (same
/// pattern as `irq::set_ops`).  Must be polled and lock-free: it is
/// called with no locks held from any context.
pub struct IprintOps {
    pub putb: fn(u8),
}

static IPRINT_OPS: Once<&'static IprintOps> = Once::new();

/// Register the direct console writer.  Call at boot once the console
/// hardware is initialised; until then iprint output is dropped.
pub fn set_iprint_ops(ops: &'static IprintOps) {
    let _ = IPRINT_OPS.set(ops);
}

fn iprint_ops() -> Option<&'static IprintOps> {
    IPRINT_OPS.get().copied()
}

/// Best-effort interlock so concurrent iprints don't interleave.
/// Never required for correctness — see `iprint_trylock`.
static IPRINT_LOCK: AtomicBool = AtomicBool::new(false);

/// Try to take the interlock, giving up after a bounded spin: if
/// another core holds it too long, print anyway — interleaved output
/// beats a silent core.  A same-core holder is impossible, as the lock
/// is only ever held with IRQs masked.
fn iprint_trylock() -> bool {
    for _ in 0..1_000_000 {
        if IPRINT_LOCK.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

struct IprintWriter {
    putb: fn(u8),
}

impl fmt::Write for IprintWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            if b == b'\n' {
                (self.putb)(b'\r');
            }
            (self.putb)(b);
        }
        Ok(())
    }
}

pub fn iprint(args: fmt::Arguments) {
    let _irq = IrqGuard::new();
    let Some(ops) = iprint_ops() else {
        return;
    };
    let locked = iprint_trylock();
    let _ = fmt::Write::write_fmt(&mut IprintWriter { putb: ops.putb }, args);
    if locked {
        IPRINT_LOCK.store(false, Ordering::Release);
    }
}

#[macro_export]
macro_rules! iprintln {
    () => ($crate::iprint!("\n"));
    ($($arg:tt)*) => ($crate::iprint!("{}\n", format_args!($($arg)*)));
}

#[macro_export]
macro_rules! iprint {
    ($($args:tt)*) => {{
        $crate::devcons::iprint(format_args!($($args)*))
    }};
}
