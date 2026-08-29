---
id: 90b
status: done
commit: 8e26fbd
---

# r9: backtrace function names

**Tier:** 4 (correctness and completeness)
**Status:** done (8e26fbd + c69cdcb, 2026-08-27)
**Origin:** user request (2026-07-03) — "provide more information with
backtraces — e.g. function names, line numbers, etc if available"
**Plan:** [r9-backtrace-symbols.md](../plans/r9-backtrace-symbols.md)

## Problem

The fault backtrace (task 90) prints raw addresses. The user wants
function names in the output. The ELF's `.symtab` is already embedded in
the kernel (`EmbeddedElf.bytes`) — it just hasn't been parsed yet.

## Fix direction

1. **ELF section parsing** (in `process::spawn_elf` or a helper): find
   `.symtab` (SHT_SYMTAB=2) and its linked `.strtab` from the section
   header table. Extract `(symtab_ptr, nsyms, strtab_ptr)` as a `SymRef`.
2. **`Process` gets `symref: Option<SymRef>`**: set at spawn (ELF path),
   `None` for raw images.
3. **`backtrace::print_backtrace` takes `symref`**: for each return
   address, linear-scan the symtab for the largest `st_value ≤ addr`.
   Print `name+0xoffset (0xaddr)` if a match, raw address if not.
4. **`faultbacktrace` test**: the serial output should show
   `level3+0x...`, `level2+0x...`, etc. instead of bare addresses.

## Done when

- `cargo xtask qemu --arch aarch64 --image faultbacktrace` shows
  function names in the backtrace (e.g. `level3+0x34`).
- A stripped ELF (no `.symtab`) falls back to raw addresses (no crash).
- Raw machine-code images (no ELF) fall back to raw addresses.
- `cargo xtask ci` green.
