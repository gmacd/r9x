# Plan: aarch64 pre-`main9` boot review, 2026-08

Deep review of everything that executes up to and including `bl main9`
on aarch64 — `l.S`, `pre_mmu/vminit.rs`, `pre_mmu/util.rs`,
`lib/kernel.ld`, `param.rs`, `kmem.rs`, and the `vm.rs`/`boot.rs` pieces
they use — at `ac98648`, 2026-08-30. Four review passes: three
independent reviews (pi, claude-opus, claude-fable), each with full file
reads and live QEMU 11.0.1 `raspi4b` experiments, merged, then an
adversarial re-verification of the merged audit against the code, the
linker output and fresh QEMU runs (claude-opus as final gate). Findings
carry their verdict labels (VERIFIED / SUSPECTED) and the reviewers who
raised them. Opened tasks 135–143, waves 0–8 (landing order):

| wave | task | What lands |
|---|---|---|
| 0 | 135 `premain-failure-observability` | the B4 cluster: early vector table, `putstr` before the DTB unwrap, pre-MMU panic hook, the mini-UART page mapped into the early tables (the post-MMU prints currently ride on the 1 GiB identity block, which task 142 makes PrivRo), mini-UART to a captured sink; xtask stdout capture, per-image failure expectations, QEMU overrides, the `-dtb` guard, the dtb's real memory node; scaffolds the first three test images (`earlyoutput.rs` implemented fully, `bootcontract.rs`, `earlytables.rs`) |
| 1 | 136 `vminit-map-error-propagation` | B1: `map_phys_range` swallows every `map_to` error; the round-vs-reject decision (D20); the `next_mut` lifetime laundering (D15) |
| 2 | 137 `vminit-null-page-policy` | B5: "Kernel Text" starts at PA 0; the null-page and boottext-start mapping-strategy decision |
| 3 | 138 `vminit-recursive-alias-live` | B2: stale slot-511 self-pointers in the inherited tables leave writable, EL1-executable page-table aliases behind — the live corruption path |
| 4 | 139 `earlyvm-pool-reservation` | D6/D14: the live tables are reserved from the allocator only by section-ordering accident; one home for the `_pa` symbols and the range list |
| 5 | 140 `ls-entry-path-convergence` | B3/D8/D19: CPACR and DAIF convergence in `el1:`, the MPIDR cluster mask, parked-core protocol |
| 6 | 141 `boot-mmu-enable-sequence` | D1/D2/D3: full SCTLR write, cache maintenance, the canonical ISB/TLBI ordering (DEN0024) |
| 7 | 142 `early-table-lifetime-hygiene` | D4/D5/D10/D12/D17: TTBR0 block lifetime and permissions, the user root's recursive slot, the stack guard, the NOBITS pool, the DTB-overlap check |
| 8 | 143 `premain-comment-sweep` | C1–C18 plus D9/D11/D13/D16/D18/D22/D23: comments, KZERO single-sourcing, the do-while BSS loops, the SError decision, self-pointer attributes, section attributes, dead code |

Rulings recorded here because the tasks branch on them:

1. **The TTBR0 identity block is a 1 GiB L1 block**, not a 2 MiB one:
   with a 48-bit VA the walk starts at level 0 (`physicalpt4`), so
   `physicalpt3` is level 1 and its non-table descriptor covers 2^30.
   The low-VA identity map covers the whole kernel image *and* the QEMU
   DTB at PA `0x8000000` while it is live (it is torn down only by
   `vm::switch`, which no short test image ever calls). Confirmed live in
   the guest (`physicalpt3[0] = 0x0040000000000701`).
2. **The QEMU `raspi4b` boot contract** (measured, QEMU 11.0.1): r9
   requires `-dtb` (QEMU does not — it hands the guest x0=0); machine RAM
   is fixed at 2 GiB; QEMU rewrites the memory node of the supplied dtb
   to 960 MiB for every accepted `-m`; the supplied dtb is otherwise
   used as-is. Consequence used throughout: no QEMU in the harness can
   expose PA `[0x3fe00000, 0x40000000)` to the guest, which bounds what
   the B2 integration tests can prove (and why the zero-memory-node
   negative test is metal/host-only).
3. **B2's reachability on stock-firmware Pi hardware is unverified and
   probably not triggered there either**: `pagealloc` takes only the
   first memory regblock, which stock firmware ends below `0x40000000`
   (the VideoCore reserve — the same convention behind QEMU's 960 MiB).
   The defect stands on the broken invariant and the live aliases, which
   are verified today.
4. **D20 (round-vs-reject) is a recorded decision**, landed with task
   136; it constrains the script change in task 139 and unblocks task
   137. **D13 (SError)** is a recorded decision, landed with task 143.
5. **Build-artefact caveat:** the prebuilt `aarch64-qemu.gz` is one page
   stale against the current debug ELF (`earlyvm_pagetables_pa`
   `0x20c5000` vs `0x20c4000`); live-guest numbers below are from the
   `.gz`, `nm`/`objdump` numbers from the ELF. A fresh build shifts
   those addresses by one page.

The audit follows in full.

---

# Audit

Scope: everything that executes up to and including the `bl main9` call:
`aarch64/src/l.S` (reset: EL3→EL2→EL1, MPIDR check, stack, BSS clear,
early UART, `init_vm`, MMU enable, higher-half jump), `pre_mmu/vminit.rs`
(`init_vm` and the pre-MMU page-table machinery), `pre_mmu/util.rs`
(mini-UART), `lib/kernel.ld` + `lib/config_default.toml`, `param.rs`,
`kmem.rs`, `boot.rs` at the boundary, the `vm.rs` pieces `vminit` uses,
and the handoff contract at `bl main9`.

Source: three independent reviews of the same code (pi, claude-opus,
claude-fable), 2026-08-30, branch
`task-105-vm-table-publish-and-barriers` @ `ac98648`, merged and
adversarially re-verified by a fourth pass (claude-opus as final gate)
against the code, the linker output (`llvm-objdump`/`readobj`/`nm`,
reproduced `objcopy -O binary`) and fresh QEMU runs (`-machine dumpdtb`,
`pmemsave` of the pool and of PA 0). Its corrections are applied
throughout.

All findings below are deduplicated across the reports; each carries the
reviewers' verdict labels (VERIFIED = checked against code and/or
observed behaviour; SUSPECTED = logic verified, consequence expected
only on hardware QEMU cannot model). Findings marked **(all three)**
appeared in all three source reports independently.

Measurement environment: QEMU 11.0.1, `raspi4b`, debug image
`target/aarch64-unknown-none-elf/debug/aarch64-qemu.gz`. Observed early
transcript (mini-UART on stdio; `putu64h` emits full 16-digit hex):

```
.
vminit:init_vm: calling init_kernel_page_tables
Kernel Text 0x0000000000000000..0x0000000000200000 -> 0xffff800000000000..0xffff800000200000
Kernel RO Data 0x0000000000200000..0x0000000002000000 -> 0xffff800000200000..0xffff800002000000
Kernel Data 0x0000000002000000..0x0000000002200000 -> 0xffff800002000000..0xffff800002200000
DTB 0x0000000008000000..0x000000000801d000 -> 0xffff800008000000..0xffff80000801d000
vminit:init_vm: switching
vminit:init_vm: complete
```

The QEMU `raspi4b` boot contract, empirically established (QEMU 11.0.1):

- **r9 requires a dtb; QEMU does not.** Without `-dtb`, QEMU boots
  happily and hands the guest x0=0; `init_vm`'s
  `DeviceTree::from_usize(0).unwrap()` (`vminit.rs:112`) then panics
  *before its first `putstr`*. The visible output is just the `.` from
  `l.S:124-125`. The kernel image then hangs forever
  (`runtime.rs:19-21`, `#[cfg(not(feature = "qemu-test"))] loop {}`); a
  **test** image exits 1 (`qemu-test` panics call `qemu::exit(FAIL)`).
  Both reproduced.
- Machine RAM is fixed at 2 GiB: `-m 512M/1G/4G` all give "Invalid RAM
  size, should be 2 GiB".
- QEMU rewrites the memory node of the supplied dtb to `reg = <0x00 0x00
  0x3c000000>` — the guest sees **960 MiB** (`-machine dumpdtb`
  confirms it for every accepted `-m`), so a dtb with a 64 KiB (or zero)
  memory node still boots. The 960 MiB is the same VideoCore-reserve
  convention as stock Pi firmware.
- The supplied dtb is otherwise used as-is (the dumped fdt has the same
  node set as the input; renaming the GIC `compatible` panics the kernel
  at `gic.rs:205`).

