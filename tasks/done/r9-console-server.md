---
status: done
---

# Task: Console server as a persistent user-space driver

## Status: done — server half (9b3920a) + client API (task 88 build)

Server half landed 2026-08-27 (9b3920a): OP_WRITE opcode dispatch,
per-client reply channel, TXFF pacing in `cmd/console/src/main.rs`.
Client half landed in the task-88 build: `std/src/console.rs` (`write`,
`println`, `reply_channel`, resolve cache, chunking at `MSG_MAX - 4`,
`ConsoleError`), the `display` image's "display passed" verdict now goes
through `console::write` via a new `cmd/consclient` test program (the image's
`main9` runs at EL1 and cannot issue `svc`, so a user process carries the
verdict), and the new `two_clients` integration image verifies per-client
reply-channel serialisation with two concurrent clients. Both images use
phased bringup (servers to fixpoint, then clients) so a client's `RESOLVE`
always finds the bound name — no scheduler-ordering dependence.

An adjacent pre-existing issue surfaced and was filed as task 101
(`display-ns-handle-form.md`): the display server reads its nameserver
handles from the extra fields while the image spawns it with them in the main
fields (works only because channel 0 == `ns_in`).

## Problem

The console server (`cmd/console`) is persistent, speaks OP_WRITE with
a per-client reply channel and TXFF pacing, and the mailbox server
resolves `/dev/console` and routes its output through it. But there is
still no `r9x_std::console` client API. User programs other than the
mailbox server have no ergonomic way to write to the console. The kernel uses the
PL011 directly for boot messages and fault handlers, but after boot,
user-space servers should own the hardware.

## Design

### Ownership

