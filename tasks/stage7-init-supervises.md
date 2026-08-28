---
id: 98
status: parked
---

# Task 98: init supervises the servers (stage 7)

## Status: parked (successor to the done task 70)

## Problem

Stage 6a shipped a hybrid: the kernel hard-starts the core servers
(`system::bringup()` spawns nameserver → mailbox → console → init,
`aarch64/src/system.rs:70-73`), and init only spawns dynamic children.
The end state every reference points at is: **kernel spawns only
init/the root task; init spawns and supervises everything else** —
Plan 9's userinit→/boot→bootrc, Linux's kernel_init→run_init_process,
seL4's root task, Zircon's userboot→component manager, and — for the
supervision half — Minix 3's reincarnation server (restart a dead
system server).

Task 70's file was the only place holding this intent; it is filed in
`done/`, so this task is the recorded successor.

## Design sketch

- `bringup()` shrinks to spawning init alone; init spawns nameserver,
  mailbox, console, display from the image registry in the same
  by-construction order (the ordering argument at `system.rs:35-39`
  moves into init).
- init `SYS_WAIT`s on its children; a dead server is respawned (bounded
  retries, then a loud complaint via SYS_PRINT). The machinery already
  exists: image registry, `SYS_SPAWN`, `SYS_WAIT`/`SYS_KILL` (02027fa).
- Depends on task 95 (channels close on exit) so clients of a dead
  server unblock with `ERR_CLOSED` and can re-resolve after the restart;
  re-BIND on restart is the nameserver-side half to design (stale
  binding vs re-bind — decide here).
- The early-boot chicken-and-egg (init needs the console for errors
  before the console server is up) is already solved by SYS_PRINT.

## Unpark when

Stage 7 opens (9P servers / fs work), or the first time a server crash
in the field costs a debugging session — whichever comes first.

## Done when

- The kernel spawns exactly one process; a killed server is observably
  restarted in an integration image (kill console, see it re-bind, a
  client write succeeds after).

Origin: backlog audit 2026-08-27 (parked group — preserving the stage-7
intent task 70's completion would otherwise have lost).
