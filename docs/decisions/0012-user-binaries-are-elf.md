---
status: accepted
---

# 0012 — User binaries are static non-PIE ELF, embedded at build time

- **Status**: accepted — implemented (`port/src/elf.rs`, `aarch64/src/process.rs` `Image::Elf`, `cmd/*`, `aarch64/src/system.rs`)
- **Date**: 2026-08-28 (record written from `tasks/plans/user-binary-loading.md`)
- **Context**: `tasks/plans/user-binary-loading.md`

## Decision

Servers are static, non-PIE, fixed-base ELF binaries. They are separate
workspace packages (in `cmd/`), built by xtask, staged into `OUT_DIR` by a
`build.rs` and embedded with `include_bytes!`. The kernel has one public
`spawn(&Image)` with the input shapes as variants of one `Image` enum, not two
entry points.

## Why

A real server needs distinct executable-text and read-write-data pages, and
r9x already models that split. A flat blob cannot carry per-segment
permissions without reinventing a header — that is, reinventing ELF. The
linker emits entry, segments, permissions and sizes for free; the loader is
small and host-testable; static non-PIE means zero relocations at load. Plan 9,
QNX and Linux all exec ELF, so the choice keeps 9P `exec` and standard tooling
reachable.

Staging through `build.rs` keeps generated arch-specific ELFs out of the source
tree while preserving the dependency that matters: server source changes →
xtask rebuilds the ELF → the image's `build.rs` reruns → the image recompiles.
Separate packages exist because a server needs different link flags (static,
`--image-base`) than the kernel, and two binaries in one package share one
`RUSTFLAGS`.

## Alternatives rejected

- **A flat blob** (`objcopy -O binary`), i.e. the raw-code path scaled up.
  Lost: no per-segment permissions without inventing a header.
- **A custom minimal r9 header.** Lost: a parser to write and maintain, for
  less than ELF gives free.
- **A `build.rs` that builds the server** (nested cargo). Lost: it deadlocks
  on the workspace build lock.
- **`include_bytes!` from a source-tree path.** Lost: pollutes the tree with
  generated arch-specific binaries.
- **A second `spawn_elf` entry point,** deferring the enum until a second user.
  Lost by explicit override: one way to start a process was worth the
  call-site sweep now, so that later servers land on a uniform `spawn`.

**Dissent** (kernel-taste and simplicity-and-interfaces lenses): the enum has
one real user today, and an ELF parser is bytes the kernel did not have.
Recorded as a deliberate override — the parser is the small stable part, and it
deletes the hand-assembled server bodies, which were the large volatile part.

## Consequences

- A bare `cargo build` of an embedding image outside xtask fails if the ELF is
  absent; `build.rs` fails loudly with "build the servers first: `cargo
  xtask build --arch aarch64`" rather than silently.
- The loader inherits the shared user page-entry set rather than defining its
  own, so hardening it hardens both spawn paths at once. At plan time that
  meant inheriting a pre-existing W+X on user text, deliberately left for a
  separate task rather than absorbed into the ELF work.
  **Update, verified 2026-08-28 at `f76d96a`:** that task landed — `Entry::ro_user_text()`
  (AP `AllRo`, PXN set) now backs both the raw path (`aarch64/src/process.rs:613`)
  and the ELF loader's executable segments (`aarch64/src/process.rs:748`), so
  user text is RO+X. See task 96, commit `d773a37`.
- The plan named `servers/console`; the tree landed on `cmd/console`. The
  reasoning is unchanged — the deviation is naming, recorded here so the plan
  and the tree can be reconciled by a reader.
