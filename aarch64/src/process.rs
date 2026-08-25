//! The process table (plans/preemption.md, task 2): the kernel's
//! scheduler state.
//!
//! A process's live state while it is off CPU is (a) the trap frame on
//! its kstack — x0–x30, ESR, ELR, FAR, type, its user sp, and SPSR —
//! and (b) the suspended kernel call chain below it.  Both live on the
//! process's own kstack, never on the shared interrupt stack: SP_EL1 is
//! the kstack pointer, and a process that is running has its frame
//! (and everything below it) on its own 64 KiB.
//!
//! TPIDR_EL1 is the current-process pointer (or null): it is the only
//! place the Rust side can find "the process I am in" without a
//! thread-local, and every writer runs with IRQs masked on this core.
//!
//! Aliasing discipline (AGENTS.md: never assume single-core):
//! `Process.context` is the only field written without the table lock —
//! by `swtch`, through the raw pointer copied out under the lock — and
//! a slot's `Process` is never freed or reused while a raw pointer to
//! it is live (slots reclaim lazily).  Every other field is touched
//! under the lock; `CURSOR` is atomic; `STARTER_CTX` is set and read
//! only in IRQ-masked kernel context this arc.  The table lock is never
//! held across a `swtch` (the MCS lock is not reentrant, and the
//! suspended context's eventual `resched`, same core, would
//! self-deadlock).  Kernel-side lock holders run only when TPIDR is
//! null, and `resched` checks TPIDR before taking the lock: the moment
//! any syscall touches the table from a running process, that invariant
//! dies and acquisition must move under a DAIF mask.
//!
//! Pages are not freed this arc: there is no free API in
//! pagealloc/vm, and an exited process's pages are leaked.  Recorded,
//! not papered over.

#[cfg(target_os = "none")]
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
#[cfg(target_os = "none")]
use core::time::Duration;

#[cfg(target_os = "none")]
use crate::swtch::{Context, SPSR_EL1H, swtch};
#[cfg(target_os = "none")]
use crate::timer::{Timer, TimerCallback};
#[cfg(target_os = "none")]
use crate::vm::Entry;
#[cfg(target_os = "none")]
use port::irq::IrqGuard;
#[cfg(target_os = "none")]
use port::mcslock::{Lock, LockNode};
#[cfg(target_os = "none")]
use port::{iprintln, mem};

// The syscall numbers are the user-facing trap ABI, defined once in
// `r9x_abi` (the single source both the kernel and the `r9x_std` target read)
// and re-exported here so the existing `process::SYS*`/`SYC*` paths keep
// working; a pinning test asserts they match.  The aarch64 register convention
// (the number in x8, arguments in x0-x4) is spelled where the trap frame is
// laid out.
pub use r9x_abi::{
    SYCCREATECHAN, SYCRECEIVE, SYCREPLY, SYCSEND, SYSEXIT, SYSIRQCLAIM, SYSMAPMMIO, SYSYIELD,
};

/// The exit status a faulted process is marked with: distinct from a clean
/// exit (which uses the svc number, 0–15 in the test images), so an image can
/// tell a fault-death from a clean exit.
#[cfg(any(target_os = "none", test))]
pub const FAULT_STATUS: u64 = 0xff;

/// The trap frame's layout in trap.S, slot by slot.  `pub(crate)` and
/// ungated because the host tests pin them to `TrapFrame`/`Context`:
/// the layout is triple-maintained (the offsets spelled in trap.S,
/// these constants, the structs), and the pins are what keep the two
/// Rust sides agreeing.
#[cfg(any(target_os = "none", test))]
pub(crate) const FRAME_SZ: usize = 304;
#[cfg(any(target_os = "none", test))]
pub(crate) const FRAME_ELR: usize = 256;
#[cfg(any(target_os = "none", test))]
pub(crate) const FRAME_SP: usize = 280;
#[cfg(any(target_os = "none", test))]
pub(crate) const FRAME_SPSR: usize = 288;
#[cfg(any(target_os = "none", test))]
pub(crate) const CONTEXT_SZ: usize = 112;

/// The table size.  A compile constant: the kstacks below are statics,
/// so there is no allocation story.
#[cfg(target_os = "none")]
const NPROCS: usize = 8;

/// Each process's kernel stack: frame (304) + suspended call chains,
/// 64 KiB, the same size as the interrupt stack (16 pages).
#[cfg(target_os = "none")]
const KSTACK_SZ: usize = 16 * 4096;

/// A magic word at the base of each kstack, asserted before every
/// switch: the kstacks are statics (no guard pages to unmap), so a
/// silent overflow walking into the neighbour is otherwise invisible.
#[cfg(target_os = "none")]
const KSTACK_CANARY: u64 = 0x7072_6f63_5f6b_7374; // "proc_kst"

/// The kstacks, one per slot: slot `i`'s kstack is `KSTACKS[i]`.
/// 16-aligned: the SP math in trap.S (304 and 112-byte frames) and the
/// 16-byte stack convention require it, and a `[u8; ..]` would have no
/// alignment at all.  The `UnsafeCell` is what puts it in `.bss` at
/// all: a zero-initialised static of a const-constructible type is
/// emitted as read-only constant data (verified against rustc's
/// output), and the kstacks must be writable — the frame and context
/// are written into them, and a store to constant data is a
/// permission fault, not a silent error.
#[cfg(target_os = "none")]
#[repr(align(16))]
struct Kstacks {
    stacks: core::cell::UnsafeCell<[[u8; KSTACK_SZ]; NPROCS]>,
}

// SAFETY: each kstack is written only by the context that owns it —
// the spawn that fabricates its frame (before the process is
// reachable) and the process itself through its own traps; the slot
// discipline in the table is what keeps two owners apart.
#[cfg(target_os = "none")]
unsafe impl Sync for Kstacks {}

#[cfg(target_os = "none")]
static KSTACKS: Kstacks =
    Kstacks { stacks: core::cell::UnsafeCell::new([[0u8; KSTACK_SZ]; NPROCS]) };

/// A process's scheduling priority: an index into QNX's 256-level range,
/// 0 most urgent, 255 the idle thread's slot.  A runnable process never
/// takes 255 (it is the never-scheduled sentinel), so the live range is
/// 0–254.  The order is QNX's inverted one — **lower is more urgent**, so a
/// priority 8 runs ahead of a priority 200 one; the derived `Ord` follows
/// the number (lower `<` higher), and `pick_next` takes the *minimum*.
/// Ties at a level round-robin (see `pick_next`).
#[cfg(any(target_os = "none", test))]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(transparent)]
pub struct Priority(u8);

#[cfg(any(target_os = "none", test))]
impl Priority {
    /// Most urgent (QNX's level 0).
    pub const MIN: Priority = Priority(0);
    /// The idle thread's level and the never-scheduled sentinel: a
    /// runnable process never takes it, so it is the top of the live range
    /// (0–254).
    pub const IDLE: Priority = Priority(255);

    /// A priority by its level number (0 most urgent).
    pub const fn new(level: u8) -> Priority {
        Priority(level)
    }

    /// The level number (0 most urgent, 255 the idle sentinel).
    pub const fn level(self) -> u8 {
        self.0
    }
}

/// The priority a new user process starts at: mid-range, a normal user
/// process — neither of the sentinel extremes.
#[cfg(any(target_os = "none", test))]
pub const DEFAULT_PRIORITY: Priority = Priority::new(128);

