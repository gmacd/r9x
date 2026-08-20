//! The kernel binary: the boot sequence, and nothing else.  Everything it
//! calls lives in the `aarch64` library, so that integration tests can link
//! the same code and run a shorter sequence of their own.
#![allow(clippy::too_many_arguments)]
#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(not(test), no_main)]

extern crate alloc;

use aarch64::kmem::{
    boottext_physrange, bss_physrange, data_physrange, rodata_physrange, text_physrange,
    total_kernel_physrange,
};
use aarch64::param::KZERO;
use aarch64::timer::{Timer, TimerCallback};
use aarch64::vm::{Entry, RootPageTableType, VaMapping};
use aarch64::{boot, mailbox, pagealloc, process, vm};
use alloc::boxed::Box;
use core::time::Duration;
use port::mem::{PhysRange, VirtRange};
use port::{iprintln, println};

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

/// The first process's whole program: `svc #0` (sysexit).  AArch64
/// `svc` is 0xd4000001 | (number << 8), little-endian (Arm ARM DDI
/// 0487).
const FIRST_PROCESS_TEXT: [u8; 4] = [0x01, 0x00, 0x00, 0xd4];

/// Where the first process's text and stack are mapped: the TTBR0
/// (user) half, not the TTBR1 (kernel) half.
const USER_TEXT_VA: usize = 0x1000;
const USER_STACK_VA: usize = 0x10000;

/// dtb_va is the virtual address of the DTB structure.  The physical address is
/// assumed to be dtb_va-KZERO.
#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    boot::irq_ops();

    // Parse the DTB before we set up memory so we can correctly map it
    let dt = unsafe { boot::device_tree(dtb_va) };
    if let Err(err) = boot::page_allocator(&dt, dtb_va) {
        panic!("couldn't init page allocator: {err:?}");
    }

    boot::console(&dt);
    mailbox::init(&dt);

    // Interrupt bringup, in a required order:
    //   1. timer::init disarms CNTP_CTL_EL0 before the GIC enables the
    //      timer PPI, so a firmware-armed timer cannot fire into a
    //      half-built handler.
    //   2. gic::init enables the distributor and this core's CPU
    //      interface.  boot::irq_ops (above, before the MMU work) must
    //      already have registered the DAIF ops IrqGuard needs.
    //   3. IRQs are unmasked only if the GIC came up.  With no driver
    //      published, an interrupt asserted by a prior boot stage would
    //      be taken, find nothing to acknowledge it, and — being
    //      level-triggered — re-fire forever.
    boot::interrupts(&dt);

    println!();
    println!("r9 from the Internet");
    print_stacks();
    print_binary_sections();
    print_board_info();
    print_memory_info();

    // Test code
    {
        let page_table = vm::kernel_pagetable();
        let entry = Entry::rw_kernel_data();
        for i in 0..3 {
            let alloc_result = pagealloc::allocate_virtpage(
                page_table,
                "testkernel",
                entry,
                VaMapping::Offset(KZERO),
                RootPageTableType::Kernel,
            );
            match alloc_result {
                Ok(_allocated_page) => {}
                Err(err) => {
                    println!("Error allocating page in kernel space ({i}): {:?}", err);
                    break;
                }
            }
        }
    }

    println!("Set up a user process");

    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), RootPageTableType::User);
    }

    // vmdebug::print_recursive_tables(RootPageTableType::Kernel);
    // vmdebug::print_recursive_tables(RootPageTableType::User);

    // The first process: its whole program is the sysexit, and it
    // enters with IRQs unmasked, so the timers keep firing while it
    // runs.  The kernel resumes here when it exits.
    println!("starting first process");
    let status = process::run(&FIRST_PROCESS_TEXT, USER_TEXT_VA, USER_STACK_VA);
    println!("first process returned, status {status}");

    let _b = Box::new("ddododo");

    PC1_TIMER.start();
    PC2_TIMER.start();
    STOP_PC1_TIMER.start();

    println!("looping now");

    #[allow(clippy::empty_loop)]
    loop {}
}

// Temp, test-related code

use core::sync::atomic::{AtomicU32, Ordering};

/// Prints "<name>:<count>" each firing; stops itself after `limit`
/// extra firings (0 = run until cancelled).
struct Ticker {
    name: &'static str,
    counter: AtomicU32,
    limit: u32,
}

impl TimerCallback for Ticker {
    fn fire(&self) -> bool {
        let n = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        iprintln!("{}:{}", self.name, n);
        if self.limit == 0 || n <= self.limit {
            true
        } else {
            iprintln!("stopping {}", self.name);
            false
        }
    }
}

/// One-shot callback that cancels another timer.
struct CancelTimer {
    victim: &'static Timer,
    msg: &'static str,
}

impl TimerCallback for CancelTimer {
    fn fire(&self) -> bool {
        iprintln!("{}", self.msg);
        self.victim.cancel();
        false
    }
}

static PC1: Ticker = Ticker { name: "pc1", counter: AtomicU32::new(0), limit: 0 };
static PC1_TIMER: Timer = Timer::periodic(Duration::from_secs(1), &PC1);

static PC2: Ticker = Ticker { name: "pc2", counter: AtomicU32::new(0), limit: 3 };
static PC2_TIMER: Timer = Timer::periodic(Duration::from_secs(2), &PC2);

static STOP_PC1: CancelTimer = CancelTimer { victim: &PC1_TIMER, msg: "stopping pc1" };
static STOP_PC1_TIMER: Timer = Timer::new(Duration::from_secs(5), &STOP_PC1);

// User process setup now lives in aarch64/tests/user_process.rs.
