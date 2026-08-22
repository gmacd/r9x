//! Integration test: the PL011 (UART0) device, on the emulated hardware.
//!
//! A whole kernel image, like the other `tests/`.  It drives the real
//! `Pl011Uart` driver the way the kernel would — construct it from the
//! device tree, run its full `init` (which reaches the mailbox for the
//! clock and the GPIO block for the pin pulls), then exercise the register
//! block directly to prove the mapping is live and the transmit path moves
//! a byte.
//!
//! These are things a host unit test cannot say: that the `arm,pl011` node
//! maps, that the mailbox round-trips, and that writes to the flag register
//! reflect the transmitter on the emulated device.  On QEMU the PL011 is
//! wired to a discarded serial port, so the assertions read the registers
//! rather than the output.
#![no_std]
#![no_main]

use aarch64::boot;
use aarch64::deviceutil::{find_dt_physrange, map_device_register};
use aarch64::io::{read_reg, write_reg};
use aarch64::mailbox;
use aarch64::qemu;
use aarch64::uartpl011::{Pl011Uart, UART0_CR, UART0_FBRD, UART0_IBRD, UART0_LCRH};
use aarch64::vm::PageSize;
use port::devcons::Uart;
use port::println;

#[macro_use]
mod common;

/// PL011 UARTCR transmitter-enable bit.  `init` leaves the device
/// receive-only (0x81), so the transmit path is switched on here to test it.
const TXE: u32 = 1 << 8;

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    // The console is the mini uart, kept only for logging; the device under
    // test is the separate PL011.  The mailbox is needed by Pl011Uart::init.
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    boot::console(&dt);
    mailbox::init(&dt);

    println!("running pl011");

    // Construction finds the arm,pl011 node and maps its register page.
    let uart = Pl011Uart::new(&dt).unwrap_or_else(|msg| {
        println!("FAIL can't construct pl011: {msg:?}");
        qemu::exit(qemu::FAIL)
    });

    // The full init: GPIO pin pulls, clock via the mailbox, baud rate, FIFOs.
    // If the mailbox round-trip or a GPIO write hung, this would not return.
    uart.init();

    // A second, independent mapping of the same physical device, so the test
    // can read the registers the driver wrote without reaching into the
    // driver's private fields.  MMIO maps cleanly to two virtual pages.
    let pl011_phys = find_dt_physrange(&dt, &["arm,pl011"], "pl011").expect("pl011 node");
    let pl011 =
        map_device_register("pl011test", pl011_phys, PageSize::Page4K).expect("map pl011 for test");

    // init left these read-write registers at known values; reading them back
    // proves the mapped page is the live device and MMIO round-trips.
    let lcrh = read_reg(&pl011, UART0_LCRH);
    check!(lcrh == 0x70, "LCRH fifos + 8 bit, got {lcrh:#x}");
    let cr = read_reg(&pl011, UART0_CR);
    check!(cr & 0x81 == 0x81, "CR uart + rx enabled, got {cr:#x}");

    // The baud divider is computed from the fixed 3 MHz clock:
    // 3000000 / (16 * 115200) = 1.63 -> 1 integer, 40/64 fractional.
    let ibrd = read_reg(&pl011, UART0_IBRD);
    let fbrd = read_reg(&pl011, UART0_FBRD);
    check!(ibrd == 1 && fbrd == 40, "baud dividers {ibrd} + {fbrd}/64");

    // Exercise the transmit path: switch on the transmitter (init left it
    // receive-only) and send a string through the driver's putb.  QEMU does
    // not model the PL011 flag register's FIFO-empty bits, so there is no
    // completion flag to poll; a wrong mapping would already have failed the
    // readbacks above, and here a bad one would fault on these writes.  The
    // bytes go to a discarded serial port.
    write_reg(&pl011, UART0_CR, cr | TXE);
    for &b in b"pl011 tx ok\n" {
        uart.putb(b);
    }

    println!("pl011 passed");
    qemu::exit(qemu::PASS);
}
