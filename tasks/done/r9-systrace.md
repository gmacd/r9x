---
status: done
---

# r9-systrace: syscall trace — debug facility (Tier 3.3)

Add a kernel-side syscall trace that logs every syscall received (number,
arguments) and its response (return value). Disabled by default; zero
overhead when off.

## Motivation

The QEMU mailbox ALLOCATE bug was untraceable because there was no way to
see which syscalls the processes were actually making and what the kernel
returned. A syscall trace would have immediately shown that the mailbox
server never received the configure request (no `SYCRECEIVE` with the
expected payload) and that the display server was receiving a reply from an
unexpected source. This class of bug — "the IPC routing is wrong" — is the
most common failure mode in a message-passing kernel and the hardest to
debug without a trace.

## Design

- **Compile-time gate:** `#[cfg(feature = "systrace")]` (or a `const`
  toggle in `param.rs` — pick whichever is cleaner for the build system).
  When off, the trace function is a no-op (the compiler eliminates the
  branch). When on, every syscall dispatch logs before and after.
- **Format:** one line per syscall:
  ```
  [systrace] proc={id} call #{num} args=({x0}, {x1}, {x2}) -> {x0}
  ```
  For `SYCSEND`/`SYCRECEIVE`/`SYCREPLY`, also log the channel id and
  message length (not the payload — that would be too verbose).
- **Output:** via `devcons` (the kernel console). No ring buffer, no
  user-space reader — this is a debug aid, not a production facility.
  (A ring buffer + `SYS_TRACE_READ` is a future refinement if needed.)
- **Per-core:** the trace is per-core (no global lock). On SMP, each core
  logs its own syscalls. The console is already per-core (the PL011 is
  shared but the writes are atomic at the byte level for short strings).
- **Not in the interrupt path:** the trace is in the syscall dispatch
  (EL0→EL1 transition), not in the interrupt handler. The tick does not
  log.

## Changes

- **`param.rs` or `Cargo.toml`:** the `systrace` feature/const.
- **`trap.rs` (aarch64):** wrap each syscall arm with a trace call.
  A helper `trace_syscall(id, num, x0, x1, x2, result)` that is `#[inline]`
  and compiles to nothing when the feature is off.
- **`devcons`:** no changes needed (the trace uses the existing `println`
  or a dedicated `trace_println`).
- **`xtask`:** a `--systrace` flag for `qemu` that enables the feature
  (or sets the const) for the build.

## Tests

- No integration image needed: the trace is a debug facility, not a
  behavioural change. Verify by enabling it manually and confirming the
  output is sensible.
- A `cargo build --features systrace` compiles clean (no warnings).

## Acceptance

- `cargo xtask ci` green with the feature off (zero overhead, zero warnings).
- `cargo build --features systrace` (or the const toggle) compiles clean.
- Manual verification: enabling the trace on the `mailbox` image shows the
  expected syscall sequence (or reveals the routing bug).

## Not in scope

- A ring buffer + user-space reader (`SYS_TRACE_READ`) — a refinement.
- Trace filtering (by process, by syscall number) — a refinement.
- Trace of interrupt entries (tick, mailbox interrupt) — a separate
  facility.
- Performance counters (syscall latency) — a separate facility.
