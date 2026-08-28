---
status: done
---

# r9-display-server: the 60 Hz heartbeat, a user-space process (Tier 2.2)

Task 5 of 7 in the r9x arc (after the clock/wait). Plan:
[plans/r9-display-server.md](plans/r9-display-server.md).
Gated on Task 75 (SYS_CLOCK + SYS_RECEIVE_AT, done 2026-08-25). The display
server is the first user-space process that owns a framebuffer and paces its
frame loop — the Amiga's demoscene routine in user-space.

## Problem

The standing goal: r9 boots to a graphical environment by default, and the
kernel's job is to keep the display alive at 60 Hz while user-space servers do
everything else. Today no process owns a framebuffer and no process paces a
frame loop. Task 75 provided the pacing primitive (SYS_RECEIVE_AT: block until
a message or a deadline, no spin). This task builds the display server: a
user-space process that owns a software framebuffer (QEMU) and paces itself
with a timer deadline.

## Changes

- **`cmd/display/`**: a new user-space server (like `cmd/console`). Its body:
  1. Allocate a software framebuffer (640×480×4 = 1,228,800 bytes RGBA) via
     `r9x_std::mem::alloc`.
  2. Create a pacing channel (`r9x_std::ipc::create_chan`).
  3. Loop: write a moving color bar to the frame buffer (driven by the frame
     number), block on the pacing channel with a timer deadline
     (`r9x_std::ipc::receive_at`), advance the frame number.
  4. The color bar: a vertical bar that moves left-to-right across the frame
     (position = frame_number % width). The rest of the frame is black.
  5. Publish the name `/dev/display` in the nameserver (like the console
     server publishes `/dev/console`).
- **`r9x_std::time`**: add a `FRAME_PERIOD` constant (16,666,667 ticks ≈
  16.7 ms at 1 GHz; the 60 Hz target). The display server uses it as the
  deadline increment.
- **Kernel bringup**: add the display server to `system::bringup()` (spawn it
  after the nameserver, before the console server — the nameserver must be up
  for the BIND). The display server is handed the nameserver's handles (like
  the console server).
- **xtask**: add the display server to the server build list (like console,
  nameserver, init). The display server's ELF is staged into OUT_DIR by
  build.rs.

## Tests

- **Integration image `display`** (aarch64): spawn the display server (via
  `bringup` or directly), let it run for a few frames, then check:
  1. The display server is alive (not ended — a frame buffer allocation
     failure or a fault would end it).
  2. The frame buffer contents: the color bar is present (a non-zero pixel
     at the expected position, zero elsewhere). The integration image reads
     the frame buffer via the display server's channel (a frame-submission
     request: "give me the current frame").
  3. The frame number advanced: the display server's frame number is > 0
     (the frame loop ran).

  The integration image checks the frame buffer by messaging the display
  server (a `GET_FRAME` request: the display server replies with the frame
  buffer contents). This is the 9P-server shape: the client (the integration
  image) asks, the server (the display server) answers.

- **No busy-wait**: the display server is blocked (off the ready set) during
  the pacing wait. The process table shows it blocked. A second process runs
  in the interval (the preemption test's shape, reused: spawn a busy process
  alongside the display server, check the busy process runs while the display
  server is blocked).

## Acceptance

- `cargo xtask ci` green (all arches; the `display` image passes).
- A process can pace a frame loop without spinning the CPU (it blocks on
  `SYS_RECEIVE_AT`, not a spin).
- The frame buffer is written (the color bar is present, the frame number
  advanced).
- The display server is a user-space process (the kernel never touches the
  frame buffer).

## Not in scope

The vblank-interrupt pacing (the Pi): the display server uses timer pacing
on QEMU (10 Hz effective, the tick resolution limit). The vblank-interrupt
pacing (60 Hz on the Pi) is a refinement: the vblank channel is a refinement
of the timer pacing, and the IRQ→message path is the existing stage-4
machinery. The GPU framebuffer (the Pi): the display server uses a software
buffer on QEMU. The GPU framebuffer (mapped via `SYS_MAP_MMIO`) is a
refinement. A window manager, an input handler, a pixel-operations server
(the Blitter): later stages. A shorter tick (sub-100 ms): a kernel change,
deferred. Multi-channel `poll`/`select`: a refinement.

## Origin

The Amiga goal: boot to a graphical environment, the kernel keeps the display
at 60 Hz, user-space servers do everything else. Task 75 (SYS_RECEIVE_AT)
unblocked the pacing primitive. This task builds the display server: the
first user-space process that owns a framebuffer and paces its frame loop.
