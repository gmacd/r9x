---
id: 96
status: done
closed: d773a37 (2026-08-27)
---

# Task 96: User text is mapped writable + executable

## Status: done (d773a37, 2026-08-27)

`rw_user_text` → `ro_user_text` (AP AllRw → AllRo); PXN was already
true. `spawn_raw`'s misleading `user_text` local renamed to `ktext`.
The load path was unaffected as predicted (kernel writes via the TTBR1
alias). The store-to-own-text-is-killed integration test was **not**
written here — it is row W^X of task 91's matrix, which also pins the
new entry encoding.

## Problem

`Entry::rw_user_text()` (`aarch64/src/vm.rs:181-190`) encodes
`AllRw` + `uxn(false)` — user text pages are **writable and
executable**. Every spawned program's code can be rewritten by the
program itself (or by a stray store), which defeats fault isolation's
value as a debugging signal and is the classic W^X violation every
reference kernel guards against (Linux ships `ptdump_check_wx` to
assert no W+X page exists, `arch/arm64/mm/ptdump.c:338-371`).

## Design

- Text pages: RO + X for EL0 (`AP` read-only for user, `UXN=0`), PXN=1
  (EL1 must never execute user pages — check the current encoding while
  here).
- **Verified load path (re-checked 2026-08-27 against the code — an
  earlier review note here claimed there is no TTBR1 alias; that is
  wrong):** `map_user_page` (aspace.rs:131-176) maps the page into the
  process's TTBR0 at the user VA **and** into the kernel table (TTBR1)
  at `pa + KZERO`, and returns the *kernel* identity pointer; its doc
  comment states the kernel "cannot write through the user VA — the
  same physical page must be reachable in TTBR1 for the text copy",
  and the ELF loader repeats it (process.rs:713-715). Both load paths
  copy through that TTBR1 alias (`spawn_raw` at process.rs:608 — the
  local is misleadingly named `user_text` but holds the kernel
  pointer; the ELF path's `kptr`/`dst` at process.rs:727-749). So
  **making the TTBR0 entry RO+X is free for the write path** — the
  kernel writes via the TTBR1 mapping (`rw_kernel_data`), which this
  change does not touch. No option (a)/(b) restructuring is needed.
- Rename `spawn_raw`'s `user_text` local (and tighten its SAFETY
  comment, process.rs:606-607) while here — the name is what misled
  the review; the ELF path's `kptr` is the honest name.
- Residual note, not a blocker: the TTBR1 alias stays mapped and
  kernel-writable for the process's life, so user text remains
  *kernel*-writable after load. Fine for a trusted kernel; the
  eventual teardown arc should unmap it with the process.
- Rename the constructor to match (`ro_user_text`), so the type reads
  true.

## Tests

- Host: the entry-encoding unit test (task 91 row 2) asserts the new
  bits — the audit flagged that row 2 as written would faithfully
  snapshot the bad encoding.
- Integration: a process that stores to its own text page is killed
  (FAULT_STATUS), asserted in an image (extends `aspace_fault.rs`).

## Done when

- User text is RO+X; both tests pass; task 91's row 2 pins the new
  encoding.
- Full `cargo xtask ci` green.

Origin: backlog audit 2026-08-27 (VM group — gap in task 91's matrix).
