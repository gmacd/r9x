---
status: done
commit: 17b881d
---

# CI caches never seed while a job is red

Both cache layers in `.github/workflows/xtask.yml` save only on success:
`actions/cache@v4` declares `post-if: success()` (and v4 removed
`save-always`), and `Swatinem/rust-cache` defaults
`cache-on-failure: false`. The workflow's
`concurrency.cancel-in-progress: true` (`xtask.yml:12-14`) amplifies
this: cancelled runs also skip the save. So while iterating on a red job
— exactly when pushes are frequent — every run pays the full cold cost
(rust-toolchain.toml pulls four targets plus rust-analyzer/rust-src/
llvm-tools, a few hundred MB) and seeds nothing; the penalty self-resolves
only at the first green run.

This is pre-existing behavior shared by `checks` and `arch-ci`, not a
defect of e022999; the new per-arch ARM rustup key just starts life cold.
CI-latency nit, not correctness.

Fix direction: `cache-on-failure: true` on the `rust-cache` step;
optionally split the rustup cache into `actions/cache/restore` +
`actions/cache/save` with `if: always()` (the sanctioned v4 workaround)
if red-run toolchain downloads prove annoying in practice.

Done when: a failed run still saves the toolchain and build caches, or a
deliberate decision to keep save-on-success is recorded here.

Origin: code review of e022999 (2026-08-20) — "cache seeding" PLAUSIBLE,
low severity. Land with [ci-cache-steps-triplicated.md] so the flag is
set once in the extracted composite action.

## Status: done (17b881d)

`cache-on-failure: true` on the rust-cache step, set once in the
extracted composite action. The rustup layer deliberately keeps
save-on-success: a toolchain download is cheap next to a cold build, so
the restore/save split stays unimplemented until it proves annoying (as
this task allowed).
