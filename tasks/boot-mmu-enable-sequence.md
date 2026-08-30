---
id: 141
status: open
wave: 6
---

# Task 141: the MMU-enable sequence is not the canonical one

## Status: open

## Problem

Three independent ordering gaps around the translation enable, all
QEMU-invisible (audit D1–D3):

**D1 (fable, opus).** `l.S:132-135` read-modify-writes `SCTLR_EL1`,
setting only `M|C|I`, over **UNKNOWN** reset state. Bits left at their
previous-stage values include `EE` (endianness — if set, everything
breaks silently), `WXN` (if set, the identity block becomes
non-executable and the very next fetch after the MMU write faults),
`A/SA/SA0`, `nAA`, `SPAN`, `UCI`, `DZE`… Several `SCTLR_EL1` fields are
architecturally UNKNOWN at reset; nothing in the EL3/EL2 path
initialises SCTLR_EL2 either. Linux's head.S writes a full value for
exactly this reason; QEMU resets benignly.

**D2 (fable, opus).** Nothing invalidates the D-cache (`dc isw` sweep) or
I-cache (`ic iallu`) before `l.S:135` enables M|C|I in one write. The
page tables were written with the MMU off (uncached accesses) while
`TCR_EL1.IRGN1/ORGN1` (`pre_mmu/vminit.rs:296-304`) program the walker
for write-back cacheable accesses; any line a previous boot stage left in
the caches over the tables, the image or the BSS becomes authoritative.
The Pi 4's armstub runs with caches enabled, so "reset leaves them
invalid" is not safe on the target. QEMU models no caches — no test can
ever show this.

**D3 (fable).** `init_vm` ends with `tlbi vmalle1is; dsb ish`
(`vminit.rs:315-321`); `l.S` then does `dsb sy; mrs sctlr; orr; msr
sctlr; isb` (`l.S:131-140`). The canonical Arm sequence (DEN0024) puts
an ISB *after* the TTBR/TCR/MAIR writes and *before* SCTLR.M=1. The
hazard is the window *between* `msr sctlr_el1` (`l.S:135`) and `isb`
(`l.S:140`): in it, instructions may fetch under the new SCTLR while the
TTBR/TCR/MAIR writes are not yet architecturally visible to the
translation machinery — a post-hoc ISB does not close a window that has
already been executed through.

## Design

- Write a complete known `SCTLR_EL1` value (RES1 ORed with the wanted
  bits) in the EL3→EL1 path, not an OR onto garbage.
- Invalidate the D-cache and `ic iallu` + `dsb`/`isb` before the SCTLR
  write.
- Match the canonical sequence: TTBR/TCR/MAIR writes; `isb`;
  `msr sctlr_el1, M|C|I`; `dsb`; `isb`.
- These are the three items whose correctness QEMU cannot adjudicate; the
  in-image value pins below give the task-127 Pi session a spec to
  compare against, and a `docs/` note should mark this set QEMU-unverifiable.

## Tests

- D5.3: `SCTLR_EL1.{M,C,I}` set and `{EE,WXN}` clear at `main9` entry.
- D5.4: `TCR_EL1` equals the exact value `init_vm` writes (T0SZ, T1SZ,
  TG0/1, IPS, SH0/1, IRGN0/1, ORGN0/1, EPD0/1) — a TCR field dropped in
  a refactor is caught here.
- D5.6: `MAIR_EL1 == 0x04ff` and attr1's Device encoding matches the
  documented intent (audit C1: `0x04` is Device **nGnRE**, not nGnRnE).
- D5.5: `TCR_EL1.IPS <= ID_AA64MMFR0_EL1.PARange` (`IPS::Bits_44` on a
  part with a smaller PARange is CONSTRAINED UNPREDICTABLE).
- Metal validation in task 127's Pi 4 session; record the transcript
  there.

## Done when

- The sequence matches DEN0024, the full-SCTLR write is in place, and the
  value pins above pass in an image.
- The QEMU-unverifiable set is noted in `docs/` so green CI isn't
  mistaken for coverage.
- Full `cargo xtask ci` green.

Origin: pre-`main9` boot audit, 2026-08-30
(`plans/premain-boot-review-2026-08.md`, D1–D3, checklist D5.3–D5.6).
