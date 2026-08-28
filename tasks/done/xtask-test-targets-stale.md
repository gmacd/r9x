---
status: done
---

# Test the riscv64 and x86_64 libraries

`TestStep::run` (xtask/src/main.rs:763) picks cargo's target selection per
package and falls through to `--bins` for riscv64 and x86_64. The comment
above it (main.rs:760-761) gives the reason: "riscv64 and x86_64 have no
library at all, so `--lib` is not a narrower selection there but an error."

f50bced and 1e7012d gave both of them one. `riscv64/src/lib.rs` and
`x86_64/src/lib.rs` exist on this branch, so the `_ => "--bins"` arm now
means any `#[cfg(test)]` module added to either library is never built and
never run by `cargo xtask test` — reported as a pass, because nothing
selected it. That is the same silent-skip failure mode `undeclared_images`
exists elsewhere in this file to prevent.

aarch64 is `--lib` for a reason worth preserving: its binary cannot be
built for a host because the boot assembly is only assembled for the bare
metal target. Whether riscv64 and x86_64 binaries have the same problem
needs checking rather than assuming — if they do, they want `--lib`; if not,
`--tests` covers both libraries and integration tests.

Fix: check what each of the two can actually build for the host, select
accordingly, and rewrite the comment to state the current shape rather than
the one that held when it was written.

Done when: a unit test added to `riscv64/src/lib.rs` or `x86_64/src/lib.rs`
runs under `cargo xtask test`.

Origin: code review of the qemu-integration-tests branch (main...HEAD).

## Status: done

- `TestStep` now passes `--tests` for every package it runs, replacing
  the per-package match (and its dead `--bins` arm): each package's lib
  unit tests, its binary's, and any integration tests are selected, with
  the QEMU kernel images excluded by their `required-features`.
- The comment's claim that the arch binaries cannot be built for a host
  was checked and found false: every `main.rs` turns `no_main` off under
  `cfg(test)`, the aarch64 assembly is gated on `target_os = "none"`, and
  x86_64's `l.S` is position independent and defines `start`, not
  `_start`.
- Arch packages now run natively only.  Tests execute, so a package
  whose inline asm assembles for one architecture alone (aarch64's irq
  masking) cannot run for a foreign target; the first attempt at
  host-target-for-all ran the aarch64 package for x86_64 on CI and
  failed to assemble, so arch selection is now "run where the arch is
  the host's", with foreign-host coverage left to check and clippy,
  which build for `<arch>-unknown-linux-gnu` (why rust-toolchain.toml
  names one for aarch64).
- Verified in CI: on the x86_64 runner the x86_64 lib and bin test
  executables build, link and run natively (0 tests today, selection
  proven); port's 29 tests run on every host.
