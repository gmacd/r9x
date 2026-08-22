//! Generic Interrupt Controller driver.
//!
//! Currently supports GIC-400/GICv2 — the controller on QEMU `virt` and
//! the BCM2711 (Pi 4).  The Pi 3's BCM2837 also has a GIC-400, but its
//! generic-timer PPIs route through the bcm2836 local interrupt
//! controller, not the GIC (its DT timer node is `arm,armv7-timer`
//! parented to the local intc), so a Pi 3 panics in `init` with a
//! message that says so: supporting it needs a local-intc driver, which
//! is a design decision, not a small task.
//!
//! Failure policy: a missing or unusable interrupt controller — or a
//! devicetree with no GIC-routed timer PPI — is a boot failure.  `init`
//! panics rather than return an error the caller can ignore: a kernel
//! that cannot take interrupts cannot run its timers (and, once the
//! scheduler lands, cannot schedule at all), and a degraded boot is
//! distinguishable from a working one only by the absence of output.
//! `timer::init` follows the same policy (it panics on a zero CNTFRQ).
//!
//! The timer PPI's INTID is not a constant: it is parsed from the
//! devicetree at init (`timer_intid_from_dt`), because the number is a
//! property of the machine's interrupt wiring, not of the kernel.

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
const GICD_IPRIORITYR: usize = 0x0400; // Priority registers (0x400-0x7fc)
const GICD_ISENABLER: usize = 0x0100; // Set-enable registers (0x100-0x17c)
const GICD_ICENABLER: usize = 0x0180; // Clear-enable registers (0x180-0x1fc)
const GICD_ICPENDR: usize = 0x0280; // Clear-pending registers (0x280-0x2fc)
const GICD_ICACTIVER: usize = 0x0380; // Clear-active registers (0x380-0x3fc)

// The GICv2 INTID space is 32 blocks of 32 INTIDs.  The enable, pending
// and active registers are one 32-bit word per block; the priority
// registers hold four INTIDs per word, so 256 words cover the same
// space.
const GICD_INTID_BLOCKS: usize = 32;
const GICD_PRIORITY_WORDS: usize = GICD_INTID_BLOCKS * 8;

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

/// Parse the INTID of the EL1 non-secure physical timer PPI from the
/// devicetree.
///
/// The generic-timer PPIs sit at the low end of the PPI range by Arm
/// Base System Architecture convention (SBSA; the GIC-400 TRM agrees):
/// INTID 26 = non-secure EL2 physical, 27 = virtual (CNTV), 28 = EL2
/// virtual (VHE), 29 = secure EL1 physical (CNTPS), 30 = non-secure
/// EL1 physical (CNTP); DT PPI numbers map to INTIDs as 16 + n.  The
/// convention — not the architecture — is what makes the number a
/// property of the machine's wiring, which is why it is parsed rather
/// than assumed.  The `arm,armv8-timer` node's `interrupts` list is a
/// series of (type, PPI, flags) triplets ordered [0] EL1 secure
/// physical, [1] EL1 non-secure physical, [2] EL1 virtual, [3] EL2
/// physical (`arm,arch_timer.yaml`).  r9 guarantees non-secure EL1
/// handoff on every supported target (QEMU `virt` boots a 64-bit guest
/// at EL1 non-secure; the BCM firmware hands the OS off at EL1
/// non-secure) and arms the CNTP, so it takes entry [1].
///
/// The positional read of entry [1] assumes the secure entry occupies
/// [0]; a list that names its entries (`interrupt-names`, used by some
/// hypervisor-generated DTs) is a different convention and is refused
/// rather than guessed at.
///
/// Verified on both supported machines: QEMU virt (live DTB) and
/// bcm2711 (Pi 4) list PPI 14 at entry [1] — INTID 30 — and on QEMU
/// virt arming CNTP in fact delivers INTID 30.
///
/// A board whose timer node is `arm,armv7-timer` parented to a local
/// interrupt controller (the Pi 3's bcm2837: its timer PPIs are
/// local-intc IRQs, not GIC PPIs) has no `arm,armv8-timer` node and
/// fails here.  That is the loud way to say the board is out of scope.
fn timer_intid_from_dt(dt: &DeviceTree) -> Result<u16> {
    let node = dt.find_compatible("arm,armv8-timer").next().ok_or(
        "no arm,armv8-timer node in devicetree: the timer PPI is not GIC-routed (on the Pi 3's bcm2837 it goes through the local interrupt controller, which is not supported)",
    )?;
    if dt.property(&node, "interrupt-names").is_some() {
        return Err(
            "arm,armv8-timer node has interrupt-names; r9 takes the positional entry [1] and does not parse named lists",
        );
    }
    let prop = dt
        .property(&node, "interrupts")
        .ok_or("arm,armv8-timer node has no interrupts property")?;
    let cells = dt.property_value_as_u32_iter(&prop);
    // Entry [1]'s PPI is the 5th cell: triplet [0] is cells 0-2,
    // triplet [1] is cells 3-5, the PPI is cell 4.  Cell 3 must be the
    // GIC_PPI type (1); anything else means the specifiers are not the
    // 3-cell GIC form this positional read assumes.
    let mut ppi = None;
    for (i, cell) in cells.enumerate() {
        match i {
            3 => {
                if cell != 1 {
                    return Err("arm,armv8-timer interrupts entry [1] is not a GIC PPI specifier");
                }
            }
            4 => ppi = Some(cell),
            _ => {}
        }
    }
    let ppi =
        ppi.ok_or("arm,armv8-timer interrupts list is too short for the non-secure EL1 PPI")?;
    if ppi > 15 {
        return Err("arm,armv8-timer interrupts entry [1] is not a local PPI number");
    }
    // ppi <= 15, so 16 + ppi fits a u16.
    Ok(16 + ppi as u16)
}

