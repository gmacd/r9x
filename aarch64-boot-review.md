# Review: aarch64 pre-`main9` boot path

Scope: everything from `start` in `aarch64/src/l.S` up to and including the
`bl main9` handoff — `l.S`, `pre_mmu/vminit.rs`, `pre_mmu/util.rs`,
`lib/kernel.ld`, `lib/config_default.toml`, `param.rs`, `kmem.rs`, `boot.rs`
(the `page_allocator` contract), and the parts of `vm.rs` vminit depends on.

Date: 2026-08-30.  Branch: `task-105-vm-table-publish-and-barriers` (clean).
Reviewed at the current HEAD (`ac98648`).

Experiments run (read-only):

- Booted the prebuilt debug kernel under
  `qemu-system-aarch64 -machine raspi4b -dtb aarch64/lib/bcm2711-rpi-4-b.dtb
  -kernel target/.../aarch64-qemu.gz -serial null -serial mon:stdio`
  and captured the early mini-UART transcript:

  ```
  .
  vminit:init_vm: calling init_kernel_page_tables
  Kernel Text 0x0..0x200000 -> 0xffff800000000000..0xffff800000200000
  Kernel RO Data 0x200000..0x2000000 -> 0xffff800000200000..0xffff800002000000
  Kernel Data 0x2000000..0x2200000 -> 0xffff800002000000..0xffff800002200000
  DTB 0x8000000..0x801d000 -> 0xffff800008000000..0xffff80000801d000
  vminit:init_vm: switching
  vminit:init_vm: complete
  ```

  So on QEMU raspi4b the DTB lands at PA `0x0800_0000` (128 MiB), clear of
  the (34 MiB debug) kernel image, and "Kernel Text" is mapped from **PA 0**.

- `objdump -h` / `nm` on the kernel ELF: section LMAs/VMAs and the `_pa`
  symbols (all values quoted below are from this build).

