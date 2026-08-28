---
id: 122
status: open
wave: 6
depends-on: 118, 121
---

# Task 122: 9P as the server protocol

## Status: open — wave 6.  Depends on 118, 121.  Pairs with task 78

## Problem

`OP_BIND = 0`, `OP_WRITE = 0`, `OP_CONFIGURE_FB = 0`, and `R_OK = 0`.
Opcode zero means four different things depending on which channel you
are on, by explicit convention — `std/src/console.rs:34-36` states it:
"the same number under a different verb, as each server numbers its own
verbs from zero".

That is precisely the property 9P exists to eliminate.  A self-describing
protocol is what makes any server mountable and makes local and remote
the same code; it is also the half of "Plan 9 ideas" that r9x has not
started.  Every ad-hoc protocol written before 9P lands gets rewritten
when it does — and each one currently bakes in the reply-channel
convention task 118 removes, so they would be rewritten twice.

Task 78 (`r9x-std-servers.md`) is the client half of this and is parked
on "fs/dev/net servers landing".  The codec both halves need is the same;
land it once, here.

## Design

- `port::ninep`: message encode/decode as pure functions over byte
  slices.  Host-testable, with malformed/truncated-input tests from the
  first commit — this is a parser taking bytes from another process, so
  task 115's lesson applies before the code exists rather than after.
- A server-side helper in `r9x_std`: a fid table plus a dispatch loop the
  server fills in with callbacks, so a new file server is a handful of
  functions rather than a hand-rolled message loop.  This is also where
  task 114's length-checked accessors and closed-channel arm live, so
  they cannot be forgotten again.
- `msize` vs `MSG_MAX`: task 121 must land first or the negotiation has
  nothing to negotiate.  Record the chosen `msize` and why.
- No `size[4]` framing — messages arrive on channels with a length
  already, per task 78's note.
- Port order: console first (`Twrite` to one file, retiring `OP_WRITE`),
  then display (framebuffer as a file, mode as a `ctl` file, retiring
  `OP_CONFIGURE_FB`), then mailbox (the property interface is naturally a
  small directory), then the nameserver's verbs in favour of
  `Tattach`/`Twalk` against a mount table — which is where task 123
  picks up.

## Tests

- Host: every message type round-trips; truncated, oversized and
  misaligned input is rejected, never panics.
- Integration: a generic client walks from the root to `/dev/console` and
  writes to it without knowing which server owns it.
- Grep gate: no opcode constants defined in `cmd/`.

## Done when

- The only protocol numbers in `cmd/` come from `port::ninep`.
- Task 78 unparked and built on this codec.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
