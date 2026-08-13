//! Integration test: setting up a user process.
//!
//! This replaces the `test_sysexit` scratch function that used to sit in
//! main.rs behind a commented-out call.  It builds the same thing that did
//! -- user page tables, a text page holding a syscall, a stack, and a
//! context pointing at them -- and checks each step instead of assuming it.
//!
//! It stops short of `swtch`.  The syscall handler in trap.rs prints and
//! then spins rather than returning, so entering EL0 is a one way trip: a
//! test that took it could never report what happened.  Asserting the
//! switch needs the syscall path to return first.
#![no_std]
#![no_main]

use aarch64::swtch::Context;
use aarch64::vm::{Entry, RootPageTableType, VaMapping};
use aarch64::{boot, pagealloc, qemu, vm};
use port::println;

/// Report and end the run on the first failure.  A test image has no
/// unwinding and nothing to hand a failure back to but its exit status.
macro_rules! check {
    ($cond:expr, $($arg:tt)+) => {
        if $cond {
            println!("ok    {}", format_args!($($arg)+));
        } else {
            println!("FAIL  {}", format_args!($($arg)+));
            qemu::exit(qemu::FAIL);
        }
    };
}

/// mov x0, #0; mov x1, #1; svc #3
const SYSCALL_EXIT: [u8; 12] =
    [0x00, 0x00, 0x80, 0xd2, 0x21, 0x00, 0x80, 0xd2, 0x61, 0x00, 0x00, 0xd4];

/// Where the user text and stack are mapped.  Both have to be addresses
/// TTBR0 translates: the scratch code this replaces put the stack at
/// KZERO - 0x1000, which is in the TTBR1 half, so the mapping went into
/// the user page table at an address the user page table never sees.
const USER_TEXT_VA: usize = 0x1000;
const USER_STACK_VA: usize = 0x10000;
const PAGE_SIZE: usize = 4096;

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    // Vectors first: without them a fault here goes nowhere at all rather
    // than reaching a handler that can report it.
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    boot::console(&dt);

    println!("running user_process");

    // Build the user address space and make it the live one, exactly as
    // the kernel binary does before it would run a process.
    unsafe {
        vm::init_user_page_tables();
        vm::switch(vm::user_pagetable(), RootPageTableType::User);
    }

    let page_table = vm::user_pagetable();

    // The text page has to land at the address the process will start at:
    // an allocator that ignored VaMapping::Addr would be silently wrong.
    let user_text = pagealloc::allocate_virtpage(
        page_table,
        "usertext",
        Entry::rw_user_text(),
        VaMapping::Addr(USER_TEXT_VA),
        RootPageTableType::User,
    );
    check!(user_text.is_ok(), "allocated user text page");
    let user_text = user_text.unwrap();
    let user_text_va = user_text as *const _ as u64;
    check!(user_text_va == USER_TEXT_VA as u64, "user text mapped at {user_text_va:#x}");

    user_text.0[..SYSCALL_EXIT.len()].copy_from_slice(&SYSCALL_EXIT);
    check!(
        user_text.0[..SYSCALL_EXIT.len()] == SYSCALL_EXIT,
        "syscall reads back from the user text page"
    );

    let user_stack = pagealloc::allocate_virtpage(
        page_table,
        "userstack",
        Entry::rw_user_data(),
        VaMapping::Addr(USER_STACK_VA),
        RootPageTableType::User,
    );
    check!(user_stack.is_ok(), "allocated user stack page");
    let user_stack = user_stack.unwrap();

    let stack_base = user_stack as *mut _ as u64;
    check!(stack_base == USER_STACK_VA as u64, "user stack mapped at {stack_base:#x}");

    // A context placed at the top of the stack, entered by returning into
    // the text page.  It is written through the page's own mapping, so it
    // has to fit: a Context grown past a page would put stack_top below
    // stack_base and scribble on whatever lies before it.
    check!(
        size_of::<Context>() <= PAGE_SIZE,
        "context fits the stack page ({} of {PAGE_SIZE} bytes)",
        size_of::<Context>()
    );
    let stack_top = stack_base + PAGE_SIZE as u64 - size_of::<Context>() as u64;
    let context_ptr = stack_top as *mut Context;
    let context = unsafe { &mut *context_ptr };

    context.set_return(user_text_va);
    context.set_stack_pointer(context_ptr as u64);

    check!(context.x30 == user_text_va, "context returns into the user text");
    check!(context.sp == context_ptr as u64, "context stack pointer is on the user stack");

    println!("user_process passed");
    qemu::exit(qemu::PASS);
}
