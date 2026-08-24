//! Integration test: user-space device MMIO ownership via SYSMAPMMIO.
//!
//! The kernel enables the PL011 UART via its own mapping (the early path),
//! then spawns a "console server" process.  The server calls SYSMAPMMIO to
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

/// The server's text: 10 instructions (40 bytes).
///
/// ```asm
///   MOVZ X0, #0x1000              // PL011 PA low
///   MOVK X0, #0xfe20, LSL #16     // PL011 PA = 0xfe201000
///   MOVZ X1, #0x200, LSL #16      // user VA = 0x20000
///   MOV  X8, #20                  // SYSMAPMMIO
///   SVC  #0                       // map the MMIO
///   MOVZ X9, #0x200, LSL #16      // MMIO base
///   MOV  W1, #65                  // 'A'
///   STR  W1, [X9, #0x00]         // write DR (FIFO is empty on first write)
///   MOV  X8, #0                   // exit(0)
///   SVC  #0
/// ```
const SERVER_TEXT: [u8; 40] = [
    // MOVZ X0, #0x1000
    0x00, 0x00, 0x82, 0xd2, // MOVK X0, #0xfe20, LSL #16
    0x00, 0xc4, 0xdf, 0xf2, // MOVZ X1, #0x200, LSL #16
    0x01, 0x40, 0xa0, 0xd2, // MOV X8, #20 (SYSMAPMMIO)
    0x88, 0x02, 0x80, 0xd2, // SVC #0
    0x01, 0x00, 0x00, 0xd4, // MOVZ X9, #0x200, LSL #16
    0x09, 0x40, 0xa0, 0xd2, // MOV W1, #65 ('A')
    0x21, 0x08, 0x80, 0xd2, // STR W1, [X9, #0x00]
    0x21, 0x01, 0x00, 0x30, // MOV X8, #0 (exit)
    0x08, 0x00, 0x80, 0xd2, // SVC #0
    0x01, 0x00, 0x00, 0xd4,
];

const SERVER_TEXT_VA: usize = 0x1000;
const SERVER_STACK_VA: usize = 0x10000;

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

    let server = process::spawn(&process::Image::Raw {
        text: &SERVER_TEXT,
        text_va: SERVER_TEXT_VA,
        stack_va: SERVER_STACK_VA,
    });
    println!("server spawned, running");

    process::run_all();

    let status = process::status(server);
    println!("server status: {status:?}");
    check!(status == Some(0), "server exited 0, got {status:?}");
    println!("console-server passed");
    qemu::exit(qemu::PASS);
}
