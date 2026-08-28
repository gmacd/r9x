---
status: done
commit: e022999
---

# Decide how arch unit tests run on non-native hosts

Current state (e9551a5): `TestStep` runs each arch package natively only.
On the x86_64 CI host, port's 29 tests and x86_64's (0 today) run; the
aarch64 package's 11 unit tests — timer clamping, page tables, register
decoding — run on **no** CI job, and riscv64's will run on none either
when they exist (there is no hosted riscv64 runner).

## Regression note

Before this episode, the aarch64 tests *did* run on the x86_64 CI host:
`cargo test --package aarch64 --lib --target x86_64-unknown-linux-gnu`
compiles and the 11 tests link and execute as ordinary x86_64 binaries.
e9551a5 dropped that coverage. The `--tests` selection that was meant to
replace it (0a0e3ed) is blocked: `--tests` also builds the lib in
non-test mode as the binary's dependency, and that build assembles
irq.rs's DAIF inline asm, which x86_64 cannot. So only the old `--lib`
cross selection covered aarch64 on CI.

Why the test-mode lib assembles at all: in the lib's test crate the only
roots are the libtest harness and `cfg(test)` code, so `init` / `IRQ_OPS`
/ `mask_irqs` are dead, their asm is never emitted, and the host-safe
tests (which lean on the `cfg(test)` mocks in `reg/cnt_el0.rs`) run
natively. In the non-test lib, `pub fn init` is an export root and the
asm must assemble. **The coverage is accidental**: the day a unit test
transitively keeps an arch-asm path alive, the cross build breaks with a
confusing assembler error.

## Options

1. Restore `--lib` for the arch packages (port keeps `--tests`):
   aarch64's tests run on any host again, riscv64/x86_64 run natively as
   today. Zero kernel change; the binary test targets stay unselected
   (boot sequence only, no tests today). Keeps the accidental-coverage
   fragility.
2. Make the non-test lib host-compilable: gate irq.rs's DAIF asm (and
   audit the rest) for `target_os = "none"`, with a `cfg(test)` fallback
   in the existing `cnt_el0.rs` mock style. Then `--tests` works cross
   and covers lib, binary, and any future integration tests. Small
   kernel change touching interrupt-context code.
3. Native runners per arch, in parallel. Findings (checked mid-2026):
   - aarch64: `ubuntu-24.04-arm` is hosted/GA/free for public repos, but
     it does **not** expose `/dev/kvm`, so QEMU there is TCG — there is no
     acceleration side benefit to moving the aarch64 `arch-ci` job there.
     It is also 24.04, whose apt QEMU is 8.2.2 (no `raspi4b`), so a job
     that runs integration tests there would need a hand-installed
     QEMU >= 10.2.1.
   - riscv64: no hosted label. Free third-party managed services exist
     (RISE RISC-V Runners GitHub App, free for OSS, GA; riscvrunners.com),
     but they are external dependencies on the repo, and riscv64 has zero
     tests today. Revisit at first riscv64 test.
   - Shape: keep `arch-ci` entirely on ubuntu-26.04 as it is; add one
     small job running only `cargo xtask test` on `ubuntu-24.04-arm`
     (port + aarch64 natively, no QEMU). Restores the coverage honestly,
     no kernel change; cost is one parallel job plus aarch64-pool queue
     latency on every push.

Done when: aarch64's unit tests run in CI at least as often as before
this episode, and the chosen option is recorded here with its rationale.

Origin: fallout of xtask-test-targets-stale (done/) — investigating the
0a0e3ed CI failure showed the old `--lib` cross selection had been
carrying aarch64 test coverage silently, and e9551a5 dropped it.

## Status: done (e022999, option 3 in its narrowed shape)

Added an `aarch64-tests` job to `.github/workflows/xtask.yml` running
only `cargo xtask test` on `ubuntu-24.04-arm`, in parallel with
everything else; `arch-ci` stayed on ubuntu-26.04 (no KVM exists on
hosted ARM runners, and 24.04's apt QEMU lacks `raspi4b`, so moving
integration tests there would have cost more than it bought).

Verified in run 32367405192: on the arm runner port ran 29 tests,
aarch64 ran its 11 lib tests, and — the line this task set out to make
honest — aarch64's binary test executable built, linked and ran
natively (0 tests today, selection proven). The rustup cache key is
per-arch (`runner.arch`), so the x86_64 caches are undisturbed.

Deliberately not done: riscv64 native execution. No hosted riscv64
runner exists; the free third-party services (RISE RISC-V Runners app,
riscvrunners.com) are external dependencies not worth adding while
riscv64 has zero tests. Revisit at riscv64's first test — at which
point option 2 (host-compilable lib via `cfg(test)` asm fallbacks)
also becomes a live alternative worth re-costing.
