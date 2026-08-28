---
status: open
---

# gate-unsafe-ratchet: deny-by-default, module by module

Task 3 of 7 in the gates-hardening arc. Plan:
[plans/gates-hardening.md](plans/gates-hardening.md).

## Goal

Ratchet the tree toward `unsafe`-free modules without a big bang:
every audited module exits as one of three buckets, the count only
goes down, and new modules start denied.

## Changes

- Nit (2026-08-27 audit): `clippy::undocumented_unsafe_blocks` is in
  the `restriction` group — **allow by default** — so a gate-level
  `-A` flag is a no-op; the soft landing is the lint's own default.
  Keep the flag only as documentation-of-intent, or drop it. The
  per-module `#![deny(...)]` mechanism is unaffected.
- Audit every module in **port, the three arch packages, and the
  crates the original scope predates: `abi`, `core`, `std`, and the
  `cmd/*` servers** (scope extended 2026-08-27 — `std/src/rt.rs`,
  `sys.rs`, `mem.rs` are real new unsafe surface: syscall stubs and
  allocator glue; silently auditing half the tree defeats "new
  modules start denied"). Top-down; each exits as:
  1. **`#![deny(unsafe_code)]`** — the module is clean today and
     stays clean (xtask itself: `forbid`, verified zero unsafe, and
     nothing there should ever want an override);
  2. **`#![deny(clippy::undocumented_unsafe_blocks)]`** with its
     handful of `// SAFETY:` comments written in the same change —
     modules with a few intentional unsafe sites;
  3. **the recorded remainder** — structural unsafe surfaces (vm.rs,
     the allocator, the fdt parsing): listed in the resolution, no
     change.
- No refactoring of existing unsafe code; the ratchet only denies
  what is already clean (or documents what is already there).

Honest sizing (refreshed 2026-08-27): ~409 unsafe grep-hits against
~46 `// SAFETY:` comments — the SAFETY count grew 8× organically
since the plan's 238/6 census, which strengthens the case that
bucket 2 won't produce rushed comments. Bucket 1's kernel-side list
is short (fdt/allocator are bucket 3); xtask stays `forbid`
(verified zero unsafe). Rushed SAFETY comments are worse than none:
bucket 2 only takes modules whose few sites are genuinely
self-evident to document. Tidy-up in the same audit: `port/src/
lib.rs:5` has a crate-level `#![forbid(unsafe_op_in_unsafe_fn)]`
duplicating the workspace lint. Precedent: Rust-for-Linux runs
`-Wclippy::undocumented_unsafe_blocks` tree-wide (linux
Makefile:502); Asterinas confines all unsafe below a
`forbid(unsafe_code)` boundary — bucket 1 is a lightweight step
toward that shape.

## Acceptance

- `cargo xtask ci` green with the gate flag and the module
  attributes in place.
- Introducing `unsafe` into a bucket-1 module, or an undocumented
  block into a bucket-2 module, fails the build.
- The resolution lists every audited module and its bucket.

## Not in scope

The tree-wide flip (238-site campaign) — deliberately cut; the
ratchet replaces it. Refactoring structural unsafe into
safe-shaped APIs.