Overall the structure is sound: clear-then-publish ordering for new table
pages is right pre-MMU (task 105's problem statement says so too), the
`tlbi; dsb ish` at the end of `init_vm` is correctly ordered, AF is set on
every descriptor (no reliance on HA/HD), the mapped ranges' 2 MiB/4 KiB
alignment holds in the actual ELF, and the early allocator is bounds-checked.
The findings below are ordered most-severe-first within each class.

---

## a. Findings

### BUG

**1. `vminit::map_phys_range` swallows `map_to` failures and returns Ok.
[BUG, VERIFIED]**
Evidence: `aarch64/src/pre_mmu/vminit.rs:361-367` —

```rust
if map_to(root_page_table, page_allocator, entry.with_phys_addr(pa), currva, page_size)
    .is_err()
{
    putstr("error:vminit:map_phys_range:map_to failed: ");
    putstr(debug_name);
    putstr("\n");
}
```

The error is logged to the mini-UART and the loop continues; the function
then returns `Ok(virtrange)` (`vminit.rs:369`), so `init_vm`'s
`let Ok(..) = .. else { panic!() }` (`vminit.rs:160-171`) never fires for a
per-page failure.  The post-MMU twin propagates with `?`
(`aarch64/src/vm.rs:560-567`).  Why it matters: every mid-map failure —
early-pool exhaustion (`EarlyPageAllocator` has 32 pages, kernel.ld:60),
a 4 KiB mapping colliding with an existing 2 MiB block
(`EntryIsNotTable`), a range crossing the recursive index
(`MappingRecursiveIndex`) — leaves the range partially unmapped and boots
anyway.  The first touch of the unmapped VA then faults with VBAR_EL1
still at its reset value (finding 8), i.e. a wordless wedge.  Concrete
scenario: if the DTB physrange falls inside one of the kernel's 2 MiB
regions (firmware on real hardware places the DTB in low memory, unlike
QEMU's measured 0x8000000), the sorted map order
(`vminit.rs:133`) maps the 2 MiB kernel block first, the DTB's 4 KiB
`map_to` then hits a block where it needs a table, errors, is swallowed —
and `main9`'s `device_tree(dtb_va)` (main.rs:23) dereferences an unmapped
VA.

**2. Stale slot-511 self-pointers in the inherited pre-MMU tables poison the
post-MMU walker. [BUG (latent), VERIFIED logic; trigger requires a
particular PA]**
Evidence: pre-MMU `next_mut` installs a self-pointer in *every* table it
allocates — `vminit.rs:454-455`:

```rust
let new_table = unsafe { &mut *(page_pa.addr() as *mut Table) };
new_table.entries[RECURSIVE_SLOT] = entry;
```

The post-MMU walker guards only level 0 and explicitly treats index 511 at
L1/L2 as "an ordinary slot [that] must still map" — `vm.rs:372-380` — and
`docs/lessons.md:209-213` justifies that with "`next_mut` allocates every
other table `clear()`ed".  That claim is false for the three tables the
post-MMU walker *inherits* from vminit: TTBR1 is the early root
(`vminit.rs:290`; `vm.rs:586-592` reads it back), so the kernel keeps using
vminit's L1 table (L0 slot 256), its L2 table (KZERO+0..1 GiB) and the
DTB's L3 table — each with a stale, valid, table-bit self-pointer at
slot 511.  Failure scenario: any post-MMU 4 KiB mapping at
`VaMapping::Offset(KZERO)` of a frame in PA `0x3fe0_0000..0x4000_0000`
(the last 2 MiB below 1 GiB — present on QEMU raspi4b's 2 GiB and on every
real Pi 4) has L2 index 511.  `Table::next_mut` finds the stale
self-pointer valid-and-table (`vm.rs:383-394`), returns the recursive
alias of the L2 table itself as the "L3 table", and the subsequent
`entry_mut(Level3, va)` write lands **inside the live L2 table** — for
PA 0x3fe00000, at L2 slot 0, i.e. it overwrites the 2 MiB block mapping
the kernel's own text.  The kernel's next instruction fetch through
KZERO+0..2 MiB then translates through a user data frame.  Reachable via
`pagealloc::allocate_virtpage` / `aspace.rs:86-91`, which both 4 KiB-map
arbitrary allocator frames at the KZERO offset.  (Slot 511 of the DTB's
L3 table is benign — a leaf `entry_mut` would simply overwrite it — and
L1 slot 511 needs PA ≥ 511 GiB; the L2 case is the reachable one.)
The pre-MMU comment (`vminit.rs:419-424`) documents that the slot "is
dangerous at any level here", but nothing removes the extra self-pointers
before the tables become the live kernel tables.

**3. SCTLR_EL1 is never set to a known value; the MMU-enable is a
read-modify-write of reset-UNKNOWN state. [BUG on hardware, VERIFIED code;
consequence SUSPECTED (QEMU resets sanely)]**
Evidence: `l.S:132-135`:

```asm
mrs  x0, sctlr_el1
ldr  x1, =(SCTLR_EL1_I|SCTLR_EL1_C|SCTLR_EL1_M)
orr  x0, x0, x1
msr  sctlr_el1, x0
```

Architecturally most SCTLR_EL1 fields are UNKNOWN at reset when EL1 is
entered from a higher EL that hasn't initialised it (nothing in the EL3 or
EL2 paths at `l.S:73-98` touches it).  If EE (endianness), A/SA
(alignment checking), WXN or nAA come up set, the kernel misbehaves in
ways QEMU (which resets to a benign value) will never show.  Linux's
head.S writes a full INIT value for this reason.  The same applies to
SCTLR_EL2 on the EL3→EL2 hop (nothing initialises it before `eret` to
EL2 at `l.S:98` — EL2 executes with UNKNOWN SCTLR_EL2).

**4. The EL1-direct entry path never enables FP/SIMD, but the target
compiles with `+neon,+fp-armv8`. [BUG, VERIFIED code path; trigger
SUSPECTED (needs EL1 entry)]**
Evidence: `CPACR_EL1_FPEN` is written only inside the `el2:` path
(`l.S:91-93`); an entry already at EL1 branches straight to `el1`
(`l.S:68-69`) and skips it.  `lib/aarch64-unknown-none-elf.json:6` sets
`"features": "+strict-align,+neon,+fp-armv8"`, so the compiler is free to
emit SIMD/FP (it routinely vectorises `memcpy`-like code) anywhere in
`init_vm`.  With FPEN=0 the first FP/SIMD instruction traps — with
VBAR_EL1 unset (finding 8), i.e. a silent wedge.  QEMU raspi4b and the
Pi 4 armstub both enter above EL1, which is why this has not bitten; a
chainloader or hypervisor entering at EL1 would hit it.  Related EL2
state also left at reset on the EL3→EL2 path: CPTR_EL2 (its TFP bit, if
set, traps EL1/EL0 FP to EL2), CNTHCTL_EL2 and CNTVOFF_EL2 (EL1 access
to the physical counter/timer; QEMU's reset value happens to permit it,
and on real hardware the armstub sets these — but r9's own EL3 path sets
none of them).

### DESIGN-RISK

**5. Every early failure is a silent hang: VBAR_EL1 is not set until
`main9` runs `boot::irq_ops`. [DESIGN-RISK, VERIFIED]**
Evidence: the only `msr vbar_el1` in the tree is `trap.rs:22`
(`trap::init`, called via `boot::irq_ops` from `main.rs:20`).  The whole
pre-`main9` path — EL drop, BSS clear, DTB parse, page-table build, MMU
enable, higher-half jump — runs with VBAR_EL1 at reset (UNKNOWN;
0 on QEMU, where VA 0 is identity-mapped RWX by TTBR0, so a fault
"executes" the Pi spin-table bytes at PA 0).  Combined with findings 1
and 6, any early misstep is indistinguishable from a hang.  This is a
contract point worth documenting at the `bl main9` site, and arguably
worth a minimal early vector stub that at least dots the mini-UART.

**6. `init_vm` panics on a bad/missing DTB before printing anything, and a
pre-console panic prints nothing at all. [DESIGN-RISK, VERIFIED]**
Evidence: `vminit.rs:112` — `DeviceTree::from_usize(dtb_pa).unwrap()` runs
*before* the first `putstr` (`vminit.rs:115`).  The panic handler prints
via `iprintln!` (`runtime.rs:12`), but `iprint` drops output until
`set_iprint_ops` is called (`port/src/devcons.rs:191-195`), which happens
at console init inside `main9`.  On the kernel image (`not(qemu-test)`)
the handler then `loop {}`s (`runtime.rs:19-21`).  This is exactly the
observed "boot fails silently without `-dtb`" behaviour: x0 points at
garbage, the unwrap panics, nothing is printed.  A `putstr` before the
unwrap (or a magic check with an early-UART error) would turn the most
common bring-up mistake into a one-line diagnosis.

**7. The TTBR0 identity map — a 1 GiB RWX-at-EL1 block over PA 0 — stays
live from MMU-enable until `vm::switch` in `main9`. [DESIGN-RISK,
VERIFIED]**
Evidence: `vminit.rs:198-217` builds `physicalpt4[0] → physicalpt3[0]`, a
1 GiB block at PA 0, PrivRw, UXN but **not PXN**; `TTBR0_EL1.set` at
`vminit.rs:291`; nothing tears it down until
`vm::switch(user_pagetable(), User)` at `main.rs:49` — after `irq_ops`,
DTB parse, page-allocator init, mailbox, console and interrupt bring-up
have all run.  Through that whole window a null or low-VA dereference in
kernel code reads/writes physical memory (or executes it) instead of
faulting.  Relatedly, the higher half maps VA KZERO+0 too: the
"Kernel Text" range starts at PA 0 (`vminit.rs:37-39,86,123`, transcript
above), so the TODO at `vminit.rs:117` ("leave the first page unmapped to
catch null pointer dereferences") is doubly unrealised.

**8. Missing ISB between the translation-register writes and the MMU
enable. [DESIGN-RISK, VERIFIED as absent; consequence SUSPECTED]**
Evidence: `init_vm` writes TTBR1/TTBR0/TCR/MAIR (`vminit.rs:290-312`) and
ends with `dsb ish` (`vminit.rs:321`); `l.S:131-135` then does `dsb sy`
and `msr sctlr_el1` with no intervening `isb`.  A DSB is not context
synchronization: direct writes to system registers are only guaranteed
visible to the translation machinery after an ISB (or other context
synchronization event).  Architecturally, the instruction fetches
immediately after `msr sctlr_el1` (the `isb` at `l.S:140` itself) may
translate using stale/UNKNOWN TTBR/TCR/MAIR.  QEMU synchronises eagerly
so this never shows there; the canonical sequence is
`msr ttbr…/tcr/mair; isb; msr sctlr; isb`.  Given this branch's task
(105) is precisely about barrier correctness in the walker, this is the
pre-MMU sibling.

**9. No cache invalidation before enabling I and C. [DESIGN-RISK,
VERIFIED as absent; hardware-only]**
Evidence: `l.S:133-135` sets SCTLR I|C|M; nowhere in the pre-`main9` path
is there an `ic iallu` or a set/way D-cache invalidate, and the page
tables were written with the D-cache off while TCR programs cacheable
table walks (IRGN/ORGN WB, `vminit.rs:297-304`).  Cache contents are
architecturally UNKNOWN out of reset; QEMU has no caches to go stale.  On
real silicon this depends entirely on the firmware having cleaned and
invalidated — the classic boot bug QEMU cannot catch.

**10. The recursive/self-pointer entries carry inconsistent memory
attributes (non-shareable, executable) versus everything else.
[DESIGN-RISK, VERIFIED]**
Evidence: the root's self-pointer (`vminit.rs:146-150`) is built from
`Entry::empty()` with only phys/accessed/valid/table — AP=PrivRw by
zero-default, **Shareable::Non**, no UXN/PXN; the per-table self-pointers
(`vminit.rs:443-450`) likewise, with `.with_shareable(Shareable::Inner)`
present but **commented out** (`vminit.rs:445`).  When the recursive walk
lands on these as leaf descriptors, page-table memory is mapped Normal
WB **non-shareable** and executable — while the same physical pages are
also mapped Inner-shareable via the "Kernel Data" 2 MiB blocks, and the
post-MMU equivalent uses `rw_kernel_data()` (Inner + UXN + PXN,
`vm.rs:694-700`).  Mismatched shareability attributes for one PA are
architecturally unpredictable for coherency on real SMP — exactly the
kind of thing this project's AGENTS.md says must be correct — and the
missing PXN makes the live page tables executable.

**11. `ebss_pa` hangs off `LOADADDR` of an empty `.end` section, and the
early-pagetable reservation is implicit. [DESIGN-RISK, VERIFIED
fragility; currently correct]**
Evidence: `kernel.ld:67-70`:

```ld
/* TODO Should this be here???  Better to put into the bss section later */
.end : ALIGN(2097152) {}
PROVIDE(ebss_pa = LOADADDR(.end));
```

In this build `nm` shows `ebss_pa = 0x2200000` (a physical address —
correct), but `objdump -h` shows `.end` with **LMA == VMA ==
0xffff800002200000**: the empty section fell out of the LMA chain, and
only the symbol arithmetic happened to land right.  A linker-version or
script change that resolves `LOADADDR(.end)` to the VMA would make the
BSS-clear loop at `l.S:110-114` run from `0x202d000` toward
`0xffff8000_02200000`, wiping memory until it faults.  Two load-bearing
invariants hide here with no assertion anywhere: (a) `bss_pa..ebss_pa`
covers `.earlyvm_pagetables` (that containment is the only thing that
maps the live kernel page tables in the higher half *and* reserves them
from the page allocator via `bss_physrange()` in `boot.rs:40`); (b)
`ebss_pa` is 2 MiB-aligned so `data.add(bss)` passes `map_phys_range`'s
alignment gate.

**12. The clear loops are do-while: an empty or non-8-byte range clears
memory until wraparound. [DESIGN-RISK, VERIFIED logic; currently
unreachable]**
Evidence: `l.S:112-114` (and 118-120):

```asm
1:	str	xzr, [x0], #8
	cmp	x0, x1
	b.ne	1b
```

The store precedes the comparison, and the exit test is `!=`, so
`bss_pa == ebss_pa` writes 8 bytes then clears essentially all of memory;
a range not a multiple of 8 never terminates.  Today both ranges are
non-empty and 4 KiB-aligned; the failure mode is what finding 11 would
produce.

**13. 16 KiB boot stack, no guard, with heavyweight pre-MMU work on it.
[DESIGN-RISK, VERIFIED layout; overflow SUSPECTED only]**
Evidence: `STACKSZ = 4096*4` (`l.S:3`), `stack` in `.bss`
(`l.S:161-163`; `nm`: stack at phys `0x20c0000`, top `0x20c4000` —
immediately below `earlyvm_pagetables_pa`).  It grows down into the rest
of `.bss`, so an overflow during the debug-build DTB parse or table build
silently corrupts kernel statics; nothing detects it.  (The early page
tables sit *above* the initial SP, so they are at least not in the
overflow path.)

**14. MPIDR check masks only Aff0. [DESIGN-RISK, VERIFIED; benign on
Pi 4]**
Evidence: `l.S:53-55` (`and x0, x0, #0xff`).  A core with Aff0==0 in a
nonzero cluster (Aff1/Aff2) would race core 0 through the whole boot
path.  BCM2711 has one cluster, so this is future-proofing only — but the
comment "All cores other than 0 should just hang" (`l.S:52`) promises
more than the mask delivers.

**15. Early mini-UART timing assumptions are firmware-configuration
dependent on real hardware. [DESIGN-RISK, SUSPECTED]**
Evidence: `util.rs:33-35` hardcodes `UART_CLOCK = 500_000_000` (the VPU
core clock).  On a real Pi 4 the mini-UART baud divisor tracks
`core_freq`, which config.txt (`enable_uart=1`, `core_freq=…`) can pin to
other values; if it differs, the early transcript is garbage at 115200.
QEMU models the fixed 500 MHz so this never shows there.  The register
programming itself checks out against BCM2711: PUP_PDN bits 28-31 for
GPIO14/15 (`util.rs:52,64`), FSEL ALT5 via GPFSEL1 bits 12-14/15-17
(`util.rs:55-72`), LCR=3 for 8-bit (the documented BCM2835 LCR quirk),
divisor 541 → 115207 baud.

### COMMENT (comment/contract mismatches — both sides quoted)

**16. The MAIR comment claims nGnRnE; the value programs nGnRE.
[COMMENT, VERIFIED]**
`vminit.rs:308-312`:

```rust
//  [1] 0x04 - Device nGnRnE (non-gathering, non-reordering, no early ack)
MAIR_EL1.set(0x04ff);
```

MAIR attr `0x00` is Device-nGnRnE; **`0x04` is Device-nGnRE** (early
write acknowledgement permitted).  Every `Mair::Device` mapping in the
kernel (all MMIO, via `rw_device()`/`rw_user_mmio()`, vm.rs:156-179)
therefore gets posted-write semantics while the comment promises
strongly-ordered.  Either the comment or the value is wrong; if nGnRnE
was the intent (the usual conservative choice for BCM2711 peripherals),
the value should be `0x00ff`.

**17. `map_phys_range`'s doc promises rounding; the code rejects
unaligned ranges. [COMMENT, VERIFIED — in both copies]**
Doc (`vminit.rs:326-329`, verbatim again at `vm.rs:523-526`):

```rust
/// Map the physical range using the requested page size.
/// This aligns on page size boundaries, and rounds the requested range so
/// that both the alignment requirements are met and the requested range are
/// covered.
```

Code (`vminit.rs:341-348`, same shape at `vm.rs:540-547`):

```rust
if !range.start.is_multiple_of(page_size.size() as u64)
    || !range.end.is_multiple_of(page_size.size() as u64)
{
    ...
    return Err(PageTableError::PhysRangeIsNotOnPageBoundary);
}
```

The only rounding left is `step_by_rounded`, which is dead weight after
the gate (callers must pre-round, as `init_vm` does for the DTB at
`vminit.rs:120-121`).

**18. `docs/lessons.md` overstates the post-MMU invariant that finding 2
breaks. [COMMENT, VERIFIED]**
`docs/lessons.md:209-211`: "The **post-MMU** sets entry 511 of the *root*
only: `next_mut` allocates every other table `clear()`ed" — true of
tables the post-MMU walker allocates, false of the three tables it
inherits from vminit (see finding 2).  The lesson's rule ("check whether
they build their tables the same way") is exactly right; its statement of
the post-MMU structure needs the "except the inherited pre-MMU tables"
qualifier — or the code needs to make the statement true.

**19. Dead `mrs x0, elr_el1` in the MMU-enable sequence. [COMMENT/NIT,
VERIFIED]**
`l.S:141` reads ELR_EL1 into x0; x0 is unconditionally overwritten at
`higher_half` (`l.S:148`) before any use.  Leftover debugging — and in a
sequence where every instruction between `msr sctlr_el1` and the
higher-half jump deserves to justify itself, dead code is actively
misleading.

**20. x28 "entrypoint" is cached and never used, from an undocumented
x4 convention. [COMMENT, VERIFIED]**
`l.S:46,50` (`mov x28, x4  // Cache entrypoint (offset)`).  x4 holding
the entry address is a QEMU-boot-stub/armstub8 artifact, not part of the
arm64 Linux boot protocol the rest of the entry contract follows; x28 is
referenced nowhere else (grep confirms).  Either use it, or drop the two
lines and the register reservation comment.

**21. "identity map the kernel virtual addresses" describes the offset
map. [COMMENT, VERIFIED]**
`vminit.rs:227-229`: "the next step is to enable the MMU and identity map
the kernel virtual addresses to the physical addresses that the kernel
was loaded into" — the TTBR1 mapping is a KZERO-*offset* mapping; the
identity map is TTBR0's transition crutch (correctly described later at
`vminit.rs:278-286`).

**22. Typos. [NIT, VERIFIED]** `l.S:41` "throught"; `l.S:130` "be be
seen"; `vm.rs:101` "secrtion" (in the Entry bitstruct doc that vminit
depends on).

