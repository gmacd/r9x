r9 is a cross-platform Plan9-inspired OS written in Rust.

The architectures supported are: aarch64, x86-64 and riscv64.

On aarch64, r9 targets machines whose generic-timer PPI is GIC-routed —
the BCM2711 (Pi 4) and QEMU's `raspi4b` machine.  The Pi 3 is out of
scope: its BCM2837 routes the timer PPIs through the bcm2836 local
interrupt controller rather than its GIC-400, and r9 says so loudly at
boot rather than trying to guess the wiring (`timer::init`).

The project is designed for multi-core SMP. All concurrency primitives, initialization patterns, and shared data structures must be correct under multi-core execution — never assume single-core.

The project must be free of errors and warnings at all times, including those from check and clippy, and for all supported architectures.  All tests must pass.

## Commands
- All gates at once (fmt, check, clippy for every arch, test for the host
  arch, dist for every arch, integration test):
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
  - runs `port` plus the host's own arch package; a foreign arch package's
    tests only execute on its native host, so they are skipped with a
    printed notice (CI's aarch64-tests job carries the arm half)
- Run a guest (bounded: `--timeout` seconds, default 15, kills the
  guest when it expires):
  - kernel image: `cargo xtask qemu --arch aarch64`
  - one test image: `cargo xtask qemu --arch aarch64 --image user_process`
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

## Knowledge base

`docs/` holds what is *true* about the project and its hardware, indexed in
`docs/README.md`: hardware references, subsystem descriptions, decision
records, and `docs/lessons.md` (gotchas that have already cost debugging
time).

- Grep `docs/` before searching the web or re-deriving hardware behaviour.
- Anything durable you learn goes back in, with its source: spec section,
  `file.rs:line`, or a dated measurement and the command behind it.
- Every page declares `covers:` (the code it describes), `sources:`, and
  `verified:` (commit and date last confirmed). Update `verified` when you
  check a page against the code.
- Status and work in flight belong in `tasks/` (see `tasks/README.md`), not in `docs/`.
