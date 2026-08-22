use core::fmt;

use aarch64_cpu::asm::barrier;
#[cfg(not(test))]
use aarch64_cpu::registers::{CNTFRQ_EL0, CNTPCT_EL0};
use aarch64_cpu::registers::{CNTKCTL_EL1, CNTP_CTL_EL0, CNTP_CVAL_EL0, Readable, Writeable};
use bitstruct::bitstruct;

// CNTPCT_EL0 — 64-bit physical counter
#[derive(Copy, Clone, Default)]
pub struct CntPctEl0(u64);

#[cfg(test)]
static MOCK_COUNT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

#[cfg(test)]
pub fn set_mock_count(count: u64) {
    MOCK_COUNT.store(count, core::sync::atomic::Ordering::Relaxed);
}

impl CntPctEl0 {
    #[cfg(test)]
    pub fn read() -> Self {
        Self(MOCK_COUNT.load(core::sync::atomic::Ordering::Relaxed))
    }

    #[cfg(not(test))]
    pub fn read() -> Self {
        Self({
            // Reads of the physical counter can be speculatively early;
            // a context synchronisation barrier ensures we read the
            // value as it is after all preceding instructions.
            barrier::isb(barrier::SY);
            CNTPCT_EL0.extract().into()
        })
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for CntPctEl0 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CntPctEl0").field(&format_args!("{}", self.0)).finish()
    }
}

// CNTFRQ_EL0 — timer frequency in Hz
#[derive(Copy, Clone, Default)]
pub struct CntFrqEl0(u64);

// Defaults to 0, i.e. "firmware never programmed the register", so a
// test must opt in to a working frequency.
#[cfg(test)]
static MOCK_FREQ: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

#[cfg(test)]
pub fn set_mock_freq(freq: u64) {
    MOCK_FREQ.store(freq, core::sync::atomic::Ordering::Relaxed);
}

impl CntFrqEl0 {
    #[cfg(test)]
    pub fn read() -> Self {
        Self(MOCK_FREQ.load(core::sync::atomic::Ordering::Relaxed))
    }

    #[cfg(not(test))]
    pub fn read() -> Self {
        Self(CNTFRQ_EL0.extract().into())
    }

    pub fn freq(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for CntFrqEl0 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CntFrqEl0").field("hz", &format_args!("{}", self.0)).finish()
    }
}

// CNTKCTL_EL1 — counter-timer control: which counter registers EL0
// may access.  Reset value is 0, which traps EL0 reads of CNTPCT_EL0
// and CNTFRQ_EL0, so a process cannot pace itself against the real
// clock without the kernel opting in.  The only bits the kernel sets
// are the two EL0 counter enables below; EL0 access to the timer
// (EL0TEN, EL0PTEN) stays trapped, as it must.
#[derive(Copy, Clone, Debug, Default)]
pub struct CntKctlEl1(u64);

// Defaults to 0, i.e. "the kernel never opted EL0 in".  Unconditional
// (unlike the counter/frequency mocks above): `write` records into it
// under `cfg!(test)`, a runtime check whose code path compiles in both
// builds.
static MOCK_KCTL: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

#[cfg(test)]
pub fn set_mock_kctl(value: u64) {
    MOCK_KCTL.store(value, core::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
pub fn mock_kctl() -> u64 {
    MOCK_KCTL.load(core::sync::atomic::Ordering::Relaxed)
}

impl CntKctlEl1 {
    // EL0PCTEN (bit 0): EL0 may read CNTPCT_EL0.
    // EL0PCNTEN (bit 1): EL0 may read CNTFRQ_EL0.
    pub const fn el0_counter_access() -> Self {
        Self(0b11)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub fn write(self) {
        if cfg!(test) {
            // Record the last write so a host test can assert what the
            // kernel opted EL0 into.
            MOCK_KCTL.store(self.0, core::sync::atomic::Ordering::Relaxed);
        } else {
            CNTKCTL_EL1.set(self.0);
            // The architectural effect of the write is visible to EL0
            // only after a context synchronisation event; an ISB here
            // keeps the opt-in self-contained, matching the other
            // writes in this file.
            barrier::isb(barrier::SY);
        }
    }
}

// CNTP_CVAL_EL0 — physical timer compare value (write-only for now)
pub struct CntpCvalEl0;

impl CntpCvalEl0 {
    pub fn write(value: u64) {
        if !cfg!(test) {
            CNTP_CVAL_EL0.set(value);
            // A direct system-register write is not guaranteed to have
            // reached the timer's interrupt output until a context
            // synchronisation event.  Callers re-arm the timer and then
            // immediately EOI at the GIC, so without this the EOI can
            // race the deassert and the level-triggered PPI re-fires.
            barrier::isb(barrier::SY);
        }
    }
}

// CNTP_CTL_EL0 — physical timer control register
bitstruct! {
    #[derive(Copy, Clone, Default)]
    pub struct CntpCtlEl0(pub u64) {
        pub enable: bool = 0..1;
        pub imask: bool = 1..2;
        pub istatus: bool = 2..3;
    }
}

impl CntpCtlEl0 {
    pub fn read() -> Self {
        Self(if cfg!(test) { 0 } else { CNTP_CTL_EL0.extract().into() })
    }

    pub fn write(self) {
        if !cfg!(test) {
            CNTP_CTL_EL0.set(self.0);
            // A direct system-register write is not guaranteed to have
            // reached the timer's interrupt output until a context
            // synchronisation event.  Callers disarm the timer and then
            // immediately EOI at the GIC, so without this the EOI can
            // race the deassert and the level-triggered PPI re-fires.
            // Linux does the same (arch/arm64/include/asm/arch_timer.h).
            barrier::isb(barrier::SY);
        }
    }
}

impl fmt::Debug for CntpCtlEl0 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CntPctl")
            .field("enable", &self.enable())
            .field("imask", &self.imask())
            .field("istatus", &self.istatus())
            .finish()
    }
}
