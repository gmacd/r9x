# Lens — hardware-truth

**The question this lens asks:** does this code address the machine that
exists, or the machine the textbook described?

## Sources

- ***Computer Structures: Readings and Examples* (1971; 1982 edition)** — the
  PMS view: a computer is processors, memories, switches and links, a
  structure rather than a CPU with accessories. Review question it yields:
  which structure is this code really talking to?
- **The component-economy maxim** (same lineage): "the cheapest, fastest and
  most reliable components are those that aren't there."
- **The law of computer classes** — hardware classes shift roughly per
  decade; assumptions baked to one class rot with it.
- **"It's Time for Operating Systems to Rediscover Hardware" (OSDI 2021
  keynote)** — the hardware-realism charter: a phone SoC runs dozens of cores
  and firmware stacks the kernel neither sees nor controls, while the kernel's
  machine model is decades stale.
- **"The Multikernel: A New OS Architecture for Scalable Multicore Systems"
  (SOSP 2009) / Barrelfish** — treat the machine as a distributed system: no
  shared kernel state across cores, explicit messages, replicas instead of
  shared memory; coherence traffic is not free.
- **Sockeye and Enzian (ETH Zurich)** — machine-readable formal descriptions
  of hardware topology and address spaces instead of folklore constants; the
  Enzian papers document how much "known" hardware behaviour is actually
  unspecified.
- **Device-tree critiques** in the same line of work — device trees are
  informal, unverified claims; kernels that trust them uncritically inherit
  their errors.
- **The PDP-11/VAX machine model** — the model most kernels still silently
  assume, and the thing the hardware-realism work shows to be false.
- **Witness tree**: `/Volumes/Code/repos/linux` — compare mainline's driver
  for the same peripheral (register sequences, barriers) before trusting or
  condemning a bringup sequence.

## Review rules

**Model vs. machine.** For every diff, name the hardware assumptions it makes:
coherent memory, a single interrupt-controller view, homogeneous cores, sole
ownership of a peripheral, one clock domain. Flag any assumption that is (a)
false on a stated target, or (b) load-bearing but undocumented. On Raspberry
Pi specifically, the VideoCore firmware owns parts of the machine — flag code
that assumes the kernel is alone.

**Memory ordering and MMIO.**
- Every barrier, `volatile` access and atomic ordering must be justified
  against the ARM or RISC-V memory model. Flag missing barriers between device
  MMIO writes and the action that depends on them (doorbell after descriptor
  writes, enable after config), and around DMA buffer handoff.
- Flag cargo-cult ordering equally: `SeqCst` sprinkled by default, or barriers
  copied without a stated reason. An unexplained barrier is a lie waiting to
  be believed — the finding is "state which reordering this prevents".
- Flag device-register access through non-volatile references, and any
  assumption that MMIO has memory-like semantics (read-back equals written,
  idempotent reads).

**Magic numbers cite the manual.** Hardware topology should be checkable
description, not folklore: every register offset, field mask and magic
constant needs a citation — document name and section (BCM2711 ARM
Peripherals §x.y, GICv2 Architecture Specification §x.y, and so on). Flag
constants inherited from "some other kernel does this". Flag values correct
for one board silently generalised to all.

**The device tree is a claim, not the truth.** Flag code that consumes device
tree (or ACPI, or any firmware table) uncritically: unvalidated ranges,
trusted counts used to size things, missing-node paths that panic instead of
degrading. Firmware-provided data is input, and input gets validated.

**The machine is a network.**
- Flag cross-core shared mutable state where coherence traffic is assumed
  free: hot shared cachelines, spinlocks on per-event paths, false sharing in
  adjacent fields.
- Flag code that treats "the other core sees it" as instantaneous or ordered
  without the interconnect actually promising that.
- IPIs, cache maintenance and TLB shootdowns are messages with real cost —
  flag per-operation use of what should be batched.

**Component economy.**
- Flag hardware-abstraction machinery serving exactly one device while
  charging every reader its generality cost. The second device pays for the
  abstraction, not the first.
- Flag code that could be deleted because the hardware, a boot stage, or an
  existing layer already guarantees the property being re-established.

**Interrupt and exception realism.** Flag handlers that assume more context
than the architecture delivers (banked state, stack validity, reentrancy),
EOI/acknowledge sequences that do not match the controller spec's state
machine, and races between mask/unmask and delivery that the spec explicitly
warns about.

## Not this lens's business

Software-side style, decomposition or abstraction taste (other lenses own
those); portability sins to hardware this project does not target; theoretical
ordering issues on ISAs whose model actually forbids the reordering — check
before claiming, because a false barrier demand is itself a finding-quality
failure.
