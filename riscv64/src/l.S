.section .text.entry
.globl start
start:
	// a0 is the hart SBI started us on, and it does not promise that is
	// hart 0.  Only the boot hart is released into the payload -- the
	// others wait in firmware until something calls the HSM extension --
	// so whichever hart arrives here is the one there is, and main9 takes
	// its id as an argument.
	la	sp, stack	// set the stack pointer
	li	t0, 4096 * 4
	add	sp, sp, t0	// add stack length
	call	main9
1:
	wfi
	j	1b

.bss
.balign 4096
stack:	.space 4096 * 4
