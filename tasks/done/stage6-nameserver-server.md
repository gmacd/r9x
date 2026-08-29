---
status: done
---

# stage6-nameserver-server: the nameserver — a user-space name→channel map

Stage 6a, task 2 of 4. Plan:
[plans/microkernel-nameserver.md](../plans/microkernel-nameserver.md).
Needs task 1 (`SYCCREATECHAN`) — though the nameserver's *own* pair is handed to
it by the spawner (task 4), it must be a real user-space server on message
syscalls.

## Goal

The first nameserver: a separate `no_std` Rust executable (the stage-5
`ServerStep` pattern, `servers/nameserver/`) that owns the bind table and serves
`BIND` / `RESOLVE` / `UNBIND` over the message syscalls. This is the process the
whole file metaphor resolves through. No kernel change in this task.

## Changes

- **New workspace member `servers/nameserver/`** (add to root `[workspace]
  members`; reuse the `ServerStep` from task 63 with its package name).
  `#![no_std] #![no_main]`, the same `sys(n,a0,a1)` shim as `servers/console`.
- **The bind table** (in the nameserver's `Aspace`):
  ```text
  const NENT: usize = 8;
  const NAME_MAX: usize = 32;
  struct Entry { name: [u8; NAME_MAX], namelen: u8, in: u32, out: u32, used: bool }
  struct BindTable { entries: [Entry; NENT] }
  ```
  Linear scan over `NENT` (single-digit; no hash, no tree — the tree is stage 7).
  A bind entry stores the server's **pair** `(in, out)`, not one handle: a
  channel is unidirectional, so a client needs the inbound handle to send and
  the outbound handle to receive the reply.
- **The message loop** (the existing `SYCRECEIVE`/`SYCREPLY` syscalls, the
  handles handed in by the spawner — see task 4):
  - `BIND { name, in, out }` → add or replace the entry; reply `OK`, or `Efull`
    if the table is full with no free/reusable slot.
  - `RESOLVE { name }` → reply the `(in, out)` pair, or `ENoent`.
  - `UNBIND { name }` → clear the entry; reply `OK` or `ENoent`.
  - `tag` carries the request/reply correlation (the envelope's existing field).
- **Envelope layout** (a stated convention, mirrored by task 4's client):
  `buf` = `name` (NUL-free, `len` = name length) for a request; `buf` =
  `[in:4 LE][out:4 LE]` for a `RESOLVE` reply. Fits `MSG_MAX` (256) easily.
- **How the nameserver gets its own pair**: the spawner (task 4) creates it
  kernel-side and hands both handles to the nameserver *and* keeps a copy for
  the client. The nameserver does not create its own pair (it is the first
  server — nothing exists yet that a client could ask to find it). The
  nameserver's `start()` reads its two handles from a fixed, agreed location
  (the stage-5 "pass constants to an embedded server" convention — a known VA,
  or a `spawn` argument; decide in task 4 and keep the nameserver side dumb).

## Tests

- Covered end-to-end by task 4's `namespace` image. This task's unit surface is
  the bind table (a `#[cfg(test)]` host test if the table is factored into a
  testable module: bind/resolve/unbind, `Efull`, `ENoent`, replace-in-place).

## Acceptance

- `cargo xtask ci` green (the server builds as part of the gate).
- `servers/nameserver` compiles `no_std` for aarch64 with the r9 target spec,
  producing `nameserver.elf`.
- The nameserver is a real Rust program (not hand-assembled bytes) and speaks
  only the message syscalls.

## Not in scope

A tree bind table (directories, bind points) — stage 7 with 9P. `LOOKUP` /
enumeration — a 9P `walk` concern (stage 7). The boot-time `init` that spawns
this (task 5). 9P as the server protocol — stage 7 (this server speaks the
native-opcode protocol the substrate plan sanctioned for non-file servers).
