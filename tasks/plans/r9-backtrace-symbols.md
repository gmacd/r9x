# Fault backtrace: function names

## Problem and constraints

The fault backtrace (task 90) prints raw addresses. Debugging a fault
requires manually running `llvm-symbolizer` against the ELF in the build
tree. The user wants function names (and line numbers if available) in
the backtrace output directly.

Standing constraints: no_std kernel, warning-free across all arches,
minimal scope, aarch64-only (the other arches don't have the process
stack yet).

## Prior art

- **Plan 9**: `9l`'s `dbgsym` reads the ELF's `.symtab` at attach time;
  the `errout` prints `mod!func+off`. The symbol table lives in the ELF
  file, not in the process's address space.
- **Linux**: `elf_core_copy` + `__elf_search` in `fs/binfmt_elf.c`; the
  kernel reads `.symtab` from the file (or from the process's `vma` if
  mapped). For our case (static, embedded ELFs), the table is in the
  kernel's own memory (the `include_bytes!` buffer).
- **Zircon**: `debug_symbolize` reads the ELF's `.symtab` from the
  process's mapped memory (the ELF is mapped, including non-loaded
  sections, for debugging). Heavier than we need.
- **r9 already has**: the full ELF bytes embedded in the kernel
  (`EmbeddedElf.bytes`). The `.symtab` and `.strtab` sections are in
  those bytes — they're just not mapped into the process's VA space.

## Hardware assumptions (required)

No new hardware assumptions. This is pure software (ELF section parsing
+ table lookup). The only assumption is that the ELFs are static,
non-PIE, and linked at `IMAGE_BASE` (0x10_0000) — already a stated
invariant of the loader.

## Design

### Approach: parse `.symtab` from the embedded ELF at boot

The ELF bytes are already in the kernel (`EmbeddedElf.bytes`). The
`.symtab` and `.strtab` sections are standard ELF sections — a series of
`Elf64_Sym` entries (24 bytes: `st_name`, `st_info`, `st_other`,
`st_shndx`, `st_value`, `st_size`) and a null-terminated string table.
No build machinery needed; the kernel parses them in-place from the
`&'static [u8]` buffer.

### Data structures

```rust
/// A parsed function symbol from an ELF's `.symtab`.
pub struct Sym {
    pub addr: u64,       // st_value (the function's start address)
    pub name: &'static str,  // from .strtab[st_name..]
}
```

The parsed table is sorted by `addr` (the linker emits them in order,
but we sort at boot to be safe — it's a few dozen entries, insertion
sort is fine).

`Process` gets:
```rust
/// The process's image symbol table (for backtrace lookup).  `None` for
/// raw machine-code images (no ELF, no symbols).  Set at spawn, read-only
/// after.  Points into the kernel's embedded ELF buffer (lives for the
/// boot's life).
syms: Option<SymTable>,
```

