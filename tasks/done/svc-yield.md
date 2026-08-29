---
status: done
---

# svc-yield: a syscall that returns

Task 1 of 3 in the preemption arc. Plan: [plans/preemption.md](../plans/preemption.md).

## Goal

Make `svc #1` (yield) return to the process instead of killing it, and make
the EL0 return path honest about SP_EL0. After this task the kernel has a
dispatched SVC handler (0 = exit, 1 = yield, n > 1 = terminate with status n)
and a CI-proven proof that a process can cross a `svc` and continue with its
register file and its user stack intact.

## Changes

All in `aarch64/`.

- `src/process.rs`: add `SYSYIELD: u64 = 1` beside `SYSEXIT`.
- `src/trap.rs`: the `SvcAarch64` arm becomes a dispatch on the ESR.ISS
  immediate: 0 → `syscall_exit` (as today); 1 → return (the handler does
  nothing — the vector's restore + `eret` is the yield); n > 1 →
  `syscall_exit(n)` (today's catch-all). Note the Svc arm can now complete,
  so restructure so it does not fall into the unhandled-exception spin loop.
- `src/trap.S`, EL0 tail: before `restore_frame`, restore the user stack
  pointer: `ldr x6, [sp, #280]; msr sp_el0, x6` (slot 280 already holds the
  interrupted sp_el0; x0–x18 are dead until `restore_frame`, so x6 is a
  safe scratch). This is the latent-bug fix: AArch64 `eret` copies no stack
  pointer, so today a resuming process gets SP_EL0 = 0 (verified empirically
  on QEMU — see the plan's Problem section). The frame stays 288 bytes; the
  TPIDR_EL1 x0-parking hack stays (task 2 deletes it).

## Tests

- New integration image `aarch64/tests/user_yield.rs` (+ `[[test]]` entry in
  `aarch64/Cargo.toml`, `harness = false`, `required-features =
  ["qemu-test"]`). The process is hand-encoded AArch64 bytes (encode with
  `clang -target arm64 --assemble` + objdump and verify the round-trip; no
  assembler in the repo): (a) store a marker to `[sp, #8]`, load distinct
  values into x0 and x19; (b) `svc #1`; (c) check all three survived
  (branch on mismatch to a different exit); (d) `svc #1` again; (e) exit
  via the catch-all with a status reachable only if both yields returned to
  the right place. The kernel asserts the status; the prints on either side
  are the assertion a host test could never make.
- The existing `user_process` image must pass unchanged (its whole program
  is one `svc #0`).
- Host tests: the SVC decode/dispatch table in `trap.rs` follows the
  existing `#[cfg(test)]` style.

## Acceptance

- `cargo xtask ci` green (integration images are now 5).
- `cargo xtask qemu --arch aarch64 --image user_yield --timeout 60` passes.
- Without the `msr sp_el0` line the image fails (the `[sp, #8]` store
  faults or is clobbered by the next trap's `save_frame`) — the test is
  load-bearing, not decorative.

## Not in scope

The 304-byte frame (SPSR slot), the kstack vector, TPIDR-as-pointer, the
proc table — all task 2. Yield does not yet reschedule (there is nothing to
reschedule to); the handler body grows to a `resched()` call in task 2.