### NIT

**23. The early page-table pool is cleared four times.** l.S BSS clear
covers it (`bss_pa..ebss_pa` ⊇ `.earlyvm_pagetables`), l.S clears it
again explicitly (`l.S:116-120`), `EarlyPageAllocator::new` clears every
page (`vminit.rs:487-489`), and `alloc_physpage` clears each page on
handout (`vminit.rs:501`).  Any one of the last two suffices; 128 KiB of
redundant stores and, more importantly, three places for the invariant to
live.

**24. `.text.boot` is `"awx"` (`l.S:37`) and `.bootdata` is lumped into
the RO+X "Kernel Text" mapping** (`kernel.ld:15`, `vminit.rs:123,129`):
writable ELF flags on a section mapped read-only, and any future bootdata
would silently become read-only-executable.

**25. `kmem.rs` and `vminit.rs` duplicate the entire `_pa` extern/accessor
block** (kmem.rs:5-79 vs vminit.rs:23-107) with one subtle shared quirk:
both define `boottext_physrange()` as starting at PA **0**, not
`boottext_pa` (0x80000) — deliberate for reserving/mapping the spin-table
low pages, but undocumented in either place, and the diagnostic output
(`main.rs:102`) prints "boottext" as starting at 0 as a result.

**26. `root_page_table.entries[511]` uses the literal 511**
(`vminit.rs:146`) where `RECURSIVE_SLOT` exists and is imported
(`vminit.rs:17`); same literal at `vm.rs:699,843,852`.