/// A slot's priority state: its own (`base`) priority and the priority it
/// currently runs at (`effective`).  `effective` is more urgent than `base`
/// (a lower level number) only while the slot is boosted (priority
/// inheritance).
#[cfg(any(target_os = "none", test))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PriorityState {
    base: Priority,
    effective: Priority,
}

#[cfg(any(target_os = "none", test))]
impl PriorityState {
    /// A process at its own `base` priority, not boosted.
    pub const fn new(base: Priority) -> Self {
        Self { base, effective: base }
    }

    /// The priority the slot is currently scheduled at.
    pub fn effective(&self) -> Priority {
        self.effective
    }

    /// True while the slot's effective priority is more urgent than its
    /// base.  `<`, not `!=`, so a call that (incorrectly) sets `effective`
    /// *less* urgent than `base` is never reported as boosted: boosted means
    /// more urgent, full stop.
    pub fn is_boosted(&self) -> bool {
        self.effective < self.base
    }

    /// Raise the effective priority to `to` (more urgent than `base`),
    /// remembering `base`.  `to` is at least as urgent as `base` (a
    /// priority-inheritance raise; a lower-urgency `to` is a no-op in effect
    /// — see `is_boosted`).  PI is at most once per slot (no stacking): a
    /// `boost` of an already-boosted slot is a no-op, so a holder keeps its
    /// first boost until `unboost`.
    pub fn boost(&mut self, to: Priority) {
        if !self.is_boosted() {
            self.effective = to;
        }
    }

    /// Restore the effective priority to `base`.  A no-op when not boosted
    /// (already at base).
    pub fn unboost(&mut self) {
        self.effective = self.base;
    }
}

#[cfg(any(target_os = "none", test))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    /// On CPU.  At most one process at a time, this arc.
    Running,
    /// May be switched to.
    Runnable,
    /// Off the ready set, waiting (on a message, this arc).  Put back on the
    /// ready set by a matching `wake`; the switch-back is an ordinary
    /// selection of the now-Runnable process.  Target-only: the host test
    /// build never blocks a process.
    #[cfg(target_os = "none")]
    Blocked,
    /// Done.  `exit_status` is valid; the slot reclaims at the next
    /// `spawn`.
    Exited,
}

/// A process in the table.
#[cfg(target_os = "none")]
struct Process {
    state: State,
    /// The resume point: the address of the `Context` on the process's
    /// kstack that `swtch` enters.  The only field written without the
    /// table lock (see the module docs).
    context: *mut Context,
    /// The process's kstack, into `KSTACKS` (its slot).  Read-only
    /// after `spawn`.
    kstack: *const u8,
    /// Valid once `state == Exited`.
    exit_status: u64,
    /// The slot's priority and its effective (possibly boosted) priority.
    /// Touched only under the table lock (module docs).
    prio: PriorityState,
    /// The process's address space (its TTBR0 root).  Built at `spawn`, lives
    /// for the process's life (the page is not freed this arc).  Read-only
    /// after `spawn`; the switch path reads it to install the TTBR0.
    aspace: crate::aspace::Aspace,
}

/// One optional process per slot, all empty.  Spelled out rather than
/// `[None; NPROCS]`: array repeat needs `Process: Copy`.
#[cfg(target_os = "none")]
const EMPTY_TABLE: [Option<Process>; NPROCS] = [None, None, None, None, None, None, None, None];

/// The table: one optional process per slot.
#[cfg(target_os = "none")]
static TABLE: Lock<[Option<Process>; NPROCS]> = Lock::new("proc", EMPTY_TABLE);

/// The slot to start the next `resched` scan at.  Round-robin fairness
/// is a later arc; this keeps the scan from re-picking slot 0 forever.
#[cfg(target_os = "none")]
static CURSOR: AtomicUsize = AtomicUsize::new(0);

/// Pick the next slot to run: the highest-priority Runnable other than
/// `current`, and among the ties at that priority the first one after
/// `cursor`, wrapping (round-robin nested under priority).  Pure over a
/// slice of per-slot `(state, effective priority)` so it is host-testable
/// without the `target_os = "none"` process representation.  An empty table
/// slot is passed as `State::Exited`, which is never picked.
#[cfg(any(target_os = "none", test))]
fn pick_next(slots: &[(State, Priority)], current: usize, cursor: usize) -> Option<usize> {
    let n = slots.len();
    // The highest effective priority among the Runnable slots other than
    // the current one; nothing else Runnable is `None`.
    let best = (0..n)
        .filter(|&i| i != current)
        .filter(|&i| slots[i].0 == State::Runnable)
        .map(|i| slots[i].1)
        .min()?;
    // Among the ties at that priority, round-robin from just after the
    // cursor: the existing fairness, nested under priority rather than
    // replaced.
    for off in 1..=n {
        let i = (cursor + off) % n;
        if i != current && slots[i].0 == State::Runnable && slots[i].1 == best {
            return Some(i);
        }
    }
    None
}

/// The address of the saved kernel starter context that `run_all`'s
/// first `swtch` fills in: the successor of the single-process arc's
/// kernel slot.  Set and read only in IRQ-masked kernel context this
/// arc (task #4 makes it per-core).
#[cfg(target_os = "none")]
static mut STARTER_CTX: *mut Context = core::ptr::null_mut();

#[cfg(target_os = "none")]
fn starter_ctx_addr() -> *mut *mut Context {
    core::ptr::addr_of_mut!(STARTER_CTX)
}

/// The switch-in order trace, recorded so a qemu-test image can assert
/// scheduling order (this task's priority/boost assertions read it).  A
/// bound of 64 covers the test images' short runs.  The write happens in
/// the selection path (a per-switch hot path), but only in qemu-test
/// builds; in production builds `record_run` compiles out to nothing, so
/// the trace is not a production cost.
#[cfg(all(target_os = "none", feature = "qemu-test"))]
const RUN_ORDER_MAX: usize = 64;
#[cfg(all(target_os = "none", feature = "qemu-test"))]
struct RunOrder {
    ids: core::cell::UnsafeCell<[usize; RUN_ORDER_MAX]>,
    len: core::cell::UnsafeCell<usize>,
}
// SAFETY: every write is under the table lock (the selection path) and the
// only reader runs after `run_all` returns (the table is then empty, so no
// further writes); single-core this arc.
#[cfg(all(target_os = "none", feature = "qemu-test"))]
unsafe impl Sync for RunOrder {}
#[cfg(all(target_os = "none", feature = "qemu-test"))]
static RUN_ORDER: RunOrder = RunOrder {
    ids: core::cell::UnsafeCell::new([0usize; RUN_ORDER_MAX]),
    len: core::cell::UnsafeCell::new(0),
};

/// Record a slot's switch-in in the run-order trace.  Present in every
/// bare-metal build; the recording is a no-op outside qemu-test images.
#[cfg(all(target_os = "none", feature = "qemu-test"))]
fn record_run(id: usize) {
    let len = unsafe { *RUN_ORDER.len.get() };
    if len < RUN_ORDER_MAX {
        unsafe {
            (*RUN_ORDER.ids.get())[len] = id;
            *RUN_ORDER.len.get() = len + 1;
        }
    }
}
#[cfg(all(target_os = "none", not(feature = "qemu-test")))]
fn record_run(_id: usize) {}

/// The order in which slots were switched in, up to the bound: the
/// qemu-test images assert scheduling order from this.
#[cfg(all(target_os = "none", feature = "qemu-test"))]
pub fn run_order() -> &'static [usize] {
    let len = unsafe { *RUN_ORDER.len.get() };
    unsafe {
        let ids = &*RUN_ORDER.ids.get();
        &ids[..len]
    }
}

