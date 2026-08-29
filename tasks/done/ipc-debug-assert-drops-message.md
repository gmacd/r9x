---
id: 102
status: done
commit: a9e1c01
---

# Task 102: IPC fast path drops every message in a release build

## Status: done (a9e1c01)

## Problem

`port/src/ipc.rs:327` (and `:264`) put the enqueue *inside* the
assertion:

```rust
debug_assert!(inner.queue.push(msg));
```

`debug_assert!(e)` expands to `if cfg!(debug_assertions) { assert!(e) }`.
With debug assertions off the expression is never evaluated, so
`queue.push(msg)` never runs.  `[profile.release]` in the workspace
`Cargo.toml` does not set `debug-assertions`, and `--release` is a
first-class `xtask` option on every build subcommand.

In a release image the fast path therefore takes the `recv_waiter`, wakes
it, and returns `Ok(())` with nothing enqueued.  The woken receiver loops,
finds an empty queue, re-blocks, and never wakes again.  All IPC
deadlocks on the first message to a blocked receiver.  `try_send`
(`:327`) is the IRQ→server path and has the same bug.

Nothing catches this today because every gate runs the dev profile.

## Design

- Hoist the call out: `let ok = inner.queue.push(msg); debug_assert!(ok);`
  at both sites.
- Set `debug-assertions = true` and `overflow-checks = true` in
  `[profile.release]`.  A kernel that trades its bounds checks for speed
  it has not measured is the wrong trade at this stage, and several
  arithmetic bugs in tasks 106/107 are caught by exactly these.
- Add a CI job that builds *and boots* a release image, so the class
  cannot recur silently.  This is the real fix; the two-line change is
  the symptom.

## Tests

- Integration: the existing `two_clients` image run under `--release`.
  It hangs today and must pass.
- CI: release build + boot added to the `xtask ci` gate list.

## Done when

- Neither `debug_assert!` in `port/src/ipc.rs` carries a side effect.
- A release image boots and passes the integration suite in CI.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28
(plans/architecture-review-2026-08.md).
