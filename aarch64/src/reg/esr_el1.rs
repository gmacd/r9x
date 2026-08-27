use core::fmt;

use bitstruct::bitstruct;
use num_enum::TryFromPrimitive;

bitstruct! {
    #[derive(Copy, Clone)]
    pub struct EsrEl1(pub u64) {
        pub iss: u32 = 0..25;
        pub il: bool = 25;
        pub ec: u8 = 26..32;
        pub iss2: u8 = 32..37;
    }
}

impl EsrEl1 {
    /// Try to convert the error into an ExceptionClass enum, or return the original number
    /// as the error.
    pub fn exception_class_enum(&self) -> Result<ExceptionClass, u8> {
        ExceptionClass::try_from(self.ec()).map_err(|e| e.number)
    }

    /// The fault status code (DFSC for a Data Abort, IFSC for an Instruction
    /// Abort — the same ISS[5:0] field either way), decoded to a class string:
    /// the way Linux's `fault_info` table does.  A translation/access-flag/
    /// permission fault carries its walk level; a synchronous external abort
    /// and an alignment fault are single classes; anything else is
    /// `"unknown"` (the caller prints the raw code alongside).
    pub fn fault_status_str(&self) -> &'static str {
        fault_status_str((self.iss() & 0x3f) as u8)
    }
}

/// Decode a fault status code (DFSC/IFSC, ISS[5:0]) into a class string.
///
/// The encoding is the AArch64 shared fault-status-code space:
/// `0b0001xx` translation fault at level `xx`, `0b0010xx` access-flag fault at
/// level `xx`, `0b0011xx` permission fault at level `xx`, `0b010000`
/// synchronous external abort (not on the walk), and `0b100001` alignment
/// fault.  Everything else is `"unknown"` — the caller prints the raw code.
pub fn fault_status_str(fsc: u8) -> &'static str {
    match fsc {
        0b000100 => "translation fault L0",
        0b000101 => "translation fault L1",
        0b000110 => "translation fault L2",
        0b000111 => "translation fault L3",
        0b001000 => "access flag fault L0",
        0b001001 => "access flag fault L1",
        0b001010 => "access flag fault L2",
        0b001011 => "access flag fault L3",
        0b001100 => "permission fault L0",
        0b001101 => "permission fault L1",
        0b001110 => "permission fault L2",
        0b001111 => "permission fault L3",
        0b010000 => "external abort",
        0b100001 => "alignment fault",
        _ => "unknown",
    }
}

impl fmt::Debug for EsrEl1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EsrEl1")
            .field("iss", &format_args!("{:#010x}", self.iss()))
            .field("il", &format_args!("{}", self.il()))
            .field("ec", &format_args!("{:?}", self.exception_class_enum()))
            .field("iss2", &format_args!("{:#04x}", self.iss2()))
            .finish()
    }
}

/// Exception class maps to ESR_EL1 EC bits[31:26]. We skip aarch32 exceptions.
#[derive(Debug, Eq, PartialEq, TryFromPrimitive)]
#[repr(u8)]
pub enum ExceptionClass {
    Unknown = 0,
    WaitFor = 1,
    FloatSimd = 7,
    Ls64 = 10,
    BranchTargetException = 13,
    IllegalExecutionState = 14,
    SvcAarch64 = 21,
    MsrMrsSystem = 24,
    Sve = 25,
    Tstart = 27,
    PointerAuthFailure = 28,
    Sme = 29,
    GranuleProtectionCheck = 30,
    InstructionAbortLowerEl = 32,
    InstructionAbortSameEl = 33,
    PcAlignmentFault = 34,
    DataAbortLowerEl = 36,
    DataAbortSameEl = 37,
    SpAlignmentFault = 38,
    MemoryOperationException = 39,
    TrappedFloatingPointException = 44,
    SError = 47,
    BreakpointLowerEl = 48,
    BreakpointSameEl = 49,
    SoftwareStepLowerEl = 50,
    SoftwareStepSameEl = 51,
    WatchpointLowerEl = 52,
    WatchpointSameEl = 53,
    Brk = 60,
}

bitstruct! {
    #[derive(Copy, Clone)]
    pub struct EsrEl1IssInstructionAbort(pub u32) {
        ifsc: u8 = 0..6;
        s1ptw: bool = 7;
        ea: bool = 9;
        fnv: bool = 10;
        set: u8 = 11..13;
    }
}

#[allow(dead_code)]
impl EsrEl1IssInstructionAbort {
    pub fn from_esr_el1(r: EsrEl1) -> Option<EsrEl1IssInstructionAbort> {
        r.exception_class_enum()
            .ok()
            .filter(|ec| *ec == ExceptionClass::InstructionAbortSameEl)
            .map(|_| EsrEl1IssInstructionAbort(r.iss()))
    }

