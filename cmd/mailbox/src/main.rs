//! The mailbox server: owns the BCM283x Mailbox property interface.
#![no_std]
#![no_main]
extern crate alloc;
use alloc::format;
use r9x_std::ipc;
use r9x_std::mem;
use r9x_std::println;
use r9x_std::process::exit;
use r9x_std::rt;

/// The page-aligned base of the Mailbox register page.  The BCM283x Mailbox
/// registers start at page offset 0x880 (absolute 0xFE00_B880 on the BCM2711).
const MBOX_PAGE_PHYS: u64 = 0xfe00_b000;
/// The Mailbox registers' word offset within the mapped page (0x880 / 4 = 544).
const MBOX_REG_BASE: usize = 0x880 / 4;
const MBOX_VA: u64 = 0x7000_0000;
/// The user VA of the Mailbox request/response buffer, mapped as Device
/// memory (see `main`): the VC reads the request and writes the response by
/// DMA, so a cached (Normal) mapping would hide the CPU's writes from the VC
/// and the VC's response from the CPU.  A free user VA in the MMIO range
/// (above the heap/stack, alongside MBOX_VA and FB_VA) holding one page.
const MBOX_BUF_VA: u64 = 0x5000_0000;
/// BCM2835 Mailbox register layout (relative to 0x880), mirroring
/// `aarch64/src/mailbox.rs`:
///   +0x00  Read (receive a response)
///   +0x18  Status (bit31 = 1: write full, bit30 = 1: read empty)
///   +0x20  Write (send a request)
const MBOX_READ: usize = MBOX_REG_BASE;
const MBOX_STATUS: usize = MBOX_REG_BASE + 0x18 / 4;
const MBOX_WRITE: usize = MBOX_REG_BASE + 0x20 / 4;
const MBOX_FULL: u32 = 0x8000_0000;
const MBOX_EMPTY: u32 = 0x4000_0000;
const CHANNEL_ARM_TO_VC: u32 = 8;
#[allow(dead_code)]
const TAG_FB_ALLOCATE: u32 = 0x0004_0001;
const TAG_FB_SET_WIDTH_HEIGHT: u32 = 0x0004_8003;
const TAG_FB_SET_DEPTH: u32 = 0x0004_8005;
const TAG_FB_SET_PIXEL_ORDER: u32 = 0x0004_8006;
/// Get the ARM memory range (base, size in bytes) via the mailbox
/// property interface.  The kernel used to print this at boot; the
/// mailbox server is the right owner of the hardware query.
const TAG_GET_ARM_MEMORY: u32 = 0x0001_0005;

#[allow(dead_code)]
const TAG_GET_FIRMWARE_REVISION: u32 = 0x0000_0001;
#[allow(dead_code)]
const TAG_GET_BOARD_MODEL: u32 = 0x0001_0001;
#[allow(dead_code)]
const TAG_GET_BOARD_REVISION: u32 = 0x0001_0002;
#[allow(dead_code)]
const TAG_GET_BOARD_MAC: u32 = 0x0001_0003;
#[allow(dead_code)]
const TAG_GET_BOARD_SERIAL: u32 = 0x0001_0004;
const OP_CONFIGURE_FB: u16 = 0;
const OP_GET_PROPERTY: u16 = 1;
const OP_BIND: u16 = 0;
const OP_RESOLVE: u16 = 1;
const R_OK: u16 = 0;
const NAME: &[u8] = b"/dev/mailbox";
const CONSOLE_NAME: &[u8] = b"/dev/console";

#[inline(never)]
#[unsafe(no_mangle)]
pub extern "C" fn start(dtb_va: usize) -> ! {
    rt::run(dtb_va, main)
}

unsafe fn mbox_reg_read(w: usize) -> u32 {
    let base = MBOX_VA as *mut u32;
    unsafe { core::ptr::read_volatile(base.add(w)) }
}
unsafe fn mbox_reg_write(w: usize, val: u32) {
    let base = MBOX_VA as *mut u32;
    unsafe { core::ptr::write_volatile(base.add(w), val) }
}
unsafe fn mbox_request(buf_pa: u64) {
    unsafe {
        while (mbox_reg_read(MBOX_STATUS) & MBOX_FULL) != 0 {}
        let r = (buf_pa as u32 & !0xF) | CHANNEL_ARM_TO_VC;
        mbox_reg_write(MBOX_WRITE, r);
        loop {
            while (mbox_reg_read(MBOX_STATUS) & MBOX_EMPTY) != 0 {}
            if mbox_reg_read(MBOX_READ) == r {
                break;
            }
        }
    }
}

