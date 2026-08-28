---
status: done
---

# Asserted integration image for the page and heap allocators

Landed 2026-08-22, no prior task file.

main9 ran two pieces of throwaway code after the boot prints: a loop that
allocated three kernel pages and a lone `Box::new`. They were allocation
smoke-tests with no real assertion — a failure just printed and broke out —
and they had no business in the kernel binary, whose stated job is "the boot
sequence, and nothing else."

## Status: done

Landed in a684d81. The smoke code moved to a new whole-kernel integration
image, `allocate` (aarch64/tests/allocate.rs, `[[test]]` entry in
aarch64/Cargo.toml), shaped like `pagetables` and `user_process`: bring up
just the page allocator and the console, then drive the real allocators. The
smoke code became assertions: each mapped kernel page is written through and
read back (a bad mapping faults rather than returns), the three allocations
are distinct and the page allocator's used-bytes grows, and the QuickFit heap
round-trips a small box, a 4 KiB box, and an allocation made after a free.

`main.rs` dropped the block and the four imports it needed; `RootPageTableType`
and `pagealloc` stay, used by the user-process switch and the memory
printout. Integration count went to 10/10.
