# r9x target: a custom Rust target + std backend for r9 user binaries

## Problem and constraints

r9x user binaries (the servers) are today hand-rolled, each from scratch. Every
one is a `#![no_std] #![no_main]` binary that:

- is built by xtask's `ServerStep` against a per-arch JSON target spec kept in
  the kernel repo (`lib/<arch>-unknown-none-elf.json`) with
  `-Z build-std=core -Z json-target-spec`, static, non-PIE, `--image-base`,
  `-e start`;
- copy-pastes the same ~20-line `svc` syscall shim (`SYS_EXIT`,
  `SYCSEND/RECEIVE/REPLY`, `SYCCREATECHAN`, `SYS_MAP_MMIO`, the
  `clobber_abi("C")` inline asm);
- defines its own `#[no_mangle] start()`, its own `#[panic_handler]` (a spin
  loop), and knows its own ABI constants (`HANDLES_VA`, `MSG_MAX`, the
  syscall numbers) mirrored by hand.

That does not scale. The next arc adds many servers (display, 9P fs/dev/net,
the process manager) and they cannot each re-derive the platform.

This plan organises the r9 user-space as a **target** — a Cargo workspace
group of small crates inside the **one `r9x` repo** (your fork, `gmacd/r9x`,
at `/Volumes/Code/r9/r9x`) — that holds the base distribution every r9
executable links against instead of re-deriving it: the target specs, the
syscall shim, the runtime/allocator, and `r9x_std` ("its own std backend"),
**plus the servers** (console, nameserver, init, and every server that
follows). Kernel and user-space live in the **same repo** (Decision 5 —
everything stays in one repo for now). *Naming note:* the **OS** is "r9"
(AGENTS.md); the **fork/repo** is `r9x` (`gmacd/r9x`), forked from upstream
`r9os/r9`; the target crates take the `r9x-` prefix to match the repo. The
target initially covers only
what the current three servers need and what the current 8-syscall kernel ABI
supports. The plan names (a) what the r9x target
implements, (b) what the kernel must add to back a *useful* std, and (c) a
roadmap of target + kernel work ordered by importance to building a real
custom kernel.

Standing constraints: warning-free across aarch64 / x86-64 / riscv64
(`cargo xtask` gates cover the whole repo — the kernel *and* the user-space
target crates — with no warnings on any arch); minimal scoped change;
Plan 9 / QNX shape (the kernel is a message-passing broker, device-dumb); the
Amiga goal (boot to graphics, keep the display at 60 Hz while user-space
servers do everything else).

**The load-bearing finding.** r9's kernel ABI is exactly eight syscalls
(`aarch64/src/process.rs`): `SYSEXIT` (0), `SYSYIELD` (1), `SYCSEND` (16),
`SYCRECEIVE` (17), `SYCREPLY` (18), `SYSIRQCLAIM` (19), `SYS_MAP_MMIO` (20),
`SYCCREATECHAN` (21). That is a QNX-shaped IPC core. It is sufficient to run
the current servers as `no_std` + shim, but it is **far too thin to back a real
`std`**:

- no heap → no growing `alloc` → no `Vec`/`String`/`Box`;
- no spawn → no `std::thread` (r9 processes are single-context today);
- no clock / timed wait → no `std::time`, no sleep, and no way for a display
  server to pace to the 60 Hz vertical blank;
- no file ops → no `std::fs` (but in r9's shape these are user-space 9P
  servers, not kernel syscalls).

So the honest "std backend" is **not** a fork of Rust's `std` (whose PAL
presumes file/proc/net/thread syscalls r9 deliberately lacks). It is a
QNX/Plan-9-shaped base distribution: a thin target + `core`/`alloc` + a
syscall shim + a curated std-shaped layer whose non-memory services are reached
by message-passing to user-space servers. Memory — the one thing that must be a
kernel call because the kernel owns the page tables and the per-process
`Aspace` — is the first kernel addition on the roadmap.