/// The preemption tick: every 100 ms the timer fires, `Tick` sets the
/// flag, and the trap tail (`irq_resched`) does the actual switch.
#[cfg(target_os = "none")]
const TICK_PERIOD: Duration = Duration::from_millis(100);

/// Set by the tick callback, consumed by `irq_resched` at the tail of
/// the IRQ path (swap-false, so a flag raised by one tick is consumed
/// exactly once, by that tick's own tail).
#[cfg(target_os = "none")]
static NEED_RESCHED: AtomicBool = AtomicBool::new(false);

/// Preemptions actually performed: tick-driven `resched` calls that
/// switched.  The image's `preemptions() >= 2` asserts on this; it is
/// incremented only on a switch (not on a tick with nothing else
/// runnable) because that is what distinguishes a healthy tick stream
/// from the stranded-EOI regression, which allows exactly one tick
/// switch and then self-heals through an exit.
#[cfg(target_os = "none")]
static PREEMPTIONS: AtomicU64 = AtomicU64::new(0);

/// The tick never restarts once started (the timer is "not designed
/// for concurrent restarts" per its docs); run_all may in principle
/// run more than once, so the start is guarded by a one-time flag.
#[cfg(target_os = "none")]
static TICK_STARTED: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "none")]
struct Tick;

#[cfg(target_os = "none")]
impl TimerCallback for Tick {
    fn fire(&self) -> bool {
        // Set the flag; the switch happens at the trap tail, never
        // here.  IAR made the intid active and its active priority
        // masks delivery of every interrupt at or below it until the
        // matching EOI; the timer line is level-triggered and stays
        // asserted until CVAL is re-armed.  A context suspended before
        // both (a switch from inside this callback) holds the
        // deassert and the EOI: no further tick is delivered while it
        // is suspended, so round robin degenerates to exactly one
        // preemption per boot.  This is the Plan 9 / Linux shape —
        // the tick sets a flag, the trap tail schedules.
        NEED_RESCHED.store(true, Ordering::Relaxed);
        true
    }
}

#[cfg(target_os = "none")]
static TICK: Tick = Tick;

#[cfg(target_os = "none")]
static TICK_TIMER: Timer = Timer::periodic(TICK_PERIOD, &TICK);

/// The tail of the IRQ path: after CVAL is re-armed (deasserting the
/// level line) and the EOI is done, consume the tick's flag and, if a
/// process is current, reschedule — counting a switch as a
/// preemption.  With TPIDR null (a timer taken while the kernel
/// runs) the flag is simply consumed: there is nothing to preempt.
#[cfg(target_os = "none")]
pub(crate) fn irq_resched() {
    if !NEED_RESCHED.swap(false, Ordering::Acquire) {
        return;
    }
    let cur = unsafe { tpidr_current() };
    if cur.is_null() {
        return;
    }
    if resched() {
        PREEMPTIONS.fetch_add(1, Ordering::Relaxed);
    }
}

/// Preemptions performed so far (see `PREEMPTIONS`).
#[cfg(target_os = "none")]
pub fn preemptions() -> u64 {
    PREEMPTIONS.load(Ordering::Relaxed)
}

/// PSTATE bits masked in every `swtch` spsr this module passes for a
/// context saved inside a trap handler: A, I, F and SS (0x3c0) — the
/// constant Linux's arm64 entry code uses for the same role.  The
/// invariant bit is I: DAIF.I stays masked from vector entry through
/// the final eret, so no nested exception can land while a process's
/// state is being moved between stacks.  D stays unmasked (debug is
/// not armed under QEMU); IL is not writable.
#[cfg(target_os = "none")]
const DAIF_MASKED: u64 = 0x3c0;

// The EL0 vector tail in trap.S: the resume target of a process's entry
// `Context`.  Declared as a function (it ends in eret) and never called.
#[cfg(target_os = "none")]
unsafe extern "C" {
    fn trapret();
}

/// TPIDR_EL1 is the current-process pointer (or null).  Writers run
/// with IRQs masked on this core; there is only one reader side (the
/// same core).
#[cfg(target_os = "none")]
unsafe fn tpidr_current() -> *mut Process {
    let v: u64;
    unsafe { core::arch::asm!("mrs {0}, tpidr_el1", out(reg) v) };
    v as *mut Process
}

#[cfg(target_os = "none")]
unsafe fn tpidr_set(p: *mut Process) {
    unsafe { core::arch::asm!("msr tpidr_el1, {0}", in(reg) p as u64) };
}

/// A process id: an index into the table.
pub type ProcessId = usize;

