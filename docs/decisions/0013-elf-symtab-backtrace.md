---
status: accepted
---

# 0013 — Fault backtraces symbolicate from the ELF `.symtab` at spawn

- **Status**: accepted — implemented (`aarch64/src/backtrace.rs`, `aarch64/src/demangle.rs`; commits `e88a6bb`/`8e26fbd` for the `.symtab` backtrace, task 90, and `c69cdcb` for the demangler, task 90c)
- **Date**: 2026-08-28 (record written from `tasks/plans/r9-backtrace-symbols.md`)
- **Context**: `tasks/plans/r9-backtrace-symbols.md`

## Decision

The kernel parses the spawned ELF's `.symtab` and resolves fault addresses to
the nearest symbol at or below the address, by linear scan. Rust v0 symbols
are demangled in-kernel. No build-time symbol generation, no separate symbol
file.

## Why

The ELF bytes are already in the kernel — the loader put them there
([0012](0012-user-binaries-are-elf.md)) — so the symbol table costs a parse,
not a pipeline. Build-time generation would duplicate the symbol data (once in
the ELF, once in a generated array) and add a build script that parses tool
output. Lookup is a once-per-crash cold path over a few dozen symbols, where a
linear scan is five lines and a binary search is fifteen plus a sort.

Fault backtraces matter more here than in most kernels because every EL0 fault
kills its process with no recovery to inspect
([0006](0006-aspace-shape-and-fault-policy.md)) — the backtrace is the whole
diagnosis.

## Alternatives rejected

- **Build-time code generation** (xtask runs `llvm-objdump`, emits a static
  array). Lost: duplicated data plus build-script fragility, to save a cold
  parse.
- **A separate symbol file beside the ELF.** Lost: a second artifact to keep
  in sync with the first.
- **Binary search over a sorted table.** Lost: machinery at n ≈ 10–50 on a
  once-per-crash path.

## Consequences

- Stripping `.symtab` from a server silently degrades backtraces to raw
  addresses; the build must not strip.
- The demangler is kernel code, so it must be panic-free on malformed input
  like any other kernel parser.
