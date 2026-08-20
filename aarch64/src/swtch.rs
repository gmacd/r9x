use core::fmt;

#[cfg(target_os = "none")]
core::arch::global_asm!(include_str!("swtch.S"));

#[derive(Copy, Clone)]
#[repr(C)]
pub struct Context {
    pub x19: u64,
    pub x20: u64,
    pub x21: u64,
    pub x22: u64,
    pub x23: u64,
    pub x24: u64,
    pub x25: u64,
    pub x26: u64,
    pub x27: u64,
    pub x28: u64,
    pub x29: u64, // Frame pointer
    pub x30: u64, // Link register (return address)
    pub sp: u64,
    pub spsr: u64,
}

impl Context {
    pub fn set_return(&mut self, addr: u64) {
        self.x30 = addr;
    }

    pub fn set_stack_pointer(&mut self, addr: u64) {
        self.sp = addr;
    }
}

impl fmt::Debug for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Context")
            .field("x19", &format_args!("{:#018x}", self.x19))
            .field("x20", &format_args!("{:#018x}", self.x20))
            .field("x21", &format_args!("{:#018x}", self.x21))
            .field("x22", &format_args!("{:#018x}", self.x22))
            .field("x23", &format_args!("{:#018x}", self.x23))
            .field("x24", &format_args!("{:#018x}", self.x24))
            .field("x25", &format_args!("{:#018x}", self.x25))
            .field("x26", &format_args!("{:#018x}", self.x26))
            .field("x27", &format_args!("{:#018x}", self.x27))
            .field("x28", &format_args!("{:#018x}", self.x28))
            .field("x29", &format_args!("{:#018x}", self.x29))
            .field("x30", &format_args!("{:#018x}", self.x30))
            .field("sp", &format_args!("{:#018x}", self.sp))
            .field("spsr", &format_args!("{:#018x}", self.spsr))
            .finish()
    }
}

// The switch and its wrapper exist only where the assembly is built in;
// host builds of the library (unit tests) never see them.
/// The state a kernel context resumes in: EL1 with SP_EL1 (M = 0b0101,
/// SP = 1), DAIF unmasked, IL = 0.  The CPSR state encoding per Arm ARM
/// DDI 0487; callers masking interrupts at switch-out OR in the DAIF
/// bits (0x3c0: D=1<<9, A=1<<8, I=1<<7, F=1<<6) so the switch-back
/// restores their mask.
pub const SPSR_EL1H: u64 = 0x7;

#[cfg(target_os = "none")]
unsafe extern "C" {
    #[link_name = "swtch"]
    fn swtch_asm(from: *mut *mut Context, to: *const Context, spsr: u64);
}

/// Switch from the current context to the one described by `to`.
///
/// Saves x19–x30, the stack pointer, and `spsr` as a `Context` on the
/// caller's stack, stores its address in `*from`, and enters the `to`
/// context.  The call does not return until the saved context is
/// switched back to with a later `swtch` call.
///
/// `spsr` is the EL/SP/DAIF state the saved context resumes in when it
/// is switched back to; AArch64 cannot read the current state, so the
/// caller supplies it.  A kernel caller passes [`SPSR_EL1H`], ORing in
/// the DAIF bits (0x3c0) if it is masking interrupts at switch-out.
///
/// # Safety
///
/// - `from` must address a writable `*mut Context` slot that stays
///   alive for the whole switch: the saved context lives on this
///   caller's stack, so the caller's frame must not return before the
///   switch-back.
/// - `to` must address a valid `Context`: either one saved by an
///   earlier `swtch`, or one a starter has fully initialised
///   (callee-saved registers, `sp` to a live stack, `x30` to an entry
///   point, `spsr` to the target EL state).
/// - `spsr` must encode a valid resume state for this caller (for
///   kernel callers, [`SPSR_EL1H`] with the caller's own DAIF mask).
/// - On the switch-back, x0–x18 hold whatever the process left in
///   them; only x19–x30, the stack, and the EL/DAIF state are
///   preserved, per the AArch64 procedure call standard.
// Nothing calls this yet: the user_process test stops short of the
// switch, because the syscall handler it would land in never returns.
// process-run (the first-user-process arc) lands the first caller; the
// expectation fails as a prompt to drop it, and to make this pub, which
// that caller needs.
#[cfg(target_os = "none")]
#[expect(dead_code)]
pub(crate) unsafe fn swtch(from: *mut *mut Context, to: *const Context, spsr: u64) {
    unsafe { swtch_asm(from, to, spsr) };
}
