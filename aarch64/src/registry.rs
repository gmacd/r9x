//! The embedded image registry: the table of image indices → embedded ELFs a
//! process can `SYS_SPAWN` by index.
//!
//! Generalises today's "one embedded ELF per test image": an image embeds a
//! *set* of ELFs (the same `include_bytes!` mechanism the server-embedding
//! images already use) and, at boot, registers them here so a running process
//! (init, first) can launch one by index.  The registry is populated at boot,
//! before any `sys_spawn` can reference an index — a stated, load-bearing
//! ordering: a spawn by an unregistered index is the error
//! [`r9x_abi::SPAWN_BAD_INDEX`], not a fault.  A spawn by index is bounded by
//! the table (an out-of-range index is an error, not a fault).
//!
//! The entries are `&'static` (the image's embedded ELFs live for the boot's
//! life), so the registry holds references, not copies.  Real (non-embedded)
//! images — `exec` by name through a file system — are a later arc (there is
//! no file system yet); the registry is embedded, per the user-binary-loading
//! plan.  Host builds see a stub so the `process`/`trap` modules compile; it
//! is never called (the spawn path is target-only).

#[cfg(target_os = "none")]
use core::sync::atomic::{AtomicUsize, Ordering};
#[cfg(target_os = "none")]
use port::mcslock::{Lock, LockNode};

/// One embedded image the registry can spawn by index: the ELF's bytes (the
/// loader reads them through `Image::Elf`) and a name (for the boot trace).
/// `Copy` data, so an image's set is a plain `static` of these.
#[derive(Clone, Copy)]
pub struct EmbeddedElf {
    /// The ELF's bytes: a static, non-PIE, fixed-base image linked at
    /// [`r9x_abi::IMAGE_BASE`] (the loader's placement check reads the same
    /// constant, so the two cannot drift).
    pub bytes: &'static [u8],
    /// The image's name: the boot trace and the debug prints read it.
    pub name: &'static str,
}

/// The registry's bound: the number of embedded images an image can register.
/// A stated constant (like `process`'s `NPROCS`); an image that registers more
/// is truncated to the bound (the extras are simply unreachable by index).
#[cfg(target_os = "none")]
const NIMAGES: usize = 8;

#[cfg(target_os = "none")]
const EMPTY: [Option<&'static EmbeddedElf>; NIMAGES] =
    [None, None, None, None, None, None, None, None];

/// The table: index → embedded image.  Populated by [`register`] (boot), read
/// by `sys_spawn` (a live process).  The lock is taken only to publish or read
/// the table (never held across a switch).
#[cfg(target_os = "none")]
static REGISTRY: Lock<[Option<&'static EmbeddedElf>; NIMAGES]> = Lock::new("registry", EMPTY);

/// How many images are registered: the valid index range is `0..NREGISTERED`.
/// Written once at boot, before any spawn (the load-bearing ordering), so a
/// spawn's read is a plain load after the boot's release store.
#[cfg(target_os = "none")]
static NREGISTERED: AtomicUsize = AtomicUsize::new(0);

/// Populate the registry from an image's embedded set (boot, init-context).
/// The images are registered in order (index 0 is the first), up to the bound.
/// Must run before `run_all` — before any process that could `sys_spawn`.
#[cfg(target_os = "none")]
pub fn register(images: &[&'static EmbeddedElf]) {
    let node = LockNode::new();
    let mut reg = REGISTRY.lock(&node);
    for (i, img) in images.iter().enumerate().take(NIMAGES) {
        reg[i] = Some(*img);
    }
    drop(reg);
    // Publish after the table is written: a spawn's acquire load (in `lookup`)
    // sees a fully-populated table, never a half-written one.
    NREGISTERED.store(images.len().min(NIMAGES), Ordering::Release);
}

/// The image for `index`, if it is registered.  An out-of-range index — the
/// registry empty, or the index at or past the registered count — is `None`
/// (the error, not a fault): the spawner maps it to
/// [`r9x_abi::SPAWN_BAD_INDEX`].
#[cfg(target_os = "none")]
pub(crate) fn lookup(index: usize) -> Option<&'static EmbeddedElf> {
    let n = NREGISTERED.load(Ordering::Acquire);
    if index >= n || index >= NIMAGES {
        return None;
    }
    let node = LockNode::new();
    let reg = REGISTRY.lock(&node);
    reg.get(index).and_then(|o| *o)
}

// Host builds (the unit tests of the process/trap modules) see stubs so those
// modules compile; they are never called (the spawn path is target-only).
#[cfg(not(target_os = "none"))]
#[allow(dead_code)]
pub fn register(_images: &[&'static EmbeddedElf]) {}

#[cfg(not(target_os = "none"))]
#[allow(dead_code)]
pub(crate) fn lookup(_index: usize) -> Option<&'static EmbeddedElf> {
    None
}
