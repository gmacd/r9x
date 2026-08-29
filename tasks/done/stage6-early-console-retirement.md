---
status: done
---

# stage6-early-console-retirement: demote the kernel's PL011 to debug-only

**Follow-on to stage 6a.** Needs `stage6-init-bringup` (the console server
must be up at boot before the kernel can stand down). Records task 65's design
intent as work. Plan: [plans/microkernel-nameserver.md](../plans/microkernel-nameserver.md).

## Goal

Once the console server is up, the kernel stops using the PL011 for normal
output. The `println!` path (the locked `Console` → `Pl011Uart::putb`) is
gated off. The `iprint` path (direct polled write, no lock, used by the panic
handler and fault paths) is **unaffected** — it is the debug backstop, always
available, no flag.

The kernel keeps its PL011 mapping permanently. It cannot unmap the device:
the early boot path and the fault handler need it before the server exists.
"Ownership" is a policy statement, not a hardware exclusion: both the kernel
and the console server have mappings of the same physical PL011, but the
policy says the kernel writes only for debug/fault output.

## The two paths, stated plainly

| Path | Mechanism | After the gate |
|------|-----------|----------------|
| `println!` | `Console` lock → `Pl011Uart::putb` | **Dropped** (no-op). The byte is silently discarded. |
| `iprint` | `IprintOps::putb` → `Pl011Uart::putb` (direct, no lock) | **Unchanged.** Always writes to the raw PL011. |

The `iprint` path is what the panic handler, the fault handler, and any
deliberate debug print use. It is the kernel's permanent debug channel to the
UART. No flag, no gate, no condition — if the PL011 is initialised (the
`static UART` is set), `iprint` writes.

## The gate

A single `static CONSOLE_LIVE: AtomicBool` (or a plain `bool` protected by
the existing `Console` lock — it is set once during bringup and never read in
interrupt context, so the lock is sufficient and simpler).

- **Set**: the kernel sets it during bringup, after the console server has
  been spawned and had at least one scheduling turn (i.e., after `run_all()`
  would first return, or simply after the spawn sequence completes and before
  the first `run_all`). The console server's BIND is processed during the
  first `run_all`, so by the time any post-bringup `println!` would fire,
  the server is up.
- **Read**: the `println!` path checks it before writing. If set, the byte is
  dropped.
- **One-way**: once set, never cleared. The console server does not die in
  this design (no death hook). If it did (stage 7's concern), the gate would
  need to reopen — that is a stage-7 decision.

## The ownership story (stated, so it is not revisited)

- **Before the server is up**: the kernel owns the PL011. All output goes to
  the raw UART. The console server does not exist yet.
- **After the server is up**: the console server owns the PL011 for normal
  output (it writes user-facing text). The kernel's raw access is the debug
  backstop: faults, panics, and deliberate debug prints. The two writers are
  serialized by the PL011's TX FIFO hardware; the worst case is a debug print
  interleaving with server output, which is acceptable for a debug channel.
- **The kernel's mapping is permanent.** It is not unmapped. The early boot
  path and the fault handler need it. This is the substrate plan's "one
  permanent kernel-resident device" — a stated exception, not an oversight.

## Future: a proper logging system

The current state after this task is: normal kernel output is silently
dropped, and debug output goes to the raw UART via `iprint`. This is correct
for the current system (the kernel has little to say after bringup), but it
will not scale. A proper logging system would:

- Route kernel log messages to the console server via IPC (a "write to
  console" opcode on the server's channel), so they appear in the same stream
  as user output, with a source tag.
- Provide a severity level (debug / info / warn / error) so that debug
  messages can be gated without losing errors.
- Buffer messages that arrive before the server is up (the early boot path)
  and flush them when the server is ready.

This is a **stage 7** concern (it needs the IPC-to-console path and the
server's "write" opcode, both of which are 9P-stage work). Recorded here so
the gate-off is not mistaken for the end state.

## Implementation

The change is small:

1. Add `static CONSOLE_LIVE: AtomicBool = AtomicBool::new(false);` to
   `aarch64/src/devcons.rs` (or the `port::devcons` module, if the gate is
   arch-agnostic — it is: the concept of "the console server is up" is not
   arch-specific).
2. In the `println!` write path (the `Console`'s `putb` or the `port::println`
   sink), check the flag. If set, return without writing.
3. The `iprint` path is untouched.
4. In `main9` (task 70's new bringup), set the flag after the spawn sequence
   and before `run_all()`.
5. A test: the `console_server` or `namespace` image prints a message after
   the gate is set and verifies it does not appear on the raw UART (while an
   `iprint` message does). Or: a new `logging` image that exercises both paths.

## Acceptance

- After the console server is up, `println!` output does not reach the raw
  PL011 (verified: the expected text is absent from the serial output).
- `iprint` output still reaches the raw PL011 (verified: a deliberate
  `iprint` call after the gate produces output on the serial).
- A panic or fault after the gate still prints to the raw PL011 (the `iprint`
  path in the panic/fault handler).
- `cargo xtask ci` green; the `console_server`/`namespace` images still pass.
- The early boot path (before the gate) still prints to the raw PL011
  (verified: the boot messages appear before the gate is set).

## Not in scope

- Routing kernel output to the console server via IPC (stage 7's logging
  system).
- Removing the kernel's PL011 mapping (it is permanent by design).
- RX (input). The kernel never reads from the PL011.
- Severity levels, buffering, or any logging infrastructure (stage 7).
- The gate reopening (no death hook; stage 7's concern).
