//! State for the first user process (plans/first-user-process.md).
//!
//! One process, one caller, so most of the state is two words: a slot
//! `swtch` fills with the address of the kernel context it saved when
//! the process started (the switch-back target), and the status the
//! process exited with.  (The process table that replaces this
//! arrives with the scheduler.)

// Only the accessors and run below use these; on host builds (unit
// tests) they are cfg'd away with the rest of the machinery.
#[cfg(target_os = "none")]
use port::mem::PAGE_SIZE_4K;

#[cfg(target_os = "none")]
use crate::pagealloc;
#[cfg(target_os = "none")]
use crate::swtch::{Context, SPSR_EL1H, swtch};
#[cfg(target_os = "none")]
use crate::vm::{self, Entry, RootPageTableType, VaMapping};

/// `svc #0`: the process asks to be killed.  The syscall number is the
/// svc immediate, held in ESR_EL1.ISS for EC 0x15 (Arm ARM DDI 0487).
pub const SYSEXIT: u64 = 0;

/// `svc #1`: yield — do nothing; the handler's return is the whole
/// syscall.
pub const SYSYIELD: u64 = 1;

// Single-core by the l.S gate (non-zero MPIDR affinity hangs at
// boot), so the statics need no synchronisation.  KERNEL_SLOT is
// swtch's `from` argument itself: the switch writes the saved kernel
// context's address into it, so nothing else ever sets it.
#[cfg(target_os = "none")]
static mut KERNEL_SLOT: *mut Context = core::ptr::null_mut();
#[cfg(target_os = "none")]
static mut EXIT_STATUS: u64 = 0;

/// The saved kernel context a running process exits to, or null.
///
/// # Safety
///
/// Single core.
#[cfg(target_os = "none")]
pub(crate) unsafe fn kernel_slot() -> *mut Context {
    unsafe { KERNEL_SLOT }
}

/// The address of the slot, to hand to `swtch` as its `from` argument.
///
/// # Safety
///
/// Single core.
#[cfg(target_os = "none")]
pub(crate) unsafe fn kernel_slot_addr() -> *mut *mut Context {
    core::ptr::addr_of_mut!(KERNEL_SLOT)
}

/// Drop the saved kernel context.  Called by `run` on resumption; the
/// slot is null again until the next start.
///
/// # Safety
///
/// Single core.
#[cfg(target_os = "none")]
pub(crate) unsafe fn clear_kernel_slot() {
    unsafe { KERNEL_SLOT = core::ptr::null_mut() };
}

/// Record a process's exit status.
///
/// # Safety
///
/// Single core.  Called only from the exit trap, which cannot run
/// unless a process is running.
#[cfg(target_os = "none")]
pub(crate) unsafe fn set_exit_status(status: u64) {
    unsafe { EXIT_STATUS = status }
}

/// The status the last process exited with.
///
/// # Safety
///
/// Single core.
#[cfg(target_os = "none")]
pub(crate) unsafe fn exit_status() -> u64 {
    unsafe { EXIT_STATUS }
}

/// Start a process: map a text page at `text_va` holding `text` and a
/// stack page at `stack_va` in the user table, place its context at
/// the top of the stack, and switch to it.  Returns when the process
/// exits, with its exit status (the svc number).
///
/// # Preconditions
///
/// TTBR0 is the user table, and interrupts are fully brought up
/// (`boot::interrupts`): the process enters with IRQs unmasked, so
/// the timer keeps firing while it runs.
///
/// # Panics
///
/// On allocation failure: callers are init-context (`main9`, the test
/// images), where a panic is the failure report.
#[cfg(target_os = "none")]
pub fn run(text: &[u8], text_va: usize, stack_va: usize) -> u64 {
    // The context must fit the stack page: it is placed at the top,
    // and one that does not fit would land below the page and
    // scribble on whatever lies before it.
    assert!(
        core::mem::size_of::<Context>() <= PAGE_SIZE_4K,
        "Context is {} bytes, the stack page is {PAGE_SIZE_4K}",
        core::mem::size_of::<Context>()
    );

    let user_text = pagealloc::allocate_virtpage(
        vm::user_pagetable(),
        "proctxt",
        Entry::rw_user_text(),
        VaMapping::Addr(text_va),
        RootPageTableType::User,
    )
    .unwrap_or_else(|err| panic!("process text page: {err:?}"));
    user_text.0[..text.len()].copy_from_slice(text);

    let user_stack = pagealloc::allocate_virtpage(
        vm::user_pagetable(),
        "procstack",
        Entry::rw_user_data(),
        VaMapping::Addr(stack_va),
        RootPageTableType::User,
    )
    .unwrap_or_else(|err| panic!("process stack page: {err:?}"));

    // The context lives at the top of the stack page and is entered
    // by returning into the text page.  The reference
    // allocate_virtpage returns points at the mapped address, so the
    // context is written through the mapping the process will run
    // under.
    // SAFETY: context_addr is inside the stack page allocated and
    // mapped above, and the assert guarantees the Context fits in it.
    let context_addr =
        (user_stack as *const _ as usize) + PAGE_SIZE_4K - core::mem::size_of::<Context>();
    let context = unsafe { &mut *(context_addr as *mut Context) };
    context.x19 = 0;
    context.x20 = 0;
    context.x21 = 0;
    context.x22 = 0;
    context.x23 = 0;
    context.x24 = 0;
    context.x25 = 0;
    context.x26 = 0;
    context.x27 = 0;
    context.x28 = 0;
    context.x29 = 0;
    context.x30 = 0;
    context.sp = 0;
    context.spsr = 0; // EL0, SP0, DAIF unmasked, IL = 0
    context.set_return(text_va as u64);
    context.set_stack_pointer(context_addr as u64);

    // The switch saves the kernel context into KERNEL_SLOT and enters
    // the process.  Control returns here only when the exit trap has
    // switched back to that context, so this frame must stay live
    // until then: nothing below returns before the switch-back.
    unsafe { swtch(kernel_slot_addr(), context, SPSR_EL1H) };

    let status = unsafe { exit_status() };
    unsafe { clear_kernel_slot() };
    status
}

/// Host (unit-test) builds have no user table, no page allocator, and
/// no switch; `run` cannot exist meaningfully there.  It is pub for
/// the bare-metal images and unreachable in tests.
#[cfg(not(target_os = "none"))]
pub fn run(_text: &[u8], _text_va: usize, _stack_va: usize) -> u64 {
    loop {
        core::hint::spin_loop();
    }
}
