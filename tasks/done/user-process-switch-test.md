---
status: done
---

# user_process test: cross the line it stops short of

`aarch64/tests/user_process.rs` builds the whole first-process scenario
and deliberately does not take the switch — its doc comment says
asserting the switch needs the syscall path to return first, which
trap-svc-exit and process-run now provide.

Change the image to:

1. Bring up interrupts like `main9` does (`boot::interrupts`) — the
   plan's decision: the process enters with IRQs unmasked, and the
   image proves the EL0→EL1 path rather than dodging it.
2. Replace the inline page/context setup with a call to
   `process::run` (text = the existing 12-byte `mov x0,#0; mov x1,#1;
   svc` sequence, renumbered to `svc #0` = SYSEXIT; same
   `USER_TEXT_VA`/`USER_STACK_VA`).
3. Keep the pre-switch checks that `run` no longer owns (none — they
   move into `run`'s invariants; the image keeps the *result* checks):
   after `run` returns, `check!` the status (0), iprintln a line a
   human reading the QEMU console can see ("process returned"), then
   `qemu::exit(qemu::PASS)`.

The image's value now: it is the only CI-executed proof that a process
actually entered EL0, called `svc`, and that the kernel resumed at the
point that started it — the prints on either side of the process's
lifetime are the assertion a host-side test could never make.

Update the doc comment accordingly (it currently documents the
stop-short position).

Done when: `cargo xtask integration-test --arch aarch64` passes with
the console showing pre-start output, the exit syscall's print, and
post-return output in that order; gates clean on all three arches.

Origin: plans/first-user-process.md — task 4 of 5.

## Done (12096bb, first-user-process branch)

The image now brings up interrupts and calls `process::run`; the test
passed and exposed two latent switch bugs that landed in the same
commit:

- swtch.S saved the context mirrored (pre-decrement stp's put x19
  last and the sp/spsr pair first) while the restore reads struct
  order from the saved address: the switch-back resumed with x30 from
  the x20 slot and sp/spsr from the caller's stale frame. The frame
  is now built in the repr(C) Context layout and the saved address is
  the frame's own base.
- SPSR_EL1H was 0x7, which sets the reserved M[1] bit (EL is M[3:2],
  SP is M[0]): the return to the kernel was an illegal exception
  return (QEMU sets PSTATE.IL and the resume faulted). EL1h is 0x5.
- syscall_exit also balances the trap_unsafe enter_interrupt it never
  exits, or the resumed kernel believes it is still in interrupt
  context.

Simplification from the task text: the process text is the 4-byte
`svc #0` rather than the 12-byte `mov x0,#0; mov x1,#1; svc` sequence
-- the registers set up are never read, so the bare syscall is the
smaller program that still proves the path.
