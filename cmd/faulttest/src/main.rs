//! The fault test: a small Rust binary that deliberately writes to an
//! unmapped address, with a few frames in the call stack.  The test image
//! spawns it and checks that it dies with FAULT_STATUS (0xff).  The kernel's
//! fault handler prints a backtrace (frame-pointer walk) before killing it.

#![no_std]
#![no_main]

use r9x_std::rt;

#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn start(dtb_va: usize) -> ! {
    rt::run(dtb_va, main)
}

#[inline(never)]
fn main() {
    level1();
    // Unreachable: level1 faults.
    r9x_std::process::exit(0);
}

#[inline(never)]
fn level1() {
    level2();
}

#[inline(never)]
fn level2() {
    level3();
}

#[inline(never)]
fn level3() {
    // Write to an unmapped address: a data abort in EL0.
    let bad_ptr: *mut u64 = 0x5000 as *mut u64;
    unsafe {
        core::ptr::write_volatile(bad_ptr, 42);
    }
    // Unreachable.
}
