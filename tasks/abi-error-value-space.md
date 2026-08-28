---
id: 126
status: open
wave: 3
---

# Task 126: the error space overlaps the value space

## Status: open — wave 3.  Small; land it before 118/119

## Problem

There was never a project-wide decision on how a syscall returns error
versus value, so each one improvised, and the collisions are real:

- **`ERR_NO_SLOT = 8` is also a valid channel handle.**
  `NCHANNELS = 16`, so handles 0–15 are all valid and
  `sys_createchan` (`aarch64/src/ipc.rs:458`) returns `8` on exhaustion.
  Nobody can detect the failure — `cmd/mailbox/src/main.rs:205` tests
  `if in_h > 15` with the comment "ERR_NO_SLOT is > 15", which is false.
  A process that overflows the table then sends and receives on another
  server's live channel.
- **`RECEIVE_CLOSED = 2`** collides with the nameserver's
  `OP_UNBIND = 2` and the console's `R_EFULL = 2`, so a closed channel is
  indistinguishable from an unbind request (task 114).
- **`SYS_ALLOC` returns `1` for failure** and a virtual address otherwise
  (`aarch64/src/ipc.rs:440`), relying on "a heap VA is never that small".
- **`SYSEXIT = 0`'s number doubles as its status** — the ABI doc at
  `abi/src/lib.rs:48-52` already flags it: "status 1 is not expressible
  (1 is yield) and every new syscall number retires one exit status".

`RECEIVE_TIMEOUT = 0xffff` (`abi/src/lib.rs:204`) is the one that got it
right: reserved, documented, unambiguous, with a stated rule that a
protocol must not send that opcode.  Generalise that.

This is cheap now and viral later: task 119 makes handles denser, and
task 122 adds a whole protocol numbering space on top.

## Design

- Choose one convention and write it down.  Recommendation: a dedicated
  status register separate from the value register — aarch64 already
  returns multiple registers (`sys` returns a 3-tuple), so it costs
  nothing and removes the entire class.
- One error enum in `abi`, with values that cannot alias any handle,
  address or opcode.
- Separate the exit-status argument from the syscall number.
- Reserve an opcode range for kernel-originated results, so
  `RECEIVE_CLOSED` and `RECEIVE_TIMEOUT` can never collide with a
  protocol verb — task 122 then inherits the guarantee for free.
- Fix `cmd/mailbox:205`'s guard and its comment.

## Tests

- Host: a pinning test asserting no error value is a valid value in its
  own return position — the same shape as the existing ABI pinning test.
- Integration: exhaust the channel table and observe a distinguishable
  error.

## Done when

- One documented convention, applied to every syscall.
- The pinning test exists.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
