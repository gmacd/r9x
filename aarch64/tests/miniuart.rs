//! Integration test: the mini uart (UART1, the aux PL016) device, on the
//! emulated hardware.
//!
//! A whole kernel image, like the other `tests/`.  The mini uart is the
//! console this kernel uses on the Raspberry Pi, but nothing else exercises
//! it as a device: this image constructs one directly, runs `init`, and
//! proves the aux + uart register blocks are live and the transmit path
//! moves a byte.
//!
//! The mini uart's fields are public, so the test reads the registers the
//! driver wrote through the very ranges the driver holds — no second mapping
//! needed.  (The PL011 test maps its own, because those fields are private.)
#![no_std]
#![no_main]

use aarch64::boot;
use aarch64::io::read_reg;
use aarch64::mailbox;
use aarch64::qemu;
use aarch64::uartmini::{AUX_ENABLE, AUX_MU_CNTL, AUX_MU_LSR, MiniUart};
use port::devcons::Uart;
use port::println;

#[macro_use]
mod common;

/// AUX_MU_LSR transmit-empty bit: set once the last byte has left the FIFO.
const TE: u32 = 1 << 5;
/// AUX_MU_LSR data-ready bit: set when a received byte is waiting.
const DR: u32 = 1 << 0;

/// Spin until `cond` holds or `deadline` cycles pass; returns whether it did.
fn poll_until(deadline: u32, mut cond: impl FnMut() -> bool) -> bool {
    for _ in 0..deadline {
        if cond() {
            return true;
        }
        core::hint::spin_loop();
    }
    cond()
}

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    // Bring up the console (itself a mini uart) for logging; the device under
    // test is a separately constructed instance of the same hardware.
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    mailbox::init(&dt);
    boot::console(&dt);

    println!("running miniuart");

    // Construction finds the gpio, aux and aux-uart nodes and maps them all.
    let uart = MiniUart::new_with_map_ranges(&dt).unwrap_or_else(|msg| {
        println!("FAIL can't construct miniuart: {msg:?}");
        qemu::exit(qemu::FAIL)
    });

    // Full init: GPIO mux + pulls, aux enable, line format, baud, enable tx.
    uart.init();

    // Reading the registers back through the driver's own ranges proves the
    // mappings are live and MMIO round-trips.  The QEMU aux model does not
    // write back the line-control or baud registers (they read 0), so assert
    // the ones it does: aux enabled, and the transmitter/receiver turned on,
    // which is init's final write to the control register.
    let aux_en = read_reg(&uart.aux_virtrange, AUX_ENABLE);
    check!(aux_en & 1 != 0, "aux enabled, got {aux_en:#x}");
    let cntl = read_reg(&uart.miniuart_virtrange, AUX_MU_CNTL);
    check!(cntl == 3, "tx + rx enabled, got {cntl:#x}");

    // Nothing is being sent in, so there is no data waiting to read.
    check!(read_reg(&uart.miniuart_virtrange, AUX_MU_LSR) & DR == 0, "no spurious rx data");

    // Exercise the transmit path through the driver's putb, then wait for the
    // transmit-empty flag: the last byte has left the FIFO.
    for &b in b"miniuart tx ok\n" {
        uart.putb(b);
    }
    let te = poll_until(10_000_000, || read_reg(&uart.miniuart_virtrange, AUX_MU_LSR) & TE != 0);
    check!(te, "transmit empty after sending");

    println!("miniuart passed");
    qemu::exit(qemu::PASS);
}
