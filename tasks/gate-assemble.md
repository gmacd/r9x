---
status: open
---

# gate-assemble: assemble every .S in the local loop

Task 4 of 7 in the gates-hardening arc. Plan:
[plans/gates-hardening.md](plans/gates-hardening.md).

## Goal

`check`/`clippy` stop at metadata and never assemble the
`global_asm!` strings; `dist` and the integration images do, but
that is minutes into the loop. A broken `.S` should fail the first
step of `ci`, naming the file.

Deliberately demoted: this is fast-fail ordering, **not new
coverage** — `dist` already catches every assembly error in CI.

**Demoted further (2026-08-27 audit): fold into task 46's landing or
drop.** Two erosions since the spec: (1) `aarch64/build.rs` now
panics when server ELFs aren't staged (build.rs:56-61), so this step
must run `ServerStep` first (as ClippyStep does) — a full user-space
build, which eats the speed win on aarch64; (2) the local-loop win
only exists with a warm cache (a cold `-Zbuild-std` build of
core+alloc dominates). Implementation cost stays genuinely low
(xtask's `KasmStep` is nearly the same step with `--emit asm`), which
is the argument for folding rather than deleting.

## Changes

New xtask step inside `ci`, before `check`: per arch package,
`cargo build --package <arch> --emit=obj` with the **same rustflags
as the clippy invocation** (the config rustflags, so the step
assembles the same code clippy lints); fail on any error and
annotate the diagnostic with the `.S` file (rustc reports
`<inline asm>:LINE` plus the `global_asm!` call site; the wrapper
maps it back to the file). No shim crate, no second assembler — the
production pipeline is the gate, and the frame-offsets task's
build.rs prelude comes along for free (it is a build).

Stated limits (in the code): object emit only — cross-file
references are relocations and must not fail (no link);
`global_asm!` scopes concatenate across modules in one build, so a
macro defined in one file and used in another is *valid in
production* but not per-file — per-file self-containment stays the
discipline (currently true for all five files), and the step does
not claim production-identical behaviour for that one case.

Rejected (recorded in the plan): clang-based checking (a second,
unpinned assembler with different brace semantics — the gate could
pass what production rejects); check-asm (a third-party binary to
pin and download for coverage this asm does not need).

## Acceptance

- A bad register or bad immediate in any `.S` fails `cargo xtask
  ci` before `check` runs, naming the file.
- Full `cargo xtask ci` green.

## Not in scope

Semantic checking of the asm (register-clobber discipline has no
honest static checker; the integration images are that gate).
