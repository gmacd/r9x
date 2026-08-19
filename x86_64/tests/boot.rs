//! Integration test: the state l.S and multiboot left us in.
//!
//! This is a whole kernel image.  It links the same library the kernel
//! binary does, so `start` in l.S runs the usual early boot and then calls
//! the `main` below instead of the real one.  Only the per-CPU block and
//! the console are brought up: nothing here needs the syscall path or a
//! context switch.
//!
//! None of this can be checked by a host unit test: every assertion is
//! about what the boot code handed a kernel that has just started.
#![no_std]
#![no_main]

use port::println;
use x86_64::dat::Stack;
use x86_64::{dat, devcons, qemu, trap, vsvm};

#[macro_use]
mod common;

#[unsafe(no_mangle)]
pub extern "C" fn main(mach: &mut dat::Mach, _mbdata: u64) {
    // The per-CPU block has to be installed before anything reads it, and
    // interrupts held off until there are handlers, exactly as the kernel
    // binary does before it prints.
    unsafe { vsvm::init(mach) };
    let spl = trap::splhi();
    devcons::init();

    println!("running boot");

    // We are executing from the image multiboot loaded, above the first
    // megabyte it keeps for itself.
    let here = main as *const () as usize;
    check!(here > 0x10_0000, "executing above the first megabyte ({here:#x})");

    // The Mach the boot code handed over is where the gs base is derived
    // from, so its address has to be one the arithmetic in vsvm::init can
    // use: page aligned, and not null.
    let mach_addr = mach as *mut dat::Mach as usize;
    check!(mach_addr != 0, "mach handed over ({mach_addr:#x})");
    check!(mach_addr.is_multiple_of(4096), "mach is page aligned ({mach_addr:#x})");

    // The exception stacks are what the IST entries point at.  Two sharing
    // a top would have one exception overwrite the other's frame, and the
    // fault that found out would be the one you least want to debug.
    let tops = [
        mach.debug_stack.top() as usize,
        mach.bp_stack.top() as usize,
        mach.nmi_stack.top() as usize,
        mach.df_stack.top() as usize,
    ];
    let distinct = tops.iter().enumerate().all(|(i, a)| tops[i + 1..].iter().all(|b| a != b));
    check!(distinct, "the four exception stacks are distinct");
    check!(tops.iter().all(|t| t.is_multiple_of(16)), "exception stacks are aligned");

    // Put back the level splhi replaced.  Nothing asserts on it: IntrStatus
    // is opaque, and giving it PartialEq to satisfy a test would be the
    // test dictating the kernel's API.
    trap::splx(spl);

    println!("boot passed");
    qemu::exit(port::qemu::PASS);
}
