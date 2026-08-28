# Amiga inspiration — real-time interactive graphics as the design goal

The Amiga (1985, Commodore) is r9's fourth design inspiration alongside
Plan 9, QNX, and Oberon. It is not a review lens — it is a system whose
shape answers the question "what does a machine that boots to a graphical
environment and runs real-time interactive graphics actually need from its
kernel?"

The standing goal this establishes: **r9 boots to a graphical environment
by default, and the kernel's job is to keep the display alive at 60 Hz
while user-space servers do everything else.** The text console is an
early-boot bringup path, not the end state.

## The Amiga's load-bearing ideas

### 1. Interrupt-driven I/O is the heartbeat, not an edge case

The custom chipset (Agnus, Denise and Paula) generates interrupts for
vertical blank, blitter done, copper done, and input events. The CPU programs the hardware, does
other work, and is woken by the hardware when it needs attention. The
vertical blank interrupt is the heartbeat of the entire graphics system:
every frame begins with one, and tearing happens when a CPU update races
the beam between two of them.

**For r9**: the IRQ→message path (stage 4) is the Amiga's interrupt-to-
message-port path generalised. The GIC IRQ handler's job is to look up
the owning `ProcId`, enqueue a pre-allocated message, and wake a blocked
receiver — nothing else. The display server (stage 5+) owns the vertical
blank interrupt; the kernel never touches the framebuffer. The interrupt
context budget (lookup, enqueue, wake) is the Amiga's interrupt handler
budget made explicit.

### 2. Message ports are the IPC primitive, and they are bounded

The Amiga's Exec kernel provides message ports: bounded FIFO queues of
messages. Tasks `SendMsg` and `WaitPort`/`GetMsg` on them. The windowing
system (Intuition) is built entirely on message ports: the input handler
sends event messages to the window's port, the window handler sends
repaint requests, the icon handler sends click events. Every interaction
is a message on a bounded port.

**For r9**: `port::ipc`'s `Channel` is the message port. The bounded
queue (fixed capacity, pre-allocated slots) is the Amiga's bounded port
queue. The `send`-blocks-when-full semantics (no drop mode) is the
Amiga's `SendMsg` blocking when the port is full. The per-IRQ message
pool (stage 4) is the Amiga's pre-allocated interrupt message — the
interrupt handler never allocates.

### 3. Boot to a graphical environment, not a text console

The Amiga boots to Workbench: a graphical desktop with windows, icons, and
a taskbar. The boot process (the kickstart ROM + the bootblock) starts the
windowing system (Intuition) and the desktop (Workbench) before the user
sees anything. There is no "press a key to enter graphics mode" — the
graphical environment is the default, and the text console is a fallback
for bringup and debugging.

**For r9**: the boot sequence starts a display server (a user-space
process that owns the GPU MMIO), a window manager, and a root window. The
early text console is the bringup path (stages 1–4); the graphical
environment is the end state (stage 5+). The console server (stage 5) is
the first step: it owns the UART MMIO and provides the text console as a
9P server. The display server (a later stage) owns the GPU MMIO and
provides the framebuffer as a 9P server.

### 4. Hardware acceleration is the norm, not an exception

The chipset provides three hardware accelerators (the blitter and copper
in Agnus, the sprite hardware in Denise):
- **Blitter**: a pixel-operations engine. The CPU programs it (source,
  destination, operation), starts it, and does other work while it runs.
  A blitter-done interrupt fires when it finishes.
- **Copper**: a display-list processor. Every vertical blank, the Copper
  executes a list of display-setup instructions (set palette, set
  display mode, set sprite positions). The CPU updates the list between
  frames; the Copper executes it during the blank.
- **Sprites**: hardware sprites that the GPU moves, not the CPU.

**For r9**: the GPU (VideoCore VI on the Pi, VirtIO GPU on QEMU) is a
user-space server. The display server programs the GPU's registers (the
Copper equivalent) and is woken by the GPU's interrupt (the vertical
blank equivalent). The kernel never touches the GPU's MMIO — the display
server owns it via a `map_mmio` verb (stage 5). The Blitter equivalent
is a pixel-operations server (a later stage).

### 5. The demoscene constraint: everything synchronises to the vertical blank

The Amiga demoscene's central constraint was the vertical blank: all
graphics updates had to be synchronized to the blank interval (the ~1 ms
between frames) to avoid tearing. The demoscene's "routines" were
interrupt handlers that ran in the blank and updated the display. The
CPU's job was to prepare the next frame; the hardware's job was to
display it.

**For r9**: the display server's main loop is the demoscene's routine:
prepare the next frame, wait for the vertical blank interrupt, update the
display, repeat. The kernel's job is to deliver the vertical blank
interrupt to the display server's channel with bounded latency. The
interrupt context budget (lookup, enqueue, wake) is what makes the
bounded latency possible.

## What the Amiga shape refuses to build

- A kernel-side framebuffer driver. The kernel never touches the display
  hardware; the display server owns it.
- A kernel-side window manager. The window manager is a user-space
  process that receives input events and repaint requests on its channel.
- A kernel-side input handler. The input handler is a user-space process
  that owns the input device MMIO and sends event messages to the
  window manager's channel.
- An unbounded interrupt handler. The interrupt context budget is three
  things: lookup, enqueue, wake. Anything more is a bug.
- A text-mode default. The graphical environment is the default; the text
  console is a bringup fallback.

## The design questions this adds

These are the Amiga-specific questions to ask in Phase 1 (interrogation):

- **Vertical blank**: which interrupt is the heartbeat of the graphics
  system on this target, and what is its period? (Pi 4: the VideoCore VI
  HBLANK/VBLANK interrupt, ~16.7 ms at 60 Hz. QEMU: the VirtIO GPU's
  config change interrupt.)
- **Interrupt context budget**: what does the IRQ handler do, and is it
  within the three-thing budget (lookup, enqueue, wake)? What would it
  cost to do anything more?
- **Per-IRQ message pool**: how many messages are pre-allocated per IRQ,
  and what happens when the pool is exhausted? (The Amiga's answer: the
  interrupt is lost — acceptable for a display refresh, not for input.)
- **Display server ownership**: which user-space process owns the GPU
  MMIO, and how does the kernel hand it over? (The `map_mmio` verb,
  stage 5.)
- **Boot to graphics**: what is the boot sequence, and where does the
  text console hand off to the graphical environment? (The console
  server, stage 5, is the first step; the display server is a later
  stage.)

## Attribution

The Amiga's design is a system, not a signature: the custom chipset, the
Exec kernel, the Intuition windowing system and the Workbench desktop were
built by teams, and the demoscene that pushed the hardware hardest was a
community. Cite the machine and its manuals, never a person.

The relevant references are:
- *The Amiga Hardware Reference Manual* — the custom chipset (Agnus,
  Denise, Paula): the blitter, the copper, sprites, and the interrupt
  sources.
- *The Amiga ROM Kernel Reference Manual* — the Exec kernel (tasks,
  message ports, interrupt servers), Intuition, and Workbench.
- The demoscene and Amiga-programming literature — the vertical-blank
  constraint and the copper/blitter programming model.