The console server owns the PL011 (user-space driver, Plan 9 shape). It maps
the PL011 via `SYS_MAP_MMIO` and writes to it directly. The kernel keeps the
PL011 for fault handlers (panic messages can't go through IPC) but stops
using it for normal output after boot.

### Protocol (amended 2026-08-27)

Opcodes ride the existing out-of-band `send(handle, opcode, tag, buf)`
header (`std/src/ipc.rs:30`), not in-band bytes. The request payload
carries a **per-client reply channel**, mirroring the nameserver
convention (task 86, commits 540d1a2/e362c13):

```
Client → Console:  op=OP_WRITE, tag, payload = [reply:4 LE][data:...]
Console → Client:  op=R_OK, same tag, empty payload — on the CLIENT's
                   reply channel, not the shared out channel
```

- `OP_WRITE = 0`, `R_OK = 0` (nameserver's result-code space)
- **Why the reply channel is mandatory:** RESOLVE hands every client the
  same `(in, out)` pair and `receive` does not filter by tag, so two
  concurrent clients receiving on the shared `out` steal each other's
  replies — the exact race task 86 fixed for the nameserver (see the
  comment at `cmd/nameserver/src/main.rs:62-66`). The original design
  here replied on `out_h` and would have re-introduced it; this task's
  own two-concurrent-clients test would have exposed it.
- **Chunking:** `MSG_MAX = 256` (`abi/src/lib.rs:41`). The client chunks
  at `MSG_MAX - 4` (the reply-channel field); long writes are multiple
  messages. Each message is the atomicity unit — two clients' output can
  interleave between messages but not within one; clients that care keep
  a line within one message (document this on `console::write`).
  (Plan 9's `putstrn0` serializes whole writes under a lock; per-message
  atomicity is the channel-shaped equivalent.)
- The console server writes bytes to the PL011 DR, spinning while
  **TXFF (FIFO full)** is set before each write — the kernel's
  `Pl011Uart::putb` (`aarch64/src/uartpl011.rs:126`) is the reference.
  (The original text said "wait for TXFE between each byte", which
  drains the FIFO per byte and throws away the FIFO's purpose.)

### Server changes (`cmd/console/src/main.rs`)

**All done (9b3920a):** the persistent loop, `SYS_MAP_MMIO` for the
PL011, `BIND` of `/dev/console`, OP_WRITE dispatch with the per-client
reply channel, and TXFF pacing. Kept for the record:

1. **OP_WRITE protocol with per-client reply channel:**
   ```
   loop {
       (op, bytes, tag) = receive(in_h)
       reply_h = u32 LE from buf[..4]
       match op {
           OP_WRITE => { write_pl011(&buf[4..bytes]); reply(reply_h, R_OK, tag, &[]); }
           _ => reply(reply_h, R_EINVAL, tag, &[]),
       }
   }
   ```
2. **TXFF pacing:** spin while TXFF (FIFO full) is set before each byte
   write — the kernel's `Pl011Uart::putb` is the reference. QEMU's model
   always has room, but real hardware (Pi 4) has an 8-byte FIFO.

### Client API (`r9x_std::console`)

A new module in `r9x_std`:

```rust
pub mod console {
    /// Write bytes to the console. Resolves `/dev/console` on first call
    /// (cached), sends the write request, waits for the reply.
    pub fn write(data: &[u8]) -> Result<(), ConsoleError>;

    /// A convenience for `write` that formats with `core::format_args`.
    pub fn println(args: core::fmt::Arguments) -> Result<(), ConsoleError>;
}
```

- The resolve is cached in a `static` (the channel pair doesn't change);
  the per-process **reply channel** is created once and cached alongside.
- `write` chunks at `MSG_MAX - 4` (see Protocol).
- `ConsoleError` distinguishes `Ipc` (send/receive failed) from `Closed`
  (`ERR_CLOSED` — the server died; task 95 delivers this instead of a
  hang).
- Name: the path is `/dev/console` (what the code binds today); the done
  task 89's file said `/dev/cons` — `/dev/console` wins, this line is
  the record.
- This is a general-purpose facility (every program might write to the
  console), so it belongs in `r9x_std`, not the server crate.

### Nameserver interaction

The console server uses the per-client reply channel protocol (task 86):
it creates a reply channel, includes it in the `BIND` request, and receives
the ack on its own channel. No change from the current code (already updated
for task 86).

### Kernel changes

- None. The kernel's `iprintln!` (boot messages, fault handlers) keeps
  the PL011 directly, and the "stop using it for normal output after
  bringup" half already landed as task 71 (34cbe80: `CONSOLE_LIVE` gates
  `println!` in `port/src/devcons.rs`). `SYS_PRINT` (task 89, done)
  remains the debug path that doesn't need this server up.

## Files to change

- `cmd/console/src/main.rs` — done (9b3920a).
- `std/src/lib.rs` — add `pub mod console;`
- `std/src/console.rs` — new: `write`, `println`, resolve cache, chunking
- `aarch64/tests/namespace.rs` — done (9b3920a): uses OP_WRITE with a
  reply channel.
- `aarch64/tests/display.rs` — the display test's init process should
  use `console::write` for its output (instead of the kernel's `iprintln!`)
- New test: two concurrent clients writing to the console (verify
  per-client reply channel serialisation)

## Tests

- The `namespace` integration image — done (9b3920a): OP_WRITE with a
  per-client reply channel, verifies write + R_OK + still-alive.
- The `display` integration image: the init process uses `console::write`
  for its "display passed" message.
- A new test: two concurrent clients writing to the console (verify
  per-client reply channel serialisation — the exact race task 86 fixed
  for the nameserver).

## Dependencies

- Task 86 (per-client reply channels) — done; this task extends the same
  convention to OP_WRITE (see Protocol).
- Task 95 (channels close on exit) — done; a crashed console server now
  delivers `ERR_CLOSED` to clients instead of hanging them.
- Task 92 (fault-checked user reads) — done; the client's `write` path
  is safe against a corrupt reply channel handle.
- Task 87 (mailbox MMIO fix) — done; the PL011 mapping was always fine.
  (The old "L1 112 vs 128" framing was bad arithmetic — the VAs differ
  at L2.)

## Related

- The console server is the first user-space device driver. The pattern
  (map MMIO, bind in nameserver, serve IPC requests) is the template for
  the mailbox server, display server, and future 9P servers.
