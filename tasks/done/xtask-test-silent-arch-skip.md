---
status: done
commit: c23cdfc
---

# `cargo xtask test` silently narrows to host-arch packages

`TestStep::run` (`xtask/src/main.rs:734-784`) builds `port` plus only the
arch package matching `std::env::consts::ARCH` (`:750-755`) and never
tells the user what it dropped: the only `println!` in the loop is the
verbose-gated `Executing {cmd:?}` (`:771`), and non-verbose runs add
`--quiet` (`:768`). On an x86_64 machine the aarch64 package's tests are
skipped with zero output naming them.

The surrounding surfaces actively suggest otherwise:

- `CiStep::run` prints per-arch headings for clippy and dist but a single
  unqualified `heading("test")` (`xtask/src/main.rs:1564`) before
  `heading("ok")` — the output shape implies the test gate is
  arch-complete like its neighbours.
- AGENTS.md says "All tests must pass" and lists `cargo xtask test`
  plainly, with no host-dependence caveat anywhere.
- The clap help is just "Runs unit tests" (`:187`). The real contract
  lives only in a code comment (`:741-749`).

Severity is low — CI covers aarch64 via the `aarch64-tests` job one
round-trip later — but a local `cargo xtask ci` reporting `ok` while
having run none of the aarch64 tests invites false confidence.

Fix direction: one line in `TestStep::run` naming the skipped arch
packages and why ("skipping aarch64: tests execute, need a native host"),
plus a sentence in AGENTS.md's test bullet. Optionally qualify the ci
heading (`test (host: x86_64)`).

Done when: a non-verbose `cargo xtask test` on any host prints which arch
packages were skipped, and AGENTS.md states the host-dependence.

Origin: code review of e022999 (2026-08-20) — "silent narrowing"
CONFIRMED. Same code region as [ci-riscv64-zero-test-signal.md]; consider
landing the two loud-skip changes as one change.

## Status: done (c23cdfc)

`cargo xtask test` prints one line naming the skipped arch packages and
the reason; the ci heading is now `test (host <arch>)`; AGENTS.md's
all-gates line and Test bullet record the host-dependence.