/// A loadable process image: raw machine code at a fixed layout, or a
/// self-describing ELF.  The two are the only ways a process starts; `spawn`
/// is the single entry point over both (the user-binary-loading plan,
/// decision 2 — an early-call unification).  Defined unconditionally (plain
/// data) so the host build sees it too.
pub enum Image<'a> {
    /// Raw machine code: `text` placed at `text_va`, the stack at `stack_va`.
    /// The caller owns the layout (the simple test images).
    Raw { text: &'a [u8], text_va: usize, stack_va: usize },
    /// A self-describing ELF: layout (segments, entry, sizes) comes from the
    /// header; the stack is derived above the highest segment (the servers).
    /// `handles`, when present, is the spawner-passed channel pair the loader
    /// writes to `HANDLES_VA` before the process starts (see `Handles`).
    Elf { bytes: &'a [u8], handles: Option<Handles> },
}

/// The channel pair the spawner hands a process: written as
/// `[inbound:4 LE][outbound:4 LE]` to [`port::user::HANDLES_VA`] before the
/// process's first instruction.  It is the one way a value crosses from the
/// spawner (a kernel image) into a process's own address space — a server
/// cannot be told its handles by any constant it knows (unlike its own MMIO
/// base), so the spawner must pass them.  For the first server (the
/// nameserver) this is its own pair — it is the first server, so nothing
/// exists yet that a client could ask to find it; for a later server (the
/// console server) this is the nameserver's pair, so it can `BIND` to it.
/// Defined unconditionally (plain data) so the host build sees it too.
#[derive(Clone, Copy)]
pub struct Handles {
    /// The inbound channel: clients send here.
    pub inbound: u32,
    /// The outbound channel: clients receive replies here.
    pub outbound: u32,
}

/// Start a process from an image, mapping its pages into a fresh per-process
/// address space, fabricating its entry frame and `Context` on its kstack,
/// and putting it in the table as Runnable.  Returns the slot.
///
/// The entry `Context` erets into the vector tail (`trapret`) with sp = the
/// frame's base, the same path every later switch-in takes: the tail stages
/// SPSR/ELR/SP_EL0 from the frame and `eret`s.  The process thus starts at
/// the frame's ELR with x30 = 0, not the `trapret` label.
///
/// # Panics
///
/// On a malformed ELF (the `Image::Elf` arm) or a bad segment placement, an
/// allocation failure, or a full table: callers are init-context (`main9`,
/// the test images), where a panic is the failure report.
#[cfg(target_os = "none")]
pub fn spawn(image: &Image) -> ProcessId {
    match image {
        Image::Raw { text, text_va, stack_va } => spawn_raw(text, *text_va, *stack_va),
        Image::Elf { bytes, handles } => spawn_elf(bytes, *handles),
    }
}

/// The `Image::Raw` arm: map one page of `text` at `text_va` and one stack
/// page at `stack_va`, starting the process at `text_va`.  The simple test
/// images' path; behaviour is unchanged from the pre-`Image` `spawn`.
#[cfg(target_os = "none")]
fn spawn_raw(text: &[u8], text_va: usize, stack_va: usize) -> ProcessId {
    let aspace = crate::aspace::Aspace::new();
    let user_text = aspace
        .map_user_page(Entry::rw_user_text(), text_va)
        .unwrap_or_else(|err| panic!("process text page: {err:?}"));
    assert!(text.len() <= mem::PAGE_SIZE_4K, "text too large for one page");
    // SAFETY: user_text is the mapped text page (text_va), valid and
    // writable, and text.len() bytes fit in the 4 KiB page (asserted above).
    unsafe { core::ptr::copy_nonoverlapping(text.as_ptr(), user_text, text.len()) };
    // The stack page itself is mapped and then leaked: the user stack
    // pointer is the only thing the kernel keeps of it.
    let _user_stack = aspace
        .map_user_page(Entry::rw_user_data(), stack_va)
        .unwrap_or_else(|err| panic!("process stack page: {err:?}"));
    // The user stack pointer leaves 16 bytes of headroom below the page's top
    // (the frame's SP must stay inside the page — see forkret_context).
    let user_sp = stack_va + mem::PAGE_SIZE_4K - 16;
    install(aspace, text_va, user_sp)
}

/// The image base a server is linked at: the shared [`port::user::IMAGE_BASE`],
/// which the build's `--image-base` also reads, so the two cannot drift.  A
/// segment placed below it is rejected by the placement check in `spawn_elf`.
#[cfg(target_os = "none")]
const ELF_BASE: usize = port::user::IMAGE_BASE;
/// The user stack's size, in pages: the same 64 KiB as a kstack, mapped
/// immediately above the highest loaded segment.  A software convention, not
/// a hardware fact.
#[cfg(target_os = "none")]
const STACK_PAGES: usize = 16;

/// The `Image::Elf` arm: load a self-describing static ELF.  Parse it, validate
/// each segment's placement (arch-specific — the `port::elf` reader checks
/// structure only), map the segments into a fresh `Aspace` (executable text vs.
/// data), copy the file bytes and zero the bss, map the stack above the
/// highest segment, write the spawner-passed channel pair to `HANDLES_VA`
/// (when present), and start the process at `e_entry`.
#[cfg(target_os = "none")]
fn spawn_elf(bytes: &[u8], handles: Option<Handles>) -> ProcessId {
    let image = port::elf::parse(bytes).unwrap_or_else(|err| panic!("elf: {err:?}"));
    let segs = image.segments();

    // Placement is arch-specific and the loader's to check: each segment must
    // be at or above the image base, and its page span must lie in the user
    // half (`< KZERO`) without overlapping a prior segment's.  A segment need
    // not be page-aligned — a static ELF places it wherever the linker chose —
    // so the loader maps the containing pages.  An embedded ELF is still input:
    // a malformed or mis-linked one is rejected, not mapped into kernel space
    // or on top of itself.
    let mut top = 0usize; // the highest mapped page, for the stack below
    for (i, seg) in segs.iter().enumerate() {
        let vaddr = seg.vaddr as usize;
        if vaddr < ELF_BASE {
            panic!("elf: segment {i} is below the image base: {vaddr:#x}");
        }
        let end = vaddr
            .checked_add(seg.memsz as usize)
            .unwrap_or_else(|| panic!("elf: segment {i} size overflows the address space"));
        let page_lo = vaddr & !(mem::PAGE_SIZE_4K - 1);
        let page_hi = (end + mem::PAGE_SIZE_4K - 1) & !(mem::PAGE_SIZE_4K - 1);
        if page_hi >= crate::param::KZERO {
            panic!("elf: segment {i} is outside the user half: {page_hi:#x}");
        }
        for prev in &segs[..i] {
            let pv = prev.vaddr as usize;
            let pend = pv + prev.memsz as usize;
            let p_lo = pv & !(mem::PAGE_SIZE_4K - 1);
            let p_hi = (pend + mem::PAGE_SIZE_4K - 1) & !(mem::PAGE_SIZE_4K - 1);
            if page_lo < p_hi && p_lo < page_hi {
                panic!("elf: segments overlap: {vaddr:#x} and {pv:#x}");
            }
        }
        if page_hi > top {
            top = page_hi;
        }
    }

    // The entry point must land in a mapped, executable segment: a valid
    // static ELF's `e_entry` is the linker's `start` symbol, in the text.
    // Given the segment checks above, this implies it is in the user half at
    // or above the image base; a malformed ELF whose entry floats in a gap or
    // in kernel space is rejected, not started at an unmapped or privileged VA.
    let entry = image.entry as usize;
    let in_segment = segs.iter().any(|seg| {
        seg.exec && entry >= seg.vaddr as usize && entry < seg.vaddr as usize + seg.memsz as usize
    });
    if !in_segment {
        panic!("elf: entry {entry:#x} is not in any segment");
    }

    // Map each segment's page span into a fresh AS (its TTBR0 root), zero
    // every page, and copy the file bytes over the zeros (the bss and each
    // page's unfiled tail stay zero).  A segment may start or end mid-page, so
    // each page takes only the file bytes that fall inside it, at the
    // page-internal offset.  `map_user_page` returns the kernel pointer (the
    // identity VA in TTBR1) — the kernel runs in TTBR1 and cannot write through
    // the user VA — so the copy goes through it, page by page.
    let aspace = crate::aspace::Aspace::new();
    for seg in segs {
        let entry = if seg.exec { Entry::rw_user_text() } else { Entry::rw_user_data() };
        let vaddr = seg.vaddr as usize;
        let filesz = seg.filesz as usize;
        let memsz = seg.memsz as usize;
        let offset = seg.offset as usize;
        let page_lo = vaddr & !(mem::PAGE_SIZE_4K - 1);
        let page_hi = (vaddr + memsz + mem::PAGE_SIZE_4K - 1) & !(mem::PAGE_SIZE_4K - 1);
        let mut page = page_lo;
        while page < page_hi {
            let kptr = aspace
                .map_user_page(entry, page)
                .unwrap_or_else(|err| panic!("elf: segment page: {err:?}"));
            // SAFETY: kptr is a freshly mapped page, valid and kernel-writable;
            // a full 4 KiB page fits.
            unsafe { core::ptr::write_bytes(kptr, 0, mem::PAGE_SIZE_4K) };
            // This page's share of the file bytes: the segment's bytes are at
            // VA [vaddr, vaddr + filesz) and file [offset, offset + filesz);
            // copy whatever falls within this page, at the page-internal
            // offset.
            let va_lo = core::cmp::max(page, vaddr);
            let va_hi = core::cmp::min(page + mem::PAGE_SIZE_4K, vaddr + filesz);
            if va_lo < va_hi {
                let n = va_hi - va_lo;
                // SAFETY: `va_lo - page` is the offset within this page (0 to
                // the page size), so the result points inside the mapped page.
                let dst = unsafe { kptr.add(va_lo - page) };
                let src = &bytes[offset + (va_lo - vaddr)..offset + (va_hi - vaddr)];
                // SAFETY: dst is within the just-mapped page (va_lo - page +
                // n <= the page size); the source slice is in-bounds (the
                // reader verified offset + filesz <= len, and va_hi - vaddr
                // <= filesz).
                unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), dst, n) };
            }
            page += mem::PAGE_SIZE_4K;
        }
    }

    // The stack: STACK_PAGES pages immediately above the highest segment
    // (page-aligned up, so it is clear of every mapped page).
    let stack_base = (top + mem::PAGE_SIZE_4K - 1) & !(mem::PAGE_SIZE_4K - 1);
    for page in 0..STACK_PAGES {
        let _kptr = aspace
            .map_user_page(Entry::rw_user_data(), stack_base + page * mem::PAGE_SIZE_4K)
            .unwrap_or_else(|err| panic!("elf: stack page: {err:?}"));
    }
    // The spawner-passed channel pair: if present, map `HANDLES_VA` and write
    // `[inbound:4 LE][outbound:4 LE]`.  The page sits in the user half, clear
    // of the image (at `ELF_BASE`) and its stack by a wide margin, so it is
    // never a segment or a stack page the placement checks above would place.
    if let Some(h) = handles {
        let mut pair = [0u8; 8];
        pair[0..4].copy_from_slice(&h.inbound.to_le_bytes());
        pair[4..8].copy_from_slice(&h.outbound.to_le_bytes());
        let kptr = aspace
            .map_user_page(Entry::rw_user_data(), port::user::HANDLES_VA)
            .unwrap_or_else(|err| panic!("elf: handles page: {err:?}"));
        // SAFETY: `kptr` is a freshly mapped page, valid and kernel-writable,
        // and 8 bytes fit.
        unsafe { core::ptr::copy_nonoverlapping(pair.as_ptr(), kptr, 8) };
    }
    let user_sp = stack_base + STACK_PAGES * mem::PAGE_SIZE_4K - 16;
    install(aspace, image.entry as usize, user_sp)
}

