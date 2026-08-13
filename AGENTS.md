r9 is a cross-platform Plan9-inspired OS written in Rust.

The architectures supported are: aarch64, x86-64 and riscv64.

The project is designed for multi-core SMP. All concurrency primitives, initialization patterns, and shared data structures must be correct under multi-core execution — never assume single-core.

The project must be free of errors and warnings at all times, including those from check and clippy, and for all supported architectures.  All tests must pass.

## Commands
- All gates at once (fmt, check, clippy for every arch, test, dist for every
  arch, integration test):
  - `cargo xtask ci` (add `--fix` to reformat in place instead of failing on formatting)
  - needs `qemu-system-<arch>` on the path for the integration tests
- Build:
  - aarch64: `cargo xtask dist --arch aarch64`
  - riscv64: `cargo xtask dist --arch riscv64`
  - x86-64: `cargo xtask dist --arch x86-64`
- Check:
  - `cargo xtask dist`
- Clippy:
  - aarch64: `cargo xtask clippy --arch aarch64`
  - riscv64: `cargo xtask clippy --arch riscv64`
  - x86-64: `cargo xtask clippy --arch x86-64`
- Test:
  - `cargo xtask test`
- Integration test (whole kernel images run under QEMU):
  - all arches: `cargo xtask integration-test`
  - one arch: `cargo xtask integration-test --arch aarch64`
  - each `<arch>/tests/*.rs` is a kernel image with its own `main9`, running
    only the initialisation it needs
  - a new image also needs a `[[test]]` entry in the arch's `Cargo.toml`,
    with `harness = false` and `required-features = ["qemu-test"]`: that
    entry is what both cargo and xtask work from, and a file in `tests/`
    without one is reported rather than run
  - shared helpers go in a subdirectory, `tests/common/mod.rs`, which is
    not a target for either
- Format:
  - `cargo xtask fmt` (`--check` to verify without rewriting)
