---
status: done
---

# r9-qemu-display: display server framebuffer via Mailbox (Tier 2.3)

Task 6 of 7 in the r9x arc (after the display server).
Gated on Task 80 (display server, done 2026-08-25).

## Hard constraint

The QEMU target is `raspi4b` — the same kernel image must boot on real Pi 4
hardware.  This is non-negotiable.

## Key finding: QEMU implements the BCM2835 framebuffer

QEMU's `raspi4b` machine implements the VideoCore firmware, including the
framebuffer, via the **Mailbox property interface** (channel 8, ARM-to-VC).
The QEMU docs list "Frame Buffer" and "VideoCore firmware (property)" as
implemented devices.  The same firmware runs on real Pi 4 hardware.

The kernel already has a complete Mailbox implementation
(`aarch64/src/mailbox.rs`): the `Mailbox` struct, the `request()` function,
the `Tag`/`Message` types, and several tag handlers
(`get_firmware_revision`, `get_board_model`, `set_clock_rate`, etc.).
What's missing is the framebuffer tag handlers.

## Mailbox framebuffer protocol

The framebuffer is in **guest DRAM** (not a device register).  The VideoCore
scans it out to the display.  The guest:

1. Sends a Mailbox request (channel 8) with a sequence of tags:
   - `SET_PHYSICAL_WIDTH_HEIGHT` (tag `0x00000006`): width, height
   - `SET_DEPTH` (tag `0x00000009`): bits-per-pixel (32 for RGBA)
   - `SET_PIXEL_ORDER` (tag `0x0000000B`): 0 = XRGB (the QEMU default)
   - `ALLOCATE` (tag `0x00000002`): the firmware allocates a buffer in
     VC RAM and returns the physical address and size
   - End tag (`0x00000000`)
2. The firmware response includes the **physical address** and **size** of
   the framebuffer in VC RAM.
3. The guest writes pixels to that physical address (in DRAM).
4. The VideoCore scans the buffer out to the display.

The tags are processed in order within a single Mailbox request.  The
firmware updates its internal config state as it processes the
`SET_*` tags, and the `ALLOCATE` tag allocates the buffer using the
accumulated config.

### Tag details (from QEMU's `bcm2835_property.c`)

| Tag | ID | Request | Response |
|-----|-----|---------|----------|
| `SET_PHYSICAL_WIDTH_HEIGHT` | `0x00000006` | u32 width, u32 height | u32 width, u32 height |
| `SET_DEPTH` | `0x00000009` | u32 bpp | u32 bpp |
| `SET_PIXEL_ORDER` | `0x0000000B` | u32 order (0=XRGB) | u32 order |
| `ALLOCATE` | `0x00000002` | (none — uses current config) | u32 phys_addr, u32 size |
| `GET_PITCH` | `0x0000000E` | (none) | u32 pitch (bytes/row) |
| `RELEASE` | `0x00000003` | (none) | (none) |

The `ALLOCATE` response's physical address is in **VC RAM** (the GPU's
memory region, returned by `GetVcMemory`).  The guest must map this
physical address into its page table to write pixels.

## Changes

### 1. Kernel: framebuffer tags in `mailbox.rs`

Add the framebuffer tag IDs to the `TagId` enum:
```rust
AllocateFramebuffer = 0x0000_0002,
ReleaseFramebuffer = 0x0000_0003,
SetPhysicalWidthHeight = 0x0000_0006,
SetDepth = 0x0000_0009,
SetPixelOrder = 0x0000_000B,
GetPitch = 0x0000_000E,
```

Add a `configure_framebuffer(width, height, bpp)` function that sends a
single Mailbox request with the `SET_PHYSICAL_WIDTH_HEIGHT`, `SET_DEPTH`,
`SET_PIXEL_ORDER`, and `ALLOCATE` tags.  Returns the physical address and
size of the framebuffer.

Note: the existing `request()` function handles a single tag per request.
The framebuffer configuration requires multiple tags in one request (the
firmware processes them in order, accumulating config state).  The
`request()` function needs to be generalized to handle multi-tag requests,
or a new `request_multi()` function is added.

### 2. Kernel: map the framebuffer into the display server's page table

The framebuffer is in VC RAM (a physical address returned by the firmware).
The kernel maps this physical address into the display server's page table
at a fixed virtual address (e.g., `FB_VA = 0x2000_0000`).

