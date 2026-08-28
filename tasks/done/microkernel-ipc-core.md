---
status: done
---

# microkernel-ipc-core: port::ipc — Channel, bounded Message, send/receive/reply

Task 2 of 7 in the microkernel-substrate arc. Plan:
[plans/microkernel-substrate.md](plans/microkernel-substrate.md). Lands after
microkernel-priority-pi; lands before the Aspace / IRQ→message stages.

## Goal

Add the central primitive: `port::ipc`. A **Channel** (a bounded message queue +
a wait set) and a bounded, typed, tagged **Message**, with `send` / `receive` /
`reply`. This is the "strict interface" the substrate is named for: a fixed-size,
opcode-dispatched, tag-correlated message that carries *ownership* (the buffer
is moved across the boundary) rather than shared state. It is **arch-agnostic in
`port`** — each arch contributes only "block this thread and wake it," which
reuses the scheduler — and it wires priority inheritance to the channel in
QNX's shape: a server blocked in `receive` runs at the priority of the client
that sends to it (server-at-client).

Two processes talking across a channel, with the kernel containing *no*
filesystem, driver, or protocol — only the channel, the scheduler, and the
buffer — is the whole proof this task must deliver.

## Changes

- `port::ipc` (new module, `no_std`, arch-agnostic):
  - `Message { opcode: u16, tag: u32, buf: [u8; MSG_MAX], len: u16 }`. `MSG_MAX`
    is a `const` (fast path ~ 256 B, matching QNX's classic size). The message
    is `Send` and *moves* across the boundary; it is never aliased. No richer
    kernel concept — a server's protocol is its own encoding of `buf`, not a
    kernel type.
  - `Channel { queue: BoundedQueue<Message>, waiters: WaitList, owner: ProcId }`.
    Queue capacity is a `const`; the slots are **pre-allocated** (no allocation
    on the hot path, ever). `IpcErr` covers channel-closed, queue-full, bad-tag.
  - `send`, `receive`, `reply`. The send has exactly two shapes:
    - **fast** — a receiver is blocked on the channel: hand the buffer to it
      directly (move, no copy), wake it; if that *sender's* (client's) priority
      exceeds the *receiver's* (server's), `boost` the **receiver** to the
      sender's priority for the exchange (stage-1 API) and `unboost` when it
      next blocks. This is where PI *triggers* — QNX's server-at-client rule.
    - **slow** — no receiver blocked: copy into a free queue slot; if the queue
      was empty, wake a waiter if there is one. A `send` to a *full* queue
      **blocks the sender** (total function, no drop mode — see plan Decision 6).
  - Blocking is **the scheduler's** blocking, not a new mechanism: a process
    blocked in `receive` (or a full `send`) is put into a Blocked state on the
    channel's `WaitList` and woken by a matching send. This reuses the existing
    `resched`/`swtch` path.
- `aarch64::process.rs` (the only per-arch change, reference implementation):
  - **The priority type.** The stage-1 `Priority` enum (`User`, `Kernel`)
    becomes a QNX-shaped 256-level range: an inverted index (0 most urgent),
    255 reserved as a never-scheduled sentinel, live range 0–254. This refines
    stage 1's capability — `boost`/`unboost`/`pick_next` semantics are
    unchanged, only the type of the value changes — and makes the PI guarantee
    load-bearing (a middle-urgency server is distinguishable from the lowest
    work, so the boost has something to do). The stage-1 `prio` image and its
    host tests move to the new values and still prove the same thing: a boosted
    process outranks its same-base peers, and the image fails under pure
    round-robin.
  - Add the scheduler's first **blocking transition**: a process can be put to
    sleep (off the ready set, onto a channel's wait list) and woken (back onto
    the ready set). Today the only blocking-ish transition is exit; this is the
    first "sleep until woken" state. Woken processes enter through the same
    `swtch`/first-entry path a freshly-scheduled process uses (no first-entry
    special case — the forkret discipline from the preemption arc).
  - The context switch to a woken process is the existing `swtch`; the slow-path
    buffer copy is safe Rust in `port`. There is no new per-arch assembly.
- x86-64 / riscv64: **gate-green only**. They compile `port::ipc` but the
  blocking transition is aarch64-first; the other arches' schedulers gain it when
  this arc reaches them (aarch64 is the source of truth, as with the preemption
  and first-user-process arcs).

## Tests

- **Host unit tests** (`#[cfg(test)]`, `port::ipc`): FIFO ordering on a channel;
  a full queue blocks the sender until a receive drains a slot; the fast path
  hands off without a copy (observable via a marker the receiver sees in the
  exact bytes sent); `tag` correlation on `reply`; `boost` is applied when the
  *sender* (client) outranks the *receiver* (server) — the server is boosted to
  the client — and `unboost` runs after the exchange; `IpcErr` on closed
  channel and bad tag. No allocation is reachable in `send`/`receive` (the slots
  are pre-allocated — assert the allocator is not called on the hot path).
- **Integration image** (aarch64, `harness = false`, `required-features =
  ["qemu-test"]`, `[[test]]` entry in `aarch64/Cargo.toml`): a **client** and a
  **server** user process connected by a channel, plus a **busy** process that
  yields in a loop. The client (high priority) sends N tagged requests; the
  server (low priority) receives them in order and `reply`s a confirmation for
  each; the client receives the replies. The kernel asserts all N arrived in
  order and replies matched tags; the image exits with a status the kernel
  checks. The message payload is **opaque bytes, not 9P** (9P encoding is stage
  7).
- **Determinism check** (the load-bearing one): the *client's* request must be
  answered by its *server* before any lower-priority work — asserted as **zero
  busy-process switch-ins between the client's send and the client switching back
  in with its reply**. This is "IPC is just a context switch" made concrete: the
  server, boosted to the client's priority, runs ahead of the busy process.
  Remove the fast-path `boost` and the busy process switches in (≥1), violating
  the check. Measured by a switch-in counter, not a wall clock: under QEMU TCG a
  timer or cycle threshold is flaky and cannot be shown to bound the wait, but
  the ordering is exact and is the property PI actually guarantees.

## Acceptance

- `cargo xtask ci` green across all three arches (x86-64/riscv64 compile
  `port::ipc`; the image is aarch64-only, matching `user_process`).
- The two-process image passes:
  `cargo xtask qemu --arch aarch64 --image <ipc> --timeout 60`.
- The determinism ordering holds with PI (zero busy switch-ins between the
  client's send and its reply) and is violated without the fast-path `boost`
  (≥1 busy switch-in) — the test proves PI is doing work, not decoration.

## Not in scope

- Per-process `Aspace` (stage 3) — both test processes share the kernel user
  table for now, exactly as `first-user-process` did; isolation arrives in stage
  3.
- IRQ → message (stage 4), the console server (stage 5), the nameserver /
  namespace (stage 6), and 9P (stage 7).
- A generic kernel "message router" — the kernel does one thing with a message
  (queue it, wake a waiter); name→channel and IRQ→channel resolution are later
  stages, not a kernel abstraction here.
- x86-64/riscv64 *exercise* of the blocking transition — they compile it; aarch64
  is the reference and the only arch the image runs on this stage.
