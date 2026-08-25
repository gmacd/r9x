//! Generic Interrupt Controller driver.
//!
//! Currently supports GIC-400/GICv2 — the controller on QEMU `virt` and
//! the BCM2711 (Pi 4).  The Pi 3's BCM2837 also has a GIC-400, but its
//! generic-timer PPIs route through the bcm2836 local interrupt
//! controller, not the GIC (its DT timer node is `arm,armv7-timer`
//! parented to the local intc), so a Pi 3 panics in `timer::init` with
//! a message that says so: supporting it needs a local-intc driver,
//! which is a design decision, not a small task.
//!
//! Failure policy: a missing or unusable interrupt controller — or a
//! devicetree with no GIC-routed timer PPI — is a boot failure.  `init`
//! panics rather than return an error the caller can ignore: a kernel
//! that cannot take interrupts cannot run its timers (and, once the
//! scheduler lands, cannot schedule at all), and a degraded boot is
//! distinguishable from a working one only by the absence of output.
//! `timer::init` follows the same policy (it panics on a zero CNTFRQ).

use core::fmt;

use crate::deviceutil::map_device_register;
use crate::io::{read_reg, write_reg};
use crate::vm;
use port::Result;
use port::irq::IrqGuard;
use port::mem::{PhysRange, VirtRange};
use port::once::Once;
use r9x_core::fdt::DeviceTree;

use bitstruct::bitstruct;

#[cfg(target_os = "none")]
use port::println;

const GICC_CTLR: usize = 0x0000;
const GICC_PMR: usize = 0x0004;
const GICC_IAR: usize = 0x000c;
const GICC_EOIR: usize = 0x0010;
const GICC_IIDR: usize = 0x00fc; // CPU Interface Identification Register

const GICD_CTLR: usize = 0x0000;
// The GICv2 map is not the GICv3 one: priority sits at 0x400, not
// 0x000, and 0x000-0x0fc is CTLR/TYPER/IGROUPR, not priority.
const GICD_TYPER: usize = 0x0004; // Type register (read-only GICD_CTLR)
const GICD_IPRIORITYR: usize = 0x0400; // Priority registers (0x400-0x7fc)
const GICD_ISENABLER: usize = 0x0100; // Set-enable registers (0x100-0x17c)
const GICD_ICENABLER: usize = 0x0180; // Clear-enable registers (0x180-0x1fc)
const GICD_ICPENDR: usize = 0x0280; // Clear-pending registers (0x280-0x2fc)
// GICD_CLRSPINACT on GICv2 hardware: the 0x380 block is clear-active
// for SPI 32..63, whose private bits are reserved there; QEMU models
// it as banked clear-active across INTIDs 0..31 as well.
const GICD_ICACTIVER: usize = 0x0380; // Clear-active registers (0x380-0x3fc)

// The banked private range — SGIs 0..15, PPIs 16..31 — is
// architecturally fixed at INTIDs 0..31, whatever ITLinesNumber says:
// 8 priority words (four INTIDs per word) and 4 enable/pending/active
// words (eight INTIDs per word).
const GICD_BANKED_INTIDS: usize = 32;

// The priority the kernel gives every INTID it programs: 0xa0, Linux's
// GICD_INT_DEF_PRI (include/linux/irqchip/arm-gic-common.h).  An
// interrupt forwards only when its priority is numerically lower than
// the core's PMR, so 0xa0 is admitted by init_cpu's PMR of 0xff while
// leaving the higher half of the space for interrupts that must
// pre-empt the default.
const DEFAULT_PRIORITY: u32 = 0xa0;
const DEFAULT_PRIORITY_WORD: u32 = DEFAULT_PRIORITY * 0x0101_0101;

// GICC_CTLR[8:5] are the FIQ/IRQ bypass-disable bits.  Their value is
// established by firmware and is live on parts that wire the legacy
// interrupt path alongside the GIC (BCM2711), so enabling the CPU
// interface must preserve them rather than write the register whole.
// Linux does the same: GICC_DIS_BYPASS_MASK, include/linux/irqchip/arm-gic.h.
const GICC_CTLR_BYPASS_MASK: u32 = 0x1e0;

// INTIDs 1020..=1023 are special (1023 = spurious: no pending interrupt).
const INTID_SPECIAL_START: u16 = 1020;

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

/// Bring up the GIC on the boot core.  The contract with the caller:
/// unmask IRQs only after this returns — with the CPU interface
/// disabled and every priority masked until it completes, an interrupt
/// asserted by a prior boot stage cannot arrive before a driver is
/// there to acknowledge it.
///
/// Panics on failure — see the module's failure policy.  The guard
/// keeps IRQs masked for the whole bringup; it is the guard's job, not
/// the boot order's, to keep that true.
///
/// Secondary cores must each run `Gic::init_cpu` before they can take
/// interrupts; the entry point for that arrives with secondary bringup.
pub fn init(dt: &DeviceTree) {
    let gic = Gic::new(dt).unwrap_or_else(|msg| {
        panic!("gic: {msg:?}: refusing to boot without a working interrupt controller")
    });

    // Publish before programming, because `Once::get` is what hands out
    // the `&Gic` the bringup methods need.
    let _irq = IrqGuard::new();
    let Ok(gic) = GIC.set(gic) else {
        panic!("gic: initialised twice");
    };
    gic.init_distributor();
    gic.init_cpu();
    println!("gic: initialised");
}

struct Gic {
    gicc_virtrange: VirtRange,
    gicd_virtrange: VirtRange,
    /// The machine's INTID space in 32-INTID blocks (GICD_TYPER's
    /// ITLinesNumber plus one): the distributor sweeps are sized from
    /// it, never from the architectural maximum.
    intid_blocks: usize,
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