This is a regular page-table mapping (not `SYS_MAP_MMIO` — the framebuffer
is in DRAM, not a device register).  The kernel's existing page-table
machinery can do this.

### 3. Kernel: spawn the display server with the framebuffer mapped

The kernel's `main9` (after `bringup()`):
1. Calls `configure_framebuffer(640, 480, 32)`.
2. Maps the returned physical address into the display server's page table
   at `FB_VA`.
3. Spawns the display server.

The display server is not in `bringup()` (it runs forever — `run_all` in
the `system` image would never return).

### 4. Display server: write to the kernel-mapped framebuffer

The display server stops allocating its own `Vec<u8>` framebuffer.
Instead, it writes to the kernel-mapped region at `FB_VA`.

The `write_frame` function is unchanged (it writes to a `&mut [u8]`);
the buffer is now a kernel-mapped region at a known virtual address.

The display server knows `FB_VA` as a compile-time constant (it's in
`r9x_abi` or passed via the child-state page).

### 5. xtask: remove `-nographic` for the default kernel image

The default kernel image (`cargo xtask qemu --arch aarch64`) removes
`-nographic` so QEMU opens a display window.  The integration test images
keep `-nographic` (they don't need a display).

The QEMU display window shows the color bar moving left-to-right at
~10 Hz (the tick resolution limit on QEMU; 60 Hz on real Pi 4 via the
vblank interrupt).

### 6. Integration image: framebuffer readback (optional)

The `display` integration image can read back the framebuffer contents
(via the kernel's page-table mapping or a readback function) and check
the color bar is present.  This strengthens the verification from
"the display server is alive" to "the display server wrote the correct
pixels."

## Tests

- **QEMU display**: `cargo xtask qemu --arch aarch64` opens a display
  window showing the color bar moving left-to-right at ~10 Hz.
- **Integration image**: the `display` image passes (liveness check;
  optionally framebuffer readback).
- **Real Pi 4**: the color bar is visible on the display at 60 Hz
  (vblank-paced).  The same kernel image boots on both.

## Acceptance

- `cargo xtask ci` green (all arches; the integration tests pass on
  `raspi4b`).
- `cargo xtask qemu --arch aarch64` opens a display window showing the
  color bar moving left-to-right.
- The display server writes to the VideoCore's framebuffer (in VC RAM),
  not its own heap.
- The kernel has a `configure_framebuffer()` function (Mailbox tags).
- The same kernel image boots on real Pi 4 hardware and shows the color
  bar on the display.

## Not in scope

- 60 Hz on QEMU: the tick period is 100 ms, so the effective frame rate
  is 10 Hz.  60 Hz requires the Pi's vblank interrupt (real hardware).
- Vblank interrupt pacing (the Pi): the vblank channel is a refinement
  of the timer pacing.  The display server falls back to timer pacing
  when no vblank interrupt is available (QEMU).
- Window manager, input, Blitter: later stages.
- Multi-display: one framebuffer, one display.
- The `virt` machine: not the target.  `raspi4b` is the hard requirement.

## Risk

- The `ALLOCATE` tag returns a physical address in **VC RAM** (the GPU's
  memory region), not ARM RAM.  The kernel must map this into the display
  server's page table.  The VC RAM physical address range is different
  from the ARM RAM range (returned by `GetArmMemory` vs `GetVcMemory`).
  The kernel's existing page-table machinery should handle this, but the
  address range needs to be checked.
- The multi-tag Mailbox request: the existing `request()` function handles
  a single tag.  The framebuffer configuration requires multiple tags in
  one request.  The `request()` function needs to be generalized.
- QEMU's framebuffer emulation may have quirks (e.g., the default
  resolution, the pixel format).  The `SET_*` tags should configure it
  correctly, but testing will reveal any mismatches.

## Origin

The user wants to see the display server's activity when they run
`cargo xtask qemu --arch aarch64`.  The hard requirement is `raspi4b` as
the QEMU target (the same kernel image must boot on real Pi 4 hardware).
QEMU's `raspi4b` implements the BCM2835 framebuffer via the Mailbox
property interface — the same firmware that runs on real Pi 4 hardware.
The kernel already has the Mailbox; it needs the framebuffer tags.