## Prior art

**r9 already has the mechanism.** The build pipeline already does
`cargo build -p <server> --target lib/<arch>.json -Z build-std=core
-Z json-target-spec` with rustflags
`-Crelocation-model=static -Clink-arg=--image-base=<IMAGE_BASE>
-Clink-arg=-estart` (`xtask/src/main.rs`, `ServerStep`), and the kernel itself
builds with `build-std=core,alloc`. r9 already builds `core` and `alloc` from
source for custom JSON specs. What is missing is that this is (i) in the kernel
repo, not a standalone project; (ii) `core`-only for servers (no `alloc`, no
allocator, no runtime); (iii) re-derived per server (the shim, `start`,
`panic_handler`, and ABI constants are copy-pasted).

**The ABI constants are a contract with a drift guard.** `port::user::IMAGE_BASE`
(0x10_0000) and `port::user::HANDLES_VA` (0x100_0000) are read by both the
build (xtask links at `IMAGE_BASE`) and the loader (which rejects any segment
below it) — the two ends read the same constants so they cannot drift.
`port::ipc::MSG_MAX` (256) bounds a message payload and is mirrored by hand in
each server. These are *format* facts (a property of the binary the target
produces), not kernel state.

**Redox** (`redox-os`) is the closest real-world analog — a Rust microkernel
whose file system, network, graphics, and init all run as user-space *agents*,
the same QNX/Plan-9 shape r9 takes — and it is the witness for the one fork
this plan does **not** take. Redox's targets are **built into rustc**
(`x86_64-unknown-redox` Tier 2; aarch64/i586/riscv64 Tier 3) and **`std` is
fully supported**: Redox is *Unix-shaped* (a broad POSIX-like syscall surface),
so `std::sys` maps to `std::sys::unix` and the Redox-specific parts are
`#[cfg(target_os = "redox")]` seams added to Rust's in-tree std — which is why
Redox **builds its own Rust toolchain** (the targets go in `bootstrap.toml`, and
a C library, `relibc`, sits in the linker path). Its layering maps one-to-one
onto this plan: `redox_syscall` (raw numbers + inline asm) ↔ `r9x_std::sys`;
`libredox` (the high-level Rust system library) ↔ `r9x_std`; and "schemes"
(scheme-rooted paths — `fs/…`, `net/…`, everything-is-a-file) ↔ r9's
nameserver + the `/dev/console`-shaped `r9x_std::io`. The one thing r9 has
**no analog for** is `relibc` itself — a POSIX libc over *direct* syscalls.
Redox can afford a real `std` precisely *because* it is Unix-shaped:
`std::sys::unix` has a broad syscall surface to bind to. r9 is the opposite
shape by design — an 8-syscall QNX/IPC core whose fs/net/etc. are user-space
servers reached by message-passing — so there is no surface for std's Unix PAL
to bind to. That asymmetry is the whole reason for Decision 1 (a curated
`no_std core+alloc` facade; **no std fork**; a plain **JSON-spec target** and a
pinned nightly, rather than a self-built toolchain). **Do-not-drift:** a
POSIX-over-IPC shim would be a large Unix-compat layer pulled toward Redox's
shape, not the curated facade — a direction to avoid.

**Plan 9** (`/Volumes/Code/repos/plan9/plan9`) is the shape to copy. `sys/src/
libc` is a thin C library: a per-arch `9syscall`/assembly shim plus a small
portable layer, where `fork`/`exec` are in the libc (`sys/src/libc/9sys/
fork.c`), memory growth is a kernel service the libc calls (`sbrk` → the
kernel's `sysbrk`), and everything else — the file system, the network, the
window system — is a **server** the libc reaches through `sys/src/lib9p`
(`auth.c`, `fid.c`, `file.c`, `dirread.c` …), the 9P client library. The libc
does not implement a VFS or a network stack; it talks to servers over 9P.

**Linux** (`/Volumes/Code/repos/linux`) is the witness for what the kernel must
supply: the heap is a kernel service (`mm/mmap.c` — `brk`/`mmap`), and
process creation is `fork`/`clone`/`execve`. r9's equivalents (`sys_alloc`,
`sys_spawn`) are the same kernel-resident services; the *rest* of Linux's
syscall surface (open/read/write/mmap-of-files, sockets) is exactly the part
r9 pushes to servers and does **not** put in the kernel.

