//! Address types: `PhysAddr`, `PhysRange`, `VirtRange` and the page-size
//! constants.  Pure data with no kernel dependencies — the neutral layer.

use crate::fdt::RegBlock;
use core::{
    cmp::{max, min},
    fmt,
    iter::{Step, StepBy},
    ops::{self, Range},
};

pub const PAGE_SIZE_4K: usize = 4 << 10;
pub const PAGE_SIZE_2M: usize = 2 << 20;
pub const PAGE_SIZE_1G: usize = 1 << 30;

/// Round up by a power of 2.
pub const fn round_up2_u64(n: u64, step: u64) -> u64 {
    assert!(step.is_power_of_two());
    (n + step - 1) & !(step - 1)
}

/// Round down by a power of 2.
pub const fn round_down2_u64(n: u64, step: u64) -> u64 {
    assert!(step.is_power_of_two());
    n & !(step - 1)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VirtRange {
    pub start: usize,
    pub end: usize,
}

impl VirtRange {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn with_len(start: usize, len: usize) -> Self {
        Self { start, end: start + len }
    }

    pub fn from_physrange(pr: &PhysRange, offset: usize) -> Self {
        Self { start: pr.start.0 as usize + offset, end: pr.end.0 as usize + offset }
    }

    pub fn offset_addr(&self, offset: usize) -> Option<usize> {
        let addr = self.start + offset;
        if self.contains(addr) { Some(addr) } else { None }
    }

    pub fn size(&self) -> usize {
        self.end - self.start
    }

    pub fn contains(&self, addr: usize) -> bool {
        addr >= self.start && addr < self.end
    }
}

/// Infallible conversion, for length-guaranteed extents.  A reg with no
/// length collapses to a zero-size range (see `PhysRange::from_regblock` for
/// the fallible device-register path).
impl From<&RegBlock> for VirtRange {
    fn from(r: &RegBlock) -> Self {
        let start = r.addr as usize;
        let end = start + r.len.unwrap_or(0) as usize;
        VirtRange { start, end }
    }
}

impl fmt::Display for VirtRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#018x}..{:#018x}", self.start, self.end)
    }
}

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord)]
#[repr(transparent)]
pub struct PhysAddr(pub u64);

impl PhysAddr {
    pub const fn new(value: u64) -> Self {
        PhysAddr(value)
    }

    pub const fn addr(&self) -> u64 {
        self.0
    }

    /// Round up by a power of 2.
    pub const fn round_up2(&self, step: u64) -> PhysAddr {
        PhysAddr(round_up2_u64(self.0, step))
    }

    /// Round down by a power of 2.
    pub const fn round_down2(&self, step: u64) -> PhysAddr {
        PhysAddr(round_down2_u64(self.0, step))
    }

    pub const fn is_multiple_of(&self, n: u64) -> bool {
        self.0.is_multiple_of(n)
    }
}

impl ops::Add<u64> for PhysAddr {
    type Output = PhysAddr;

    fn add(self, offset: u64) -> PhysAddr {
        PhysAddr(self.0 + offset)
    }
}

impl Step for PhysAddr {
    fn steps_between(&startpa: &Self, &endpa: &Self) -> (usize, Option<usize>) {
        if startpa.0 <= endpa.0
            && let Some(diff) = endpa.0.checked_sub(startpa.0)
            && let Ok(diff) = usize::try_from(diff)
        {
            return (diff, Some(diff));
        }
        (0, None)
    }

    fn forward_checked(startpa: Self, count: usize) -> Option<Self> {
        startpa.0.checked_add(count as u64).map(PhysAddr)
    }

    fn backward_checked(startpa: Self, count: usize) -> Option<Self> {
        startpa.0.checked_sub(count as u64).map(PhysAddr)
    }

    fn forward_overflowing(startpa: Self, count: usize) -> (Self, bool) {
        let (pa, carried) = startpa.0.overflowing_add(count as u64);
        (PhysAddr(pa), carried)
    }

    fn backward_overflowing(startpa: Self, count: usize) -> (Self, bool) {
        let (pa, carried) = startpa.0.overflowing_sub(count as u64);
        (PhysAddr(pa), carried)
    }
}

