//! Generic Interrupt Controller driver.
//!
//! Currently supports GIC-400/GICv2.
//! Initialises the distributor and CPU interface, provides IRQ enable/disable
//! and priority management.

use core::fmt;

use crate::deviceutil::map_device_register;
use crate::io::{read_reg, write_reg};
use crate::vm;
use port::Result;
use port::fdt::DeviceTree;
use port::irq::IrqGuard;
use port::mem::{PhysRange, VirtRange};
use port::once::Once;

use bitstruct::bitstruct;

#[cfg(not(test))]
use port::println;

const GICC_CTLR: usize = 0x0000;
const GICC_PMR: usize = 0x0004;
const GICC_IAR: usize = 0x000c;
const GICC_EOIR: usize = 0x0010;
const GICC_IIDR: usize = 0x00fc; // CPU Interface Identification Register

const GICD_CTLR: usize = 0x0000;
const GICD_ISENABLER: usize = 0x0100; // Set-enable registers (0x100-0x17c)
const GICD_ICENABLER: usize = 0x0180; // Clear-enable registers (0x180-0x1fc)
const GICD_ICACTIVER: usize = 0x0380; // Clear-active registers (0x380-0x3fc)

// GICC_CTLR[8:5] are the FIQ/IRQ bypass-disable bits.  Their value is
// established by firmware and is live on parts that wire the legacy
// interrupt path alongside the GIC (BCM2711), so enabling the CPU
// interface must preserve them rather than write the register whole.
// Linux does the same: GICC_DIS_BYPASS_MASK, include/linux/irqchip/arm-gic.h.
const GICC_CTLR_BYPASS_MASK: u32 = 0x1e0;

// INTIDs 1020..=1023 are special (1023 = spurious: no pending interrupt).
const INTID_SPECIAL_START: u16 = 1020;

// EL1 physical timer PPI. DT PPI numbers map to INTIDs as 16 + n:
// secure phys timer is DT PPI 13 → INTID 29, non-secure phys is
// DT PPI 14 → INTID 30. Which one CNTP_* raises depends on the
// security state we boot in.
pub const TIMER_INTID: u16 = 30;

bitstruct! {
    #[derive(Copy, Clone)]
    pub struct GiccIidr(pub u32) {
        pub implementer: u32 = 0..12;
        pub revision: u16 = 12..16;
        pub arch_version: u8 = 16..20;
        pub product_id: u16 = 20..32;
    }
}

impl fmt::Debug for GiccIidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GiccIidr")
            .field("implementer", &format_args!("{:#x}", self.implementer()))
            .field("revision", &format_args!("{}", self.revision()))
            .field("arch_version", &format_args!("{}", self.arch_version()))
            .field("product_id", &format_args!("{:#x}", self.product_id()))
            .finish()
    }
}

bitstruct! {
    #[derive(Copy, Clone)]
    pub struct GiccIar(pub u32) {
        pub int_id: u16 = 0..10;
        pub cpu_id: u8 = 10..13;
    }
}

impl fmt::Debug for GiccIar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GiccIar")
            .field("int_id", &format_args!("{}", self.int_id()))
            .field("cpu_id", &format_args!("{}", self.cpu_id()))
            .finish()
    }
}

bitstruct! {
    #[derive(Copy, Clone)]
    pub struct GicdTyper(pub u32) {
        pub it_lines_number: u8 = 0..5;
        pub cpu_number: u8 = 5..8;
        pub security_extn: u8 = 10..11;
        pub lspi: u16 = 11..16;
    }
}

impl fmt::Debug for GicdTyper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GicdTyper")
            .field("it_lines_number", &format_args!("{:#}", self.it_lines_number()))
            .field("cpu_number", &format_args!("{}", self.cpu_number()))
            .field("security_extn", &format_args!("{}", self.security_extn()))
            .field("lspi", &format_args!("{}", self.lspi()))
            .finish()
    }
}

bitstruct! {
    #[derive(Copy, Clone)]
    pub struct GicdCtlr(pub u32) {
        pub enable: bool = 0..1;
    }
}

impl From<GicdCtlr> for u32 {
    fn from(r: GicdCtlr) -> u32 {
        r.0
    }
}

impl fmt::Debug for GicdCtlr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GicdCtlr").field("enable", &format_args!("{:#}", self.enable())).finish()
    }
}

