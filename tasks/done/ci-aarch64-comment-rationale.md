---
status: done
commit: 294b845
---

# Fix the aarch64-tests job comment's false rationale

The comment on the `aarch64-tests` job (`.github/workflows/xtask.yml:39-42`,
added in e022999, echoed in that commit's message) says "the aarch64 lib
assembles inline asm the x86_64 host in checks cannot, so those 11 tests
need an aarch64 host." The stated reason is false:

- `checks` *does* compile the aarch64 test code from an x86_64 host:
  `CheckStep` emits `cargo check --package aarch64 --tests --benches
  --target aarch64-unknown-linux-gnu` (`xtask/src/main.rs:921-941`), and
  `rust-toolchain.toml:8-11` installs that target for exactly this purpose
  ("For checking aarch64's tests and benches ... on a host that is not
  aarch64"). Clippy does the same cross-host. The host is not the limiting
  factor for compilation; the real limit is that test binaries must
  *execute*, which is what needs a native host — stated correctly by the
  `TestStep` comment at `xtask/src/main.rs:743-749`.
- "those 11 tests" is a count nothing asserts; the first added or removed
  `#[test]` under `aarch64/src` silently falsifies it, and the digit
  carries no information the reader acts on.

Fix direction: reword the comment to the TestStep rationale (tests execute,
so the arch package only runs on a native host; cross-host coverage is
check's and clippy's job) and drop the count.

Done when: the workflow comment gives execution, not assembly, as the
reason; contains no test count; and no longer contradicts
`xtask/src/main.rs:743-749`.

Origin: code review of e022999 (2026-08-20) — "comment rationale"
CONFIRMED and "stale count" PLAUSIBLE; both live in the same four lines,
so they land as one change.

## Status: done (294b845)

The comment now gives the execution rationale (TestStep runs arch
packages natively because tests execute; checks cross-compiles them but
cannot run them) and the test count is gone.
