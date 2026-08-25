//! The kernel binary: the boot sequence, and nothing else.  Everything it
//! calls lives in the `aarch64` library, so that integration tests can link
//! the same code and run a shorter sequence of their own.
#![allow(clippy::too_many_arguments)]
#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(not(test), no_main)]

use aarch64::kmem::{
    boottext_physrange, bss_physrange, data_physrange, rodata_physrange, text_physrange,
    total_kernel_physrange,
};
use aarch64::vm::RootPageTableType;
use aarch64::{boot, ipc, mailbox, pagealloc, process, vm};
use port::mem::{PhysRange, VirtRange};
use port::println;

fn print_memory_range(name: &str, range: &PhysRange) {
    let size = range.size();
    println!("  {name}{range} ({size:#x})");
}

fn print_binary_sections() {
    println!("Binary sections:");
    print_memory_range("boottext:\t", &boottext_physrange());
    print_memory_range("text:\t\t", &text_physrange());
    print_memory_range("rodata:\t", &rodata_physrange());
    print_memory_range("data:\t\t", &data_physrange());
    print_memory_range("bss:\t\t", &bss_physrange());
    print_memory_range("total:\t", &total_kernel_physrange());
}

fn print_memory_info() {
    println!("Memory usage:");
    let (used, total) = pagealloc::usage_bytes();
    println!("  Used:\t\t{used:#016x}");
    println!("  Total:\t{total:#016x}");
}

// https://github.com/raspberrypi/documentation/blob/develop/documentation/asciidoc/computers/raspberry-pi/revision-codes.adoc
fn print_pi_name(board_revision: u32) {
    let name = match board_revision {
        0xa21041 => "Raspberry Pi 2B",
        0xa02082 => "Raspberry Pi 3B",
        0xb03115 => "Raspberry Pi 4B",
        0xa220a0 => "Raspberry Compute Module 3",
        _ => "Unrecognised",
    };
    println!("  Board Name:\t{name}");
}

fn print_board_info() {
    println!("Board information:");
    let board_revision = mailbox::get_board_revision();
    print_pi_name(board_revision);
    println!("  Board Rev:\t{board_revision:#010x}");
    let model = mailbox::get_board_model();
    println!("  Board Model:\t{model:#010x}");
    let serial = mailbox::get_board_serial();
    println!("  Serial Num:\t{serial:#010x}");
    let mailbox::MacAddress { a, b, c, d, e, f } = mailbox::get_board_macaddr();
    println!("  MAC Address:\t{a:02x}:{b:02x}:{c:02x}:{d:02x}:{e:02x}:{f:02x}");
    let fw_revision = mailbox::get_firmware_revision();
    println!("  Firmware Rev:\t{fw_revision:#010x}");
}

fn print_stacks() {
    unsafe extern "C" {
        static interruptstackbase: [u64; 0];
        static interruptstacksz: [u64; 0];
    }

    let interrupt_stack_base = unsafe { interruptstackbase.as_ptr().addr() };
    let interrupt_stack_max = interrupt_stack_base + unsafe { interruptstacksz.as_ptr().addr() };
    let range = VirtRange::new(interrupt_stack_base, interrupt_stack_max);
    let range_size = range.size();
    println!("Interrupt stack:{range} ({range_size:#x})");
}

// The user-space server ELFs, staged into OUT_DIR by build.rs.
static CONSOLE_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/console.elf"));
static NAMESERVER_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/nameserver.elf"));
static INIT_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/init.elf"));

/// dtb_va is the virtual address of the DTB structure.  The physical address is
/// assumed to be dtb_va-KZERO.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn main9(dtb_va: usize) {
    boot::irq_ops();

    // Parse the DTB before we set up memory so we can correctly map it
    let dt = unsafe { boot::device_tree(dtb_va) };
    if let Err(err) = boot::page_allocator(&dt, dtb_va) {
        panic!("couldn't init page allocator: {err:?}");
    }

    // The mailbox is a separate device, up before the console: the PL011
    // console needs it for its clock, and the board info printed below reads
    // it too.
    mailbox::init(&dt);
    boot::console(&dt);

    // Interrupt bringup, in a required order:
    //   1. gic::init enables the distributor and this core's CPU
    //      interface.  boot::irq_ops (above, before the MMU work) must
    //      already have registered the DAIF ops IrqGuard needs.
    //   2. timer::init disarms CNTP_CTL_EL0 and only then enables the
    //      timer PPI at the distributor, so a firmware-armed timer
    //      cannot be admitted pending into a half-built handler.
    //   3. IRQs are unmasked last.  Both inits panic on failure, so an
    //      unmask here means a driver is published: an interrupt
    //      asserted by a prior boot stage is taken and acknowledged,
    //      not left level-asserted re-firing forever.
    boot::interrupts(&dt);

    println!();
    println!("r9 from the Internet");
    print_stacks();
    print_binary_sections();
    print_board_info();
    print_memory_info();

    println!("Set up a user process");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), RootPageTableType::User);
    }

    // vmdebug::print_recursive_tables(RootPageTableType::Kernel);
    // vmdebug::print_recursive_tables(RootPageTableType::User);

    // The real bringup: the kernel spawns the nameserver (handed its own
    // channel pair — the first-server asymmetry), the console server (which
    // creates its own pair and BINDs /dev/console), and init (which blocks
    // forever; stage 7 fills it in as the process manager).
    //
    // The nameserver must be up before the console server's BIND is
    // processed. This holds by construction: the nameserver is spawned first
    // and blocks on its first receive; the console server's BIND send wakes
    // it (the IPC fast path); the nameserver processes the BIND before the
    // console server blocks on its post-bind receive.
    println!("starting system");

    let ns_in = ipc::create();
    let ns_out = ipc::create();
    let ns_handles = process::Handles { inbound: ns_in as u32, outbound: ns_out as u32 };

    process::spawn(&process::Image::Elf { bytes: NAMESERVER_ELF, handles: Some(ns_handles) });
    process::spawn(&process::Image::Elf { bytes: CONSOLE_ELF, handles: Some(ns_handles) });
    process::spawn(&process::Image::Elf { bytes: INIT_ELF, handles: None });

    // The console server is up (spawned; its BIND is processed during the
    // first run_all).  Gate off the kernel's normal output: from here on,
    // println! is dropped.  The iprint path (debug/fault) is unaffected.
    port::devcons::set_console_live();

    // The system is live. `run_all` runs the processes to a fixpoint: the
    // nameserver is blocked on its receive loop, the console server on its
    // post-bind receive, and init on its receive.  When all are blocked, the
    // kernel regains control and idles.  A future event (a client message,
    // stage 7) would wake a process; re-entering the scheduler from here is
    // the idle mechanism that stage 7 provides (WFI or an idle process).
    process::run_all();

    #[allow(clippy::empty_loop)]
    loop {}
}

// User process setup now lives in aarch64/tests/user_process.rs, the timer
// exercise in aarch64/tests/timers.rs, and the page/heap allocation exercise
// in aarch64/tests/allocate.rs.
