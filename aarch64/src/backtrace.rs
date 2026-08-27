//! Fault backtrace: walk the user's frame-pointer chain and print return
//! addresses.  Called from `process::fault` after the FAR/ESR line.
//!
//! With `"frame-pointer": "always"` in the target spec, every function has a
//! stable FP chain: `fp->saved_fp` (next frame's FP) and `fp->lr` (return
//! address).  The walk is bounded (32 frames) and every read goes through
//! `read_user` (a bad FP pointing at an unmapped VA is a walk stop, not a
//! kernel data abort).
//!
//! Symbolication is offline: the log shows raw addresses + the image
//! registry index.  The ELF is in the build tree; `llvm-symbolizer` or an
//! xtask helper turns the addresses into names.

use port::iprintln;

/// Maximum frames to print (corrupt-stack guard).
const MAX_FRAMES: usize = 32;

/// Walk the user's FP chain starting at `fp` (the fault frame's frame
/// pointer).  If `fp` is zero, fall back to `lr` (the fault frame's link
/// register — the return address of the faulting function).  Prints each
/// return address as `#N 0x...`.  Stops after `MAX_FRAMES` frames, when `fp`
/// is zero, not 16-byte aligned, or a read fails (unmapped or bad page).
pub(crate) fn print_backtrace(sp: u64, fp: u64, lr: u64) {
    // If the FP is zero (the function didn't set it, or it's the outermost
    // frame), print just the LR (the faulting function's return address).
    if fp == 0 {
        if lr != 0 {
            iprintln!("  backtrace:");
            iprintln!("    #0  {lr:#018x}");
        }
        return;
    }

    iprintln!("  backtrace:");
    let mut depth = 0usize;
    let mut cur_fp = fp;

    // Print the faulting function's return address first (the LR at the
    // fault point — the address that will be executed when the current
    // function returns).
    if lr != 0 {
        iprintln!("    #0  {lr:#018x}");
        depth += 1;
    }

    while cur_fp != 0 && cur_fp.is_multiple_of(16) && depth < MAX_FRAMES {
        // Read the saved FP (next frame's FP) and the return address (LR)
        // from the current frame.  Layout: [fp] = saved_fp, [fp+8] = lr.
        let mut buf = [0u8; 16];
        let ok = unsafe { crate::ipc::read_user(&mut buf, cur_fp as *const u8, 16) };
        if !ok {
            break;
        }
        let saved_fp = u64::from_le_bytes(buf[0..8].try_into().unwrap());
        let ret_addr = u64::from_le_bytes(buf[8..16].try_into().unwrap());
        if ret_addr != 0 {
            iprintln!("    #{depth}  {ret_addr:#018x}");
        }
        depth += 1;
        if saved_fp == cur_fp {
            // Corrupt: the chain is looping.
            break;
        }
        cur_fp = saved_fp;
    }

    // `sp` is used as a sanity bound in a future extension (verify each FP is
    // within the process's stack range); kept in the signature for that.
    let _ = sp;
}
