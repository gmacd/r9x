---
status: done
---

# irq-integration — the IRQ→message integration image

## Problem

The routing table, `try_send`, and `SYSIRQCLAIM` exist but are not exercised
end-to-end under QEMU. The integration image proves the full path: a
user-space process claims a SPI, the GIC fires, the process receives a
message on its channel.

## Evidence

- `irq-route` (preceding task): the routing table, `try_send`, and
  `SYSIRQCLAIM` are implemented.
- The existing integration images (`ipc.rs`, `aspace.rs`, `aspace_fault.rs`)
  show the pattern: spawn a process, run it, assert on its exit status.
- QEMU `raspi4b` has the PL011 UART at INTID 129 (a SPI in the GICv2 range).
  The UART can be triggered by writing to its DR register (from the kernel,
  before the process runs).

## Fix direction

- A new integration image (`aarch64/tests/irq.rs`) that:
  1. Spawns a process that calls `SYSIRQCLAIM(129, channel_handle)` to claim
     the PL011 UART's SPI.
  2. The process then calls `SYCRECEIVE(channel_handle)` and blocks.
  3. The kernel (after the process blocks) writes a byte to the PL011 DR
     register (triggering the UART's TX interrupt, INTID 129).
  4. The GIC fires, the trap handler looks up the routing table, calls
     `try_send`, and wakes the blocked process.
  5. The process receives the message (opcode = 129), exits with status 129.
  6. The image asserts the process exited with status 129.

- The kernel's "write a byte to the PL011 DR register" is a test-only
  helper in the image (not in the kernel proper): the image maps the PL011
  MMIO (via the existing `map_device_register`), writes a byte, and the
  UART's TX interrupt fires.

- A `[[test]]` entry in `aarch64/Cargo.toml` for the new image.

## Done-when

- `cargo xtask ci` is green, including the new image.
- The image prints "irq passed" and exits with `qemu::PASS`.
- The process exits with status 129 (the INTID it claimed).

## Origin

Stage 4 of `tasks/plans/microkernel-substrate.md`, designed in
`tasks/plans/microkernel-irq-message.md`. Stands on `irq-route`. The
integration proof that the IRQ→message path works end-to-end under QEMU.
