---
status: open
---

# gate-symbol-manifest: post-link structural assertions (ld-first)

Task 1 of 7 in the gates-hardening arc. Plan:
[plans/gates-hardening.md](plans/gates-hardening.md).

**Mechanism rewritten 2026-08-27:** the original spec did everything in
xtask with llvm-nm. Linux does the structural half **in the linker
script** — `arch/arm64/kernel/vmlinux.lds.S:400-434` is a wall of
`ASSERT()`s (entry == KIMAGE_VADDR, vector alignment, bss bracketing,
even `.equ` absolute-symbol values). An ld `ASSERT` is an *eliminator in
the production link* — the same philosophy this arc invokes for task 46
— with zero new tooling, no demangling, no ELF parsing. The nm-based
manifest shrinks to the one thing the linker cannot see: `st_size`.

## Goal

A deliberately-constant symbol that is GC'd, resized, or misaligned
fails the build naming the symbol — the two incident classes
(KSTACKS-4x, the GC'd `interruptstackbase`) become un-landable.

## Changes

- **kernel.ld `ASSERT()`s** (per arch), templated the same way
  `${LOAD-ADDRESS}` already is:
  - entry point equals the load address — inside the script,
    `ASSERT(start == ...)` sees both VMA and LMA for free; an xtask
    check would have to *evaluate* the arithmetic expression string in
    `config_default.toml:10` (`'0xffff800000100000 - 0x80000'`) and pick
    VMA vs LMA across the `.text.boot` `AT()` split — all of that
    disappears in ld;
  - `ASSERT(exception_vectors % 2048 == 0, ...)`;
  - bss start/end PROVIDE symbols bracket the table symbols (bounds
    only — under QEMU the RAM is zeroed before us; the zeroing loop is
    exercised only on real hardware, and the script comment says so);
  - absolute-symbol values (`interruptstacksz` is a global absolute
    symbol, trap.S:12-13) where they are deliberate constants;
  - x86-64: multiboot header within the first 8 KiB.
- **Section GC protection, ordered first**: move the vector table to
  `.text.vectors` (it sits in plain `.text` with `.balign 2048`,
  trap.S:179-181; `KEEP(*(.text*))` would kill GC for all code), add
  `KEEP(*(.text.vectors))` + the boot sections to kernel.ld, **then**
  add the assertions (KEEP resurrects previously-GC'd symbols;
  asserting first flags immediately).
- **The nm manifest, shrunk to sizes only**: `<arch>/lib/`
  `symbols.manifest.txt`, ≤10 lines per arch, `size` entries only —
  per-symbol `st_size` is not linker-visible (only `SIZEOF(section)`
  is). Requires adding `.size sym, .-sym` / `.type` directives to the
  `.S` definitions (none exist today, so `st_size = 0` and the check
  could never fire). xtask runs `llvm-nm --demangle --print-size` on
  the ELF artifact after `dist`. **Matcher rule:** strip the legacy
  `::h<hash>` mangling suffix / match by path prefix — `KSTACKS` is a
  private static whose hash churns with unrelated changes, and exact
  hash matching becomes reflexive-baseline-update disease.
- Always print the `llvm-size` per-section table (standing bloat
  report; no threshold). Optional rider from the audit:
  `-Zemit-stack-sizes` + a max-frame report next to it (report, not
  gate — the Linux analogue is FRAME_WARN/checkstack.pl) against the
  fixed 64K interrupt stack.

## Acceptance

- Deleting a listed symbol, or changing a listed size, fails
  `cargo xtask ci` naming the symbol (ld error or manifest line).
- A `kernel.ld` edit that misaligns the vectors fails the link with the
  ASSERT message.
- The size table appears in every CI log.
- Full `cargo xtask ci` green.

## Not in scope

Real-hardware zeroing verification; x86-64/riscv64 asm `.size`
directives beyond what their manifest lines need; evaluating the
load-address expression in xtask (deliberately avoided — see above).
