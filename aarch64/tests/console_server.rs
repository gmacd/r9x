//! Integration test: user-space device MMIO ownership via SYSMAPMMIO.
//!
//! The kernel enables the PL011 UART via its own mapping (the early path),
//! then spawns the console server — a Rust-built ELF (built by xtask's
//! ServerStep, embedded here, loaded by `spawn_elf`) that calls SYSMAPMMIO to
//! map the PL011's physical register page into its own TTBR0 (Device memory
//! attributes), writes 'A' to the UART's data register, and exits 0.
//!
//! The kernel brings the device up and uses it as a measuring instrument
//! (enable, loopback, RX readback) but does not map it into the server's
//! address space: the server knows its platform (0xfe201000, a BCM2711
//! constant) and requests its own mapping via SYSMAPMMIO — the QNX model.

#![no_std]
#![no_main]

use aarch64::io::{read_reg, write_reg};
use aarch64::uartpl011::{UART0_CR, UART0_DR};
use aarch64::{boot, deviceutil, process, qemu, vm};
use port::fdt::DeviceTree;
use port::mem::VirtRange;
use port::println;

#[macro_use]
mod common;

/// The built console server's ELF, embedded: xtask's `ServerStep` builds it
/// (static, non-PIE, linked at the shared image base), this crate's `build.rs`
/// stages it into `OUT_DIR`, and `include_bytes!` pulls the bytes in.  The
/// loader reads it through `Image::Elf` — the unified entry point the raw
/// images reach through `Image::Raw`.
static CONSOLE_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/console.elf"));

/// The PL011's physical base on the BCM2711 (QEMU `raspi4b`): the value the
/// console server hardcodes (`servers/console`) and the device tree is
/// expected to report.  The cross-check below binds the DT to this value so a
/// machine whose PL011 sits elsewhere fails here, not as a silent wrong-page
/// map.  It guards the DT against a fixed base; it does not bind the server's
/// copy against drift.
const SERVER_PL011_PHYS: u64 = 0xfe20_1000;

/// PL011 UARTCR control bits (TRM: bit 0 UARTEN, bit 7 LBE loopback, bit 8
/// TXE, bit 9 RXE).
const UARTEN: u32 = 1 << 0;
const LBE: u32 = 1 << 7;
const TXE: u32 = 1 << 8;
const RXE: u32 = 1 << 9;

/// Enable the PL011 UART via the kernel's own mapping (the early path), and
/// cross-check the device tree's address against the base the server hardcodes.
/// Returns the kernel's mapping of the PL011's register page so the image can
/// drive the device (loopback, RX readback) around the server's run.
fn enable_pl011(dt: &DeviceTree) -> VirtRange {
    let pl011_range = deviceutil::find_dt_physrange(dt, &["arm,pl011"], "can't find pl011")
        .unwrap_or_else(|e| panic!("console_server: {e}"));
    // The server hardcodes this base (a BCM2711 constant it knows, not
    // something the kernel looks up); a device tree that disagrees means the
    // server maps the wrong page.  Cross-check where the failure is
    // actionable, not as a silent pass.
    check!(
        pl011_range.start.addr() == SERVER_PL011_PHYS,
        "device tree PL011 base {:#x} matches the server's {:#x}",
        pl011_range.start.addr(),
        SERVER_PL011_PHYS
    );
    let vrange = deviceutil::map_device_register("pl011-test", pl011_range, vm::PageSize::Page4K)
        .unwrap_or_else(|e| panic!("console_server: map pl011: {e:?}"));
    // Enable UART, TX, RX.  Read-modify-write so the enable doesn't clobber
    // other control bits (the in-pattern idiom, pl011.rs).  The line format
    // (LCRH) and baud (IBRD/FBRD) are left at reset — invalid on real
    // hardware (5-bit words, no baud clock) — because this image exercises
    // the device through QEMU's model, which honours LBE and ignores those
    // fields rather than the TRM's programming sequence.
    let cr = read_reg(&vrange, UART0_CR);
    write_reg(&vrange, UART0_CR, cr | UARTEN | TXE | RXE);
    // Read the control register back so an enable failure is caught here,
    // before a process is spawned.  It confirms the enable landed on a live
    // mapping; a round-trip does not discriminate the mapping's memory
    // attributes, and it says nothing about the server's independent
    // SYSMAPMMIO mapping of the same page.
    let cr = read_reg(&vrange, UART0_CR);
    check!(
        cr & (UARTEN | TXE | RXE) == (UARTEN | TXE | RXE),
        "CR has UARTEN+TXE+RXE after enable, got {cr:#x}"
    );
    vrange
}

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    boot::console(&dt);
    boot::interrupts(&dt);

    println!("running console-server");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), vm::RootPageTableType::User);
    }

    let pl011 = enable_pl011(&dt);
    println!("pl011 enabled (kernel side)");

    let server = process::spawn(&process::Image::Elf(CONSOLE_ELF));
    println!("server spawned, running");

    // Switch the PL011 into loopback so the server's 'A' (written to the TX
    // data register) is routed by the device into this image's RX path
    // instead of the wire — the only in-image observable that the server's
    // write reached the device.  (The kernel's console is the MiniUart, a
    // different device, so no kernel byte can contend for this one.)
    let cr = read_reg(&pl011, UART0_CR);
    write_reg(&pl011, UART0_CR, cr | LBE);

    process::run_all();

    let cr = read_reg(&pl011, UART0_CR);
    write_reg(&pl011, UART0_CR, cr & !LBE);

    let status = process::status(server);
    println!("server status: {status:?}");
    // The process assertion comes first: if the server faulted before its DR
    // write, name that rather than a spurious empty-RX reading.
    check!(status == Some(0), "server exited 0, got {status:?}");

    // The server's 'A' is now in the RX path, looped back from its TX.  Mask
    // to the data bits: DR's upper bits are the overrun/framing error flags.
    let dr = read_reg(&pl011, UART0_DR);
    check!(dr & 0xff == b'A' as u32, "server's 'A' looped back to RX, got {dr:#x}");
    println!("console-server passed");
    qemu::exit(qemu::PASS);
}