The TTBR0 identity block is a **1 GiB L1 block**: with a 48-bit VA and
4K granules the walk starts at level 0; `physicalpt4` is the level-0
root (`vminit.rs:291`), `physicalpt4[0]` is a table descriptor
(`:216-217`) pointing at `physicalpt3`, which is a **level-1** table,
and `physicalpt3[0]` is a non-table descriptor (`:198-206`) — a
level-1 block, which covers **2^30 = 1 GiB** (a 2 MiB block would sit
one level deeper). The final gate confirmed the descriptor live in the
guest (`physicalpt3[0] = 0x0040000000000701`: block, PA 0, PrivRw,
Inner, Normal, PXN=0, UXN=1). The consequence is material: while live,
the low-VA identity map covers the whole kernel image *and* the QEMU
DTB at PA `0x8000000`.

## Findings

### Bugs

```

The self-pointer descriptor is valid, table, `AP=PrivRw`, `PXN=0`: **every
page table also exists as a writable, EL1-executable alias** at a KZERO VA
the kernel believes is unmapped (opus read the L3 table through
`0xffff8000081ff000` and `0xffff80003fe40000`).
The corruption path: any post-MMU 4 KiB mapping of a frame in PA
`[0x3fe00000, 0x40000000)` at `VaMapping::Offset(KZERO)` — which
`aspace.rs:77-96` and `aspace.rs:162-175` do with arbitrary allocator frames
— has L2 index 511 under L1 index 0; `next_mut` finds the stale self-pointer
valid-and-table, returns the L2 table *itself* as the "L3" table (the
final gate walked it independently: the recursive VA `0xffffffc0001ff000`
resolves root→root→L1→L2→the L2 table itself), and the leaf write lands at
L2[0] — **the 2 MiB block mapping the kernel's own text**.
Reachability: *what is verified* is that the invariant is broken and the
aliases are live, writable and EL1-executable **today** (probed on the
running guest), and that any code path that reaches `KZERO +
[0x3fe00000, 0x40000000)` corrupts the kernel-text mapping. *What is not
verified:* that a stock-firmware Pi actually hands out such a frame —
`pagealloc::init_page_allocator` (`pagealloc.rs:46-53`) takes only the
**first** memory regblock, and stock firmware ends the first bank below
`0x40000000` (the VideoCore reserve sits at the top of the first GiB — the
same convention that gives QEMU its 960 MiB), so the triggering frame may
never be allocated on a stock Pi either. The defect stands on the broken
invariant and the live aliases regardless.
**Fix:** delete `vminit.rs:455` (nothing pre-MMU ever follows the pointer —
the walk uses physical addresses directly) and narrow the pre-MMU guard to
level 0 to match `vm.rs`. Deleting line 455 *makes* the texts at
`vm.rs:376-378` and `docs/lessons.md:209-213` true; keep them and add a note
that the invariant now depends on the pre-MMU side not installing
self-pointers, so a future reader doesn't re-introduce them.

**B3. Entering already at EL1 skips FPU enable and inherits unspecified
DAIF. (fable, opus)** — VERIFIED code path; consequence SUSPECTED (QEMU and
Pi firmware both enter above EL1)
`l.S:64-69` branches straight to `el1:` when `CurrentEL == EL1h`, bypassing
the `el2:` block that writes `CPACR_EL1.FPEN` (`l.S:91-93`) and the `eret`
paths that load SPSRs with all DAIF masked (`l.S:76,88`). The target compiles
with `+neon,+fp-armv8`, so `memcpy`/`core::fmt` in `init_vm` may use SIMD
freely; with FP trapped the first FP instruction vectors into the unset
`VBAR_EL1` (B4). `main9` also assumes IRQs are masked until
`irq::unmask_irqs()` (`main.rs:31-41`); a direct-EL1 handoff inherits
whatever the previous stage left.
Related state left at reset on r9's own EL3→EL2 path (`l.S:84-98` writes
only `HCR_EL2.RW`): `CPTR_EL2.TFP` (traps EL1 FP to EL2), `CNTHCTL_EL2` and
`CNTVOFF_EL2` — the last directly relevant given the generic-timer focus in
`AGENTS.md`. QEMU's reset values happen to permit EL1 counter access; the
real armstub sets all of these.
**Fix:** move `msr cpacr_el1` and an explicit `msr daifset, #0xf` into the
`el1:` block so all three entry paths converge; consider the EL2 register
set for the metal path (task 127).

**B4. Every pre-`main9` failure is a silent hang. (all three, in pieces)** —
VERIFIED (opus reproduced: no `-dtb` → just the dot, exit 1)
Five independent properties multiply:
1. **`VBAR_EL1` is not set until `main9`'s first statement**
   (`boot::irq_ops()` → `trap::init()`, the only `msr vbar_el1` in the tree,
   `trap.rs:22`). From reset to there, any exception fetches its vector from
   the reset value (0 on QEMU, UNKNOWN elsewhere); VA 0 is mapped by TTBR0's
   identity block, so on QEMU the CPU *executes whatever is at PA 0* as EL1
   vector code — a wordless wedge, not even a fault.
2. **`init_vm` unwraps the DTB before its first `putstr`** (`vminit.rs:112`
   vs `:115`) — the most common bring-up mistake (missing `-dtb`) dies at the
   unwrap with zero diagnostics.
3. **Pre-console panics print nothing**: `runtime.rs:10-22` prints via
   `iprintln!` → `port::devcons`, whose sink is installed by `devcons::init`
   inside `main9` — the panic handler is a no-op writer in this window
   (opus additionally notes the `&'static Location` and format-statics make
   pre-MMU panics fragile under PIE).
4. **The mini-UART loses its address at MMU-enable**: its MMIO
   (PA `0xfe215040`) is mapped in TTBR1's half at all (TTBR1 maps only
   `[0, 0x2200000)` + the DTB); only TTBR0's 1 GiB identity block covers it,
   and that block is torn down by `vm::switch` in the full kernel — so from
   `l.S:135` to `boot::console` **nothing can print** even if a handler
   tried.

(Refinement from the final gate: a pre-MMU *panic* in a **test** image
already produces a clean exit-1 via `qemu::exit(FAIL)`; what hangs
unconditionally is *exceptions* before `VBAR_EL1` is set (item 1). The
"silent" in the headline is about the output, which is absent in both cases.)
5. **CI discards the only visible channel**: xtask wires the mini-UART to
   `-serial null` (`xtask/src/main.rs:1758-1761`) and inspects exit status
   only — no test can currently observe anything `init_vm` prints.
**Fix:** install a minimal early vector table in `.boottext` before
`init_vm` (print `ESR/FAR/ELR` over the early UART, park); `putstr` before
the DTB unwrap; a pre-MMU panic hook over `init_early_uart_putc`; give the
mini-UART a VA across the MMU window (or document the window); route the
mini-UART to a captured sink in the harness (D0.2).

**B5. "Kernel Text" maps/reserves PA `[0, etext)` — the null page does not
fault and the kernel claims firmware memory. (all three, in pieces)** —
VERIFIED
`vminit.rs:37-39,85-87`: `boottext_physrange = PhysRange::new(base_physaddr()
/* = 0 */, eboottext)`, so "Kernel Text" is PA `0x0..0x200000` (debug build)
and its first 2 MiB block maps `KZERO + 0`. What is *behind* that page is
not kernel code: the LMA is `0x80000`, so PA 0 holds QEMU's machine/boot
data on QEMU and firmware structures on the metal (final gate:
`pmemsave 0 0x100` shows machine data; `l.S`'s first word `0xaa0003fb`
lives at `0x80000`). A kernel-mode null *read* or *jump* through `KZERO + 0`
lands on that memory instead of faulting. Consequences:
- the `TODO leave the first page unmapped to catch null pointer dereferences`
  (`vminit.rs:117`) is violated: a kernel-mode null *read* or *jump* hits the
  mapped page instead of faulting (writes do fault — PrivRo);
- PA `[0, 0x80000)` — below the LMA, firmware territory on the metal — is
  claimed as kernel text in the mapping **and** in `boot::page_allocator`'s
  reserved list;
- in the debug build a full 2 MiB of RAM is permanently reserved for
  non-kernel pages.
**Fix (not one line):** "start the boottext range at `boottext_pa`
(0x80000)" is rejected as stated — the range is mapped with
`PageSize::Page2M` (`vminit.rs:129`) and `map_phys_range` rejects
non-2 MiB-aligned ranges (D20), so `0x80000` panics `init_vm`. And
"leave page 0 unmapped" needs 4 KiB granularity across the first 2 MiB,
which collides with the existing 2 MiB block (`EntryIsNotTable`). B5 is a
mapping-strategy decision (split the first block into 4K leaves, or map
`[0x80000, …)` with a different page size) that must be sequenced **after**
D20's round-vs-reject decision; pin the chosen policy with D3.5.