/// The INTID of the timer PPI on this machine, parsed from the
/// devicetree at init (`timer_intid_from_dt`).  IRQs are masked until
/// after init, so the first read is always after the parse.
pub fn timer_intid() -> u16 {
    GIC.get().expect("gic: not initialised").timer_intid
}

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
    /// The timer PPI's INTID on this machine — parsed from the DT, not
    /// assumed (see `timer_intid_from_dt`).
    timer_intid: u16,
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

        let timer_intid = timer_intid_from_dt(dt)?;

        Ok(Gic { gicc_virtrange, gicd_virtrange, timer_intid })
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
    /// INTID space, trusting nothing inherited; Linux follows the same
    /// policy in gic_dist_init (drivers/irqchip/irq-gic.c).  The
    /// INTID 0..31 registers are banked per core on GICv2, so the
    /// sweeps here reach only the boot core's bank; `init_cpu`
    /// re-establishes the banked half on every core.
    fn init_distributor(&self) {
        // Disable every INTID.
        for block in 0..GICD_INTID_BLOCKS {
            write_reg(&self.gicd_virtrange, GICD_ICENABLER + 4 * block, !0);
        }
        // Clear every pending bit.
        for block in 0..GICD_INTID_BLOCKS {
            write_reg(&self.gicd_virtrange, GICD_ICPENDR + 4 * block, !0);
        }
        // Program the default priority for every INTID.
        for word in 0..GICD_PRIORITY_WORDS {
            write_reg(
                &self.gicd_virtrange,
                GICD_IPRIORITYR + 4 * word,
                DEFAULT_PRIORITY_WORD,
            );
        }
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
        // The priority registers for INTIDs 0..31 are banked per core,
        // so this core programs its own bank — the distributor's sweep
        // reached only the boot core's.  A timer PPI left at 0xff by
        // firmware is numerically equal to the PMR about to be set and
        // is never forwarded, with nothing to say why.  Linux writes
        // the same default per core in gic_cpu_config()
        // (drivers/irqchip/irq-gic-common.c).
        for word in 0..(GICD_INTID_BLOCKS / 4) {
            write_reg(&self.gicd_virtrange, GICD_IPRIORITYR + 4 * word, DEFAULT_PRIORITY_WORD);
        }
        // Admit all priorities (lower value = higher priority; 0xff is
        // the lowest threshold, masking nothing).
        write_reg(&self.gicc_virtrange, GICC_PMR, GiccPmr(0).with_priority(0xff).into());
        // Enable this core's timer PPI.
        self.enable_interrupt(self.timer_intid);
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
