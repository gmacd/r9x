---
id: 13
status: open
---

# Task 13: Mailbox server's `unsafe` blocks carry no `// SAFETY:` proof

## Status: open

## Problem

`cmd/mailbox/src/main.rs` has ten `unsafe` items and **zero** `// SAFETY:`
comments — the worst of the user-space servers (console and display carry
three each, nameserver one; mailbox zero, `cmd/*` census 2026-08-28). The
blocks that prove the hardest claims have no proof:

- `mbox_request` (the register-poll loop) — the safety is that `MBOX_VA` is
  a live Device mapping of the Mailbox register page for the process's
  life, and `buf_pa` is a physical page the VC can read.
- `configure_framebuffer` / `get_property` (the tag-buffer writes) — the
  safety is that `buf_va` is `MBOX_BUF_VA`, a live Device mapping of the
  request page, one full page, and the tag offsets stay in bounds of it.
- `mbox_reg_read` / `mbox_reg_write` — `MBOX_VA + w` stays within the mapped
  page (the register words are at `MBOX_REG_BASE`..`+8`).

The microkernel-and-firmware lens: *unsafe code carries proof
obligations; make the proof visible at the site.* A future reader auditing
the Device-buffer change (task 87) cannot confirm the buffer mapping is
valid at the write site without re-deriving it.

## Fix

Add a `// SAFETY:` to each `unsafe` block stating the specific invariant
(mapped-and-live pointer, in-bounds offset, Device attribute). Fold in the
`MBOX_BUF_VA` one-page bound for the tag writers.

## Verification

- Every `unsafe` item in `cmd/mailbox/src/main.rs` has a `// SAFETY:`.
- `cargo xtask clippy --arch aarch64` clean (no new warnings).
- Broader: extend to the remaining `cmd/*` gaps (child, heaptask, init,
  nameserver) if doing a sweep — but the mailbox is the load-bearing one.

## Related

- Spun off task 87 (mailbox MMIO fix) by the hardware-truth /
  microkernel-and-firmware review of that diff. The blocks are pre-existing;
  the Device-buffer change made their proof load-bearing, which is why the
  gap now shows.
