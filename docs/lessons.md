---
covers: aarch64/src/timer.rs, aarch64/src/gic.rs, port/src/mcslock.rs, port/src/ipc.rs, xtask/src
sources: the code cited per entry; debugging sessions, dated
verified: b11022f (2026-08-29)
---

# Lessons learned

Gotchas that cost real debugging time. Each entry states the rule, shows the
shape, and cites code in the tree that demonstrates it — so a reader can check
whether the lesson still holds rather than trusting this page.

Add an entry the day you lose an hour to something. Delete an entry when the
underlying cause is gone.

## bitstruct — use withers, never mutate

`bitstruct` generates immutable structs with getter methods (`enable()`,
`imask()`) and **wither methods** (`with_enable(true)`, `with_imask(true)`).
Build a new value with the wither chain; there is no in-place mutation.

```rust
// Good — immutable wither chain
let ctl = CntpCtlEl0::read().with_enable(true).with_imask(false);
CntpCtlEl0::write(ctl);

// Bad — doesn't compile, bitstructs are immutable
// self.iss = 123;
```

Witnesses: `aarch64/src/gic.rs:283` (`GicdCtlr(0).with_enable(true)`),
`aarch64/src/timer.rs:108`.

## aarch64_cpu — prefer `.set()` over inline asm

`aarch64_cpu::registers` provides `Writeable::set()` on all register types.
Use it instead of inline assembly.

```rust
// Good — uses aarch64_cpu's Writeable trait
CNTP_CVAL_EL0.set(value);

// Bad — verbose, error-prone
unsafe { core::arch::asm!("msr cntp_cval_el0, x0", in("x0") value, ...) };
```

Note that `aarch64/src/reg/` also defines r9's own bitstruct register types
for some of these registers, and `aarch64/src/timer.rs` writes the whole of
`CNTP_CTL_EL0` rather than read-modify-write. Follow whichever pattern the
neighbouring code already uses.

## Inline asm register names need underscores

System register names in inline asm must match the architecture naming
convention with underscores between components:

```asm
msr cntp_cval_el0, x0   // correct
msr cnctp_cval_el0, x0  // WRONG — missing underscore between CNTP and CVAL
```

## `LockGuard` requires `let mut`

`Lock::lock()` borrows the lock node and returns a `DerefMut` guard. The guard
must be declared `let mut`, or the borrow to `&mut *guard` fails:

```rust
let mut guard = LOCK.lock(&node);  // must be mutable
func(&mut *guard);
```

Witnesses: `port/src/mcslock.rs:97` (the guard-returning `lock`),
`port/src/ipc.rs:216`.

## `NonNull` has no `is_null()`

Use `*mut T` when null-checking is needed. `NonNull<T>` cannot be null-checked.

## Raw pointer comparison uses `ptr::eq`

For raw pointers, use `ptr::eq(a, b)` instead of `==`, which clippy rejects.

## QEMU — never let a run outlive the command that started it

A hung QEMU (kernel spin, missing semihosting exit) maxes the CPU until
killed. Run guests with `cargo xtask qemu`: every run is bounded by
`--timeout` (default 15 s; `--timeout 0` waits indefinitely, e.g. for gdb),
the guest is killed when the deadline expires, and pass / fail / timeout is
reported as the command's outcome. `--image <name>` builds and runs one of
the arch's test images — e.g. `cargo xtask qemu --arch aarch64 --image
user_process` — and without it the kernel image runs. The `xtask
integration-test` harness bounds runs the same way.

Only when that does not fit (e.g. serial to a file) run bare QEMU in the
**foreground**, with the bound in front of it — the sleep expires, QEMU is
killed, the pipeline returns, and nothing is left to track:

```bash
sleep 12; qemu-system-aarch64 -M raspi4b \
  -dtb aarch64/lib/bcm2711-rpi-4-b.dtb -nographic \
  -serial null -serial file:/tmp/serial.txt \
  -semihosting -no-reboot -kernel <image.gz>
cat /tmp/serial.txt
```

Do **not** use `qemu ... & PID=$!; sleep N; kill $PID`: when the surrounding
command is interrupted or the tool times out, the background QEMU is orphaned
and no later command owns the kill. If a session used background QEMU, verify
before finishing with `pgrep -fl qemu-system` (expect none);
`pkill -9 -f qemu-system-aarch64` clears strays.

## `debug_assert!` wraps a predicate, never a side effect

`debug_assert!(e)` expands to `if cfg!(debug_assertions) { assert!(e) }`. With
debug assertions off (release builds) `e` is never evaluated, so any side
effect inside it vanishes silently. The IPC fast path kept its enqueue inside
the assert, so a release image dropped every fast-path message and deadlocked
on the first send (task 102). Do the work in its own statement; assert only
on the result.

```rust
// Good — the push runs in every build; the assert checks it
let ok = inner.queue.push(msg);
debug_assert!(ok);

