---
status: done
---

# irq-route — the IRQ routing table, `try_send`, and `SYSIRQCLAIM`

## Problem

There is no path from a GIC INTID to a `Channel`. The trap handler's IRQ
dispatch handles the timer PPI in-kernel and disables every other INTID. A
user-space process cannot own a hardware interrupt.

## Evidence

- `trap.rs`: the IRQ dispatch is `if intid == timer::intid() { ... } else { iprintln!(...); gic::disable_interrupt(intid); }`.
- `port/ipc.rs`: `send` blocks the sender when the queue is full; there is no non-blocking variant (the interrupt handler cannot block).
- `aarch64/src/ipc.rs`: the channel table is a static array (`NCHANNELS = 4`); there is no INTID-to-channel mapping.
- `gic.rs`: `enable_interrupt` is available (the routing table can enable an SPI at the distributor).

## Fix direction

- **`port/ipc.rs`**: add `try_send<S: IpcScheduler>(sched, ch, msg) -> Result<(), IpcErr>` — a `send` that returns `Err(IpcErr::Full)` instead of blocking when the queue is full. The fast path (a receiver is blocked) is the same as `send` (hand the message, wake it); no PI (the sender is the kernel). The slow path (room in the queue) enqueues and returns `Ok(())`. The full-queue path returns `Err(IpcErr::Full)`.
- **`aarch64/src/ipc.rs`**: add the routing table (`IrqRoute { intid, channel, owner }`, `NIRQS = 16`), the `route(intid) -> Option<&'static Channel>` lookup (linear scan), and the `SYSIRQCLAIM` handler (check INTID in SPI range 32..=1019, check channel handle valid, check not already claimed, add the entry, call `gic::enable_interrupt`).
- **`aarch64/src/trap.rs`**: change the IRQ dispatch to `if intid == timer::intid() { ... } else if let Some(ch) = ipc::route(intid) { let _ = ipc::try_send(&KernSched, ch, msg); } else { ... }`.
- **`aarch64/src/process.rs`**: add `SYSIRQCLAIM: u64 = 19`.

## Done-when

- `cargo xtask ci` is green (all three arches, all host tests).
- The host unit tests cover `try_send` (fast path, slow path, full-queue path) and the routing table lookup (claim, lookup, duplicate-claim error, bad-INTID error).
- The aarch64 target builds (the `SYSIRQCLAIM` handler compiles).

## Origin

Stage 4 of `tasks/plans/microkernel-substrate.md`, designed in
`tasks/plans/microkernel-irq-message.md`. The aarch64 reference
implementation. Stands on stage 2 (IPC) and stage 3 (Aspace). The
integration image (`irq-integration.md`) is the next task.
