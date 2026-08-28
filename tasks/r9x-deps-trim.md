---
status: open
---

# r9x-deps-trim: trim the external dependency set (clean-build + audit surface)

A dependency-hygiene task, independent of the r9x arc (it touches the kernel
arch crates, not the user-space target). Parked 2026-08-25: the audit is done
and the cuts are scoped here, but it is not yet scheduled.

Rationale: a clean build pulls **62 external crates** (73 packages total, 11
workspace). Most are host-only, but ~20 compile for the kernel targets, and
the proc-macro machinery is the hidden cost. The goal is to cut the
proc-macro *sugar* crates the register code reaches for, without touching the
load-bearing register definitions — fewer crates to compile per target, fewer
to audit, and no behavior change.

## The audit (2026-08-25, `cargo metadata` + `cargo tree`)

**73 packages: 11 workspace + 62 external.** The 62 split by where they
compile:

- **Host-only (~28, via `xtask`)** — never build for the kernel targets:
  `clap`(+`clap_builder`,`clap_derive`,`clap_lex`) and its terminal-styling
  tree (`anstream`,`anstyle`,`anstyle-parse`,`anstyle-query`,`colorchoice`,
  `is_terminal_polyfill`,`utf8parse`); `serde`(+`serde_core`,`serde_derive`);
  `toml`(+`toml_datetime`,`toml_parser`,`toml_writer`,`winnow`,
  `serde_spanned`); `target-lexicon`(+`indexmap`,`hashbrown`,`equivalent`).
- **Target-compiled (~20, the ones that slow the OS build):**
  - **Keep (load-bearing register definitions):** `aarch64-cpu` +
    `tock-registers` (aarch64: `CNTPCT_EL0`/timer, `MIDR_EL1`, GIC, barriers),
    `sbi-rt` + `sbi-spec` (riscv64), `x86` + `raw-cpuid` (x86_64).
  - **Keep (tiny, no proc-macro):** `bitflags`, `bit_field`, `seq-macro`,
    `static_assertions`.
  - **Cut candidates (proc-macro sugar, replaceable by hand):** `bitstruct`
    (+`bitstruct_derive`) — **scoping corrected 2026-08-27: 15
    `bitstruct!` invocations across 7 files** (gic.rs ×6, vm.rs,
    reg/cnt_el0.rs, reg/esr_el1.rs ×2, reg/midr_el1.rs, x86_64
    vsvm.rs ×3, dat.rs — not "22 lines"; change 3 is meaningfully
    larger than the original scope); `num_enum`
    (+`num_enum_derive`) — a few aarch64 enums; `zerocopy`
    (+`zerocopy-derive`) — `FromZeros` in two x86_64 files.
  - **Hidden cost:** the proc-macro machinery `syn` (**×2**: 1.0.109 +
    2.0.106), `quote`, `proc-macro2`, `unicode-ident`, `rustversion`, `heck`
    — driven by the `_derive` crates above; `syn` compiles twice per target.

Direct externals per crate (the "who pulls in what"):
`aarch64` → aarch64-cpu, bitstruct, num_enum · `riscv64` → sbi-rt ·
`x86_64` → bit_field, bitstruct, seq-macro, static_assertions, x86, zerocopy ·
`port` → bitflags · `xtask` → clap, serde, target-lexicon, toml ·
`r9x-abi`/`r9x-core`/`r9x-std`/the `cmd/` servers → none.

Note: the **dominant** clean-build cost is `build-std=core,alloc` × 3 targets
(core+alloc from the toolchain's `rust-src`, per arch) — inherent to the custom
`no_std` target, not addressable here. This task trims the *external* set on
top of that.

## Changes

One change per arch crate, each independent and revertible; no public API
change, no behavior change.

1. **`zerocopy` → manual (x86_64).** `FromZeros` (a "zero-init is safe"
   derive) in `vsvm.rs` + `dat.rs` becomes a hand-written zeroed initializer
   (a `const`/`[0u8; N]` seed + field reads). Removes `zerocopy` +
   `zerocopy-derive`.
2. **`num_enum` → hand-written (aarch64).** The `#[num_enum(default)]` enums
   in `vm.rs` and the `TryFromPrimitive` uses in `midr_el1.rs`/`esr_el1.rs`
   become small `fn from(u32) -> Option<Self>` / `fn into(self) -> u32`
   impls. Removes `num_enum` + `num_enum_derive`.
3. **`bitstruct` → replaced (aarch64 + x86_64, 15 invocations / 7
   files).** Preferred target (2026-08-27 audit): **`tock-registers`**
   — already a kept, load-bearing dep via `aarch64-cpu`, and its
   `register_bitfields!` is `macro_rules`, not proc-macro — migrating
   the GIC/ESR/MIDR/CNT blocks there keeps declarative field
   definitions while still killing `bitstruct`, both `_derive` crates,
   and `syn 1.0`. Raw shift/mask getters remain the fallback for the
   x86_64 blocks if tock-registers fits badly there. Removes
   `bitstruct` + `bitstruct_derive`, **and very likely the
   `syn 1.0.109` version skew** (verify with `cargo tree` after).
   None of the three replacements needs new `unsafe` (zerocopy here is
   zero-*init* only, not `FromBytes` reinterpretation — if that ever
   changes, zerocopy stays).

Do these bottom-up (zerocopy first — the smallest), one commit each, `ci`
green after each. Re-check `cargo tree` per arch to confirm the crate (and,
for bitstruct, the duplicate `syn`) is gone from the target closure.

## Acceptance

- `cargo tree -p <arch> --target lib/<arch>-unknown-none-elf.json -Z
  json-target-spec` no longer lists `zerocopy`/`num_enum`/`bitstruct` (and
  `syn 1.0.109`) for the affected arches.
- `cargo xtask ci` green (all arches, warning-free) after each of the three
  changes — the register accessors are covered by the existing integration
  images (the timer/GIC/ID paths that read those registers).
- The external target-compiled count drops from ~20 to ~13; the duplicate
  `syn` is gone.

## Optional (host-only, separate from the above)

If the *count* (not kernel build time) is the concern: slim `xtask` — drop
`serde`+`toml`+`target-lexicon` (hardcode the three server names and spec
filenames instead of parsing `Cargo.toml` / the triples) and swap `clap` for a
lighter parser. Cuts ~15–20 host crates but only speeds the one-time `xtask`
build. File this as a separate task only if xtask build time is felt.

## Not in scope

The register-definition crates (`aarch64-cpu`, `tock-registers`, `sbi-rt`,
`sbi-spec`, `x86`, `raw-cpuid`) — they *are* the register definitions;
hand-writing AArch64/x86/SBI registers is the wrong trade. The `bitflags`
`1.3.2`-vs-`2.9.4` skew (controlled by the `x86` crate's own dep — revisit
only if `x86` moves to `bitflags 2`). `build-std` (inherent to the target).
Any change to register *behavior* — this is a pure dependency swap.
