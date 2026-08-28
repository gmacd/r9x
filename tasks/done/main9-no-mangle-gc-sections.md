---
status: done
commit: eea651b
---

# main9's unconditional no_mangle survives hosted links only by gc-sections

`aarch64/src/main.rs:87` marks `main9` `#[unsafe(no_mangle)]`
unconditionally, while `x86_64/src/main.rs:25` uses the guarded
`#[cfg_attr(not(test), unsafe(no_mangle))]`. Since e022999 the aarch64
bin target is built and run as a hosted unittest (`--tests` on
`aarch64-unknown-linux-gnu`); it links today only because rustc passes
`--gc-sections` and ld discards `main9`'s section — and with it the
references to link-script symbols (`interruptstackbase`,
`interruptstacksz`, `main.rs:73-76`; transitively `kmem.rs:5-15`'s
`etext_pa` et al. and `pre_mmu/vminit.rs:107-108`) — before relocation
processing. Verified green at e022999 (run 32367405192: the bin unittest
links, runs, 0 tests).

The fragility: anything that defeats section GC — `-C link-dead-code`
(e.g. for coverage), `-Wl,--no-gc-sections`, or a future `#[test]` in the
bin that transitively keeps `main9` alive — turns this into undefined-
reference link errors on the hosted target. The x86_64 package already
has the fix as its idiom.

Fix direction: change `aarch64/src/main.rs:87` (and riscv64's equivalent
if it shares the pattern) to `#[cfg_attr(not(test), unsafe(no_mangle))]`,
matching `x86_64/src/main.rs:25`.

Done when: all three arch `main9` declarations use the same cfg_attr
pattern and gates stay clean.

Origin: code review of e022999 (2026-08-20) — consistency nit surfaced by
the (refuted) link-failure candidate; the link works today, this makes it
stop depending on gc-sections.

## Status: done (eea651b)

aarch64 and riscv64 main9 now use x86_64's
`#[cfg_attr(not(test), unsafe(no_mangle))]` guard, so the hosted bin
unittest no longer depends on gc-sections discarding the entry point.
All three arches dist-build and 4 of 4 integration images pass.