**27. An EL0 entry falls into the EL3 path.** `l.S:65-71` dispatches
EL1/EL2 and falls through "Must be EL3" for anything else; CurrentEL==0
would attempt `msr scr_el3` and take an undefined-instruction trap with
no vectors.  Unreachable in practice; a `b .` guard would make the
assumption explicit.

**28. `"relocation-model": "pie"`** (`aarch64-unknown-none-elf.json:12`)
with no boot-time relocation processing: correct today only because the
link addresses are final and lld pre-fills relative-reloc targets, but it
makes the image carry `.got` entries (`kernel.ld:44-45`) whose validity
is an accident of linker behaviour rather than a stated contract.

---

## Handoff contract at `bl main9` (l.S:155), as actually implemented

| State | Value at handoff | Evidence |
|---|---|---|
| x0 | `KZERO + dtb_pa` (dtb_pa from x27, QEMU/firmware x0) — mapped RO 4 KiB, rounded range | l.S:153-154; vminit.rs:120-128 |
| sp | `KZERO + stack_pa + 0x4000` (16 KiB, in .bss, RW via 2 MiB Kernel Data blocks, no guard) | l.S:148-150, 161-163 |
| SCTLR_EL1 | reset value OR (M\|C\|I) — other bits *not* controlled (finding 3) | l.S:132-135 |
| TTBR1_EL1 | early root (earlyvm pool page 0, phys 0x20c4000 this build) — **stays live**; post-MMU code adopts it via `mrs` | vminit.rs:290; vm.rs:586-592,724-735 |
| TTBR0_EL1 | `physicalpt4`: 1 GiB identity RWX-at-EL1 block at PA 0 — live until `vm::switch` in main9 (finding 7) | vminit.rs:198-217,291; main.rs:49 |
| TCR_EL1 | 4 KiB granules both halves, T0SZ=T1SZ=16 (48-bit), IPS=44-bit, WB RA WA inner-shareable walks | vminit.rs:293-306 |
| MAIR_EL1 | 0x04ff: idx0 Normal WB-WA, idx1 Device-**nGnRE** (comment says nGnRnE — finding 16) | vminit.rs:312 |
| DAIF | D,A,I,F all masked (set via SPSR_EL2/EL3 before each eret) | l.S:76-77, 88-89 |
| VBAR_EL1 | **reset/UNKNOWN** — first set by `boot::irq_ops` → `trap::init` inside main9 (finding 5) | trap.rs:22; main.rs:20 |
| CPACR_EL1 | FPEN set **only if entry EL > EL1** (finding 4) | l.S:91-93 |
| Live UART | mini-UART (AUX) initialised and reachable **pre-MMU only**; its MMIO (0xfe215000) is unmapped in both halves after MMU-enable (identity map covers only 0..1 GiB). PL011 untouched. iprint/console: nothing until main9's console init — output before that is dropped | util.rs:31-79; vminit.rs:198-206; devcons.rs:191-195 |
| TLBs | `tlbi vmalle1is; dsb ish` done pre-enable; `isb` after enable | vminit.rs:315-321; l.S:140 |
| Caches | never invalidated by r9 (finding 9) | l.S:122-143 |
| Secondary cores | parked in `wfe` at physical `dnr`, MMU off, never released | l.S:52-55, 157-159 |

