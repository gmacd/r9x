# r9 user binaries: built, embedded, and loaded without a filesystem

## Problem and constraints

The microkernel substrate's next step is real user-space servers — the console
server first, then the display server, the nameserver, the 9P servers. Today
the only "user programs" in the tree are hand-assembled machine-code byte
arrays embedded in test source and `main.rs` (`SERVER_TEXT`,
`FIRST_PROCESS_TEXT`, `PROG_A`, …). That does not scale: a server with a
driver loop and IPC is hundreds of instructions and cannot be maintained as
raw `MOVZ` bytes.

This plan adds the machinery to:

1. build servers (the console server first) as **separate Rust executables**
   in r9's user-binary format;
2. **embed** those binaries into the kernel's or a test image's executable so
   they load at runtime **with no file access by the kernel**; and
3. have the build system **rebuild the embedding image whenever the server
   binary changes** (the dependency the user asked for).

Standing constraints: warning-free across aarch64 / x86-64 / riscv64
(`cargo xtask` gates); minimal scoped change; Plan 9 shape; aarch64 is the
reference implementation. The display-server 60 Hz goal is downstream — this
arc is the *loading machinery*, not the display server.

The user was explicit that the **simple raw-code images are fine as-is**
(`user_process`, `user_yield`, `two_yield`, `ipc`, `aspace`, …): their program
*bytes* stay hand-assembled — they are not rewritten to ELFs.

The process entry point is **unified under one image type**. Rather than keep
`spawn(&[u8], text_va, stack_va)` and add a second `spawn_elf`, this arc
introduces a single `process::spawn(&Image)` over an `Image` enum
(`Image::Raw { text, text_va, stack_va }` for the raw-code images, and
`Image::Elf(&[u8])` for the servers). This is an **early call** — the
uniform-metaphor move the panel would otherwise have deferred until a second
server existed (decision 2) — taken now at the user's direction. The cost is a
mechanical sweep: the ~19 existing raw call sites (the 9 raw test images and
`main.rs`) become `Image::Raw { … }`. The *programs* are untouched; only the
`spawn` call changes shape.

**The cost, stated up front.** The console server is the first *user* of this
machinery, not its whole justification. The user asked for "a way to build the
console server **(and others)**," and the substrate end-state names many
servers (display, nameserver, the 9P fs/dev/net servers). The parser, loader,
and build machinery are paid **once** and amortized over every later server;
the first server is their *proof*, not their entire case. That is the honest
tension (paying for the general mechanism now, for the many-servers-later),
and it is why the arc is sized to the machinery, not to the one server.

## Prior art

**r9 already has the load-bearing 80%.** Per-process `Aspace` (stage 3) gives
each process an isolated TTBR0. `Entry::rw_user_text` / `Entry::rw_user_data`
already model the executable-text vs. read-write-data split a real binary
needs. `forkret_context` already fabricates an entry context from an ELR and a
user SP. The build pipeline (xtask → `cargo build-std` against a JSON target
spec → ELF → objcopy → QEMU) already emits ELFs; the kernel itself is a
fixed-address ELF (`kernel.ld`, `KZERO`). What is missing is a *reader* for a
user ELF and a *loader* that maps its segments into a process.

**The format fact, verified on the pinned nightly** (rustc 1.100.0-nightly):
a Rust bin targeting the `-unknown-none-elf` spec with
`-C relocation-model=static -C link-arg=--image-base=<base>` is a **non-PIE
`ET_EXEC` ELF with per-segment `R-X` / `R--` / `RW-` `PT_LOAD`s, a real
`e_entry`, and zero relocation sections**. So the loader is exactly:
*map each `PT_LOAD` at `p_vaddr`, copy `p_filesz` bytes, zero
`p_memsz − p_filesz`, jump to `e_entry`* — **no relocation processing**.

**Plan 9** — `exec()` reads an ELF, maps each `PT_LOAD` with the segment's
R/W/X flags, copies `p_filesz`, zeroes the bss remainder, and jumps to
`e_entry`. For a statically-linked binary there are no relocations. r9's
loader is this, with the file replaced by an embedded `&[u8]`.

**Linux** — `fs/binfmt_elf.c` (`load_elf_phdrs` / `elf_map`) is the same
`PT_LOAD` loop, with the byte copy behind a `filp`. r9 is that loop with the
`filp` swapped for an embedded slice. (This is the essential part; the rest of
`binfmt_elf.c` — dynamic linking, `PT_INTERP`, vma accounting — is accreted and
not wanted.)