        // ITLinesNumber is the block count minus one, so this is the
        // 1..32 the architecture allows.
        let intid_blocks =
            GicdTyper(read_reg(&gicd_virtrange, GICD_TYPER)).it_lines_number() as usize + 1;

        Ok(Gic { gicc_virtrange, gicd_virtrange, intid_blocks })
    }

    /// System-wide bringup: establish every part of the distributor's
    /// state, then enable it.  Run once, on the boot core; the per-core
    /// half is `init_cpu`.
    ///
    /// The boot firmware has already run against this distributor, and
    /// its state is a claim, not truth: an interrupt left enabled
    /// arrives with no handler, one left pending is armed to fire the
    /// moment anything re-enables it, and one left at priority 0xff is
    /// never forwarded to a core running PMR 0xff — it cannot fire and
    /// nothing explains why.  The sweeps therefore cover the whole
    /// INTID space the machine implements (Linux sizes its sweeps the
    /// same way, from ITLinesNumber, drivers/irqchip/irq-gic.c),
    /// trusting nothing inherited.  The INTID 0..31 registers are
    /// banked per core on GICv2, so the sweeps here reach only the
    /// boot core's bank; `init_cpu` re-establishes the banked half on
    /// every core.
    fn init_distributor(&self) {
        // Disable every INTID.
        for block in 0..self.intid_blocks {
            write_reg(&self.gicd_virtrange, GICD_ICENABLER + 4 * block, !0);
        }
        // Clear every pending bit.
        for block in 0..self.intid_blocks {
            write_reg(&self.gicd_virtrange, GICD_ICPENDR + 4 * block, !0);
        }
        // Default priority for the whole INTID space.  An interrupt is
        // forwarded only while its priority is numerically lower than
        // the core's PMR, and a stale 0xff (or a PPI bank left at it)
        // equals the PMR and is never delivered, with nothing to say
        // why.
        for word in 0..self.intid_blocks * 8 {
            write_reg(&self.gicd_virtrange, GICD_IPRIORITYR + 4 * word, DEFAULT_PRIORITY_WORD);
        }
        write_reg(&self.gicd_virtrange, GICD_CTLR, GicdCtlr(0).with_enable(true).into());
    }

    /// Per-core bringup: run on every core that takes interrupts.
    ///
    /// GICC_PMR and GICC_CTLR are banked per CPU interface, and the
    /// INTID 0..=31 registers (SGIs and PPIs) are banked per core.  A
    /// secondary that skips this comes up with every priority masked,
    /// whatever firmware left its private interrupts enabled, and its
    /// CPU interface disabled — however thoroughly the boot core
    /// initialised.
    /// The banking is done by the hardware behind one set of addresses,
    /// so there is no per-core state to store: every core runs this
    /// through the same shared mappings.
    fn init_cpu(&self) {
        // Firmware and earlier boot stages leave state behind in the
        // banked private registers, and it is not ours to inherit:
        // an interrupt left enabled arrives with no handler, and one
        // left active is never delivered again until something ends
        // it.  Clear both across the whole banked range — the boot
        // core's sweeps reached only its own bank — before enabling
        // anything.  (Linux's per-core pass clears the same registers,
        // one word each, drivers/irqchip/irq-gic-common.c
        // gic_cpu_config; we don't share its assumption about the
        // other words.)
        for word in 0..GICD_BANKED_INTIDS / 8 {
            write_reg(&self.gicd_virtrange, GICD_ICACTIVER + 4 * word, !0);
            write_reg(&self.gicd_virtrange, GICD_ICENABLER + 4 * word, !0);
        }
        // The priority registers for INTIDs 0..=31 are banked per
        // core, so this core programs its own bank — the
        // distributor's sweep reached only the boot core's.  A timer
        // PPI left at 0xff by firmware is numerically equal to the PMR
        // about to be set and is never forwarded, with nothing to say
        // why.  Linux writes the same default per core in
        // gic_cpu_config() (drivers/irqchip/irq-gic-common.c).
        for word in 0..GICD_BANKED_INTIDS / 4 {
            write_reg(&self.gicd_virtrange, GICD_IPRIORITYR + 4 * word, DEFAULT_PRIORITY_WORD);
        }
        // Admit all priorities (lower value = higher priority; 0xff is
        // the lowest threshold, masking nothing).
        write_reg(&self.gicc_virtrange, GICC_PMR, GiccPmr(0).with_priority(0xff).into());
        // CPU interface last: nothing is forwarded to this core until
        // the priority mask is in place.  Read-modify-write rather than
        // a whole-register write — GICC_CTLR[8:5] are firmware's, not
        // ours (see GICC_CTLR_BYPASS_MASK).
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
            .and_then(|reg| PhysRange::from_regblock(&reg))
            .map(|physrange| map_device_register("gicc", physrange, vm::PageSize::Page4K))
            .unwrap_or(Err("can't get gicc regblock from devicetree"))?;

        let gicd_virtrange = dt
            .property_translated_reg_iter(gic_node)
            .next()
            .and_then(|reg| reg.regblock())
            .and_then(|reg| PhysRange::from_regblock(&reg))
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

/// Enable delivery of an interrupt at the distributor.
///
/// INTIDs 0..31 are banked per core on GICv2: the write lands in the
/// calling core's bank only, so a per-core interrupt (the timer PPI is
/// one) must be enabled on every core that takes it.
pub fn enable_interrupt(intid: u16) {
    if let Some(gic) = GIC.get() {
        gic.enable_interrupt(intid);
    }
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
