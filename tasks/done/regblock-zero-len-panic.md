---
status: done
---

# A device-tree reg with no length aborts the kernel on first MMIO access

`io::read_reg`, `write_reg` and `write_or_reg`
(`aarch64/src/io.rs:22,31,41`) resolve an offset with
`.expect("offset outside bounds")`. That is reachable on firmware-supplied
input, not merely on programmer error:

- `property_reg_iter` explicitly permits `size_cells == 0`, which means
  the `reg` property carries no length — the code comments this case
  (`port/src/fdt.rs:217`).
- `impl From<RegBlock> for PhysRange` turns the absent length into zero
  via `len.unwrap_or(0)` (`port/src/mem.rs:199`), producing a zero-size
  range. The `RegBlock` → `VirtRange` conversion (`port/src/mem.rs:49`)
  does the same `unwrap_or(0)` one door over, so both device-range paths
  arm the landmine.
- `VirtRange::offset_addr` then returns `None` for *every* offset, so
  every register access on that device aborts.
- The workspace is `panic = "abort"` in both profiles (`Cargo.toml:6,10`),
  so this is a dead kernel, and the abort surfaces far from the firmware
  data that caused it.

The GIC branch raises the stakes: `aarch64/src/gic.rs` accesses registers
at INTID-derived offsets from `try_ack_interrupt`/`end_interrupt`, i.e.
this would become an abort in interrupt context.

Fix direction (pick one, and say which at the probe site):
- Reject at probe: make the `RegBlock` → range conversion fallible for
  device registers, so a `reg` with no length fails the driver's `init`
  with a named error instead of arming a landmine. An infallible `From`
  may simply be the wrong trait for firmware-supplied extents.
- Or make the MMIO helpers return `Result` and have drivers handle it —
  larger blast radius, touches every accessor.

Either way the failure should name the device and the offset, and should
be attributable to the DT node that produced it.

Done when: a device-tree node whose `reg` has no length produces a
diagnosable failure at probe or a handled error at access, not an abort;
the chosen policy is stated where the conversion happens.

Origin: plan `tasks/plans/range-by-value.md`, Failure policy (microkernel
and hardware-truth lenses — "the device tree is a claim, not the truth").

## Status: done

Landed in b5e99e4. The reject-at-probe option was chosen: new
`PhysRange::from_regblock` (port/src/mem.rs:169) is the fallible path for
firmware-supplied device registers — `None` when the reg carries no length
(`size_cells == 0`) — and every device probe goes through it: the shared
probe helper (aarch64/src/deviceutil.rs:24) turns the `None` into the named
error `"device reg has no length (size_cells == 0)"`, and the GIC maps both
of its ranges through it directly (gic.rs:380, :388). A no-length reg now
fails the driver's init with a diagnosable message instead of arming an
abort. The infallible `From<&RegBlock>` is kept only for length-guaranteed
extents (memory nodes), and the policy is stated on both conversions in
port/src/mem.rs, as the "done when" asked.