---

## b. Integration-test checklist

Conventions available (verified against `aarch64/tests/pagetables.rs`,
`common/`, and the xtask harness): a test image links the same library,
`start` runs the full l.S prefix, the image's own `main9` runs whatever
init it needs, asserts with `check!`, and leaves via
`qemu::exit(PASS/FAIL)` — the harness asserts exit status 0 within a
timeout (`xtask/src/main.rs:1419,1750-1782`).  Harness-side checks can
also assert on captured serial output.  Note QEMU wires the PL011 as the
first `-serial` and the mini-UART as the second, so a harness check on
early output must capture serial index 1.

### Area 1: register-state contract at main9 (new image, e.g. `boot_contract.rs` — no init needed before the asserts)

1. `mrs CurrentEL` == EL1.  *Catches*: regressions in the EL3→EL2→EL1
   drop (wrong SPSR M-field, eret to the wrong level).
2. `mrs SCTLR_EL1`: assert M, C, I set; assert A, SA, WXN, EE, E0E
   clear.  *Catches*: finding 3 becoming real (an inherited garbage bit),
   and any future change to the enable mask.
3. `mrs DAIF`: all four masked at entry.  *Catches*: an eret path that
   stops masking, which would let a spurious interrupt hit the unset-VBAR
   window.
4. `mrs TCR_EL1`: T0SZ==16, T1SZ==16, TG0==4K, TG1==4K, IPS==44-bit,
   EPD0==EPD1==0.  *Catches*: TCR drift (e.g. someone enabling EPD1 per
   the commented line at vminit.rs:299, or changing granule assumptions
   the `vm.rs` walker hardcodes).
