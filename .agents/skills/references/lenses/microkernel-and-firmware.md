# Lens — microkernel-and-firmware

**The question this lens asks:** does this belong in the kernel at all, is its
latency bounded, and could a reviewer with the manual open audit it?

## Sources

- **QNX Neutrino System Architecture documentation** — the doctrine: a
  microkernel that does message passing (send/receive/reply), scheduling and
  little else; drivers, filesystems and network stacks as ordinary userspace
  processes (resource managers); priority inheritance to bound inversion;
  determinism as the product. The recurring theme of that tradition:
  predictable latency beats throughput, everything is a process, and system
  services are restartable.
- **coreboot and LinuxBoot/NERF** ("Replace Your Exploit-Ridden Firmware with
  Linux", 2017) — firmware should be minimal, auditable and owner-replaceable;
  every opaque blob is unaudited attack surface; boot code should be readable
  against the manual.
- **"What Have We Lost?"** — what older systems did structurally better; a
  direct statement of the standards this repo was founded on.
- **Plan 9 ports and Harvey OS** — the Plan 9 model carried onto modern
  hardware.
- **This repository's own history** — primary source, and weighted above any
  talk. Before judging a pattern, check `git log` and existing code for the
  convention already established here.
- **Rigor in systems Rust**, as practised in illumos and in Oxide's firmware
  and hypervisor work: unsafe code carries proof obligations, panics are not
  error handling in a kernel, illegal states are made unrepresentable.

## Review rules

**Does this belong in the kernel at all?** Every addition to kernel code
carries a burden of proof. Flag new kernel-resident functionality that could
be a server, a userspace service, or a 9P file server, unless the diff or the
architecture justifies residence. "It was easier here" is not justification;
it is how microkernels die.

**Determinism.**
- Flag unbounded loops, retries without limits, or input-proportional work in
  interrupt handlers and other latency-critical paths.
- Flag allocation in interrupt context or while holding a spinlock.
- Flag priority and ordering hazards: work deferred with no bound on when it
  runs, or handlers that can starve one another.

**Message-passing discipline.** Prefer explicit ownership transfer over shared
mutable state. Flag new shared mutable state between contexts (cores, IRQ
versus thread) where handing the data through a queue or channel would make
ownership obvious. Every piece of mutable state needs an identifiable owner
and a stated synchronisation story.

**Boot and bringup auditability.**
- Bringup code must read as a checklist against the hardware manual. Flag
  magic register sequences with no citation — the hardware-truth lens owns the
  numeric side; this lens's concern is auditability: could a reviewer with the
  TRM open verify this sequence line by line?
- Flag initialisation-order dependencies that are load-bearing but unstated.
- Early-boot paths should be as short and dumb as possible; cleverness before
  memory, consoles and interrupts exist is unrecoverable cleverness.

**Rust-in-kernel rigor.**
- Every `unsafe` block: minimal in extent, with a `// SAFETY:` comment stating
  the invariant and why it holds *here*. An unsafe block without one is a
  blocker, matching this repo's established practice.
- Panic-freedom in kernel paths: flag `unwrap`/`expect`/indexing/arithmetic
  that can panic on reachable input, anywhere that is not demonstrably
  init-only or genuinely impossible — and where impossible, the code should
  say why.
- Make illegal states unrepresentable: flag flag-plus-data pairs that should
  be an enum, and integers that should be newtypes where confusion is
  plausible (PA versus VA, byte counts versus frame counts).
- Prefer const/static construction over runtime initialisation; flag runtime
  initialisation of what could be built at compile time.

**Repo consistency.** This repo has conventions — per-arch layout, port
abstractions, the xtask workflow. Flag diffs that introduce a second way to do
something the repo already does one way, even when the new way is defensible
in isolation.

## Not this lens's business

Monolithic-kernel pragmatism as such (the architecture is this project's own
choice; flag *unjustified* kernel residence, not residence); style and naming
(other lenses own those); hardware-model assumptions (the hardware-truth lens
owns those).
