---
id: 125
status: open
wave: 6
depends-on: 118, 124
---

# Task 125: threads inside an address space, and concurrent servers

## Status: open — wave 6.  Depends on 118, 124

## Problem

`std/src/thread.rs` is honest about it: "a thread is a *process*".  There
is no execution context inside an address space, and `recv_waiter` is a
single slot.  So **every server is a serialized state machine**: one slow
request blocks every client of that server.

QNX servers are multi-threaded by default for exactly this reason, and
the constraint silently shapes every protocol designed while it holds —
task 122's 9P dispatch loop is being designed against it right now.

The immediate action is not the implementation.  It is to **write the
constraint down in `AGENTS.md`**, because it currently reads as a design
choice and is an accident.  A protocol author who knows "no server can
overlap two requests" designs differently from one who assumes it will
become possible.

## Design

- Split `Process` (address space, handle table, namespace) from `Thread`
  (context, kstack, state, priority, deadline).  Today these are one
  struct (`aarch64/src/process.rs:219`, `:240-275`), which is also why
  task 112's slot-identity confusion is possible.
- The run queue becomes a queue of threads; the address-space switch
  becomes conditional on the thread's process changing.
- `recv_waiter` becomes a queue (task 110 leaves the slots for here);
  `receive` hands the message to whichever thread is waiting.
- `SYS_THREAD_CREATE` taking an entry point and a stack, plus a
  thread-local storage convention.
- A thread-pool server helper in `r9x_std`; convert the console server to
  it as the proving case.
- Task 116's note comes due here: `user_range_ok`'s validate-then-copy
  window is safe today only because one thread owns each address space.
  Threads break that; design the fix with this task, not after it.

## Tests

- Integration: a server with a thread pool serves a second client while
  the first is blocked in a slow request.
- Integration: two threads in one address space observe each other's
  writes and are scheduled independently.

## Done when

- A server can overlap requests.
- The serialized-state-machine constraint is removed from `AGENTS.md`
  rather than merely documented there.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
