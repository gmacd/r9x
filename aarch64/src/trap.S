// Space reserved for interrupt stack:
//    interruptstackbase to interruptstackbase+INTERRUPTSTACKSZ.
// Stack base address (bottom):
//    interruptstackbase+INTERRUPTSTACKSZ
// Stack pointer will reduce in value as data is pushed to the stack.
INTERRUPTSTACKSZ = 4096*16

.bss
.balign	4096
.globl	interruptstackbase
interruptstackbase:	.space INTERRUPTSTACKSZ
.globl	interruptstacksz
.equ	interruptstacksz,	INTERRUPTSTACKSZ

.section .text

.equ	SYNC_INVALID_EL1t,	0
.equ	IRQ_INVALID_EL1t,	1
.equ	FIQ_INVALID_EL1t,	2
.equ	ERROR_INVALID_EL1t,	3

.equ	SYNC_INVALID_EL1h,	4
.equ	IRQ_INVALID_EL1h,	5
.equ	FIQ_INVALID_EL1h,	6
.equ	ERROR_INVALID_EL1h,	7

.equ	SYNC_INVALID_EL0_64,	8
.equ	IRQ_INVALID_EL0_64,	9
.equ	FIQ_INVALID_EL0_64,	10
.equ	ERROR_INVALID_EL0_64,	11

.equ	SYNC_INVALID_EL0_32,	12
.equ	IRQ_INVALID_EL0_32,	13
.equ	FIQ_INVALID_EL0_32,	14
.equ	ERROR_INVALID_EL0_32,	15

// Ventry aligns to 128 bytes and branches to label, and is used for the interrupt vector table
.macro	ventry	label
.balign	128
	b	\label
.endm

// We save all general purpose registers, and a few useful system registers:
// - x0-28
// - x29 (Frame pointer)
// - x30 (Link register)
// We additionally save the registers below, so that they're availalble via the TrapFrame
// - ESR_EL1 (Exception syndrome register EL1)
// - ELR_EL1 (Exception link register EL1)
// - FAR_EL1 (Fault address register EL1)
// and finally the interrupt type and the interrupted stack pointer.