**What will be composed, not built:** the `core`/`alloc` libraries (built from
the pinned toolchain's `rust-src`, as today); the per-arch target data-layout/
feature/cpu facts (mirrored from the in-repo specs); the ELF user-binary format
and the loader (the user-binary-loading plan — already built). The r9x target
builds *around* these, not over them.

## Hardware assumptions (required)

The r9x target is a binary-format + syscall layer; it makes **no firmware or
board assumptions**. Its assumptions are the machine-model facts the loader and
the trap handler already establish, per arch:

- **aarch64** (Pi 4 / QEMU `raspi4b`): user binaries are non-PIE `ET_EXEC`
  `ELF64`, linked at `IMAGE_BASE`, entered at symbol `start` with the user
  stack set by the loader (per-process `Aspace`, TTBR0). The syscall is
  `svc #0` with the number in `x8`, args in `x0`–`x5`, result in `x0`
  (`aarch64/src/trap.rs`, the `SvcAarch64` arm; ESR_EL1.ISS not used). The
  spec assumes `+strict-align,+neon,+fp-armv8`, `max-atomic-width 128`,
  `disable-redzone`, static relocation.
- **riscv64**: the syscall is the U-mode `ecall`; the spec assumes
  `generic-rv64` with `+m,+a,+f,+d,+c`, `llvm-abiname lp64d`,
  `max-atomic-width 64`. **Status: the server path is not live on riscv64** —
  the per-process `Aspace` the loader needs has only landed for aarch64, so the
  riscv64 spec is correct-but-unexercised, exactly as the in-repo spec is.
  Stated, not hidden.
- **x86-64**: the syscall is `syscall` (number in `RAX`, `RCX`/`R11`
  clobbered by the instruction); the spec assumes soft-float, code-model
  kernel, `disable-redzone`, frame-pointer always. **Status: same as
  riscv64 — spec present, server path not live.**
- **Firmware co-tenancy:** none at the target layer. The Pi's
  VideoCore VI firmware and QEMU's machine are the *kernel's* concerns. The
  only facts the target shares with the kernel are the syscall numbers and the
  register convention — architecture-defined, not firmware-defined — and the
  binary-format constants (`IMAGE_BASE`, `HANDLES_VA`, `MSG_MAX`), which are
  stated conventions, not hardware facts.
- **Memory model:** flat, no redzone (aarch64/x86-64), one syscall
  instruction per arch, `panic = abort`. The target assumes the kernel routes
  the arch's user-mode syscall to the dispatch and delivers a fault (not a
  syscall) for any bad memory access in a process — the isolation property the
  trap handler already provides.

The one assumption that *is* arch-load-bearing and worth naming: the syscall
**register convention is r9's own** (Linux-arm64-style: number in `x8`), not
any OS's ABI. The target emits the arch's syscall instruction with r9's
convention; there is no existing OS whose PAL can be reused, which is one of
the two reasons the std-fork candidate (below) loses.

## Design

### Data structures

The central data structure is **the binary-format contract**, made a single
source of truth. Everything else is thin and per-arch. The `r9x` repo holds
two halves: the **target** (the crates below) and the **servers** (its first
consumers); the crate list is the target half.

- **`r9x_abi`** (a tiny crate): `IMAGE_BASE`, `HANDLES_VA`, `MSG_MAX`, and the
  syscall numbers, carved out of `port::user`/`port::ipc`. It is the *one place*
  the format facts live. Both the kernel
  (`port`) and the target depend on it, so the build, the loader, and the
  servers all read the same constants and cannot drift (today the drift guard
  is a loader placement check; this makes the guard a type).
