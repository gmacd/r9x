---
status: done
---

# elf-reader-port: a pure ELF64 reader in `port`

Task 1 of 4 in the user-binary-loading arc. Plan:
[plans/user-binary-loading.md](../plans/user-binary-loading.md).

## Goal

A minimal, arch-agnostic, host-testable reader that turns an ELF64 byte blob
(the embedded server image) into its entry point and its `PT_LOAD` segments.
This is the shared core of the user-binary machinery: the aarch64 loader
(task 2) consumes it, and every arch gets a host-testable parse for free. It
follows the `port::fdt` precedent — a pure parser over a byte slice, no I/O,
no allocation, `Result`-returning.

## Changes

New module `port/src/elf.rs`, `pub use`d from `port/src/lib.rs`.

- Types:
  - `ElfError` (an enum): `NotElf`, `Not64`, `WrongEndian`, `NotExec`
    (only `ET_EXEC` is accepted for now — `ET_DYN`/PIE is refused, see the
    plan's Not-building), `NoLoadSegment`, `Truncated` (header or program
    headers run past the slice), `BadSegment` (`filesz > memsz`, or
    `p_offset + filesz` past the slice).
  - `Segment { vaddr: u64, offset: u64, filesz: u64, memsz: u64, exec: bool }`.
  - `Elf { entry: u64, segments: [Segment; N] , nsegments: usize }` — `N` a
    small const bound (a real server has a handful; the bound is stated, not a
    knob). `segments()` returns `&[Segment]`.
- `pub fn parse(elf: &[u8]) -> Result<Elf, ElfError>`:
  - Check `e_ident` magic `0x7f 'E' 'L' 'F'`, `EI_CLASS == 2` (64-bit),
    `EI_DATA == 1` (little-endian — r9's targets are LE; a BE blob is
    `WrongEndian`, not mis-parsed).
  - Require `e_type == ET_EXEC`; read `e_entry`, `e_phoff`, `e_phnum`,
    `e_phentsize`.
  - Walk `e_phnum` program headers from `e_phoff`; bounds-check each (and the
    phdr table) against the slice; keep `PT_LOAD` entries (`p_type == 1`),
    with `exec = p_flags & PF_X` (`PF_X == 1`); enforce `filesz <= memsz` and
    `p_offset + p_filesz <= slice.len()`.
  - `NoLoadSegment` if there is no `PT_LOAD`; `BadSegment` on a size violation.
  - **Placement is not checked here** (user-half, alignment, overlap) — that
    is arch-specific and belongs to the loader (task 2).

The ELF64 field offsets are the System V AMD64/AArch64 ABI: header
`e_entry@24`, `e_phoff@32`, `e_phentsize@54`, `e_phnum@56`; `Phdr`
`p_type@0`, `p_flags@4`, `p_offset@8`, `p_vaddr@16`, `p_filesz@32`,
`p_memsz@40`. Cite them in a comment at the reader.

## Tests

`#[cfg(test)]` in `port/src/elf.rs` (the `port::fdt` host-test style). Build
the fixtures in memory (no files):
- A minimal valid ELF64 (one `R-X` `PT_LOAD`, one `RW-` `PT_LOAD` with a bss
  tail `memsz > filesz`): assert `entry`, segment count, per-segment
  `vaddr/filesz/memsz/exec`.
- Each `ElfError`: a 32-bit blob (`Not64`), a big-endian blob (`WrongEndian`),
  `ET_DYN` (`NotExec`), no `PT_LOAD` (`NoLoadSegment`), a truncated phdr
  (`Truncated`), and `filesz > memsz` (`BadSegment`).
- A short slice that cuts off mid-header (`Truncated`).

## Acceptance

- `cargo xtask test` green (the parser runs on the host).
- `cargo xtask check` and `cargo xtask clippy` green for **all three arches**
  (the module is arch-agnostic, so it must be warning-free everywhere).
- No `unsafe`; no `alloc`.

## Not in scope

Relocation parsing (the format is static non-PIE — there are none); `ET_DYN`/
PIE; the section header table (only the program headers are needed to load);
placement validation (task 2); any I/O.
