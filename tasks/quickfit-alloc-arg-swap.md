---
id: 106
status: open
wave: 0
---

# Task 106: alloc_tail passes (size, align) to a function taking (align, size)

## Status: open — wave 0

## Problem

`BumpAlloc::try_alloc` is declared `fn try_alloc(&self, align: usize,
size: usize)` (`port/src/allocator.rs:86`).  `QuickFit::alloc_tail`
calls it `self.tail.try_alloc(size, align)` (`:266`).  The arguments are
swapped.

Quick-list sizes hide it, because `adjust` (`:219-228`) makes
`align == size` for those.  Above `MAX_QUICK_SIZE` it does not:
`Vec::with_capacity(65536)` becomes `malloc(Layout { size: 65536,
align: 1 })`, then `alloc_tail(65536, 1)`, then `try_alloc(align =
65536, size = 1)` — the bump cursor advances **one byte** and a pointer
to a one-byte block is returned for a 64 KiB request.  The next
allocation is handed overlapping memory.

A non-power-of-two size above the quick-list bound instead trips
`align_offset`'s power-of-two assertion at `:92` and panics.

`Allocator::allocate` (`:111`) has the identical swap:
`self.try_alloc(layout.size(), layout.align())`, then returns
`NonNull::slice_from_raw_parts(ptr, align)`.  Not currently reached — the
kernels use `BumpAlloc` only through `QuickFit` — but it is the obvious
next caller.

## Design

- Fix both call sites.  Consider making the signature take a `Layout`
  instead of two same-typed `usize`s, so the class cannot recur: this bug
  is only possible because the two parameters are indistinguishable to
  the type system.

## Tests

- Host: allocate above `MAX_QUICK_SIZE` and assert the returned block is
  at least `layout.size()` and correctly aligned; assert two consecutive
  large allocations do not overlap.  Both fail today.
- Host: a non-power-of-two large size no longer panics.

## Done when

- Both call sites pass arguments in the declared order, or the signature
  no longer admits the confusion.
- The overlap and alignment tests exist and pass.
- Full `cargo xtask ci` green.

Origin: architecture and correctness review of `f76d96a`, 2026-08-28.
