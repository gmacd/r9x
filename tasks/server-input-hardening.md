---
id: 114
status: open
wave: 3
---

# Task 114: any client can kill or wedge any server with a short message

## Status: open — wave 3 (prerequisite for task 98)

## Problem

Every server in `cmd/` indexes its request payload before checking its
length, and none handles a closed channel.  The panic handler is
`exit(0)` (`std/src/rt.rs:97`), so a server that dies this way looks like
a clean shutdown in `any_exited()` — which is the test suite's only
oracle.

- **Nameserver (`cmd/nameserver/src/main.rs:67`, `:74`).**
  `payload[payload.len() - 4..]` runs before any validation, so a 0–3
  byte message underflows and panics.  `let name_len = payload.len() - 12`
  in the `OP_BIND` arm underflows below 12.  Any process can permanently
  remove the name service with `ipc::send(ns_in, 0, 0, &[0u8; 1])`.  The
  same line fires on a `RECEIVE_CLOSED` return (`bytes == 0`), which is
  reachable via task 113.
- **Console (`cmd/console/src/main.rs:130`).**  `if bytes < 4 { continue; }`
  treats a closed-channel return as a short message, but `receive` on a
  closed channel returns *immediately and permanently*.  The server
  busy-spins at its own priority forever, starving everything below it,
  and never serves another write.
- **Mailbox (`cmd/mailbox/src/main.rs:277`).**  `op` is read from
  `req_buf[0..2]` before `n` is validated.  `req_buf` is zeroed each
  iteration, so an empty message reads as `OP_CONFIGURE_FB`, fails
  `n < 14`, and hits `exit(1)` — killing the mailbox server, and with it
  the display pipeline, for every client.
- **Bind table (`cmd/nameserver/src/bind_table.rs:73`).**  `bind`
  truncates a name to `NAME_MAX` and returns `R_OK`, while `matches()`
  compares the full length — so an over-long name binds unresolvably, and
  two names sharing 32 bytes collide silently.
- **Display (`cmd/display/src/main.rs:162`).**  Receives the configure
  reply on the mailbox server's *shared* outbound channel, the hazard
  `std/src/console.rs:8-11` documents and avoids.  Task 118 removes the
  shape entirely; until then it is a live two-client race.

Note `RECEIVE_CLOSED = 2` also collides numerically with the
nameserver's `OP_UNBIND = 2` and the console's `R_EFULL = 2` (task 126).

## Design

- One length-checked payload accessor in `r9x_std`, returning
  `Option<&[u8]>` for a requested field, so a server cannot index past
  the end by construction.  The bug is uniform across four servers
  because each hand-rolled the same indexing.
- One closed-channel arm helper for the receive loop, likewise — the arm
  cannot be forgotten in the fifth server if the loop shape provides it.
- A server never exits on client input.  Reply an error; the client is
  the one at fault.
- Reject an over-long bind rather than truncating.
- Make the panic handler exit non-zero and name the process (task 98
  wants this too, and it is what makes all of the above visible).

## Tests

- Integration: a hostile-client image that sends 0-, 1-, and 3-byte
  messages and malformed opcodes to each server in turn; every server
  must still be alive and serving afterwards.
- Integration: a server whose inbound channel is closed exits or idles —
  it must not busy-spin.  Assert via the tick count or a second process
  making progress.

## Done when

- No server indexes a payload it has not length-checked.
- No receive loop treats closed as "try again".
- The hostile-client image passes.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
