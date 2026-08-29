---
status: done
---

# stage6-namespace-test: the namespace image — resolve a name, round-trip a byte

Stage 6a, task 4 of 4. Plan:
[plans/microkernel-nameserver.md](../plans/microkernel-nameserver.md).
Needs tasks 1–3. This is the proof the slice works end to end.

## Goal

A new integration image (`aarch64/tests/namespace.rs`, its own `main9`, a
`[[test]]` entry in `aarch64/Cargo.toml` with `harness = false` and
`required-features = ["qemu-test"]`) that proves the file metaphor: a client
resolves `/dev/console` to a channel pair and round-trips a byte through the
console server — with no test image hardcoding the server's PL011 base or its
channel handles. The image wires the *nameserver's* pair (the forced first-
server asymmetry) and lets everything else be found by name.

## Changes

### 1. Extend the console server with a post-bind receive/reply loop

The current console server (task 68) exits after binding. The round-trip
requires it to stay alive and answer one message. Add, after the BIND reply:

- `SYCRECEIVE` on its own inbound channel (blocks until a client sends).
- `SYCREPLY` on its own outbound channel with the received byte echoed back
  (the reply opcode is `0` / `R_OK`, the payload is the one received byte).
- Exit 0.

This is one-shot: one request, one reply, then the server is done. A persistent
loop is stage 7's concern (9P servers serve many clients). The change is ~15
lines in `servers/console/src/main.rs` (the `SYCRECEIVE` + `SYCREPLY` pair
already exists as the BIND-reply receive; the new code mirrors it with the
server's own handles instead of the nameserver's).

### 2. The new integration image (`aarch64/tests/namespace.rs`)

Boots exactly like `console_server` does (`mailbox::init` before
`boot::console`, interrupts, user page tables). The **client is the kernel
itself** (the image's `main9` calling `port::ipc` directly on the channels) —
not a spawned user process. The 4-slot channel table is exactly full with the
nameserver's pair (2) + the console server's pair (2); a separate client
process that `SYCCREATECHAN`s its own pair would overflow it. The user-space
syscall path is already proven by the servers themselves (the nameserver uses
`SYCRECEIVE`/`SYCREPLY`, the console server uses `SYCSEND`/`SYCCREATECHAN`/
`SYCRECEIVE`); the kernel-side `port::ipc` path exercises the same channel
machinery without the syscall indirection.

Sequencing (each send-to-user-process + receive-from-it pair requires an
intervening `process::run_all()` to let the server run):

1. Create the **nameserver's** pair kernel-side (`ipc::create()` × 2); spawn
   `nameserver.elf` (`Image::Elf`, `handles: Some(ns_handles)`).
2. Spawn the **console server** ELF (`Image::Elf`, `handles: Some(ns_handles)`
   — it reads the nameserver's pair from `HANDLES_VA` to send its BIND).
3. `process::run_all()` — runs until both servers are blocked: the nameserver
   is blocked on its first `SYCRECEIVE` (waiting for a message); the console
   server has mapped the PL011, written `'A'`, created its pair, sent the
   BIND (which woke the nameserver), received the BIND reply, and is now
   blocked on its post-bind `SYCRECEIVE` (waiting for a client).
4. **RESOLVE**: the kernel sends a `RESOLVE("/dev/console")` request to the
   nameserver's inbound channel (`channel(ns_in).send(...)`), calls
   `process::run_all()` (the nameserver wakes, looks up the name, replies
   with the pair on `ns_out`), then reads the reply from the nameserver's
   outbound channel (`channel(ns_out).receive()`). `check!` the opcode is
   `R_OK` (not `R_ENOENT`) — this is the assertion that the bind happened
   *by name*, not by a hardcoded handle. Extract the console server's
   `(in, out)` pair from the reply payload.
5. **Round-trip**: the kernel sends a byte (e.g. `b'x'`) on the console
   server's inbound channel (`channel(con_in).send(...)`), calls
   `process::run_all()` (the console server wakes, echoes the byte back on
   `con_out`), then reads the reply from the console server's outbound
   channel (`channel(con_out).receive()`). `check!` the reply byte equals
   `b'x'`.
6. `check!` the console server's exit status is 0 (it exited after the
   one-shot reply).
7. `qemu::exit(qemu::PASS)`.

### 3. Embedding

- `aarch64/build.rs` already stages both `console.elf` and `nameserver.elf`
  into `OUT_DIR` with `rerun-if-changed` + loud-failure (task 67 added the
  nameserver; task 68 added the console). No new `build.rs` surface.
- The image `include_bytes!`s both ELFs (same pattern as `console_server.rs`).

### 4. xtask

- The image builds both servers via the existing `ServerStep` (which already
  builds `servers/console` and `servers/nameserver` for aarch64). No new
  xtask surface.

## Tests

- The image is the test: `cargo xtask qemu --arch aarch64 --image namespace`
  must print the resolve + round-trip `ok` lines and exit `PASS`.
- A negative probe (dev-time, not shipped): a `RESOLVE` of a name that was
  never bound returns `R_ENOENT` cleanly (no hang) — confirms the failure
  policy. Can be checked by resolving a bogus name *before* the console
  server has bound (i.e. between steps 2 and 3, or by resolving a name the
  console server never published).

## Acceptance

- `cargo xtask ci` green (18 → 19 integration images; the new one passes).
- The client (kernel) finds the console server **by name** — no console-server
  channel handle is hardcoded in the image. The handles come from the
  `RESOLVE` reply.
- The round-trip byte arrives: the console server echoes it back; the
  `check!`s assert both the `R_OK` resolve and the echoed byte.
- The console server's exit status is 0 (it completed the one-shot reply and
  exited cleanly).

## Not in scope

The boot-time `init` that would spawn this at real boot (task 70). 9P as the
protocol (stage 7). A multi-client or concurrent-resolve test. A persistent
console-server receive loop (one-shot is sufficient for the proof). RX.
