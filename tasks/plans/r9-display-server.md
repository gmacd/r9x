# Display server: the 60 Hz heartbeat, a user-space process

## Problem and constraints

The standing goal: r9 boots to a graphical environment by default, and the
kernel's job is to keep the display alive at 60 Hz while user-space servers do
everything else (the Amiga heartbeat). Task 75 just landed (SYS_CLOCK +
SYS_RECEIVE_AT), which provides the pacing primitive a frame loop needs:
measure the deadline, sleep until it, no spin.

The display server is the first user-space process that:
1. Owns a framebuffer (a buffer in RAM on QEMU; the GPU's frame buffer on the
   Pi — the frame *sink*).
2. Paces its frame loop to 60 Hz (a timer on QEMU; the vblank interrupt on the
   Pi — the *pacing source*).
3. Writes a visible pattern (proof it is alive and running).

Standing constraints: warning-free for all three arches; the display server is
a user-space process (not a kernel subsystem); the kernel never touches the
framebuffer; the interrupt context budget is three things (lookup, enqueue,
wake) when a vblank interrupt exists; the frame loop is the only hot path and
it must not spin the CPU (it blocks on SYS_RECEIVE_AT).

## Prior art

- **r9 already has**: the console server (a user-space process that owns the
  PL011 UART MMIO via SYS_MAP_MMIO, publishes its name in the nameserver, and
  serves one round-trip). The display server follows the same shape: own the
  device MMIO, publish a name, serve requests. The console server is the
  template.
- **r9 already has**: SYS_ALLOC/SYS_FREE (per-process brk-style heap),
  SYS_SPAWN (init spawns servers by index), SYS_CLOCK/SYS_RECEIVE_AT (the
  pacing primitives). The display server composes all of these.
- **Plan 9**: the display is a device (`/dev/fb` or `/dev/display`), owned by
  a display server. The server is dumb (it owns the hardware); the client is
  smart (it draws). r9's display server is the same: it owns the framebuffer
  and provides a channel for frame submissions.
- **Linux**: the framebuffer console (`fbcon`) is a kernel driver — the
  kernel touches the display hardware. r9 refuses this: the display server is
  user-space, the kernel never touches the framebuffer.
- **QEMU `raspi4b`**: no VideoCore GPU (QEMU does not implement it). The
  machine runs with `-nographic` (no display output). The framebuffer is a
  software buffer in RAM — testable by reading it back, not visually
  verifiable.

## Hardware assumptions (required)

### aarch64 / QEMU `raspi4b`

- **No GPU**: the VideoCore VI is not implemented in QEMU. There is no vblank
  interrupt, no GPU MMIO, no frame buffer register. The display is a software
  buffer in RAM.
- **Pacing**: the timer (CNTPCT_EL0, 1 GHz on QEMU) is the pacing source.
  The display server blocks on `SYS_RECEIVE_AT` with a 16,666,667-tick
  deadline (≈16.7 ms = 1/60 s).
- **Frame sink**: a software framebuffer (a buffer in the process's heap,
  allocated via `SYS_ALLOC`). The display server writes pixels to the buffer.
  No `SYS_MAP_MMIO` (there is no MMIO to map).
- **Visibility**: `-nographic` means no display output. The framebuffer is
  testable (the integration image can read the buffer back and check the
  pattern) but not visually verifiable.

### aarch64 / Pi 4 (BCM2711)

- **VideoCore VI**: the GPU is present. The frame buffer is in SDRAM, mapped
  via the GPU's register page (the `framebuffer` node in the device tree,
  `BCM2835_FBTAG` / `bcm2708_fb` DT property `linux,framebuffer`). The
  display server maps the frame buffer via `SYS_MAP_MMIO`.
