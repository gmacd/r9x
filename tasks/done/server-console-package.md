---
status: done
---

# server-console-package: the console server as a built Rust executable

Task 3 of 4 in the user-binary-loading arc. Plan:
[plans/user-binary-loading.md](plans/user-binary-loading.md).

## Goal

The first real server: the console server as a **separate Rust executable**,
built to r9's user-binary format (a static, non-PIE, fixed-base ELF — see the
plan's verified format fact). Plus the xtask step that builds it. This is the
build-side artifact the embedding (task 4) consumes. Independent of tasks 1/2.

## Changes

- **New workspace member `servers/console/`** (add to the root `[workspace]
  members`). A `#![no_std] #![no_main]` bin:
  - `Cargo.toml`: `default-target = "aarch64-unknown-none-elf"` (or built via
    an explicit `--target`; see the ServerStep). `edition = "2024"`. Depends on
    `port` (for the syscall numbers / a shared `sys` shim if one is added) —
    or is fully self-contained for the arc. No std, no alloc.
  - `src/main.rs`:
    - a `#[panic_handler]` (exit non-zero, or halt loudly — the plan's
      failure policy: a panicking server is killed by the EL0 fault path
      anyway; the handler exists so a panic is not a link error);
    - a tiny syscall shim `unsafe fn sys(n: u64, a0: u64, a1: u64) -> u64`
      (`asm!("mov x8, {n}; mov x0, {a0}; mov x1, {a1}; svc #0", ...)`);
    - a `#[unsafe(no_mangle)] pub extern "C" fn start() -> !` that: calls
      `SYSMAPMMIO` (20) to map the PL011 (`0xfe201000`) into a server-chosen
      VA, writes `'A'` to the PL011 DR (offset 0x00), and exits 0
      (`sys(0, 0, 0)`). The same behavior the hand-assembled `SERVER_TEXT`
      has today, written in Rust.
  - The PL011 address (`0xfe201000`) is a BCM2711 constant (the DT confirms it
    on QEMU raspi4b; see `console-mmapmmio`). The server-chosen MMIO VA must
    sit in the user half, above the image's segments and stack (a stated
    convention, non-overlapping).
- **xtask `ServerStep`** (`xtask/src/main.rs`): build the server for an arch
  into a stable path:
  - `cargo build -p servers/console --target lib/<arch>.json -Z
    build-std=core -Z json-target-spec` with `RUSTFLAGS = "-C
    relocation-model=static -C link-arg=--image-base=<ELF_BASE>
    -C link-arg=-e<start>"` (the flags the plan's probe verified produce a
    non-PIE `ET_EXEC` with zero relocations).
  - Output: `target/<spec>/<profile>/console.elf`. (The `--image-base`
    `ELF_BASE` matches the loader's constant in task 2; keep them in sync —
    ideally one shared constant xtask reads and the loader uses, so they
    cannot drift.)
  - Wired into the steps that build server-embedding images: `integration-test`
    (before compiling each image), `qemu --image` (before compiling the named
    image), and `check` / `clippy` (before their per-test `--features
    qemu-test` passes). Cargo's own mtime caching makes a re-run with an
    unchanged server a no-op.
- The `start` entry symbol name is a convention (matches the `-e<start>`
  link flag). If a per-arch link script is preferred over `-e`, that is an
  implementation choice; the invariant is "the ELF's `e_entry` is the
  server's `start`."

## Tests

- `cargo xtask check` / `cargo xtask clippy` build the server package
  warning-free for aarch64 (the server is a `no_std` bin; clippy it the way
  the kernel is linted).
- The produced ELF is a non-PIE `ET_EXEC` with the expected `PT_LOAD`s and
  zero relocation sections (a local `readelf`/`objdump` check before
  shipping; the load proof is task 4).

## Acceptance

- `cargo xtask ci` green (the server builds as part of the gate).
- `servers/console` compiles `no_std` for aarch64 with the r9 target spec and
  the static/`--image-base` flags, producing `console.elf`.
- The server is a real Rust program (not hand-assembled bytes).

## Not in scope

A shared `r9-sys` user-space crate (for the arc the shim lives in the server;
when a second server needs the same `sys`/syscall numbers, extract it).
RX (the UART RX IRQ waking the server) — stage 5's refinement, not this arc.
The server's 9P/opcode client interface (stage 7). Cross-arch servers
(x86-64 / riscv64) — when their `Aspace` lands. The build.rs that *consumes*
the ELF (task 4).
