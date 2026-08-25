//! The kernel binary: the boot sequence, and nothing else.  Everything it
//! calls lives in the `riscv64` library, so that integration tests can link
//! the same code and run a shorter sequence of their own.
#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(not(test), no_main)]

use port::println;
use r9x_core::fdt::DeviceTree;
use riscv64::platform::{devcons, platform_init};
use riscv64::sbi;

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn main9(hartid: usize, dtb_ptr: usize) -> ! {
    let dt = unsafe { DeviceTree::from_usize(dtb_ptr).unwrap() };
    devcons::init(&dt);
    platform_init();

    println!();
    println!("r9 from the Internet");
    println!("Domain0 Boot HART = {hartid}");
    println!("DTB found at: {dtb_ptr:#x}");

    sbi::shutdown();
}
