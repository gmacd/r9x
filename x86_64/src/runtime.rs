#![cfg(target_os = "none")]

extern crate alloc;

use alloc::alloc::Layout;
use core::panic::PanicInfo;

#[panic_handler]
pub fn panic(_info: &PanicInfo) -> ! {
    // Under test a panic is a failed run, and looping here would leave the
    // harness waiting for its timeout rather than reporting the failure.
    #[cfg(feature = "qemu-test")]
    crate::qemu::exit(crate::qemu::FAIL);

    #[cfg(not(feature = "qemu-test"))]
    #[allow(clippy::empty_loop)]
    loop {}
}

#[alloc_error_handler]
fn oom(_layout: Layout) -> ! {
    panic!("oom");
}
