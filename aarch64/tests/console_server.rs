//! Integration test: user-space device MMIO ownership via SYSMAPMMIO.
//!
//! The kernel enables the PL011 UART via its own mapping (the early path),
//! then spawns the console server — a Rust-built ELF (built by xtask's
//! ServerStep, embedded here, loaded by `spawn_elf`) that calls SYSMAPMMIO to
//! map the PL011's physical register page into its own TTBR0 (Device memory
//! attributes), writes 'A' to the UART's data register, and exits 0.
//!
//! The kernel is device-dumb: it does not parse the DT for the PL011's
//! address or map it into the server's AS.  The server knows its platform
//! (0xfe201000, a BCM2711 constant) and requests the mapping itself — the
//! QNX model.

#![no_std]
#![no_main]

use aarch64::{boot, deviceutil, process, qemu, vm};
use port::fdt::DeviceTree;
use port::println;

#[macro_use]
mod common;

/// The built console server's ELF, embedded: xtask's `ServerStep` builds it
/// (static, non-PIE, linked at the shared image base), this crate's `build.rs`
/// stages it into `OUT_DIR`, and `include_bytes!` pulls the bytes in.  The
/// loader reads it through `Image::Elf` — the unified entry point the raw
/// images reach through `Image::Raw`.
static CONSOLE_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/console.elf"));

/// Enable the PL011 UART via the kernel's own mapping (the early path).
fn enable_pl011(dt: &DeviceTree) {
    let pl011_range = deviceutil::find_dt_physrange(dt, &["arm,pl011"], "can't find pl011")
        .unwrap_or_else(|e| panic!("console_server: {e}"));
    let vrange = deviceutil::map_device_register("pl011-test", pl011_range, vm::PageSize::Page4K)
        .unwrap_or_else(|e| panic!("console_server: map pl011: {e:?}"));
    // Enable UART, TX, RX (CR bits 0, 1, 4).  PL011 TRM r1p2 §3.3.
    let cr = (vrange.start + 0x30) as *mut u32;
    unsafe {
        core::ptr::write_volatile(cr, 0x31);
    }
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

    enable_pl011(&dt);
    println!("pl011 enabled (kernel side)");

    let server = process::spawn(&process::Image::Elf(CONSOLE_ELF));
    println!("server spawned, running");

    process::run_all();

    let status = process::status(server);
    println!("server status: {status:?}");
    check!(status == Some(0), "server exited 0, got {status:?}");
    println!("console-server passed");
    qemu::exit(qemu::PASS);
}
