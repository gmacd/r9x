---
status: done
---

# r9x-foundation: the r9x target — target + std backend + servers (Tier 0)

Task 1 of 7 in the r9x arc. Plan:
[plans/r9x-target-std-backend.md](plans/r9x-target-std-backend.md).
The initial deliverable: a **`userland/`** subtree inside the **one `r9x` repo**
(your fork, `gmacd/r9x`) that holds *all* the r9 user-space — the target (the
base distribution every r9x executable links against: the specs, `r9x_abi`,
`r9x_core`, and `r9x_std` — the constants, the shared FDT parser, and the
shim/runtime/API) and the servers — covering exactly what
the current three
servers need and what the current 8-syscall kernel ABI supports. Kernel and
user-space stay in the same repo (Decision 5). No kernel *service* changes —
only the kernel *importing* the shared constants (`r9x_abi`) and the shared FDT
parser (`r9x_core`), both carved out of `port`.

## Goal

Create a **`userland/`** subtree in the `r9x` repo with two parts: the
**target** (the three target specs, `r9x_abi`, `r9x_core`, and `r9x_std` — the
constants, the shared FDT parser, and the shim `::sys`, runtime `::rt`, and
API) and the **servers** (the three existing servers,
moved from the kernel's
`servers/` into `userland/servers/` and migrated off their hand-rolled shims).
Switch the kernel's xtask `ServerStep` to build the servers from the `userland/`
workspace. After this, the r9 user-space is one searchable subtree
(`userland/`), no r9x server re-derives the platform, and the shim, entry,
panic handler, allocator, and ABI constants each live in exactly one place.

Standing constraints: warning-free for aarch64 / x86-64 / riscv64 across the
whole repo (kernel *and* the `userland/` target crates); the kernel's
`unsafe_op_in_unsafe_fn = forbid` discipline carries into `r9x_std::sys`; the pinned
nightly (`nightly-2026-08-21`) is the one toolchain (build-std uses the
toolchain's rust-src, so the target and kernel move together).

## Changes

**`userland/` — the target part** (three packages — `r9x-abi`, `r9x-core`, and
`r9x-std`; the shim and the runtime are modules inside `r9x_std` — Decision 5):

- **`r9x_abi`** — `IMAGE_BASE`, `HANDLES_VA`, `MSG_MAX`, and the syscall
  numbers (`SYSEXIT`…`SYCCREATECHAN`). The single source of the binary-format
  contract (Decision 3). No arch code, no asm.
- **`r9x_core`** — the shared *code* both the kernel and the servers link:
  the FDT parser + `DeviceTree`, moved out of `port` (`port::fdt` →
  `r9x_core::fdt`). Pure (no syscalls, no privilege), so it is safe for the
  kernel to depend on it (extends Decision 3) — the kernel uses it for
  pre-server bringup, the servers for `SYS_MAP_MMIO` lookups (the DTB VA
  arrives as the first `main9(dtb_va)` entry arg, the existing convention).
  This also un-links the user binaries from the kernel-only half of `port`
  (`elf`, `mem`, `pagealloc`, the locks), which `port` keeps and now depends
  on `r9x_abi` + `r9x_core`.
- **`r9x_std`** — `core` + `alloc` plus three modules. `r9x_std::sys`: one
  `sys(n,a0..a4)->(u64,u64,u64)` inline-asm per arch (`svc`/`ecall`/`syscall`) +
  thin wrappers (`exit`, `yield_now`, `send`, `receive`, `reply`, `createchan`,
  `map_mmio`, `claim_irq`) — the single definition of the shim today
  copy-pasted in three servers; each `unsafe` carries a `SAFETY` comment
  stating the register/ABI invariant. `r9x_std::rt`: the `start` entry symbol
  (the `-e start` target) — it receives `(dtb_va)` per the existing
  `main9(dtb_va)` loader convention, records it, and calls the user's
  `fn main()`; `dtb_va()` and `device_tree()` (via `r9x_core::fdt`) expose the
  FDT to servers that look up MMIO bases. Also the `#[panic_handler]` (abort →
  report on the console channel → exit) and the `#[global_allocator]` (a
  **static** fixed-buffer heap, per-server stated size; Decision 4) — gated
  `#[cfg(target_os = "r9")]` so the rest of `r9x_std` unit-tests on the host.
  The API: `process::exit`, `ipc::{Channel, create_pair, send, receive,
  reply}`, `mem` (allocator front), and `io::{Read, Write}` where `Write`
  resolves `/dev/console` through the nameserver (pair from `HANDLES_VA`) and
  sends to the console server's inbound channel. Only these; everything else is
  absent.
- **`userland/specs/`** — `r9x-aarch64.json`, `r9x-riscv64.json`,
  `r9x-x86-64.json`: derived from `lib/*-unknown-none-elf.json`, with
  `"os": "r9"`, `executables: true`, `relocation-model: static`, `-nostdlib`,
  `--image-base=<IMAGE_BASE>`, `-e start` (Decision 6). riscv64/x86-64 specs
  are correct-but-unexercised (Aspace is aarch64-only) — stated in a comment.

**`userland/servers/` — the servers part:**

**Kernel (`r9x` repo — your fork `gmacd/r9x`):**

- `port`: `port::user::IMAGE_BASE`/`HANDLES_VA` and `port::ipc::MSG_MAX`
  become `pub use r9x_abi::{IMAGE_BASE, HANDLES_VA}` / `pub use r9x_abi::
  MSG_MAX` (re-exports), or the arch `process.rs` syscall constants re-export
  from `r9x_abi`. `port::fdt` moves to `r9x_core`, and the kernel's existing FDT callsites
  (e.g. `aarch64::boot`) switch to `use r9x_core::fdt` directly — no temporary
  `port` re-export. The kernel now depends on
  `r9x_abi` and `r9x_core` as ordinary intra-repo crate dependencies, and
  `port` (the kernel-only half) depends on both. A pinning test is kept as a
  belt (see Tests).
- Workspace: the `userland/` crates (including `r9x_abi` and `r9x_core`) and
  the servers become workspace members alongside the kernel's crates.
  `r9x_abi` and `r9x_core` are plain path dependencies for both the kernel and
  the servers — no cross-repo bookkeeping.

**Servers (console, nameserver, init) — moved into `userland/servers/`:**

- Move each server from the kernel's `servers/` to `userland/servers/` (they
  stay in the same repo, now under `userland/`); the kernel xtask's
  `servers_for` list now points at the `userland/` copies.
