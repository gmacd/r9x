#![cfg(target_os = "none")]

extern crate alloc;

use alloc::alloc::Layout;
use core::panic::PanicInfo;

use port::iprintln;

// TODO
//  - Add qemu integration test
#[panic_handler]
pub fn panic(info: &PanicInfo) -> ! {
    iprintln!("{}\n", info);

    #[allow(clippy::empty_loop)]
    loop {}
}

#[alloc_error_handler]
fn oom(_layout: Layout) -> ! {
    panic!("oom");
}
