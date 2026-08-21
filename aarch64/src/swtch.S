// Context switch
//
//   pub(crate) unsafe fn swtch(from: *mut *mut Context, to: *const Context,
//                              spsr: u64)
//
// Save the current registers on the stack, creating a Context struct,
// and save its address in `*from`.  Switch stacks to `to` and pop
// previously-saved registers.
//
// x2 (`spsr`) is recorded as the saved context's spsr: AArch64 has no
// instruction to read the current state (the mrs-rd-cpsr that the AArch32
// original used does not exist here; Arm ARM DDI 0487), so the caller
// supplies the EL/SP/DAIF state the saved context should resume in.

.section .text

.globl swtch
swtch:
	// (1) Save x19-x30, the stack pointer, and the caller's resume
	// state as a Context at the bottom of the caller's stack, in the
	// repr(C) layout the restore below (and the struct) read: x19
	// first, sp and spsr last.  Pre-decrement stp's would lay the
	// pairs out mirrored and the switch-back would read them shifted
	// by a pair (x30 from the x20 slot, sp/spsr from stale caller
	// frame), so the frame is built with sp lowered once and each
	// pair stored at an explicit offset.
	mov x4, sp
	sub sp, sp, #112
	stp x19, x20, [sp]
	stp x21, x22, [sp, #16]
	stp x23, x24, [sp, #32]
	stp x25, x26, [sp, #48]
	stp x27, x28, [sp, #64]
	stp x29, x30, [sp, #80]
	// x2, the caller's resume state, not spsr_el1: the latter holds the
	// state of the last context trapped to EL1, not the kernel's own
	// (Arm ARM DDI 0487, SPSR_EL1), and is uninitialised until then.
	stp x4, x2, [sp, #96]

	// (2) Switch stacks.  Once this section completes we will be in the
	// context of the process referred to by the 'to' argument.

	mov x5, sp
	str x5, [x0]   // saved context address (the Context's own base)
	mov sp, x1     // Load new SP from second argument (new context)

	// (3) Load the process registers (typically previously saved in (1) during
	// a previous invocation of this subroutine).
	ldp x19, x20, [sp], #16
	ldp x21, x22, [sp], #16
	ldp x23, x24, [sp], #16
	ldp x25, x26, [sp], #16
	ldp x27, x28, [sp], #16
	ldp x29, x30, [sp], #16
	ldp x4, x5, [sp], #16
	msr spsr_el1, x5            // x5 is the to-context's own spsr

	msr elr_el1, x30

	// The eret copies no stack pointer (Arm ARM DDI 0487,
	// AArch64.ExceptionReturn), so the to-context's recorded sp goes
	// to SP_EL0: an EL0 return lands on the process's user stack, and
	// for an EL1 target the value is inert (SP_EL0 is banked) until
	// the next EL0 entry re-establishes it.
	msr sp_el0, x4

	// Return to location in elr_el1
	eret
