---
status: open
---

# PhysRange::with_end is public API with no production caller

Paths refreshed 2026-08-27: the type moved to `r9x-core` (commit
e4a403a).

`PhysRange::with_end` (`core/src/addr.rs:162-164`) is `pub`, but every
one of its six call sites is inside a test:

- `port/src/bitmapalloc.rs:359, 362, 366, 377, 381` — inside `mod tests`
- `aarch64/src/vm.rs:889` — inside the `#[cfg(test)]` block

It is also `PhysRange::new` modulo wrapping the two `u64`s in `PhysAddr`, so
`port::mem` currently offers two ways to construct a range from bounds — one
of which exists only for test ergonomics. Because `port` is a lib crate, a
dead or test-only `pub` item never triggers a warning, which is why the
`range-by-value-sweep` had to hand-audit the module in the first place.

The sweep deliberately applied its "caller-free" criterion literally and left
this alone: `with_end` does have callers, they are just all tests. Whether
production-dead is the criterion that should govern a `pub` surface is a
judgement call, not a defect, so it was filed rather than acted on.

Fix direction (revised 2026-08-27 — **the first option no longer
works**):
- ~~`#[cfg(test)] pub fn with_end(...)`~~ — **broken by the crate
  move**: `cfg(test)` is set only when compiling the crate under test,
  and `r9x-core` builds as a plain dependency when `port`'s and
  `aarch64`'s tests build, so a `#[cfg(test)]` `with_end` would be
  invisible to every one of its actual callers. (It was already
  cross-crate-broken for the vm.rs caller when the fn lived in `port`.)
- **Delete it** and have the tests call
  `PhysRange::new(PhysAddr::new(a), PhysAddr::new(b))` — six mechanical
  test edits; now the honest cheapest option, and the recommended one.
- Or a `test-helpers` cargo feature — more machinery than six call
  sites justify.
- Or keep it public and accept a constructor with no production user —
  but then say why in a doc comment. (Note `with_len`/`with_pa_len`
  have production callers, e.g. `from_regblock`; the duplication claim
  is specific to `with_end` vs `new`.)

Done when: `port::mem`'s public surface contains no constructor that only
tests call, or the exception is documented; tests still pass and gates are
clean on all three architectures.

Origin: panel review of the `range-by-value-sweep` working diff
(whole-system lens — "A Plea for Lean Software": a module's `pub`
surface is its specification).