- **`r9x_core`** (shared *code* — pure, no syscalls, no privilege): the FDT
  parser and its `DeviceTree` type, moved out of `port` (`port::fdt` →
  `r9x_core::fdt`) so **both the kernel (pre-server bringup) and the servers
  (MMIO-map lookups) link one copy**. Both the kernel and the target depend on
  it, exactly like `r9x_abi` but for logic rather than constants. It is the
  shared half of the existing `port` crate carved out; the kernel-only half
  (`elf`, `mem`, `pagealloc`, the locks) stays in `port`, which then depends on
  `r9x_abi` + `r9x_core`. (Today the user test images already link all of
  `port` just to use `fdt`/`println`; after the split they link `r9x_core` +
  `r9x_std`, never `port`. The servers need no new mechanism to reach the FDT
  bytes: the DTB is mapped RO into user space and its VA is the first
  `main9(dtb_va)` entry argument — the existing convention.)
- **`r9x_std`** (the "std backend" proper — the user-space half): `core` +
  `alloc` plus, as modules, the **shim** (`r9x_std::sys` — one `sys()` inline
  asm per arch, `svc`/`ecall`/`syscall`, the single definition of the shim
  today copy-pasted into three servers), the **runtime** (`r9x_std::rt` — the
  `start` entry symbol (the `-e start` target); it receives `(dtb_va)` per the
  existing `main9(dtb_va)` loader convention, records it, and calls the
  user's `fn main()`; it also exposes `dtb_va()` and `device_tree()` (parsed
  via `r9x_core::fdt`) for servers that look up MMIO bases), the
  `#[panic_handler]` (abort → report on the console channel → exit), and the
  `#[global_allocator]` (a **static** fixed-buffer heap for the initial slice;
  the kernel-backed heap replaces it in Tier 1.1)), and the **curated std-shaped API** (`process`/
  `ipc`/`mem`/`io`). The API surface is *exactly* what the current servers use
  and the current kernel supports; every other std item is **absent, not
  stubbed**. `r9x_std::rt` is gated `#[cfg(target_os = "r9")]` so the rest of
  `r9x_std` still unit-tests on the host (the `#[panic_handler]` /
  `#[global_allocator]` are target-only). Adding an API item is a roadmap item
  gated on the kernel/server support.

Who owns what state: the target owns the *format* (the specs, `r9x_abi`, the
syscall convention). The kernel owns the *services* (the syscall
implementations). The servers own their *data* (in their own `Aspace`). No fact
is stored twice once `r9x_abi` exists.

### Interfaces

Public surface, by module. Three crates: `r9x_abi`, `r9x_core`, and
`r9x_std`. Day-one users: the three existing servers (console, nameserver,
init), which migrate from hand-rolled shims to these.

- `r9x_abi`: the constants (above).
- `r9x_core`: `fdt` — the FDT parser and `DeviceTree`, used by the kernel and
  by servers making `SYS_MAP_MMIO` requests (shared code; see Data
  structures).
- `r9x_std`:
  - `r9x_std::sys` — `fn sys(n, a0..a4) -> (u64,u64,u64)`; `exit`,
    `yield_now`, `send`, `receive`, `reply`, `createchan`, `map_mmio`,
    `claim_irq` (the per-arch shim; `no_std`).
  - `r9x_std::rt` — the entry trampoline, the panic handler, the global
    allocator, and the DTB accessors. The `start` entry receives `(dtb_va)`
    (the existing loader convention) and calls the user's `fn main()` (no
    args); `dtb_va() -> usize` and
    `device_tree() -> r9x_core::fdt::DeviceTree` let a server reach the FDT
    (e.g. for `SYS_MAP_MMIO` lookups). A server stops defining `start` and
    `panic_handler`; it defines `main` and links `r9x_std`. Gated
    `#[cfg(target_os = "r9")]`.
  - `r9x_std::process::exit`
  - `r9x_std::ipc::{Channel, create_pair, send, receive, reply}` — the message
    primitives, the QNX/Amiga port made a Rust type.
  - `r9x_std::mem` — the allocator front (static for now).
  - `r9x_std::io::{Read, Write}` — **the first server-backed API**: a `Write`
    that resolves `/dev/console` through the nameserver (whose pair the
    spawner passes at `HANDLES_VA`, exactly as the console server already
    does) and `send`s bytes to the console server's inbound channel. Supportable
    with today's ABI (pure send/receive); it is the seed of the server-backed
    tier, not a requirement of the current servers.