### Design risks

**D1. `SCTLR_EL1` is read-modify-written over UNKNOWN reset state.
(fable, opus)** — VERIFIED code; consequence SUSPECTED
`l.S:132-135` does `mrs/orr/msr` setting only `M|C|I`. Bits left at their
previous-stage values include `EE` (endianness — if set, everything breaks
silently), `WXN` (if set, the identity block becomes non-executable and the
very next fetch after the MMU write faults), `A/SA/SA0`, `nAA`, `SPAN`,
`UCI`, `DZE`… Several `SCTLR_EL1` fields are architecturally UNKNOWN at
reset; nothing in the EL3/EL2 path initialises SCTLR_EL2 either. Linux's
head.S writes a full value for exactly this reason; QEMU resets benignly.
**Fix:** write a complete known value (RES1 ORed with the wanted bits) in the
EL3→EL1 path, not an OR onto garbage.

**D2. No cache maintenance before the MMU/C/I enable. (fable, opus)** —
VERIFIED as absent; hardware-only consequence
Nothing invalidates the D-cache (`dc isw` sweep) or I-cache (`ic iallu`)
before `l.S:135` enables M|C|I in one write. The page tables were written
with the MMU off (uncached accesses) while `TCR_EL1.IRGN1/ORGN1`
(`vminit.rs:296-304`) program the walker for write-back cacheable accesses;
any line a previous boot stage left in the caches over the tables, the image
or the BSS becomes authoritative. The Pi 4's armstub runs with caches
enabled, so "reset leaves them invalid" is not safe on the target. QEMU
models no caches — no test can ever show this. **Fix:** invalidate D-cache
and `ic iallu` + `dsb`/`isb` before the SCTLR write (validate in the
task-127 Pi session).

**D3. No ISB between the translation-register writes and the MMU enable.
(fable)** — VERIFIED as absent; consequence SUSPECTED
`init_vm` ends with `tlbi vmalle1is; dsb ish` (`vminit.rs:315-321`); `l.S`
then does `dsb sy; mrs sctlr; orr; msr sctlr; isb` (`l.S:131-140`). The
canonical Arm sequence (DEN0024) puts an ISB *after* the TTBR/TCR/MAIR
writes and *before* SCTLR.M=1. The hazard is the window *between*
`msr sctlr_el1` (`l.S:135`) and `isb` (`l.S:140`): in it, instructions may
fetch under the new SCTLR while the TTBR/TCR/MAIR writes are not yet
architecturally visible to the translation machinery — a post-hoc ISB does
not close a window that has already been executed through. QEMU applies
translations eagerly, so it never shows.
**Fix:** match the canonical sequence (`…; isb; msr sctlr, m=1; dsb; isb`).

**D4. The TTBR0 identity block outlives its transition, and its comments say
otherwise. (all three, in pieces)** — VERIFIED
`vminit.rs:187-188`: "Once we've jumped to the higher half, this will no
longer be used." `vminit.rs:283-286`: "…during the transition until the PC is
also in the higher half … Once we enter rust-land, we can define a new set of
tables." Neither is true: l.S never touches TTBR0 after `init_vm`;
`main.rs:47-50` replaces it only in `vm::switch(user_pagetable(), User)`,
which the *full* kernel reaches after irq_ops, DTB parse, page allocator,
mailbox, console and interrupts — and **none of the short test images calls
it at all** (21 of 28 images call `vm::switch`; none of `pagetables`,
`miniuart`, `pl011`, `clock`, `irq`, `timers`, `allocate` do), so they run
to completion with the block live. While live, PA `[0, 1 GiB)` — the whole
kernel image *and* the QEMU DTB at `0x8000000`, the entire low GiB on the
metal — is PrivRw (read/write/EL1-executable) at low VAs: a stray low
pointer reads/writes/`exec`s real memory instead of faulting.
**Fix:** at minimum correct the comments and state the lifetime; better, make
the block PrivRo (the transition only needs EL1 execute) and have the test
images switch TTBR0 to a null root.

**D5. `physicalpt4` (the TTBR0/user root) has no recursive slot. (opus)** —
VERIFIED
`vm.rs:689-701` (`init_empty_root_page_table`) documents the invariant that
every root page table carries a self-pointer at entry 511 (what `replace_recursive_entry` and the recursive
walk rely on). `init_vm` writes it for the kernel root
(`vminit.rs:146-150`) but not for `physicalpt4` (`vminit.rs:208-217`;
opus's dump: `[511]` is zero), yet `physicalpt4` is installed in TTBR0.
Between MMU-enable and `vm::switch`, any `RootPageTableType::User` operation
works on a root that breaks the invariant; a test image that builds an
`Aspace` without switching would. **Fix:** write the slot for `physicalpt4`
too (keeps the invariant universal), or scope the invariant to "the kernel
root" and make `replace_recursive_entry(User, …)` refuse the early root.

**D6. The live kernel page tables are reserved from the allocator only by
accident of section ordering. (all three, in pieces)** — VERIFIED
`kernel.ld:67-70`:

```ld
/* TODO Should this be here???  Better to put into the bss section later */
.end : ALIGN(2097152) {}
PROVIDE(bss_pa  = LOADADDR(.bss));
PROVIDE(ebss_pa = LOADADDR(.end));
```

`ebss_pa` is the LMA of an **empty** section (objdump: `.end` LMA == VMA ==
`0xffff800002200000`; the `LOADADDR` expression currently evaluates to the
physical 0x2200000 — nm verified). `.earlyvm_pagetables` sits *between*
`bss_pa` and `ebss_pa`, so `data_physrange().add(&bss_physrange())` in
`boot::page_allocator` (`boot.rs:36-41`) swallows the early pool — the
**only** reason the allocator doesn't hand out TTBR1's own tables.
Compounding:
- `PhysRange::add` (`core/src/addr.rs:197-199`) is a **hull**
  (min-start/max-end), not a union — the reservation silently covers any gap
  between data and bss, and the fact that they abut is asserted nowhere;
- `kmem.rs` doesn't declare the `earlyvm_*` symbols (only `vminit.rs`'s
  private copy does), so the pool is invisible to the module that owns the
  allocator contract;
- acting on the script's own TODO (move the pool, or change `ebss_pa`)
  *without* an explicit reservation makes the allocator hand out the live
  page tables. Nothing in the tree catches any of it.
**Fix:** reserve the pool by name in `boot::page_allocator` (add the symbols
to `kmem.rs`), stop `ebss_pa` pretending to be "end of bss", assert
adjacency where it's load-bearing (D4.2/D4.3/D4.5 below).

**D7. The QEMU boot contract is fragile, undocumented, and partly a lie.
(pi)** — VERIFIED by experiment
See the "QEMU `raspi4b` contract" above. The repo's `bcm2711-rpi-4-b.dts`
carries `memory@0 { reg = <0x00 0x00 0x00>; }` — a **zero-size** memory node
that only works because QEMU rewrites it; on any machine that doesn't patch,
the page allocator sees zero RAM. The vendor-style filename implies a real
device tree. **Fix:** document the contract (QEMU 11 `raspi4b`: `-dtb`
mandatory, 2 GiB machine RAM, 960 MiB memory node, user dtb otherwise
authoritative); give the dtb a real 2 GiB memory node so the file is
self-consistent; make a missing FDT loud in l.S before parsing (pairs with
B4.2); consider an xtask guard that fails if the assembled command lacks
`-dtb` (D0.5/D7.1).

**D8. MPIDR check masks Aff0 only. (fable, opus)** — VERIFIED; benign on
Pi 4 (single cluster)
`l.S:52-55` masks `0xff` (Aff0): a core with `Aff0==0` in a non-zero cluster
races core 0 through the whole boot path. `AGENTS.md` says the project is
designed for multi-core SMP; masking `0xffff` costs nothing. Separately,
parked cores `wfe`/`b` forever at their entry EL, MMU off, with no release
protocol — a comment should say SMP bring-up needs a different entry.

**D9. KZERO is defined three times, in three languages, with no
cross-check. (opus)** — VERIFIED
`kernel.ld:6`, `l.S:35`, `param.rs:2`. `param.rs:1` even says "This needs to
match KZERO in l.S" — omitting the third copy. All three participate in the
same arithmetic (LMA computation, boot stack, `VaMapping::Offset(KZERO)`); a
mismatch in any one dies silently at `msr sctlr_el1`. **Fix:** assert
equality in a test image (D4.8) and/or derive the constants from one place.

