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
	// (1) Save callee-saved and other registers onto the caller's stack.
	// We do this so that when we call swtch again (giving this stack pointer
	// as the 'to' address),
	stp x19, x20, [sp, #-16]!
	stp x21, x22, [sp, #-16]!
	stp x23, x24, [sp, #-16]!
	stp x25, x26, [sp, #-16]!
	stp x27, x28, [sp, #-16]!
	stp x29, x30, [sp, #-16]!
	mov x4, sp
	// x2, the caller's resume state, not spsr_el1: the latter holds the
	// state of the last context trapped to EL1, not the kernel's own
	// (Arm ARM DDI 0487, SPSR_EL1), and is uninitialised until then.
	stp x4, x2, [sp, #-16]!

	// (2) Switch stacks.  Once this section completes we will be in the
	// context of the process referred to by the 'to' argument.

	str x4, [x0]  // Store current SP to first argument (old context)
	mov sp, x1    // Load new SP from second argument (new context)

	// (3) Load the process registers (typically previously saved in (1) during
	// a previous invocation of this subroutine).
	ldp x19, x20, [sp], #16
	ldp x21, x22, [sp], #16
	ldp x23, x24, [sp], #16
	ldp x25, x26, [sp], #16
	ldp x27, x28, [sp], #16
	ldp x29, x30, [sp], #16
	ldp x4, x5, [sp], #16       // x4 (the saved sp) is dead: sp is now x1
	msr spsr_el1, x5            // x5 is the to-context's own spsr

	msr elr_el1, x30

	// Return to location in elr_el1
	eret