**QNX** — a resource manager is a user process, built as a separate executable
and loaded by the kernel; the kernel is device-dumb. "The server is a separate
executable the kernel loads" is the QNX shape r9 already follows at the
syscall layer; this plan extends it to the *binary* layer.

**Composed, not built:** the ELF is produced by the existing rustc + rust-lld
toolchain (no new tool); the loader is Plan 9's / Linux's `PT_LOAD` loop; the
rebuild-on-change is cargo's own mtime tracking (`include_bytes!` / a
`build.rs` `rerun-if-changed`).

## Hardware assumptions (required)

- **aarch64 (Pi 4 / QEMU `raspi4b`) — the reference.** Servers are aarch64
  `ET_EXEC` ELFs. The loader maps their `PT_LOAD` segments into the process's
  own TTBR0, which is **isolated per process**, so a fixed image base is valid
  for every process (there is no shared user VA space to collide in). Text
  segments get `Entry::rw_user_text`; data/rodata get `Entry::rw_user_data`.
  The MMIO a server maps via `SYSMAPMMIO` is a separate TTBR0 region the server
  chooses. **No new hardware assumption**: the process / Aspace / TTBR0 /
  switch machinery already exists and is proven by the `aspace`,
  `aspace_fault`, and `ipc` images.
- **x86-64 / riscv64 — gate-green.** The `port::elf` reader is
  arch-agnostic and host-tested on all three arches. The `Image::Elf` arm
  (`spawn_elf`) and the server build are **aarch64-gated** (they need the `Aspace`, which has only
  landed for aarch64). No new assumption; they light up when the arch's
  `Aspace` lands (stage 3 for that arch).
- **Memory ordering.** The loader copies segment bytes with
  `copy_nonoverlapping` (plain memory, not MMIO). The `Aspace::install` already
  does the `TLBI`/`DSB`/`ISB` the switch needs. No new barriers.
- **Firmware.** None consumed. The server bytes are embedded in the image; no
  DT, no tables, no file.

## Design

### Data structures

- **`port::elf`** — a minimal ELF64 reader over `&[u8]`. It returns the entry
  point and the `PT_LOAD` segments as `{ vaddr, filesz, memsz, exec }`.
  Pure, no I/O, no allocation: segments live in a small fixed array and the
  byte ranges borrow from the input slice. Host-testable on every arch (the
  test fixtures are tiny in-memory ELFs built in the test). It follows the
  existing **`port::fdt`** precedent — a pure, host-tested, arch-agnostic
  parser that turns a firmware/image byte blob into validated structure. It
  validates *structure*: the magic, ELF64-ness, at least one `PT_LOAD`, header
  and program-header bounds inside the slice, `filesz ≤ memsz`, and alignment
  sanity. Placement (does a segment land in a legal place?) is arch-specific
  and is the loader's job, below.
- **`aarch64::process::Image`** — the one type `spawn` takes:
  ```rust
  pub enum Image<'a> {
      /// Raw machine code: `text` placed at `text_va`, stack at `stack_va`
      /// (the caller owns the layout — the simple test images).
      Raw { text: &'a [u8], text_va: usize, stack_va: usize },
      /// A self-describing ELF: layout comes from the header; the stack is
      /// derived above the highest segment (the servers).
      Elf(&'a [u8]),
  }
  ```
- **`aarch64::process::spawn(image: &Image) -> ProcessId`** — the single entry
  point: a `match` dispatching `Image::Raw` to the existing raw path (the
  current `spawn` body, renamed `spawn_raw`) and `Image::Elf` to the new
  `spawn_elf`. Both arms are `aarch64`-private; the public surface is `Image`
  + `spawn` (one way to start a process).
- **`spawn_elf(elf: &[u8])`** (the `Image::Elf` arm) — builds an `Aspace`, maps
  each segment, derives the stack, and reuses the existing `forkret_context`
  (which already takes an ELR and a user SP; we feed it `e_entry` and the
  derived stack top). Before mapping it **validates placement**: each segment's
  `p_vaddr` is page-aligned, is in the user (TTBR0) half (`< KZERO`), and the
  segments do not overlap. An embedded ELF is still *input* — a malformed or
  mis-linked one must be rejected at `spawn` with a named error, not mapped
  into kernel space or on top of itself.