impl fmt::Debug for PhysAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PhysAddr({:#016x})", self.0)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysRange {
    pub start: PhysAddr,
    pub end: PhysAddr,
}

impl PhysRange {
    pub fn new(start: PhysAddr, end: PhysAddr) -> Self {
        Self { start, end }
    }

    pub fn with_end(start: u64, end: u64) -> Self {
        Self { start: PhysAddr(start), end: PhysAddr(end) }
    }

    pub fn with_len(start: u64, len: usize) -> Self {
        Self { start: PhysAddr(start), end: PhysAddr(start + len as u64) }
    }

    pub fn with_pa_len(start: PhysAddr, len: usize) -> Self {
        Self { start, end: PhysAddr(start.0 + len as u64) }
    }

    /// Build a device-register range from a device-tree reg block, or `None`
    /// if the block carries no length (`size_cells == 0`).
    pub fn from_regblock(r: &RegBlock) -> Option<PhysRange> {
        let len = r.len?;
        Some(PhysRange::with_len(r.addr, len as usize))
    }

    #[allow(dead_code)]
    pub fn offset_addr(&self, offset: u64) -> Option<PhysAddr> {
        let addr = self.start + offset;
        if self.contains(addr) { Some(addr) } else { None }
    }

    pub fn size(&self) -> usize {
        (self.end.addr() - self.start.addr()) as usize
    }

    pub fn step_by_rounded(&self, step_size: usize) -> StepBy<Range<PhysAddr>> {
        let startpa = self.start.round_down2(step_size as u64);
        let endpa = self.end.round_up2(step_size as u64);
        (startpa..endpa).step_by(step_size)
    }

    pub fn add(&self, other: &PhysRange) -> Self {
        Self { start: min(self.start, other.start), end: max(self.end, other.end) }
    }

    /// Round extents so that start and end lie on multiples of step_size.
    pub fn round(&self, step_size: usize) -> Self {
        Self {
            start: self.start.round_down2(step_size as u64),
            end: self.end.round_up2(step_size as u64),
        }
    }

    pub fn contains(&self, addr: PhysAddr) -> bool {
        addr >= self.start && addr < self.end
    }
}

impl fmt::Display for PhysRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#018x}..{:#018x}", self.start.addr(), self.end.addr())
    }
}

/// Infallible conversion, for length-guaranteed extents (e.g. memory nodes).
/// Device registers must use `PhysRange::from_regblock`, which refuses a reg
/// with no length rather than silently collapsing it to a zero-size range.
impl From<&RegBlock> for PhysRange {
    fn from(r: &RegBlock) -> Self {
        let start = PhysAddr(r.addr);
        let end = start + r.len.unwrap_or(0);
        PhysRange::new(start, end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn physaddr_step() {
        let range = PhysRange::new(PhysAddr::new(4096), PhysAddr::new(4096 * 3));
        let pas: Vec<PhysAddr> = range.step_by_rounded(PAGE_SIZE_4K).collect();
        assert_eq!(pas, vec![PhysAddr::new(4096), PhysAddr::new(4096 * 2)]);
    }

    #[test]
    fn physaddr_step_rounds_up_and_down() {
        let range = PhysRange::new(PhysAddr::new(9000), PhysAddr::new(5000 * 3));
        let pas: Vec<PhysAddr> = range.step_by_rounded(PAGE_SIZE_4K).collect();
        assert_eq!(pas, vec![PhysAddr::new(4096 * 2), PhysAddr::new(4096 * 3)]);
    }

    #[test]
    fn physaddr_step_2m() {
        let range =
            PhysRange::new(PhysAddr::new(0x3f000000), PhysAddr::new(0x3f000000 + 4 * 1024 * 1024));
        let pas: Vec<PhysAddr> = range.step_by_rounded(PAGE_SIZE_2M).collect();
        assert_eq!(
            pas,
            vec![PhysAddr::new(0x3f000000), PhysAddr::new(0x3f000000 + 2 * 1024 * 1024)]
        );
    }

    #[test]
    fn round_up2() {
        assert_eq!(round_up2_u64(0, 4096), 0);
        assert_eq!(round_up2_u64(6, 4096), 4096);
        assert_eq!(round_up2_u64(4096, 4096), 4096);
        assert_eq!(round_up2_u64(4097, 4096), 8192);
    }

    #[test]
    fn round_down2() {
        assert_eq!(round_down2_u64(0, 4096), 0);
        assert_eq!(round_down2_u64(6, 4096), 0);
        assert_eq!(round_down2_u64(4096, 4096), 4096);
        assert_eq!(round_down2_u64(4097, 4096), 4096);
    }
}
