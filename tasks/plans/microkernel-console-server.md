# Console server: first driver across the IPC boundary

## Problem and constraints

Stage 5 of the microkernel substrate. The proof of concept: one driver (the
PL011 UART) moves from kernel-resident to a user-space process that owns the
device's MMIO. The kernel adds a `SYSMAPMMIO` syscall so a process can map a
physical page into its own address space with device memory attributes. The
console server calls the syscall to claim the PL011; the kernel is
device-dumb (it provides the capability, not the device knowledge). The
early console (MiniUart) stays in the kernel for boot messages.

Standing constraints: warning-free across aarch64/x86-64/riscv64; minimal
scoped changes; Plan 9 shape; aarch64 is the reference implementation.

## Prior art

**r9 already has the hard parts.** `Aspace` (stage 3) gives each process an
isolated TTBR0 root. `map_user_page` maps a pagealloc'd page into both TTBR0
and TTBR1. `port::ipc` (stage 2) gives channels with PI. The IRQ→message path
(stage 4) routes a SPI to a channel. The `Pl011Uart` driver exists in
`aarch64::uartpl011`. `deviceutil::map_device_register` maps a device's
physical range into the kernel's TTBR1.

What is missing: a way for a *process* to map a *device's* MMIO into its own
TTBR0 (with device memory attributes), and a user-space process that uses it.

**Plan 9** — the console is a file server. The server owns the UART; clients
open/read/write. For stage 5 the server speaks a native opcode API (the
documented exception); 9P arrives in stage 7.

**QNX** — the resource manager calls `PhysMem` (a kernel syscall) to map
device MMIO into its own address space. The kernel provides the capability;
the resource manager decides which MMIO to map. The kernel is device-dumb.
This is the model r9 follows.

**Linux** — the user-space equivalent is UIO or `mmap` on a platform driver.
The microkernel inverts this: the *server* owns the MMIO, not the kernel.

## Hardware assumptions (required)

- **aarch64 (Pi 4 / QEMU `raspi4b`)**: PL011 is a 4K MMIO region at
  `0xfe201000` (BCM2711 peripheral base + UART0 offset; a hardware constant,
  confirmed by the DT). SPI is INTID 129 on QEMU raspi4b. The MMIO mapping
  uses `Device` memory (MAIR index 1, inner-shareable, XN both, AllRw).
  Arm ARM DDI 0487 §D7.2, Table D7-3.
- **x86-64 / riscv64**: gate-green only (the syscall is arch-agnostic in
  shape; each crate provides its own `map_mmio` when its Aspace lands).
- **Memory ordering**: `Device` attribute ensures no caching or reordering.
  No explicit barriers beyond what the attribute provides.
- **Firmware**: PL011 is free on QEMU raspi4b (the MiniUart is the firmware
  console). On a real Pi 4, firmware may own UART0 — a later concern.

## Design

### Data structures

No new persistent data structure. `map_mmio` adds a page-table entry to the
process's existing TTBR0 root.

One new `Entry` constructor in `aarch64::vm`:

```
Entry::rw_user_mmio()  // AllRw, Device, XN both, InnerShareable
```

One new method on `Aspace`:

```
Aspace::map_mmio(&self, range: &PhysRange, va: usize) -> Result<(), PageAllocError>
```

Maps the physical range into the process's TTBR0 at `va` with
`Entry::rw_user_mmio()`. Does **not** map into TTBR1 (the kernel does not
need the device page; the server owns it exclusively). The range must be
page-aligned and ≤ one page for this arc (the PL011 is 4K).

### Interfaces

**`SYSMAPMMIO`** (syscall number 20):
- x0 = physical address (page-aligned)
- x1 = user VA (where to map it)
- Fixed 4K length (one page)
- Maps the physical page into the *current* process's TTBR0 with
  `Entry::rw_user_mmio()`. No TTBR1 mapping.
- Returns 0 on success, 1 on failure (bad address, mapping error).
- No permission check: the process is trusted to map only the MMIO it needs
  (single-tenant; the QNX model).

The kernel is device-dumb: it provides a generic "map this physical page
into my AS" capability. It does not know about the PL011, does not parse the
DT for device addresses, and does not decide which server gets which MMIO.
The server knows its platform and requests the mapping itself.

The console server's interface (for future clients) is a native opcode API
over a channel (the documented exception — the raw console is not a file):

```
OP_READ  = 1   // read a byte, reply with the byte
OP_WRITE = 2   // write buf[0], reply ok
```

For the stage-5 proof image, the server calls `SYSMAPMMIO`, writes a byte to
the UART, and exits. The channel is created but not yet used by a client.

### Init and bringup order

