---
status: done
commit: 862b508
---

# Derive the arch set from Arch::ALL everywhere

The supported-architecture set is re-spelled as string literals in
`TestStep` (`xtask/src/main.rs:751`) and `CheckStep` (`:891-918`, `:928`)
and as a hand-written three-arm match in `exclude_other_arches`
(`:1630`), instead of being derived from the single source `Arch::ALL`.

Adding a fourth architecture to `Arch::ALL` leaves the stray lists stale
with no compiler signal: the new arch's unit tests and checks silently
never run, and `exclude_other_arches` stops excluding it from other
arches' workspace builds — the same silent-green shape as the riscv64
gap, but for every future arch.

Fix direction: derive all three sites from `Arch::ALL` (e.g.
`Arch::ALL.iter().filter(|a| **a != arch)` for the excludes; iterate
`Arch::ALL` for the check/test package lists), so a new arch is
exhaustively handled or fails to compile.

Done when: `rg '"aarch64"|"x86_64"|"riscv64"' xtask/src` finds no
arch-set literals outside `Arch` itself, and adding a variant to `Arch`
either just works or produces compile errors at every site that needs a
decision.

Origin: code review of xtask/CI (2026-08-20, high effort) — CONFIRMED.
Batch with [xtask-host-target-detection.md],
[xtask-test-silent-arch-skip.md], [ci-riscv64-zero-test-signal.md].

## Status: done (862b508)

TestStep, CheckStep and exclude_other_arches all derive from
`Arch::ALL` via a new `Arch::package()` helper (itself derived from the
Display impl, so no new list). A fourth arch variant now flows to every
site or fails to compile.