fn configure_framebuffer(
    buf_va: usize,
    buf_pa: u64,
    width: u32,
    height: u32,
    bpp: u32,
) -> (u64, u64) {
    let base = buf_va as *mut u32;
    unsafe {
        core::ptr::write_bytes(base, 0, 16);
        core::ptr::write_volatile(base.add(0), 64);
        core::ptr::write_volatile(base.add(1), 0);
        core::ptr::write_volatile(base.add(2), TAG_FB_SET_WIDTH_HEIGHT);
        core::ptr::write_volatile(base.add(3), 8);
        core::ptr::write_volatile(base.add(4), 0);
        core::ptr::write_volatile(base.add(5), width);
        core::ptr::write_volatile(base.add(6), height);
        core::ptr::write_volatile(base.add(7), TAG_FB_SET_DEPTH);
        core::ptr::write_volatile(base.add(8), 4);
        core::ptr::write_volatile(base.add(9), 0);
        core::ptr::write_volatile(base.add(10), bpp);
        core::ptr::write_volatile(base.add(11), TAG_FB_SET_PIXEL_ORDER);
        core::ptr::write_volatile(base.add(12), 4);
        core::ptr::write_volatile(base.add(13), 0);
        core::ptr::write_volatile(base.add(14), 0);
        core::ptr::write_volatile(base.add(15), 0);
        mbox_request(buf_pa);
        // Clear the whole buffer before the second request: the ALLOCATE
        // tag's value fields (words 5-6) must be zeroed or the firmware sees
        // the previous response's bytes as a non-zero address and skips the
        // allocation.
        core::ptr::write_bytes(base, 0, 32);
        core::ptr::write_volatile(base.add(0), 32);
        core::ptr::write_volatile(base.add(1), 0);
        core::ptr::write_volatile(base.add(2), TAG_FB_ALLOCATE);
        core::ptr::write_volatile(base.add(3), 8);
        core::ptr::write_volatile(base.add(4), 0);
        core::ptr::write_volatile(base.add(7), 0);
        mbox_request(buf_pa);
        let phys = core::ptr::read_volatile(base.add(5)) as u64;
        let size = core::ptr::read_volatile(base.add(6)) as u64;
        (phys, size)
    }
}

fn get_property(buf_va: usize, buf_pa: u64, tag_id: u32) -> u32 {
    let base = buf_va as *mut u32;
    unsafe {
        core::ptr::write_bytes(base, 0, 6);
        core::ptr::write_volatile(base.add(0), 24);
        core::ptr::write_volatile(base.add(1), 0);
        core::ptr::write_volatile(base.add(2), tag_id);
        core::ptr::write_volatile(base.add(3), 4);
        core::ptr::write_volatile(base.add(4), 0);
        core::ptr::write_volatile(base.add(5), 0);
        mbox_request(buf_pa);
        core::ptr::read_volatile(base.add(5))
    }
}

/// Query the ARM memory range via the mailbox property interface.
/// Returns (base_phys, size_bytes).
fn get_arm_memory(buf_va: usize, buf_pa: u64) -> (u64, u64) {
    let base = buf_va as *mut u32;
    unsafe {
        // The ARM memory response carries two u32 values (base, size),
        // so the buffer needs 8 words: size, code, tag_id, val_size,
        // val_type, base, size, end_tag.
        core::ptr::write_bytes(base, 0, 8);
        core::ptr::write_volatile(base.add(0), 32);
        core::ptr::write_volatile(base.add(1), 0);
        core::ptr::write_volatile(base.add(2), TAG_GET_ARM_MEMORY);
        core::ptr::write_volatile(base.add(3), 8);
        core::ptr::write_volatile(base.add(4), 0);
        core::ptr::write_volatile(base.add(5), 0);
        core::ptr::write_volatile(base.add(6), 0);
        core::ptr::write_volatile(base.add(7), 0);
        mbox_request(buf_pa);
        let mem_base = core::ptr::read_volatile(base.add(5)) as u64;
        let mem_size = core::ptr::read_volatile(base.add(6)) as u64;
        (mem_base, mem_size)
    }
}