**D10. Boot stack: 16 KiB, no guard, abutting the page-table pool. (fable,
opus)** — VERIFIED layout; overflow SUSPECTED
`l.S:3` `STACKSZ = 4096*4`; the build-independent invariant (verified in
both builds) is **`stack + STACKSZ == earlyvm_pagetables_pa`, exactly, with
zero gap**: the stack's exclusive top *is* the pool's first byte. No guard
page: a 16 KiB overflow silently walks down through `.bss` statics.
Plausible in debug builds — `init_vm` alone takes a ~0x260-byte frame and
the DTB parser recurses. **Fix:** guard page or a bigger stack; D4.9 pins
the current layout.

**D11. The BSS clear loops are do-while: an empty or inverted range clears
memory until wraparound. (fable, opus)** — VERIFIED logic; currently
unreachable (guarded only by linker order, which D6 shows is soft)
`l.S:110-114` and `:116-120` store before comparing. Two leading
`cmp`/`b.eq` instructions make the failure mode unreachable. While here: the
second loop re-zeroes the pool the first just zeroed (D6), and
`EarlyPageAllocator::new` / `alloc_physpage` / `next_mut`'s `page.clear()`
zero it again — the pool is cleared up to five times for one invariant.

**D12. `.earlyvm_pagetables` is emitted as CONTENTS, not NOBITS. (opus)** —
VERIFIED
The 128 KiB pool carries `LOAD, DATA` (objdump: type DATA, no NOBITS), so
`objcopy -O binary` materialises it — the flat image is 33,968,128 bytes,
ending exactly at `eearlyvm_pagetables_pa`. The zeros are redundant (l.S and
the allocator both clear the region) and they make the image size depend on
the pool size. **Fix:** emit it as NOBITS (or at least document the bloat).

**D13. SError (DAIF.A) is masked at reset and never unmasked. (pi)** —
VERIFIED
l.S loads both SPSRs with F|I|A|D; `irq::unmask_irqs` clears only I
(`irq.rs:42`, `DAIFClr, #2`). The vector table has SError handlers that are
never reachable: hardware SError (RAS, uncorrectable memory errors) is
suppressed for the life of the kernel with no log line. **Fix:** unmask A
once the vectors are installed and make the handler ack RAS, or document the
decision to run RAS-off.

**D14. `kmem.rs` and `pre_mmu/vminit.rs` duplicate the `_pa` symbol block and
the range list, and they have drifted. (all three, in pieces)** — VERIFIED
Both declare the `*_pa` externs and range constructors; `vminit` has the
`earlyvm_*` symbols, `kmem` doesn't; `kmem` has `total_kernel_physrange()`,
`vminit` doesn't. The *range list* also exists twice: `init_vm`'s
`custom_map` (`vminit.rs:127-134`) and `boot::page_allocator`'s
`physranges` (`boot.rs:36-42`) must agree or the allocator hands out mapped
kernel memory (or the kernel runs on allocated memory). They agree today;
nothing enforces it. The copy is pure — the pre-MMU module only reads
linker symbols. **Fix:** one home for both (kmem), asserted in tests
(D2.11/D4.4).

**D15. Pre-MMU `next_mut` launders an unbounded lifetime and mints aliasing
`&mut`s. (opus)** — VERIFIED
`vminit.rs:410-415`: `next_mut<'a>(table: &mut Table, …) -> Result<&'a mut
Table, _>` — `'a` is unconstrained by any input. Inside, `&mut *(page_pa…)`
and `&mut *(entry.phys_addr()…)` mint `&mut` from raw addresses while
`table` is still borrowed; `map_to` chains them. Under Rust's aliasing model
these are overlapping mutable borrows of the page-table pool; it works
because the pointers happen to be distinct pages and LLVM isn't currently
exploiting `noalias`. The post-MMU twin ties its lifetime to `&mut self`.
**Fix:** return `*mut Table` (keep the walk raw) or tie the lifetime to
`table`.

**D16. Mini-UART parameters assume one board configuration. (fable, opus)** —
SUSPECTED (QEMU-green)
`util.rs:33-35` hard-codes `UART_CLOCK = 500 MHz`, but the mini-UART's
reference clock is the VPU **core** clock, which `core_freq`, `force_turbo`,
`arm_boost` and thermal throttling all move — the classic Pi symptom is
garbled early boot output. The canonical mitigation (`core_freq_min=500` /
`enable_uart=1` in config.txt) is documented nowhere in `docs/`. Also
`init_early_uart_putc` (`util.rs:83-90`) spins on LSR bit 5 **unbounded**: a
mis-clocked or absent UART hangs the boot at the first `.`. Cross-ref task
127 (Pi 4 bring-up batch).

**D17. Nothing checks that the kernel image does not overlap the DTB.
(opus)** — VERIFIED
The debug image spans `0x80000..0x20e4000` (~33 MiB; `.rodata` alone is
30 MiB) and the QEMU DTB sits at PA `0x8000000` — a quarter of the way
there. If the image ever reaches `0x8000000`, the loader overwrites the DTB
and `DeviceTree::from_usize(…).unwrap()` panics silently (B4). **Fix:**
one-line check in `init_vm` + the D4.6 test.

**D18. Self-pointer entries carry wrong memory attributes *as recursive
leaves*. (fable; final gate resolved a source disagreement)** — VERIFIED
The recursive/self-pointer descriptors (root: `vminit.rs:146-150`;
per-table: `:443-450`) are PrivRw, Normal, `PXN=0` — **executable** — and
non-shareable (the `.with_shareable(Inner)` line is commented out at
`:445`), while the same physical pages are also mapped Inner-shareable via
the kernel data blocks and the post-MMU equivalent uses `rw_kernel_data()`
(Inner, UXN, PXN). Nuance the final gate pinned: `SH`/`AP`/`AttrIndx`/`PXN`/
`UXN` *are* ignored when the descriptor is walked **as a table** — but the
self-pointer is also reached **as a leaf** through the recursive alias, and
there all of them apply. Final-gate dump: root self-pointer
`0x00000000020c5403` → PrivRw, Non-shareable, AF=1, PXN=0, UXN=0, while the
*same physical page* is covered by a Kernel Data 2 MiB block at
SH=Inner. Mismatched shareability for one PA is architecturally
unpredictable for coherency on SMP; missing PXN makes live page tables
executable. **Fix:** build them from the same attribute set as
`rw_kernel_data()` — still required for the root even after B2 removes the
L1/L2/L3 copies.

**D19. EL0 entry falls into the EL3 path. (fable)** — VERIFIED; defensive
`l.S:65-73`: `cmp #1; beq el1; cmp #2; beq el2;` — an EL0 entry falls
through to `msr scr_el3` and UNDEFs with no vectors. A `cmp #0` branch to
the park loop costs one instruction.

**D20. `map_phys_range`'s doc says it rounds; it rejects. (fable, opus)** —
VERIFIED — in both copies
Doc (`vminit.rs:326-329`, verbatim at `vm.rs:523-526`): "This aligns on page
size boundaries, and rounds the requested range…". Code
(`vminit.rs:341-348`, `vm.rs:540-547`): returns
`PhysRangeIsNotOnPageBoundary` for any unaligned range; the
`step_by_rounded` rounding after the gate is dead. Callers must pre-round
(init_vm does, only for the DTB). This matters because 2 MiB alignment of
the mapped ranges is guaranteed only by `. = ALIGN(2097152)` inside `.text`,
the `.rodata`/`.data` alignment, and the empty `.end` section the script
itself questions (D6) — a script change that breaks alignment is swallowed
by B1 and ends as a silent hang. **Fix:** pick a contract — round
(document it) or keep rejecting (fix the doc) — and assert alignment in
tests (D4.7).

**D21. TCR_EL1 walk-attribute fields are correct and load-bearing (draft
error withdrawn). (final gate)** — VERIFIED
A draft of this ticket claimed the TCR ORGN/IRGN fields
(`vminit.rs:296-304`) are redundant because "MAIR takes precedence". That is
wrong: `TCR_EL1.{IRGN0,ORGN0,SH0,IRGN1,ORGN1,SH1}` describe the attributes
of **translation-table-walk accesses** (how the MMU reads the tables);
`MAIR_EL1` describes the attributes of *translated* memory, selected
per-descriptor by `AttrIndx`. They are orthogonal, and deleting the TCR
fields would make table walks non-cacheable — a regression, and a direct
contradiction of D2, which correctly relies on those fields. The only real
observation is that `EPD0/EPD1` are commented out at `vminit.rs:299`; no
action.

