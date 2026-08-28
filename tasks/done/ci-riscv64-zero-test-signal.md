---
status: done
commit: c23cdfc
---

# riscv64 test code has zero compile signal, and the skip is silent

riscv64 is the only arch whose `#[test]`/`#[cfg(test)]` code nothing in CI
even compiles, and every skip that produces this state is invisible:

- `TestStep` never selects riscv64: it only adds the arch matching the
  host (`xtask/src/main.rs:751-755`), and no hosted riscv64 runner exists.
- `CheckStep` skips its test/bench targets with a bare `continue`
  (`xtask/src/main.rs:928-931`) when `std_supported_target` returns None;
  `ClippyStep` has the identical silent guard (`:824-829`).
- `std_supported_target` (`:124-127`) accepts only the toolchain triple or
  `<arch>-unknown-linux-gnu`, and `rust-toolchain.toml:4-13` deliberately
  installs no `riscv64gc-unknown-linux-gnu` ("get no such target").
- The workspace-wide `--bins` passes build the bare-metal target, which
  never compiles `cfg(test)` code.

Net: the first riscv64 `#[test]` would pass every CI job green even if it
does not compile. The only acknowledgement is prose in e022999's commit
message ("revisit at its first test (task ci-arch-tests-cross-host)") —
that task is in `done/` and nothing in the repo carries the reminder.

Fix direction: minimum honest fix is a loud skip — check/clippy/test each
print one line naming the arch and the reason when they drop it, so a
riscv64 test author sees the gap the day they hit it. The full fix is
option 2 of `done/ci-arch-tests-cross-host.md` (make the non-test lib
host-compilable so `--tests` works cross-host), re-costed when riscv64
grows its first test.

Done when: no arch's test targets can be dropped from check, clippy, or
test without a line of output saying so.

Origin: code review of e022999 (2026-08-20) — "riscv64 gap" CONFIRMED.
Same code region as [xtask-test-silent-arch-skip.md]; the loud-skip halves
should land as one change.

## Status: done (c23cdfc) — the loud-skip half

check and clippy print one line naming the arch whose tests and benches
they drop when no std-capable target is installed, so a riscv64 test
author sees the gap the day they hit it (the skip fires on CI; local
machines with riscv64gc-unknown-linux-gnu installed genuinely cover it,
correctly printing nothing). The full fix — cross-host `--tests` via a
host-compilable lib — remains gated on [irq-daif-asm-test-gating.md]
and riscv64's first actual test, per the original plan.
