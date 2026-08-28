---
status: done
commit: 4a74cda
---

# config.rs panics on a [link] table missing `script`

`xtask/src/config.rs:192` indexes the config HashMap directly with
`link["script"]`. A `config_<name>.toml` whose `[link]` table lacks a
`script` key (e.g. only `arch = 'riscv'`) makes every build / dist /
qemu / integration-test step panic on the index with a raw backtrace
instead of a readable error naming the file and the missing key.

Fix direction: `link.get("script")`, reporting the config path and key
through the crate's Result type.

Done when: a config file with `[link]` but no `script` produces a
one-line diagnostic naming file and key, no panic.

Origin: code review of xtask/CI (2026-08-20, high effort) — CONFIRMED.
Same file and theme as [xtask-kernel-ld-write-unchecked.md]; land
together.

## Status: done (4a74cda)

`apply_link` uses `link.get("script")` and returns "config [link] table
has no 'script' key" through the crate's Result; `apply_to_build_step`
and both call sites propagate. Verified with a scratch config missing
the key: one-line error, exit 1, no backtrace.
