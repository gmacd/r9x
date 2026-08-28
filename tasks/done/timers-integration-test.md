---
status: done
---

# Timers integration test; the trap.S sp/x0 corruption it uncovered

Not a pre-existing task: it grew out of the 2026-08-21 question "do we
have any integration tests covering timers?"  The answer was no — the
unit tests mock the counter, and the only end-to-end firing was main9's
pc1/pc2 ticker demo, watched by eye on a manual qemu run.

## Done (1ad4c42 trap fix, c93cfda test image)

`aarch64/tests/timers.rs` moves the demo scenario into an asserted
image: a 5ms periodic re-arms until a 40ms one-shot cancels it, a 10ms
periodic stops itself at its 3-fire limit, and after a settle period
every counter is checked unchanged.  Waits are bounded by counter time
(CNTPCT/CNTFRQ), so a dead timer fails the image rather than hanging to
the runner's timeout.  main9 lost the demo; the kernel binary is boot
sequence only again.  `xtask integration-test` now runs 5 images.

The image's first run instruction-aborted at the GIC's mapped VA and
exposed two latent trap.S bugs, fatal to any code that keeps running
after an interrupt (the demo's idle `loop {}` used neither sp nor x0,
which is the only reason the tickers ever worked):

- every vector reset sp to the interrupt stack top and never saved the
  interrupted sp — eret restores no stack pointer, so interrupted EL1
  code resumed on the interrupt stack, and the next interrupt overwrote
  the live frames there;
- the stack switch clobbered x0 before the frame saved it, so every
  interrupt restored garbage into the interrupted code's x0.

Fix (1ad4c42): the vector macro split by origin.  EL1 exceptions push
the frame on the stack they interrupted (sp correct by construction,
nested exceptions stack instead of overwriting); EL0 exceptions keep
the dedicated interrupt stack — SP_EL1 is not a live kernel stack while
a process runs — parking x0 in TPIDR_EL1 across the switch.  The
frame's padding slot now records the interrupted sp (SP_EL0 for EL0
entries) and TrapFrame grew the matching field.

Note for SMP bringup: TPIDR_EL1 is used as a scratch register in the
EL0 vector path on the single-core assumption; if it later becomes the
per-core pointer, that path must switch to another stash.
