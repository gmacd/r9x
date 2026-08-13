use core::fmt;

use crate::reg::esr_el1::EsrEl1;
use crate::{gic, timer};
use port::iprintln;

#[cfg(target_os = "none")]
core::arch::global_asm!(include_str!("trap.S"));

pub fn init() {
    #[cfg(target_os = "none")]
    unsafe {
        // Set up a vector table for any exception that is taken to EL1.
        // IRQs are enabled later in main9 after GIC init completes.
        core::arch::asm!(
            "adr {tmp}, exception_vectors",
            "msr vbar_el1, {tmp}",
            tmp = out(reg) _,
        );
    }
}

/// AArch64 exception vector class, matching the constants in trap.S.
/// Each value corresponds to one of the 16 exception vectors (EL0/EL1 ×
/// Sync/IRQ/FIQ/Error). The raw u64 from trap.S is parsed into this type.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExceptionType {
    SyncInvalidEL1t = 0,
    IrqInvalidEL1t = 1,
    FiqInvalidEL1t = 2,
    ErrorInvalidEL1t = 3,
    SyncInvalidEL1h = 4,
    IrqInvalidEL1h = 5,
    FiqInvalidEL1h = 6,
    ErrorInvalidEL1h = 7,
    SyncInvalidEL0_64 = 8,
    IrqInvalidEL0_64 = 9,
    FiqInvalidEL0_64 = 10,
    ErrorInvalidEL0_64 = 11,
    SyncInvalidEL0_32 = 12,
    IrqInvalidEL0_32 = 13,
    FiqInvalidEL0_32 = 14,
    ErrorInvalidEL0_32 = 15,
}

impl ExceptionType {
    /// Whether this is an IRQ vector (GIC-driven, odd-numbered).
    fn is_irq(self) -> bool {
        matches!(
            self,
            Self::IrqInvalidEL1t
                | Self::IrqInvalidEL1h
                | Self::IrqInvalidEL0_64
                | Self::IrqInvalidEL0_32
        )
    }
}

impl TryFrom<u64> for ExceptionType {
    type Error = ();
    fn try_from(v: u64) -> Result<Self, Self::Error> {
        Ok(match v {
            0 => Self::SyncInvalidEL1t,
            1 => Self::IrqInvalidEL1t,
            2 => Self::FiqInvalidEL1t,
            3 => Self::ErrorInvalidEL1t,
            4 => Self::SyncInvalidEL1h,
            5 => Self::IrqInvalidEL1h,
            6 => Self::FiqInvalidEL1h,
            7 => Self::ErrorInvalidEL1h,
            8 => Self::SyncInvalidEL0_64,
            9 => Self::IrqInvalidEL0_64,
            10 => Self::FiqInvalidEL0_64,
            11 => Self::ErrorInvalidEL0_64,
            12 => Self::SyncInvalidEL0_32,
            13 => Self::IrqInvalidEL0_32,
            14 => Self::FiqInvalidEL0_32,
            15 => Self::ErrorInvalidEL0_32,
            _ => return Err(()),
        })
    }
}

/// Register frame at time interrupt was taken
#[repr(C, align(16))]
pub struct TrapFrame {
    x0: u64,
    x1: u64,
    x2: u64,
    x3: u64,
    x4: u64,
    x5: u64,
    x6: u64,
    x7: u64,
    x8: u64,
    x9: u64,
    x10: u64,
    x11: u64,
    x12: u64,
    x13: u64,
    x14: u64,
    x15: u64,
    x16: u64,
    x17: u64,
    x18: u64,
    x19: u64,
    x20: u64,
    x21: u64,
    x22: u64,
    x23: u64,
    x24: u64,
    x25: u64,
    x26: u64,
    x27: u64,
    x28: u64,
    frame_pointer: u64, // x29
    link_register: u64, // x30
    esr_el1: EsrEl1,
    elr_el1: u64,
    far_el1: u64,
    interrupt_type: u64,
}

impl fmt::Debug for TrapFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrapFrame")
            .field("x0", &format_args!("{:#018x}", self.x0))
            .field("x1", &format_args!("{:#018x}", self.x1))
            .field("x2", &format_args!("{:#018x}", self.x2))
            .field("x3", &format_args!("{:#018x}", self.x3))
            .field("x4", &format_args!("{:#018x}", self.x4))
            .field("x5", &format_args!("{:#018x}", self.x5))
            .field("x6", &format_args!("{:#018x}", self.x6))
            .field("x7", &format_args!("{:#018x}", self.x7))
            .field("x8", &format_args!("{:#018x}", self.x8))
            .field("x9", &format_args!("{:#018x}", self.x9))
            .field("x10", &format_args!("{:#018x}", self.x10))
            .field("x11", &format_args!("{:#018x}", self.x11))
            .field("x12", &format_args!("{:#018x}", self.x12))
            .field("x13", &format_args!("{:#018x}", self.x13))
            .field("x14", &format_args!("{:#018x}", self.x14))
            .field("x15", &format_args!("{:#018x}", self.x15))
            .field("x16", &format_args!("{:#018x}", self.x16))
            .field("x17", &format_args!("{:#018x}", self.x17))
            .field("x18", &format_args!("{:#018x}", self.x18))
            .field("x19", &format_args!("{:#018x}", self.x19))
            .field("x20", &format_args!("{:#018x}", self.x20))
            .field("x21", &format_args!("{:#018x}", self.x21))
            .field("x22", &format_args!("{:#018x}", self.x22))
            .field("x23", &format_args!("{:#018x}", self.x23))
            .field("x24", &format_args!("{:#018x}", self.x24))
            .field("x25", &format_args!("{:#018x}", self.x25))
            .field("x26", &format_args!("{:#018x}", self.x26))
            .field("x27", &format_args!("{:#018x}", self.x27))
            .field("x28", &format_args!("{:#018x}", self.x28))
            .field("x29 (frame_pointer)", &format_args!("{:#018x}", self.frame_pointer))
            .field("x30 (link_register)", &format_args!("{:#018x}", self.link_register))
            .field("esr_el1", &format_args!("{:#?}", self.esr_el1))
            .field("elr_el1", &format_args!("{:#018?}", self.elr_el1))
            .field("far_el1", &format_args!("{:#018?}", self.far_el1))
            .field("interrupt_type", &self.interrupt_type_name())
            .finish()
    }
}

