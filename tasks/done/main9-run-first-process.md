---
status: done
---

# main9: print, run the first process, print again

The feature itself, in the kernel image (plan:
plans/first-user-process.md). In `main9`, where the "Set up a user
process" section currently initializes and switches the user table and
then falls through to the ticker loop:

```rust
println!("starting first process");
let status = process::run(TEXT, USER_TEXT_VA, USER_STACK_VA);
println!("first process returned, status {status}");
```

`TEXT` is the smallest possible user program: a few instructions
ending in `svc #0` (sysexit) — the same 12-byte sequence the test
image uses, or shorter. Everything else in `main9` (tickers, the
`loop {}`) stays: the process runs while the 1 s timer fires, which is
the point of entering EL0 with IRQs unmasked.

This task is last in the arc: the mechanism is "done" in CI via the
test image (user-process-switch-test), and the feature is done when
the kernel image does it.

Done when: `cargo xtask dist --arch aarch64` and a manual QEMU run of
the kernel image show "starting first process", the exit handler's
print, and "first process returned, status 0" in order, with the
timer ticking through; gates clean on all three arches.

Origin: plans/first-user-process.md — task 5 of 5; the user's stated
milestone ("the kernel would print just before starting the process,
then again once the process returned").

## Done (6b36a76, first-user-process branch)

main9's "Set up a user process" section now starts the process right
after switching the user table live: print, `process::run`, print the
status.  The text is the 4-byte `svc #0` (the same program the test
image runs -- the task's "12-byte sequence, or shorter").  A 12 s
QEMU run of the kernel image shows "starting first process", the
exit handler's print, "first process returned, status 0", then the
timers ticking through; gates clean on all three arches.
