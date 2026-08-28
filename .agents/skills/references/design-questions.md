# Design interrogation checklist

Design-time questions derived from the six lenses. Every question gets an
answer in the plan or an explicit "N/A because…". Each section names the lens
it comes from; the lens files are in `lenses/`.

## Composition — what already exists? (clarity-and-composition)
- What in this repo, in `core`, or in an accepted dependency already does part
  of this? Grep before designing; the best plan is a composition.
- What does Plan 9 do for this problem? (`/Volumes/Code/repos/plan9`) What
  does Linux do, and which parts of that are essential rather than accreted?
  (`/Volumes/Code/repos/linux`)
- Will this design's output become someone's input? Does it return data, or
  format and act too early?

## Data structures — what makes the code boring? (kernel-taste, simplicity-and-interfaces)
- What is the central data structure? Does it eliminate the special cases, or
  will every function need edge-case branches?
- Where does state live, who owns it, and is any fact stored twice?
- What is the hot path, and what does this design cost per invocation on it?
  For any performance claim: what is the measurement plan?

## Interface shape — how is it used? (simplicity-and-interfaces)
- Could this be file-server-shaped (narrow read/write/ctl) instead of a wide
  API? If not, why not — recorded, not assumed?
- How many concrete users exist on day one? One user means no abstraction
  layer yet: design the concrete thing.
- What is the simplest thing that could work, and what does the plan add
  beyond it? Justify each addition or cut it.

## Kernel residence and determinism (microkernel-and-firmware)
- Does this belong in the kernel? What would the userspace or 9P-server
  version look like, and why is it rejected?
- What runs in interrupt context, and is its work bounded? Where may it
  allocate? Where may it panic — and is every reachable panic converted to a
  `Result` or a checked invariant?
- What is the init and bringup order, what does it depend on, and what makes
  the order load-bearing-but-stated rather than silent?
- Which `unsafe` will this need, and what invariant will each `// SAFETY:`
  comment state?

## Hardware truth (hardware-truth)
- **Required section.** Name every hardware assumption per target
  (aarch64/Pi, QEMU virt, x86-64, riscv64): coherency, interrupt topology,
  sole ownership of peripherals, reset state, firmware co-tenancy. Which are
  false on some target, and what happens there?
- Every register and constant: which document and section establishes it?
- What memory ordering does the design require, and which barriers with what
  justification?
- What firmware-provided data (device tree, tables) does it consume, and how
  is it validated?
- Component-economy test: which component of this design could not exist?
  What does the hardware, a boot stage, or an existing layer already
  guarantee?

## Whole system (whole-system-design)
- How many new concepts (types, traits, states, invariants) does this add, and
  what does it remove? Net concept count is a budget line, not a vibe.
- Does it extend an existing metaphor or introduce a parallel one? Will the
  system have two ways to say one thing afterwards?
- Which decisions is the plan making now, and which is it exporting as knobs?
  Every tunable must name the caller who will actually vary it, or become a
  decision.
- Mechanism and policy: what will foreseeably need to vary, and is that late
  bound while everything else stays static? Record the tension with the
  data-structures section honestly; it is the panel's standing disagreement.
- After this lands, can one person still hold the subsystem in their head?

## Amiga shape — real-time interactive graphics (amiga-inspiration.md)
- **Vertical blank**: which interrupt is the heartbeat of the graphics system
  on this target, and what is its period? (Pi 4: VideoCore VI HBLANK/VBLANK,
  ~16.7 ms at 60 Hz. QEMU: VirtIO GPU config change.)
- **Interrupt context budget**: what does the IRQ handler do, and is it within
  the three-thing budget (lookup, enqueue, wake)? What would it cost to do
  anything more?
- **Per-IRQ message pool**: how many messages are pre-allocated per IRQ, and
  what happens when the pool is exhausted? (The Amiga's answer: the interrupt
  is lost — acceptable for a display refresh, not for input.)
- **Display server ownership**: which user-space process owns the GPU MMIO,
  and how does the kernel hand it over? (The `map_mmio` verb, stage 5.)
- **Boot to graphics**: what is the boot sequence, and where does the text
  console hand off to the graphical environment? (The console server, stage 5,
  is the first step; the display server is a later stage.)