5. `mrs MAIR_EL1` == the decided value, with a comment stating
   nGnRE-vs-nGnRnE intent.  *Catches*: finding 16's resolution
   regressing; silent device-memory-type changes.
6. `mrs TTBR1_EL1`: nonzero, 4 KiB-aligned, and within
   `earlyvm_pagetables_pa..eearlyvm_pagetables_pa` (compare against the
   linker symbols in-image).  Same for TTBR0 at entry.  *Catches*: root
   table moving out of the reserved pool (which would un-reserve it from
   the page allocator).
7. `mrs VBAR_EL1` after `boot::irq_ops`: nonzero and 2 KiB-aligned.
   *Catches*: vector installation silently dropping out of the early
   main9 sequence (finding 5's window growing).
8. Execute an FP op before any init (`black_box(1.5f64) * black_box(2.0)`,
   compare bits).  *Catches*: CPACR/CPTR trap configuration on whatever
   EL path QEMU took (partial coverage of finding 4).
9. Record the entry EL: have l.S stash CurrentEL-at-entry in a callee-
   saved register / bss word and assert in the test that the FP check
   passed *for that path*.  *Catches*: the EL1-direct hole in finding 4
   once any entry path changes.

### Area 2: page-table structure and attributes (extend `pagetables.rs` or a sibling image; console up, software-walk via `vm::kernel_pagetable()`)

10. Software-walk TTBR1 for the first and last byte of each of
    boottext/text (expect leaf: valid, AF, PrivRo, **PXN clear**, UXN
    set, Inner, Mair 0), rodata (PrivRo, PXN+UXN), data/bss/stack
    (PrivRw, PXN+UXN), DTB (PrivRo, 4 KiB leaf).  *Catches*: attribute
    regressions that don't fault in normal boot (e.g. text accidentally
    writable, rodata executable) and wrong page-size choices.
