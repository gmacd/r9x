---
status: done
---

# Install every arch's QEMU in CI

`.github/workflows/xtask.yml:29` installs `qemu-system-arm` alone, and the
comment above it explains why: aarch64 was "the only architecture with
tests so far". That stopped being true on this branch — f50bced and 1e7012d
added `[[test]] boot` images to `riscv64/Cargo.toml:27` and
`x86_64/Cargo.toml:32`.

`cargo xtask ci` runs `IntegrationTestStep::for_ci`, which iterates
`Arch::ALL` (main.rs:1054). riscv64 now yields one test name, so the runner
reaches `runner.qemu(&image)?` (main.rs:1106) and
`cmd.spawn().map_err(...)?` (main.rs:1342) returns ENOENT for the absent
`qemu-system-riscv64`. That `?` propagates out of `run()` and fails the
whole job — before x86_64 is attempted, and after aarch64 has already
passed.

The propagation is deliberate and should stay: e80ef40 made a missing host
tool abort the run rather than be recorded as the image failing, because
the tool says nothing about the image and would say the same for every one
of them. The bug is that CI no longer supplies the tools it needs.

Fix: install `qemu-system-misc` (provides `qemu-system-riscv64`) and
`qemu-system-x86` alongside `qemu-system-arm`, and replace the stale
comment with one that does not need editing per arch.

Done when: the `ci` job runs all three arches' boot images, and adding a
fourth arch's image does not silently depend on a package nobody installed.

Origin: code review of the qemu-integration-tests branch (main...HEAD).

## Status: done

- The arch-ci job installs `qemu-system-arm`, `qemu-system-riscv` and
  `qemu-system-x86` — on ubuntu-26.04 the riscv64 emulator moved out of
  `qemu-system-misc` into `qemu-system-riscv`.
- The same job must run on ubuntu-26.04 at all: ubuntu-latest (24.04)
  ships qemu 8.2, which does not provide the `raspi4b` machine the
  aarch64 images boot, so the test dies at machine setup.
- The install step ends by printing all three emulators' versions, so a
  missing one cannot masquerade as a pass.
- Every apt call is timeout-bounded and the .debs are cached across runs
  (key `apt-qemu-2604`): a stalled mirror fails in minutes and warm runs
  need no mirror at all.