**D22. `.bootdata` is lumped into the RO+X "Kernel Text" mapping, and
`.text.boot` claims write permission it doesn't have. (fable)** — VERIFIED
`kernel.ld:14-15` puts `*(.boottext .bootdata)` in `.text.boot`, mapped
`ro_kernel_text()`. Any future bootdata silently becomes
read-only-executable. Same mismatch from the other side: the section is
declared `"awx"` (`l.S:37`) — write permission the mapping does not grant.
**Fix:** separate section (with honest attributes), or an explicit mapping
with the right attributes.

**D23. `"relocation-model": "pie"` with no boot-time relocation processing.
(fable)** — VERIFIED; works by accident
Correct only because link addresses are final and lld pre-fills
relative-reloc targets; this is an accident of linker behaviour, not a stated
contract. Worth a sentence in docs so the next build-system change doesn't
"fix" it into a brick.

### Comment findings (both sides quoted)

**C1. MAIR attr 1 is Device-nGnRE, not nGnRnE. (fable, opus)** — VERIFIED
`vminit.rs:308-312`: `// [1] 0x04 - Device nGnRnE (… no early ack)` but
`0x04` is **Device-nGnRE** (early write ack *permitted*; nGnRnE would be
`0x00`). Every `Mair::Device` mapping (all MMIO) gets nGnRE semantics —
probably the right value (Linux's `pgprot_device` uses it) but a driver
author told "no early ack" may omit a `dsb` before a poll. Fix the comment or
the value — and pin whichever is chosen (D5.6 below).

**C2. All four GPIO mask comments describe the inverse of their masks. (pi,
opus)** — VERIFIED
`util.rs:51` "Mask all but bits 28:29" with `0xcfffffff` (which *clears*
28–29 and keeps 0–27, 30–31); `:56` "Mask all but bits 12:14 (pin 14)" with
`0xffff8fff` (clears 12–14); `:63` "Mask all but bits 30:31" with
`0x3fffffff` (clears 30–31); `:68` "Mask all but bits 15:17 (pin 15)" with
`0xfffc7fff` (clears 15–17). The code is correct (each AND clears exactly
the target field or pull bit; each OR sets ALT5); all four comments describe
the complement.

**C3. `mov x28, x4 // Cache entrypoint (offset)` is a dead store with a wrong
comment. (all three)** — VERIFIED
`l.S:50`: x28 is never read anywhere in l.S, and x4 is not an entrypoint
offset — under the AArch64 boot protocol x4 is end-of-kernel PA; under QEMU
`-kernel` it is whatever the loader left. Delete the store; the header note
"x28: Entrypoint address" goes with it.

**C4. The l.S reserved-register header is inaccurate. (all three)** — VERIFIED
`l.S:40-48` lists x27/x28/x29/x30 ("There's also a couple that are best
avoided out of principle"), but x28 is unused, **x20 is clobbered**
(`l.S:142`, the higher-half jump) and unlisted, and x29/x30 are ordinary ABI
registers here. Typos in the same block: "throught" (`:40`); "be be seen"
(`:130` and `:137`). `vm.rs:101`: "secrtion".

**C5. Dead `mrs x0, elr_el1` in the MMU-enable sequence. (fable, opus)** —
VERIFIED
`l.S:141`: x0 is unconditionally reloaded at `higher_half` before any use.

**C6. Message names a function that does not exist. (opus)** — VERIFIED
`vminit.rs:115` prints "calling init_kernel_page_tables" — the only
occurrence in the tree. Same class: `"failed to alloc root_kernelpt4"`
(`:143`) and "kernelpt4 is the root of the page hierarchy" (`:279-281`); the
variable has been `root_page_table` for some time. (This string is also what
the harness would assert in D1.1 — rename to something real first.)

**C7. "identity map the kernel virtual addresses" describes the offset map.
(fable)** — VERIFIED
`vminit.rs:228`: the TTBR1 mapping is a KZERO-*offset* map; the identity map
is TTBR0's transition block (correctly described at `:278-286`).

**C8. TTBR0 lifetime comments are false. (opus)** — see D4 (quoted there).

**C9. "Only level 0 has the self-pointer" / "every other table clear()ed".
(fable, opus)** — VERIFIED false for the inherited tables — see B2
(`vm.rs:376-378`; `docs/lessons.md:209-213`).

**C10. "recursive address" comments in the pre-MMU module. (opus)** —
VERIFIED
`vminit.rs:461-463` "Return the address of the next table as a recursive
address" and `:375` "root_page_table should be a direct va - not a recursive
va" — there are no VAs and no recursive addressing pre-MMU; the walk uses
physical addresses with the MMU off.

**C11. `ebss_pa` is not the end of `.bss`. (opus)** — VERIFIED — see D6.
Every consumer (l.S, kmem.rs, boot.rs, the boot diagnostics in main.rs) reads
it as "end of bss".

**C12. Level naming: L4–L1 in the essay, Level0–Level3 in the code. (opus)** —
VERIFIED
`vminit.rs:232-247` numbers tables L4 (top) to L1 (leaf); `vm.rs`'s `Level`
enum is the opposite (Level0 = va[47:39]). Anyone reading the essay block
then the code has to invert.

**C13. `irq.rs` copy-pasted target list. (pi)** — VERIFIED
`current_core`'s doc (`irq.rs:91`): "every supported target (Pi 4, QEMU
`q35`/`virt`) is single-cluster" — in this tree `q35` is the x86-64 machine
(`xtask/src/main.rs:1775`) and `virt` is the **riscv64** machine
(`xtask/src/main.rs:1768`); aarch64 runs on `raspi4b`. A draft of this
ticket additionally claimed the mock's `DAIF_IRQ_BIT = 1 << 7` was a
mislabel of the PSTATE bit; that is wrong — as read by `mrs {}, daif` (which
is what `mask_irqs`/`restore_irqs` use, `irq.rs:55-67`) the fields sit at
D=9, A=8, **I=7**, F=6, so `1 << 7` *is* the real I bit and the comment is
accurate. (`#2` at `irq.rs:42` is the different `DAIFSet/Clr` *immediate*
encoding.) Only the target list needs fixing.

**C14. xtask comment names the wrong early console. (pi)** — VERIFIED
`xtask/src/main.rs:754`: "The PL011 (UART0, serial_hd(0)) is the early
console" — inverted: PL011 is the *post-MMU* console (devcons); the
mini-UART (serial_hd(1)) is the pre-MMU one and it goes to `null`. Fix
together with B4.5/D0.2.

**C15. Error strings: no trailing newline, no context. (opus)** — VERIFIED
`vminit.rs:397, 422, 458, 504` — none ends in `\n` (they run into the next
print) and none carries the VA, level or index — the only facts that would
make them actionable.

**C16. `load-address` is an unexplained expression. (opus)** — VERIFIED
`config_default.toml:9-10`: `load-address = '0xffff800000100000 - 0x80000'`
= `KZERO + 0x80000`. Writing it as `KZERO + 0x80000` says what it means; as
written it reads as though 1 MiB matters.

**C17. Stale `.quad`/`PT_*` assembler fragments, commented-out code, and
minor dead items. (pi, fable, opus)** — VERIFIED
`vminit.rs:196-197, 214-215` (`.quad (0*2*GiB) + (PT_BLOCK|…)`, referencing
macros no longer in the tree; the value `0*2*GiB` = 0 matches the entry's PA,
but the implied **2 GiB stride** doesn't match the level-1 1 GiB stride it
annotates), `:178-185` (commented-out `puttable` block), `:445`
(commented-out `.with_shareable`), `util.rs:110-125` (`puttable`, dead). The
long AArch64 granule/tutorial essay at `vminit.rs:232-267` is filler inside a
function. Also: `entry_mut` (`vminit.rs:470-473`, same in `vm.rs:356-359`) is
`pub` in a private module and its `Result` has no `Err` arm; `vm.rs:885-891`
`test_to_use_for_debugging_vaddrs` is an empty test body; `PhysPage4K::clear`
(`vm.rs:55-61`) passes `count = 1` to `volatile_set_memory` over a
`[u8; 4096]` — correct, but reads as a one-byte write.

**C18. `RECURSIVE_SLOT` imported but the literal `511` used at the sites that
matter. (fable, opus)** — VERIFIED
`vminit.rs:146` (the *root* self-pointer — the one that must be right), also
`vm.rs:699, 843, 852`.

### Checked and correct (no action)

- EL3→EL2→EL1 drop: SCR_EL3 (NS|SMD|HCE|RW), SPSR with all DAIF masked,
  HCR_EL2.RW, CPACR_EL1 FPEN (on the el2 path), eret targets — correct.
- KZERO upper-half math: the kernel VA's Level0 index is **256**, so the
  root self-pointer at [511] does not collide with any kernel or DTB mapping
  (DTB at QEMU PA 0x8000000 → L0 256, L2 64). Verified by boot.
- `Entry` bitstruct matches the Arm ARM descriptor layout; AF pre-set on
  every entry (`TCR_EL1.HA == 0` means software must set AF or every access
  takes an Access Flag fault); `ro_kernel_text` PrivRo+X (PXN=0, UXN=1);
  MAIR attr0 `0xff` Normal WB; TCR T0SZ=T1SZ=16, TG=4K, SH=Inner, IPS=44.
- Mapped ranges are 2 MiB/4 KiB aligned in the actual ELF (objdump + nm
  against the debug image); the allocator's used list matches what `init_vm`
  maps today (DTB 4K-rounded; [0, etext); [rodata, erodata);
  [data, ebss)).
- Higher-half jump, stack re-base, semihosting exit (exit 0 pass / 1 fail,
  verified in QEMU), `map_to` replacement semantics, `is_table` checks for
  the levels used, clear-then-publish ordering for new table pages.
- The boot prefix end-to-end: `spawn` image passes under QEMU (prefix
  irq_ops → device_tree → page_allocator → console → interrupts).

## Handoff contract at `bl main9` (l.S:155), as implemented

| State | Value at handoff | Established by | Risk |
|---|---|---|---|
| x0 | `KZERO + dtb_pa`, mapped RO 4K (rounded) | l.S:153-154 | — |
| sp | `stack + STACKSZ` VA; 16 KiB, .bss, RW 2M blocks, **no guard** | l.S:148-150 | D10 |
| CurrentEL / SPSel | EL1h (if entered ≥EL2; otherwise whatever) | l.S:64-99 | B3 |
| SCTLR_EL1 | previous-stage value OR `I\|C\|M` | l.S:132-135 | D1 |
| TTBR1 | kernel root; kernel/DTB + **three stale self-pointer aliases** | vminit.rs:290 | B2 |
| TTBR0 | `physicalpt4`: **1 GiB** PrivRw identity block at PA 0, **no recursive slot**, live until `vm::switch` (never, in short images) | vminit.rs:291 | D4, D5 |
| TCR_EL1 | T0SZ=T1SZ=16, TG=4K, IPS=44, Inner, EPD0/1 on, ORGN/IRGN redundant | vminit.rs:293-306 | D21 |
| MAIR_EL1 | `0x04ff`: attr0 Normal WB, attr1 Device-**nGnRE** (comment says nGnRnE) | vminit.rs:312 | C1 |
| DAIF | all masked **if** entered ≥EL2; unspecified otherwise | l.S:76, 88 | B3, D13 |
| CPACR_EL1 | FPEN **if** entered ≥EL2; trapping otherwise | l.S:91-93 | B3 |
| VBAR_EL1 | **unset (UNKNOWN)** until `boot::irq_ops` in main9 | trap.rs:22, main.rs:20 | B4 |
| Live UART | mini-UART initialised, but **unmapped in both halves** from MMU-on; PL011 untouched; iprint dropped until console init | util.rs, devcons.rs | B4 |
| TLB | `tlbi vmalle1is; dsb ish` pre-enable, `isb` post-enable | vminit.rs:315-321, l.S:140 | D3 |
| Caches | **never invalidated**; C/I enabled over unknown state | l.S:135 | D2 |
| Secondaries | parked `wfe` at their entry EL, MMU off, no release protocol | l.S:52-55, 157-159 | D8 |
| Allocator contract | main9 must reserve exactly init_vm's ranges, plus the pool *by section-ordering accident* | boot.rs:34-44 | D6, D14 |

## Integration-test checklist

Ground rules: a test image is a whole kernel with its own `main9`
(`aarch64/tests/pagetables.rs` is the model), a `[[test]]` entry in
`aarch64/Cargo.toml` (`harness = false`, `required-features =
["qemu-test"]`), `check!` → `qemu::exit(PASS/FAIL)`. Items marked
**[harness]** change xtask, not the kernel. **[metal]** items cannot run
under QEMU at all and belong to the task-127 Pi 4 session.

### D0. Harness capabilities that have to exist first

| # | Change | Unlocks |
|---|---|---|
| D0.1 | Capture QEMU stdout always (not only `--verbose`) and match against a per-image expectations file | all output assertions below |
| D0.2 | Wire the **mini-UART** (serial_hd(1)) to a captured sink instead of `null` (xtask/src/main.rs:1758-1761); fix the PL011 comment (C14) | any pre-MMU output assertion (B4.5) |
| D0.3 | ~~Distinguish "image exited FAIL" from "timed out"~~ **already implemented** (`xtask/src/main.rs:1521-1543`: `Ok(Some(code))` → `FAILED (exit {code})` vs `Ok(None)` → `TIMED OUT after {n}s`) | — |
| D0.4 | Per-image expected *non-zero* exit / expected failure message | negative tests (D7) |
| D0.5 | Per-image QEMU overrides (extra `-machine` opts, omitting `-dtb`, custom dtbs) | D6.2, D7.1, D7.4 |

### D1. Pre-MMU output (needs D0.1+D0.2)

| # | Check | Catches |
|---|---|---|
| D1.1 | Mini-UART emits exactly `.` then the init_vm banner | early UART init broken; l.S ordering (fix the banner name first — C6) |
| D1.2 | The four map lines appear, in ascending `range.start` order, named Kernel Text / Kernel RO Data / Kernel Data / DTB | a range dropped from `custom_map`; sort removed; D14 drift |
| D1.3 | For each line: `end - start` == range size and `va == pa + KZERO` | `VaMapping::Offset` regression; `endva` accounting |
| D1.4 | **No line beginning `error:vminit:` ever appears** | **B1** — highest-value harness check: a swallowed `map_to` failure is otherwise invisible |
| D1.5 | "switching" then "complete", both before any PL011 output | init_vm returning early; console ordering |
| D1.6 | Kernel Text starts at 0x0 (until B5 is fixed) and all ranges 2 MiB/4 KiB aligned as appropriate | **D20/D6** — catches a `.end`/ALIGN change before it becomes a silent unmapped section |
| D1.7 | Output byte-identical across two runs of the same image | non-determinism in init_vm (allocator order, DTB size) |
| D1.8 | Exactly **one** boot banner, no interleaved duplicates | **D8** — the only proposed test for the `Aff0`-only MPIDR mask (D5.10 pins the MPIDR of the core that reached main9; it cannot detect a *second* core that also got there) |

### D2. Page-table structure (in-image, after the boot prefix)

| # | Check | Catches |
|---|---|---|
| D2.1 | **For every table reachable from the kernel root at L1/L2/L3: `entries[511]` does not point at the table's own PA** (enumerate with a visited set / level bound — pre-fix, the stale self-pointers are *cycles* and a naive recursive walk never terminates) | **B2**, directly — write this one first |
| D2.2 | Root `entries[511].phys_addr() == TTBR1_EL1`; every other root entry invalid or pointing elsewhere | recursive slot broken or duplicated |
| D2.3 | Walk `KZERO + 0x3fe00000` (L2 index 511 under L1 0) and assert **no valid entry** | B2's reachable alias, in the exact form `Aspace` would hit |
| D2.4 | Walk `KZERO + 511*GiB` (L1 index 511) and assert no valid entry | B2's L1 alias |
| D2.5 | Map a 4 KiB frame at `KZERO + 0x3fe00000` via `kernel_pagetable().map_phys_range`. **Pre-fix: a dedicated image whose expected result is a timeout/prefetch abort** (needs D0.4) — the leaf write lands on L2[0], the block the executing code and `trap.S`'s vector table are fetched through, and `write_entry`'s `tlbi vmalle1is; dsb ish; isb` makes the image die on the next instruction fetch, so no post-write re-verify can run. **Post-fix: a normal image** where the mapping succeeds and D2.1/D2.2 still hold | **B2's corruption path, executed** (must be driven explicitly — QEMU's 960 MiB means the allocator would never reach that PA) |
| D2.6 | Every L2 kernel block `page_or_table == false`; every L3 leaf `== true` | `map_to`'s conditional block/page bit |
| D2.7 | Kernel text: AP==PrivRo, PXN==false, UXN==true, MAIR==Normal, SH==Inner, AF==true | W^X regression in `ro_kernel_text` |
| D2.8 | RO data blocks: PrivRo, PXN, UXN | rodata becoming executable/writable |
| D2.9 | Data/bss blocks: PrivRw, PXN, UXN | data becoming executable |
| D2.10 | DTB leaves: PrivRo, PXN, UXN, 4K, count == `round_up(dt.size(),4K)/4K` | DTB mapped RW, or short (off-by-one in the rounding) |
| D2.11 | The set of mapped kernel VAs in `[KZERO, KZERO+4GiB)` is exactly the four `custom_map` ranges offset by KZERO — nothing else translates. **Window-sensitive:** only before any `Aspace` exists — `aspace.rs:85-96`/`:163-174` then add 4 KiB `Offset(KZERO)` mappings of arbitrary allocator frames (the B2 trigger); device pages are safe (L0 index 288, outside the window). The boot prefix through `boot::interrupts` creates no `Aspace` | **B2** again, plus any accidental extra mapping; D14 drift |
| D2.12 | A store to a kernel-text VA and to a DTB VA takes a permission fault (scaffolding exists in `aspace_fault.rs`); a call into .data takes an instruction abort | text/DTB really RO in hardware, not just in the descriptor |
| D2.13 | Self-pointer entries (root and, post-fix, the recursive alias) match `rw_kernel_data()` attributes | **D18** |
| D2.14 | Enumerate every table reachable from the root (visited set, see D2.1): phys addrs inside the earlyvm pool; count == expected; pool headroom ≤ N of 32 | silent pool growth toward the exhaustion B1 swallows; wild pointers |

