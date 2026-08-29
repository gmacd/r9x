---
id: 93
status: done
commit: 742a8bc
---

# Task 93: Decode the DFSC in the EL0 fault print

## Status: done (742a8bc)

## Problem

`process::fault` prints the raw syndrome:
`process 1 faulted: far 0x700000c4 esr EsrEl1 { iss: 0x00000010, ... }`.
The reader has to decode ISS[5:0] by hand, and task 87's misdiagnosis
("translation fault, PTE not valid" for what was actually `0x10` =
synchronous external abort) stood for a whole task file because nothing
printed the fault *class*.

## Design

Decode DFSC (Data Abort) / IFSC (Instruction Abort) into a string in the
fault print, the way Linux's `fault_info` table does
(`arch/arm64/mm/fault.c:917-929`):

- `0b0001xx` — translation fault, level 0–3
- `0b0010xx` — access flag fault, level
- `0b0011xx` — permission fault, level
- `0b010000` — synchronous external abort
- `0b100001` — alignment fault
- everything else — print the raw bits with "unknown"

The decode lives next to `EsrEl1` (`aarch64/src/reg/esr_el1.rs`) so it is
host-testable: one unit test pins the strings for the values above (the
audit's point — a fault printer wrong in the same way as the raw print
would be worse than none).

Example target output:
`process 1 faulted: far 0x700000c4 external abort (esr ...)`
`process 2 faulted: far 0x40000000 translation fault L1 (esr ...)`

## Done when

- The EL0 fault path prints the decoded class + level alongside FAR/ESR.
- A host unit test pins the decode for translation/permission/external
  abort values.
- The `aspace_fault` integration image's expected output is updated.
- Full `cargo xtask ci` green.

Origin: backlog audit 2026-08-27 (VM group — the raw ISS print is what
let 87's misdiagnosis stand).