bitstruct! {
    #[derive(Copy, Clone)]
    pub struct GiccCtlr(pub u32) {
        pub enable: bool = 0..1;
    }
}

impl From<GiccCtlr> for u32 {
    fn from(r: GiccCtlr) -> u32 {
        r.0
    }
}

bitstruct! {
    #[derive(Copy, Clone)]
    pub struct GiccPmr(pub u32) {
        pub priority: u8 = 0..8;
    }
}

impl From<GiccPmr> for u32 {
    fn from(r: GiccPmr) -> u32 {
        r.0
    }
}

/// Published GIC driver.  Set once during init, then read without
/// mutual exclusion from the interrupt path.  No lock is needed because
/// every register the driver touches afterwards is race-free by
/// construction: GICC_IAR and GICC_EOIR are banked per CPU interface,
/// and GICD_ISENABLER/ICENABLER are write-1-to-set/clear, so each
/// access is a single write with no read-modify-write to interleave.
static GIC: Once<Gic> = Once::new();

/// Bring up the GIC on the boot core.  On success the caller may unmask
/// IRQs; on failure it must not, or an interrupt asserted by a prior
/// boot stage arrives with no driver to acknowledge it and, being
/// level-triggered, re-fires forever.
///
/// Secondary cores must each run `Gic::init_cpu` before they can take
/// interrupts; the entry point for that arrives with secondary bringup.
pub fn init(dt: &DeviceTree) -> Result<()> {
    let gic = Gic::new(dt).inspect_err(|msg| println!("can't initialise gic: {msg:?}"))?;

    // Publish before programming, because `Once::get` is what hands out
    // the `&Gic` the bringup methods need.  Ordering against interrupt
    // delivery is not the reason: IRQs are masked here, and the guard
    // keeps that true if the boot order ever changes.
    let _irq = IrqGuard::new();
    let Ok(gic) = GIC.set(gic) else {
        println!("gic: already initialised");
        return Err("gic already initialised");
    };
    gic.init_distributor();
    gic.init_cpu();
    println!("gic: initialised");
    Ok(())
}

struct Gic {
    gicc_virtrange: VirtRange,
    gicd_virtrange: VirtRange,
}

impl Gic {
    /// Map the GIC and check the architecture version.  Programs no
    /// registers, so that all of it can happen after the driver is
    /// published: bringup is split into `init_distributor`, run once
    /// for the machine, and `init_cpu`, run on every core.
    fn new(dt: &DeviceTree) -> Result<Self> {
        let (gicc_virtrange, gicd_virtrange) = find_gicc_gicd_virtranges(dt, "arm,gic-400")
            .or_else(|_| find_gicc_gicd_virtranges(dt, "arm,cortex-a15-gic"))?;

        let gicc_iidr = GiccIidr(read_reg(&gicc_virtrange, GICC_IIDR));
        if gicc_iidr.arch_version() == 1 {
            return Err("gic v1 unsupported");
        }

        Ok(Gic { gicc_virtrange, gicd_virtrange })
    }

    /// System-wide bringup: enable the distributor.  Run once, on the
    /// boot core.  Everything else the GIC needs is per-core — see
    /// `init_cpu`.
    fn init_distributor(&self) {
        write_reg(&self.gicd_virtrange, GICD_CTLR, GicdCtlr(0).with_enable(true).into());
    }

    /// Per-core bringup: run on every core that takes interrupts.
    ///
    /// GICC_PMR and GICC_CTLR are banked per CPU interface, and
    /// GICD_ISENABLER0 — INTIDs 0..32, where the timer PPI lives — is
    /// banked per core.  A secondary that skips this comes up with
    /// every priority masked, its CPU interface disabled and its timer
    /// PPI undelivered, however thoroughly the boot core initialised.
    /// The banking is done by the hardware behind one set of addresses,
    /// so there is no per-core state to store: every core runs this
    /// through the same shared mappings.
    fn init_cpu(&self) {
        // Firmware and earlier boot stages leave state behind in the
        // banked INTID 0..32 registers, and it is not ours to inherit:
        // an interrupt left Active is never delivered again, and one
        // left enabled arrives with no handler.  Clear both before
        // enabling anything.  (Linux does this per core in
        // gic_cpu_config(), drivers/irqchip/irq-gic-common.c.)
        write_reg(&self.gicd_virtrange, GICD_ICACTIVER, !0);
        write_reg(&self.gicd_virtrange, GICD_ICENABLER, !0);
        // Admit all priorities (lower value = higher priority; 0xff is
        // the lowest threshold, masking nothing).
        write_reg(&self.gicc_virtrange, GICC_PMR, GiccPmr(0).with_priority(0xff).into());
        // Enable this core's timer PPI.
        self.enable_interrupt(TIMER_INTID);
        // CPU interface last: nothing is forwarded to this core until
        // the priority mask and PPI enables are in place.  Read-modify-
        // write rather than a whole-register write — GICC_CTLR[8:5] are
        // firmware's, not ours (see GICC_CTLR_BYPASS_MASK).
        let bypass = read_reg(&self.gicc_virtrange, GICC_CTLR) & GICC_CTLR_BYPASS_MASK;
        write_reg(
            &self.gicc_virtrange,
            GICC_CTLR,
            bypass | u32::from(GiccCtlr(0).with_enable(true)),
        );
    }