/// Claim a free slot and store a new process with the given `Aspace`, entry
/// point (`elr`), and user stack pointer (`user_sp`).  Shared by the raw and
/// ELF spawn arms: the only difference between them is how the `Aspace` is
/// built and what the entry/stack point at.  Called after the `Aspace` is
/// fully built (the page allocs already ran, outside the table lock).
#[cfg(target_os = "none")]
fn install(aspace: crate::aspace::Aspace, elr: usize, user_sp: usize) -> ProcessId {
    let node = LockNode::new();
    // A slot is free when it is empty or already Exited: reclaiming is
    // lazy, and a slot is never overwritten while a raw pointer to its
    // process is live (module docs).  Finding and claiming the slot and
    // fabricating the kstack frame are one critical section: two concurrent
    // spawns must never pick the same slot and interleave frame writes into
    // the same kstack.  The frame writes are plain stores, so holding the
    // table lock across them costs nothing the allocator lock already does
    // not.
    let mut table = TABLE.lock(&node);
    let id = table.iter().position(|slot| match slot {
        None => true,
        Some(p) => p.state == State::Exited,
    });
    let Some(id) = id else {
        panic!("proc: no free slot: all {NPROCS} slots are Running or Runnable")
    };
    let kstack = unsafe { KSTACKS.stacks.get().cast::<u8>().add(id * KSTACK_SZ) };
    let context = forkret_context(id, elr, user_sp);
    let proc = Process {
        state: State::Runnable,
        context,
        kstack,
        exit_status: 0,
        prio: PriorityState::new(DEFAULT_PRIORITY),
        aspace,
    };
    table[id] = Some(proc);
    id
}

/// Fabricate a process's entry frame and `Context` on its kstack.
///
/// The frame sits at the kstack's top (304 bytes); the `Context` (112)
/// sits directly below it.  The canary goes at the base.  `elr` is where the
/// process starts (the text VA for `Image::Raw`, `e_entry` for `Image::Elf`)
/// and `user_sp` is its user stack pointer (the caller has already left 16
/// bytes of headroom below the stack's top, so an EL0 store to `[sp, #8]`
/// stays inside the stack).
#[cfg(target_os = "none")]
fn forkret_context(id: usize, elr: usize, user_sp: usize) -> *mut Context {
    let kstack = unsafe { KSTACKS.stacks.get().cast::<u8>().add(id * KSTACK_SZ) };
    let top = kstack as usize + KSTACK_SZ;

    unsafe {
        // The canary at the base, asserted in resched before every
        // switch (the kstacks are statics: no guard pages to catch an
        // overflow in hardware).
        (kstack as *mut u64).write_volatile(KSTACK_CANARY);

        let frame_base = top - FRAME_SZ;
        // The frame and the context below it are kernel state, so zero
        // them before the field writes (all bytes initialised).
        let context_base = frame_base - CONTEXT_SZ;
        (context_base as *mut u8).write_bytes(0, FRAME_SZ + CONTEXT_SZ);

        // The frame's fields the tail will stage: ELR = where the
        // process starts, sp = its user stack, SPSR = 0 (EL0, SP0,
        // IRQs unmasked: the process enters with IRQs on, as the
        // single-process arc did).
        let frame = frame_base as *mut u8;
        frame.add(FRAME_ELR).cast::<u64>().write(elr as u64);
        frame.add(FRAME_SP).cast::<u64>().write(user_sp as u64);
        frame.add(FRAME_SPSR).cast::<u64>().write(0);

        // The entry Context.  Its x30 is trapret, not the text VA:
        // ereting into the tail runs exactly the path every later
        // switch-in takes (stage SPSR/ELR/SP_EL0 from the frame,
        // restore the user registers, eret), so the first entry is
        // indistinguishable from a resume and the tail is the single
        // EL0 return path.  The process itself starts at the frame's
        // ELR with x30 = 0 (the frame's), not trapret.
        //
        // The Context sits directly below the frame — deliberately:
        // nothing may live there after the first trap, because the
        // handler's own frames grow down from the frame and would
        // clobber it.  It is consumed by the first entry and dead
        // after: the first trap's handler frames clobber this memory,
        // but the process's `context` pointer is re-pointed to a
        // suspend frame on its first switch-out — which always
        // precedes any later switch-in (a process is selectable only
        // after being demoted, and the demotion is the switch-out).
        // The invariant that keeps the clobber harmless: a context
        // pointer is never read for a switch-in unless it was last
        // written by a switch-out of that same process, or it is the
        // forkret context of a process that has not trapped yet.
        let ctx = context_base as *mut Context;
        (*ctx).x30 = trapret as *const () as u64;
        (*ctx).sp = frame_base as u64;
        (*ctx).spsr = SPSR_EL1H | DAIF_MASKED;
        ctx
    }
}

/// Switch to the first Runnable process and run the table until no
/// process is Runnable any more (every process has exited).  Returns
/// in the context that called it; the table is not reset, so
/// `status` works after the return.
///
/// # Panics
///
/// If no process is Runnable: nothing to run is a caller error this
/// arc (the images spawn before they run).
#[cfg(target_os = "none")]
pub fn run_all() {
    // The mask covers the TPIDR write through the swtch: an IRQ
    // landing in that window would see TPIDR non-null while the kernel
    // is still on the caller's stack, not a process's kstack.
    let _guard = IrqGuard::new();
    // Start the preemption tick before the first switch: a one-time
    // start, never restarted or cancelled.  It keeps firing after the
    // table runs empty, harmlessly — with no current process the
    // tail consumes the flag and does nothing.
    if !TICK_STARTED.swap(true, Ordering::AcqRel) {
        // The tick is the scheduler's heartbeat: without it the kernel
        // cannot pre-empt, and a kernel that cannot pre-empt cannot go
        // on.
        TICK_TIMER.start().unwrap();
    }
    let node = LockNode::new();
    let first = {
        let table = TABLE.lock(&node);
        let id =
            table.iter().position(|slot| matches!(slot, Some(p) if p.state == State::Runnable));
        let Some(id) = id else { panic!("proc: run_all with no Runnable process") };
        let mut table = table;
        let Some(p) = table[id].as_mut() else { unreachable!() };
        p.state = State::Running;
        record_run(id);
        p as *mut Process
    };
    let starter = starter_ctx_addr();
    unsafe {
        tpidr_set(first);
        // Install the first process's TTBR0 before the switch: the table must
        // be live before the process's first EL0 instruction.
        // SAFETY: the first process's AS is live (built at spawn, never freed
        // this arc).
        (*first).aspace.install();
        swtch(starter, (*first).context, SPSR_EL1H | DAIF_MASKED);
    }
    // Resumed: the last process exited (exit_current switched back to
    // the starter with TPIDR already cleared).  The guard's drop
    // unmasking completes the return.
}

