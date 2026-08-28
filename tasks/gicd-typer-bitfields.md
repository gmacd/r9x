---
status: open
---

# Fix `GicdTyper` bit fields against IHI 0048B.b Table 4-6

`GicdTyper` (aarch64/src/gic.rs:119-124) is constructed at gic.rs:245
(`it_lines_number()` sizes the distributor sweeps), but two of its four
fields do not match the GICv2 architecture:

| field            | code (gic.rs)  | IHI 0048B.b Table 4-6 / Figure 4-3 |
|------------------|----------------|------------------------------------|
| `it_lines_number` | `0..5`        | `[7:0]` — 8 bits                   |
| `cpu_number`      | `5..8`         | `[9:8]` — 2 bits                   |
| `security_extn`   | `10..11`       | `[10]` — correct                   |
| `lspi`            | `11..16`       | `[15:11]` — correct                |

(`[31:16]` is reserved. The layout is also in
`docs/hardware/gicv2.md#registers`, which the page's verified stamp now
backs.)

## Why it is wrong today but not misbehaving

- `it_lines_number` reads only `[4:0]`. Behaviourally safe on conforming
  hardware — the architectural value is 0..31 (1..32 blocks), so bits
  `[7:5]` are zero — but the 5-bit definition can never surface a
  non-conforming value and documents the wrong architecture.
- `cpu_number` reads `[7:5]`, the top three bits of `ITLinesNumber`, and
  never touches the real field at `[9:8]`. On the GIC-400 (BCM2711: 4 CPU
  interfaces → `CPUNumber = 3`; `ITLinesNumber = 31`) the register reads
  `0x0000_0B1F`, so `cpu_number()` returns **7**, not 3. Unused today
  (only `it_lines_number()` is read), so nothing misbehaves.

## Why it matters before SMP

[0011](../docs/decisions/0011-multicore-is-imminent.md) is the standing
decision that multicore is imminent. Secondary bringup will want the CPU
interface count (per-core `init_cpu`, per-core banked INTID 0..31 sweeps,
core-id bookkeeping), and `cpu_number()` as currently defined is exactly
the field it would reach for — with a wrong value. Fixing the definition
now is two lines; discovering the overlap under SMP is a debugging
session.

## Task

- `it_lines_number: u8 = 0..8`, `cpu_number: u8 = 8..10` (gic.rs:120-121).
- Leave `security_extn`/`lspi` as they are.
- Add the spec citation to the bitstruct or the register-offset comments:
  IHI 0048B.b §4.3.2 (GICD_TYPER), Table 4-6 — the same convention nit
  item 5 of `gic-timer-review-nits.md` asks for at the offsets.
- The Debug impl (gic.rs:127-135) needs no change; it will then print
  truthful fields.

Done when: the bitstruct matches Table 4-6 field for field;
`cargo xtask check`/`clippy` clean on all three architectures;
`cargo xtask test` passes. No behaviour change expected (the one consumer
reads a value the width change does not alter on conforming hardware) — if
a test or boot log disagrees, that is the finding, not the fix.

Origin: os-review panel of the docs/skills working diff (2026-08-28); the
spec layout was extracted from IHI 0048B.b itself (Table 4-6 and Figure
4-3). Related: `gic-timer-review-nits.md` item 3 (which predicted
`GicdTyper` would come back for INTID validation) and item 5 (spec
citations at the registers).
