use core::ptr::NonNull;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::deviceutil::{find_dt_physrange, map_device_register};
use crate::io::{read_reg, write_reg};
use crate::vm;
use port::Result;
use port::mcslock::{Lock, LockNode};
use port::mem::{PhysAddr, PhysRange, VirtRange};
use r9x_core::fdt::DeviceTree;

use port::iprintln;

const MBOX_READ: usize = 0x00;
const MBOX_STATUS: usize = 0x18;
const MBOX_WRITE: usize = 0x20;

const MBOX_FULL: u32 = 0x8000_0000;
const MBOX_EMPTY: u32 = 0x4000_0000;

static MAILBOX: Lock<Option<Mailbox>> = Lock::new("mailbox", None);

/// Mailbox init.  Mainly initialises a lock to ensure only one mailbox request
/// can be made at a time.
pub fn init(dt: &DeviceTree) {
    match Mailbox::new(dt) {
        Ok(mbox) => {
            let node = LockNode::new();
            let mut mailbox = MAILBOX.lock(&node);
            *mailbox = Some(mbox);
        }
        Err(msg) => {
            iprintln!("can't initialise mailbox: {:?}", msg);
        }
    }
}

/// https://developer.arm.com/documentation/ddi0306/b/CHDGHAIG
/// https://github.com/raspberrypi/firmware/wiki/Mailbox-property-interface
struct Mailbox {
    mbox_virtrange: VirtRange,
    req_buf_virtrange: VirtRange,
    req_buf_physrange: PhysRange,
}

impl Mailbox {
    fn new(dt: &DeviceTree) -> Result<Self> {
        // Allocate a page of device memory for the mailbox request/response buffer
        // TODO Split this into multiple buffers to allow parallel requests.
        let (req_buf_virtrange, req_buf_physrange) =
            crate::deviceutil::alloc_device_page("mailboxbuf", vm::PageSize::Page4K)?;

        let mbox_physrange = find_dt_physrange(dt, &["brcm,bcm2835-mbox"], "can't find mbox")?;
        let mbox = match map_device_register("mailbox", mbox_physrange, vm::PageSize::Page4K) {
            Ok(mbox_virtrange) => {
                Ok(Mailbox { mbox_virtrange, req_buf_virtrange, req_buf_physrange })
            }
            Err(msg) => {
                iprintln!("can't map mailbox {:?}", msg);
                Err("can't create mailbox")
            }
        }?;

        Ok(mbox)
    }

    fn request(&self) {
        // Read status register until full flag not set
        while (read_reg(&self.mbox_virtrange, MBOX_STATUS) & MBOX_FULL) != 0 {}

        // Write the request address combined with the channel to the write register
        let channel = ChannelId::ArmToVc as u32;
        let uart_mbox_u32 = self.req_buf_physrange.start.addr() as u32;
        let r = (uart_mbox_u32 & !0xF) | channel;
        write_reg(&self.mbox_virtrange, MBOX_WRITE, r);

        // Wait for response
        // FIXME: two infinite loops - could go awry
        loop {
            while (read_reg(&self.mbox_virtrange, MBOX_STATUS) & MBOX_EMPTY) != 0 {}
            let response = read_reg(&self.mbox_virtrange, MBOX_READ);
            if response == r {
                break;
            }
        }
    }
}

