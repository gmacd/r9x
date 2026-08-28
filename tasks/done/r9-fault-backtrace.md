---
status: done
---

# Task: Fault backtrace — print a user-code stack trace on EL0 faults

**Tier:** 2.5
**Arch:** aarch64 (first), then riscv64 / x86-64 (with 74b — their frame
layouts don't exist yet, so "mechanical" was optimistic)
**Depends on:** Task 74 (ELF loader, `load_elf` — done) and task 92
(fault-checked user reads — the walker cannot exist without it, see
Safety). Does **not** depend on 74b: the aarch64 half is the whole
deliverable for now (the 2026-08-27 audit removed todo.md's wrong claim).
**Status:** done (commit e88a6bb, branch `correctness-batch`)

## Context

When a user process faults, the kernel prints:

```
process 2 faulted: far 0x40000000 esr EsrEl1 { iss: 0x00000043, il: true, ec: Ok(DataAbortLowerEl), iss2: 0x00 }
```

This tells you *what* faulted (the address and syndrome) but not *where* in
user code. Debugging a black-screen display server or a silent panic-reduced
to `exit(0)` is miserable without a backtrace.

Linux prints a "Call trace:" for exactly this reason. We want the same:
when a process faults, walk its user stack and print the return addresses
mapped to symbol names from the ELF.

## Goal

On an EL0 synchronous fault (data abort, instruction abort), after printing
the FAR/ESR line, print a backtrace:

```
process 2 faulted: far 0x40000000 esr ...
  backtrace:
    #0  0x000104a8  flip+0x28
    #1  0x000103f0  main+0x44
    #2  0x00010120  start+0x14
```

The backtrace is best-effort: if the stack is corrupt or the symbol table
is absent, print raw addresses (or stop). It must not fault itself (every
stack read is bounds-checked against the process's mapped stack pages).

## Design

### Stack walking (aarch64)

The fault frame (in `trap.rs`) saves the user's `SP` and `LR`. For aarch64
function calls (`bl`/`blr`), the frame layout is:

```
[sp]      = saved FP (frame pointer), if the function uses one
[sp + 8]  = return address (LR at call time)
```

Frame pointers are **not currently forced** (verified 2026-08-27: no
`force-frame-pointers` in xtask, no `"frame-pointer"` key in
`lib/r9x-aarch64.json`). Set `"frame-pointer": "always"` in the target
spec JSON rather than an xtask RUSTFLAGS bolt-on — it then applies to
every crate built for the target, `r9x_std` included. That gives every
frame a stable FP chain:

```
fp->saved_fp  (next frame's FP)
fp->lr        (return address)
```

Walk: start at the fault frame's `FP` (if non-zero) or `SP+8` (the first
return address). For each frame, read `fp` and `lr`, print `lr`, advance
to the saved `fp`. Stop after 32 frames (corrupt-stack guard) or when
`fp` is zero, not **16-byte aligned** (AAPCS64 — the original "page-
aligned" check was wrong), or outside the process's stack VA range.

### Symbol resolution

**Decision (2026-08-28): offline symbolication first (Zircon shape).**
Print raw addresses + the image registry index. The ELF is in the build
tree; an xtask helper (or `llvm-symbolizer`) turns the log into names.
Zero kernel-resident symbol storage, no ELF parser changes, no per-
process heap. The log line is already actionable:
`#0 0x000104a8 (image 2)` → `llvm-symbolizer -e target/.../display.elf
0x000104a8` → `flip+0x28`.

In-kernel symbol tables (`.symtab`/`.strtab` retained per process, binary
search on print) are a follow-up if the offline path proves insufficient.
No reference OS keeps user symtabs in the kernel — Linux user faults get
SIGSEGV and a debugger symbolicates; Zircon's crashsvc emits symbolizer
markup that a host tool resolves.

### Safety

- **The existing `copy_from_user` does not do what this section
  originally claimed.** It is an unchecked `copy_nonoverlapping`
  (`aarch64/src/ipc.rs:224-234`) — a garbage FP chain pointing at an
  unmapped VA would take a *kernel* data abort inside the fault handler.
  Every stack read must go through task 92's `read_user_word`: a genuine
  software walk of the process's TTBR0 that checks the leaf is valid and
  readable before touching the VA. Task 92 is a hard prerequisite.
- The walk is bounded (32 frames max) and each step checks that `fp` is
  16-byte aligned and within the process's stack VA range.
- The symbol table is read-only after load (no locking needed).

### Integration point

In `process::fault()` (called from `trap.rs` on EL0 data/instruction abort):
after the existing `iprintln!` of the FAR/ESR, call a new
`backtrace::print(process, fault_frame)`.

The fault frame is available in `trap.rs` (the `ExceptionFrame`). Pass the
relevant fields (`sp`, `fp`, `lr`) to the backtrace function.

## Changes

- **`lib/r9x-aarch64.json`**: add `"frame-pointer": "always"` to the
  target spec (applies to every crate built for the target, `r9x_std`
  included; preferred over an xtask RUSTFLAGS bolt-on).
- **`aarch64/src/trap.rs`**: pass the fault frame's `sp`/`fp`/`lr` to the
  new backtrace function.
- **New `aarch64/src/backtrace.rs`** (or a section in `process.rs`):
  the stack-walk + raw-address print logic (no symbol table).
- **`aarch64/src/process.rs`**: the `fault` function calls the backtrace
  printer after the existing FAR/ESR line.

Not in scope (deferred to a follow-up): ELF parser changes for
`.symtab`/`.strtab`, `Process` struct extension for a symbol table,
in-kernel binary-search symbolication.

## Verification

- Trigger a deliberate fault in a test image (e.g., write to an unmapped
  page) and check the backtrace shows the faulting function.
- The existing `display` test's fault should now show `flip` or `write_frame`
  in the backtrace.
- Warning-free for all three arches.
- The backtrace must not itself fault (test with a corrupt stack: fill the
  user stack with garbage and verify the walk stops cleanly).

## Future (not in scope)

- DWO / split-dwarf symbols (the current build has full `.symtab`).
- Inlining resolution (the backtrace shows the inlining function, not the
  inlined call site).
- Signal-based delivery (the backtrace is printed to the kernel console;
  a future `SYS_BACKTRACE` syscall could deliver it to user-space).
