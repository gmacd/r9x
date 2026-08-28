---
status: done
commit: 86cbdd6
---

# Host test target picked by fragile toolchain-name suffix match

`std_supported_target` (`xtask/src/main.rs:124-127`) chooses the target
for hosted test builds by `curr_toolchain.ends_with(triple)` inside an
order-dependent `find()` over rustup's installed-target list. Two
fragilities:

- Native-host preference works only because rustup lists targets
  alphabetically and `apple` sorts before `unknown`. Nothing asserts
  that ordering.
- Any toolchain whose name does not end in a triple — a linked stage1
  (`RUSTUP_TOOLCHAIN=stage1`), a named custom toolchain — never matches,
  so the code silently falls back to `<arch>-unknown-linux-gnu`. On an
  aarch64 Mac, TestStep then runs
  `cargo test --target aarch64-unknown-linux-gnu` on macOS, failing at
  link or producing binaries the host cannot run.

Fix direction: derive the host triple from `rustc -vV`'s `host:` line
and compare targets against that, instead of parsing the toolchain name.

Done when: target selection is independent of rustup's list order and of
the toolchain's name, and a linked-toolchain `cargo xtask test` picks the
real host triple.

Origin: code review of xtask/CI (2026-08-20, high effort) — CONFIRMED.
Same machinery as [xtask-arch-list-single-source.md] and the loud-skip
tasks; batch together.

## Status: done (86cbdd6)

`RustupState` caches the host triple from `rustc -vV` and
`std_supported_target` prefers it explicitly, falling back to
`<arch>-unknown-linux-gnu` in a second pass — no list-order dependence,
no toolchain-name parsing; the RUSTUP_TOOLCHAIN variable is no longer
read at all. Verified on an aarch64 mac: the test gate picks
aarch64-apple-darwin (the fallback would have produced unrunnable
binaries).
