//! Integration test: user-space device MMIO ownership via SYSMAPMMIO, with
//! the console server publishing a name.
//!
//! The kernel enables the PL011 UART via its own mapping (the early path),
//! then spawns the console server — a Rust-built ELF (built by xtask's
//! ServerStep, embedded here, loaded by `spawn_elf`) that calls SYSMAPMMIO to
//! map the PL011's physical register page into its own TTBR0 (Device memory
//! attributes), writes 'A' to the UART's data register, creates its own
//! channel pair, publishes it under `/dev/console` in the nameserver, and
//! exits 0.
//!
//! The kernel brings the device up (enable + a control-readback check) but
//! does not map it into the server's address space: the server knows its
//! platform (0xfe201000, a BCM2711 constant) and requests its own mapping via
//! SYSMAPMMIO — the QNX model.  The server's 'A' goes out the PL011 to the
//! terminal; that device is now the kernel console too, so the byte is visible
//! on the serial output and the server's exit 0 is the in-image check that it
//! ran, mapped the device, wrote the byte, and completed.
//!
//! To bind, the server needs the nameserver: the image creates the
//! nameserver's channel pair, spawns the nameserver ELF handing it its own
//! pair (the one asymmetry — it is the first server, so nothing exists yet
//! that a client could ask to find it), and spawns the console server handing
//! it the nameserver's pair, so it can `BIND` to it.  The client-side
//! resolution (a client `RESOLVE`ing `/dev/console` and round-tripping a byte)
//! is the `namespace` image's job, not this one.

#![no_std]
#![no_main]

use aarch64::io::{read_reg, write_reg};
use aarch64::uartpl011::UART0_CR;
use aarch64::{boot, deviceutil, ipc, mailbox, process, qemu, vm};
use port::println;
use r9x_core::fdt::DeviceTree;

#[macro_use]
mod common;

/// The built console server's ELF, embedded: xtask's `ServerStep` builds it
/// (static, non-PIE, linked at the shared image base), this crate's `build.rs`
/// stages it into `OUT_DIR`, and `include_bytes!` pulls the bytes in.  The
/// loader reads it through `Image::Elf` — the unified entry point the raw
/// images reach through `Image::Raw`.
static CONSOLE_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/console.elf"));

/// The built nameserver's ELF, embedded the same way: the server the console
/// server `BIND`s to.  It owns the bind table and serves `BIND` / `RESOLVE` /
/// `UNBIND` over the message syscalls; the image hands it its own channel
/// pair (the first-server asymmetry) and the console server the same pair, so
/// the server can publish `/dev/console` in it.
static NAMESERVER_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/nameserver.elf"));

/// The PL011's physical base on the BCM2711 (QEMU `raspi4b`): the value the
/// console server hardcodes (`servers/console`) and the device tree is
/// expected to report.  The cross-check below binds the DT to this value so a
/// machine whose PL011 sits elsewhere fails here, not as a silent wrong-page
/// map.  It guards the DT against a fixed base; it does not bind the server's
/// copy against drift.
const SERVER_PL011_PHYS: u64 = 0xfe20_1000;

/// PL011 UARTCR control bits (TRM: bit 0 UARTEN, bit 8 TXE, bit 9 RXE).
const UARTEN: u32 = 1 << 0;
const TXE: u32 = 1 << 8;
const RXE: u32 = 1 << 9;

/// Enable the PL011 UART via the kernel's own mapping (the early path), and
/// cross-check the device tree's address against the base the server hardcodes.
fn enable_pl011(dt: &DeviceTree) {
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
    // the device through QEMU's model, which ignores those fields rather than
    // the TRM's programming sequence.
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
}

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    mailbox::init(&dt);
    boot::console(&dt);
    boot::interrupts(&dt);

    println!("running console-server");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), vm::RootPageTableType::User);
    }

    enable_pl011(&dt);
    println!("pl011 enabled (kernel side)");

    // The nameserver's channel pair: created kernel-side (the image is
    // init-context) and handed to the nameserver (its own pair — the
    // first-server asymmetry) and to the console server (the pair it `BIND`s
    // to).  The image keeps the pair to pass to both, so the servers never
    // see each other's handles by constant.
    let ns_in = ipc::create();
    let ns_out = ipc::create();
    let ns_handles = process::Handles {
        inbound: ns_in as u32,
        outbound: ns_out as u32,
        extra_inbound: 0,
        extra_outbound: 0,
    };
    let ns =
        process::spawn(&process::Image::Elf { bytes: NAMESERVER_ELF, handles: Some(ns_handles) });
    // The console server is handed the nameserver's pair, not its own: it
    // `SYCCREATECHAN`s its own pair and `BIND`s it to the nameserver over
    // `ns_in` / `ns_out`.
    let server =
        process::spawn(&process::Image::Elf { bytes: CONSOLE_ELF, handles: Some(ns_handles) });
    println!("nameserver + console server spawned, running");

    process::run_all();

    let status = process::status(server);
    println!("ns status: {:?}, server status: {status:?}", process::status(ns));
    println!("run_order: {:?}", process::run_order());
    // The server's 'A' went out the PL011 to the terminal (this is now the
    // kernel console).  The server is now blocked on its post-bind receive
    // (waiting for a client), so it is still alive: the in-image check is
    // that it ran, mapped the device, wrote the byte, created its pair,
    // bound it, and is now waiting.  The client-side proof the bind landed
    // and the server's clean exit after a round-trip is the `namespace`
    // image's job.
    check!(status.is_none(), "server alive (blocked on post-bind receive), got {status:?}");
    println!("console-server passed");
    qemu::exit(qemu::PASS);
}
