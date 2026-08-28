# Lens — clarity-and-composition

**The question this lens asks:** can the next reader follow this, and does it
already exist as a composition of what we have?

## Sources

- **"The Elements of Programming Style" (1974/1978)** — the canon: "write
  clearly — don't be too clever"; "say what you mean, simply and directly";
  and the debugging maxim: debugging is twice as hard as writing the code in
  the first place, so code written as cleverly as possible is by definition
  too clever to debug.
- **"The Practice of Programming" (1999)** — comments say why, not what; idiom
  as the unit of readability; symmetry, because parallel code should look
  parallel.
- **"The Unix Programming Environment" (1984)** — composition as the working
  method, not an ideal.
- **"The Development of the C Language" (HOPL II, 1993)** — economy of means;
  declarations mirror use; a small core carried further than feature
  accretion.
- **"The C Programming Language"** — the prose itself is the style guide:
  every example minimal, every word earning its place.
- **The Unix papers (CACM, 1974)** — restraint as design method, and famously
  understated changelogs.
- **The Unix philosophy**, in its classic formulation: write programs that do
  one thing and do it well; write programs to work together; write programs to
  handle text streams, because that is a universal interface.
- **The 1986 "Literate Programming" column (CACM)** — invited to critique a
  beautifully crafted literate word-count program, the reviewer answered with
  a six-stage shell pipeline and asked why a bespoke monolith had been built
  at all. The single best model of a code review in the literature: review the
  *decision to write the code*, not just the code.
- **"A Research Unix Reader" (1986)** — the history of the toolkit culture and
  the argument for pipes.

## Review rules

**The clarity test.** Flag any block that had to be mentally simulated more
than once to be believed. That difficulty is the finding; report where the
reading stalled. Prefer the obvious construction: flag expression-golf, nested
conditionals that a `match` or an early return would flatten, and boolean
logic that needs De Morgan applied in the reader's head.

**One thing well.** Applied to functions: flag a function that does N things
in sequence — it is N functions and a caller. The test is whether it can be
named honestly without "and". Expect output to become input: flag functions
that render, format or log deep inside logic instead of returning data the
caller composes.

**Review the decision to write the code.** Before accepting any nontrivial
loop or algorithm, ask what already exists.
- Flag hand-rolled loops that are `iter().filter().map().fold()` in disguise —
  but only where the combinator form is genuinely *clearer*; combinator soup
  is the same sin in the other direction. Clarity is the test, not style
  allegiance.
- Flag reimplementation of anything that already exists in `core`, in this
  repo's own crates, or in an already-accepted dependency. Grep first: if the
  codebase solves this pattern somewhere, the diff must use or improve that,
  not fork it.

**Economy of expression.**
- Every declaration, parameter and field earns its place. Flag unused
  generality: parameters always passed the same value, fields written but
  never read, lifetimes and generics more general than any caller needs.
- Declarations should mirror use; representation should make the common
  operation trivial. If callers keep destructuring or converting a type, the
  type is shaped wrong.

**Comments.** Do not comment what — comment why.
- Flag comments that restate the line below them; they will rot.
- Flag the *absence* of a comment on anything non-obvious: an invariant, a
  hardware quirk, an ordering requirement. Tricky code with no why is a
  should-fix.

**Symmetry and uniformity.** Parallel logic should look parallel. Flag two
arms of a match, or two similar functions, that do the same job with
gratuitously different structure — the reader will hunt for a semantic
difference that is not there. Error handling should follow one pattern per
subsystem, not ad-hoc per call site.

## Not this lens's business

Kernel-domain complexity that is irreducible (hardware really is like that —
the hardware-truth lens owns those); Rust verbosity the language forces;
naming and interface-shape issues (the simplicity lens owns those) unless the
name actively misleads.
