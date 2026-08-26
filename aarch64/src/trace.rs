//! Syscall trace: log every syscall (number, args, result) to the kernel
//! console. Disabled by default; zero overhead when off (the module is not
//! compiled).
//!
//! Enable with `--features systrace`.

/// Trace a syscall entry: the process id, syscall number, and input args.
#[cfg(feature = "systrace")]
#[inline(never)]
pub fn syscall(proc_id: usize, number: u64, x0: u64, x1: u64, x2: u64, _dummy: u64) {
    let name = name(number);
    port::iprintln!(
        "[systrace] proc={proc_id} call {name}({number}) args=({x0:#x}, {x1:#x}, {x2:#x})"
    );
}

/// Trace a syscall result: the process id, syscall number, and return
/// values.
#[cfg(feature = "systrace")]
#[inline(never)]
pub fn result(proc_id: usize, number: u64, r0: u64, r1: u64, r2: u64) {
    let name = name(number);
    if r1 != 0 || r2 != 0 {
        port::iprintln!("[systrace] proc={proc_id} ret {name} -> ({r0:#x}, {r1:#x}, {r2:#x})");
    } else {
        port::iprintln!("[systrace] proc={proc_id} ret {name} -> {r0:#x}");
    }
}

/// The syscall number→name lookup.
#[cfg(feature = "systrace")]
fn name(number: u64) -> &'static str {
    match number {
        0 => "SYSEXIT",
        1 => "SYSYIELD",
        16 => "SYCSEND",
        17 => "SYCRECEIVE",
        18 => "SYCREPLY",
        19 => "SYSIRQCLAIM",
        20 => "SYS_MAP_MMIO",
        21 => "SYCCREATECHAN",
        22 => "SYS_ALLOC",
        23 => "SYS_FREE",
        24 => "SYS_SPAWN",
        25 => "SYS_CLOCK",
        26 => "SYS_RECEIVE_AT",
        27 => "SYS_ALLOC_PAGE",
        28 => "SYS_WAIT",
        29 => "SYS_KILL",
        30 => "SYS_SETPRIO",
        31 => "SYS_PRINT",
        _ => "?",
    }
}
