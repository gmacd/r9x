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
use core::sync::atomic::{AtomicUsize, Ordering};

#[cfg(target_os = "none")]
use crate::pagealloc;
#[cfg(target_os = "none")]
use crate::swtch::{Context, SPSR_EL1H, swtch};
#[cfg(target_os = "none")]
use crate::vm::{self, Entry, RootPageTableType, VaMapping};
#[cfg(target_os = "none")]
use port::irq::IrqGuard;
#[cfg(target_os = "none")]
use port::mcslock::{Lock, LockNode};
#[cfg(target_os = "none")]
use port::{iprintln, mem};

/// x0 value for exit: the process asks to be killed.  The syscall
/// number is x0 at the trap (the Linux convention; the svc immediate
/// in ESR_EL1.ISS is not used), so a terminate-with-status is `mov
/// x0, #n; svc #0`.
pub const SYSEXIT: u64 = 0;

/// x0 value for yield: return to the process; if another process is
/// Runnable, the handler's `resched` switches to it first.
pub const SYSYIELD: u64 = 1;

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

#[cfg(target_os = "none")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    /// On CPU.  At most one process at a time, this arc.
    Running,
    /// May be switched to.
    Runnable,
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

/// Start a process: map its text and stack pages into the user table,
/// fabricate its entry frame and `Context` on its kstack, and put it
/// in the table as Runnable.  Returns the slot.
///
/// The entry `Context` erets into the vector tail (`trapret`) with sp
/// = the frame's base, the same path every later switch-in takes: the
/// tail stages SPSR/ELR/SP_EL0 from the frame and `eret`s.  The
/// process thus starts at the frame's ELR (the text VA) with x30 = 0,
/// not the `trapret` label.
///
/// # Panics
///
/// On allocation failure or a full table: callers are init-context
/// (`main9`, the test images), where a panic is the failure report.
#[cfg(target_os = "none")]
pub fn spawn(text: &[u8], text_va: usize, stack_va: usize) -> ProcessId {
    // Allocations run outside the table lock: the lock guards the
    // table, not the page allocator.
    let user_text = pagealloc::allocate_virtpage(
        vm::user_pagetable(),
        "proctxt",
        Entry::rw_user_text(),
        VaMapping::Addr(text_va),
        RootPageTableType::User,
    )
    .unwrap_or_else(|err| panic!("process text page: {err:?}"));
    user_text.0[..text.len()].copy_from_slice(text);
    // The stack page itself is mapped and then leaked: the user stack
    // pointer is the only thing the kernel keeps of it.
    let _user_stack = pagealloc::allocate_virtpage(
        vm::user_pagetable(),
        "procstack",
        Entry::rw_user_data(),
        VaMapping::Addr(stack_va),
        RootPageTableType::User,
    )
    .unwrap_or_else(|err| panic!("process stack page: {err:?}"));
    let node = LockNode::new();
    // A slot is free when it is empty or already Exited: reclaiming is
    // lazy, and a slot is never overwritten while a raw pointer to its
    // process is live (module docs).  Finding and claiming the slot
    // and fabricating the kstack frame are one critical section: two
    // concurrent spawns must never pick the same slot and interleave
    // frame writes into the same kstack.  The frame writes are plain
    // stores, so holding the table lock across them costs nothing the
    // allocator lock already does not.
    let mut table = TABLE.lock(&node);
    let id = table.iter().position(|slot| match slot {
        None => true,
        Some(p) => p.state == State::Exited,
    });
    let Some(id) = id else {
        panic!("proc: no free slot: all {NPROCS} slots are Running or Runnable")
    };
    let kstack = unsafe { KSTACKS.stacks.get().cast::<u8>().add(id * KSTACK_SZ) };
    let context = forkret_context(id, text_va, stack_va);
    let proc = Process { state: State::Runnable, context, kstack, exit_status: 0 };
    table[id] = Some(proc);
    id
}

