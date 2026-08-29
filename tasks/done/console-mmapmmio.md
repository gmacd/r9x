---
status: done
---

# console-mmapmmio

**From:** [stage 5 console-server design](../plans/microkernel-console-server.md)
— first user-space device MMIO ownership via `SYSMAPMMIO`.
**Depends on:** stage 3 (per-process Aspace) — done.

## Context

`Aspace::map_user_page` maps a pagealloc'd page into both TTBR0 (the process
sees it) and TTBR1 (the kernel can write initial contents). A device's MMIO
is different: it is a *fixed* physical range (the device's registers), it
must be mapped with `Device` memory attributes (not cacheable), and the
kernel does not need to reach it (the server owns it exclusively).

This task adds:
1. `Entry::rw_user_mmio()` — a page-table entry for user-accessible device
   MMIO (AllRw, Device, XN both, InnerShareable).
2. `Aspace::map_mmio()` — maps a physical range into the process's TTBR0
   only (no TTBR1 mapping).
3. `SYSMAPMMIO` (syscall 20) — the user-space interface: x0 = physical
   address, x1 = user VA; maps the page into the *current* process's TTBR0.

The kernel is device-dumb: it provides the capability, the server decides
which MMIO to map (the QNX model).

## Changes

### `aarch64/src/vm.rs`

Add `Entry::rw_user_mmio()`:

```rust
/// User+kernel read/write, Device memory (not cacheable), execute-never,
/// inner-shareable.  For device MMIO mapped into a process's TTBR0.
pub fn rw_user_mmio() -> Self {
    Entry(0)
        .with_access_permission(AccessPermission::AllRw)
        .with_shareable(Shareable::Inner)
        .with_accessed(true)
        .with_uxn(true)
        .with_pxn(true)
        .with_mair_index(Mair::Device)
        .with_valid(true)
}
```

Verify: the existing `rw_device()` has `PrivRw` (kernel-only) — this one
has `AllRw` (user+kernel). The MAIR index and shareability are the same.

### `aarch64/src/aspace.rs`

Add `Aspace::map_mmio`:

```rust
/// Map a device's MMIO range into this AS at `va` (the process sees the
/// device registers).  Does NOT map into TTBR1 (the kernel does not need
/// the device page; the server owns it exclusively).  The range must be
/// page-aligned and ≤ one page for this arc.
pub fn map_mmio(&self, range: &PhysRange, va: usize) -> Result<(), PageAllocError> {
    unsafe { &mut *self.root }.map_phys_range(
        &mut PhysPageAllocator {},
        &mut VmTraitImpl {},
        "aspace-mmio",
        range,
        VaMapping::Addr(va),
        Entry::rw_user_mmio(),
        crate::vm::PageSize::Page4K,
        RootPageTableType::User,
    ).map_err(|_| PageAllocError::UnableToMap)
}
```

No TTBR1 mapping (unlike `map_user_page`). No pagealloc (the MMIO pages are
not allocated — they are the device's fixed physical registers).

### `aarch64/src/process.rs`

Add `SYSMAPMMIO: u64 = 20`.

### `aarch64/src/ipc.rs` (or `trap.rs` — wherever the syscall handlers live)

Add the `SYSMAPMMIO` handler:

```rust
/// Map a physical page into the current process's TTBR0 with Device
/// attributes.  x0 = physical address (page-aligned), x1 = user VA.
/// Returns 0 on success, 1 on failure.
pub fn sys_map_mmio(pa: usize, va: usize) -> u64 {
    if pa & (PAGE_SIZE_4K - 1) != 0 {
        return 1; // not page-aligned
    }
    let id = match current_id() { Some(id) => id, None => return 1 };
    // Access the current process's Aspace via the process table.
    let range = PhysRange::with_pa_len(PhysAddr::new(pa as u64), PAGE_SIZE_4K);
    match /* process's aspace */ .map_mmio(&range, va) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}
```

The handler accesses the process table to get the current process's Aspace.
The pattern matches the existing `sys_send`/`sys_receive` handlers (which
access the process table for the current process).

### `aarch64/src/trap.rs`

Add `SYSMAPMMIO` to the svc dispatch match:

```rust
SYSMAPMMIO => { x0 = ipc::sys_map_mmio(x0 as usize, x1 as usize); }
```

### Host stub

The host stub `Aspace` gains a matching `map_mmio` (never called):

```rust
pub fn map_mmio(&self, _range: &PhysRange, _va: usize) -> Result<(), ()> { Ok(()) }
```

## Tests

- Host: none (the method and syscall are target-only).
- aarch64 target compiles clean (clippy).
- The integration image (next task) exercises the path.

## Done when

- `Entry::rw_user_mmio()` exists with the correct attributes (AllRw, Device,
  XN both, InnerShareable).
- `Aspace::map_mmio` maps the range into TTBR0 only (no TTBR1 mapping).
- `SYSMAPMMIO` (20) is in the svc dispatch; the handler maps the page into
  the current process's TTBR0.
- xtask green across all three arches (fmt, check, clippy ×3, test, dist ×3).