- **Vblank interrupt**: the VideoCore VI generates a VBLANK interrupt
  (~16.7 ms at 60 Hz). The display server owns the interrupt (the kernel's
  IRQ→message path delivers it to the display server's channel). The display
  server blocks on `SYS_RECEIVE_AT` on its vblank channel; the vblank
  interrupt's message wakes it.
- **Pacing**: the vblank interrupt is the pacing source. The timer is the
  fallback (if the vblank message is lost, the deadline fires).
- **Interrupt context budget**: the VBLANK IRQ handler does lookup (find the
  display server's channel), enqueue (send a message), and wake. Three
  things, within the budget.

### riscv64 / x86-64

- **Not yet**: the display server is aarch64-first (QEMU `raspi4b`). The
  riscv64 and x86-64 ports are mechanical follow-ups (the display server is
  arch-agnostic user-space code; the arch-specific parts are the frame sink
  and the pacing source, both pluggable).

## Design

### Data structures

The display server is a user-space process (like the console server). Its
central data structures:

- **Frame buffer**: a `[u8]` buffer in the process's heap (allocated via
  `SYS_ALLOC`). On QEMU: a software buffer (640×480×4 = 1,228,800 bytes,
  RGBA). On the Pi: the GPU's frame buffer (mapped via `SYS_MAP_MMIO`, the
  size from the device tree). The buffer is the *frame sink*: where the
  frame is written.
- **Pacing channel**: a channel (created via `SYS_CREATECHAN`). On QEMU: the
  display server blocks on the channel with a timer deadline (no one sends to
  it — the deadline is the wake). On the Pi: the display server blocks on the
  channel; the vblank interrupt's message wakes it (the deadline is the
  fallback). The channel is the *pacing source*: how the frame loop is paced.
- **Frame state**: the current frame number (a `u64` counter). The display
  server increments it each frame. The frame number drives the pattern (e.g.,
  the color bar's position).

The display server's main loop:

```
loop {
    // 1. Prepare the next frame: write the pattern to the frame buffer.
    write_frame(frame_buf, frame_number);

    // 2. Wait for the next frame's deadline (the pacing source).
    //    On QEMU: a timer deadline (no one sends to the channel).
    //    On the Pi: the vblank interrupt's message (or the timer deadline).
    let mut buf = [0u8; 0];
    let (op, _, _) = ipc::receive_at(pacing_chan, &mut buf, deadline);

    // 3. Advance the frame number.
    frame_number += 1;
}
```

The frame loop is the Amiga's demoscene routine: prepare the frame, wait for
the vertical blank, repeat. The kernel's job is to deliver the vblank
interrupt to the display server's channel with bounded latency (on the Pi);
on QEMU, the kernel's job is to fire the timer at the deadline (the
`check_deadlines` trap-tail scan).

### Interfaces

The display server's interface is a message channel (like the console server).
The display server publishes its name in the nameserver (`/dev/display`).
Clients send frame-submission requests on the display server's inbound
channel; the display server acknowledges on its outbound channel.

**Day-one users**: the integration image (the test that checks the frame
buffer contents). There is no client yet (the window manager, the input
handler, and the pixel-operations server are later stages). The display
server is the concrete thing: it writes a visible pattern to the frame buffer
and paces itself. No abstraction layer yet.

**Frame submission protocol** (a refinement, not day-one): a client sends a
frame-submission request (the frame number, the pattern parameters) on the
display server's inbound channel. The display server acknowledges on its
outbound channel. This is the 9P-server shape: the client is smart (it
computes the frame), the display server is dumb (it writes the frame to the
frame buffer and paces itself).

### Init and bringup order

The display server is spawned by the kernel's `bringup()` (like the console
server and the nameserver). The bringup order:

1. The nameserver is spawned first (it must be up before the display server's
   BIND is processed).
2. The display server is spawned (it creates its own channel pair, publishes
   its name in the nameserver, and starts its frame loop).
3. The console server is spawned (it publishes its name and serves one
   round-trip).
4. init is spawned (the process manager).

The display server's frame loop starts as soon as it is spawned (no external
trigger). The display server blocks on its pacing channel (the `SYS_RECEIVE_AT`
with a timer deadline on QEMU; the vblank channel on the Pi). The display
server never exits (it runs until the kernel is shut down).

### Failure policy

- **Frame buffer allocation failure** (on QEMU: the `SYS_ALLOC` for the
  software buffer fails; on the Pi: the `SYS_MAP_MMIO` for the GPU frame
  buffer fails): the display server exits (a fatal error — the display server
  cannot display anything without a frame buffer). The integration image
  checks the display server's status (it should be alive, not ended).
