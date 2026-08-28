---
status: done
---

# r9: in-kernel Rust v0 symbol demangler

**Tier:** 4 (correctness and completeness)
**Status:** DONE (2026-07-03, commit c69cdcb)
**Origin:** user request (2026-07-03) — backtrace shows mangled names;
the user wants readable `crate::module::function` output.

## Problem

The fault backtrace (task 90 + symtab lookup) prints mangled Rust v0
symbol names. `nm -C` (or `llvm-cxxfilt`) demangles them:

| Mangled | Demangled |
|---------|-----------|
| `_RNvCs3CFL12ppSuB_9faulttest4main` | `faulttest::main` |
| `_RNvNtCsap4Ua6k489x_7r9x_std7process4exit` | `r9x_std::process::exit` |
| `_RINvNtCsap4Ua6k489x_7r9x_std2rt3runNvCs3CFL12ppSuB_9faulttest4mainEBz_` | `r9x_std::rt::run::<faulttest::main>` |

The v0 format is a sequence of length-prefixed name strings interleaved
with structural markers. The `rustc-demangle` crate is ~400 lines and
requires alloc; a no_std, no-alloc version for the kernel is a focused
task.

## Why the naive approach fails

A simple "scan for digit sequences followed by that many identifier
bytes" picks up spurious names from the disambiguator. The disambiguator
(after `C`) is a base62 string that can contain digits; a digit in the
disambiguator followed by a short run of identifier bytes looks like a
valid "length + name" pair (e.g. `3CFL` looks like "length 3, name CFL"
but is actually part of the disambiguator `s3CFL`). The correct boundary
is determined by the v0 grammar, not by a local "is this a valid name?"
check.

## Fix direction

Implement a proper v0 path parser in `aarch64/src/demangle.rs`:

1. **Grammar**: parse the v0 path per the [Rust reference](https://doc.rust-lang.org/reference/abi.html#rusts-legacy-name-mangling-scheme).
   The key elements:
   - Disambiguated name: `(N|S) (v|C<disamb>) <len> <name>`
   - Crate root: `C (v|C<disamb>) <len> <name>`
   - Path segment: `(N|M|I) (v|C<disamb>) <len> <name>`
   - Namespace markers: `t`, `v`, `m`, `b`, `l`, `o`, `p`, `y`, `q`
   - End of generics: `E`
   - Opaque type: `B` + optional disamb

2. **Disambiguator parsing**: after `C`, the disambiguator is a base62
   string. The boundary between disambiguator and length is found by
   trying each position: at position `i`, if `bytes[i..]` starts with a
   decimal number `n` and `bytes[i+digits..i+digits+n]` is a valid
   identifier of length `n`, that's the split. The compiler guarantees
   exactly one valid split.

3. **Output**: collect name slices in order of appearance, reverse
   (the v0 encoding is innermost-first for the path), join with `::`.
   Generic params (after `E`) are wrapped in `<>`. Write into a
   caller-provided 80-byte buffer (no alloc).

4. **Unit tests**: a table of known mangled → demangled pairs from our
   servers' symtabs (the three above, plus a few more from the display
   and console servers).

## Done when

- `cargo xtask qemu --arch aarch64 --image faultbacktrace` shows
  `faulttest::main+0xc` instead of `_RNvCs3CFL12ppSuB_9faulttest4main+0xc`.
- Unit tests cover: crate roots, nested modules, generic functions
  (`rt::run::<...>`), the `B` (opaque type) marker.
- `cargo xtask ci` green.
