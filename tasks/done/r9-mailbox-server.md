---
status: done
---

# Task: Mailbox Server (move Mailbox ownership to user-space)

**Tier:** 2.4
**Arch:** aarch64 (the Mailbox is BCM283x-specific)
**Depends on:** Task 81 (display server, `SYS_MAP_MMIO`)
**Status:** DONE 2026-08-26 (commit `16f0a75`)

## Implementation notes

- `SYS_FB_CONFIGURE` (27) repurposed as `SYS_ALLOC_PAGE` (same slot): allocates a
  page in the process heap, returns (VA, PA). The Mailbox takes a physical address
  for its request buffer, which user-space can't know otherwise.
- The kernel's `mailbox.rs` is kept (the PL011 driver calls `set_clock_rate`;
  the firmware does NOT pre-set the UART clock on all boards). The board-info
  prints and `configure_framebuffer()` are removed from the kernel.
- `NCHANNELS` raised from 6 to 16 (the `bringup` function now creates 6 kernel
  channels; servers create their own pairs on top).
- `Handles` extended with `extra_inbound`/`extra_outbound` (the display server
  needs both the nameserver's and mailbox's pairs).
- The mailbox server hardcodes `0xFE0000B8` (BCM283x Mailbox physical base) rather
  than parsing the FDT — the FDT approach is a future refinement.

## Context

The Mailbox property interface is currently a kernel concern: `aarch64/src/mailbox.rs`
implements the protocol, reads board info at boot, and configures the framebuffer via
a dedicated syscall (`SYS_FB_CONFIGURE`). This is wrong on two counts:

1. The Mailbox is arch-specific (BCM283x only), but `SYS_FB_CONFIGURE` lives in the
   cross-arch ABI (`r9x_abi`).
2. The kernel implements device protocol, which is the server's job (Plan 9 / QNX
   shape: the kernel is a message-passing broker, the device is dumb).

The PL011 (console) does not depend on the Mailbox (its clock is set by firmware
before the kernel starts). The kernel has no operational need for the Mailbox — its
only uses are debug output (board info) and the framebuffer config.

## Goal

A `cmd/mailbox` server that owns the Mailbox property interface. Any process that
needs a firmware property sends a message to the mailbox server. The kernel's
Mailbox interaction drops to zero: it just spawns the server like any other process.

## Design

### `cmd/mailbox` (new server)

- Parses the FDT (via `r9x_core::fdt`, using `dtb_va` from `start()`) to find the
  Mailbox node and its physical address.
- Calls `SYS_MAP_MMIO` to map the 64-byte Mailbox register page into its own page
  table.
- Implements the Mailbox protocol: the property tag format, the request channel
  (write tag list + zero word, spin for status), the read-back.
- Handles IPC requests from other processes:
  - **Configure framebuffer** (the two-request sequence: SET_* tags, then ALLOCATE).
    Replies with the framebuffer's physical address and size.
  - **Board info** (revision, model, serial, MAC, firmware revision). Replies with
    the values.
  - Future: ARM frequency, SDRAM size, toggle interrupts, etc.
- Publishes its name in the nameserver (`/dev/mailbox`).
- The protocol is message-based: each request is a tagged message on the server's
  inbound channel; the reply goes on the caller's outbound channel.

### `cmd/display` (modified)

- Sends an IPC message to the mailbox server: "configure framebuffer 640×480×32".
- The mailbox server performs the two-request sequence, replies with physical
  address + size.
- The display server calls `SYS_MAP_MMIO` to map the framebuffer into its own page
  table at `FB_VA`.
- The display server writes pixels to the mapped region (unchanged).
- No `SYS_FB_CONFIGURE` call (that syscall is removed).

### Kernel (reduced)

- **Remove** `aarch64/src/mailbox.rs` (the protocol code, the board info reads,
  the `configure_framebuffer()` function, the `FB_PHYS` global).
