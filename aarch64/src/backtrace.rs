//! Fault backtrace: walk the user's frame-pointer chain and print return
//! addresses with function names.  Called from `process::fault` after the
//! FAR/ESR line.
//!
//! With `"frame-pointer": "always"` in the target spec, every function has a
//! stable FP chain: `fp->saved_fp` (next frame's FP) and `fp->lr` (return
//! address).  The walk is bounded (32 frames) and every read goes through
//! `read_user` (a bad FP pointing at an unmapped VA is a walk stop, not a
//! kernel data abort).
//!
//! Symbolication: if the process was spawned from an ELF with a `.symtab`,
//! each return address is resolved to `name+0xoffset` via a linear scan of
//! the symbol table (cold path, a few dozen entries).  Raw images or stripped
//! ELFs fall back to raw addresses.

use port::elf::SymRef;
use port::iprintln;

/// Maximum frames to print (corrupt-stack guard).
const MAX_FRAMES: usize = 32;

/// Walk the user's FP chain starting at `fp` (the fault frame's frame
/// pointer).  If `fp` is zero, fall back to `lr` (the fault frame's link
/// register — the return address of the faulting function).  Prints each
/// return address as `#N name+0xoffset (0x...)` (or just `#N 0x...` when no
/// symbol table is available).  Stops after `MAX_FRAMES` frames, when `fp`
/// is zero, not 16-byte aligned, or a read fails (unmapped or bad page).
pub(crate) fn print_backtrace(sp: u64, fp: u64, lr: u64, sym: Option<SymRef>) {
    let sym = sym.as_ref();

    // If the FP is zero (the function didn't set it, or it's the outermost
    // frame), print just the LR (the faulting function's return address).
    if fp == 0 {
        if lr != 0 {
            iprintln!("  backtrace:");
            print_addr(0, lr, sym);
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
        print_addr(depth, lr, sym);
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
            print_addr(depth, ret_addr, sym);
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

/// Print one backtrace frame: `#N name+0xoffset (0xaddr)` if a symbol
/// matches, or `#N 0xaddr` if not.
fn print_addr(depth: usize, addr: u64, sym: Option<&SymRef>) {
    if let Some(sym) = sym
        && let Some((name, offset)) = lookup_symbol(sym, addr)
    {
        iprintln!("    #{depth}  {name}+{offset:#x}  ({addr:#018x})");
        return;
    }
    iprintln!("    #{depth}  {addr:#018x}");
}

/// Find the nearest symbol at or below `addr` in the ELF's `.symtab`.
/// Returns `(name, offset)` or `None` if no function symbol matches.
fn lookup_symbol(sym: &SymRef, addr: u64) -> Option<(&'static str, u64)> {
    // SAFETY: `sym.bytes` is a pointer into the kernel's embedded ELF buffer
    // (a `&'static [u8]` from `include_bytes!`), valid for the boot's life.
    // `sym.len` is the buffer's length; all reads below are bounds-checked.
    let bytes = unsafe { core::slice::from_raw_parts(sym.bytes, sym.len) };

    let tab = &sym.tab;
    let symtab =
        bytes.get(tab.offset as usize..(tab.offset as usize) + tab.nsyms * 24).unwrap_or_default();
    let strtab_start = tab.strtab_offset as usize;
    let strtab = bytes.get(strtab_start..)?;

    let mut best: Option<(u64, u32)> = None; // (st_value, st_name)
    for i in 0..tab.nsyms {
        let base = i * 24;
        let st_name = u32::from_le_bytes(symtab[base..base + 4].try_into().unwrap());
        let st_info = symtab[base + 4];
        let st_value = u64::from_le_bytes(symtab[base + 8..base + 16].try_into().unwrap());
        // Only function symbols with a non-zero address.
        if st_info & 0x0f != 2 || st_value == 0 {
            continue;
        }
        if st_value <= addr {
            // Keep the largest st_value ≤ addr.
            match best {
                Some((bv, _)) if bv >= st_value => {}
                _ => best = Some((st_value, st_name)),
            }
        }
    }

    let (st_value, st_name) = best?;
    // Read the symbol name from the strtab.
    let name_bytes = strtab.get(st_name as usize..)?;
    let nul = name_bytes.iter().position(|&b| b == 0)?;
    // SAFETY: the bytes are a valid UTF-8 substring of the ELF's strtab
    // (symbol names are ASCII/UTF-8 by the ELF spec).
    let name = core::str::from_utf8(&name_bytes[..nul]).ok()?;
    Some((name, addr - st_value))
}