// Store x0-x30 and the system registers into a 288-byte frame already
// made at sp.  The frame's last slot — the interrupted stack pointer,
// offset 280 — is the caller's to fill: where it comes from depends on
// which EL the exception came from.
.macro	save_frame type
	stp 	x0, x1, [sp, #16 * 0]
	stp 	x2, x3, [sp, #16 * 1]
	stp	x4, x5, [sp, #16 * 2]
	stp	x6, x7, [sp, #16 * 3]
	stp	x8, x9, [sp, #16 * 4]
	stp	x10, x11, [sp, #16 * 5]
	stp	x12, x13, [sp, #16 * 6]
	stp	x14, x15, [sp, #16 * 7]
	stp	x16, x17, [sp, #16 * 8]
	stp	x18, x19, [sp, #16 * 9]
	stp	x20, x21, [sp, #16 * 10]
	stp	x22, x23, [sp, #16 * 11]
	stp	x24, x25, [sp, #16 * 12]
	stp	x26, x27, [sp, #16 * 13]
	stp	x28, x29, [sp, #16 * 14]

	// LR, ESR_EL1
	mrs	x0, esr_el1
	stp	x30, x0, [sp, #16 * 15]

	// ELR_EL1, FAR_EL1
	mrs	x1, elr_el1
	mrs	x2, far_el1
	stp	x1, x2, [sp, #16 * 16]

	// Interrupt type
	ldr	x3, =\type
	str	x3, [sp, #16 * 17]

	// Interrupted SPSR: trapret stages it back before the eret, so a
	// process resumed through a switch gets its own PSTATE (NZCV in
	// particular) back instead of the slot's initial value.
	mrs	x3, spsr_el1
	str	x3, [sp, #16 * 18]
.endm

// Restore x0-x30 from the frame at sp.
.macro	restore_frame
	ldp	x0, x1, [sp, #16 * 0]
	ldp	x2, x3, [sp, #16 * 1]
	ldp	x4, x5, [sp, #16 * 2]
	ldp	x6, x7, [sp, #16 * 3]
	ldp	x8, x9, [sp, #16 * 4]
	ldp	x10, x11, [sp, #16 * 5]
	ldp	x12, x13, [sp, #16 * 6]
	ldp	x14, x15, [sp, #16 * 7]
	ldp	x16, x17, [sp, #16 * 8]
	ldp	x18, x19, [sp, #16 * 9]
	ldp	x20, x21, [sp, #16 * 10]
	ldp	x22, x23, [sp, #16 * 11]
	ldp	x24, x25, [sp, #16 * 12]
	ldp	x26, x27, [sp, #16 * 13]
	ldp	x28, x29, [sp, #16 * 14]
	ldr	x30, [sp, #16 * 15]
.endm

// An exception taken from EL1: the kernel was interrupted mid-run, so
// the frame goes on the stack it was running on and popping it puts sp
// back exactly where it was.  eret restores no stack pointer, so a
// switch to a dedicated stack here would resume the interrupted code
// on the wrong stack — and a nested exception would reset to that
// stack's top and overwrite the live outer frame.
.macro	handle_interrupt_el1 type
	// Mask IRQs before anything else: one landing inside a handler
	// would push its frame on top of the live one being built.  eret
	// restores the interrupted DAIF from the hardware SPSR_EL1, so the
	// mask leaks nowhere.  DAIFSet imm4 encodes D,A,I,F as bits 3..0,
	// so #2 is the I bit (Arm ARM DDI 0487).
	msr	daifset, #2

	sub 	sp, sp, #304
	save_frame \type

	// The interrupted sp is where the frame push started.
	add	x0, sp, #304
	str	x0, [sp, #16 * 17 + 8]

	// Pass pointer to TrapFrame (on stack) as the first arg
	mov	x0, sp
	bl	trap_unsafe

	restore_frame
	add	sp, sp, #304

	eret
.endm

// An exception taken from EL0: SP_EL1 is the process's own kstack
// (its top is the invariant the process runs under), so the frame is
// pushed straight there with no scratch register before save_frame —
// x0-x18 are all stored — and no TPIDR parking: TPIDR_EL1 now carries
// the current-process pointer for the Rust side, a use the parking
// hack (which overwrote TPIDR on every trap) was mutually exclusive
// with.
.macro	handle_interrupt_el0 type
	// Mask IRQs before anything else: one landing before the frame is
	// pushed would walk the kstack's top with no frame under it.  eret
	// restores the interrupted DAIF from the hardware SPSR_EL1, so the
	// mask leaks nowhere.
	msr	daifset, #2

	sub	sp, sp, #304
	save_frame \type

	// The interrupted sp is the process's, on SP_EL0.
	mrs	x0, sp_el0
	str	x0, [sp, #16 * 17 + 8]

	// Pass pointer to TrapFrame (on the stack) as the first arg
	mov	x0, sp
	bl	trap_unsafe

	// Every EL0 return goes through the shared tail, which switch-ins
	// (forkret entry, a resched resume) reach by eret into trapret
	// with sp = frame base — exactly as this bl returns.
	b	trapret
.endm

/// The exception vector table for exceptions taken to EL1.
/// Each entry is 16 instructions/128 bytes.
/// Ventry handles alignment of individual entries.
.balign	2048
.globl exception_vectors
exception_vectors:
	// Current EL with SP0
	ventry	sync_invalid_el1t			// Synchronous EL1t
	ventry	irq_invalid_el1t			// IRQ EL1t
	ventry	fiq_invalid_el1t			// FIQ EL1t
	ventry	error_invalid_el1t			// Error EL1t

	// Current EL with SPx
	ventry	sync_invalid_el1h			// Synchronous EL1h
	ventry	irq_invalid_el1h			// IRQ EL1h
	ventry	fiq_invalid_el1h			// FIQ EL1h
	ventry	error_invalid_el1h			// Error EL1h

	// Lower EL using AArch64
	ventry	sync_invalid_el0_64			// Synchronous 64-bit EL0
	ventry	irq_invalid_el0_64			// IRQ 64-bit EL0
	ventry	fiq_invalid_el0_64			// FIQ 64-bit EL0
	ventry	error_invalid_el0_64			// Error 64-bit EL0

	// Lower EL using AArch32
	ventry	sync_invalid_el0_32			// Synchronous 32-bit EL0
	ventry	irq_invalid_el0_32			// IRQ 32-bit EL0
	ventry	fiq_invalid_el0_32			// FIQ 32-bit EL0
	ventry	error_invalid_el0_32			// Error 32-bit EL0

sync_invalid_el1t:
	handle_interrupt_el1  SYNC_INVALID_EL1t

irq_invalid_el1t:
	handle_interrupt_el1  IRQ_INVALID_EL1t

fiq_invalid_el1t:
	handle_interrupt_el1  FIQ_INVALID_EL1t

error_invalid_el1t:
	handle_interrupt_el1  ERROR_INVALID_EL1t

sync_invalid_el1h:
	handle_interrupt_el1  SYNC_INVALID_EL1h

irq_invalid_el1h:
	handle_interrupt_el1  IRQ_INVALID_EL1h

fiq_invalid_el1h:
	handle_interrupt_el1  FIQ_INVALID_EL1h

error_invalid_el1h:
	handle_interrupt_el1  ERROR_INVALID_EL1h

sync_invalid_el0_64:
	handle_interrupt_el0  SYNC_INVALID_EL0_64

irq_invalid_el0_64:
	handle_interrupt_el0  IRQ_INVALID_EL0_64

fiq_invalid_el0_64:
	handle_interrupt_el0  FIQ_INVALID_EL0_64

error_invalid_el0_64:
	handle_interrupt_el0  ERROR_INVALID_EL0_64

sync_invalid_el0_32:
	handle_interrupt_el0  SYNC_INVALID_EL0_32

irq_invalid_el0_32:
	handle_interrupt_el0  IRQ_INVALID_EL0_32

fiq_invalid_el0_32:
	handle_interrupt_el0  FIQ_INVALID_EL0_32

error_invalid_el0_32:
	handle_interrupt_el0  ERROR_INVALID_EL0_32

// The EL0 tail, shared by every EL0 return path and reached by
// switch-in with sp = the frame's base.  SPSR, ELR and the user stack
// pointer are staged from the frame — not left to the live registers
// — because a switch-in that erets here leaves kernel values in them
// (swtch wrote them for the context it resumed), and the frame holds
// the process's own.  x0-x18 are dead until restore_frame, so x6 is
// a safe scratch.  DAIF.I has been masked since vector entry (no
// nested exception can land between the msr's and the eret: SPSR_EL1
// is a holding register, the live PSTATE stays masked).
//
// x6 is the scratch: it is dead on every path (vector entry stores it
// before the handler; a switch-in erets here with x6 = the saved
// kernel value), and restore_frame restores it before the eret.
//
// The frame holds no saved SP_EL1 — that is the point: a process runs
// on its own kstack, so sp at the frame base IS the kstack top, and
// the add below is what puts the invariant back.
.globl	trapret
trapret:
	ldr	x6, [sp, #16 * 18]
	msr	spsr_el1, x6
	ldr	x6, [sp, #16 * 16]
	msr	elr_el1, x6
	ldr	x6, [sp, #16 * 17 + 8]
	msr	sp_el0, x6

	restore_frame
	add	sp, sp, #304

	eret