### D3. TTBR0 / identity block / null page

| # | Check | Catches |
|---|---|---|
| D3.1 | At `main9` entry, `TTBR0_EL1 != TTBR1_EL1` and TTBR0 translates VA 0 to PA 0 | documents the D4 window; makes any change visible |
| D3.2 | `physicalpt4.entries[511]` — assert the intended contract (post-fix: self-pointer, per D5) | **D5**; catches `replace_recursive_entry(User, …)` scribbling on the early root |
| D3.3 | After `init_user_page_tables()` + `switch(user_pagetable(), User)`: VA 0 and VA `0x80000` unmapped | the switch taking effect; D4's mitigation |
| D3.4 | An image that *never* switches: assert the low-VA behaviour per the documented policy | D4 for the short test images — explicit rather than accidental |
| D3.5 | `KZERO + 0` is/is not mapped — pin the decision | **B5** and the `vminit.rs:117` TODO; makes acting on it a one-line test change |
| D3.6 | The identity block is `PXN == false` (required for the transition) and `UXN == true`; post-fix, AP per the documented policy | a regression making `higher_half` unreachable |

### D4. Linker layout and symbol contract (cheap in-image asserts on `_pa` symbols)

| # | Check | Catches |
|---|---|---|
| D4.1 | `eearlyvm_pagetables_pa - earlyvm_pagetables_pa == 32 * 4096` | the pool silently shrinking |
| D4.2 | `bss_physrange()` contains `earlyvm_pagetables_pa..eearlyvm_pagetables_pa` | **D6** — the accidental reservation, made explicit |
| D4.3 | `TTBR1_EL1` and `TTBR0_EL1` both fall inside the reserved ranges | D6 from the other side: the live tables are reserved |
| D4.4 | Allocate pages in a loop (bounded) until failure; assert no PA falls in the DTB (4K-rounded) or kernel ranges | **D6/D14** — the allocator handing out mapped memory |
| D4.5 | `boottext.end == text.start`, `text.end == rodata.start`, `rodata.end == data.start`, `data.end == bss.start` — *equality*, not the `<=` used in `pagetables.rs:59-62` (all four equalities hold in the current ELF) | `PhysRange::add`'s hull silently swallowing a gap (D6) |
| D4.6 | `total_kernel_physrange().end <= dtb_pa` | **D17** — the image growing into the DTB |
| D4.7 | `etext_pa`, `rodata_pa`, `erodata_pa`, `data_pa`, `ebss_pa` all 2 MiB-aligned; `eboottext_pa`, `bss_pa`, `earlyvm_pagetables_pa` 4 KiB-aligned | **D20/D6** — an alignment break that B1 would swallow |
| D4.8 | `param::KZERO` matches the linker (D9). **Not** via `(&some_static as usize) - phys(&some_static)`: the only `phys()` in the tree (`kmem.rs:90-99`) is *defined* as `va - param::KZERO` — circular, passes unconditionally. Instead compare a linker-placed symbol's VA against its `_pa` twin (e.g. `&stack as usize` vs `earlyvm_pagetables_pa` arithmetic). The `l.S` copy is not observable at main9: a wrong value there yields a bad pre-MMU SP and the boot dies before main9 (B4) | **D9** — the linker-vs-`param.rs` half of the three-way split |
| D4.9 | The `stack` range doesn't overlap the pool; SP at `main9` entry is inside it. **Boundary:** at the `bl main9` handoff SP *equals* `stack + STACKSZ` (the exclusive end, and the pool's first byte — `stack + STACKSZ == earlyvm_pagetables_pa` exactly in both builds): assert `stack <= sp <= stack + STACKSZ`, or sample after `main9`'s prologue | **D10**; STACKSZ/.balign drift |
| D4.10 | **post-D12** (fails today by construction): the pool contributes no bytes to the flat image — compare `objcopy -O binary` size against `LOADADDR`/`SIZEOF(.earlyvm_pagetables)`; today the image ends at `eearlyvm_pagetables_pa` (`0x20e4000`, 33,964,032 bytes in the current ELF) — **[harness]** | D12's PROGBITS bloat (the draft's "flat-image size == `ebss_pa`" is false: `ebss_pa` is `0x2200000`) |
| D4.11 | `ebss_pa < 0x1_0000_0000` — it must be a physical address, not a KZERO offset (today `ebss_pa == 0x2200000`; a KZERO-valued `LOADADDR(.end)` would be `0xffff800002200000` and would also break D4.7's alignment check in a different, confusing way) | **D6/C11** — the only proposed check for the `.end` LMA fragility |

### D5. `main9` handoff contract (the table above, as assertions)

| # | Check | Catches |
|---|---|---|
| D5.1 | `dtb_va >= KZERO`, the first 4 bytes at `dtb_va` are the big-endian FDT magic 0xd00dfeed, **and the last byte of `dt.size()` is readable** — the last-byte read is what catches a short DTB mapping (the rounding at `vminit.rs:121`), the exact failure B1 currently swallows | x0 clobbering; short mapping; missing `-dtb` (D7) |
| D5.2 | Parsed FDT `totalsize` == `dt.size()`; `dtb_pa + size` inside the memory node's range; the range contains the kernel LMA and dtb_pa; size ≥ 0x200000 (QEMU: == 0x3c000000) | header/size misparse; **D7** — fails on any QEMU that stops patching the node |
| D5.3 | `SCTLR_EL1.{M,C,I}` set and `{EE,WXN}` clear | **D1** — pins the reset-garbage exposure |
| D5.4 | `TCR_EL1` equals the exact value `init_vm` writes (T0SZ, T1SZ, TG0/1, IPS, SH0/1, IRGN0/1, ORGN0/1, EPD0/1) | a TCR field dropped in a refactor |
| D5.5 | `TCR_EL1.IPS <= ID_AA64MMFR0_EL1.PARange` | `IPS::Bits_44` on a part with a smaller PARange — CONSTRAINED UNPREDICTABLE |
| D5.6 | `MAIR_EL1 == 0x04ff` and attr1's Device encoding matches the documented intent (C1's resolution) | **C1** |
| D5.7 | `DAIF`: I and F set at entry (and A, D per D13's decision); I clear after `boot::interrupts()` | B3/D13 ordering |
| D5.8 | `CPACR_EL1.FPEN == 0b11` at entry and an actual FP instruction executes without trapping | **B3** |
| D5.9 | **needs D0.4.** `VBAR_EL1 != 0` and points into kernel text, asserted *before* `boot::irq_ops()` in a dedicated image. Dependency: `check!` reports via `port::println!`, whose sink is installed by `devcons::init` in `boot::console` — three steps *after* `irq_ops` — so without D0.4 a failure is a bare exit-1 with no message | **B4.1** — fails today, which is the point |
| D5.10 | `CurrentEL == EL1h`; `MPIDR_EL1 & 0x00ffffff == 0` (post-D8 fix) | the EL-drop path; **D8** |
| D5.11 | A one-shot timer fires after `boot::interrupts()` (existing `timers` image) | DAIF unmask + GIC + timer ordering end-to-end (regression guard) |

### D6. Robustness of `init_vm` (mostly needs D0.5)

| # | Check | Catches |
|---|---|---|
| D6.1 | An image that deliberately exhausts the `EarlyPageAllocator` fails *loudly* (non-zero exit or an `error:` line), not a hang | **B1 + B4** together |
| D6.2 | Boot with `-dtb` omitted: expect a specific failure signature on the mini-UART, not silence — **[harness]** | **B4.2**; currently produces `.` and nothing else (reproduced) |
| D6.3 | **needs D0.5.** Boot with a hand-padded dtb whose `totalsize` crosses a 2 MiB boundary. The DTB is mapped 4 KiB (`vminit.rs:120,128`), so crossing the boundary needs a **second L2 entry and a second L3 table**, not a second 2 MiB block; QEMU's own dtb is ~111 KiB, so this needs a padded dtb | the DTB `round()` path and the extra table allocations |
| D6.4 | Zero-sized memory node, **`[metal]` / `[host]` only**: no QEMU in the harness can express it — `raspi4b` accepts only 2 GiB and rewrites the node to 960 MiB every time (refuted by experiment; this is why the negative-dtb check must live in a host unit test of `bitmapalloc::free_unused_ranges` with a zero-size available range, or on metal) | **D7** observability: expect a loud failure, not a silent exit 1 |
| D6.5 | `EarlyPageAllocator` high-water mark reported at boot and asserted `< 32` with margin | the pool silently filling as the map grows |
| D6.6 | A second mapping run over a pre-populated table hits `EntryIsNotTable` and reports it | the `is_table` arm of `next_mut` (`vminit.rs:457-459`), currently untested |

### D7. Negative / fault-injection (needs D0.3, D0.4, D0.5)

| # | Check | Catches |
|---|---|---|
| D7.1 | An image that faults *before* `trap::init()` produces a diagnosable failure, not a silent hang | **B4.1**; drives the early-vector-table fix |
| D7.2 | A deliberate kernel-text write, a rodata write, and a jump to a data page each fault with the expected `ESR_EL1` EC | D2.7–D2.9 in hardware terms; extends `aspace_fault.rs` |
| D7.3 | A read of an unmapped kernel VA in the KZERO window faults | **B2/D6** — that the offset map really is sparse |
| D7.4 | Panic inside `init_vm` (feature-gated injection) emits *something* on the mini-UART | **B4.3** |

### D8. Host unit tests (no QEMU; `cargo xtask test`)

| # | Check | Catches |
|---|---|---|
| D8.1 | `va_index` round-trips for each level against known VAs, including index 511 at every level | B2's index arithmetic |
| D8.2 | A host harness for the pre-MMU `map_to`/`next_mut`. **Blocked by more than the raw-pointer deref:** (a) `pre_mmu/mod.rs` declares both modules *private* — neither is reachable from any test; (b) every error path calls `putstr` → `read_volatile(0xfe215040)`, which segfaults in a host build. Needs the diagnostic sink abstracted | **B1** could have been a unit test |
| D8.3 | Pre-MMU `map_phys_range` with a failing allocator returns `Err`, not `Ok`. **Same blockers as D8.2, and worse** — this item exercises *only* error paths, so it would segfault before returning anything | **B1**, once D8.2 exists |
| D8.4 | `PhysRange::add` on non-adjacent ranges — assert the hull semantics explicitly | D6/D4.5 |
| D8.5 | `Entry` bitstruct round-trip against hand-computed constants, using the real descriptor values confirmed live in the guest: `0x0040000000000781` (text 2M block), `0x0060000000200781` (rodata 2M block), `0x0060000008000783` (DTB 4K leaf) | a silent `Entry` field-offset change — the cheapest guard in this section |
| D8.6 | `Entry::is_table` at `Level3` is always false (`vm.rs:214-216`) | the "no sub-pages" invariant the B2 walk depends on |

### Suggested first three images

Most of the above lands in three test images, mirroring the existing ones
(`[[test]]` entry in `aarch64/Cargo.toml`, `harness = false`,
`required-features = ["qemu-test"]`):

1. **`bootcontract.rs`** — D5.1–D5.10, D2.11, D3.1–D3.5: runs only
   `boot::device_tree`, then reads registers and the mapped ranges, then
   exits. No `vm::switch`, so it exercises the exact handoff state.
2. **`earlytables.rs`** — D2.1–D2.6, D2.12–D2.14: walks the live TTBR1 root
   and asserts the table/pool invariants. Needs D0.4 to fail loudly.
3. **`earlyoutput.rs`** — D1.1–D1.8, D7.1: a short image that reaches
   `boot::console`; its harness-side output is the golden file. Needs D0.1
   and D0.2 to exist at all.

Scaffolding for all three images is owned by task 135 (wave 0 — the
harness it builds is what makes two of the three observable):
`earlyoutput.rs` is implemented fully by 135; `bootcontract.rs` lands
with 135's dtb pins and is filled by 137/138/140/141/142; `earlytables.rs`
lands with 138 (its D2.1 pin only passes once 138's fix is in) and is
filled by 136/142/143.

D8.1, D8.4–D8.6 are plain `#[cfg(test)]` unit tests in the `aarch64`
package (D8.2/D8.3 first need the sink abstraction); D4.10, D4.11, D6.1,
D6.3, D7.1 are xtask changes; D6.4's host variant is a `port` unit test.

### Honest gaps: not catchable under QEMU

- D1 (SCTLR reset garbage), D2 (no cache invalidation), D3 (ISB placement),
  D16 (mini-UART clock), D18 (shareability coherency) depend on behaviour
  QEMU does not model: it resets registers benignly, has no caches,
  synchronises translations eagerly, and models a fixed clock. These need the
  task-127 Pi 4 serial-transcript session; the D1/D2/D3/D5 checks above pin
  the *values* so the hardware run has a spec to compare against. A `docs/`
  page should mark this set QEMU-unverifiable so green CI isn't mistaken for
  coverage.
- **B2's end-to-end corruption on stock-firmware Pi hardware:** the
  allocator takes only the first memory bank, which stock firmware ends
  below `0x40000000` (VideoCore reserve — the same convention behind QEMU's
  960 MiB), so the exact triggering frame may never be allocated on a stock
  Pi either. The defect stands on the broken invariant and the live,
  writable, executable aliases (D2.1–D2.5 pin those); proving the full
  corruption needs metal with firmware that maps the top of the first GiB to
  the guest.
- **The `.end` LMA fragility** (D6/C11): `llvm-objdump -h` reports `.end`
  with LMA == VMA == `0xffff800002200000` (the empty section falls out of
  the LMA chain) while `LOADADDR(.end)` resolves to `0x2200000` — the value
  `ebss_pa` takes. D4.10 and D4.11 are the observable consequences.
- **Multi-core pre-MMU** (`l.S` runs per core; the BSS loop and `init_vm`
  assume core 0): no test image brings up a secondary core pre-MMU; the
  bringup path isn't in scope. Flagged, not testable here.

## Triage / suggested fix ordering

1. **Observability first (B4 cluster + D0)**: early vector table, `putstr`
   before the DTB unwrap, pre-MMU panic hook, mini-UART to a captured sink.
   Everything else in this ticket is easier to land and verify with these in
   place, and they turn the entire class of "dead machine, no output"
   failures into one-line diagnoses.
2. **B1** (one-line `?` fix, unblocks D1.4/D8.3) — small, local, high
   value. **B5** lands with step 8: it is a mapping-strategy decision
   (see B5's fix — `0x80000` isn't 2 MiB-aligned and `map_phys_range`
   rejects it, and leaving page 0 unmapped collides with the existing block),
   blocked by D20's round-vs-reject choice; the D3.5 pin can land first.
3. **B2** (delete `vminit.rs:455`, narrow the guard, keep and annotate the two
   doc sites) + D2.1–D2.5 — the only finding that corrupts live kernel state,
   and the only one invisible on QEMU (see B2's reachability note on what is
   and isn't verified about stock-firmware triggering).
4. **D6 + D14 + C11** (allocator reservation by name; one home for the `_pa`
   block and the range list) + D4.1–D4.5 — removes the accidental
   reservation and the duplicated contract at once.
5. **B3, D8, D19** (l.S entry-path convergence: CPACR/DAIF in `el1:`,
   `0xffff` MPIDR mask, EL0 park) — one small patch.
6. **D1, D2, D3, D16** (SCTLR full write, cache invalidation, canonical ISB
   sequence, UART clock docs) — land the code now (it's the canonical
   sequence), validate in the task-127 Pi session.
7. **D4/D5, D10, D12, D17** (TTBR0 lifetime, recursive slot, stack guard,
   NOBITS pool, DTB-overlap check) — layout and lifetime hygiene.
8. **D20 (round-vs-reject) + D13 (SError) — the two recorded decisions** —
   D20 **unblocks B5 and constrains step 4's script change** (the real
   end-of-`.bss` is 4 KiB- but not 2 MiB-aligned, so the alignment
   behaviour decides what `ebss_pa` may become); D2.12/D2.13 and D15 ship
   with it.
9. **Comments (C1–C18) and D15/D18/D21–D23** — fold into whichever patch
   touches the file.
