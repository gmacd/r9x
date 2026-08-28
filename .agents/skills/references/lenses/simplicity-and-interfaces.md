# Lens — simplicity-and-interfaces

**The question this lens asks:** is this the simplest thing that works, and is
its interface shaped like the rest of the system?

## Sources

- **Plan 9 itself** (`/Volumes/Code/repos/plan9`) — the system is the primary
  document. When a claim about "the Plan 9 way" matters, verify it in the tree
  rather than asserting it from memory.
- **"Notes on Programming in C" (1989)** — the rules of programming
  (measurement before optimisation; fancy algorithms are slow when n is small;
  fancy algorithms are buggier; data dominates), plus the naming doctrine:
  short names for close scope, no type encoding, consistency over
  expressiveness.
- **"Systems Software Research is Irrelevant" (2000)** — the cultural stance:
  build whole simple systems, resist accretion.
- **"Simplicity is Complicated" (dotGo 2015) and the Go proverbs** — "clear is
  better than clever"; "a little copying is better than a little dependency";
  "don't communicate by sharing memory, share memory by communicating".
- **"Reflections on Trusting Trust" (Turing lecture, 1984)** — trust and
  minimality of the trusted base; what gets to be load-bearing is a decision.
- **The brute-force maxims** (*Coders at Work*, 2009): "when in doubt, use
  brute force", and deletion as a productive day's work.
- **Plan 9 network papers** — "The Organization of Networks in Plan 9" and the
  IL protocol paper: network stacks as file servers, `/net` as the interface,
  narrow ctl/data file pairs instead of wide APIs.
- **"Upas — a Simpler Approach to Network Mail" (1985)** — small cooperating
  programs instead of one monolith.
- **Unix v6/v7 and the Plan 9 kernel sources** — the house style: small, flat,
  brute-force-honest functions. Concrete pattern for interfaces:
  `/Volumes/Code/repos/plan9/sys/src/9/ip` and `/sys/src/cmd/upas` — an
  interface is a small directory of files with read/write/ctl semantics.

## Review rules

**The rules of programming** (*Notes on Programming in C*):
1. No optimisation without measurement. Flag code whose stated or apparent
   justification is speed with no numbers behind it. Bottlenecks are in
   surprising places; tuning a guess is negative work.
2. Fancy algorithms are slow when n is small, and n is usually small. Flag
   clever data structures where a linear scan over a small fixed set would do.
3. Fancy algorithms are buggier. When two approaches work, the simpler one
   wins the review.
4. Data dominates. Review the data structures first; if they are right, the
   code is self-evident. A convoluted function frequently signals a wrong
   type, not a wrong loop.

**Abstraction discipline.**
- Flag any trait, generic parameter or abstraction layer with exactly one
  implementation and no concrete second one in sight. Interfaces are
  discovered from use, not designed in advance.
- "A little copying is better than a little dependency" — flag a new shared
  helper or module coupling introduced to deduplicate two small, incidentally
  similar pieces of code.
- "Clear is better than clever." If understanding a block took two reads, say
  so; that is a finding, not a style opinion.

**Brute force.**
- Flag cleverness that exists to avoid straightforward work.
- Deletion is a feature. If the diff could reach its goal by removing or
  unifying code instead of adding, propose that.

**Naming.**
- Short names for short-lived, close-scoped things; descriptive names only
  where scope is wide. Flag `index_of_current_interrupt_handler` where `i` is
  honest, and flag single letters with file-wide scope.
- No type information encoded in names. The declaration says the type; the
  name says the role.
- Names must match the codebase's existing vocabulary — check what this repo
  and Plan 9 call the same concept before accepting a new synonym.

**Interface shape.**
- Resources want to be file-server-shaped. When the diff adds a new API
  surface (a trait with many methods, a control interface, an ioctl-like
  escape hatch), ask whether a narrow, file-like read/write/ctl interface
  would compose better with what exists.
- Flag chatty interfaces: many small calls with shared implicit state where
  one well-shaped message would do.
- Protocol and layer boundaries must be crossable in one direction only. Flag
  layering violations and callbacks that let a lower layer reach up.

**Default no.** Flag anything added "because we'll need it later" —
speculative parameters, unused config, dead flexibility. The feature can be
added the day it is needed.

## Not this lens's business

Formatting the tools already enforce; idioms that are standard Rust even when
un-Plan-9-ish (error enums, `Result`); cleverness that is genuinely
load-bearing and documented.
