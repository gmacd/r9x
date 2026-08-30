---
id: 140
status: open
wave: 5
---

# Task 140: `l.S` entry paths don't converge — CPACR, DAIF and the MPIDR mask

## Status: open

## Problem

**B3 (fable, opus; VERIFIED code path, consequence SUSPECTED — QEMU and Pi
firmware both enter above EL1).** `aarch64/src/l.S:64-69` branches
straight to `el1:` when `CurrentEL == EL1h`, bypassing the `el2:` block
that writes `CPACR_EL1.FPEN` (`l.S:91-93`) and the `eret` paths that load
SPSRs with all DAIF masked (`l.S:76,88`). The target compiles with
`+neon,+fp-armv8`, so `memcpy`/`core::fmt` in `init_vm` may use SIMD
freely; with FP trapped, the first FP instruction vectors into the unset
`VBAR_EL1` (task 135's window). `main9` also assumes IRQs are masked
until `irq::unmask_irqs()` (`aarch64/src/main.rs:31-41`); a direct-EL1
handoff inherits whatever the previous stage left.

Related state left at reset on r9's own EL3→EL2 path (`l.S:84-98` writes
only `HCR_EL2.RW`): `CPTR_EL2.TFP` (traps EL1 FP to EL2),
`CNTHCTL_EL2` and `CNTVOFF_EL2` — the last directly relevant given the
generic-timer focus in `AGENTS.md`. QEMU's reset values happen to permit
EL1 counter access; the real armstub sets all of these.

**D8 (fable, opus; benign on Pi 4, single cluster).** `l.S:52-55` masks
MPIDR with `0xff` (Aff0 only): a core with `Aff0 == 0` in a non-zero
cluster races core 0 through the whole boot path. `AGENTS.md` says the
project is designed for multi-core SMP; masking `0xffff` costs nothing.
Separately, parked cores `wfe`/`b` forever at their entry EL, MMU off,
with no release protocol — nothing says SMP bring-up needs a different
entry.

**D19.** (audit design-risk list) parked secondaries spin at reset; the
comment should name the missing protocol instead of implying
`wfe`-parking is the bring-up.

## Design

- Move `msr cpacr_el1, FPEN` and an explicit `msr daifset, #0xf` into the
  `el1:` block so all three entry paths (EL3→EL2→EL1, EL2→EL1, direct
  EL1) converge on the same register state.
- Widen the MPIDR mask to `0xffff` (Aff0 + Aff1).
- Comment the parked-core path: it is a park, not a bring-up; SMP entry
  needs its own protocol (task 124 owns bring-up).
- For the metal path, set `CPTR_EL2.TFP`, `CNTHCTL_EL2` and
  `CNTVOFF_EL2` explicitly (validate in task 127's Pi session; QEMU's
  reset values hide it).

## Tests

- D5.7: `DAIF` has I and F set at `main9` entry (and A, D per task 143's
  SError decision); I clear after `boot::interrupts()`.
- D5.8: `CPACR_EL1.FPEN == 0b11` at entry and an actual FP instruction
  executes without trapping.
- D5.10: `MPIDR_EL1 & 0x00ffffff == 0` for the core that reached `main9`.
- D1.8 (task 135's harness): **exactly one** boot banner — the only
  proposed test that can detect a *second* core also reaching the boot
  path, which D5.10 cannot.

## Done when

- All three entry paths leave the same CPACR/DAIF state, asserted in an
  image.
- The MPIDR mask covers the cluster, and the parked-core comment names
  the missing protocol.
- The EL2 register set for the metal path is set (or recorded as a
  task-127 item with the reasoning).
- Full `cargo xtask ci` green.

Origin: pre-`main9` boot audit, 2026-08-30
(`plans/premain-boot-review-2026-08.md`, B3, D8, D19, checklist D5.7–D5.10).
