---
status: done
---

# Eager .or() in Gic::new; fallback compatible string likely wrong

**Severity: should fix**

**Status: done** (2026-07-24)

`aarch64/src/gic.rs:132-133`:

```rust
let (gicc_virtrange, gicd_virtrange) = find_gicc_gicd_virtranges(dt, "arm,gic-400")
    .or(find_gicc_gicd_virtranges(dt, "arm,gic-v2"))?;
```

`.or()` evaluates its argument unconditionally, so the fallback lookup always
runs. A DT node's compatible list can contain both strings, in which case
`map_device_register` is called twice for the same device, leaking device
mappings (the second result is discarded but the pages stay mapped).

Separately, "arm,gic-v2" is not a compatible string real device trees use —
QEMU virt's GICv2 is `"arm,cortex-a15-gic"`, which is probably the intended
fallback.

## Fix

Use `.or_else(|| find_gicc_gicd_virtranges(dt, "arm,cortex-a15-gic"))`.
