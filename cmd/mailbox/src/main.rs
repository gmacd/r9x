//! The mailbox server: owns the BCM283x Mailbox property interface.
#![no_std]
#![no_main]
extern crate alloc;
use r9x_std::ipc;
use r9x_std::mem;
use r9x_std::process::exit;
use r9x_std::rt;

/// The page-aligned base of the Mailbox register page.  The Mailbox
/// registers start at offset 0xB8 within this page.
const MBOX_PAGE_PHYS: u64 = 0xfe00_0000;
/// The Mailbox registers' offset within the mapped page (0xB8 / 4 = 29 words).
const MBOX_REG_BASE: usize = 0xB8 / 4;
const MBOX_VA: u64 = 0x7000_0000;
const MBOX_STATUS: usize = MBOX_REG_BASE + 6;
const MBOX_WRITE: usize = MBOX_REG_BASE + 8;
const MBOX_READ: usize = MBOX_REG_BASE;
const MBOX_FULL: u32 = 0x8000_0000;
const MBOX_EMPTY: u32 = 0x4000_0000;
const CHANNEL_ARM_TO_VC: u32 = 8;
#[allow(dead_code)]
const TAG_FB_ALLOCATE: u32 = 0x0004_0001;
const TAG_FB_SET_WIDTH_HEIGHT: u32 = 0x0004_8003;
const TAG_FB_SET_DEPTH: u32 = 0x0004_8005;
const TAG_FB_SET_PIXEL_ORDER: u32 = 0x0004_8006;
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
const R_OK: u16 = 0;
const NAME: &[u8] = b"/dev/mailbox";

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

fn main() {
    let m = mem::map_mmio(MBOX_PAGE_PHYS, MBOX_VA, 4096);
    if m != 0 {
        exit(10);
    }
    let (buf_va, buf_pa) = match mem::alloc_page() {
        Some((va, pa)) => (va, pa),
        None => exit(1),
    };
    let (in_h, out_h) = ipc::create_pair();
    if in_h > 15 {
        exit(11);
    } // ERR_NO_SLOT is > 15
    // The nameserver's handles are passed in the extra fields (the kernel-created
    // pair in `inbound`/`outbound` is unused — the server makes its own pair).
    let ns_in = rt::handle_at(2) as u64;
    let ns_out = rt::handle_at(3) as u64;
    if ns_in > 15 && ns_out > 15 {
        exit(12);
    }
    let mut req = [0u8; NAME.len() + 8];
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
    };
    let _ = ipc::send(ns_in, OP_BIND, 0, &req);
    let mut reply = [0u8; 8];
    let _ = ipc::receive(ns_out, &mut reply);
    loop {
        let mut req_buf = [0u8; 64];
        let (_, bytes, tag) = ipc::receive(in_h, &mut req_buf);
        let n = bytes.min(64);
        let op = u16::from_le_bytes([req_buf[0], req_buf[1]]);
        match op {
            OP_CONFIGURE_FB => {
                if n < 14 {
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
