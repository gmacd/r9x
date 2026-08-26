//! The system bringup: the set of processes the full system runs, and how
//! they are spawned.  Extracted from the kernel binary's `main9` so that the
//! kernel image *and* the `system` integration test call the **same** function —
//! they cannot drift apart the way the two diverged when init came to require a
//! channel pair (the kernel still passed `handles: None`).
//!
//! This is the "spawn the servers" part of the boot: register the image
//! registry, create the channel pairs, and spawn the nameserver, the console,
//! and init.  The caller does the rest — `set_console_live` (kernel only),
//! `run_all`, and its own tail (the idle loop, or the test's check).

use crate::{ipc, process, registry};

// The user-space server ELFs, staged into OUT_DIR by build.rs.
static CONSOLE_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/console.elf"));
static NAMESERVER_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/nameserver.elf"));
static INIT_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/init.elf"));
static CHILD_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/child.elf"));
static DISPLAY_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/display.elf"));
static MAILBOX_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/mailbox.elf"));

// The registry entry for the child: `static` so it outlives the `register`
// borrow (the registry holds `&'static` entries).
static CHILD_EMBEDDED: registry::EmbeddedElf =
    registry::EmbeddedElf { bytes: CHILD_ELF, name: "child" };

/// Spawn the full system: the nameserver, the console, and init (the process
/// manager, which `SYS_SPAWN`s the child by index).
///
/// The nameserver must be up before the console server's BIND is processed.
/// This holds by construction: the nameserver is spawned first and blocks on
/// its first receive; the console server's BIND send wakes it (the IPC fast
/// path); the nameserver processes the BIND before the console server blocks
/// on its post-bind receive.
pub fn bringup() -> (process::Handles, process::Handles) {
    // Register the image registry: init (the process manager) spawns the
    // child by index, so the child must be registered before init runs.
    // index 0 is the child; the nameserver and console are the init-context
    // spawn (not registry entries — they are hard-started by the kernel, not
    // launched by init).
    registry::register(&[&CHILD_EMBEDDED]);

    let ns_in = ipc::create();
    let ns_out = ipc::create();
    let ns_handles = process::Handles {
        inbound: ns_in as u32,
        outbound: ns_out as u32,
        extra_inbound: 0,
        extra_outbound: 0,
    };
    // init's own pair: the process manager reads its child-state (the
    // generalized `HANDLES_VA` header) before it can spawn, so it needs a
    // real pair — a zero page would be `n_handles == 0`, not a pair.
    let init_in = ipc::create();
    let init_out = ipc::create();
    let init_handles = process::Handles {
        inbound: init_in as u32,
        outbound: init_out as u32,
        extra_inbound: 0,
        extra_outbound: 0,
    };

    // The mailbox server: owns the BCM283x Mailbox property interface.  It
    // must be up before the display server (the display server sends it a
    // framebuffer config request during init).
    let mbox_in = ipc::create();
    let mbox_out = ipc::create();
    let mbox_handles = process::Handles {
        inbound: mbox_in as u32,
        outbound: mbox_out as u32,
        extra_inbound: 0,
        extra_outbound: 0,
    };

    process::spawn(&process::Image::Elf { bytes: NAMESERVER_ELF, handles: Some(ns_handles) });
    process::spawn(&process::Image::Elf { bytes: MAILBOX_ELF, handles: Some(ns_handles) });
    process::spawn(&process::Image::Elf { bytes: CONSOLE_ELF, handles: Some(ns_handles) });
    process::spawn(&process::Image::Elf { bytes: INIT_ELF, handles: Some(init_handles) });
    (ns_handles, mbox_handles)
}

/// Spawn the display server, handing it the nameserver's channel pair so it
/// can publish `/dev/display`.  Called after `bringup()` (the nameserver must
/// be up before the BIND is processed).  The display server runs forever
/// (the frame loop), so it is not in `bringup()` — `run_all` in the `system`
/// image would never return.
pub fn spawn_display(ns_handles: process::Handles, mbox_handles: process::Handles) {
    let display_handles = process::Handles {
        inbound: ns_handles.inbound,
        outbound: ns_handles.outbound,
        extra_inbound: mbox_handles.inbound,
        extra_outbound: mbox_handles.outbound,
    };
    process::spawn(&process::Image::Elf { bytes: DISPLAY_ELF, handles: Some(display_handles) });
}