The shape rule (Oberon's graft): every `r9x_std` item is either (a) a thin call
into one of the eight syscalls, or (b) a message to one named user-space
server. Anything that is neither is not in `r9x_std`.

### Init and bringup order

1. **Target project exists** (Tier 0): `r9x_abi`, `r9x_std` (shim `::sys`,
   runtime `::rt` (static allocator), and `process`/`ipc`/`mem`/`io`), the
   three specs.
2. **Servers move + migrate**: the three servers move from the kernel's
   `servers/` into the target's `servers/` (still in the `r9x` repo); each
   drops its local shim, `start`, `panic_handler`, and mirrored constants; links `r9x_std`. The kernel's
   xtask `ServerStep` no longer builds them
   as top-level kernel members — it builds them as target workspace members
   (against the target specs, `build-std=core,alloc`) and embeds the resulting
   ELFs.
   Load-bearing: `r9x_abi` is what the *kernel* now imports for
   `IMAGE_BASE`/`HANDLES_VA`/`MSG_MAX`, so the kernel build and the server build
   read one source (now a plain intra-repo crate dependency, not a cross-repo
   one).
3. **Inside `r9x_std`, the order that matters:** the shim (`::sys`) and the
   runtime (`::rt` — entry + allocator) must exist before any server can link
   (a server with no `::rt` has no entry and no allocator); the API modules
   build on `::sys`. So: `r9x_abi` first (the kernel depends on it), then
   `r9x_std::sys`, then `r9x_std::rt`, then the API, then the migration.

### Failure policy

- **`r9x_std::sys`** returns `Result`-shaped values (a syscall's error code), never
  panics on a *return*; the only `unsafe` is the syscall asm, and each carries
  a `SAFETY` comment stating the register/ABI invariant (mirroring the
  discipline already in the servers).
- **`r9x_std::rt` allocator (static):** an over-size or exhausted allocation is an
  init-only condition for the current servers (they fit in the static heap);
  the handler reports it on the console channel and exits (the kernel's
  process-fault path is the backstop). It does **not** silently grow.
- **`r9x_std::io`:** a resolve failure (nameserver absent) degrades to a
  no-op write with a one-time report — a server that cannot reach the console
  still runs its real job.
- The `panic = abort` strategy and the abort-report-then-exit panic handler are
  the uniform policy: a panic in a server is a fatal, reported event, not
  error handling.

## Not building

- **A fork of Rust's `std`** and the `sys/r9x` PAL it would need (see
  Decision 1).
- **Kernel-resident** file system, network stack, window manager, or
  framebuffer — those are user-space servers (the Plan 9 / QNX / Amiga shape);
  the kernel stays a message-passing broker.
- **In-process user threads (M:N).** Threads are processes (Decision 2).
- **A growing heap before the kernel provides one** — the static heap is a
  named, scoped stopgap (Decision 4).
- **A dynamic linker, PIE user binaries, or shared libraries.** r9x binaries
  are static `ET_EXEC`, embedded in the image, loaded with no file access by
  the kernel (the user-binary-loading plan).
- **A general-purpose libc.** `r9x_std` is curated; features are added one at a
  time, each gated on the kernel/server support that backs it.
- **Any kernel syscall that is not a service the kernel already owns**
  (memory, process, scheduling, time). File/net are servers, not syscalls.

## Decision records

**Decision 1 — Curated `r9x_std` (no_std `core`+`alloc`), not a `std` fork.**
- **Chosen:** a curated `r9x_std` on `core`+`alloc`; the "std backend" is this
  curated layer + the runtime, not a build of `std` itself.
- **Alternatives:** (a) fork `std`, add a `library/std/src/sys/r9x/` PAL,
  `build-std=core,alloc,std`. Loses: you own a fast-moving ~500K-LOC fork; you
  must implement or stub every PAL trait (fs, thread, net, process, time, env,
  rand, condvar, …); most are unimplementable or *wrong* against a QNX/Plan-9
  microkernel (there are no file/proc/net/thread syscalls by design); you lag
  upstream and fight std's shape for years. (b) `build-std` with `os` set to an
  existing OS (e.g. `"linux"`): rejected — the PAL would issue Linux's
  syscalls (numbers + `syscall` mechanism) against r9's `svc`-based ABI; a
  silent lie.
- **Dissent:** the kernel-taste lens (abstraction is earned by the second user; a
  hand-rolled `r9x_std` is "reimplementing std") and the plain reading of "its
  own std backend" both push toward real `std`. We chose the curated layer
  because r9's ABI is a *subset* of what std's PAL presumes, so most of std's
  surface is unbackable; a curated layer is the shape that matches the kernel.
  The word "std" is read as "the standard base library r9x binaries link
  against," not "the `std` crate." Recorded.

**Decision 2 — Threads are processes; no in-process user threads.**
- **Chosen:** `r9x_std::thread::spawn` (when it lands) maps to
  spawn-a-process; r9x processes stay single-context (one kernel stack, one
  EL0 context), as today.
- **Alternatives:** an M:N green-thread runtime on one OS context. Loses: a
  process has one kernel stack (`KSTACK_SZ`, 16 pages) and one context; the
  preempt/fault model (one TPIDR slot, one trap frame) is built for one
  context; and it collides with the QNX doctrine (determinism from many small
  processes, not many threads on one).
- **Dissent:** the Amiga/real-time lens wants cheap concurrency; the whole-system lens
  likes late binding. We chose processes because r9's scheduler, fault
  isolation, and determinism story are all per-process, and QNX — r9's named
  model — does exactly this. Recorded.

**Decision 3 — ABI constants live in a neutral `r9x_abi` crate, depended on by
both the kernel and the target.**
- **Chosen:** `r9x_abi` (a crate in the `r9x` repo) owns `IMAGE_BASE`, `HANDLES_VA`,
  `MSG_MAX`, the syscall numbers; the kernel's `port` and the target/servers
  both depend on it.
- **Alternatives:** (a) the kernel keeps owning them in `port::user`/`port::
  ipc` and the target mirrors (status quo; drift guarded only by the loader
  placement check). (b) the target owns them and the kernel imports a
  "user-space" crate (a layering inversion).
- **Dissent:** the microkernel/kernel-residence lens is uncomfortable
  with the kernel depending on a crate that also serves user-space (it muddies
  the trust boundary). We accept it because the constants are *format* facts,
  not kernel state, and a neutral crate is the cleanest no-drift option.
  Fallback if the layering is rejected: a pinning test
  (`port::user::IMAGE_BASE == r9x_abi::IMAGE_BASE`, etc.) with the kernel
  keeping ownership. Recorded.

**Decision 4 — Static allocator now, kernel-backed heap later.**
- **Chosen:** `r9x_std::rt` ships a static (fixed-buffer) global allocator for
  the initial slice; the kernel-backed heap (Tier 1.1) replaces it. The static
  heap size is a per-server stated constant (like the current 64 KiB stack).
- **Alternatives:** (a) require the heap syscall *before* the target exists
  (blocks Tier 0 on a kernel change). (b) a bump-only allocator (no free →
  leaky for a long-running server).
- **Dissent:** the whole-system/lean lens says do not ship a stopgap you know you will
  replace. We ship it because the current servers allocate nothing, so the
  static heap is *honest today* (they fit in it), and it unblocks the target
  without a kernel change; the replacement is one well-scoped task (Tier 1.1)
  that also grows the kernel. Recorded.

**Decision 5 — One repo for now: kernel + user-space target + servers in `r9x`.**
- **Chosen:** everything stays in the **one `r9x` repo** (your fork,
  `gmacd/r9x`, at `/Volumes/Code/r9/r9x`). The user-space target is a **Cargo
  workspace group** of small crates inside it (the specs, `r9x_abi`, and
  `r9x_std` — shim + runtime + API as modules); the servers live beside it
  (re-pointed at `r9x_std`); the kernel depends on `r9x_abi` as an ordinary
  intra-repo crate;
  xtask builds the server ELFs from the same workspace. One repo, one CI, one
  toolchain pin, atomic ABI bumps.
- **Why it changed:** this was previously "two repos" (kernel + a separate
  `r9x-userland`). That split was justified by the assumption that "the std
  backend" meant a **clone-and-patch of Rust** — a big, separate, fast-moving
  tree best kept out of the kernel. That assumption is **wrong** (see the
  Prior-art Redox note and the earlier discussion): the "target" is only a few
  dozen-line JSON specs plus small crates on top of `build-std=core,alloc`, not
  a Rust fork. With nothing fork-sized to isolate, the thin-seam that made the
  split cheap (the kernel depends only on `r9x_abi`) no longer carries the
  decision, and one repo wins on the practical points: one tree, one CI, one
  nightly pin, an atomic ABI bump is one commit, and cross-repo git/path deps
  and lockstep versioning disappear.
- **Searchability (the original objection, settled by the layout):** keeping
  the target + servers in the kernel repo makes the user-space less greppable as
  a standalone unit. Settled in favour of a **flat root** (not a `userland/`
  subtree): the kernel already lives at the root (`aarch64/`, `riscv64/`,
  `x86_64/`, `port/`), so a nested `userland/` was the one odd thing in an
  otherwise flat tree. The user-space crates sit at the root (`abi/`, `core/`,
  `std/`), the target specs join the kernel's in `lib/`, and the servers live in
  `cmd/` — the 9front name, where *all* user-space programs (commands **and**
  servers) live, so the future `sh`/`ls`/… commands land beside them. The crate
  names (`r9x_*` libs; `cmd/*` servers) keep the user-space greppable without a
  subtree. **Splitting it out later is still a pure, low-risk move** (the
  `r9x_abi` boundary already exists) — so choosing flat-now costs nothing.
- **Crate naming (settled here):** one prefix, one family. Three crates:
  `r9x-abi`, `r9x-core`, and `r9x-std` (Cargo *package* names, hyphens); you
  `use` them in code by their *library* names, the same with underscores:
  `r9x_abi`, `r9x_core`, and `r9x_std`. The shim and the runtime are **modules
  inside `r9x_std`** (`r9x_std::sys`, `r9x_std::rt`), not separate crates.
  Naming `r9x-core` (not e.g. `r9x-common`) is deliberate and safe: it's a
  distinct identifier from Rust's built-in `core` (no `use` collision), and
  since r9 ships its *own* std, "core" aptly names the shared foundation both
  the kernel and user-space link. This doc writes the underscore (lib) form in
  body text and code paths.
- **Granularity (settled): three crates — `r9x-abi` + `r9x-core` +
  `r9x-std`.** `r9x_abi` (constants) and `r9x_core` (shared code, FDT first)
  are forced separate — the kernel imports both and they must not pull in the
  rest. The shim and the runtime are modules inside `r9x_std` (`::sys`,
  `::rt`). The one real reason to *keep* the runtime separate was
  host-testability: the `#[panic_handler]` / `#[global_allocator]` can't
  compile on a host (std) target, but `r9x_std` must also unit-test on the
  host (the allocator is pure `core`). Folding works because `::rt` is gated
  `#[cfg(target_os = "r9")]`, so those attributes never appear in the host
  build. Recorded: three crates; if a second user for the raw shim, or a
  binary that wants the API without the runtime, ever appears, split
  `::sys`/`::rt` out — cheap and local.

**Decision 6 — Spec `os` field and relocation model.**
- **Chosen:** the target specs set `"os": "r9"` (the OS name, per AGENTS.md —
  not the fork name `r9x`; forward-looking so a future real-std PAL could key
  on it, harmless for `build-std=core,alloc`) and
  `relocation-model: static` directly, so the server build no longer needs the
  `-Crelocation-model=static` rustflag that today patches over the in-repo
  specs' `pie`.
- **Alternatives:** keep `os` unset / `pie` + override (status quo; the override
  is a smell). Low contention.

**Decision 7 — Roadmap order: heap → spawn → clock/timed-wait.**
- **Chosen:** Tier 1.1 heap, then Tier 1.2 spawn, then Tier 2.1
  clock/timed-wait.
- **Dissent:** the Amiga lens argues clock/timed-wait is #2, because the
  standing 60 Hz graphics goal needs the display server to pace to the vertical
  blank, and that is the kernel's load-bearing real-time duty. For the
  *graphics* track, clock and spawn are load-bearing together. For a *general*
  custom kernel, spawn (multi-process) is the larger capability unlock, and a
  single-process display server is already expressible (the console server is
  one process) — the clock is what you add to make it *paced*. We ordered
  spawn #2 on the general-kernel reading, with the Amiga dissent recorded so
  the graphics track can pull clock forward. Recorded.

## Tasks

Sequenced; the critical path is 1 → 2 → 3. 4–7 are the roadmap, each gated on
the one before it (or on a server landing).

1. `r9x-foundation.md` — **Tier 0.** Create the target workspace in the `r9x`
   repo: the three specs, `r9x_abi`, and `r9x_std` (shim `::sys`, runtime
   `::rt` (static allocator), `process`/`ipc`/`mem`/`io`); move the three
   servers under it and migrate
   them off their hand-rolled shims; switch xtask's `ServerStep` to build them
   as target workspace members. *Near-term; the
   initial deliverable the user asked for.*
2. `r9-syscall-heap.md` — **Tier 1.1.** Kernel `sys_alloc`/`sys_free` (the
   heap); replace `r9x_std::rt`'s static allocator with the kernel-backed one.
   *Unblocks every server that buffers.*
3. `r9-syscall-spawn.md` — **Tier 1.2.** Kernel `sys_spawn` + the embedded
   image registry; `r9x_std::thread` (threads-as-processes). *Unblocks the
   process manager (stage 7) and the display server as a process.*
4. `r9-syscall-clock-wait.md` — **Tier 2.1.** Kernel `sys_clock` +
   `sys_nanosleep` / receive-with-deadline; `r9x_std::time`. *Unblocks the 60 Hz
   vblank pacing (Amiga heartbeat) and `sleep`.*
5. `r9-syscall-proc-control.md` — **Tier 3.1.** Kernel `sys_wait` (reap) +
   `sys_kill`; `r9x_std::process` completion. *Unblocks the process manager
   actually managing (detect + restart dead servers).*
6. `r9-syscall-sched.md` — **Tier 3.2.** Kernel `sys_setprio` + priority
   inheritance. *Unbounds real-time priority for display/input.*
7. `r9x-std-servers.md` — **Tier 4.** The server-backed `r9x_std` surface
   (`fs`, `net`, `io` against real 9P servers), added as each server lands.
   *Gated on the fs/dev/net servers; no kernel syscall — pure target side.*
