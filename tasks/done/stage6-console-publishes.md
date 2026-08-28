---
status: done
---

# stage6-console-publishes: the console server takes a name

Stage 6a, task 3 of 4. Plan:
[plans/microkernel-nameserver.md](plans/microkernel-nameserver.md).
Needs tasks 1 (`SYCCREATECHAN`) and 2 (the nameserver to bind to).

## Goal

Give the stage-5 console server a name. After it maps the PL011 and writes its
byte, it now also creates its own channel pair and publishes it under
`/dev/console` in the nameserver. The server gains a name; the "kernel is
device-dumb" property is unchanged (the server still knows its own PL011 base
and still requests its own mapping — it just also requests its own channels and
tells the nameserver their handles).

## Changes

- **`servers/console/src/main.rs`**: after the existing PL011 map + `'A'` write,
  before `exit`:
  1. `SYCCREATECHAN` twice → the server's `(in, out)` pair (the inbound channel
     clients send to, the outbound channel it replies on).
  2. Send `BIND { name="/dev/console", in, out }` to the **nameserver's**
     inbound handle (handed in by the spawner, task 4 — the same "pass
     constants to an embedded server" convention as the nameserver's own
     handles). Check the `OK`/`Efull` reply; on a non-`OK` the server still
     exits 0 for the arc (binding is the namespace's concern, and the image
     asserts the bind landed).
  3. A short `SYCRECEIVE` on its own `in` to service one round-trip so the
     test image's client can confirm the byte, *then* `exit` — or keep the
     stage-5 "write and exit" shape and let the image's client do the
     round-trip against the PL011 as today. **Decide in task 4**: the minimal
     change is to add the bind and keep the exit; the round-trip is the
     image's job. Prefer the minimal change.

## Tests

- Covered end-to-end by task 4's `namespace` image (the client resolves
  `/dev/console` and round-trips). This task adds the bind to the server's
  `start()`; the observable is that the nameserver now holds a
  `/dev/console` entry after the server runs.

## Acceptance

- `cargo xtask ci` green (the server rebuilds; the image that embeds it
  rebuilds too — the stage-5 `build.rs` `rerun-if-changed`).
- The console server still maps its own PL011 and writes `'A'` (stage-5
  behavior preserved); it additionally `SYCCREATECHAN`s and `BIND`s.
- No kernel change.

## Not in scope

The nameserver (task 2). The test image (task 4). RX (the console server's RX
IRQ waking it) — a separate refinement. 9P as the server protocol — stage 7.
