#![cfg(target_os = "none")]

extern crate alloc;

use alloc::alloc::Layout;
use core::arch::asm;
use core::panic::PanicInfo;

use port::{print, println};

#[unsafe(no_mangle)]
extern "C" fn eh_personality() {}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    print!("Panic: ");
    if let Some(p) = info.location() {
        println!("line {}, file {}: {}", p.line(), p.file(), info.message());
    } else {
        println!("no information available.");
    }

    // Under test a panic is a failed run, and spinning here would leave the
    // harness waiting for its timeout rather than reporting the failure.
    #[cfg(feature = "qemu-test")]
    crate::qemu::exit(crate::qemu::FAIL);

    #[cfg(not(feature = "qemu-test"))]
    abort();
}

#[unsafe(no_mangle)]
extern "C" fn abort() -> ! {
    loop {
        unsafe {
            asm!("wfi");
        }
    }
}

#[alloc_error_handler]
fn oom(_layout: Layout) -> ! {
    panic!("oom");
}