    pub fn instruction_fault(&self) -> Result<InstructionFaultStatusCode, u8> {
        InstructionFaultStatusCode::try_from(self.ifsc()).map_err(|e| e.number)
    }
}

#[derive(Debug, Eq, PartialEq, TryFromPrimitive)]
#[repr(u8)]
pub enum InstructionFaultStatusCode {
    AddressSizeFaultLevel0 = 0,
    AddressSizeFaultLevel1 = 1,
    AddressSizeFaultLevel2 = 2,
    AddressSizeFaultLevel3 = 3,
    TranslationFaultLevel0 = 4,
    TranslationFaultLevel1 = 5,
    TranslationFaultLevel2 = 6,
    TranslationFaultLevel3 = 7,
    AccessFlagFaultLevel0 = 8,
    AccessFlagFaultLevel1 = 9,
    AccessFlagFaultLevel2 = 10,
    AccessFlagFaultLevel3 = 11,
    PermissionFaultLevel0 = 12,
    PermissionFaultLevel1 = 13,
    PermissionFaultLevel2 = 14,
    PermissionFaultLevel3 = 15,
    SyncExtAbortNotOnWalkOrUpdate = 16,
    SyncExtAbortOnWalkOrUpdateLevelNeg1 = 19,
    SyncExtAbortOnWalkOrUpdateLevel0 = 20,
    SyncExtAbortOnWalkOrUpdateLevel1 = 21,
    SyncExtAbortOnWalkOrUpdateLevel2 = 22,
    SyncExtAbortOnWalkOrUpdateLevel3 = 23,
    SyncParityOrEccErrOnMemAccessNotOnWalk = 24,
    SyncParityOrEccErrOnMemAccessOnWalkOrUpdateLevelNeg1 = 27,
    SyncParityOrEccErrOnMemAccessOnWalkOrUpdateLevel0 = 28,
    SyncParityOrEccErrOnMemAccessOnWalkOrUpdateLevel1 = 29,
    SyncParityOrEccErrOnMemAccessOnWalkOrUpdateLevel2 = 30,
    SyncParityOrEccErrOnMemAccessOnWalkOrUpdateLevel3 = 31,
    GranuleProtectFaultOnWalkOrUpdateLevelNeg1 = 35,
    GranuleProtectFaultOnWalkOrUpdateLevel0 = 36,
    GranuleProtectFaultOnWalkOrUpdateLevel1 = 37,
    GranuleProtectFaultOnWalkOrUpdateLevel2 = 38,
    GranuleProtectFaultOnWalkOrUpdateLevel3 = 39,
    GranuleProtectFaultNotOnWalkOrUpdateLevel = 40,
    AddressSizeFaultLevelNeg1 = 41,
    TranslationFaultLevelNeg1 = 43,
    TlbConflictAbort = 48,
    UnsupportedAtomicHardwareUpdateFault = 49,
}

#[cfg(test)]
mod tests {
    use super::*;

    // This test is useful for making sense of early-stage exceptions.  Qemu
    // will report an exception of the form below.  Copy the ESR value into
    // this test to break it down.
    //
    // Exception return from AArch64 EL2 to AArch64 EL1 PC 0x8006c
    // Taking exception 3 [Prefetch Abort] on CPU 0
    // ...from EL1 to EL1
    // ...with ESR 0x21/0x86000004
    // ...with FAR 0x80090
    // ...with ELR 0x80090
    // ...to EL1 PC 0x200 PSTATE 0x3c5
    #[test]
    fn test_parse_esr_el1() {
        let r = EsrEl1(0x86000004);
        assert_eq!(r.exception_class_enum().unwrap(), ExceptionClass::InstructionAbortSameEl);
        assert_eq!(
            EsrEl1IssInstructionAbort::from_esr_el1(r).unwrap().instruction_fault().unwrap(),
            InstructionFaultStatusCode::TranslationFaultLevel0
        );
    }

    // Pins the DFSC/IFSC decode so a fault printer wrong in the same way as
    // the raw ISS print (the 87 misdiagnosis) is caught here, not in the field.
    #[test]
    fn test_fault_status_str() {
        assert_eq!(fault_status_str(0b000100), "translation fault L0");
        assert_eq!(fault_status_str(0b000101), "translation fault L1");
        assert_eq!(fault_status_str(0b000110), "translation fault L2");
        assert_eq!(fault_status_str(0b000111), "translation fault L3");
        assert_eq!(fault_status_str(0b001001), "access flag fault L1");
        assert_eq!(fault_status_str(0b001110), "permission fault L2");
        assert_eq!(fault_status_str(0b010000), "external abort");
        assert_eq!(fault_status_str(0b100001), "alignment fault");
        assert_eq!(fault_status_str(0b101010), "unknown");
    }
}
