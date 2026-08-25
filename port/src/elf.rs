//! A minimal ELF64 reader over a byte slice.
//!
//! Turns an embedded user binary (a static, non-PIE, fixed-base ELF64) into
//! its entry point and its `PT_LOAD` segments, for the per-arch loader (see
//! the user-binary-loading plan). Pure: no I/O, no allocation, no arch
//! dependency — it reads the bytes in place and records their coordinates, so
//! the loader maps `&elf[offset..offset + filesz]` itself. Follows the
//! `r9x_core::fdt` precedent: a host-testable parser that turns an image byte blob
//! into validated structure.
//!
//! Only *structure* is validated here: the magic, ELF64-ness, little-endian,
//! `ET_EXEC`, the header and program-header bounds, and per-segment size
//! sanity. *Placement* (does a segment land in the user half, page-aligned,
//! without overlapping a neighbour?) is arch-specific and belongs to the
//! loader.
//!
//! Field offsets are the System V ELF64 ABI (AMD64 / AArch64): the ELF header's
//! `e_entry @ 24`, `e_phoff @ 32`, `e_phentsize @ 54`, `e_phnum @ 56`; the
//! program header's `p_type @ 0`, `p_flags @ 4`, `p_offset @ 8`, `p_vaddr @ 16`,
//! `p_filesz @ 32`, `p_memsz @ 40`.

/// A `PT_LOAD` segment of a user ELF. Coordinates into the source slice; the
/// loader owns the mapping.
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct Segment {
    /// Virtual address the segment is placed at. Arch-specific validity
    /// (user half, alignment, overlap) is the loader's to check.
    pub vaddr: u64,
    /// Offset into the ELF of the file-backed part.
    pub offset: u64,
    /// Number of file bytes to copy.
    pub filesz: u64,
    /// In-memory size; the `memsz - filesz` tail is zero-filled bss.
    pub memsz: u64,
    /// The segment is executable text (`p_flags & PF_X`).
    pub exec: bool,
}

/// A parsed user ELF: the entry point and its `PT_LOAD` segments.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Elf {
    /// `e_entry`, where the process starts.
    pub entry: u64,
    /// The `PT_LOAD` segments, front-packed up to `nsegments`.
    segments: [Segment; MAX_SEGMENTS],
    /// Number of valid `PT_LOAD` segments in `segments`.
    nsegments: usize,
}

impl Elf {
    /// The valid `PT_LOAD` segments, in program-header order.
    pub fn segments(&self) -> &[Segment] {
        &self.segments[..self.nsegments]
    }
}

/// The most `PT_LOAD` segments the reader accepts. A real static binary has a
/// handful; this is a stated bound, not a tuning knob. A slice with more is
/// refused, not silently truncated.
const MAX_SEGMENTS: usize = 16;

/// The reasons a user ELF is refused.
#[derive(Debug, PartialEq, Eq)]
pub enum ElfError {
    /// `e_ident` is not `\x7fELF`.
    NotElf,
    /// `EI_CLASS` is not 64-bit.
    Not64,
    /// `EI_DATA` is not little-endian (r9's targets are LE).
    WrongEndian,
    /// `e_type` is not `ET_EXEC` (a PIE / `ET_DYN` image is refused).
    NotExec,
    /// No `PT_LOAD` segments — nothing to load.
    NoLoadSegment,
    /// The ELF header or the program-header table runs past the slice.
    Truncated,
    /// A segment is ill-formed: `filesz > memsz`, or `offset + filesz` runs
    /// past the slice.
    BadSegment,
    /// More than [`MAX_SEGMENTS`] `PT_LOAD` segments.
    TooManySegments,
}

type Result<T> = core::result::Result<T, ElfError>;

/// `e_ident[EI_CLASS]` for a 64-bit ELF.
const EI_CLASS64: u8 = 2;
/// `e_ident[EI_DATA]` for little-endian.
const EI_DATA_LE: u8 = 1;
/// `e_type` the loader accepts: a static, fixed-base executable.
const ET_EXEC: u16 = 2;
/// `p_type` the loader maps.
const PT_LOAD: u32 = 1;
/// `p_flags` bit: executable.
const PF_X: u32 = 0x1;
/// ELF64 header size; the header must fit before anything is read.
const EHDR_SIZE: usize = 64;
/// ELF64 program-header size.
const PHDR_SIZE: usize = 56;