/// The exit status of `id`, if it has exited.
#[cfg(target_os = "none")]
pub fn status(id: ProcessId) -> Option<u64> {
    let node = LockNode::new();
    let table = TABLE.lock(&node);
    match table.get(id) {
        Some(Some(p)) if p.state == State::Exited => Some(p.exit_status),
        _ => None,
    }
}

/// Set `id`'s base priority (and its effective, when not boosted).  A process
/// is spawned at [`DEFAULT_PRIORITY`]; this is how a test image places a
/// process at a specific level (a boost only raises, so it cannot lower a
/// base).  A no-op if `id` is not a live slot.
#[cfg(target_os = "none")]
pub fn set_priority(id: ProcessId, prio: Priority) {
    let node = LockNode::new();
    let mut table = TABLE.lock(&node);
    if let Some(Some(p)) = table.get_mut(id) {
        p.prio.base = prio;
        if !p.prio.is_boosted() {
            p.prio.effective = prio;
        }
    }
}

/// Raise the effective priority of `id` to `to` (at or above its base),
/// remembering its base so `unboost` can restore it.  This is the hook a
/// blocking wait will use to hand a waiter's priority to the resource it is
/// held on (priority inheritance); stage 2 fires it on a blocking send,
/// passing the waiter's own priority as `to`.  PI is at most once per slot:
/// re-boosting a boosted slot is a no-op (see `PriorityState`).  A no-op if
/// `id` is not a live slot.
#[cfg(target_os = "none")]
pub fn boost(id: ProcessId, to: Priority) {
    let node = LockNode::new();
    let mut table = TABLE.lock(&node);
    if let Some(Some(p)) = table.get_mut(id) {
        p.prio.boost(to);
    }
}

/// Restore `id`'s effective priority to its base.  The inverse of
/// `boost`; a no-op if `id` is not a live slot or is not boosted.
#[cfg(target_os = "none")]
pub fn unboost(id: ProcessId) {
    let node = LockNode::new();
    let mut table = TABLE.lock(&node);
    if let Some(Some(p)) = table.get_mut(id) {
        p.prio.unboost();
    }
}

/// The effective priority a slot currently runs at, if it is a live slot.
/// The read half of the boost/`unboost` capability: a qemu-test image
/// confirms a boost took effect through it, and stage 2's IPC reads
/// inherited priorities from it.
#[cfg(target_os = "none")]
pub fn effective_priority(id: ProcessId) -> Option<Priority> {
    let node = LockNode::new();
    let table = TABLE.lock(&node);
    table.get(id).and_then(|s| s.as_ref()).map(|p| p.prio.effective())
}

// The narrow seam for trap.rs: the two things an EL0 svc can do to the
// process it came from.  Both require a current process; a null TPIDR
// in either is a kernel bug (an EL0 svc cannot be taken without a
// process running), reported loudly.
#[cfg(target_os = "none")]
pub(crate) fn yield_current() {
    resched();
}

/// End the current process with `status`.  Never returns: either
/// `resched` switches to the next Runnable (and this context is
/// suspended), or the table is empty and the switch back to the starter
/// unwinds `run_all`.
#[cfg(target_os = "none")]
pub(crate) fn exit_current(status: u64) -> ! {
    let current = unsafe { tpidr_current() };
    if current.is_null() {
        iprintln!("exit trap with no process running");
        loop {
            core::hint::spin_loop();
        }
    }

    let node = LockNode::new();
    {
        let mut table = TABLE.lock(&node);
        let Some(slot) =
            table.iter_mut().find(|slot| matches!(slot, Some(p) if p.state == State::Running))
        else {
            panic!("exit_current: no Running process in the table");
        };
        let p = slot.as_mut().unwrap();
        p.exit_status = status;
        p.state = State::Exited;
    }
    iprintln!("process exited, status {status}");

    if !resched() {
        // No next Runnable: the last process.  Unwind run_all: switch
        // back to the kernel context run_all saved (the starter),
        // discarding this handler's context.  The from slot is a local:
        // the starter address must survive the switch, and the process's
        // own context is not the target — it is the dead forkret context
        // (or a stale handler one), and ereting into a stale context is
        // how a resumed kernel finds itself in EL0 at a stale address.
        unsafe { tpidr_set(core::ptr::null_mut()) };
        // The suspended handler's trap_unsafe epilogue would never
        // balance its enter_interrupt; do it now, or the resumed kernel
        // believes it is still in interrupt context and every println
        // trips the in_interrupt assert.
        port::irq::exit_interrupt();
        let starter = unsafe { core::ptr::read(starter_ctx_addr()) };
        let mut slot: *mut Context = core::ptr::null_mut();
        unsafe { swtch(&mut slot, starter, SPSR_EL1H | DAIF_MASKED) };
        unreachable!("swtch resumed the discarded trap context");
    }
    unreachable!("resched switched to the next process");
}

