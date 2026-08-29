---
covers: aarch64/src/timer.rs, aarch64/src/gic.rs, port/src/mcslock.rs, xtask/src
sources: the code cited per entry; debugging sessions, dated
verified: afdea4f (2026-08-28)
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
