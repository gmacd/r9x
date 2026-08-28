# Lens — whole-system-design

**The question this lens asks:** after this change, can one person still hold
the system in their head?

## Sources

- **"Design Principles Behind Smalltalk" (*Byte*, August 1981)** — the closest
  thing to a ready-made review rubric in the literature. *Personal Mastery*:
  "if a system is to serve the creative spirit, it must be entirely
  comprehensible to a single individual". *Good Design*: "a system should be
  built with a minimum set of unchangeable parts". *Uniform Metaphor*: "a
  language should be designed around a powerful metaphor that can be uniformly
  applied in all areas". *Factoring*: "each independent component in a system
  would appear in only one place". And the jab that lands directly on kernel
  work: "an operating system is a collection of things that don't fit into a
  language. There shouldn't be one."
- **Smalltalk-76/78 and the Lively Kernel** — whole systems small enough to
  hold.
- **"The Early History of Smalltalk" (HOPL II, 1993)** — late binding,
  messaging as the central idea, and biology as the growth metaphor: scale by
  uniform composition, not accretion.
- **"The Computer Revolution Hasn't Happened Yet" (OOPSLA 1997) and the VPRI
  STEPS reports (2007–2012)** — a full personal-computing system in about 20K
  lines as an existence proof of comprehensibility; "simple things should be
  simple, complex things should be possible".
- **"A Plea for Lean Software" (*IEEE Computer*, February 1995)** — "a primary
  cause of complexity is that software vendors uncritically adopt almost any
  feature that users want"; fat software as a failure of discipline.
- **Project Oberon (1992; 2013 edition)** — an entire OS, compiler and
  toolchain built by two people; modules with explicit narrow interfaces. The
  system is the argument.
- **"Program Development by Stepwise Refinement" (CACM, 1971)** — design as a
  sequence of committed decisions, not deferred knobs.

## Review rules

Unlike the line-level lenses, this one reviews the *change against the whole*:
read the diff, then read the subsystem it touches, and judge what the change
does to the system's shape.

**Personal mastery.** The governing question for every finding: after this
change, can one person still hold this subsystem in their head? Count the
concepts the diff adds (new types, traits, states, invariants, special cases)
against what it removes. A diff that adds three concepts to save ten lines is
a net loss; say so. Flag documentation-by-tribal-knowledge: an invariant that
lives only in the author's head spends the comprehensibility budget silently.

**Uniform metaphor.** A system should be built around a small set of ideas
applied uniformly. For Smalltalk it was objects and messages; for Plan 9, and
therefore for this kernel, it is files and servers.
- Flag mechanisms that introduce a *parallel* concept where extending an
  existing one would serve: a new registry when the namespace could hold it, a
  bespoke control API where a file-shaped interface fits, a second event
  mechanism beside an existing one.
- The test is not "is this mechanism good?" but "does the system now have two
  ways to say one thing?"

**Simple things simple, complex things possible.** Flag designs where the
common case pays for the rare case: a caller doing the ordinary thing should
not thread through machinery that exists for the exotic thing. Complexity may
exist, but it must be pay-as-you-go.

**Late binding of decisions.** Defer commitments to the point of use.
- Flag policy hardcoded into mechanism: numbers, orderings and choices baked
  into a driver or subsystem that a caller will foreseeably need to vary.
  Mechanism in the kernel, policy at the edge.
- State the tension honestly: in kernel code, static dispatch and compile-time
  binding are often correct, and the kernel-taste lens will argue exactly
  that. When flagging early binding, name what concretely will need to vary
  and when. No speculative flexibility — that is the lean-software sin in the
  other direction.

**Lean software.** Every feature is judged against the whole system's weight,
not its local usefulness.
- Flag additions whose benefit is narrower than their footprint.
- Flag configuration knobs added instead of decisions made. A tunable is a
  design decision the author refused to make, exported as permanent interface.
- Flag modules whose boundary cannot be stated in one sentence. In Rust terms:
  a module's `pub` surface is its specification — flag `pub` items that leak
  internals or exist "for tests".

**Growth direction.** Systems should scale by uniform composition, not
accretion. When a diff extends a subsystem, ask whether it grows the design or
barnacles it. Three special cases accreted onto a clean mechanism signal the
mechanism's abstraction is due for rethinking — flag the trend, not just the
instance.

## Not this lens's business

Line-level style, naming, loop shape (other lenses own those); necessary
domain complexity from hardware (the hardware-truth lens owns it); Rust's own
ceremony. This lens operates at subsystem altitude — every finding should be
about the system's shape, not a line's.