/// Fabricate a process's entry frame and `Context` on its kstack.
///
/// The frame sits at the kstack's top (304 bytes); the `Context` (112)
/// sits directly below it.  The canary goes at the base.
#[cfg(target_os = "none")]
fn forkret_context(id: usize, text_va: usize, stack_va: usize) -> *mut Context {
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
        // single-process arc did).  The user stack pointer leaves 16
        // bytes of headroom below the page's top: an EL0 store to
        // [sp, #8] must stay inside the page.
        let user_sp = stack_va + mem::PAGE_SIZE_4K - 16;
        let frame = frame_base as *mut u8;
        frame.add(FRAME_ELR).cast::<u64>().write(text_va as u64);
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
        // after; a kstack that is switched to again holds only live
        // frames above the water line the canary guards.
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
    let node = LockNode::new();
    let first = {
        let table = TABLE.lock(&node);
        let id =
            table.iter().position(|slot| matches!(slot, Some(p) if p.state == State::Runnable));
        let Some(id) = id else { panic!("proc: run_all with no Runnable process") };
        let mut table = table;
        let Some(p) = table[id].as_mut() else { unreachable!() };
        p.state = State::Running;
        p as *mut Process
    };
    let starter = starter_ctx_addr();
    unsafe {
        tpidr_set(first);
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
/// is one.  Called only from a trap handler with the current process
/// on CPU (TPIDR non-null).
///
/// Returns `true` — and never returns at all — when it switched: the
/// caller's context is suspended on the kstack and will resume inside
/// this same handler's frame (the vector tail completes the return to
/// EL0 from there).  Returns `false` when nothing was Runnable.
///
/// The bracketing: the caller is in interrupt context (depth 1).  The
/// switched-to process must run *outside* interrupt context — without
/// the exit, depth would stay 1 across the switch and every `println`
/// while it runs trips the `in_interrupt` assert — so `exit_interrupt`
/// before the `swtch`, and on resume `enter_interrupt`: we are back
/// inside the suspended handler, whose `trap_unsafe` epilogue exits it.
#[cfg(target_os = "none")]
fn resched() -> bool {
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

        // Demote the current process so a preempted/yielding process is
        // selectable again.  The condition keeps the exit path from
        // resurrecting a process it just marked Exited — and the
        // `demoted` flag keeps the re-mark below from doing the same.
        let mut demoted = false;
        if let Some(p) = table[current_id].as_mut()
            && p.state == State::Running
        {
            p.state = State::Runnable;
            demoted = true;
        }

        // Scan from the cursor for a Runnable other than the current,
        // wrapping.
        let mut scan = CURSOR.load(Ordering::Relaxed);
        let mut next_id: Option<usize> = None;
        for _ in 0..NPROCS {
            scan = (scan + 1) % NPROCS;
            if let Some(p) = &table[scan]
                && p.state == State::Runnable
                && scan != current_id
            {
                next_id = Some(scan);
                break;
            }
        }

        match next_id {
            None => {
                // No next: the demotion was premature and the current
                // process is still on CPU.  Put the state back to what
                // it was — but only if we changed it; re-marking an
                // Exited process Running would resurrect it.
                if demoted && let Some(p) = table[current_id].as_mut() {
                    p.state = State::Running;
                }
                (current, None)
            }
            Some(nid) => {
                let Some(p) = table[nid].as_mut() else { unreachable!() };
                p.state = State::Running;
                CURSOR.store(nid, Ordering::Relaxed);
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
    port::irq::exit_interrupt();
    unsafe { swtch(&mut (*cur).context, (*next).context, SPSR_EL1H | DAIF_MASKED) };

    // Resumed inside the suspended handler.
    port::irq::enter_interrupt();
    unsafe { tpidr_set(cur) };
    true
}

// Host (unit-test) builds have no table, no kstacks, and no switch;
// the constants above are what they exercise.
#[cfg(not(target_os = "none"))]
pub fn spawn(_text: &[u8], _text_va: usize, _stack_va: usize) -> ProcessId {
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
