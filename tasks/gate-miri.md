---
status: open
---

# gate-miri: Miri on the host-side tests

Task 5 of 7 in the gates-hardening arc. Plan:
[plans/gates-hardening.md](plans/gates-hardening.md).

## Goal

The host path of `port` (MCS lock, atomics, IRQ-depth logic,
allocator) runs real atomics and real `unsafe` under `cargo test` —
code QEMU never sees, and exactly where a pointer-provenance or
ordering bug would be hardest to attribute. Miri checks it.

**Coverage caveat (2026-08-27 audit): the headline targets have no
tests to run.** `mcslock.rs` and `allocator.rs` contain zero
`#[test]`s — port's 55 tests live in bitmapalloc/irq/elf/mem/ipc/
maths/lib/once — so `cargo miri test -p port` as specced exercises
`once.rs` and the ipc/elf/mem tests: worthwhile but modest, and not
what the plan's "MCS lock, atomics, allocator" sweet-spot claim
describes. **This gate lands with task 97 (mcslock-loom-tests) or
after it** — the lock/allocator host tests are what make miri (and
`-Zmiri-many-seeds`) earn its slot; loom is the stronger tool for
schedule exploration and lives in 97.

Verified premises: `port/src` and its deps `r9x-abi`/`r9x-core`
contain zero `asm!` (Miri's inline-asm limitation does not bite);
the `miri` component exists for the current pin nightly-2026-08-21
(re-verified 2026-08-27; the original check was against the old
pin).

## Changes

- `rust-toolchain.toml`: add `miri` to components.
- CI: a step (or job) running `cargo miri test -p port`.
- If a lock test spins under the interpreter and a timeout appears:
  bound the affected test rather than letting the CI timeout be the
  discovery mechanism (record whatever is needed in the
  resolution).

## Acceptance

- `cargo miri test -p port` green in CI on the pinned nightly.
- A deliberately aliasing host test fails under Miri (local check
  before shipping; do not land the aliasing test).

## Not in scope

`--workspace` (a later one-flag change if noise is manageable); the
bare-metal builds (out of scope by construction — Miri validates the
host build only, and that is the point).
