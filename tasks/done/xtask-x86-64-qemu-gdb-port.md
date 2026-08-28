---
status: done
---

# x86-64 qemu always binds the gdb port

The x86-64 qemu step passes `-s` (gdb server on tcp:1234) unconditionally
(`xtask/src/main.rs:616`), then adds `-s -S` again under `--gdb` — so a
plain `cargo xtask qemu --arch x86-64` binds tcp:1234, and `--gdb` passes
`-s` twice. aarch64 and riscv64 only pass `-s` under `--gdb`.

Consequence: a plain x86-64 run fails with address-in-use whenever
another QEMU (or a stale one) holds tcp:1234, even though gdb was never
requested.

Fix direction: move the unconditional `-s` at `:616` under the
`wait_for_gdb` branch, matching the other two arches.

Done when: plain x86-64 qemu runs bind no gdb port and `--gdb` passes
`-s -S` exactly once, consistent across all three arches.

Origin: code review of xtask/CI (2026-08-20, high effort) — CONFIRMED.

## Status: done (r9x2 working tree, 2026-08-20)

Deleted the unconditional `-s` from the x86-64 QemuStep; the
`wait_for_gdb` branch already passes `-s -S`, so plain runs bind no gdb
port and `--gdb` passes `-s` exactly once, matching aarch64 and riscv64.
Verified: clippy clean, `cargo xtask integration-test --arch x86-64`
passes.
