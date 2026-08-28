---
status: done
commit: 2a5a81c
---

# Gate irq.rs's DAIF asm for hosted test builds

`aarch64/src/irq.rs:29-56` emits privileged DAIF asm with no cfg gating:
`unmask_irqs` (`msr DAIFClr, #2`, `:31`), `mask_irqs` (`mrs {daif}, daif`
+ `msr daifset, #2`, `:38-39`), `restore_irqs` (`msr daif, {daif}`,
`:51`). Every other privileged-asm site in the package is gated —
`swtch.rs:3`, `lib.rs:43`, `gic.rs:20`, and `reg/cnt_el0.rs:22-35` (which
has the full `cfg(test)` mock pattern). Since e022999 the aarch64 package
runs as a hosted EL0 test binary on CI, where EL0 DAIF access traps
(SCTLR_EL1.UMA is unset under Linux) — SIGILL.

Nothing reaches the asm today, deliberately: `port::irq::IrqGuard::new`
goes through a `Once<&IrqOps>` that no test registers
(`port/src/irq.rs:52-79`, documented at `:14-17`), so guards are no-ops in
hosted builds. But two footguns await the first test author who steps off
that path:

- `unmask_irqs` is `pub` (`aarch64/src/irq.rs:29`) and executes DAIF
  unconditionally — a direct call from any `#[test]` SIGILLs immediately.
- `set_ops` is a `Once`: a single test calling `boot::irq_ops()` or
  `irq::init()` irreversibly arms every subsequent `IrqGuard` in the same
  test process — including the existing path through
  `aarch64/src/timer.rs:227` via `with_timers` — turning one new test
  into a cascade of opaque signal deaths.

Fix direction: mirror `reg/cnt_el0.rs:22-35` — `cfg(test)` (or
`not(target_os = "none")`) bodies for the three functions that skip the
asm, keeping the real asm for kernel builds.

Done when: the aarch64 test binary can call unmask/mask/restore (directly
or via a registered `IrqOps`) without SIGILL, and all gates stay clean on
all three architectures.

Origin: code review of e022999 (2026-08-20) — "irq SIGILL" verified
PLAUSIBLE (latent hazard, not a live defect). Also the standing blocker
for option 2 of `done/ci-arch-tests-cross-host.md` (cross-host `--tests`),
so fixing it buys back that option for riscv64.

## Status: done (2a5a81c)

Test builds run against a MOCK_DAIF atomic in the reg::cnt_el0 style:
unmask/mask/restore get #[cfg(test)] bodies operating on the mock (which
starts masked, like a booting core), the real asm stays for kernel
builds, and a round-trip test exercises all three — calls that each
SIGILLed before the mock. The Once-poisoning footgun is defused rather
than removed: registering the ops in a test now arms a working mock, not
a trap. Cross-host --tests (option 2 of done/ci-arch-tests-cross-host)
is unblocked for aarch64; re-cost at riscv64's first test.
