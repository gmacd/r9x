//! Memory constants and address types.
//!
//! The address types (`PhysAddr`, `PhysRange`, `VirtRange`) and page-size
//! constants are defined in `r9x-core` (the neutral layer) and re-exported
//! here so existing `use port::mem::PhysAddr` paths still work.

pub use r9x_core::addr::{
    PAGE_SIZE_1G, PAGE_SIZE_2M, PAGE_SIZE_4K, PhysAddr, PhysRange, VirtRange, round_down2_u64,
    round_up2_u64,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physaddr_step() {
        let range = PhysRange::new(PhysAddr::new(4096), PhysAddr::new(4096 * 3));
        let pas = range.step_by_rounded(PAGE_SIZE_4K).collect::<Vec<PhysAddr>>();
        assert_eq!(pas, [PhysAddr::new(4096), PhysAddr::new(4096 * 2)]);
    }

    #[test]
    fn physaddr_step_rounds_up_and_down() {
        let range = PhysRange::new(PhysAddr::new(9000), PhysAddr::new(5000 * 3));
        let pas = range.step_by_rounded(PAGE_SIZE_4K).collect::<Vec<PhysAddr>>();
        assert_eq!(pas, [PhysAddr::new(4096 * 2), PhysAddr::new(4096 * 3)]);
    }

    #[test]
    fn physaddr_step_2m() {
        let range =
            PhysRange::new(PhysAddr::new(0x3f000000), PhysAddr::new(0x3f000000 + 4 * 1024 * 1024));
        let pas = range.step_by_rounded(PAGE_SIZE_2M).collect::<Vec<PhysAddr>>();
        assert_eq!(pas, [PhysAddr::new(0x3f000000), PhysAddr::new(0x3f000000 + 2 * 1024 * 1024)]);
    }
}
