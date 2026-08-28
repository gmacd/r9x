---
id: 115
status: open
wave: 3
---

# Task 115: FDT, ELF and backtrace parsers index unvalidated offsets

## Status: open — wave 3

## Problem

Three parsers take untrusted or externally-supplied input and index it
with raw slicing.  Same fix shape throughout, all host-testable with
hand-built fixtures.

**FDT header offsets (`core/src/fdt.rs:93`, `:100`).**
`FdtHeader::new` (`:616`) validates only the magic and
`totalsize == len`.  `structs()` and `strings()` then do
`&self.data[start..start + size]` with the firmware-supplied
`off_dt_struct`, `size_dt_struct`, `off_dt_strings` and
`size_dt_strings`.  A device tree whose offsets are inconsistent with its
`totalsize` panics the kernel inside the first `root()` call at boot.
Every other read in the file is `.get()`-guarded, so these two are the
hole rather than the pattern — the recent `fdt-u32-iter-length` fix
(b2880a3) closed a neighbouring one.

**ELF section bounds (`port/src/elf.rs:263`).**  Unchecked
`sh_offset + sh_size` where the rest of the file uses `checked_add`.

**Backtrace (`aarch64/src/backtrace.rs:108`).**  The `unwrap_or_default()`
fallback yields an empty slice, and the loop below then indexes it —
guaranteeing an out-of-bounds panic *inside the EL0 fault handler*.  That
is a panic in the panic path: the diagnostic machinery destroys the
diagnostic.  Worst of the three despite being the smallest, because it
fires exactly when something else has already gone wrong.

## Design

- Validate the FDT header's four offset/size pairs against `totalsize`
  in `FdtHeader::new`, returning `ParseError`, so `structs()`/`strings()`
  cannot be reached with bad values.
- `checked_add` in `elf.rs`, matching the file's own convention.
- `backtrace.rs` handles the empty-slice case explicitly and degrades to
  "no symbols" rather than panicking.  A fault handler must be total.

## Tests

- Host: truncated, oversized and self-inconsistent DTBs are rejected
  rather than panicking — the `core/tests/fdt_test.rs` harness already
  builds DTBs by hand, so the fixtures are cheap.
- Host: an ELF with a section extending past the file is rejected.
- Integration: a fault in a process with no symtab produces a backtrace
  without symbols, not a double panic.

## Done when

- No parser indexes an offset it has not validated.
- The three fixtures exist and fail before the change.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
