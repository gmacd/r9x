---
id: 110
status: open
wave: 2
---

# Task 110: the single waiter slots lose messages, strand senders, and duplicate requests

## Status: open — wave 2 (largely subsumed by task 118)

## Problem

`Channel` has one `send_waiter` and one `recv_waiter`, both
`Option<ProcId>`, both assigned with no check.  Four distinct failures.

**A second waiter strands the first (`port/src/ipc.rs:271`, `:399`,
`:459`).**  Single-core, reachable with two clients and one 8-deep
channel — two processes writing to the console is enough.  Sender B fills
the queue and blocks (`send_waiter = B`); sender A finds it full and
stores `send_waiter = A` over it.  Nothing holds B's id again —
`receive`, `try_receive` and `close` can only wake whoever is in the slot
— so B is `Blocked` forever with its `send` never returning.  Same shape
for two receivers.  The docs describe one-waiter as a design assumption;
the code overwrites rather than rejecting, so the assumption manifests as
a silent permanent hang.

**`send` can return `Ok(())` without enqueuing (`:286-292`).**  The
full-queue branch records the waiter, drops the lock, then re-acquires it
and asks `inner.send_waiter.is_some()` to mean "am I the blocked
sender?".  If a receiver drains in that window it takes the waiter, so
the check sees `None`, the `if !blocked { return Ok(()) }` branch is
taken — and the `else` at `:271` never pushed the message.  Success is
reported and the message is gone.

**Duplicate delivery (SMP).**  The mirror of the same ambiguity: a sender
whose push *succeeded* sees another sender's registration, blocks with no
wakeup owed, and if anything wakes it the loop re-pushes the same
`Message` (it is `Copy`).

**Lost wakeup (`:399`, with `process.rs:1546`).**  The receiver publishes
`recv_waiter = me` under the channel lock, drops it, and only then calls
`sched.block(me)`, which sets `Blocked` under the *table* lock.
`process::wake` is a no-op unless the target is already `Blocked`.  A
sender on another core in that window takes the waiter, pushes, and wakes
a still-`Running` process — the wake evaporates and the receiver sleeps
forever with its message queued.  Single-core is safe only because
`trap.S:126` masks DAIF.I for the whole syscall.

**Stale waiter after a timeout (`:453`).**  `receive_at`'s timeout path
returns without clearing `recv_waiter`.  `cmd/display/src/main.rs:223`
uses `receive_at` on `pacing_chan` purely as a sleep, so that channel
carries a stale waiter for the process's life.  Consequences: the next
`send` PI-boosts a process that is not waiting there and forces a
spurious wake on the channel it really wants; `is_blocked_on` reports
true forever, so `close_all_for` (task 113) closes a channel the process
was never blocked on.

## Design

- Fix the two reachable-today bugs now, ahead of task 118: carry "did I
  block?" out of the first critical section as a local rather than
  re-reading shared state, and clear `recv_waiter` on the timeout path.
- The waiter slots themselves are task 118's problem — the reply-table
  rework replaces this protocol wholesale, and turning the slots into
  queues twice is wasted work.  Until then, reject a second waiter with
  an error rather than overwriting: a returned error is debuggable, a
  stranded process is not.
- The lost-wakeup window needs the sleep-flag/generation shape (commit to
  blocking before the waiter is visible), and task 118 must not rebuild
  it in the new reply table.

## Tests

- Integration: two clients contending on one full channel; both sends
  complete, neither message is lost.
- Integration: a `receive_at` timeout leaves the channel with no waiter —
  assert a subsequent send does not wake the wrong process.
- Task 124's soak image for the SMP halves.

## Done when

- No path reports a successful send without enqueuing.
- A timed-out receive leaves no waiter behind.
- A second waiter is refused, not silently dropped.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
