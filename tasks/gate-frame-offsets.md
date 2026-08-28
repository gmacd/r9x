---
status: open
---

# gate-frame-offsets: single-source the trap-frame layout (aarch64)

Task 2 of 7 in the gates-hardening arc. Plan:
[plans/gates-hardening.md](plans/gates-hardening.md).

## Goal

The frame layout is triple-maintained (trap.S/swtch.S literals, the
`FRAME_*`/`CONTEXT_SZ` consts, the structs); the host pins cover
Rust↔Rust only, and the asm leg — where the missing-SPSR-store bug
lived — is checked by comment. Make a Rust↔asm mismatch fail at
**assembly time** in the production pipeline: an eliminator, not a
detector.

**The disease is live (2026-08-27 audit):** trap.S:53-56 says "a
288-byte frame ... the interrupted stack pointer, offset 280", but the
frame is 304 bytes (`sub sp, sp, #304`, trap.S:128) and SPSR at 288
(process.rs:82 `FRAME_SPSR`) is now the last slot — the comment-checked
layout drifted within a month of the plan being written. swtch.S:28
hardcodes `#112` = `CONTEXT_SZ`. This is the highest-value gate in the
arc; land it first.

## Verified design premise

`global_asm!` takes no operands (a const after the string is a parse
error, tested on the old pin nightly-2026-07-27; re-verify on the
current pin when landing) but accepts **multiple template strings,
concatenated** (tested: a `.equ` in the first string is visible in the
second; a bad immediate fails with `index must be an integer in range
[-256, 255]`).

One honest difference from Linux's asm-offsets (which computes
`offsetof` with the *target* compiler): the `.equ` prelude comes from a
hand-maintained consts file, and the struct leg is closed by *host-run*
`offset_of!` pins — assuming host and aarch64 agree on the `repr(C)`
layout. True for all-u64 `repr(C)` structs on 64-bit hosts; computing
offsets in build.rs directly is impractical (the structs pull in
kernel-only deps), so consts-file + pins is the right call — but the
assumption is now stated. The x86_64 tree already proves the local
half of the pattern (`dat.rs:241-242`, `vsvm.rs:227-233` use
`offset_of!` pins).

## Changes

- The consts move to **`aarch64/src/frame_offsets.rs`** (consts
  only: FRAME_SZ, FRAME_ELR, FRAME_SP, FRAME_SPSR, CONTEXT_SZ),
  `include!`-ed by process.rs (replacing the current consts there).
  The current consts are cfg-gated `#[cfg(any(target_os = "none",
  test))]` (process.rs:75-84); the shared file must be **plain ungated
  consts** — build.rs includes it in a host context where that cfg
  doesn't match.
- The offsets writer merges into the **existing `aarch64/build.rs`**
  (the "first build.rs in the tree" premise is stale — server-ELF
  staging added one; note that script panics when server ELFs aren't
  staged, build.rs:56-61, so a bare `cargo build` of the crate hits
  that before the prelude matters): `include!`s the same file and
  writes an `offsets.s` prelude of `.equ` lines to OUT_DIR.
- The `global_asm!` calls take the prelude first:
  `global_asm!(include_str!(concat!(env!("OUT_DIR"), "/offsets.s")),
  include_str!("trap.S"))` — same for swtch.rs (its hardcoded 112 is
  the same disease).
- trap.S / swtch.S reference the non-structural offsets as symbols
  (`str x3, [sp, #FRAME_SPSR]`, the staging loads, the frame and
  context sizes). The save/restore stp pairs stay index-based: the
  slot *is* the register index.
- The existing host pins (consts↔structs, trap.rs tests) stay and
  now complete the circle.

## Acceptance

- Changing `FRAME_SPSR` and nothing else fails the build at assembly
  naming the slot.
- Changing a `TrapFrame` field order without the consts fails the
  host pins.
- Full `cargo xtask ci` green (the assemble-gate task, when it
  lands, must pick the prelude up for free — it is a build).

## Not in scope

x86-64/riscv64 (their `l.S` is boot-only; revisit when their entry
paths get a comparable frame); a whole-`.S` generator.
