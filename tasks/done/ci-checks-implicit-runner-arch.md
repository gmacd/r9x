---
status: done
commit: 9fa5203
---

# checks' unit-test coverage hangs on ubuntu-latest being x86_64

Which arch package `cargo xtask test` covers is purely a function of the
runner's CPU (`std::env::consts::ARCH`, `xtask/src/main.rs:737,751-755`).
The `checks` job is the only place x86_64 unit tests could run, and it
selects them implicitly via `runs-on: ubuntu-latest`
(`.github/workflows/xtask.yml:18`); `aarch64-tests` is the only job with
an explicitly arch-pinned label (`ubuntu-24.04-arm`, `:43`). If GitHub
repoints `ubuntu-latest` or someone edits line 18 to an ARM label, x86_64
drops out of the test gate with everything still green.

Impact today is nil — `x86_64/src` has zero `#[test]`s, and the QEMU
integration image is host-arch-independent — so this is latent and
pre-existing, not introduced by e022999. But the first x86_64 unit test
makes it real, silently.

Fix direction: pin `checks` to an explicit x86_64 label (e.g.
`ubuntu-24.04`) so the pairing with `aarch64-tests` is symmetric and
intentional, or at minimum a comment on line 18 recording that the test
gate's x86_64 coverage depends on this label's architecture.

Done when: no job's arch-package test coverage depends on a floating
runner label.

Origin: code review of e022999 (2026-08-20) — "ubuntu-latest drift"
PLAUSIBLE, low severity.

## Status: done (9fa5203)

`checks` pinned to `ubuntu-24.04` (what ubuntu-latest resolves to
today, so no image change) with a comment recording that the job's
x86-64 test coverage depends on the label's architecture.