fn main() {
    let m = mem::map_mmio(MBOX_PAGE_PHYS, MBOX_VA, 4096);
    if m != 0 {
        println!("mailbox: map_mmio({MBOX_PAGE_PHYS:#x} @ {MBOX_VA:#x}) -> {m}");
        exit(10);
    }
    // A physical page backs the request/response buffer.  The buffer is used
    // at MBOX_BUF_VA, mapped as Device memory (not the page's Normal heap VA,
    // which is left unmapped-but-allocated): the VC reads the request and
    // writes the response over the Mailbox by DMA, and a cached (Normal)
    // mapping would let the CPU's writes sit in the cache, so the VC would
    // read stale bytes and the CPU would miss the VC's response.  Device
    // memory is DMA-safe in both directions.
    let Some((_, buf_pa)) = mem::alloc_page() else {
        println!("mailbox: alloc_page failed");
        exit(1);
    };
    if mem::map_mmio(buf_pa, MBOX_BUF_VA, 4096) != 0 {
        println!("mailbox: map_mmio(buf {buf_pa:#x} @ {MBOX_BUF_VA:#x}) failed");
        exit(13);
    }
    let buf_va = MBOX_BUF_VA as usize;
    // Query the ARM memory range: the kernel used to print this at boot, but
    // the mailbox server owns the hardware query.  The text is written through
    // the console server (below) so it appears on the terminal.
    let (mem_base, mem_size) = get_arm_memory(buf_va, buf_pa);
    let (in_h, out_h) = ipc::create_pair();
    if in_h > 15 {
        println!("mailbox: create_pair failed (in_h={in_h})");
        exit(11);
    } // ERR_NO_SLOT is > 15
    // The nameserver's inbound channel is passed in the extra fields.
    let ns_in = rt::handle_at(2) as u64;
    if ns_in > 15 {
        println!("mailbox: bad ns_inbound handle ({ns_in})");
        exit(12);
    }
    // Create a reply channel: the nameserver sends the result here, not on its
    // own outbound (which would be shared by all clients and race).
    let reply_chan = ipc::create_chan();
    let mut req = [0u8; NAME.len() + 12];
    unsafe {
        core::ptr::copy_nonoverlapping(NAME.as_ptr(), req.as_mut_ptr(), NAME.len());
        core::ptr::copy_nonoverlapping(
            in_h.to_le_bytes().as_ptr(),
            req.as_mut_ptr().add(NAME.len()),
            4,
        );
        core::ptr::copy_nonoverlapping(
            out_h.to_le_bytes().as_ptr(),
            req.as_mut_ptr().add(NAME.len() + 4),
            4,
        );
        core::ptr::copy_nonoverlapping(
            (reply_chan as u32).to_le_bytes().as_ptr(),
            req.as_mut_ptr().add(NAME.len() + 8),
            4,
        );
    };
    let _ = ipc::send(ns_in, OP_BIND, 0, &req);
    let mut reply = [0u8; 8];
    let _ = ipc::receive(reply_chan, &mut reply);

    // Resolve /dev/console and write the memory info through the console
    // server so it appears on the terminal (not the raw debug UART).
    {
        let reply2 = ipc::create_chan();
        let mut res_req = [0u8; CONSOLE_NAME.len() + 4];
        unsafe {
            core::ptr::copy_nonoverlapping(
                CONSOLE_NAME.as_ptr(),
                res_req.as_mut_ptr(),
                CONSOLE_NAME.len(),
            );
            core::ptr::copy_nonoverlapping(
                (reply2 as u32).to_le_bytes().as_ptr(),
                res_req.as_mut_ptr().add(CONSOLE_NAME.len()),
                4,
            );
        };
        let _ = ipc::send(ns_in, OP_RESOLVE, 0, &res_req);
        let mut res_reply = [0u8; 8];
        let (op, _, _) = ipc::receive(reply2, &mut res_reply);
        if op == R_OK {
            let con_in = u32::from_le_bytes(res_reply[0..4].try_into().unwrap()) as u64;
            let text = format!("Memory: {mem_base:#010x} ({mem_size:#x} bytes)\n");
            let _ = ipc::send(con_in, 0, 0, text.as_bytes());
            let mut ack = [0u8; 4];
            let _ = ipc::receive(out_h, &mut ack);
        }
    }

    loop {
        let mut req_buf = [0u8; 64];
        let (_, bytes, tag) = ipc::receive(in_h, &mut req_buf);
        let n = bytes.min(64);
        let op = u16::from_le_bytes([req_buf[0], req_buf[1]]);
        match op {
            OP_CONFIGURE_FB => {
                if n < 14 {
                    println!("mailbox: short configure request (n={n})");
                    exit(1);
                }
                let w = u32::from_le_bytes([req_buf[2], req_buf[3], req_buf[4], req_buf[5]]);
                let h = u32::from_le_bytes([req_buf[6], req_buf[7], req_buf[8], req_buf[9]]);
                let b = u32::from_le_bytes([req_buf[10], req_buf[11], req_buf[12], req_buf[13]]);
                let (pa, size) = configure_framebuffer(buf_va, buf_pa, w, h, b);
                let mut resp = [0u8; 17];
                resp[1..5].copy_from_slice(&(pa as u32).to_le_bytes());
                resp[5..9].copy_from_slice(&((pa >> 32) as u32).to_le_bytes());
                resp[9..13].copy_from_slice(&(size as u32).to_le_bytes());
                resp[13..17].copy_from_slice(&((size >> 32) as u32).to_le_bytes());
                ipc::reply(out_h, R_OK, tag, &resp);
            }
            OP_GET_PROPERTY => {
                if n < 6 {
                    println!("mailbox: short get_property request (n={n})");
                    exit(1);
                }
                let tag_id = u32::from_le_bytes([req_buf[2], req_buf[3], req_buf[4], req_buf[5]]);
                let val = get_property(buf_va, buf_pa, tag_id);
                let mut resp = [0u8; 5];
                resp[1..5].copy_from_slice(&val.to_le_bytes());
                ipc::reply(out_h, R_OK, tag, &resp);
            }
            _ => {
                ipc::reply(out_h, R_OK, tag, &[1u8]);
            }
        }
    }
}
