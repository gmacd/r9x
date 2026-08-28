---
status: done
---

# kernel-console-pl011

**From:** the console-server arc follow-up (the PL011-vs-mini-UART discussion).
**Depends on:** nothing new — the `Pl011Uart` driver already exists and is
exercised by the `pl011` integration image; this repoints the early console at
it and re-wires the QEMU serial sink to match.

## Context

r9's kernel and its console server currently use **two different UARTs**:

| | device | DT compatible | kernel phys | role |
|---|---|---|---|---|
| kernel early console | **MiniUart** | `brcm,bcm2835-aux-uart` (UART1) | `0xfe21_5040` | `boot::console` → `devcons::init` → `MiniUart` |
| console server | **PL011** | `arm,pl011` (UART0) | `0xfe20_1000` | the stage-5 server, and (per this arc) the real console going forward |

On the Pi 4/5 the **PL011 is the "main" / primary UART** — a full 16550 UART
with a real baud generator and a 16-byte FIFO — while the mini UART is the
auxiliary one (no hardware baud generator, a 1-byte FIFO, no modem control).
The board's default *console* alias happens to be the mini UART, but that is a
firmware default, not a statement about which is the better console device.

So today the kernel prints boot messages on the *weaker* UART while the
console server — the process that will own the console — works on the *main*
UART. This task makes them consistent: **the kernel early console moves to the
PL011**, the same device the console server owns.

## Design intent: the kernel UART is the early-boot + debug console

The kernel UART is not the console once the console server is up. The division
of roles is:

- **Early boot** (before any user process): the in-kernel console on the PL011
  is the only console; boot messages go there.
- **Once the console server is running:** the console server *is* the console.
  The in-kernel PL011 console is demoted to a **debug-only** channel — used,
  if at all, only for kernel-internal diagnostics that cannot be routed
  through the server (a fault before the server is reachable, a fatal trap,
  a deliberate debug print). It is not the path normal kernel output takes.

This task only changes *which device* the in-kernel early console drives; it
does not implement the demotion (gating normal kernel output off the UART once
the server is up). That demotion is the same open question stage 6 carries
(whether the in-kernel console is retired at all once the console server is
the real console). Recording the intent here is so the change is understood as
"put the early console on the right device," not "make the in-kernel console
the permanent console."

This overrides a stage-5 design decision. `plans/microkernel-console-server.md`
says, under *Not building*: *"Retiring the early console. The MiniUart stays.
It is a different UART from the PL011. Retiring it requires the namespace
(stage 6)."* That rationale was about *retiring* the early console (a
user-space server replacing it as the console), not about *which* UART the
early console sits on. Moving the early console to the PL011 now is
independent of the namespace: it just changes the device the in-kernel early
console drives.

## Changes

### `aarch64/src/devcons.rs` — repoint the console at the PL011

`init` currently builds the console from a `MiniUart`:

```rust
pub fn init(dt: &DeviceTree) {
    Console::set_uart(|| {
        let uart = MiniUart::new_with_map_ranges(dt);   // ← MiniUart (UART1)
        ...
        uart.init();
        ...
    });
}
```

Repoint it at the existing `Pl011Uart` (already in `aarch64::uartpl011`,
already used by the `pl011` image):

```rust
pub fn init(dt: &DeviceTree) {
    Console::set_uart(|| {
        let uart = Pl011Uart::new(dt);                  // ← PL011 (UART0)
        ...
        uart.init();
        ...
    });
}
```

`Pl011Uart::new` already maps the GPIO and the PL011 (`arm,pl011`) via
`deviceutil::map_device_register`, and `Pl011Uart::init` already muxes GPIO 14/15
to the PL011's ALT function, disables then enables the UART, and programs LCRH
and the baud dividers — the same bring-up the `pl011` image exercises. The
`Uart` trait is implemented for both, so `Console::set_uart` and the
`iprint`/console lock are unaffected. The `MiniUart` and its `uartmini`
module remain (still exercised by the `miniuart` image); only `devcons` stops
using it.

### `xtask/src/main.rs` — land the PL011 on the visible serial sink