- **Pacing failure** (the `SYS_RECEIVE_AT` returns a timeout before the
  deadline — a missed frame): the display server logs a warning and retries
  (the frame loop continues). A missed frame is acceptable (the display is
  not torn — the frame buffer is updated atomically, and the next frame is
  paced to the next deadline).
- **No busy-wait**: the display server is blocked (off the ready set) during
  the wait. The process table shows it blocked; a second process runs in the
  interval (the preemption test's shape, reused).

## Not building

- **A kernel-side framebuffer driver**: the kernel never touches the display
  hardware; the display server owns it. (The Amiga shape refuses this.)
- **A window manager**: the window manager is a user-space process that
  receives input events and repaint requests on its channel. It is a later
  stage (after the display server).
- **A pixel-operations server** (the Blitter equivalent): a later stage. The
  display server writes the frame directly to the frame buffer (no
  hardware acceleration yet).
- **Multi-channel `poll`/`select`**: the display server uses one channel +
  one deadline (the pacing source). A multi-channel `poll` is a refinement.
- **Real-time (wall-clock) clock**: the display server uses the monotonic
  clock (SYS_CLOCK). A wall-clock is a refinement.
- **Sub-tick resolution**: the timer's granularity is 100 ms (the existing
  TICK_PERIOD). The display server paces to 60 Hz (16.7 ms) via the
  `SYS_RECEIVE_AT` deadline — the deadline is checked at each tick, so the
  effective resolution is 100 ms. A finer timer is a hardware question, not
  a service question. **Wait** — this is a problem. The tick resolution is
  100 ms, but the display needs 16.7 ms. Let me reconsider.

**Revised pacing resolution**: the `SYS_RECEIVE_AT` deadline is checked at
each tick (100 ms). So the display server's frame loop runs at 10 Hz (100 ms
per frame), not 60 Hz (16.7 ms per frame). This is a problem: the display
server cannot pace to 60 Hz with a 100 ms tick.

**Resolution**: the display server's pacing is limited by the tick
resolution. On QEMU, the tick is 100 ms, so the display server runs at 10
Hz. On the Pi, the vblank interrupt is 16.7 ms, so the display server runs
at 60 Hz (the vblank interrupt is the pacing source, not the timer).

The 60 Hz goal is achievable on the Pi (the vblank interrupt is the pacing
source). On QEMU, the display server runs at 10 Hz (the tick resolution).
This is acceptable for the first display server (it proves the frame loop
works, the frame buffer is written, and the pacing blocks without spinning).
The 60 Hz goal is fully realized on the Pi (the vblank interrupt).

A finer timer (sub-tick resolution) is a kernel change (a shorter
TICK_PERIOD or a separate high-resolution timer). It is deferred: the
display server works with the current tick resolution, and a finer timer is
a hardware question, not a service question.

## Decision records

### Decision 1: Software framebuffer on QEMU, GPU framebuffer on the Pi

- **Decision**: the frame sink is pluggable. On QEMU: a software buffer in
  the process's heap (allocated via `SYS_ALLOC`). On the Pi: the GPU's frame
  buffer (mapped via `SYS_MAP_MMIO`, the size from the device tree).
- **Alternatives**: (a) always use a software buffer (simpler, but the Pi's
  GPU frame buffer is the real display — a software buffer is not visible).
  (b) always use the GPU frame buffer (simpler, but QEMU has no GPU).
- **Dissent**: the whole-system lens argues for the fewest concepts: one frame sink,
  not two. We chose the two-axis pluggable design because the hardware is
  different (QEMU has no GPU; the Pi does). The software buffer is the
  concrete thing on QEMU; the GPU frame buffer is the concrete thing on the
  Pi. The pluggability is a hardware truth, not a design choice.

### Decision 2: Timer pacing on QEMU, vblank-interrupt pacing on the Pi

- **Decision**: the pacing source is pluggable. On QEMU: a timer deadline
  (`SYS_RECEIVE_AT` with a 16.7 ms deadline, checked at each 100 ms tick —
  effective 10 Hz). On the Pi: the vblank interrupt's message (the kernel's
  IRQ→message path delivers it to the display server's channel; the
  `SYS_RECEIVE_AT` deadline is the fallback).