impl TrapFrame {
    /// Returns the exception type name for this frame's interrupt_type.
    fn interrupt_type_name(&self) -> &'static str {
        match ExceptionType::try_from(self.interrupt_type) {
            Ok(ExceptionType::SyncInvalidEL1t) => "SyncInvalidEL1t",
            Ok(ExceptionType::IrqInvalidEL1t) => "IrqInvalidEL1t",
            Ok(ExceptionType::FiqInvalidEL1t) => "FiqInvalidEL1t",
            Ok(ExceptionType::ErrorInvalidEL1t) => "ErrorInvalidEL1t",
            Ok(ExceptionType::SyncInvalidEL1h) => "SyncInvalidEL1h",
            Ok(ExceptionType::IrqInvalidEL1h) => "IrqInvalidEL1h",
            Ok(ExceptionType::FiqInvalidEL1h) => "FiqInvalidEL1h",
            Ok(ExceptionType::ErrorInvalidEL1h) => "ErrorInvalidEL1h",
            Ok(ExceptionType::SyncInvalidEL0_64) => "SyncInvalidEL0_64",
            Ok(ExceptionType::IrqInvalidEL0_64) => "IrqInvalidEL0_64",
            Ok(ExceptionType::FiqInvalidEL0_64) => "FiqInvalidEL0_64",
            Ok(ExceptionType::ErrorInvalidEL0_64) => "ErrorInvalidEL0_64",
            Ok(ExceptionType::SyncInvalidEL0_32) => "SyncInvalidEL0_32",
            Ok(ExceptionType::IrqInvalidEL0_32) => "IrqInvalidEL0_32",
            Ok(ExceptionType::FiqInvalidEL0_32) => "FiqInvalidEL0_32",
            Ok(ExceptionType::ErrorInvalidEL0_32) => "ErrorInvalidEL0_32",
            Err(()) => "unknown",
        }
    }
}

/// Entry point called from the vector table in trap.S.
///
/// # Safety
/// `frame` must point to a `TrapFrame` the vector code has just pushed,
/// valid for the duration of the call.  Called only from assembly.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trap_unsafe(frame: *mut TrapFrame) {
    port::irq::enter_interrupt();
    unsafe { trap(frame.as_mut().unwrap()) }
    port::irq::exit_interrupt();
}

fn trap(frame: &mut TrapFrame) {
    let exc = match ExceptionType::try_from(frame.interrupt_type) {
        Ok(e) => e,
        Err(()) => {
            iprintln!("Unknown exception type {}", frame.interrupt_type);
            loop {
                core::hint::spin_loop();
            }
        }
    };

    if exc.is_irq() {
        let iar = gic::try_ack_interrupt();
        // AArch64 GIC: IAR = 1023 means no pending interrupt (spurious).
        // This is valid on an IRQ vector and should be a no-op.
        if let Some(iar) = iar {
            match iar.int_id() {
                gic::TIMER_INTID => timer::interrupt_handler(),
                intid => {
                    iprintln!("Unhandled GIC IRQ {intid}");
                    // Disable to avoid repeated unhandled interrupts
                    gic::disable_interrupt(intid);
                }
            }
            gic::end_interrupt(iar);
        }
        return;
    }

    match frame.esr_el1.ec() {
        0x15 => {
            // Syscall
            let syscallid = frame.esr_el1.iss();
            iprintln!("Syscall {syscallid}");
        }
        _ => {
            // Synchronous exception (data abort, instruction abort, etc.)
            iprintln!("{:#?}", frame);
            iprintln!("Unhandled exception");
        }
    }

    loop {
        core::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests {
    use super::ExceptionType;

    #[test]
    fn exception_type_try_from_all_values() {
        for v in 0..=15 {
            let ty = ExceptionType::try_from(v);
            assert!(ty.is_ok(), "value {v} should map to an ExceptionType; got {:?}", ty);
        }
        assert_eq!(ExceptionType::try_from(16), Err(()));
        assert_eq!(ExceptionType::try_from(u64::MAX), Err(()));
    }

    #[test]
    fn exception_type_is_irq() {
        // IRQ vectors are odd: 1, 5, 9, 13
        for v in [1u64, 5, 9, 13] {
            assert!(ExceptionType::try_from(v).unwrap().is_irq(), "{v} should be an IRQ vector");
        }
        // All other vectors are non-IRQ
        for v in [0, 2, 3, 4, 6, 7, 8, 10, 11, 12, 14, 15] {
            assert!(
                !ExceptionType::try_from(v).unwrap().is_irq(),
                "{v} should not be an IRQ vector"
            );
        }
    }

    #[test]
    fn exception_type_repr_is_u8() {
        assert_eq!(core::mem::size_of::<ExceptionType>(), 1);
    }

    #[test]
    fn exception_type_debug_name() {
        // Verify the Debug derive gives readable output
        let t = ExceptionType::try_from(10).unwrap();
        let debug_str = format!("{:?}", t);
        assert_eq!(debug_str, "FiqInvalidEL0_64");
    }
}
