---
id: 135
status: open
wave: 0
---

# Task 135: pre-`main9` failures are silent hangs — make every one diagnosable

## Status: open

## Problem

Five independent properties multiply so that **every** pre-`main9` failure
on aarch64 is a wordless wedge (audit B4, finding raised by all three
reviewers):

1. **`VBAR_EL1` is not set until `main9`'s first statement**
   (`boot::irq_ops()` → `trap::init()`, the only `msr vbar_el1` in the
   tree, `aarch64/src/trap.rs:22`, called from `aarch64/src/main.rs:20`).
   From reset to there, any exception fetches its vector from the reset
   value (0 on QEMU, UNKNOWN elsewhere). VA 0 is mapped by TTBR0's 1 GiB
   identity block, so on QEMU the CPU *executes whatever is at PA 0* as
   EL1 vector code — a wedge, not even a fault.
2. **`init_vm` unwraps the DTB before its first `putstr`**
   (`aarch64/src/pre_mmu/vminit.rs:112` vs `:115`). The most common
   bring-up mistake (missing `-dtb`) dies at the unwrap with zero
   diagnostics.
3. **Pre-console panics print nothing**: `aarch64/src/runtime.rs:10-22`
   prints via `iprintln!` → `port::devcons`, whose sink is installed by
   `devcons::init` inside `main9`; the panic handler is a no-op writer in
   this window. (Refinement from the audit's final gate: a pre-MMU *panic*
   in a **test** image already produces a clean exit-1 via
   `qemu::exit(FAIL)`; what hangs unconditionally is *exceptions* before
   `VBAR_EL1` is set. "Silent" refers to the output, absent in both
   cases.)
4. **The mini-UART loses its address at MMU-enable**: its MMIO (PA
   `0xfe215040`) is mapped in TTBR1's half at all (TTBR1 maps only
   `[0, 0x2200000)` + the DTB); only TTBR0's identity block covers it —
   so from `l.S:135` to `boot::console` nothing can print even if a
   handler tried.
5. **CI discards the only visible channel**: xtask wires the mini-UART to
   `-serial null` (`xtask/src/main.rs:1758-1761`) and inspects exit
   status only; QEMU stdout is captured only under `--verbose`
   (`xtask/src/main.rs:1786-1794`). No test can observe anything
   `init_vm` prints.

The QEMU `raspi4b` boot contract compounds this (audit D7): the repo's
`bcm2711-rpi-4-b.dts` carries `memory@0 { reg = <0x00 0x00 0x00>; }`
(`aarch64/lib/bcm2711-rpi-4-b.dts:2051-2054`) — a zero-size memory node
that only works because QEMU rewrites it to 960 MiB; on any machine that
doesn't patch, the page allocator sees zero RAM. The vendor-style
filename implies a real device tree, and nothing in xtask fails if the
assembled command lacks `-dtb`.

## Design

- `l.S`: install a minimal early vector table in `.boottext` before
  `init_vm` — print `ESR_EL1`/`FAR_EL1`/`ELR_EL1` over the early UART,
  then park. This also covers SError and FP-trap exceptions in the window
  (task 141 and task 140 shrink it).
- `vminit.rs`: `putstr` before the DTB unwrap; a pre-MMU panic hook over
  `init_early_uart_putc` so `unwrap`/`panic!` before the console emit
  their location.
- Give the mini-UART a VA across the MMU window (map its page in the
  early tables) or document the window in which it is dead.
- xtask: capture QEMU stdout always (not only `--verbose`); wire the
  mini-UART (serial_hd(1)) to a captured sink instead of `null` and fix
  the PL011 comment at `xtask/src/main.rs:754`; per-image expected
  non-zero exit / failure message; per-image QEMU overrides (extra
  `-machine` opts, custom dtbs, omitted `-dtb`); a guard that fails if
  the assembled command lacks `-dtb`.
- Give the dtb a real 2 GiB memory node so the file is self-consistent,
  and document the contract (QEMU 11 `raspi4b`: `-dtb` mandatory for r9,
  machine RAM fixed at 2 GiB, memory node rewritten to 960 MiB, supplied
  dtb otherwise authoritative).

## Tests

- The golden early transcript (checklist D1.1–D1.8): the `.` then the
  banner, the four map lines in ascending `range.start` order with
  `va == pa + KZERO` and correct sizes, "switching" then "complete"
  before any PL011 output, byte-identical across runs, and **exactly one
  boot banner** (the only test that can catch a second core racing boot,
  task 140's `Aff0`-only MPIDR mask).
- **No line beginning `error:vminit:` ever appears** (D1.4) — the only
  observable pre-MMU diagnostics once task 136 lands; this is the
  highest-value harness check in the audit.
- Boot with `-dtb` omitted: a specific failure signature on the captured
  mini-UART, not silence (D6.2, reproduced today as just the dot).
- An image that deliberately exhausts the `EarlyPageAllocator` fails
  loudly (non-zero exit or an `error:` line), not a hang (D6.1).
- A feature-gated panic inside `init_vm` emits something (D7.4).

## Done when

- Any pre-`main9` exception or panic produces a diagnosable line on the
  captured serial before the machine is wedged.
- The golden transcript and the `error:vminit:` absence are checked in
  `cargo xtask ci`.
- The dtb's memory node is self-consistent and the contract is documented
  where the dtb is assembled.
- Full `cargo xtask ci` green.

Origin: pre-`main9` boot audit, 2026-08-30
(`plans/premain-boot-review-2026-08.md`, B4 + D7 + checklist D0/D1/D6/D7).
