---
status: done
---

# stage6-createchan-syscall: SYCCREATECHAN — a user process can make a channel

Stage 6a, task 1 of 4. Plan:
[plans/microkernel-nameserver.md](../plans/microkernel-nameserver.md).
*Prerequisite for tasks 2–4.*

## Goal

Today a channel handle is minted only kernel-side
(`aarch64::ipc::create()`), so a user-space process — the thing the namespace
is built for — cannot make one. Add the one syscall the user-space nameserver
needs: `SYCCREATECHAN` returns a fresh `ChannelHandle`. No protocol, no server —
the mechanism verb alone.

## Changes

- **`aarch64/src/process.rs`**: a new `x8` value `SYCCREATECHAN: u64 = 21`
  (next free, above `SYSMAPMMIO`=20). `x0` = 0 on return with a fresh handle in
  `x0`; a distinct non-zero error when the table is full. No arguments. Mirror
  the `SYSMAPMMIO` doc-comment shape (the `x0`-in / `x0`-out contract).
- **`aarch64/src/ipc.rs`**: the dispatch arm calls the existing
  `ipc::create()` and returns its handle. The full-table `assert!` in `create()`
  must not fire from a live process — either guard the count before calling and
  return the error, or change `create()` to a `Result` the dispatch maps to the
  error code. Decide in implementation; the observable is "a full table is an
  error in `x0`, not a panic."
- The host (non-`none`) stub for the dispatch arm (the module already stubs the
  real handlers so the trap dispatch compiles under host unit tests).

## Tests

- A host unit test of the dispatch: a fresh `SYCCREATECHAN` returns a handle
  that `ipc::channel(h)` resolves; a 5th create (table of 4) returns the error,
  not a panic.

## Acceptance

- `cargo xtask ci` green across all three arches.
- `SYCCREATECHAN` is the *only* new kernel surface this task adds — no protocol,
  no server, no nameserver code.
- A user process can create a channel and the handle round-trips through
  `ipc::channel`.

## Not in scope

The nameserver, the console server's publish, and the test image (tasks 2–4).
`SYCCREATECHANPAIR` (a connected pair in one call) — refused in the plan
(decision record 2); two calls are the boring, total form. Channel
reclamation / close-on-owner-death — stage 5's known limitation, carried.