```
boot::irq_ops()            → trap, IRQ mask
boot::page_allocator()     → pagealloc
boot::console()            → early console (MiniUart)
boot::interrupts()         → GIC, timer, unmask
vm::init_user_page_tables() → shared user table
vm::switch(user)           → install user table

// Stage 5:
ipc::create()              → channel (for future clients)
process::spawn(...)        → console server (its own Aspace, text/stack)
process::run_all()         → server runs:
                             calls SYSMAPMMIO (maps PL011 into its TTBR0)
                             writes 'A' to the UART
                             exits 0
```

The `SYSMAPMMIO` call happens *inside* the server (after it starts running),
not before. The kernel does not map the MMIO — the server does, via the
syscall. This is the QNX model: the resource manager maps its own devices.

### Failure policy

- **`SYSMAPMMIO` with a bad address** (not page-aligned, or the page is not
  a valid device range): return error (1). The process handles it (or exits).
- **`map_mmio` mapping failure** (page-table walk error): return error (1).
  Init-only context is not required — the process can retry or exit.
- **Process faults on the MMIO** (e.g., accessing beyond the mapped page):
  the stage-3 fault handler kills the process.
- **Server crash**: kills only its Aspace. The early console (MiniUart) is
  unaffected.
- **No new panics in interrupt context.**

## Not building

- **Kernel-side MMIO assignment.** The kernel does not parse the DT for
  device addresses or decide which server gets which MMIO. The server knows
  its platform and requests the mapping itself. The kernel is device-dumb.
- **Permission checks on `SYSMAPMMIO`.** Single-tenant: the process is
  trusted. A multi-tenant system would add a per-process device whitelist.
- **A multi-page MMIO mapping.** The PL011 is one page. The method takes a
  `PhysRange`, so the generalisation is a loop, not a new API.
- **A 9P file server interface.** Native opcode for now; 9P is stage 7.
- **Retiring the early console.** The MiniUart stays. It is a different UART
  from the PL011. Retiring it requires the namespace (stage 6).
- **RX (input) in the stage-5 image.** The proof writes a byte (TX). RX
  (the UART's RX IRQ waking the server) is the next refinement.

## Decision records

- **Decision: the server calls `SYSMAPMMIO`; the kernel is device-dumb.**
  - Alternatives: the kernel calls `map_mmio` at spawn time (the kernel
    parses the DT, finds the PL011, and hands it to the server); a spawn
    parameter (the mapping is part of the `spawn` call).
  - Lost: the kernel-side approach makes the kernel device-smart — it knows
    about the PL011's address and decides which server gets it. This violates
    the "device-dumb, channel-routed" principle from the substrate design.
    The QNX model (resource manager maps its own devices via a syscall) is
    the correct shape: the kernel provides the capability, the server
    decides.
  - Dissent: the microkernel lens notes that without permission
    checks, any process can map any MMIO (including the GIC or timer).
    Accepted: single-tenant, the process is trusted. A multi-tenant system
    would add a per-process device whitelist — deferred.

- **Decision: the console server speaks a native opcode API, not 9P.**
  - Alternatives: 9P file server; raw MMIO with no IPC.
  - Lost: 9P is stage 7 (needs the namespace, Fid/Req pools). Raw MMIO with
    no IPC doesn't prove the IPC model. The native opcode is the documented
    exception.
  - Dissent: the simplicity lens wants only files. The raw console
    genuinely is not a file (it is a polled/interrupt-driven char device).
    Recorded, not averaged away.

- **Decision: `map_mmio` maps only TTBR0, not TTBR1.**
  - Alternatives: map both (like `map_user_page`); map only TTBR1.
  - Lost: mapping TTBR1 gives the kernel access to the device page, which
    defeats the purpose (the server owns the MMIO exclusively). Mapping only
    TTBR1 would make the MMIO unreachable from the process. TTBR0-only is
    the isolation property applied to devices.
  - Dissent: the hardware lens notes the kernel may need to
    read a device register for diagnostics. It can do so by mapping the
    range into TTBR1 explicitly (a `map_mmio_kernel` variant) — deferred until
    the need exists.

- **Decision: the stage-5 proof image is TX-only (write a byte).**
  - Alternatives: TX+RX (full echo); TX only.
  - Lost: RX exercises the full IRQ→message→server path, but the TX proof
    establishes the critical property (a user process can own and access
    device MMIO). RX is the natural next step but is not required to prove
    the model.
  - Dissent: the microkernel lens notes the interrupt path is the
    *point* of the microkernel. We agree, and file RX as the next task —
    the stage-5 arc is "MMIO ownership proven," not "full device server."

## Tasks

Ordered, filed in `tasks/`:

1. `console-mmapmmio.md` — `Entry::rw_user_mmio()` + `Aspace::map_mmio()`
   + `SYSMAPMMIO` handler. The kernel-internal verb and the syscall that
   exposes it to user space.
2. `console-server.md` — the integration image: spawn a process, the process
   calls `SYSMAPMMIO` to map the PL011 MMIO, writes a byte to the UART,
   exits 0. Proves end-to-end MMIO ownership from user space.

Sequencing: task 1 is a prerequisite for task 2 (the image needs the syscall).