/// Read a little-endian `u16` at `off`, or `Truncated` if it runs past the end.
fn read_u16(elf: &[u8], off: usize) -> Result<u16> {
    let b = elf.get(off..off + 2).ok_or(ElfError::Truncated)?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

/// Read a little-endian `u32` at `off`, or `Truncated` if it runs past the end.
fn read_u32(elf: &[u8], off: usize) -> Result<u32> {
    let b = elf.get(off..off + 4).ok_or(ElfError::Truncated)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Read a little-endian `u64` at `off`, or `Truncated` if it runs past the end.
fn read_u64(elf: &[u8], off: usize) -> Result<u64> {
    let b = elf.get(off..off + 8).ok_or(ElfError::Truncated)?;
    Ok(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

/// Turn an embedded user ELF into its entry point and `PT_LOAD` segments.
///
/// Validates structure only; placement is the arch loader's job.
pub fn parse(elf: &[u8]) -> Result<Elf> {
    // The 16-byte `e_ident` and the whole 64-byte header must be present.
    let ident = elf.get(..16).ok_or(ElfError::Truncated)?;
    if &ident[0..4] != b"\x7fELF" {
        return Err(ElfError::NotElf);
    }
    if ident[4] != EI_CLASS64 {
        return Err(ElfError::Not64);
    }
    if ident[5] != EI_DATA_LE {
        return Err(ElfError::WrongEndian);
    }
    if elf.len() < EHDR_SIZE {
        return Err(ElfError::Truncated);
    }

    let e_type = read_u16(elf, 16)?;
    if e_type != ET_EXEC {
        return Err(ElfError::NotExec);
    }
    let entry = read_u64(elf, 24)?;
    let phoff = read_u64(elf, 32)?;
    let phentsize = read_u16(elf, 54)?;
    let phnum = read_u16(elf, 56)?;
    let len = elf.len() as u64;

    if phnum > 0 {
        // A 64-bit program header is exactly `PHDR_SIZE`; a differing
        // `e_phentsize` is not a 64-bit ELF as this reader understands it.
        if phentsize as usize != PHDR_SIZE {
            return Err(ElfError::Truncated);
        }
        // The whole program-header table must fit before we walk it.
        let table_end =
            phoff.checked_add(phnum as u64 * PHDR_SIZE as u64).ok_or(ElfError::Truncated)?;
        if table_end > len {
            return Err(ElfError::Truncated);
        }
    }

    let mut segments =
        [Segment { vaddr: 0, offset: 0, filesz: 0, memsz: 0, exec: false }; MAX_SEGMENTS];
    let mut nsegments = 0;
    for i in 0..(phnum as usize) {
        // `i < phnum` and the table fits, so `base` is in-slice.
        let base = (phoff + i as u64 * PHDR_SIZE as u64) as usize;
        let p_type = read_u32(elf, base)?;
        if p_type != PT_LOAD {
            continue;
        }
        let p_flags = read_u32(elf, base + 4)?;
        let offset = read_u64(elf, base + 8)?;
        let vaddr = read_u64(elf, base + 16)?;
        let filesz = read_u64(elf, base + 32)?;
        let memsz = read_u64(elf, base + 40)?;

        // Size sanity (structure, not placement): the file part is within the
        // slice and never larger than the in-memory size.
        if filesz > memsz {
            return Err(ElfError::BadSegment);
        }
        let seg_end = offset.checked_add(filesz).ok_or(ElfError::BadSegment)?;
        if seg_end > len {
            return Err(ElfError::BadSegment);
        }
        if nsegments == MAX_SEGMENTS {
            return Err(ElfError::TooManySegments);
        }

        segments[nsegments] = Segment { vaddr, offset, filesz, memsz, exec: p_flags & PF_X != 0 };
        nsegments += 1;
    }

    if nsegments == 0 {
        return Err(ElfError::NoLoadSegment);
    }

    Ok(Elf { entry, segments, nsegments })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid two-segment ELF64: an `R-X` text segment and an
    /// `RW-` data segment whose `memsz` exceeds `filesz` (a bss tail). The
    /// buffer is sized to hold each segment's file bytes so the in-slice check
    /// passes.
    fn valid_elf() -> Vec<u8> {
        // Layout: header (64) | 2 phdrs (112) | seg0 file (0x100) | seg1 file (0x10).
        let mut buf = vec![0u8; 512];

        // e_ident
        buf[0..4].copy_from_slice(b"\x7fELF");
        buf[4] = EI_CLASS64;
        buf[5] = EI_DATA_LE;
        buf[6] = 1; // EI_VERSION
        // Header fields
        put16(&mut buf, 16, ET_EXEC); // e_type
        put16(&mut buf, 18, 183); // e_machine (AArch64); value is not checked
        put32(&mut buf, 20, 1); // e_version
        put64(&mut buf, 24, 0x1000); // e_entry
        put64(&mut buf, 32, 64); // e_phoff
        put16(&mut buf, 52, EHDR_SIZE as u16); // e_ehsize
        put16(&mut buf, 54, PHDR_SIZE as u16); // e_phentsize
        put16(&mut buf, 56, 2); // e_phnum

        // phdr 0: R-X text
        let p0 = 64;
        put32(&mut buf, p0, PT_LOAD);
        put32(&mut buf, p0 + 4, 5); // PF_R | PF_X
        put64(&mut buf, p0 + 8, 176); // p_offset
        put64(&mut buf, p0 + 16, 0x1000); // p_vaddr
        put64(&mut buf, p0 + 32, 0x100); // p_filesz
        put64(&mut buf, p0 + 40, 0x100); // p_memsz

        // phdr 1: RW- data, bss tail (memsz 0x100 > filesz 0x10)
        let p1 = 64 + PHDR_SIZE;
        put32(&mut buf, p1, PT_LOAD);
        put32(&mut buf, p1 + 4, 6); // PF_R | PF_W
        put64(&mut buf, p1 + 8, 272); // p_offset
        put64(&mut buf, p1 + 16, 0x2000); // p_vaddr
        put64(&mut buf, p1 + 32, 0x10); // p_filesz
        put64(&mut buf, p1 + 40, 0x100); // p_memsz

        buf
    }

    fn put16(buf: &mut [u8], off: usize, v: u16) {
        buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
    }
    fn put32(buf: &mut [u8], off: usize, v: u32) {
        buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn put64(buf: &mut [u8], off: usize, v: u64) {
        buf[off..off + 8].copy_from_slice(&v.to_le_bytes());
    }

    #[test]
    fn parses_a_valid_elf() {
        let elf = parse(&valid_elf()).unwrap();
        assert_eq!(elf.entry, 0x1000);

        let segs = elf.segments();
        assert_eq!(
            segs,
            &[
                Segment { vaddr: 0x1000, offset: 176, filesz: 0x100, memsz: 0x100, exec: true },
                Segment { vaddr: 0x2000, offset: 272, filesz: 0x10, memsz: 0x100, exec: false }
            ]
        );
    }

    #[test]
    fn rejects_bad_magic() {
        let mut elf = valid_elf();
        elf[0..4].copy_from_slice(b"NOTE");
        assert_eq!(parse(&elf), Err(ElfError::NotElf));
    }

    #[test]
    fn rejects_32_bit() {
        let mut elf = valid_elf();
        elf[4] = 1; // ELFCLASS32
        assert_eq!(parse(&elf), Err(ElfError::Not64));
    }

    #[test]
    fn rejects_big_endian() {
        let mut elf = valid_elf();
        elf[5] = 2; // ELFDATA2MSB
        assert_eq!(parse(&elf), Err(ElfError::WrongEndian));
    }

    #[test]
    fn rejects_dyn() {
        let mut elf = valid_elf();
        put16(&mut elf, 16, 3); // ET_DYN
        assert_eq!(parse(&elf), Err(ElfError::NotExec));
    }

    #[test]
    fn rejects_no_load_segment() {
        let mut elf = valid_elf();
        // Both phdrs are non-LOAD (PT_NOTE = 4).
        put32(&mut elf, 64, 4);
        put32(&mut elf, 64 + PHDR_SIZE, 4);
        assert_eq!(parse(&elf), Err(ElfError::NoLoadSegment));
    }

    #[test]
    fn rejects_zero_program_headers() {
        let mut elf = valid_elf();
        put16(&mut elf, 54, 0); // e_phentsize = 0
        put16(&mut elf, 56, 0); // e_phnum = 0
        assert_eq!(parse(&elf), Err(ElfError::NoLoadSegment));
    }

    #[test]
    fn rejects_truncated_phdr_table() {
        let mut elf = valid_elf();
        // Header fits (>= 64) but the 2-phdr table (needs 176) does not.
        elf.truncate(100);
        assert_eq!(parse(&elf), Err(ElfError::Truncated));
    }

    #[test]
    fn rejects_short_header() {
        let mut elf = valid_elf();
        // Valid magic but shorter than the 64-byte header.
        elf.truncate(40);
        assert_eq!(parse(&elf), Err(ElfError::Truncated));
    }

    #[test]
    fn rejects_filesz_gt_memsz() {
        let mut elf = valid_elf();
        // Segment 0: filesz (0x200) exceeds memsz (0x100).
        put64(&mut elf, 64 + 32, 0x200);
        assert_eq!(parse(&elf), Err(ElfError::BadSegment));
    }

    #[test]
    fn rejects_segment_past_slice() {
        let mut elf = valid_elf();
        // Segment 0: offset + filesz runs past the end of the buffer.
        put64(&mut elf, 64 + 8, 500); // p_offset
        assert_eq!(parse(&elf), Err(ElfError::BadSegment));
    }
}