Where `SymTable` is a small struct: `*const Sym` + `len` (or just a
`&'static [Sym]` since it's static-lifetime).

### Parsing

At spawn (ELF path), after the loader maps the text/data:
1. Parse the ELF header (already done by the loader — reuse its section
   header table).
2. Find `.symtab` (type `SHT_SYMTAB`) and `.strtab` (linked via
   `sh_link`).
3. Iterate the `Elf64_Sym` entries; keep only those with `st_info` type
   `STT_FUNC` (4) and `st_value != 0`.
4. Build a `&'static [Sym]` (the data lives in the `EmbeddedElf.bytes`
   buffer, which is `&'static`).

Wait — the `Sym.name` is a `&'static str` into the `bytes` buffer. The
`bytes` buffer is `&'static [u8]`, so this is safe (no lifetime issue).
The `Sym` struct itself is built on the stack at spawn, but it points
into static memory. I can either:
- Store the `Sym` array in a per-process static (wastes space)
- Or just store the `(symtab_ptr, strtab_ptr, nsyms)` tuple in `Process`
  and do the lookup at fault time (re-parse the entry, read the name)

The second is simpler and cheaper: store the section pointers, do the
lookup at fault time. The fault path is cold (once per crash), so a
few-dozen-entry linear scan is fine.

Revised `Process` field:
```rust
/// The ELF symbol sections for backtrace lookup: pointers into the
/// embedded ELF's `bytes` buffer.  `None` for raw images.
symtab: Option<(*const u8, usize, *const u8, usize)>,
//  (symtab_ptr, nsyms, strtab_ptr, strtab_len)
```

Actually, that's ugly. Let me use a small struct:

```rust
struct SymRef {
    /// Pointer to the first `Elf64_Sym` in the embedded ELF's `.symtab`.
    syms: *const u8,
    /// Number of symbols in the table.
    nsyms: usize,
    /// Pointer to the `.strtab` section's bytes.
    strtab: *const u8,
}
```

`Process` gets `symref: Option<SymRef>`.

### Lookup at fault time

`backtrace::print_backtrace(sp, fp, lr, symref)`:
- For each return address, find the largest `st_value ≤ addr` in the
  symbol table (linear scan — a few dozen entries, cold path).
- Print `#N  name+0xoffset  (0xaddr)`.
- If `symref` is `None` (raw image) or no symbol matches, print just the
  raw address (the current behaviour).

### ELF parsing

The loader (`process::spawn_elf`) already parses the ELF header and
program headers. I need to extend it to also find the section headers
(specifically `.symtab` and its linked `.strtab`). The section header
table is a standard ELF structure:
- `e_shoff` (offset to section header table)
- `e_shnum` (number of sections)
- `e_shstrndx` (index of the section name string table)

For each section: `sh_type == SHT_SYMTAB` (2) → it's the symtab. Its
`sh_link` gives the index of the `.strtab` (the linked string table).

This is ~30 lines of parsing code. No new dependencies.

### Interfaces

No new public API. The change is internal to the kernel's fault path:
- `process::spawn_elf` extracts the symtab reference (if present)
- `process::fault` passes it to `backtrace::print_backtrace`
- `backtrace::print_backtrace` does the lookup and prints names

### Init and bringup order

No new ordering. The symbol table is in the embedded ELF (already in
memory at spawn time). The parse happens at spawn (one-time, cold).

### Failure policy

- If the ELF has no `.symtab` (stripped), `symref` is `None` → fall
  back to raw addresses (the current behaviour).
- If a return address doesn't match any symbol (e.g., it's in the
  kernel, not in the user image), print the raw address with no name.
- The parse is infallible (the loader already validated the ELF header;
  a malformed section table means the ELF is corrupt, which the loader
  would have caught). No new panic paths.

## Not building

- **Line numbers**: require DWARF (`.debug_line`), which is 10-50x the
  binary size. Deferred. The function name + offset is the 90% use case.
- **In-process symbol mapping** (Zircon shape): mapping the `.symtab`
  into the process's VA space so user-space debuggers can read it.
  Deferred — no user-space debugger yet.
- **Kernel symbol table**: the kernel's own symbols for kernel-side
  backtraces (panic backtrace). Separate concern; the kernel's panic
  handler is a later task.
- **Dynamic symbol loading** (from a file system): no file system yet.
  The registry is embedded.

## Decision records

### Parse at spawn vs. build-time code generation

- **Decision**: Parse the ELF's `.symtab` at spawn (runtime).
- **Alternatives**: (a) Build-time code generation (xtask runs
  `llvm-objdump`, emits a Rust static array). (b) Embed a separate
  symbol file alongside the ELF.
- **Why runtime**: The ELF bytes are already in the kernel. Parsing
  `.symtab` is ~30 lines of straightforward code. No new build
  machinery, no code generation, no `OUT_DIR` staging. The "change the
  server, the image rebuilds" property already works (the ELF is
  re-embedded on change). A build-time approach would duplicate the
  symbol data (once in the ELF, once in a generated array) and add
  build-script fragility (parsing `llvm-objdump` output).
- **Dissent**: The kernel-taste lens would prefer build-time (no parsing in
  the kernel, the data is ready before boot). But the parsing is
  one-time, cold, and the code is small. The simplicity of "the ELF has
  everything we need, we just read it" wins.

### Linear scan vs. binary search

- **Decision**: Linear scan (cold path, a few dozen entries).
- **Alternatives**: Binary search (requires sorted table, or a sort at
  spawn).
- **Why linear**: The tables are tiny (10-50 function symbols per
  server). The fault path is once-per-crash. A linear scan is 5 lines;
  a binary search is 15 lines + a sort. Not worth it.
- **Dissent**: None. Both agree this is a non-issue at this scale.

## Tasks

1. **`tasks/r9-backtrace-symbols.md`** — Extend the backtrace to print
   function names: parse `.symtab`/`.strtab` from the embedded ELF at
   spawn, store a `SymRef` in `Process`, and do a nearest-symbol lookup
   in `backtrace::print_backtrace`. The `faultbacktrace` test image
   should show `level3+0x...` instead of raw addresses.