11. Assert root[511] is valid, table-bit, and its phys_addr equals
    TTBR1's PA (self-pointer integrity), and — once finding 10 is fixed —
    assert its attribute bits equal `rw_kernel_data()`'s.  *Catches*:
    recursive-slot corruption at boot; attribute drift between pre- and
    post-MMU recursive entries.
12. Assert the *inherited* tables' slot 511 state explicitly: read the L1
    table (root[256]) and the L2 table (L1[0]) through KZERO-offset
    pointers and check entry 511.  Today this asserts the documented
    self-pointer; after the finding-2 fix it asserts invalid.  Either
    way, pin it — this is the test that makes finding 2's silent
    corruption a loud failure.  *Catches*: pre/post-MMU slot-511
    structure drift in both directions.
13. Trigger test for finding 2: `map_phys_range` a scratch 4 KiB frame at
    `VaMapping::Addr(KZERO + 0x3fe00000)` (L2 index 511), then verify (a)
    the call's result, (b) the L2 table's entry 0 still maps kernel text
    (read a known text byte through KZERO+0x80000), (c) execution still
    works (call a function).  Today (b)/(c) fail catastrophically or the
    corruption is latent — run this in a dedicated image so a wedge is a
    clean timeout-FAIL.  *Catches*: the walk-through-stale-self-pointer
    corruption, and its fix.
14. Walk every valid entry reachable from the TTBR1 root and assert each
    table-descriptor phys_addr lies inside the earlyvm pool (pre-main9
    state) — count the tables and assert the count (currently 4:
    L1, L2, DTB-L3, +root) and that pool usage leaves headroom
    (e.g. ≤ 16 of 32).  *Catches*: silent pool growth toward the
    exhaustion that finding 1 currently swallows; wild table pointers.
15. Assert TTBR1 root has valid entries only at index 256 (and 511).
    *Catches*: stray early mappings appearing in the kernel half.
16. Fault-path checks (pattern exists in `aspace_fault.rs`): after trap
    init, (a) write to a rodata VA → expect data abort (permission), (b)
    write to the DTB VA → expect data abort, (c) call through a pointer
    into .data → expect instruction abort (PXN).  *Catches*: any mapping
    accidentally RW/executable — QEMU enforces AP/PXN faithfully.
17. After `vm::switch` to the user table: read from a low VA (e.g. 0x0)
    under a fault-catching handler → expect translation fault.  Before
    the switch, assert TTBR0's root[0] is the identity table (documents
    finding 7's window); after, assert low VA faults.  *Catches*: the
    identity map outliving its documented lifetime.

### Area 3: DTB and handoff values

18. `check!(dtb_va >= KZERO)` (exists, pagetables.rs:46) — extend with:
    read the FDT magic *and* the last byte of `dt.size()` through
    `dtb_va` (touches the final mapped page).  *Catches*: off-by-one in
    the DTB range rounding (`round`/`with_pa_len`) leaving the tail page
    unmapped — currently that error would be swallowed (finding 1) and
    only bite on a large DTB.
