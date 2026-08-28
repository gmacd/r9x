# Gates hardening: lints and checkers the project could be running

Status: build-ready after the 2026-08-27 premise refresh (two review
rounds, then an audit re-verified every load-bearing claim against the
tree at fd7e96c — the pin, build.rs, and unsafe-census claims below had
gone stale and are corrected in place; task 1's mechanism was rewritten
in its file around linker-script ASSERT, and tasks 4/5 carry demotion/
caveat notes in theirs).
Arc: standalone — each task lands independently. Landing order revised
by the audit: **2 → 6 → 3 → 1 → 7 → 5 (with mcslock-loom-tests, task
97); 4 folds into 2's landing or drops.**

## Context

r9 is a multi-arch (aarch64, x86-64, riscv64) no_std OS in Rust with
hand-written `.S` per arch, a pinned toolchain (nightly-2026-08-21 —
the pin moved once already since this plan's first draft, which is
task 7's argument made for it), everything driven by `cargo xtask`, CI
on GitHub Actions (x86-64 host job + aarch64 native job) with QEMU
system emulators on the path. The workspace has grown since the first
draft: `abi`, `core`, `std`, and the `cmd/*` servers are members too
(task 3's scope now includes them).

### Current coverage (verified in xtask source)

- `rustfmt` over the workspace; `cargo check` and `cargo clippy`
  (`-Dwarnings`) per arch package, port's tests/benches, and each
  qemu-test image individually with `--features qemu-test` (files in
  `tests/` without a `[[test]]` entry are reported, not skipped).
- Workspace lints table: `unsafe_op_in_unsafe_fn = forbid`,
  `unexpected_cfgs = deny` (check-cfg for the injected `platform`
  cfg), `non_ascii_idents = deny`; clippy denies on the pointer/cast
  hygiene set. `clippy::missing_safety_doc` is already enforced
  (warn-by-default + the gate's `-Dwarnings`) — the tree's `# Safety`
  doc sections are the proof.
- Host tests, `dist` per arch, QEMU integration images that assert
  their own stdout.

### Verified facts this plan builds on

- The `.S` files enter the build via `global_asm!(include_str!(...))`
  (aarch64: lib.rs/trap.rs/swtch.rs; x86-64 and riscv64: lib.rs, with
  `options(att_syntax)` on x86-64). rustc's integrated LLVM assembler
  assembles at codegen. `check`/`clippy` stop at metadata and never
  assemble; `dist` and the images do. ~~No build.rs anywhere~~ —
  **stale**: `aarch64/build.rs` exists now (server-ELF staging for the
  embedded images) and *panics* when server ELFs aren't staged
  (build.rs:56-61). Task 2's offsets writer merges into it; task 4's
  speed claim is eroded by it (the step needs ServerStep first).
- `global_asm!` treats `{...}` as template operands (literals need
  `{{`) and takes **no operands** — a const after the string is a
  parse error (tested on the pin). It **does** accept multiple
  template strings, concatenated (tested: a `.equ` in the first
  string is visible in the second, and a bad immediate fails with
  `index must be an integer in range [-256, 255]`).
- llvm-tools (already a toolchain component) ships llvm-nm,
  llvm-readobj, llvm-size, llvm-objdump; not llvm-mc.
- `aarch64/lib/kernel.ld` has PROVIDE symbols and **no KEEP()**; the
  vector table sits in plain `.text` (`.balign 2048`), so there is
  currently nothing a targeted KEEP can grab.
- None of the `.S` files emit `.size`/`.type` directives, so
  asm-defined symbols have `st_size = 0` in the ELF; `.equ` constants
  are absolute symbols whose *value* is the data.
- `KSTACKS` is a private mangled static (no `#[no_mangle]`) — task
  1's matcher must strip the `::h<hash>` suffix or it churns.
- Unsafe census (refreshed 2026-08-27): ~409 grep-hits, ~46
  `// SAFETY:` comments (was 238/6 — the practice took hold without
  the gate). Source-level lint attributes override command-line
  levels (so a per-module `deny` works under a gate-level `allow`);
  note `undocumented_unsafe_blocks` is allow-by-default, so task 3's
  `-A` flag is a no-op (see its file).
- `rust-toolchain.toml` lacks `miri`; the component exists for the
  current pin (re-verified 2026-08-27). Task 5's caveat: mcslock.rs
  and allocator.rs have zero `#[test]`s today, so its headline
  coverage claim waits on task 97's tests.
- `port/src` has zero `asm!`.

### Motivating incidents (and which check kills each)

1. KSTACKS was 4x the intended size — no gate noticed → **task 1**.
2. `interruptstackbase` silently GC'd by `--gc-sections` → **task 1**.
3. A SPSR store missing from a `.macro` (valid asm, wrong layout) →
   **task 2** (the only incident class turned into an impossibility).
4. A live `unsafe` call glued onto a comment line (compiled
   warning-free) → already covered by the `two_yield` regression
   test; nothing new.

## Tasks (in landing order)

### 1. Symbol manifest + post-link structural assertions

**Mechanism rewritten 2026-08-27 in the task file
([gate-symbol-manifest.md](../gate-symbol-manifest.md)): the
structural assertions move into kernel.ld `ASSERT()`s (the Linux
`vmlinux.lds.S:400-434` mechanism — eliminators in the production
link, no demangling, no expression-evaluation of the load-address
config string), and the nm manifest shrinks to `st_size` checks only.
The section below is the original spec, kept for the parts the rewrite
retains (KEEP ordering, admission rule, the size table).**

Per-arch committed manifest at **`<arch>/lib/`** (next to kernel.ld —
xtask already treats `lib/` as the per-arch config home, and the diff
that changes a constant and its manifest line lands in one
directory), run by xtask after `dist` on the **ELF** artifact
(`target/<triple>/<profile>/<arch>` — the flat binary drops NOBITS,
so `.bss` is unmeasurable there).

Each manifest entry has a **check kind**, because one size column
cannot express the three real cases:

- `value` — absolute `.equ` symbols (`INTERRUPTSTACKSZ`, the kstack
  size): assert the symbol's value;
- `size` — sized symbols: assert `st_size` — **requires adding
  `.size sym, .-sym` and `.type` directives to the `.S` files in the
  same task** (none exist today, so the check could never fire as
  specced otherwise);
- `alignment` — e.g. `exception_vectors ≡ 0 mod 2048`: within the
  section today via `.balign`, but nothing checks the *linker* kept
  the section aligned after a `kernel.ld` edit.

Admission rule (written in the manifest header): only
deliberate-constant symbols are listed — stack sizes, the vector
table, the boot pagetable reservation. Evolving structs get
presence+section at most, or stay out. Exact-match on a listed symbol
is the review signal (KSTACKS-4x is exactly a size change someone
should have stared at); that is what prevents
reflexive-baseline-update disease. Match **demangled** names
(`llvm-nm --demangle`) — do not add `no_mangle` to please a checker.

Structural assertions per arch, in the same step: entry point equals
the expected load address **read from the same per-arch config
xtask's config.rs templates into kernel.ld** (not hardcoded);
x86-64: multiboot header in the first 8 KiB; `.bss` zeroing scoped
honestly — under QEMU the RAM is zeroed before us, so only the
bounds are checkable (the bss start/end PROVIDE symbols bracket the
table symbols); the zeroing loop itself is exercised only on real
hardware, and the task says so.

The always-print `llvm-size` per-section table is the standing bloat
report (no threshold).

**Same task, ordered**: move the vector table to a named section
(`.text.vectors`) — it sits in plain `.text` today, and
`KEEP(*(.text*))` would disable section GC for all code, which is
worse than no KEEP — then add `KEEP(*(.text.vectors))` (and the boot
sections worth keeping) to kernel.ld. KEEP cannot reorder retained
sections but can resurrect previously-GC'd ones, so the order is:
named section → KEEP → build → **then** baseline the manifest
(baselining first flags immediately).

Acceptance: deleting a listed symbol or changing a listed size fails
`ci` naming the symbol and the manifest line; a `kernel.ld` edit that
misaligns the vectors fails the alignment assertion; the size table
appears in every CI log.

### 2. Frame-offset single-sourcing (aarch64)

The layout is triple-maintained (trap.S literals, the
`FRAME_*`/`CONTEXT_SZ` consts, the structs); the existing host pins
cover Rust↔Rust only — the asm leg, where the SPSR-store bug lived,
is checked by comment. Const operands are impossible (verified), but
the multiple-template-strings path works (verified end to end), so:

- the consts move to a plain **`aarch64/src/frame_offsets.rs`**
  (consts only), `include!`-ed by process.rs;
- **aarch64 gets its first build.rs** (the only one in the tree):
  it `include!`s the same file and writes an `offsets.s` prelude of
  `.equ` lines to OUT_DIR;
- the `global_asm!` calls take the prelude first:
  `global_asm!(include_str!(concat!(env!("OUT_DIR"), "/offsets.s")),
  include_str!("trap.S"))` (same for swtch.S — its hardcoded 112 is
  the same disease), and trap.S/swtch.S reference the offsets as
  symbols (`str x3, [sp, #FRAME_SPSR]`) instead of literals, where
  the layout is non-structural (the staging loads/stores, the frame
  size, the context size; the save/restore stp pairs stay index-based
  — the slot *is* the register index).

This is an **eliminator, not a detector**: a mismatch fails at
assembly time in the production pipeline itself. The existing host
pins (consts↔structs) stay and now complete the circle. The
x86-64/riscv64 equivalents come when their entry paths get a
comparable frame (their `l.S` is boot-only).

Acceptance: changing `FRAME_SPSR` and nothing else fails the build at
assembly naming the slot; changing a struct field order without the
consts fails the host pins.

### 3. `unsafe` ratchet (three buckets)

Gate-level: add `-A clippy::undocumented_unsafe_blocks` to the clippy
invocation (source-level denies override it, so this is the soft
landing that makes per-module ratcheting possible).

Per module, audited top-down, each exits as one of:

1. **`#![deny(unsafe_code)]`** (xtask: `forbid` — verified zero
   unsafe, and nothing there should ever want an override) — the
   module is clean and stays clean;
2. **`#![deny(clippy::undocumented_unsafe_blocks)]`** with its
   handful of `// SAFETY:` comments written in the same change —
   modules with a few intentional unsafe sites;
3. **the recorded remainder** — modules whose unsafe surface is
   structural (vm.rs, the allocator, the fdt parsing): listed in the
   task's resolution, no change.

No big bang: the count only ratchets down, new modules start denied,
and a rushed-tree campaign is avoided (rushed SAFETY comments are
worse than none). Honest sizing: bucket 1's list is short (port's
fdt.rs ≈10 unsafe blocks, allocator.rs ≈17 — both bucket 3).

Acceptance: `cargo xtask ci` green; introducing `unsafe` into a
bucket-1 module or an undocumented block in a bucket-2 module fails
the build; the resolution lists every audited module and its bucket.

### 4. Assemble gate (fast-fail; demoted on purpose)

`dist` and the images already catch every assembly error in CI; the
value here is the local loop: `check`/`clippy` never assemble
`global_asm!` strings, so a broken `.S` is discovered minutes later
at the first build. New xtask step inside `ci` before `check`:
`cargo build --package <arch> --emit=obj` with the **same rustflags
as the clippy invocation** (the config rustflags — so the step
assembles the same code clippy lints), failing on any error and
annotating the `global_asm!` call site with the `.S` file (rustc
reports `<inline asm>:LINE`; the wrapper maps it). No shim crate, no
second assembler: the production pipeline is the gate. After task 2
the build.rs prelude comes along for free (it is a build).

Stated limits: cross-file references are relocations and must not
fail (object emit, no link); `global_asm!` scopes concatenate across
modules in one build, so a macro defined in one file and used in
another is *valid in production* but not per-file — per-file
self-containment stays the discipline (currently true for all five
files, and this step keeps it that way without saying it is
production-identical for that one case).

Rejected: clang-based checking (a second, unpinned assembler with
different brace semantics — the gate could pass what production
rejects); check-asm (a third-party binary to pin and download for
coverage this asm does not need).

Acceptance: a bad register in any `.S` fails `cargo xtask ci` before
`check` runs, naming the file.

### 5. Miri on the host-side tests

Add `miri` to `rust-toolchain.toml` components (verified available
for the pin) and run `cargo miri test -p port` as a CI step. port has
zero `asm!` (verified) — the MCS lock, atomics, and allocator
`unsafe` are Miri's sweet spot, and they are the host path QEMU never
sees (the host and bare-metal paths of port deliberately diverge;
that is exactly where a provenance/ordering bug is hardest to
attribute). Guardrail: if a lock test spins under the interpreter and
a timeout appears, bound the test rather than letting the CI timeout
be the discovery mechanism. `--workspace` extension is a later
one-flag change, not now.

Acceptance: `cargo miri test -p port` green in CI on the pinned
nightly; a deliberately aliasing host test fails under Miri.

### 6. typos

`typos` (crate-ci/typos) as a CI step with a committed `typos.toml`
ignoring the register names (elr, spsr, far, esr, daif...) and the
hex-ish identifiers in the `.S` files. SAFETY and rationale comments
carry correctness; a mangled word there has cost. ~1s.

### 7. Drift watch (nightly + QEMU)

- **Nightly-drift cron**: weekly workflow — install current
  `nightly`, override, `cargo xtask ci`. The build leans on
  `-Zbuild-std`, `-Zjson-target-spec`, and unstable lint behavior; a
  pin bump is where this project breaks. **No `continue-on-error`**
  (a scheduled workflow gates nothing, so a red run costs nothing
  and hiding it makes the job feel like coverage while being none):
  the job goes red, and a failure step updates a pinned
  "nightly drift" issue — that push channel is what earns the keep.
  Without the push channel the honest alternative is cutting the job
  and bumping the pin on breakage.
- **QEMU version to `$GITHUB_STEP_SUMMARY`**: the workflow already
  documents the raspi4b/QEMU image coupling and prints
  `qemu --version`; redirect those lines into the step summary so a
  behavior change is attributable to an image bump. No version
  pinning (the apt-pinning machinery is not worth it).

## Note (no task)

`CheckStep` runs without the config rustflags while `ClippyStep`
applies `config_default.toml` — both cfg states compile *by
accident*. Only `config_default.toml` exists per arch today, so there
is nothing to gate; the moment a second config lands (the `nezha`
value in check-cfg suggests one is expected), `ci` needs a config
loop or that config rots unbuilt.

## Gaps found by the 2026-08-27 audit (now tracked)

- **Linker-script ASSERT** — folded into task 1's rewrite (the
  biggest free win the original plan missed).
- **No concurrency gate at all** for a project whose charter mandates
  SMP correctness while `mcslock.rs` has zero tests → task 97
  (`mcslock-loom-tests.md`): host tests + loom (or miri many-seeds);
  task 5 lands with it.
- **`cargo deny check advisories`** — rides task 7's weekly cron for
  free (see its file).
- **Stack-depth reporting** — `-Zemit-stack-sizes` max-frame report
  next to task 1's llvm-size table (report, not gate; Linux analogue
  FRAME_WARN/checkstack.pl). Optional rider recorded in task 1.
- Cast-hygiene lint additions: already substantially covered by the
  workspace clippy set (at or beyond Rust-for-Linux parity); optional
  polish, no task.

## Cut (with reason)

- **`cargo llvm-cov` artifact** — an artifact nobody is forced to
  read is a check nobody runs.
- **`clippy::missing_safety_doc`** — already enforced today
  (warn-by-default + `-Dwarnings`).
- **`clippy::undocumented_unsafe_blocks` as a tree-wide flip** —
  238 sites against 6 `// SAFETY:` comments; the three-bucket
  ratchet (task 3) is the same walk with a soft landing instead of a
  big bang.
- **Register-clobber static checking** — no honest static checker
  exists; the integration images (two_process, two_yield,
  user_yield) are the real gate and already exist.
- **Coverage gate** — maintenance tax without matching payoff.
- **clang/check-asm assemble checking** — see task 4.

## Out of scope

Real-hardware gates (the `.bss` zeroing loop, the Pi 3 local intc),
per-config CI loops (note above), and anything on the x86-64/riscv64
entry paths beyond the shared manifest/structural assertions.