    /// Acknowledge the highest-priority pending interrupt by reading
    /// GICC_IAR.  Returns the raw IAR value (INTID in bits 9:0), or `None`
    /// if no interrupt is pending (spurious).  The returned value must be
    /// passed to `eoi` after the interrupt source has been handled.
    fn try_ack_interrupt(&self) -> Option<GiccIar> {
        let iar = GiccIar(read_reg(&self.gicc_virtrange, GICC_IAR));
        if iar.int_id() >= INTID_SPECIAL_START { None } else { Some(iar) }
    }

    /// Signal end of interrupt by writing the raw IAR value back to GICC_EOIR.
    fn end_interrupt(&self, iar: GiccIar) {
        write_reg(&self.gicc_virtrange, GICC_EOIR, iar.0);
    }

    /// Enable delivery of an interrupt at the distributor.
    fn enable_interrupt(&self, intid: u16) {
        let n = intid as usize;
        write_reg(&self.gicd_virtrange, GICD_ISENABLER + 4 * (n / 32), 1 << (n % 32));
    }

    /// Stop delivery of an interrupt at the distributor.
    fn disable_interrupt(&self, intid: u16) {
        let n = intid as usize;
        write_reg(&self.gicd_virtrange, GICD_ICENABLER + 4 * (n / 32), 1 << (n % 32));
    }
}

fn find_gicc_gicd_virtranges(dt: &DeviceTree, id: &'static str) -> Result<(VirtRange, VirtRange)> {
    // The GICD reg is first (index 0), GICC is second (index 1)
    if let Some(gic_node) = dt.find_compatible(id).next() {
        let gicc_virtrange = dt
            .property_translated_reg_iter(gic_node)
            .nth(1)
            .and_then(|reg| reg.regblock())
            .map(|reg| PhysRange::from(&reg))
            .map(|physrange| map_device_register("gicc", physrange, vm::PageSize::Page4K))
            .unwrap_or(Err("can't get gicc regblock from devicetree"))?;

        let gicd_virtrange = dt
            .property_translated_reg_iter(gic_node)
            .next()
            .and_then(|reg| reg.regblock())
            .map(|reg| PhysRange::from(&reg))
            .map(|physrange| map_device_register("gicd", physrange, vm::PageSize::Page4K))
            .unwrap_or(Err("can't get gicd regblock from devicetree"))?;

        Ok((gicc_virtrange, gicd_virtrange))
    } else {
        Err("Couldn't parse gic node in devicetree")
    }
}

/// Called from the trap handler to acknowledge a pending GIC interrupt.
/// Returns the raw IAR value, or `None` if no interrupt was pending
/// (spurious) or the GIC is not initialised.  The caller must handle the
/// interrupt (deasserting its source — the timer PPI is level-triggered)
/// and then pass the value to `eoi`.  EOI before deassertion would
/// immediately re-raise the interrupt.
///
/// Lock-free: single `load(Acquire)` then register I/O.
pub fn try_ack_interrupt() -> Option<GiccIar> {
    GIC.get()?.try_ack_interrupt()
}

/// Called from the trap handler for an IRQ nothing claims.
pub fn disable_interrupt(intid: u16) {
    if let Some(gic) = GIC.get() {
        gic.disable_interrupt(intid);
    }
}

/// Called from the trap handler to signal end-of-interrupt for a value
/// previously returned by `try_ack_interrupt`.
pub fn end_interrupt(iar: GiccIar) {
    if let Some(gic) = GIC.get() {
        gic.end_interrupt(iar);
    }
}
