---
status: done
---

# console-server

**From:** [stage 5 console-server design](../plans/microkernel-console-server.md)
— the proof of concept: a user-space process maps and owns device MMIO.
**Depends on:** `console-mmapmmio` (the `SYSMAPMMIO` syscall).

## Context

The stage-5 proof image: the kernel spawns a process (the "console server").
The server calls `SYSMAPMMIO` to map the PL011 UART's MMIO into its own
address space, writes a byte to the UART, and exits 0. If the process exits
0 (no fault on the MMIO access), the model is proven: a user-space process
can own and access device MMIO.

The kernel is device-dumb: it does not know about the PL011, does not parse
the DT for the UART's address, and does not map the MMIO. The server knows
its platform (the PL011 is at `0xfe201000` on BCM2711) and requests the
mapping itself via `SYSMAPMMIO`.

The PL011 register map (PL011 TRM r1p2 §3.3):
- DR (data) at offset 0x00: write a byte to transmit.
- FR (flags) at offset 0x18: bit 5 (TXIS) is 1 when the TX FIFO is empty.
- CR (control) at offset 0x30: bits 0,1,4 enable UART, RX, TX.

The UART must be enabled (CR) before the server writes to it. The kernel
enables it via its own PL011 mapping (via `deviceutil::map_device_register`,
as `uartpl011.rs` already does) — this is the *early* path, the kernel's
one device. The server's `SYSMAPMMIO` mapping is for the *process's* access.

The exit convention (from `two_process.rs`): `mov x8, #N; svc #0` exits
with status N. So `mov x8, #0; svc #0` exits with status 0.

## Changes

### `aarch64/tests/console_server.rs`

New integration image. The kernel (in the image's `main9`):
1. Parses the DT, initialises page allocator, console, interrupts.
2. Enables the PL011 UART via the kernel's own mapping (CR register, as
   `uartpl011.rs` does). This is the kernel's one device (the early path).
3. Spawns the console server process (text + stack at the conventional VAs).
4. Calls `process::run_all()` — the server runs.
5. The server calls `SYSMAPMMIO` (maps the PL011 into its TTBR0).
6. The server writes 'A' to the UART (proving MMIO access).
7. The server exits 0.
8. The kernel reports the exit status (expect 0).

The server's text (raw ARM64, ~12 instructions):
```
// Load PL011 base address (0xfe201000 on BCM2711)
MOVZ X0, #0x1000            // X0 = 0x1000 (low 16 bits)
MOVK X0, #0xfe20, LSL #16   // X0 = 0xfe201000 (physical address)
MOVZ X1, #0x200, LSL #16    // X1 = 0x20000 (user VA)
MOV  X8, #20                // SYSMAPMMIO
SVC  #0                     // map the MMIO into this process's TTBR0

// X9 = 0x20000 (the MMIO base, now mapped)
MOVZ X9, #0x200, LSL #16

loop:
LDR  W0, [X9, #0x18]        // read FR
TST  W0, #0x20              // TXIS (bit 5)?
B.EQ loop                    // wait for TX FIFO empty
MOV  W1, #65                // 'A'
STR  W1, [X9, #0x00]       // write DR
MOV  X8, #0                 // exit(0)
SVC  #0
```

Encodings must be verified at implementation time (as in stages 2–4). The
`MOVZ`/`MOVK` with `hw=1` (LSL #16) needs a new `const fn` helper or an
inline constant (the existing `const fn mov` handles hw=0 only).

### `aarch64/src/uartpl011.rs` (or `boot.rs`)

The kernel must enable the PL011 (CR register) before the server writes to
it. The existing `Pl011Uart::init()` does this (it writes the CR register).
The test image can call `Pl011Uart::new(dt)` + `init()` to set up the UART
on the kernel side. The server's `SYSMAPMMIO` mapping is independent (it
maps the same physical registers into the process's TTBR0).

Alternatively, the test image can write the CR register directly via the
kernel's mapping (simpler than instantiating the full `Pl011Uart`).

### `aarch64/Cargo.toml`

Add the `[[test]]` entry for `console_server` (harness = false,
required-features = ["qemu-test"]).

## Tests

- The `console_server` image passes under QEMU (aarch64): the server calls
  `SYSMAPMMIO`, writes to the PL011 MMIO without faulting, exits 0.
- xtask green across all three arches.

## Done when

- The `console_server` image passes: the server maps the PL011 via
  `SYSMAPMMIO`, writes 'A', and exits 0 (no fault).
- The kernel does not parse the DT for the PL011's address or map it into
  the server's AS (the server does this itself via the syscall).
- xtask green across all three arches (fmt, check, clippy ×3, test, dist ×3,
  integration-test).