/// Switch from the current process to the next Runnable one, if there
/// is one, demoting the current process to `demote_to` in the table.
/// Called only from a trap handler with the current process on CPU
/// (TPIDR non-null).
///
/// `demote_to` is `Runnable` for a yield/preempt (the process is
/// selectable again immediately) and `Blocked` for a blocking wait (it
/// is selectable only after a matching `wake`).  Either way, the
/// switch-back is an ordinary selection: the demoted process is picked
/// by a later `switch_out` once it is Runnable again.
///
/// Returns `true` — and never returns at all — when it switched: the
/// caller's context is suspended on the kstack and will resume inside
/// this same handler's frame (the vector tail completes the return to
/// EL0 from there).  Returns `false` when nothing else was Runnable.
///
/// The bracketing: the caller is in interrupt context (depth 1).  The
/// switched-to process must run *outside* interrupt context — without
/// the exit, depth would stay 1 across the switch and every `println`
/// while it runs trips the `in_interrupt` assert — so `exit_interrupt`
/// before the `swtch`, and on resume `enter_interrupt`: we are back
/// inside the suspended handler, whose `trap_unsafe` epilogue exits it.
#[cfg(target_os = "none")]
fn switch_out(demote_to: State) -> bool {
    let current = unsafe { tpidr_current() };
    if current.is_null() {
        // No process on CPU: nothing to switch from.  The EL1 path
        // (a handler taken while the kernel runs) reaches this and
        // does nothing.
        return false;
    }

    // A kstack overflow walks the canary at the base; catch it while
    // the evidence is fresh (before either context is moved).
    unsafe {
        let cur_canary = ((*current).kstack as *const u64).read_volatile();
        // A hard assert: the canary is the only kstack-overflow
        // detector and a release image must not lose it.
        assert_eq!(cur_canary, KSTACK_CANARY, "kstack overflow: current process's canary is gone");
    }

    let node = LockNode::new();
    let (cur, next): (*mut Process, Option<*mut Process>) = {
        let mut table = TABLE.lock(&node);
        // The slot the current pointer sits in.  A slot is never
        // overwritten while a pointer to it is live, so the comparison
        // is stable for the lock's duration.
        let current_id = table
            .iter()
            .position(|slot| {
                matches!(slot, Some(p) if (p as *const Process as *const ()) == (current as *const Process as *const ()))
            })
            .unwrap_or_else(|| panic!("resched: current process not in the table"));

        // Demote the current process: `Runnable` for a yield/preempt
        // (selectable again immediately), `Blocked` for a blocking wait
        // (selectable only after a matching `wake`).  The condition
        // keeps the exit path from resurrecting a process it just marked
        // Exited — and the `demoted` flag keeps the re-mark below from
        // doing the same.
        let mut demoted = false;
        if let Some(p) = table[current_id].as_mut()
            && p.state == State::Running
        {
            p.state = demote_to;
            demoted = true;
        }

        // Selection is by highest effective priority, then round-robin
        // within the winning class: build the per-slot (state, priority)
        // view and let pick_next choose.  An empty slot reads Exited and
        // is never picked.
        let slots: [(State, Priority); NPROCS] = core::array::from_fn(|i| match &table[i] {
            Some(p) => (p.state, p.prio.effective()),
            None => (State::Exited, Priority::IDLE),
        });
        let next_id = pick_next(&slots, current_id, CURSOR.load(Ordering::Relaxed));

        match next_id {
            None => {
                // No next: the demotion was premature and the current
                // process is still on CPU.  For a `Runnable` demotion
                // (yield/preempt) put the state back — but only if we
                // changed it; re-marking an Exited process Running would
                // resurrect it.  For a `Blocked` demotion leave the state
                // as Blocked: the caller (`block_current`) handles the
                // all-blocked case by returning to the kernel.
                if demoted
                    && demote_to == State::Runnable
                    && let Some(p) = table[current_id].as_mut()
                {
                    p.state = State::Running;
                }
                (current, None)
            }
            Some(nid) => {
                let Some(p) = table[nid].as_mut() else { unreachable!() };
                p.state = State::Running;
                CURSOR.store(nid, Ordering::Relaxed);
                record_run(nid);
                (current, Some(p as *mut Process))
            }
        }
    };

    let Some(next) = next else { return false };

    unsafe {
        let next_canary = ((*next).kstack as *const u64).read_volatile();
        assert_eq!(next_canary, KSTACK_CANARY, "kstack overflow: next process's canary is gone");
    }

    // The lock is dropped before the switch: the suspended context's
    // eventual resched, same core, must not self-deadlock on it.
    unsafe { tpidr_set(next) };
    // Install the next process's TTBR0 before the switch: the table must be
    // live before the process's first EL0 instruction.  TPIDR is set first so
    // a fault on that first instruction can find the process.
    // SAFETY: the next process's AS is live (built at spawn, never freed this
    // arc); install puts its root in TTBR0 with the TLBI/DSB/ISB the switch
    // needs.
    unsafe { (*next).aspace.install() };
    port::irq::exit_interrupt();
    unsafe { swtch(&mut (*cur).context, (*next).context, SPSR_EL1H | DAIF_MASKED) };

    // Resumed inside the suspended handler.
    port::irq::enter_interrupt();
    unsafe { tpidr_set(cur) };
    true
}

/// Kill the current process with a fault: a data or instruction abort in EL0.
/// Prints the FAR/ESR and the faulting process's id, marks the process
/// `Exited` with [`FAULT_STATUS`], and reschedules (the same path
/// `exit_current` takes).  A fault with no current process (TPIDR null) is a
/// kernel fault: print and spin.  This arc has no demand-paging, so every EL0
/// fault is a kill (the fix path is a later change to this one function).
#[cfg(target_os = "none")]
pub(crate) fn fault(far: u64, esr: crate::reg::esr_el1::EsrEl1) -> ! {
    let current = unsafe { tpidr_current() };
    if current.is_null() {
        iprintln!("EL0 fault with no process running: far {far:#x} esr {esr:?}");
        loop {
            core::hint::spin_loop();
        }
    }

    let node = LockNode::new();
    // The slot whose pointer matches the TPIDR value is the faulting
    // process.  `panic!` (as `switch_out` uses) rather than a silent
    // fallback: a mismatched TPIDR is a kernel bug, and the default panic
    // handler prints and halts — visible, rather than marking the wrong
    // process.
    let current_id = {
        let mut table = TABLE.lock(&node);
        let id = table
            .iter()
            .position(|slot| {
                matches!(
                    slot,
                    Some(p)
                        if (p as *const Process as *const ())
                            == (current as *const Process as *const ())
                )
            })
            .unwrap_or_else(|| panic!("fault: TPIDR does not match any slot"));
        let p = table
            .get_mut(id)
            .and_then(|slot| slot.as_mut())
            .unwrap_or_else(|| panic!("fault: slot {id} is empty"));
        if p.state != State::Running {
            panic!("fault: process {id} is not Running (state {:?})", p.state);
        }
        p.exit_status = FAULT_STATUS;
        p.state = State::Exited;
        id
    };
    iprintln!("process {current_id} faulted: far {far:#x} esr {esr:?}");

    if !resched() {
        // No next Runnable: the last process.  Unwind run_all.
        unsafe { tpidr_set(core::ptr::null_mut()) };
        port::irq::exit_interrupt();
        let starter = unsafe { core::ptr::read(starter_ctx_addr()) };
        let mut slot: *mut Context = core::ptr::null_mut();
        unsafe { swtch(&mut slot, starter, SPSR_EL1H | DAIF_MASKED) };
        unreachable!("swtch resumed the discarded trap context");
    }
    unreachable!("resched switched to the next process");
}

/// Yield or preempt: switch to the next Runnable process, demoting the
/// current one to Runnable (it is selectable again immediately).
#[cfg(target_os = "none")]
fn resched() -> bool {
    switch_out(State::Runnable)
}

/// Put the current process off the ready set (a blocking wait).  Switches to
/// the next Runnable process and does not return until the process is `wake`
///-en and selected again.  If nothing else is Runnable the process stays
/// Blocked and the kernel regains control (`run_all` returns), so the kernel
/// can do work (a `try_send` that wakes the blocked process) and re-enter
/// the scheduler.
#[cfg(target_os = "none")]
pub(crate) fn block_current() -> bool {
    if switch_out(State::Blocked) {
        return true;
    }
    // `switch_out` returned false: no next process.  The current process is
    // now Blocked (not resurrected to Running).  All processes are blocked:
    // save this process's context (it resumes when woken) and switch back to
    // the kernel (the starter), so `run_all` returns.
    let current = unsafe { tpidr_current() };
    if current.is_null() {
        return false;
    }
    unsafe { tpidr_set(core::ptr::null_mut()) };
    port::irq::exit_interrupt();
    let starter = unsafe { core::ptr::read(starter_ctx_addr()) };
    // The from slot is the process's own context field in the table: when
    // the process is later woken and selected, `switch_out` loads it and
    // execution resumes here (the suspended handler's continuation).
    // SAFETY: `current` is a live table slot; its context field is the
    // saved switch point.  The starter is the kernel context `run_all`
    // saved before its first switch.
    unsafe { swtch(&mut (*current).context, starter, SPSR_EL1H | DAIF_MASKED) };
    // Resumed inside the suspended handler: the process was woken and
    // selected.  Re-enter interrupt context (the `exit_interrupt` above
    // was for the kernel side) and restore TPIDR.
    port::irq::enter_interrupt();
    unsafe { tpidr_set(current) };
    true
}

