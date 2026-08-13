//! The kernel binary: the boot sequence, and nothing else.  Everything it
//! calls lives in the `x86_64` library, so that integration tests can link
//! the same code and run a shorter sequence of their own.
#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(not(test), no_main)]
#![forbid(unsafe_op_in_unsafe_fn)]

use port::println;
use x86_64::proc::{Label, swtch};
use x86_64::{dat, devcons, syscall, trap, vsvm};

static mut THRSTACK: [u64; 1024] = [0; 1024];
static mut CTX: u64 = 0;
static mut THR: u64 = 0;

fn jumpback() {
    println!("in a thread");
    unsafe {
        let thr = &mut *(THR as *mut Label);
        let ctx = &mut *(CTX as *mut Label);
        swtch(thr, ctx);
    }
}

#[cfg_attr(not(test), unsafe(no_mangle))]
pub extern "C" fn main(mach: &mut dat::Mach, _mbdata: u64) {
    unsafe {
        vsvm::init(mach);
    }
    syscall::init();
    let x = trap::splhi();
    devcons::init();
    println!();
    println!("r9 from the Internet");
    println!("looping now");
    let mut ctx = Label::new();
    let mut thr = Label::new();
    thr.pc = jumpback as *const () as usize as u64;
    unsafe {
        thr.sp = &mut THRSTACK[1023] as *mut _ as u64;
        CTX = &mut ctx as *mut _ as u64;
        THR = &mut thr as *mut _ as u64;
        swtch(&mut ctx, &mut thr);
    }
    println!("came out the other side of a context switch");
    trap::splx(x);
    unsafe { core::arch::asm!("int3") };
    loop {
        trap::spllo();
    }
}
