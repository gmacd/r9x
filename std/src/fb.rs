//! The framebuffer: configure the VideoCore framebuffer and get access to
//! the kernel-mapped region.
//!
//! The display server calls [`configure`] during its initialization.  The
//! kernel sends the Mailbox `SET_*` + `ALLOCATE` sequence and maps the
//! returned physical address at [`r9x_abi::FB_VA`] with Device memory
//! attributes.  The server then writes pixels to [`r9x_abi::FB_VA`].

use crate::sys;

/// Configure the VideoCore framebuffer and map it into this process's page
/// table.  Returns 0 on success, 1 if already configured.
pub fn configure() -> u64 {
    // SAFETY: no arguments; the kernel reads no user buffers.
    let (result, _, _) = unsafe { sys::sys(r9x_abi::SYS_FB_CONFIGURE, 0, 0, 0, 0, 0) };
    result
}