#[repr(u8)]
enum ChannelId {
    ArmToVc = 8,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Request<T> {
    size: u32, // size in bytes
    code: u32, // request code (0)
    tags: T,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Response<T> {
    size: u32, // size in bytes
    code: u32, // response code
    tags: T,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Tag<T> {
    tag_id0: TagId,
    tag_buffer_size0: u32,
    tag_code0: u32,
    body: T,
    end_tag: u32,
}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
union Message<T: Copy, U: Copy> {
    request: Request<T>,
    response: Response<U>,
}

type MessageWithTags<T, U> = Message<Tag<T>, Tag<U>>;

fn request<T, U>(code: u32, tags: &Tag<T>) -> U
where
    T: Copy,
    U: Copy,
{
    let size = size_of::<Message<T, U>>() as u32;
    let node = LockNode::new();
    MAILBOX
        .lock(&node)
        .as_mut()
        .map(|mb| {
            let msg = unsafe {
                let page_va_ptr = mb.req_buf_virtrange.start as u64 as *mut MessageWithTags<T, U>;
                core::intrinsics::volatile_set_memory(page_va_ptr, 0, 1);
                let msg = NonNull::new_unchecked(page_va_ptr).as_mut();
                msg.request.size = size;
                msg.request.code = code;
                msg.request.tags = *tags;
                msg
            };

            mb.request();
            unsafe { msg.response.tags.body }
        })
        .expect("mailbox not initialised")
}

// https://github.com/raspberrypi/firmware/wiki/Mailbox-property-interface#tags-arm-to-vc
#[repr(u32)]
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
enum TagId {
    // Framebuffer tags (channel 4, per QEMU's raspberrypi-fw-defs.h).
    FbAllocate = 0x0004_0001,
    FbRelease = 0x0004_8001,
    FbSetPhysicalWidthHeight = 0x0004_8003,
    FbSetDepth = 0x0004_8005,
    FbSetPixelOrder = 0x0004_8006,
    FbGetPitch = 0x0004_0008,
    GetFirmwareRevision = 0x0000_0001,
    GetBoardModel = 0x0001_0001,
    GetBoardRevision = 0x0001_0002,
    GetBoardMacAddress = 0x0001_0003,
    GetBoardSerial = 0x0001_0004,
    GetArmMemory = 0x0001_0005,
    GetVcMemory = 0x0001_0006,
    SetClockRate = 0x0003_8002,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct SetClockRateRequest {
    clock_id: u32,
    rate_hz: u32,
    skip_setting_turbo: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct SetClockRateResponse {
    clock_id: u32,
    rate_hz: u32,
}

#[allow(dead_code)]
pub fn set_clock_rate(clock_id: u32, rate_hz: u32, skip_setting_turbo: u32) {
    let tags = Tag::<SetClockRateRequest> {
        tag_id0: TagId::SetClockRate,
        tag_buffer_size0: 12,
        tag_code0: 0,
        body: SetClockRateRequest { clock_id, rate_hz, skip_setting_turbo },
        end_tag: 0,
    };
    let _: SetClockRateResponse = request(0, &tags);
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct EmptyRequest {}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct MemoryResponse {
    base_addr: u32,
    size: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct MemoryInfo {
    pub start: u32,
    pub size: u32,
    pub end: u32,
}

#[allow(dead_code)]
pub fn get_arm_memory() -> PhysRange {
    let tags = Tag::<EmptyRequest> {
        tag_id0: TagId::GetArmMemory,
        tag_buffer_size0: 12,
        tag_code0: 0,
        body: EmptyRequest {},
        end_tag: 0,
    };
    let res: MemoryResponse = request(0, &tags);
    let start = res.base_addr;
    let size = res.size;
    let end = start + size;

    PhysRange::new(PhysAddr::new(start as u64), PhysAddr::new(end as u64))
}

#[allow(dead_code)]
pub fn get_vc_memory() -> PhysRange {
    let tags = Tag::<EmptyRequest> {
        tag_id0: TagId::GetVcMemory,
        tag_buffer_size0: 12,
        tag_code0: 0,
        body: EmptyRequest {},
        end_tag: 0,
    };
    let res: MemoryResponse = request(0, &tags);
    let start = res.base_addr;
    let size = res.size;
    let end = start + size;

    PhysRange::new(PhysAddr::new(start as u64), PhysAddr::new(end as u64))
}

pub fn get_firmware_revision() -> u32 {
    let tags = Tag::<EmptyRequest> {
        tag_id0: TagId::GetFirmwareRevision,
        tag_buffer_size0: 4,
        tag_code0: 0,
        body: EmptyRequest {},
        end_tag: 0,
    };
    request::<_, u32>(0, &tags)
}

pub fn get_board_model() -> u32 {
    let tags = Tag::<EmptyRequest> {
        tag_id0: TagId::GetBoardModel,
        tag_buffer_size0: 4,
        tag_code0: 0,
        body: EmptyRequest {},
        end_tag: 0,
    };
    request::<_, u32>(0, &tags)
}

pub fn get_board_revision() -> u32 {
    let tags = Tag::<EmptyRequest> {
        tag_id0: TagId::GetBoardRevision,
        tag_buffer_size0: 4,
        tag_code0: 0,
        body: EmptyRequest {},
        end_tag: 0,
    };
    request::<_, u32>(0, &tags)
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MacAddress {
    pub a: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub f: u8,
}

pub fn get_board_macaddr() -> MacAddress {
    let tags = Tag::<EmptyRequest> {
        tag_id0: TagId::GetBoardMacAddress,
        tag_buffer_size0: 6,
        tag_code0: 0,
        body: EmptyRequest {},
        end_tag: 0,
    };
    request::<_, MacAddress>(0, &tags)
}

pub fn get_board_serial() -> u64 {
    let tags = Tag::<EmptyRequest> {
        tag_id0: TagId::GetBoardSerial,
        tag_buffer_size0: 8,
        tag_code0: 0,
        body: EmptyRequest {},
        end_tag: 0,
    };
    // FIXME: Treating this a `u64` gets us a memory address. Pointer fun ahead.
    // Wrapping in a struct holding a single u64 doesn't work either.
    let res: [u32; 2] = request(0, &tags);
    ((res[0] as u64) << 32) | res[1] as u64
}

/// The physical address of the framebuffer (set by `configure_framebuffer`).
/// Zero when the framebuffer is not configured.
static FB_PHYS: AtomicU64 = AtomicU64::new(0);

/// Configure the VideoCore framebuffer via the Mailbox property interface.
///
/// The QEMU firmware (and the real Pi firmware) copies the current config
/// at the start of each request.  The `ALLOCATE` tag reads `fbconfig.base`
/// from that copy, so a single request with `SET_*` + `ALLOCATE` returns the
/// *previous* config's base (0 on first call).  We therefore send two
/// requests:
///
/// 1. `SET_PHYSICAL_WIDTH_HEIGHT`, `SET_DEPTH`, `SET_PIXEL_ORDER` — the
///    firmware validates and reconfigures the framebuffer (allocating the
///    buffer in VC RAM).
/// 2. `ALLOCATE` — the firmware reads the now-current config's base address
///    and size.
///
/// Stores the physical address in `FB_PHYS` and returns the physical range.
#[allow(dead_code)]
pub fn configure_framebuffer(width: u32, height: u32, bpp: u32) -> PhysRange {
    let node = LockNode::new();
    MAILBOX
        .lock(&node)
        .as_mut()
        .map(|mb| {
            let base = mb.req_buf_virtrange.start as u64 as *mut u32;
            // SAFETY: base is a valid device-memory pointer to the mailbox
            // request buffer, a full page, and the mailbox is locked.
            unsafe {
                // --- Request 1: configure the framebuffer ---
                // Tag layout (all little-endian u32 words):
                //   word 0:  size = 60
                //   word 1:  code = 0 (request)
                //   word 2:  SET_PHYSICAL_WIDTH_HEIGHT tag_id
                //   word 3:  buf_size = 8
                //   word 4:  code = 0 (request)
                //   word 5:  width
                //   word 6:  height
                //   word 7:  SET_DEPTH tag_id
                //   word 8:  buf_size = 4
                //   word 9:  code = 0 (request)
                //   word 10: bpp
                //   word 11: SET_PIXEL_ORDER tag_id
                //   word 12: buf_size = 4
                //   word 13: code = 0 (request)
                //   word 14: pixel_order = 0 (XRGB)
                //   word 15: end tag = 0
                const CFG_SIZE: u32 = 64;
                core::intrinsics::volatile_set_memory(base as *mut u8, 0, CFG_SIZE as usize);
                core::intrinsics::volatile_store(base.add(0), CFG_SIZE);
                core::intrinsics::volatile_store(base.add(1), 0);
                core::intrinsics::volatile_store(
                    base.add(2),
                    TagId::FbSetPhysicalWidthHeight as u32,
                );
                core::intrinsics::volatile_store(base.add(3), 8);
                core::intrinsics::volatile_store(base.add(4), 0);
                core::intrinsics::volatile_store(base.add(5), width);
                core::intrinsics::volatile_store(base.add(6), height);
                core::intrinsics::volatile_store(base.add(7), TagId::FbSetDepth as u32);
                core::intrinsics::volatile_store(base.add(8), 4);
                core::intrinsics::volatile_store(base.add(9), 0);
                core::intrinsics::volatile_store(base.add(10), bpp);
                core::intrinsics::volatile_store(base.add(11), TagId::FbSetPixelOrder as u32);
                core::intrinsics::volatile_store(base.add(12), 4);
                core::intrinsics::volatile_store(base.add(13), 0);
                core::intrinsics::volatile_store(base.add(14), 0); // XRGB
                core::intrinsics::volatile_store(base.add(15), 0); // end tag

                mb.request();

                // --- Request 2: allocate (read the base address) ---
                // Tag layout:
                //   word 0:  size = 20
                //   word 1:  code = 0 (request)
                //   word 2:  ALLOCATE tag_id
                //   word 3:  buf_size = 8 (response: phys_addr + size)
                //   word 4:  code = 0 (request)
                //   word 5:  (response: phys_addr)
                //   word 6:  (response: size)
                //   word 7:  end tag = 0
                const ALLOC_SIZE: u32 = 32;
                core::intrinsics::volatile_set_memory(base as *mut u8, 0, ALLOC_SIZE as usize);
                core::intrinsics::volatile_store(base.add(0), ALLOC_SIZE);
                core::intrinsics::volatile_store(base.add(1), 0);
                core::intrinsics::volatile_store(base.add(2), TagId::FbAllocate as u32);
                core::intrinsics::volatile_store(base.add(3), 8); // response body size
                core::intrinsics::volatile_store(base.add(4), 0); // request
                // words 5-6: response (phys_addr, size) — written by firmware
                core::intrinsics::volatile_store(base.add(7), 0); // end tag

                mb.request();

                // Read the ALLOCATE response: phys_addr at word 5, size at word 6.
                let phys_addr = core::intrinsics::volatile_load(base.add(5));
                let size = core::intrinsics::volatile_load(base.add(6));
                FB_PHYS.store(phys_addr as u64, Ordering::SeqCst);
                iprintln!("  Framebuffer: {phys_addr:#010x} ({size:#x} bytes)");
                PhysRange::new(
                    PhysAddr::new(phys_addr as u64),
                    PhysAddr::new(phys_addr as u64 + size as u64),
                )
            }
        })
        .expect("mailbox not initialised")
}
