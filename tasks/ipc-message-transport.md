---
id: 121
status: open
wave: 5
depends-on: task
---

# Task 121: variable-size payloads and a bulk transfer path

## Status: open — wave 5.  Depends on task 118

## Problem

`Message` is a 256-byte inline array (`port/src/ipc.rs:35`) inside an
8-deep queue inside each of 16 static channels — roughly 33 KB of kernel
memory permanently reserved — and every send costs two copies, user →
kernel → user.

`MSG_MAX = 256` is documented as "QNX's classic message size", but QNX's
256 bytes is the *fast path* of a design that also has `MsgReadv`,
`MsgWritev` and shared memory behind it.  r9x has only the fast path, so
the bound is a ceiling rather than an optimisation:

- `std/src/console.rs` chunks writes into `MSG_MAX - 4` pieces and
  concedes in its own docs that "two clients' output may interleave at
  message boundaries" — the console cannot atomically write a line longer
  than 252 bytes.
- Task 122's 9P `Tread`/`Twrite` will force a chunking protocol into
  every layer above it, and the `msize` negotiation has nothing useful to
  negotiate.

## Precedents

QNX: small messages inline, large ones copied directly between address
spaces by the kernel during `MsgSend`, plus shared memory for the rest.
seL4: short IPC in registers, everything else via shared frames.  Both
tier it; neither reserves the maximum per channel.

## Design

- Split `Message` into a header (opcode, tag, length, receive id) and a
  payload reference.  Keep a small inline buffer for the fast path.
- Task 118's blocking send is what makes the bulk path cheap: the
  sender's buffer stays live for the duration, so `receive` can copy
  sender → receiver directly, sized by `min(sender_len, receiver_cap)`.
  One copy, no intermediate kernel buffer, no per-channel reservation.
- Shrink the per-channel queue reservation accordingly; record the
  before/after static footprint in the commit message.
- Report truncation explicitly.  Today a receive with a small buffer
  silently gets a short message and cannot tell that from a short send.
- Delete the chunking loop and the interleaving caveat from
  `std/src/console.rs`.
- Shared memory is *not* in scope — direct copy alone removes the ceiling
  and one of the two copies.  Revisit when a framebuffer or file-cache
  workload actually asks for it.

## Tests

- Integration: a 4 KB console write arrives as one message and appears
  uninterleaved alongside a concurrent writer.
- Integration: a receive with a buffer smaller than the message reports
  truncation.
- Host: the header/payload split's length arithmetic.

## Done when

- A message larger than 252 bytes is atomic end to end.
- Static channel memory measurably drops.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
