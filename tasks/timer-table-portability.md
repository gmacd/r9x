---
status: open
---

# Plan the timer table's move to port/ at the second architecture

The timer subsystem (aarch64/src/timer.rs) has grown to ~600 lines
(2026-08-27 audit; the task originally said ~200 — the table,
`TimerCallback`, `arm_hardware`, plus SMP CVAL coordination), of which
only a modest arch-specific share touches aarch64: `now()`, the
CNTFRQ-based tick conversion, and `arm_hardware()`. The table itself —
registration, earliest-deadline selection, fire/re-arm/cancel semantics,
the TimerCallback trait — is architecture-independent, and riscv64 and
x86-64 will need exactly it. The growth strengthens the "lift rather
than copy" intent: the eventual port-lift is bigger than first scoped.

Trigger status: unfired — riscv64 has only a legacy SBI `_set_timer`
stub (riscv64/src/sbi.rs:37-38), x86_64 has no timer module. Natural
trigger: task 74b (the SYS_SPAWN port), since `SYS_RECEIVE_AT`/
`SYS_CLOCK` on a second arch need the table.

Writing it arch-local first was right (hoisting now would be speculative
generality). The wrong outcome is the default one: per-arch copies accreting
by copy-paste when the second architecture arrives.

Task: when riscv64 or x86-64 grows a timer, lift the table into `port/`
behind a small arch trait (now / duration_to_ticks / arm / disarm) rather
than copying. Until then this file is the recorded intent; no code change
now.

Done when: either the table lives in port/ with one arch backend per
architecture, or a deliberate decision to keep per-arch copies is recorded
here instead.

Origin: panel review of 46a59c9 (whole-system lens — composed
essentials).