- **Remove** `SYS_FB_CONFIGURE` from `r9x_abi` and the kernel's trap dispatch.
- **Remove** the board info `println!`s from `main9` (revision, model, serial,
  MAC, firmware revision). The mailbox server prints its own info on startup.
- **Remove** `mailbox::init(&dt)` from `main9` and the integration images.
- The kernel spawns the mailbox server via `system::bringup()` (or a new
  `system::spawn_mailbox()`), like any other server.

### `r9x_abi` (reduced)

- **Remove** `SYS_FB_CONFIGURE` (syscall 27). The number is retired.
- The `FB_VA`, `FB_WIDTH`, `FB_HEIGHT`, `FB_SIZE` constants stay (they're the
  display server's conventions, not the Mailbox's).

### `r9x_std` (reduced)

- **Remove** the `fb` module (`std/src/fb.rs`). The display server uses IPC to
  the mailbox server + `SYS_MAP_MMIO` directly (no `fb::configure()` wrapper).

## Spawn order

The mailbox server must be up before the display server (the display server sends
it a request during init). In `system::bringup()`:

1. Nameserver (0)
2. Mailbox server (1) — new
3. Console server (2)
4. Init (3)

The display server is spawned after `bringup()` returns (it runs forever), same as
now. By the time it starts, the mailbox server is up and serving.

## Message protocol

The mailbox server's IPC protocol (on its inbound channel):

```
Request (opcode = 0):
  [tag: 2 bytes LE]  — the property tag (e.g. 0x00040001 = FbAllocate)
  [payload: N bytes]  — the tag's input value (0 for SET tags, width/height for Fb)

  For the framebuffer config, a special opcode:
  [opcode: 2 bytes LE = 1]  — "configure framebuffer"
  [width: 4 bytes LE]
  [height: 4 bytes LE]
  [depth: 4 bytes LE]

Reply (on the caller's outbound channel):
  [status: 2 bytes LE]  — 0 = OK, 1 = error
  [data: N bytes]       — the result (phys_addr + size for Fb, values for board info)
```

The exact wire format is to be refined during implementation. The key point: the
mailbox server is a general property server, not framebuffer-specific.

## Files touched

| File | Change |
|---|---|
| `cmd/mailbox/Cargo.toml` | New (server crate) |
| `cmd/mailbox/src/main.rs` | New (FDT parse, SYS_MAP_MMIO, protocol, IPC loop) |
| `cmd/display/src/main.rs` | Modified (IPC to mailbox server + SYS_MAP_MMIO, remove fb::configure) |
| `aarch64/src/mailbox.rs` | Removed (protocol moves to the server) |
| `aarch64/src/main.rs` | Modified (remove mailbox::init, board info prints) |
| `aarch64/src/system.rs` | Modified (spawn mailbox server) |
| `aarch64/src/ipc.rs` | Modified (remove sys_fb_configure) |
| `aarch64/src/trap.rs` | Modified (remove SYS_FB_CONFIGURE dispatch) |
| `aarch64/src/process.rs` | Modified (remove SYS_FB_CONFIGURE re-export) |
| `aarch64/tests/display.rs` | Modified (remove mailbox::init, add mailbox server spawn) |
| `abi/src/lib.rs` | Modified (remove SYS_FB_CONFIGURE) |
| `std/src/fb.rs` | Removed |
| `std/src/lib.rs` | Modified (remove fb module) |
| `aarch64/Cargo.toml` | Modified (add mailbox to NIMAGES / image registry) |

## Acceptance

- `cargo xtask ci` green (all arches, warning-free, all QEMU images pass).
- The display server configures the framebuffer via IPC to the mailbox server
  (no syscall).
- The kernel has zero Mailbox interaction (no `mailbox.rs`, no board info reads).
- The mailbox server prints board info on startup (the debug output moves there).
- `SYS_FB_CONFIGURE` is removed from the ABI.