The kernel now prints on the PL011, so QEMU must route the PL011 to the
terminal, not the mini UART. On QEMU `raspi4b` the machine wires
**`serial_hd(0)` = UART0/PL011** and **`serial_hd(1)` = UART1/mini-UART**.
xtask currently feeds them `-serial null` then `-serial mon:stdio`, i.e. the
PL011 → `null` and the mini-UART → the terminal. That is exactly the inverse
of what the kernel now uses, so the early console's output would go to the
null sink and the (now-unused) mini-UART would own the terminal.

Swap the two aarch64 `-serial` args so the PL011 lands on `mon:stdio`. There
are **two sites**, both aarch64-only:

1. The run path (the `qemu` step, ~line 640), which currently reads:
   ```rust
   // If using UART0 (PL011), this enables serial
   cmd.arg("-serial");
   cmd.arg("null");
   cmd.arg("-serial");
   cmd.arg("mon:stdio");
   ```
   becomes `-serial mon:stdio` then `-serial null` (and the two "If using
   UART…" comments swap to match).

2. The integration-test path (the `ArchIntegrationTests` aarch64 arm, ~line
   1593), which currently reads:
   ```rust
   cmd.arg("-serial").arg("null");
   cmd.arg("-serial").arg("mon:stdio");
   ```
   becomes `-serial mon:stdio` then `-serial null`.

The riscv64 and x86-64 arms each use a single `-serial mon:stdio` and are
untouched.

## Verification

- `cargo xtask ci` green (warning-free ×3, host tests, all 18 integration
  images). The integration images run through the re-wired serial path, so a
  wrong sink would show up as a hang/timeout rather than a clean exit.
- Boot messages are visible on the terminal: `cargo xtask qemu --arch aarch64`
  (or the `console_server` image) now prints the kernel's early-console output
  on the PL011 → `mon:stdio`. This is the observable that the repoint worked —
  before the swap the messages are on the mini-UART; after, they are on the
  PL011.
- The `console_server` image's loopback cross-check still holds: the server
  maps the same `0xfe20_1000` PL011 the kernel now brings up, so the two agree
  on the device.

## Out of scope

- **Demoting the in-kernel console to debug-only once the server is up**
  (gating normal kernel output off the UART). That is stage 6's
  retire-the-early-console question; this task only puts the early console on
  the right device and records the intent.
- **RX / console input** (the UART's RX IRQ waking the server). A separate
  refinement.
- **Real-Pi EEPROM/config** for assigning the PL011 to the serial GPIO pins
  (noted on `Pl011Uart` as "a bit fiddly on a real board"). QEMU `raspi4b`
  needs none of it; a real-Pi target is a later concern.

## Done

Done. The devcons repoint and the xtask `-serial` swap are as described above,
plus two changes the task file did not anticipate, both forced by the repoint:

1. **`Pl011Uart::init` CR bug fixed.** The old enable value `0x81` was
   `UARTEN|LBE` — loopback — mislabelled "receive only" in the comment. A
   loopback console never transmits. Now `CR_UARTEN | CR_TXE | CR_RXE` with the
   three bits named (cited to the PL011 TRM §3.3.2) and the old value explained
   in the comment. The `pl011` image's `TXE`-compensation line and its
   receive-only `0x81` check update to match (`CR_ENABLE` = the three bits).

2. **Mailbox now inits before the console, in all 17 aarch64 boot images**
   (`main9` + the 16 integration images). `Pl011Uart::init` sets the PL011
   clock through the VideoCore mailbox, so the mailbox must be up before
   `boot::console` builds the PL011 console; it previously ran after. The
   mailbox is a standalone device, so it is brought up on its own before the
   console, with a one-line why-comment at `main.rs` and `pl011.rs`.

3. **The `console_server` image's LBE loopback check is removed.** That check
   existed because the kernel console was a *different* UART (the MiniUart), so
   the PL011's RX path held only the server's byte. With the kernel now on the
   same PL011, every kernel console byte written while loopback is armed lands
   in the RX FIFO ahead of the server's `'A'`, and QEMU's PL011 never clears
   RXFIFO-empty, so neither a single DR read nor a drain loop can isolate the
   server's byte. The server's `'A'` now goes out the PL011 to the terminal (it
   is visible on the serial output), and the in-image check is the server's
   exit-0 (it ran, mapped the device, wrote the byte, and completed). The
   device-tree cross-check of the PL011 base is kept.

The "still holds" verification line above (the loopback cross-check) is
superseded by item 3: the loopback is gone; the cross-check that remains is the
DT-base one in `enable_pl011`.