/// Put `id` back on the ready set: a Blocked process becomes Runnable and is
/// selectable at the next selection.  A no-op unless `id` is a live Blocked
/// slot (waking a Running/Runnable/Exited slot is a caller bug).
#[cfg(target_os = "none")]
pub(crate) fn wake(id: ProcessId) {
    let node = LockNode::new();
    let mut table = TABLE.lock(&node);
    if let Some(Some(p)) = table.get_mut(id)
        && p.state == State::Blocked
    {
        p.state = State::Runnable;
    }
}

/// The id of the process on CPU (TPIDR non-null), if any.
#[cfg(target_os = "none")]
pub(crate) fn current_id() -> Option<ProcessId> {
    let current = unsafe { tpidr_current() };
    if current.is_null() {
        return None;
    }
    let node = LockNode::new();
    let table = TABLE.lock(&node);
    table
        .iter()
        .position(|slot| {
            matches!(slot, Some(p) if (p as *const Process as *const ()) == (current as *const Process as *const ()))
        })
}

/// The current process's `Aspace`, if a process is on CPU (TPIDR non-null).
/// The caller (a syscall handler) runs in the process's context; the Aspace
/// is written only by this process's own `map_mmio` call (no other core
/// writes to this process's root), so a shared reference is sufficient.
#[cfg(target_os = "none")]
pub(crate) fn current_aspace() -> Option<&'static crate::aspace::Aspace> {
    let current = unsafe { tpidr_current() };
    if current.is_null() {
        return None;
    }
    // SAFETY: TPIDR points to a live `Process` in the table (the table's
    // discipline: a slot's `Process` is never freed or reused while TPIDR
    // points to it); the `Aspace` field lives for the process's life.
    Some(unsafe { &(*current).aspace })
}

// Host (unit-test) builds have no table, no kstacks, and no switch;
// the constants above are what they exercise.
#[cfg(not(target_os = "none"))]
pub fn spawn(_image: &Image) -> ProcessId {
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(not(target_os = "none"))]
pub fn run_all() {
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(not(target_os = "none"))]
pub fn status(_id: ProcessId) -> Option<u64> {
    None
}

#[cfg(not(target_os = "none"))]
pub(crate) fn yield_current() {
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(not(target_os = "none"))]
pub(crate) fn exit_current(_status: u64) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(not(target_os = "none"))]
pub(crate) fn fault(_far: u64, _esr: crate::reg::esr_el1::EsrEl1) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(not(target_os = "none"))]
pub(crate) fn irq_resched() {}

#[cfg(not(target_os = "none"))]
pub fn preemptions() -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decision 3 fallback: the syscall numbers `process` re-exports must equal
    /// `r9x_abi`, the single source both the kernel and the `r9x_std` target read.
    #[test]
    fn syscall_numbers_match_r9x_abi() {
        assert_eq!(SYCCREATECHAN, r9x_abi::SYCCREATECHAN);
        assert_eq!(SYCRECEIVE, r9x_abi::SYCRECEIVE);
        assert_eq!(SYCREPLY, r9x_abi::SYCREPLY);
        assert_eq!(SYCSEND, r9x_abi::SYCSEND);
        assert_eq!(SYSEXIT, r9x_abi::SYSEXIT);
        assert_eq!(SYSIRQCLAIM, r9x_abi::SYSIRQCLAIM);
        assert_eq!(SYSMAPMMIO, r9x_abi::SYSMAPMMIO);
        assert_eq!(SYSYIELD, r9x_abi::SYSYIELD);
    }

    fn s(state: State, prio: Priority) -> (State, Priority) {
        (state, prio)
    }

    // QNX levels, lower more urgent: a high-urgency boost target, the
    // normal-user default, and a low-urgency one.
    const HIGH: Priority = Priority::new(16);
    const MID: Priority = Priority::new(128);
    const LOW: Priority = Priority::new(200);

    #[test]
    fn picks_the_highest_priority_runnable() {
        // Slot 0 empty, 1 a MID (current), 2 a HIGH, 3 a MID.  The HIGH
        // at 2 beats the MID at 3.
        let slots = [
            s(State::Exited, MID),
            s(State::Runnable, MID),
            s(State::Runnable, HIGH),
            s(State::Runnable, MID),
        ];
        assert_eq!(pick_next(&slots, 1, 1), Some(2));
    }

    #[test]
    fn same_priority_round_robin_from_cursor() {
        // Three MID processes: the pick rotates with the cursor.
        let slots = [s(State::Runnable, MID); 3];
        assert_eq!(pick_next(&slots, 0, 0), Some(1));
        assert_eq!(pick_next(&slots, 1, 1), Some(2));
        assert_eq!(pick_next(&slots, 2, 2), Some(0));
    }

    #[test]
    fn boost_raises_and_unboost_restores() {
        let mut p = PriorityState::new(MID);
        assert_eq!(p.effective(), MID);
        assert!(!p.is_boosted());
        p.boost(HIGH);
        assert_eq!(p.effective(), HIGH);
        assert!(p.is_boosted());
        p.unboost();
        assert_eq!(p.effective(), MID);
        assert!(!p.is_boosted());
    }

    #[test]
    fn boosted_sorts_above_peers_and_drops_back() {
        // Slot 0 a MID peer, slot 1 boosted to HIGH.  While boosted, slot
        // 1 is picked for its urgency whatever the cursor is.
        let boosted = [s(State::Runnable, MID), s(State::Runnable, HIGH)];
        assert_eq!(pick_next(&boosted, 2, 0), Some(1));
        assert_eq!(pick_next(&boosted, 2, 1), Some(1));
        // Once unboosted the two are tied (both MID): the pick follows the
        // cursor (round-robin), not slot 1's former privilege — cursor past
        // the peer serves the peer.
        let unboosted = [s(State::Runnable, MID), s(State::Runnable, MID)];
        assert_eq!(pick_next(&unboosted, 2, 0), Some(1));
        assert_eq!(pick_next(&unboosted, 2, 1), Some(0));
    }

    #[test]
    fn reboost_is_a_no_op() {
        let mut p = PriorityState::new(MID);
        p.boost(HIGH);
        // A second boost, even to a different (lower-urgency) value, does
        // not restack.
        p.boost(LOW);
        assert_eq!(p.effective(), HIGH);
        p.unboost();
        assert_eq!(p.effective(), MID);
    }

    #[test]
    fn current_is_never_reselected() {
        // Only the current slot is Runnable; it must not be reselected.
        let slots = [s(State::Runnable, HIGH), s(State::Exited, MID)];
        assert_eq!(pick_next(&slots, 0, 0), None);
    }

    #[test]
    fn running_is_not_selectable() {
        // A Running slot other than the current is on CPU, not Runnable:
        // it is not picked even at a higher urgency.
        let slots = [s(State::Running, HIGH), s(State::Runnable, MID), s(State::Exited, MID)];
        // current is the Exited slot (the kernel); the Running HIGH at 0
        // must not be picked — the Runnable MID at 1 is.
        assert_eq!(pick_next(&slots, 2, 2), Some(1));
    }
}
