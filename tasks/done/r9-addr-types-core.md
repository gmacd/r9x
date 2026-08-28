---
status: done
---

# r9-addr-types-core: move address types to r9x-core (Tier 4 cleanup)

Move `PhysAddr`, `PhysRange`, and `VirtRange` from `port/src/mem.rs` to
`r9x-core`. These are pure data types with no kernel dependencies — they
belong in the neutral layer.

## Rationale

- All three are pure data: newtypes and small structs with arithmetic
  helpers. No locks, no channels, no kernel context.
- All three arches use them (152 uses across `port/` and the arch crates).
- `r9x-core` is the neutral layer between `r9x-abi` (wire format) and
  `port` (shared kernel infra). Address types are neutral.
- If user-space ever needs to express a physical address (DMA, framebuffer
  physical base), it's already reachable via `r9x-core`.

## Changes

- **`r9x-core` (`core/src/addr.rs` or `core/src/lib.rs`):**
  - `PhysAddr(u64)` — moved from `port/src/mem.rs`, with its existing
    methods (`new`, `round_up2`, `round_down2`, `Add<u64>`, `Step`).
  - `PhysRange` — moved from `port/src/mem.rs` (a pair of PhysAddrs with
    length/contains/overlap helpers).
  - `VirtRange` — moved from `port/src/mem.rs` (a pair of VirtAddrs /
    usizes with the same shape).
- **`port/src/mem.rs`:** remove the types, re-export from `r9x-core`
  (`pub use r9x_core::addr::{PhysAddr, PhysRange, VirtRange}`) so existing
  `use port::mem::PhysAddr` paths still work.
- **`port/Cargo.toml`:** verify `r9x-core` dependency is present.
- **Arch crates:** no changes needed (they use `port::mem::PhysAddr`
  which re-exports from `r9x-core`).

## Tests

- Host unit tests in `r9x-core`: arithmetic on `PhysAddr`, range
  operations (`PhysRange`, `VirtRange`).
- `cargo xtask test` green (existing tests that use these types via
  `port::mem` still pass through the re-export).

## Acceptance

- `cargo xtask ci` green (all arches, no warnings).
- `PhysAddr`, `PhysRange`, `VirtRange` are defined in `r9x-core`,
  re-exported from `port::mem`.
- No behavior change — a pure move.

## Not in scope

- Introducing a `VirtAddr` type — the kernel uses bare `usize` for
  virtual addresses and that's fine for now.
- Migrating existing `usize`-as-address uses to a typed address — a
  future refinement.
- Adding these types to `r9x_std` (user-space) — only if a user-space
  facility needs them.
