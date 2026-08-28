# Lens — kernel-taste

**The question this lens asks:** would a maintainer of a long-lived kernel
accept this, and what does every future reader pay for it?

## Sources

- **The Linux kernel tree** (`/Volumes/Code/repos/linux`) — the living style
  witness. Before claiming "Linux does X", check how mainline shapes a
  comparable driver.
- **`Documentation/process/coding-style.rst`** — functions do one thing;
  more than three levels of indentation means the function needs redesigning,
  not reformatting; naming discipline.
- **The "good taste" linked-list example** (widely circulated from a 2016
  conference talk) — list deletion where an indirect pointer (`**pp`)
  eliminates the head-versus-interior special case. The generalisable rule: an
  edge-case branch is a data-representation smell.
- **The data-structures principle**, as stated on the git mailing list in
  2006: worrying about data structures and their relationships beats worrying
  about the code.
- **The "midlayer mistake"** (LKML position, written up in LWN) — layers that
  only forward are a tax; abstraction is earned by the second user.
- **"We don't break userspace"** — the Linux stability rule for externally
  visible contracts, absolute in its home tree.
- **LKML review culture, three decades and public** — performance claims
  require numbers; hot-path cost is measured, not asserted.

## Review rules

**Good taste is eliminating special cases.**
- Every `if` guarding an edge case is a question: could better initial
  conditions, a sentinel, or a different representation make the edge case
  take the normal path? Flag special-case branches that a taste-level
  restructure would delete.
- Flag boolean parameters that make a function do two things; the caller knows
  which one it wants — make it two functions, or fix the data.

**Data structures first.**
- Review the types in the diff before the logic. If the code is contorted,
  name the type change that would make it boring. Boring code is the goal.
- Flag state that lives in two places and must be kept in sync by hand.

**No midlayers, no speculative abstraction.**
- Flag layers that only forward calls downward. An abstraction that does not
  decide anything is a tax every reader pays forever.
- Flag helpers used once that hide four lines behind a name less informative
  than the four lines.
- Flag generality nothing uses: trait parameters with one instantiation, hooks
  with one registrant, "pluggable" designs with one plug. Kernel code earns
  abstraction after the second concrete user exists, not before.

**Function shape.** Functions do one thing. Past three levels of indentation
the function needs redesigning, not reformatting. Long functions are
acceptable only as flat, sequential, case-free narratives.

**Locking and concurrency.**
- Critical sections minimal and obvious; flag work under a lock that does not
  need the lock, especially allocation or I/O.
- Lock ordering must be documented wherever two locks can be held; flag any
  new lock whose relationship to existing locks is unstated.
- Flag interrupt-context paths that can sleep, allocate unboundedly, or take
  locks that are also taken without masking.

**Interface stability.** This kernel is pre-users, so apply the graded form of
the no-breakage rule: flag changes to externally visible interfaces (syscall
shapes, 9P behaviour, boot protocol expectations) made casually or without
being called out as breaking. Internal churn is fine; silent contract change
is not.

**Performance claims need numbers.** Flag any optimisation justified by
assertion. Conversely, flag work added carelessly to hot paths (per-interrupt,
per-syscall, per-packet) — per-invocation cost there is a real regression even
when each unit is small.

## Not this lens's business

Rust-versus-C idiom differences (this kernel chose Rust; review it as Rust);
style the formatter owns; microkernel-versus-monolith philosophy (other lenses
own architecture); anything that cannot be tied to a concrete cost for a
reader, a maintainer, or the CPU.

## Voice note

This lens takes the judgment and drops the flame. A finding is stated as cost
("every reader pays for this layer"), never as insult. Pastiche adds nothing
checkable.
