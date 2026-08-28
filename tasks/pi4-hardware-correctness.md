---
id: 127
status: open
wave: 6
---

# Task 127: correctness defects that only bite off QEMU

## Status: open — wave 6.  Batch behind one Pi 4 bring-up session

## Problem

Six defects invisible under `raspi4b` and fatal on the metal.  Batched
because they share a validation session, not because they share a cause.

**`COUNTER_FREQ` is hardcoded (`std/src/time.rs:13`).**  User space
assumes QEMU's 1 GHz while the kernel reads `CNTFRQ_EL0` and panics if it
is zero (`timer.rs:271`), and `timer::init` deliberately enables EL0
reads of it.  On the BCM2711 it is 54 MHz, so every user-space duration
is ~18× wrong and `FRAME_PERIOD = 16_666_667` paces the display at about
3 fps instead of 60.  Read the register.

**Bus address not masked (`cmd/mailbox/src/main.rs:127-131`).**  The
`FB_ALLOCATE` response is a VideoCore *bus* address; the ARM physical
address needs the alias bits masked off.  Unmasked it happens to work
under QEMU.

**Framebuffer size and pitch ignored (`cmd/display/src/main.rs:176`).**
The mailbox reply carries the allocated size at `reply[9..17]` and
nothing decodes it; `map_mmio` and `flip` both use the hardcoded
`FB_SIZE = 640*480*4`.  If the firmware clamps the mode or pads the
pitch, the server maps and writes 1.2 MB past the real framebuffer into
adjacent VC RAM.  (Introduced in shape by f76d96a, which moved the
constant into the server without making it agree with the firmware.)

**EOI without deassertion (`aarch64/src/trap.rs:261`).**  `gic.rs:394`
states the contract: "The caller must handle the interrupt (deasserting
its source ...) and then pass the value to `eoi`.  EOI before deassertion
would immediately re-raise the interrupt."  The timer path honours it by
re-arming CVAL first; the user-routed SPI path only enqueues a message,
then EOIs.  For a level-triggered SPI — exactly what `SYSIRQCLAIM` is for
— the line is still asserted, so the IRQ re-fires the instant IRQs are
unmasked, before the server can touch the device.  Interrupt storm, the
channel queue fills, `try_send` starts returning `Err(Full)` and dropping
messages.  Nothing masks the INTID at delivery or unmasks it on the
server's ack.  `aarch64/tests/irq.rs` never sees it because it calls
`ipc::route` + `try_send` directly from `main9` rather than taking a real
interrupt — so the test to add is one that takes a real one.

**`sys_irq_claim` publish order (`aarch64/src/ipc.rs:377-386`).**  The
`NIRQUEUED.fetch_add` makes the slot visible to `route()` *before*
`*IRQ_ROUTES[slot].0.get() = Some(...)` runs — the inverse of what its
own SAFETY comment claims ("the write-then-publish pattern ... makes the
read see a fully-written route").  A concurrent reader races the write of
an `Option<IrqRoute>`.  The claimed-check at `:366-375` is also not
atomic with the insert, so two cores can claim the same INTID.  Task 120
folds INTID claiming into the device capability, which is where this
should land.

**4 GiB allocator cap (`aarch64/src/pagealloc.rs:31`).**
`BitmapPageAlloc<32, PAGE_SIZE_4K>` covers exactly 4 GiB, so an 8 GB
Pi 4's `mark_free` for a range ending at 8 GiB hits `mark_range`'s
`range.end > self.end` check and `init_page_allocator` fails rather than
clamping to the covered window.

**Cleanup while here:** f76d96a left `FB_PHYS` and
`configure_framebuffer` in `aarch64/src/mailbox.rs:330,348` as dead
kernel code, now that the user-space mailbox server owns framebuffer
configuration.  Delete them.

## Tests

- Integration under QEMU where possible: the pitch/size handling and the
  4 GiB clamp can both be exercised by faking the reply and the memory
  size.
- An IRQ image that takes a *real* level-triggered SPI and asserts the
  server sees exactly one message per device assertion.
- The rest needs hardware; record what was verified on the metal in the
  resolution.

## Done when

- The six are fixed, the dead code is gone, and the display runs at 60 Hz
  on a Pi 4.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
