#![allow(unused_variables, dead_code)]

/// Write a byte to an I/O port.
///
/// # Safety
/// A port write is a device command: the port must be one this kernel owns
/// and the value one that device expects. Nothing here can check either.
pub unsafe fn outb(port: u16, b: u8) {
    #[cfg(target_os = "none")]
    unsafe {
        core::arch::asm!("outb %al, %dx", in("dx") port, in("al") b, options(att_syntax));
    }
}

/// Write a word to an I/O port.
///
/// # Safety
/// A port write is a device command: the port must be one this kernel owns
/// and the value one that device expects. Nothing here can check either.
pub unsafe fn outw(port: u16, w: u16) {
    #[cfg(target_os = "none")]
    unsafe {
        core::arch::asm!("outw %ax, %dx", in("dx") port, in("ax") w, options(att_syntax));
    }
}

/// Write a long to an I/O port.
///
/// # Safety
/// A port write is a device command: the port must be one this kernel owns
/// and the value one that device expects. Nothing here can check either.
pub unsafe fn outl(port: u16, l: u32) {
    #[cfg(target_os = "none")]
    unsafe {
        core::arch::asm!("outl %eax, %dx", in("dx") port, in("ax") l, options(att_syntax));
    }
}
