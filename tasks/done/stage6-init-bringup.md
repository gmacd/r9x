---
id: 70
status: done
commit: e6d9145
---

# stage6-init-bringup: the boot-time bringup (kernel spawns, `init` is the first user process)

**Follow-on to stage 6a** — deferred from the narrow slice (which is driven by
the `namespace` test image). Needs tasks 66–69 done and proven. Plan:
[plans/microkernel-nameserver.md](../plans/microkernel-nameserver.md).

## Goal

Replace the kernel's trivial "first process" (the `svc #0` stub in `main9`)
with the real bringup: the kernel spawns the nameserver, the console server,
and `init` — then calls `run_all()` and the system is live. The kernel's
user-process work is done the moment `run_all()` is called; everything after
is user space.

## Why it is its own task

It touches the kernel boot path (`main9`'s first-process spawn) and makes the
test-image wiring (the `namespace` image's spawn sequence) the production
sequence. The "one permanent kernel-resident device" decision (the kernel keeps
its PL011 mapping) is made explicit here rather than left implicit.

## Design

### The spawn model: kernel-side (option B)

The kernel's `main9` does all the spawning. There is no user-space spawn
syscall (`SYCSPAWN`) in this task. The rationale:

- The kernel already has access to the embedded ELFs (the `build.rs` stages
  them into `OUT_DIR`, the same mechanism the test images use).
- The `namespace` test image already proves the spawn sequence works: create
  the nameserver's pair, spawn the nameserver, spawn the console server,
  `run_all()`. The production `main9` does the same thing without the test
  harness.
- A user-space `SYCSPAWN` is a significant new kernel surface (image
  registration, index validation, handle handoff from user space, scheduling
  of the new process). It is a **stage 7** concern: `init` needs it when it
  must manage multiple servers, handle crashes, and restart them. Building it
  now, with one server to spawn and no crash story, is premature.

`init` is the first "real" user process: it exists, it is alive, it will be
the parent of stage-7 servers. In this task it does nothing but stay alive
(blocked on a receive, or a simple loop). It is the placeholder for the
process-manager that stage 7 fills in.

### The `main9` sequence

```text
1. Hardware bringup (as today): mailbox, console, GIC, timer, user page tables
2. Create the nameserver's channel pair (kernel-side `ipc::create()` × 2)
3. Spawn the nameserver ELF, handing it its pair via `HANDLES_VA`
4. Spawn the console server ELF (it creates its own pair via `SYCCREATECHAN`,
   maps the PL011, writes 'A', BINDs `/dev/console`)
5. Spawn `init` (a trivial ELF: block on a receive it never satisfies, or an
   infinite loop — it just exists)
6. `run_all()` — never returns (the servers and `init` keep running)
```

The nameserver must be up before the console server's BIND is processed. This
holds by construction: the nameserver is spawned first and blocks on its
first receive; the console server's BIND send wakes it (the IPC fast path);
the nameserver processes the BIND before the console server blocks on its
post-bind receive. No polling, no retry.

### What `init` is (this task)

A minimal ELF built with the `ServerStep` pattern. Its `start()`:
- Blocks on a `SYCRECEIVE` on a channel that no one sends to (it sleeps
  forever), or
- Spins in an infinite loop (it is always Runnable; the scheduler round-robins
  it with the servers)

The first is preferable: a blocked `init` costs zero CPU. It needs a channel
pair to block on — but the 4-slot table is full (nameserver pair + console
server pair). Two options:
- **(a)** Grow the table to 6 slots (the `NCHANNELS` const). `init` creates
  a pair via `SYCCREATECHAN` and blocks on it. Clean, but a new const change.
- **(b)** `init` doesn't create a channel; it just spins. Simpler, but costs
  a time-slice.

Prefer (a): the table size is a const, growing it from 4 to 6 is a one-line
change, and a blocked process is the correct shape for a process manager that
will eventually be woken by events.

### Channel table

With the grow to 6:
- Nameserver pair: slots 0, 1 (kernel-created, handed via `HANDLES_VA`)
- Console server pair: slots 2, 3 (server-created via `SYCCREATECHAN`)
- `init` pair: slots 4, 5 (server-created via `SYCCREATECHAN`)

Six slots covers the current system with room for one more server. The size
is a const; the next grow is a decision, not an accident.

### What changes in `main9`

The current `main9` (after hardware bringup):
```rust
let id = process::spawn(&process::Image::Raw { text: &FIRST_PROCESS_TEXT, ... });
process::run_all();
let status = process::status(id);
println!("first process returned, status {status:?}");
println!("looping now");
loop {}
```

Becomes:
```rust
// Nameserver: kernel creates its pair, hands it via HANDLES_VA.
let ns_in = ipc::create();
let ns_out = ipc::create();
let ns_handles = process::Handles { inbound: ns_in as u32, outbound: ns_out as u32 };
process::spawn(&process::Image::Elf { bytes: NAMESERVER_ELF, handles: Some(ns_handles) });

// Console server: creates its own pair, maps its own PL011, BINDs.
process::spawn(&process::Image::Elf { bytes: CONSOLE_ELF, handles: None });

// init: the first real user process. Blocks forever (stage 7 fills it in).
process::spawn(&process::Image::Elf { bytes: INIT_ELF, handles: None });

// The system is live. The kernel's role is scheduler + interrupt handler.
process::run_all();
// If run_all returns, init exited — fatal.
panic!("init exited");
```

The `FIRST_PROCESS_TEXT` raw stub is deleted. The `loop {}` is gone.

### The `build.rs` change

`aarch64/build.rs` already stages the console and nameserver ELFs. It gains a
third: `init.elf`. The `build.rs` panics if any are missing (as today).

### The `init` server package

`servers/init/` — a `no_std` Rust bin, same pattern as `servers/console` and
`servers/nameserver`. Its `start()`:
```rust
pub extern "C" fn start() -> ! {
    // Create a channel pair to block on (stage 7 will send events here).
    let in_h = unsafe { sys(SYCCREATECHAN, 0, 0, 0, 0, 0) }.0;
    let _out_h = unsafe { sys(SYCCREATECHAN, 0, 0, 0, 0, 0) }.0;
    // Block forever: receive on the inbound channel (no one sends).
    let mut buf = [0u8; 256];
    loop {
        unsafe { sys(SYCRECEIVE, in_h, buf.as_mut_ptr() as u64, buf.len() as u64, 0, 0) };
    }
}
```

The `ServerStep` gains `init` to its list of servers to build.

## Acceptance

- `cargo xtask qemu --arch aarch64` boots to a running nameserver + console
  server + `init`, with no test harness. The kernel's only user-process action
  is the three spawns and the `run_all`.
- The console server's 'A' appears on the serial output (it wrote it during
  its bringup).
- `cargo xtask ci` green (all existing test images still pass; they have their
  own spawn sequences and are unaffected by the `main9` change).
- The `namespace` test image still passes (it is independent of `main9`).

## Not in scope

- `SYCSPAWN` (user-space spawn). Stage 7, with the crash/restart story.
- The early-console retirement (task 71).
- 9P / the stage-7 servers. A shell.
- `init` doing anything other than exist and block.
