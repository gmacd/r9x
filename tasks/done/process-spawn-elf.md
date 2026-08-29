---
status: done
---

# process-spawn-elf: the Image enum and the single spawn

Task 2 of 4 in the user-binary-loading arc. Plan:
[plans/user-binary-loading.md](../plans/user-binary-loading.md).

## Goal

Unify process creation under one image type and one entry point, and add the
ELF path. `aarch64::process::spawn(&Image)` where `Image::Raw { text,
text_va, stack_va }` is the current raw path and `Image::Elf(&[u8])` loads a
self-describing ELF — mapping each `PT_LOAD` into the process's own `Aspace`
(TTBR0), copying the file bytes, zeroing the bss tail, deriving a stack, and
starting the process at `e_entry`. The raw path's behavior is unchanged; its
~19 call sites move to `Image::Raw { … }`.

This is the **early-call unification** the plan's decision 2 records: the user
chose the `Image` enum now rather than deferring it to the second server, so
this task carries the call-site sweep as part of the signature change.

Depends on `port::elf` (task 1).

## Changes

All in `aarch64/`.

- `src/process.rs`:
  - Add the `Image` enum (as in the plan): `Raw { text: &[u8], text_va: usize,
    stack_va: usize }` and `Elf(&[u8])`. Defined **unconditionally** (plain
    data; the host build sees it too).
  - Rename the current `spawn` body to `fn spawn_raw(text, text_va, stack_va)`
    (`#[cfg(target_os = "none")]`, private). Behavior unchanged.
  - `pub fn spawn(image: &Image) -> ProcessId`: a `match` —
    `Image::Raw { .. } => spawn_raw(..)`, `Image::Elf(e) => spawn_elf(e)`.
    Bare-metal body is the `match`; the host build keeps the existing spin
    stub (the current `#[cfg(not(target_os = "none"))]` `spawn`), re-shaped to
    take `&Image`.
  - `fn spawn_elf(elf: &[u8]) -> ProcessId` (`#[cfg(target_os = "none")]`,
    private) — the `Image::Elf` arm:
    1. `port::elf::parse(elf)`; on `Err(e)` `panic!` with the named error
       (callers are `main9` / the test images — init context).
    2. **Placement validation** (arch-specific, not in the parser): for each
       segment, `vaddr` is page-aligned, `vaddr < KZERO` (the user half;
       `crate::param::KZERO`), and the segment's `[vaddr, vaddr+memsz)` does
       not overlap a prior segment. A violation `panic!`s with a named reason —
       an embedded ELF is still *input* and must not map into kernel space or
       alias itself.
    3. `Aspace::new()`; for each segment map its pages: `Entry::rw_user_text()`
       if `seg.exec` else `Entry::rw_user_data()`, at `seg.vaddr`, spanning
       `ceil(memsz, PAGE)` pages. Copy `filesz` bytes from
       `&elf[seg.offset .. seg.offset+filesz]` into the first mapped page with
       `copy_nonoverlapping`; zero the `memsz - filesz` bss tail. `// SAFETY:`
       on the copy — the page was just mapped user-writable and `filesz` fits
       the span (asserted).
    4. Map the stack pages (the region above the highest segment) as
       `rw_user_data`; the user SP is `stack_top - 16` (the same 16-byte
       headroom `spawn_raw` uses).
    5. Fabricate the entry context with the **existing** `forkret_context`
       path, feeding it `ELR = elf.entry` and the derived SP (it already takes
       ELR + SP; reuse it, do not fork a second context builder). Put the slot
       in the table as Runnable, exactly as `spawn_raw` does (share the
       slot-claim + table-store shape with `spawn_raw` if cleanly shareable;
       otherwise duplicate the ~10 lines rather than over-abstract).
  - Add the layout constants (per-arch, stated, in the TTBR0/user half): the
    image base `ELF_BASE` (page-aligned); `STACK_SZ` (a const, e.g. 64 KiB —
    match the kstack size); the derived stack sits in the `STACK_SZ` pages
    immediately above the highest loaded segment. These are **software
    conventions, not hardware facts** (a comment says so).
- **Migrate the ~19 raw call sites** to `Image::Raw { text: …, text_va: …,
  stack_va: … }`: `src/main.rs` (1) and the raw test images — `user_process`,
  `user_yield`, `two_process`, `two_yield`, `ipc` (3), `aspace` (2),
  `aspace_fault` (2), `preempt` (2), `prio` (3). Mechanical (a `sed`-able shape
  change); the program byte arrays are untouched. Also update the
  `src/aspace.rs` doc comment that names `process::spawn`. `console_server`'s
  site is migrated here to `Image::Raw` too (so the tree compiles); task 4
  flips that one to `Image::Elf`.
- The `unsafe` blocks carry `// SAFETY:` comments mirroring the existing
  `spawn` (repo standing constraint: every unsafe op spelled out).

## Tests

- Host: none new here (the parse is host-tested in task 1; the mapping is arch
  code that runs only at boot). The boot proof is task 4's image.
- **The migration regression net:** every migrated raw image must pass
  unchanged (they are behavior-identical; only the `spawn` call shape differs).
- The `aspace` image (two processes, same VAs, isolated) and `aspace_fault`
  (a faulting process dies, peers survive) already prove the isolation this
  relies on; they must still pass.

## Acceptance

- `cargo xtask check` / `cargo xtask clippy` green for **all three arches**
  (the aarch64-gated code must be warning-free; x86-64/riscv64 do not build
  this path yet — see Not in scope). Every raw call site compiles as
  `Image::Raw`.
- `cargo xtask ci` green — the **whole raw-image suite** passes on the unified
  `spawn` (this is what proves the migration changed no behavior).
- Proven end-to-end by task 4 (`console_server` image boots a real ELF).

## Not in scope

Cross-arch loaders (x86-64 / riscv64 have no `Aspace` yet; `spawn_elf` is
aarch64-only until their stage-3 lands. `Image` + `spawn` live in the arch
module beside them; hoist `Image` to `port` if a second arch adds `spawn`).
Tightening the `AllRw` user page model (W^X) — a cross-cutting hardening task,
inherited here as-is (plan, decision 5). Relocation processing (the format is
static non-PIE). A per-process configurable stack. Rewriting the raw
*programs* to ELFs (they stay hand-assembled; only the `spawn` call is
wrapped).