// Bad — in release the push never runs; the message is silently dropped
debug_assert!(inner.queue.push(msg));
```

The workspace denies `clippy::debug_assert_with_mut_call`. It catches a
direct `&mut` receiver but **not** a receiver reached through a `DerefMut`
guard — which is exactly the shape above (`inner.queue.push`), so the lint
would have let this bug through. The load-bearing guard is the CI job that
builds and *boots* a release image (task 102): a dropped message deadlocks
the boot. The rule, stated fully: assert on a pure predicate, and keep
every side effect out of the macro.

Witness: `port/src/ipc.rs`, the `send` and `try_send` fast paths.

## let-else reads and writes of one index do not sequence

The workspace denies `clippy::mixed-read-write-in-expression` (root
`Cargo.toml`). A let-else that **reads** an index in its scrutinee and
**writes** it in the else branch is rejected: `lines[i]` in the scrutinee
plus `i += 1` in the else is a mixed read/write as far as the lint is
concerned. Bind the value in its own statement first — the read is then
plainly sequenced before the write:

```rust
// Good — the read of `i` is its own statement
let line = lines[i];
let Some((number, file)) = parse_entry_head(line) else {
    rebuilt.push(line.to_string());
    i += 1;
    continue;
};

// Bad — denied: read in the scrutinee, write in the else
let Some((number, file)) = parse_entry_head(lines[i]) else {
    ...;
    i += 1;  // error: unsequenced read of `i`
};
```

Witness: `xtask/src/tasks.rs`, the rebuild loop in `fix()`.

## A timing test must not assert a host-scheduling-dependent count

An integration image that proves “the periodic timer re-armed” by counting
fires (`fast >= 2`) asserts a quantity that scales with the vCPU’s *host*
service rate, not with the timer’s correctness.  The `timers` image runs
under QEMU with its deadline measured on the physical counter (real host
time); on a loaded runner the vCPU is starved for tens of ms, so a 5 ms
timer’s first interrupt sits unhandled until the vCPU finally runs, the
handler fires it **once** and re-arms to the clamped future — past the
one-shot that has already cancelled it.  `fast` lands on 1 and `fast >= 2`
fails: under load a *working* periodic and a *broken* one are
indistinguishable by count (task 133).

```rust
// Good — one fire proves the hardware path (CVAL -> PPI -> GIC -> trap);
// the re-arm logic is unit-tested against a mocked counter
check!(fast >= 1, "fast periodic fired via the hardware path, {fast} fires");

// Bad — fire count tracks vCPU time, not correctness; flakes on a loaded host
check!(fast >= 2, "fast periodic re-armed before cancel, {fast} fires");
```

The split that makes it robust: the *logic* (re-arm, self-stop, cancel) is
proven deterministically by unit tests against a mocked counter
(`periodic_rearm_clamps_missed_deadlines`,
`periodic_stops_when_callback_returns_false`); the integration test proves
only what needs real hardware — that a fire arrives through the PPI, and
that the level-triggered interrupt deasserts (the load-robust quiescence
check).  Rule: in a timing test, assert a *fact the hardware produces*,
never a *count that depends on how fast the host ran the guest*.

Witness: `aarch64/tests/timers.rs`; the re-arm clamp at
`aarch64/src/timer.rs:351`.

## The recursive slot is root-only in the post-MMU; a blanket `index == 511` guard is wrong

The self-pointer that lets the kernel reach its own page tables through the
MMU (the recursive alias) is written by init, never by the walk — so a VA
whose index is that slot must be refused, not followed, or the walk maps into
the live page tables.  But *how far the self-pointer reaches* differs by
phase, and the guard must match:

- The **post-MMU** sets entry 511 of the *root* only: `next_mut` allocates
  every other table `clear()`ed (`aarch64/src/vm.rs`).  So a VA with index 511
  at L1 or L2 (e.g. `RECURSIVE_SLOT << 21`) is an *ordinary* slot and must
  still map.  A blanket `if index == 511` guard — the obvious port of the
  pre-MMU check — would wrongly refuse it.
- The **pre-MMU** sets entry 511 of *every* table it allocates
  (`new_table.entries[RECURSIVE_SLOT] = entry`, vminit.rs), so there index 511
  is a self-pointer at any level and the blanket check is correct.

```rust
// Good (post-MMU) — only level 0 has the self-pointer
if level == Level::Level0 && index == RECURSIVE_SLOT {
    return Err(PageTableError::MappingRecursiveIndex);
}

// Bad (post-MMU) — refuses a legitimate L1/L2 index-511 VA
if index == RECURSIVE_SLOT {
    return Err(PageTableError::MappingRecursiveIndex);
}
```

Rule: before porting a guard between two walkers, check whether they build
their tables the same way.  These two share the *slot* (511) but not the
*structure*, so they need different level checks.  The positive test
(`map_to_allows_index_511_below_level0`) pins the post-MMU case: it fails under
a blanket guard.  (KZERO is L0 index 256, not 511 — the kernel is not in the
recursive slot.)

Witness: `aarch64/src/vm.rs` (`RECURSIVE_SLOT`, `Table::next_mut`);
`aarch64/src/pre_mmu/vminit.rs` (`next_mut`).
