---
covers: aarch64/src/gic.rs, aarch64/src/irq.rs, aarch64/src/timer.rs
sources: ARM IHI 0048B.b (GICv2 Architecture Specification) §4 register maps and Tables 4-1, 4-3, 4-4/4-5, 4-6, 4-15, 4-18/4-19, 4-45; BCM2711 ARM Peripherals (GIC-400); Linux witness include/linux/irqchip/arm-gic.h
verified: f76d96a (2026-08-28)
---

# GICv2 — Generic Interrupt Controller v2

> Reference for developing r9's GICv2 support.

**Scope.** The GICv2 architecture as r9 uses it: registers, the init
sequence, dispatch, and the Pi 4 / QEMU wiring. It does *not* track what is
implemented — that lives in `../../tasks/`. Where this page describes r9's own code it
names the file; those names are the `covers:` list above, and are what the
staleness check watches.

**Spec note.** The GICv2 architecture is **IHI 0048B** (issue B.b is the
2013 re-issue). GICv3 is a different architecture document (IHI 0065): it
moves the CPU interface into system registers, widens the INTID space, and
adds registers such as `GICD_IROUTERn` and the `GICD_CTLR.{ARE_NS, ARE_S,
DISABLE_G1A}` bits — none of which exist in GICv2. Mixing the two maps is a
frequent mistake, including in this page's first revision.

## Contents

