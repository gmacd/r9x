---
id: 87
status: done
commit: ad739c6
---

# Task 87: Mailbox server maps the wrong PA — external abort, not a VM bug

## Status: done (ad739c6)

Supersedes `r9-user-mmio-translation-fault.md` (2026-08-27 audit). The
symptom is the same; the diagnosis there was wrong in three load-bearing
ways, and the investigation it prescribed (the recursive page-table walk)
was a dead end — the walk's index math is correct (verified by hand for
`0x70000000` at every level).

## Problem

`SYS_MAP_MMIO(0xFE000000, 0x70000000, 0x1000)` succeeds and the mapping is
**valid** — but the first access to `0x700000C4` takes a **synchronous
external abort** (bus error), because the physical address is a hole.

Reproduces at HEAD (`fd7e96c`):
`cargo xtask qemu --arch aarch64 --image display --timeout 60` →
`process 1 faulted: far 0x700000c4 esr ... iss: 0x00000010`.

## Corrected diagnosis

1. **The fault is DFSC 0x10, not "DFSC=4".** For a Data Abort, ISS[5:0]
   is the DFSC; `0x10` = 0b010000 = synchronous external abort on a
   *successful* translation (Linux `arch/arm64/mm/fault.c:929` `do_sea`).
   The PTE is valid; the physical access failed. The old file's own
   evidence (`iss=0x10`) refuted its "PTE not valid" claim.
2. **PA `0xFE00_0000` is unassigned on raspi4b.** QEMU `info mtree -f`:
   nothing between `0xFE000000` and `0xFE002FFF`; the mailbox device
   (`bcm2835-mbox`) is at `0xFE00_B880`–`0xFE00_BBFF`. raspi4b is a
   post-2017 board model, so unassigned accesses raise SEAs rather than
   being ignored. The PL011 "works" simply because `0xFE201000` is a
   real device.
3. **The register layout is also wrong.** `cmd/mailbox/src/main.rs:12-26`
   assumes `+0x00 READ, +0x04 POLL_READ, +0x08 WRITE, +0x0C POLL_WRITE`
   with FULL/EMPTY at bit 0 (`0xB8` looks like a mangled `0xB880`). The
   kernel's own working driver (`aarch64/src/mailbox.rs:14-19`, base from
   the DT) has the real BCM layout: `READ 0x00, STATUS 0x18, WRITE 0x20`,
   `FULL 0x8000_0000`, `EMPTY 0x4000_0000`.
4. The old file's "L1 index 112 vs 128" table was bad arithmetic:
   `0x70000000>>30 = 1`, `0x80000000>>30 = 2` — the two VAs differ at
   **L2** (384 vs 0), not L1.
5. **The request buffer's memory type is the last (and silent) blocker.**
   The correct PA + register layout were *necessary but not sufficient*.
   With only those fixed, both Mailbox requests complete without faulting
   but the firmware's `ALLOCATE` returns `phys=0 size=0`. The reason:
   the user-space server backed its request/response buffer with
   `SYS_ALLOC_PAGE` → `heap_alloc_page` → `map_user_data_page_pa`, i.e.
   **Normal, write-back, cacheable** memory. The CPU's writes to the
   buffer sit in the cache; the VC reads the buffer by DMA from DRAM and
   sees stale zeros (an empty/invalid request → no allocation). The kernel
   driver works only because `Mailbox::new` uses `alloc_device_page`
   (Device/uncached): the writes reach DRAM directly, and the VC's response
   is likewise visible to the CPU. A cached buffer breaks *both*
   directions (CPU→VC request and VC→CPU response).

   The EL0 process cannot flush its own cache (no cache-maintenance
   permission, no syscall for it), so the buffer must be mapped Device.
   Fix: back the buffer with a page (`SYS_ALLOC_PAGE`, whose Normal VA is
   left unused) and re-map that same PA at `MBOX_BUF_VA` via
   `SYS_MAP_MMIO` (Device attributes) — no kernel/ABI change.

## Fix

In `cmd/mailbox/src/main.rs`:
- map page `0xFE00_B000` (registers at page offset `0x880`);
- use READ +0x00 / STATUS +0x18 (bit31 FULL, bit30 EMPTY) / WRITE +0x20,
  mirroring `aarch64/src/mailbox.rs`;
- back the request/response buffer with a page re-mapped as **Device**
  memory at `MBOX_BUF_VA` (`SYS_MAP_MMIO`) so the VC's DMA sees the CPU's
  writes and vice-versa (see diagnosis point 5);
- better: pass the DT-derived physrange to the server rather than
  hard-coding (the kernel already parses `brcm,bcm2835-mbox`).

Then **revert commit 3da8c4b's workaround** (`cmd/display/src/main.rs`
hard-codes `phys = 0x3c10_0000` "for a QEMU mailbox ALLOCATE bug") and
retest: the "garbage phys address" of 62b50ed and the "ALLOCATE bug" both
fit a mailbox server that never talked to the real device. If the real
ALLOCATE path works after the fix, both "QEMU bugs" dissolve.

## Verification

- `cargo xtask qemu --arch aarch64 --image display` passes with the
  workaround reverted; the firmware returns `phys=0x3c100000 size=0x12c000`
  (640×480×4), which the display server maps and renders.
- `cargo xtask integration-test --arch aarch64` is 24/24 (the display image
  was the one failing before).
- Task 91's row 11 mailbox variant (map `0xFE00B000` @ `0x7000_0000`,
  read STATUS at page offset `0x898`) goes green.
- Full `cargo xtask ci` green.

## Note: server exit branches now print their reason

While chasing the silent `phys=0`, every early-exit branch of the mailbox
and display servers was given a `println!` reason (the reason the process
exited with a bare `status 0` was invisible). Keep this convention: a
server that takes an error exit prints why first.

## Spun-off findings (now their own tasks)

Auditing the wrongly-blamed walk surfaced three real latent issues:
- task 94 — `map_to`'s `Err` arm leaks the clobbered recursive entry,
  and `Ok` is returned without read-back verification;
- task 93 — the fault print shows raw ISS; a DFSC decoder would have
  made "external abort ⇒ bad PA, not bad PTE" obvious immediately;
- task 99 (`r9-mmio-pa-validation.md`) — the device-dumb `sys_map_mmio`
  will map any PA, including holes; validate requested PAs against
  DT-derived device ranges so a bad address fails at the syscall with a
  named error instead of a delayed external abort. Parked; see its file
  for the unpark trigger.

## Related

- Unmasked by task 86 (the channel race had kept the mailbox server from
  ever reaching its MMIO access); "24/24 tests pass" on 3da8c4b predates
  the unmasking.
- The kernel's own mailbox mapping works because it maps the DT-provided
  PA — the path difference (TTBR1 vs TTBR0) was never the distinction;
  the *address source* was.