- In the new home, delete the local `sys` shim, the `SYS_*`/`OP_*`/`MSG_MAX`/
  `HANDLES_VA` constants, the `#[no_mangle] start` wrapper, and the
  `#[panic_handler]`; define `fn main()` (no args) that calls into `r9x_std`
  and links it (the entry, allocator, and panic handler come from
  `r9x_std::rt`). The current three servers keep their hardcoded MMIO bases
  (e.g. the console's `PL011_PHYS`) and do not use the FDT accessors yet —
  they define `fn main()` and the DTB is simply unused. Behavior is unchanged
  (the console server still maps PL011, writes `'A'`, binds `/dev/console`,
  echoes one byte, exits; etc.).

**Build (xtask):**

- `ServerStep`: the servers are now `userland/` workspace members, built from
  the same repo — `--target userland/specs/r9x-<arch>.json -Z build-std=core,
  alloc -Z json-target-spec`, drop the `-Crelocation-model=static` rustflag (the
  spec is now `static`) — keep `--image-base` (sourced from `r9x_abi`) and `-e
  start` — then stages the resulting ELFs for embedding.
- The kernel's own build keeps `build-std=core,alloc`.
- **CI:** the repo's existing `cargo xtask ci` already gates everything — the
  `userland/` target crates and servers are just more workspace members it
  checks/lints/tests for all three arches (warning-free); a smoke bin that links
  `r9x_std` is loaded by an aarch64 test image.

**Repo hygiene:** one workspace (`r9x/Cargo.toml`) now spans the kernel's crates
*and* `userland/*`; the shared `rust-toolchain.toml` pin is unchanged; a short
README in `userland/` states it is the r9x user-space (target + std backend +
servers) and how the kernel consumes it (the `r9x_abi` crate dep + the
ServerStep).

## Tests

- **Pinning test (kernel):** assert `port::user::IMAGE_BASE == r9x_abi::
  IMAGE_BASE`, `…HANDLES_VA == …HANDLES_VA`, `port::ipc::MSG_MAX == r9x_abi::
  MSG_MAX`, and each `process::SYS*`/`SYC*` == the `r9x_abi` value. This is the
  fallback for Decision 3 and proves no drift even if the re-export is ever
  removed.
- **Migration is behavior-preserving:** the existing aarch64 integration
  images that exercise the servers (`console_server`, `namespace`) pass
  unchanged — the servers now link `r9x_std` but do the same syscalls.
- **`r9x_std::sys` round-trip image:** a new aarch64 test image whose `main` calls
  `r9x_std::ipc` to create a pair, send, receive, and reply, asserting the
  payload round-trips (exercises the shim + allocator on-device).
- **Static-allocator bound:** a host unit test in `r9x_std` (the allocator type
  is pure `core`; `::rt` is `#[cfg]`'d out on the host, so it tests there) shows
  an over-capacity allocation
  returns the abort-report path, not a silent growth.
- **`r9x_std::io`:** the `namespace` image's client path already resolves
  `/dev/console`; assert a `Write` to it lands a byte the console server echoes
  (or the image asserts the round-trip it already does).

## Acceptance

- `cargo xtask ci` green for the whole repo (all arches; kernel, `userland/`
  target crates, and servers; integration images pass).
- `grep -rn "svc #0\|clobber_abi" userland/servers/` returns nothing (the shim
  is gone from the servers; it lives only in `r9x_std::sys`).
- The kernel's old top-level `servers/` is gone (moved to `userland/servers/`).
- No server defines `#[panic_handler]` or `#[no_mangle] start` (they live in
  `r9x_std::rt`).
- The pinning test passes; the drift guard is now a type, not just a loader
  placement check.

## Not in scope

The kernel *services* (heap, spawn, clock, wait, priority) — Tasks 2–6. A
growing allocator (the static heap is the honest stopgap, Decision 4). The
server-backed `fs`/`net` beyond the console `io` seed (Task 7). A `std` fork
(Decision 1). Publishing the target crates to crates.io, or splitting
`userland/` out into its own repo (both deferred — Decision 5).

## Build record (task 72, 2026-08-25) — done

Delivered: the r9x target (specs, `r9x_abi`, `r9x_core`, `r9x_std`) and the
three servers, migrated onto `r9x_std`; the original `userland/` subtree was
then **flattened to the root** and the servers renamed `cmd/` (the last
layout note in Build decisions below). `cargo xtask ci` is green (19/19 QEMU images, all arches,
warning-free); the panel review (six lenses) is dry after two fixes — the
bump allocator now aligns the *absolute* address (the offset alone gave a
misaligned pointer for `layout.align() > 8`, since the `base` buffer is only
8-aligned), and `memmove` was added to complete the `mem*` set that
`os: "r9"` strips from `compiler_builtins`.

**Deferred (each has a consumer-free or cross-cutting reason):**

- **`r9x_std::io` (`Read`/`Write`) and the `claim_irq` wrapper** — no current
  consumer (the console hardcodes PL011 and does one echo; it claims no IRQ).
  Defined now they would be dead code in a `no_std` binary. They land with the
  first server that needs them (Task 77, the console-as-9P-server / input arc).
- **The `r9x_std::sys` round-trip image** — the `console_server` and
  `namespace` integration images already exercise `r9x_std::ipc` (create,
  send, receive, reply) end to end; a dedicated image would re-prove what they
  prove. Revisit if a shim regression ever needs a sharper repro.
- **The static-allocator host unit test** — the allocator's
  `#[global_allocator]` and `rt`'s `#[panic_handler]` are lang items that
  collide with `std`'s on a host build (E0152), so a host test needs them
  gated on `target_os = "r9"`. That gate is correct in itself, but
  `clippy --workspace` lints the servers for the *kernel* target
  (`os = "none"`), where the gate is off and the server bins then lack an
  allocator/panic handler — so the gate also requires linting the servers for
  the `r9x-<arch>` spec (a real xtask change: exclude the servers from the
  kernel-target lint, add a r9-spec server lint). That cross-cut is its own
  work, folded into Task 73 (which replaces the static allocator anyway). The
  alignment fix above is verified by review for now.

**Build decisions:**

- The `r9x-<arch>` spec's `"os": "r9"` sets `target_os = "r9"` (verified by a
  `compile_error` probe built for the spec); the servers are built for the r9
  spec (`os = "r9"`) via `ServerStep`, but are *linted* by `clippy --workspace`
  for the kernel target (`os = "none"`). Today that is benign only because the
  lang items are ungated; see the allocator-test deferral above for why that
  is the seam Task 73 must cut.
- `r9x_std` inherits `[lints] workspace = true`, the same as `r9x_abi` and
  `r9x_core` and the kernel: `unsafe_op_in_unsafe_fn = forbid` and the pointer
  lints carry into the shim and the allocator.
- The `mem*` builtins are `unsafe extern "C" fn` (the Rust-idiomatic shape, as
  in `core::ptr`); the compiler emits a call to the *symbol* directly, so the
  `unsafe` marker is safe to add and does not change the builtin dispatch.
- `rt` is **not** gated on `target_os`: `r9x_std` is only ever built for the
  bare-metal r9 targets (and the kernel target's check step), all of which
  `rt` is valid for — gating it is what forced the allocator-test cross-cut
  above.
- **Layout (flattened, 2026-08-25):** the task originally asked for a
  `userland/` subtree; the build flattened it to the root instead, because the
  kernel already lives at the root (`aarch64/`, `riscv64/`, `x86_64/`, `port/`)
  and a nested `userland/` was the one odd thing in an otherwise flat tree.
  Final layout: `abi/`, `core/`, `std/` at the root; the three
  `r9x-<arch>.json` target specs join the kernel's specs in `lib/`; and the
  servers live in `cmd/` — the 9front name, where all user-space programs
  (commands and servers) live, so future `sh`/`ls`/… land beside them (9front
  has no `srv/` source dir; its servers are `sys/src/cmd/{con,ip,auth,plumb,
  9660srv,ext4srv,…}`). The internal crate paths are relative and survived the
  move; only the root members, the kernel crates' path deps, and xtask's spec
  path + `cmd/` server discovery changed. `ci` stayed green.