1. [Overview](#overview)
2. [Registers](#registers)
3. [Interrupt Types](#interrupt-types)
4. [Initialization Sequence](#initialization-sequence)
5. [Interrupt Handling](#interrupt-handling)
6. [Priority and Groups](#priority-and-groups)
7. [r9 Current State](#r9-current-state)
8. [Device Tree Binding](#device-tree-binding)

---

## Overview

The Generic Interrupt Controller (GIC) is the standard ARM interrupt
controller for ARMv8-A systems. GICv2 is the version found on:

- Raspberry Pi 4 (BCM2711) → **GIC-400** — r9's target
- QEMU `-machine raspi4b` (r9's aarch64 test machine) → emulated GIC-400
- Raspberry Pi 3 (BCM2837) → GIC-400, but its generic-timer PPIs route
  through the bcm2836 local interrupt controller, not the GIC; out of scope
  (see `aarch64/src/gic.rs` module docs)

**r9 targets GICv2 / GIC-400 only.**

### Blocks

Two functional blocks, mapped as separate MMIO regions (two `reg` entries in
the device tree, **GICD first**):

- **Distributor (GICD)** — one per system: routing, masking, priority,
  group assignment for all interrupts.
- **CPU interface (GICC)** — one per processor: acknowledges pending
  interrupts, holds the local priority mask. Its registers are *banked* per
  CPU behind one set of addresses; so are the INTID 0..31 (SGI/PPI)
  distributor registers.

The GIC-400's CPU interface is the standard GICv2 map below — there is no
separate "legacy" CPU-interface layout to special-case.

---

## Registers

All offsets are from the IHI 0048B.b §4 register maps (distributor Table 4-1,
CPU interface Table 4-3). Ranges are the full run of a `n`-indexed register
block.

### Distributor (GICD)

| Offset        | Register          | Access | Notes |
|---------------|-------------------|--------|-------|
| `0x0000`      | `GICD_CTLR`       | RW     | Distributor control |
| `0x0004`      | `GICD_TYPER`      | RO     | Type |
| `0x0008`      | `GICD_IIDR`       | RO     | Implementer ID |
| `0x00C`–`0x07C` | —              |        | Reserved / IMPLEMENTATION DEFINED |
| `0x080`–`0x0FC` | `GICD_IGROUPRn` | RW     | Group 0/1; reset value IMPLEMENTATION DEFINED for GICv2 without Security Extensions |
| `0x100`–`0x17C` | `GICD_ISENABLERn` | W1S  | |
| `0x180`–`0x1FC` | `GICD_ICENABLERn` | W1C  | |
| `0x200`–`0x27C` | `GICD_ISPENDRn` | W1S    | |
| `0x280`–`0x2FC` | `GICD_ICPENDRn` | W1C    | |
| `0x300`–`0x37C` | `GICD_ISACTIVERn` | W1S  | |
| `0x380`–`0x3FC` | `GICD_ICACTIVERn` | W1C  | |
| `0x400`–`0x7FC` | `GICD_IPRIORITYRn` | RW   | 4 INTIDs per word |
| `0x800`–`0x81C` | `GICD_ITARGETSRn` | RW   | CPU target list — *this* is GICv2 SPI affinity (`GICD_IROUTERn` is GICv3) |
| `0x820`–`0xBFC` | —               |        | IMPLEMENTATION DEFINED |
| `0xC00`–`0xCFC` | `GICD_ICFGRn`   | RW     | Edge/level configuration |
| `0xD00`–`0xDFC` | —               |        | IMPLEMENTATION DEFINED |
| `0xE00`–`0xEFC` | `GICD_NSACRn`   | RW     | Non-secure access control (Security Extensions only) |
| `0xF00`       | `GICD_SGIR`       | WO     | Software-generated interrupt |
| `0xF04`–`0xF0C` | —              |        | IMPLEMENTATION DEFINED |
| `0xF10`       | `GICD_SPENDSGIRn` | W1S    | SGI set-pending |
| `0xF20`       | `GICD_CLRPENDSGIRn` | W1C | SGI clear-pending |
| `0xF3C`–`0xFFC` | —               |        | IMPLEMENTATION DEFINED |

**GICD_CTLR** (GICv2 bit assignments, Tables 4-4/4-5):

```
 Secure copy:      [31:2] Reserved  [1] EnableGrp1  [0] EnableGrp0
 Non-secure copy:  [31:1] Reserved  [0] Enable   (with Security Extensions,
                                               controls Group 1 forwarding)
```

There are **no** `ARE_NS` / `ARE_S` / `DISABLE_G1A` bits in GICv2 — those
appear in GICv3 (IHI 0065). r9 writes `0x1` (`gic.rs:283`).

**GICD_TYPER** (Table 4-6):

```
 [31:16] Reserved
 [15:11] LSPI          (Security Extensions only; reserved otherwise)
 [10]    SecurityExtn
 [9:8]   CPUNumber     (CPU interfaces minus one)
 [7:0]   ITLinesNumber (32-INTID blocks minus one; 1..32 blocks)
```

`ITLinesNumber` sizes the distributor sweeps (r9 does the same, `gic.rs:242`).
It says nothing about the INTID 0..31 banked private region, which always
exists.

**GICD_IPRIORITYRn** (Table 4-15): one 8-bit byte per INTID, priority in
bits `[7:3]`, `[2:0]` reserved — four INTIDs per 32-bit register.

**GICD_ICFGRn** (Table 4-18): one 2-bit `Int_config` per INTID, at
`[2F+1 : 2F]` within each byte. Architecturally, bit `[2F+1]` is
`0` = level-sensitive, `1` = edge-triggered; bit `[2F]` is reserved, but
early GICv1-derived implementations — the GIC-400 among them — use the
legacy encoding of Table 4-19 (`00` = edge, `11` = level). SGI fields are
read-only; PPI fields are IMPLEMENTATION DEFINED. r9 does not program
ICFGR.

### CPU interface (GICC)

| Offset       | Register       | Access | Notes |
|--------------|----------------|--------|-------|
| `0x0000`     | `GICC_CTLR`    | RW     | CPU interface control |
| `0x0004`     | `GICC_PMR`     | RW     | Priority mask: an interrupt forwards only while its priority is numerically *lower* than the PMR |
| `0x0008`     | `GICC_BPR`     | RW     | Binary point (priority/sub-priority split) |
| `0x000C`     | `GICC_IAR`     | RO     | Read = acknowledge |
| `0x0010`     | `GICC_EOIR`    | WO     | End of interrupt |
| `0x0014`     | `GICC_RPR`     | RO     | Running priority |
| `0x0018`     | `GICC_HPPIR`   | RO     | Highest-priority pending interrupt |
| `0x001C`     | `GICC_ABPR`    | RW     | Alias of BPR |
| `0x0020`–`0x002C` | `GICC_AIARn` | RO   | Aliased IAR |
| `0x0024`     | `GICC_AEOIR`   | WO     | Aliased EOIR (GICv2 only) |
| `0x0028`     | `GICC_AHPPIR`  | RO     | Aliased HPPIR |
| `0x002C`–`0x00CF` | —          |        | Reserved / IMPLEMENTATION DEFINED |
| `0x00D0`–`0x00DC` | `GICC_APRn`  | RW     | Active-priority registers |
| `0x00E0`–`0x00EC` | `GICC_NSAPRn` | RW   | Non-secure APRs |
| `0x00ED`–`0x00F8` | —           |        | Reserved |
| `0x00FC`     | `GICC_IIDR`    | RO     | Implementer ID; `arch_version` field distinguishes GICv1 from GICv2+ |
| `0x1000`     | `GICC_DIR`     | WO     | Deactivate (GICv2 only) |

**GICC_CTLR**: bit 0 `Enable`, bit 1 `EnableGrp1`; bits `[8:5]` are the
legacy IRQ/FIQ bypass-disable bits that firmware establishes — r9 preserves
them on every write (`GICC_CTLR_BYPASS_MASK`, `gic.rs:74`; Linux keeps the
same mask, `arm-gic.h` `GICC_DIS_BYPASS_MASK`).

---

## Interrupt Types

| Type | INTIDs  | Notes |
|------|---------|-------|
| SGI  | 0–15    | Software-generated; routed with `GICD_SGIR` (+ `GICD_SPENDSGIRn`) |
| PPI  | 16–31   | Per-processor; distributor registers for 0..31 are banked per core |
| SPI  | 32–1019 | Shared; routed with `GICD_ITARGETSRn` |

Generic-timer PPIs (SBSA convention; the table is repeated in
`aarch64/src/timer.rs:190-195`):

| INTID | PPI | Timer |
|-------|-----|-------|
| 26    | 10  | non-secure EL2 physical |
| 27    | 11  | virtual (CNTV) |
| 28    | 12  | EL2 virtual (VHE) |
| 29    | 13  | **secure** EL1 physical (CNTPS) |
| 30    | 14  | **non-secure** EL1 physical (CNTP) — the one r9 arms |

r9 parses the INTID from the devicetree rather than assuming it
(`timer_intid_from_dt`, `aarch64/src/timer.rs:218`): on bcm2711 and QEMU
raspi4b entry [1] of the `arm,armv8-timer` interrupts list is PPI 14 → INTID
30.

### GICC_IAR format (Table 4-45)

```
 [31:13] Reserved
 [12:10] CPUID        (SGI target; meaningless for PPI/SPI)
 [ 9:0]  Interrupt ID (0–1019)
```

- **1023 (0x3FF)**: no interrupt pending — a spurious read.
- There is no "is SPI" bit; PPI vs SPI vs SGI falls out of the INTID range.

`GICC_EOIR`: write the INTID back. Writing the spurious ID has no effect.

---

## Initialization Sequence

What r9 actually does (`aarch64/src/gic.rs`), in order. The contract: IRQs
stay masked until this completes, so an interrupt a prior boot stage left
asserted cannot arrive before a driver exists.

1. **Find and map.** Match compatible `arm,gic-400` (fallback
   `arm,cortex-a15-gic`). `reg[0]` is the **distributor**, `reg[1]` the CPU
   interface (`find_gicc_gicd_virtranges`, `gic.rs:364`).
2. **Architecture check.** Read `GICC_IIDR`; reject `arch_version == 1`
   (GICv1). `Gic::new`, `gic.rs:233-238`.
3. **Size the INTID space.** `GICD_TYPER.ITLinesNumber + 1` blocks; the
   INTID 0..31 banked region is fixed regardless (`gic.rs:245`).
4. **`init_distributor`** (boot core, IRQs masked): disable every INTID
   (`ICENABLER` sweep), clear every pending bit (`ICPENDR` sweep), write the
   default priority `0xa0` (Linux's `GICD_INT_DEF_PRI`) into every
   `IPRIORITYR` word, then `GICD_CTLR = 0x1`. Firmware state is treated as a
   claim, not truth: an interrupt left enabled fires with no handler, one
   left pending is armed, and one left at priority 0xff equals the PMR
   about to be set and can never be delivered — silently.
5. **`init_cpu`** (every core that takes interrupts): clear the banked
   `ICACTIVER`/`ICENABLER` for INTID 0..31, rewrite the banked
   `IPRIORITYR` words to the default, `GICC_PMR = 0xff` (masks nothing —
   lower priority value = more urgent, 0xff admits everything below it),
   then `GICC_CTLR |= 1` as a read-modify-write that preserves firmware's
   `[8:5]` bypass bits.
6. **Per-IRQ enable** (drivers, after their init):
   `GICD_ISENABLER[n/32] |= 1 << (n % 32)`. For INTID 0..31 this lands in
   the calling core's bank only.

The timer is enabled last of all, and only after the hardware is disarmed
(`timer::init`), so a firmware-armed timer cannot be admitted pending into a
half-built handler.

---

## Interrupt Handling

The real path (`aarch64/src/trap.rs:231-261`):

```
IRQ vector
  → gic::try_ack_interrupt()      // read GICC_IAR; None if INTID ≥ 1020
  → dispatch on INTID:
      timer::intid()              // the timer PPI (DT-derived)
      ipc SGI range               // kernel↔user IPC
      unclaimed                   // gic::disable_interrupt(intid), loudly
  → gic::end_interrupt(iar)       // write GICC_EOIR with the IAR value
```

The one ordering rule that matters: **deassert the source before EOI** for
level-triggered interrupts (the timer PPI is level). An EOI before
deassertion immediately re-raises the interrupt (`gic.rs`, `end_interrupt`
docs). Edge-triggered interrupts are released by the EOI.

The GIC driver is published lock-free as `Once<Gic>` (`gic.rs:189`):
everything after init is per-core banked state or a single write-1-to-set
register access, so there is no read-modify-write to race.

---

## Priority and Groups

- 8-bit priority per INTID, value in `[7:3]`: **lower value = higher
  priority**. An interrupt is forwarded only while its priority is numerically
  lower than the core's PMR.
- r9 programs default priority `0xa0` and PMR `0xff`: the default is
  admitted, and the upper half of the space is left for interrupts that must
  pre-empt it.
- **`GICC_BPR`** (binary point) splits the 5 effective bits into
  priority/sub-priority; r9 does not program it — the value is whatever the
  IMPLEMENTATION DEFINED reset left.
- **Groups.** r9 does not program `GICD_IGROUPR` at all. For GICv2 without
  the Security Extensions the IGROUPR reset values are IMPLEMENTATION
  DEFINED (spec §4.3.4); on r9's targets the boot firmware leaves the
  controller in a state that delivers to non-secure EL1, and r9 inherits
  that. Both QEMU raspi4b and the Pi 4 deliver their interrupts without any
  group configuration.

---

## r9 Current State

What `aarch64/src/gic.rs` does today (verified against `f76d96a`):

- GIC-400 discovery by compatible; GICv1 rejected via `GICC_IIDR`
- DT `reg[0]`=GICD / `reg[1]`=GICC mapping
- full distributor sweeps sized from `ITLinesNumber`
- per-core banked-state re-establishment (`init_cpu`)
- per-INTID enable/disable, ack/EOI, spurious handling
- timer PPI enabled from the DT-derived INTID

What it does not do: program `GICD_IGROUPR`, `GICD_ICFGR`, or
`GICD_ITARGETSRn`; send SGIs; call `init_cpu` from a secondary-core bringup
(the function exists, the caller does not).

**Status and next steps live in `../../tasks/`** — per this page's scope
note, a knowledge-base page tracks what is true, not what is planned.

---

## Device Tree Binding

```dts
interrupt-controller;
compatible = "arm,gic-400";
/* reg[0] = distributor (GICD), reg[1] = CPU interface (GICC) — order matters */
reg = <0x0 0x41000000 0 0x10000>,
      <0x0 0x41010000 0 0x10000>;
```

Interrupt specifiers on r9's targets are 3 cells — `(type, intid, flags)` —
which is how `arm,armv8-timer`'s `interrupts` list is parsed
(`aarch64/src/timer.rs:218-253`, verified against the live DTBs of both
supported machines):

- type: `0` = SPI, `1` = PPI, `2` = SGI
- flags: `1` = edge rising, `2` = edge falling, `4` = level high,
  `8` = level low

Example — the bcm2711 timer node's non-secure EL1 entry: PPI 14, level high
→ INTID 30.
