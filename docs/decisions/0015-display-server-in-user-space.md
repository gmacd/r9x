---
status: accepted
---

# 0015 — The display server is a user-space process with pluggable sink and pacing

- **Status**: accepted — in progress (`cmd/display`; `SYS_RECEIVE_AT` at `abi/src/lib.rs:135`)
- **Date**: 2026-08-28 (record written from `tasks/plans/r9-display-server.md`)
- **Context**: `tasks/plans/r9-display-server.md`; the design goal in `.agents/skills/references/amiga-inspiration.md`

## Decision

The display server is an ordinary user process that owns the framebuffer and
paces its own frame loop; the kernel never touches display hardware. Both the
frame sink and the pacing source are pluggable along the same axis — the
machine:

| | QEMU (raspi4b) | Raspberry Pi |
|---|---|---|
| Frame sink | GPU framebuffer via `SYS_MAP_MMIO` (VC-RAM configured over `/dev/mailbox`) | the same path |
| Pacing | `SYS_RECEIVE_AT` deadline — the machine wires no vblank interrupt | vblank interrupt message, deadline as fallback — **pending** |

The sink axis has collapsed: `cmd/display` takes the mailbox + `SYS_MAP_MMIO`
path on both machines today (`cmd/display/src/main.rs`); pacing is the only
live axis, and the vblank trigger is what earns the second pacing path.

The proof image draws a moving pattern, not a blank frame.

## Why

The standing goal is that r9x boots to a graphical environment and the kernel's
only real-time duty is delivering the vblank interrupt with bounded latency —
the three-thing budget of [0008](0008-irq-to-message-routing.md). The frame
loop belongs to the server, exactly as a display list belongs to the display
processor rather than the CPU.

The remaining split is not a design preference but a hardware fact: QEMU's
raspi4b emulates the BCM283x mailbox and VC-RAM — the same sink path as the
Pi — but wires no vblank interrupt; the Pi has one. A moving pattern is what
makes "the frame loop is running" observable — a blank frame proves nothing.

## Alternatives rejected

- **A kernel-side framebuffer driver or display subsystem.** Lost: it puts a
  device and a real-time policy in the kernel, against
  [0002](0002-qnx-mechanism-plan9-interface.md) and
  [0007](0007-device-dumb-kernel.md).
- **A software (heap) frame sink.** Lost: invisible on both real targets; the
  mailbox + `SYS_MAP_MMIO` path is available on QEMU's raspi4b and on the Pi,
  so there is one sink, not two. The heap buffer that remains (the back
  buffer for double buffering) is an implementation detail of the sink, not
  a second one.
- **Timer pacing everywhere.** Lost: on the Pi the vblank interrupt is the
  heartbeat, and a timer would be a permanent fallback masquerading as the
  design.

**Dissent** (whole-system lens): two sinks and two pacing sources are more
concepts than one. Held: the second concept is the hardware's, not the
design's.

**Dissent** (microkernel-and-firmware lens): the kernel should own the 60 Hz
real-time duty. Held: the kernel's duty is bounded-latency delivery; the frame
loop is the server's.

## Consequences

- Frame timing on QEMU is effectively 10 Hz (a 16.7 ms deadline checked at each
  100 ms tick) — a test-harness property, not the design's target.
- The server needs its GPU range granted once
  [0010](0010-map-mmio-becomes-a-capability.md) lands; until it does, the
  mailbox + `SYS_MAP_MMIO` path is the whole of the sink axis on both
  machines, and "pluggable sink" in the title is prospective, not present.
- The vblank pacing path is the next arc: it lands when the interrupt is
  wired on the Pi, not before — a pacing source with no source earns nothing.
- A stalled display server is visible as a frozen pattern rather than a hang.