19. Assert `dt.size()` is sane (> 0, < 4 MiB) and that
    `PhysRange(dtb_pa, +size)` does not intersect
    `total_kernel_physrange()`.  *Catches*: the DTB-inside-kernel-image
    overlap scenario of finding 1 the moment QEMU/firmware placement or
    kernel size makes it real — turning a wordless post-boot fault into a
    named assertion.
20. Assert `(dtb_va - KZERO)` matches what `boot::page_allocator` reserved:
    allocate a few pages and assert none falls inside the DTB or kernel
    ranges.  *Catches*: the vminit/boot.rs dual bookkeeping (two
    independently computed range lists, vminit.rs:118-135 vs
    boot.rs:35-42) drifting apart.

### Area 4: linker-layout invariants (cheap asserts on the `_pa` symbols, in-image)

21. Assert each of: `text_pa`..`etext_pa`, `rodata_pa`..`erodata_pa`
    ascending; `etext_pa`, `erodata_pa`, `data_pa`, `ebss_pa` 2 MiB-
    aligned; `bss_pa`, `edata_pa`, earlyvm bounds 4 KiB-aligned.
    *Catches*: kernel.ld drift that would make `map_phys_range` reject a
    range — which today is a swallowed error and a hang (finding 1).
22. Assert `ebss_pa < 0x1_0000_0000` (it must be a *physical* address).
    *Catches*: the empty-`.end` LMA fragility (finding 11) turning
    `ebss_pa` into a VMA — before the BSS clear loop destroys the world.
23. Assert `bss_pa <= earlyvm_pagetables_pa` and
    `eearlyvm_pagetables_pa <= ebss_pa` (containment), and
    `earlyvm_pagetables_pa - stack_top >= 0` documented explicitly.
    *Catches*: someone moving `.earlyvm_pagetables` out of the
    data+bss reservation — which would hand the live kernel page tables
    to the page allocator.
24. Assert BSS was actually cleared: place a canary
    `static mut X: u64 = 0;`-style bss symbol and check it's zero before
    anything writes it; also assert a known `.data` value survived.
    *Catches*: clear-range regressions from findings 11/12.

### Area 5: harness-side (no kernel change needed)

25. Boot the *full kernel* image capturing **both** serials; assert the
    mini-UART stream contains `vminit:init_vm: complete` and matches
    `Kernel Text 0x0+\.\.0x0*[0-9a-f]+ -> 0xffff8000…` for all four map
    lines, and assert it contains **no** line matching `error:vminit`.
    *Catches*: every failure path that finding 1 currently reduces to a
    log line — pool exhaustion, misaligned ranges, map collisions —
    with zero kernel changes.  (This is the highest-value single check
    on this list.)
26. Assert the early `.` (l.S:124-125) appears before the vminit banner.
    *Catches*: early-UART init regressions and l.S sequencing changes.
27. Negative test: run a test image *without* `-dtb` and assert the run
    FAILs *with a diagnostic* rather than timing out — this is the
    pinning test for finding 6 once init_vm prints before the unwrap
    (today it can only assert "timeout", which is still worth pinning so
    the fix is observable).
28. Assert exactly one boot banner (no duplicated/interleaved lines).
    *Catches*: a secondary core escaping the `dnr` park (finding 14
    becoming real on a future multi-cluster target).
29. Golden-transcript drift check: the four mapped ranges' *starts* are
    stable across builds (`0x0`, `0x200000` only if rodata moves is it
    expected to change); flag any change for review rather than fail.
    *Catches*: silent layout changes (e.g. text growing past 2 MiB
    boundary changes the rodata start) that alter what is RO/RX.

### Area 6: honest gaps — not catchable under QEMU

30. Missing ISB before MMU enable (finding 8), missing cache invalidation
    (finding 9), SCTLR/SCTLR_EL2 reset garbage (finding 3), mismatched
    shareability on the recursive alias (finding 10), mini-UART clock
    assumptions (finding 15): QEMU synchronises eagerly, has no caches,
    resets registers to benign values, and models a fixed clock.  These
    need a real-Pi 4 boot smoke test (serial transcript over the wire) in
    CI-adjacent tooling, or at minimum a `docs/` page marking them as
    QEMU-unverifiable so nobody mistakes green CI for coverage.  The
    checklist items in Areas 1-2 still pin the *values* so hardware
    behaviour has a spec to be compared against.

---

## c. Cross-reference: comment/contract mismatches

Quoted in findings 16 (MAIR nGnRnE vs 0x04), 17 (`map_phys_range` "rounds"
vs rejects), 18 (lessons.md "every other table clear()ed" vs inherited
vminit tables), 14 ("All cores other than 0" vs Aff0-only mask), 21
("identity map" vs offset map), 20 (x28 "entrypoint" vs never used), and
the `l.S:42-48` register-reservation comment which omits x20
(clobbered at l.S:142).