- **Alternatives**: (a) always use a timer (simpler, but the Pi's vblank
  interrupt is the real heartbeat — a timer is a fallback, not the primary).
  (b) always use the vblank interrupt (simpler, but QEMU has no vblank
  interrupt).
- **Dissent**: the Amiga lens argues that the vblank interrupt is the
  heartbeat, not an edge case. We chose the two-axis pluggable design
  because the hardware is different (QEMU has no vblank interrupt; the Pi
  does). The timer is the concrete thing on QEMU; the vblank interrupt is
  the concrete thing on the Pi. The pluggability is a hardware truth, not a
  design choice.

### Decision 3: The display server is a user-space process, not a kernel
subsystem

- **Decision**: the display server is a user-space process (like the console
  server). It owns the frame buffer and paces itself. The kernel never
  touches the frame buffer.
- **Alternatives**: (a) a kernel-side framebuffer driver (the Linux shape —
  the kernel touches the display hardware). (b) a kernel-side display
  subsystem (the kernel paces the frame loop).
- **Dissent**: the microkernel lens argues that the kernel should own
  the real-time duty (the 60 Hz pacing). We chose the user-space design
  because the Amiga shape is explicit: the kernel's job is to deliver the
  vblank interrupt to the display server's channel with bounded latency; the
  display server owns the frame loop. The kernel's real-time duty is the
  interrupt delivery (the three-thing budget), not the frame loop.

### Decision 4: The display server writes a visible pattern (not a blank
frame)

- **Decision**: the display server writes a visible pattern to the frame
  buffer (a moving color bar, driven by the frame number). The pattern
  proves the frame loop is running (the frame number advances, the pattern
  moves).
- **Alternatives**: (a) a blank frame (simpler, but it doesn't prove the
  frame loop is running). (b) a user-supplied frame (the 9P-server shape —
  the client computes the frame, the display server writes it).
- **Dissent**: the simplicity lens argues for the simplest thing: a blank
  frame. We chose the visible pattern because the integration image needs
  proof that the frame loop is running (the frame number advances, the
  pattern moves). A blank frame doesn't provide this proof. The visible
  pattern is the minimal thing that works (the Amiga's demoscene routine,
  simplified).

### Decision 5: The tick resolution limits QEMU pacing to 10 Hz

- **Decision**: the display server's pacing on QEMU is limited by the tick
  resolution (100 ms). The display server runs at 10 Hz on QEMU (not 60 Hz).
  On the Pi, the vblank interrupt is the pacing source (16.7 ms), so the
  display server runs at 60 Hz.
- **Alternatives**: (a) a shorter tick (e.g., 10 ms) to achieve 60 Hz on
  QEMU. (b) a separate high-resolution timer (a hardware question). (c)
  accept 10 Hz on QEMU (the display server works, the 60 Hz goal is fully
  realized on the Pi).
- **Dissent**: the Amiga lens argues that 60 Hz is the goal, not 10 Hz. We
  chose 10 Hz on QEMU because the tick resolution is 100 ms (the existing
  TICK_PERIOD), and a shorter tick or a high-resolution timer is a kernel
  change (a hardware question, not a service question). The display server
  works with the current tick resolution, and the 60 Hz goal is fully
  realized on the Pi (the vblank interrupt). A finer timer is deferred.

## Tasks

1. **`r9-display-server.md`**: the display server — a user-space process that
   owns a software framebuffer (QEMU) and paces itself with `SYS_RECEIVE_AT`
   (timer deadline, 10 Hz effective on QEMU). The frame loop: write a moving
   color bar, block on the pacing channel, repeat. The integration image
   checks the frame buffer contents (the pattern is present, the frame number
   advanced). Gated on Task 75 (SYS_RECEIVE_AT, done).

Sequencing: the display server is gated on Task 75 (done). It can proceed in
parallel with Tasks 76 (proc-control) and 77 (sched). The vblank-interrupt
pacing (the Pi) is a refinement (deferred: the vblank interrupt's IRQ→message
path is the existing stage-4 machinery, and the display server's vblank
channel is a refinement of the timer pacing).
