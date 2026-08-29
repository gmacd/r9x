---
status: open
---

# Delete mark_range's check_end control flag

`BitmapPageAlloc::mark_range` takes `check_end: bool`
(`port/src/bitmapalloc.rs:216`), a control flag that makes the function do
two different things. It has exactly one `false` caller: `:123`, marking
everything past `self.end` as allocated, immediately after `self.end` is
assigned at `:119`. Both public wrappers — `mark_allocated` (`:88`) and
`mark_free` (`:94`) — always pass `true`.

The flag exists only to disable a bounds check that the public API always
wants. That is a special case guarded rather than eliminated.

Fix direction: hoist `if range.end > self.end { return Err(OutOfBounds) }`
into `mark_allocated` and `mark_free`, delete the parameter, and let
`mark_range` become one unconditional loop. The one `false` call site at
`:123` then calls the unchecked `mark_range` directly, which is what it
actually means.

Done when: `mark_range` takes no `check_end`; the bound check lives in the
two public wrappers; the bitmapalloc tests still pass and gates are clean
on all three architectures.

Origin: item 3 of the `range-by-value` plan (kernel-taste lens — boolean
parameters that make a function do two things).