- **Layout convention** (per-arch constants, all in the TTBR0/user half): a
  page-aligned image base `B`; the stack is `STACK_SZ` pages immediately above
  the highest loaded segment; any MMIO the server maps sits above the stack.
  Non-overlapping by construction, and reusable across processes because each
  TTBR0 is isolated.

### Interfaces

- **`process::spawn(image: &Image) -> ProcessId`** — the single entry point.
  `Image::Raw { … }` for the raw-code images (the current `spawn` body,
  unchanged in behavior); `Image::Elf(&[u8])` for the servers. The old
  `spawn(&[u8], text_va, stack_va)` signature is **retired**; its ~19 call
  sites become `Image::Raw { … }` (task 2's mechanical sweep).
- **`port::elf::parse(elf: &[u8]) -> Result<Elf, ElfError>`** — the shared
  reader. `Elf` carries `entry: u64` and the segment list. Consumed only by the
  `Image::Elf` arm.
- **The embedding.** Each server is a workspace package (`servers/console`).
  xtask builds it to `target/<spec>/<profile>/console.elf`. The image that
  boots it `include_bytes!`s that ELF, staged into `OUT_DIR` by a `build.rs`
  that also declares `rerun-if-changed` on the ELF.
- **The server's own shape.** A `#![no_std] #![no_main]` bin: a `start` entry,
  a tiny `sys(n, a0, a1)` syscall shim (`mov x8, n; svc #0`), a panic handler,
  and the body. `core` plus the shim — no libc.

### Init and bringup order

*Build (host):*

```
xtask ServerStep            build the server (static, --image-base)
                            -> target/<spec>/<profile>/console.elf
image build.rs              stage the ELF into OUT_DIR;
                            cargo:rerun-if-changed=<elf>
image compile               include_bytes!(OUT_DIR/console.elf)
```

Order is *server before image*; cargo's mtime tracking means a changed server
rebuilds the image.

*Boot (target):*

```
boot::… (irq, dt, pagealloc, console, interrupts)   [unchanged]
enable PL011 (kernel's early path)                   [unchanged]
process::spawn(&Image::Elf(CONSOLE_ELF))  map segments + stack into the server's
                                          TTBR0, ELR = e_entry
process::run_all()               server: SYSMAPMMIO (map the PL011 into its own
                                  TTBR0), write a byte, exit 0
process::status(id)              expect 0
```

### Failure policy

- **`port::elf::parse`** returns `Result<Elf, ElfError>`: bad magic, not
  ELF64, no `PT_LOAD`, truncated header/program headers, `filesz > memsz`,
  unaligned segment. Pure and host-tested; every rejection is a named error.
- **`spawn_elf`** placement validation rejects a segment whose `p_vaddr` is not
  page-aligned, is `≥ KZERO` (outside the user half), or overlaps a prior
  segment. A mapping failure (pagealloc exhausted) is the same init-only
  `panic!` the current `spawn` uses — callers are `main9` / the test images.
- **`unsafe` discipline** (the repo's standing constraint: every unsafe op
  spelled out). The loader's `unsafe` blocks each carry a `// SAFETY:` comment,
  mirroring the existing `spawn`: the byte copy is into a page `spawn_elf` just
  mapped as user-writable and the fit (`filesz` within the mapped span) is
  asserted up front; the `Aspace::install` is on a live AS built at `spawn`.
  No new `unsafe` in any path the raw `spawn` does not already use.
- **A server that faults** (e.g. writes outside its segments) is killed by the
  existing EL0 fault path (stage 3); its `Aspace` is isolated, so the kernel
  and peers survive (the `aspace_fault` image already proves this shape).
- **No new panics in interrupt context.** The loader runs at `spawn` time
  (init context), never in the IRQ path.

## Not building

- **A relocation processor.** Static non-PIE ELFs have none. A dynamic or PIE
  server would need one; we refuse it. Servers are statically linked at a
  fixed base — a stated invariant.
- **A filesystem or a `read` syscall.** The bytes are embedded; the kernel does
  no file I/O. (The eventual 9P `exec` reads from a server, not the kernel.)
- **Tightening the user page permissions (non-writable user text; a proper RO
  entry).** The current user page model is `AllRw` for *both* constructors:
  `rw_user_text` is writable **and** executable (a pre-existing W+X, inherited
  from the raw `spawn`, which copies hand-assembled bytes into a
  `rw_user_text` page and leaves it so), and `rw_user_data` is writable + XN.
  The ELF loader **inherits** this model per segment (X → `rw_user_text`, else
  → `rw_user_data`), so it introduces no new looseness — and the byte copy
  succeeds precisely because the pages are writable. Making user text
  non-writable after load, and adding a RO entry for rodata, is a
  **cross-cutting** refinement that applies to the raw path and the ELF path
  alike; it is not an ELF-arc task.
- **A per-process configurable stack or an ELF-note stack.** A constant
  `STACK_SZ` for now.
- **Cross-arch servers (x86-64 / riscv64) and the display server.** The arc is
  the aarch64 console server proving the machinery; the rest reuse it.
- **Rewriting the simple raw-code images to ELFs.** Their *programs* stay
  hand-assembled (the user's call); only the `spawn` call is wrapped in
  `Image::Raw`.

## Decision records

**1. The format is ELF — specifically a static, non-PIE, fixed-base ELF — not
a flat or custom binary.**
- *Alternatives:* (a) a flat blob (`objcopy -O binary`), i.e. the current
  raw-code path scaled up; (b) a custom minimal r9 header.
- *Why ELF:* a real server needs distinct executable-text vs. read-write-data
  pages, and r9 already models that split (`rw_user_text` / `rw_user_data`). A
  flat blob cannot carry per-segment permissions without reinventing a header —
  i.e. reinventing ELF. The linker emits entry, segments, permissions, and
  sizes for free; the loader is ~50 lines and host-testable. Plan 9, QNX, and
  Linux all exec ELF, so the choice keeps the door open to 9P `exec` and
  standard tooling. Static non-PIE means zero relocations at load (verified).
  The user's "(elfs?)" is resolved **for** ELF.
- *Dissent:* the kernel-taste / simplicity lens would prefer
  the fewest bytes and a custom format with no parser. Accepted cost: a
  ~100-line ELF64 header + `PT_LOAD` reader. It is pure and host-tested, and it
  **deletes** the hand-assembly — a larger and growing cost. The parser is the
  small, stable part; the hand-assembled server bodies are the large, volatile
  part it removes.

**2. Unify under one `Image` enum and a single `spawn(&Image)` — now, as an
early call.**
- *Alternatives:* (a) keep `spawn(&[u8], …)` and add a second
  `spawn_elf(&[u8])`, deferring the enum until a second ELF user exists (the
  panel's original recommendation); (b) the `Image` enum now.
- *Why (b):* the whole-system lens (uniform metaphor) wants one
  way to start a process, and the user chose that over the deferral. One public
  `spawn`, the two input shapes as variants of one type, each arm a concrete
  private function — there is genuinely one entry point, not two ways to say
  one thing.
- *Cost (stated):* the signature change forces the ~19 existing raw call sites
  (the 9 raw test images + `main.rs`) to become `Image::Raw { … }` in the same
  change. Mechanical and grep-able, and the *program bytes* are untouched — but
  it touches files the new machinery would otherwise not have, so it is a real
  diff beyond the loader itself.
- *Dissent:* the kernel-taste lens (no midlayer, no abstraction with
  one instantiation) and the simplicity lens would defer the enum to the second
  server: an `Image::Elf` arm with a single user today is a layer not yet
  earning its keep. Recorded, not averaged away: the user overrode the deferral
  deliberately, accepting the call-site sweep now so that the second server
  (and the kernel's own ELF boot) lands on an already-uniform `spawn` instead of
  refactoring it.

**3. Embed via `include_bytes!` staged through a `build.rs` into `OUT_DIR`;
xtask builds the server first.**
- *Alternatives:* (a) `include_bytes!` a source-tree ELF path (no build.rs);
  (b) a `build.rs` that *builds* the server (nested cargo).
- *Why this:* (b) deadlocks — a build script invoking cargo in the same
  workspace contends for the build lock. (a) works but pollutes the source tree
  with generated, arch-specific ELFs. This keeps the source tree clean, uses
  cargo's native mtime dependency (`rerun-if-changed` + the included `OUT_DIR`
  file), and leaves the *ordering* (server before image) to xtask, which
  already orchestrates every build step. The requested dependency holds:
  server source changes → xtask rebuilds the ELF (new mtime) → the image's
  build.rs reruns → rustc recompiles the image.
- *Cost / dissent:* a bare `cargo check` / `build` of the embedding image
  *outside* xtask fails if the ELF is absent. Accepted: the documented build
  path is xtask (AGENTS.md: every gate goes through `cargo xtask`), and the
  build.rs fails **loudly** with "build the server via xtask first" rather than
  silently. The microkernel lens (restartability, no silent states)
  endorses the loud failure.

**4. Servers are separate workspace packages (`servers/console`),
aarch64-scoped for the arc.**
- *Alternatives:* a `[[bin]]` in the `aarch64` package; per-arch server
  packages now.
- *Why separate packages:* the user asked for "separate executables," and a
  server bin needs different link flags (static, `--image-base`) than the
  kernel bin. Two bins in one package share one `RUSTFLAGS`/target, so they
  cannot differ in a single build. Separate packages give each server its own
  build config. aarch64-scoped because the `Aspace` is aarch64-only; x86-64 /
  riscv64 servers appear when their `Aspace` lands.
- *Dissent:* the whole-system lens notes a top-level `servers/` member adds a
  workspace tree for one package. Accepted: it is the natural home for the
  many servers the substrate end-state names, and it keeps them out of the
  kernel package's build.

**5. The loader inherits the existing `AllRw` user page model; it does not
tighten it, and W^X tightening is not an ELF-arc task.**
- *Why:* both user `Entry` constructors are `AllRw` — `rw_user_text` is
  writable+executable (a pre-existing W+X the raw `spawn` already carries) and
  `rw_user_data` is writable+XN. The copy into a mapped page *requires*
  writable pages, and the loader reuses exactly the entries `spawn` uses, so
  the ELF path adds no new looseness and the copy cannot fault. A stricter
  model (non-writable user text, a RO entry for rodata) is a single
  cross-cutting improvement to the *shared* `Entry` set, benefiting both the
  raw and ELF paths — deferred, and not gated on this arc.
- *Dissent:* the hardware lens and the simplicity lens both want
  W^X (no page both writable and executable). Recorded: the W+X is real and
  pre-existing; it is flagged here so it is tracked as its own hardening task
  rather than silently absorbed into the ELF work or mistaken for something the
  ELF arc must fix.

## Tasks

Ordered; each is a `tasks/*.md`.

1. **`elf-reader-port.md`** — `port::elf`: the ELF64 header + `PT_LOAD` reader
   over `&[u8]`, `Result`-returning, with host unit tests (in-memory ELF
   fixtures). No arch dependencies. *Prerequisite for the loader.*
2. **`process-spawn-elf.md`** — the `Image` enum + the single
   `aarch64::process::spawn(&Image)`: rename the current `spawn` body to
   `spawn_raw`, add the `spawn_elf` arm (map the `PT_LOAD`s exec-vs-data,
   copy/zero, derive the stack, `forkret` at `e_entry`, placement-validated),
   and migrate the ~19 existing raw call sites (9 raw test images + `main.rs`)
   to `Image::Raw { … }`. Add the layout constants. Proven by task 4's image
   (the mapping is arch code that runs only at boot). *Needs 1.*
3. **`server-console-package.md`** — the `servers/console` workspace member
   (the minimal Rust console server: `start`, the syscall shim, `SYSMAPMMIO` +
   write 'A' + exit, panic handler) **and** the xtask `ServerStep` (static,
   `--image-base`, entry symbol → `target/<spec>/<profile>/console.elf`).
   *Independent of 1/2* (it is a build-side artifact).
4. **`console-server-elf-image.md`** — the embedding: the `console_server`
   test's `build.rs` stages the console ELF into `OUT_DIR`
   (`rerun-if-changed`); the test swaps its `Image::Raw { text: &SERVER_TEXT,
   … }` (from task 2's sweep) for `Image::Elf(CONSOLE_ELF)` and deletes the
   hand-assembled bytes. The end-to-end proof: a Rust-built server ELF loads
   and runs with no kernel file access, and changing the server rebuilds the
   image. *Needs 1 + 2 + 3; the proof.*

Sequencing: 1 → 2 (the loader needs the parser); 3 is independent of 1/2 and
can land in parallel with them; 4 last (needs all three).

*Next arc, not filed here:* the kernel (`main9`) adopting the built console
server in place of its raw `FIRST_PROCESS_TEXT` is the bridge to "boot to
graphics" (the Amiga end-state). It reuses everything this arc builds; it is
separate because it changes the default boot path, not just the machinery.
