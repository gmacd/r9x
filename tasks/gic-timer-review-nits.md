---
status: open
---

# Nit sweep from the 46a59c9 review (GICv2 + timer)

Small, independent cleanups from the panel review of 46a59c9. None changes
behaviour except where noted; all should stay warning-free across the three
architectures.

## Status (re-audited 2026-08-27 against fd7e96c)

1. **Done** — no `cfg!(test)` guards remain in timer.rs.
2. **Done** — `find_gicc_gicd_virtranges` is one helper (gic.rs:364)
   called twice via `or_else` (gic.rs:234-235).
3. Open, **shrunk**: the `GicdTyper` half is moot in exactly the
   direction the nit predicted — it is now constructed at gic.rs:245
   (`it_lines_number()` sizes the distributor sweeps; `GICD_TYPER`
   offset exists at gic.rs:44). Only its Debug impl (gic.rs:127-135)
   may still be dead. Still standing: `#[allow(dead_code)]` at
   vmdebug.rs:91 (`print_recursive_tables`), reg/midr_el1.rs:18 (on
   `impl MidrEl1`), and reg/mod.rs:1 (note: it sits on `pub mod
   cnt_el0`, not module-wide on all of reg — the text below
   overstates it); the commented vmdebug calls are now at
   main.rs:93-94.
4. Open, lines drifted — gic.rs:340 "must be passed to `eoi`" and
   gic.rs:393 "pass the value to `eoi`"; the functions are
   `end_interrupt` (gic.rs:347, :421) and `try_ack_interrupt`
   (gic.rs:341, :397).
5. Open — GIC offsets (gic.rs:35-52) carry prose comments and even
   Linux cross-references (gic.rs:60-66, :69-74) but no IHI 0048B
   section numbers. Lower value now that the comments explain
   semantics; stands as written.
6. **Done** — the banked-vs-global story is written down in
   `arm_hardware`'s doc (timer.rs:150-159, which cites this nit), and
   the GIC side in `init_distributor`'s doc.
7. **Done** — resolved by c93cfda's move of the ticker into
   tests/timers.rs: the counter there uses `n < self.limit`.
8. Open, drifted, borderline moot — the print is now at main.rs:49
   inside its own `print_stacks` fn; the tab-aligned neighbours are
   main.rs:35-36, so it no longer sits in a run of aligned prints.
9. Moot, as recorded in item 9 itself (confirmed: no `Lock` in
   gic.rs; `GIC` is a `Once<Gic>` at gic.rs:189).

Remaining work: items 3-rest, 4, 5, 8. Item 4 carries the only
decision (fix docs vs rename to `ack`/`eoi`).

1. **Drop the doubled `cfg!(test)` guards** — `timer.rs:75-85`
   (`timer_enable`/`timer_disable`) wrap `if !cfg!(test)` around
   `CntpCtlEl0` accessors that already no-op under test inside
   `reg/cnt_el0.rs`; `interrupt_handler` (timer.rs:167) adds a third guard.
   The hosted-test concern belongs at one altitude — the reg layer owns it.
   Removing the outer checks likely lets both helpers collapse into their
   call sites.

2. **Deduplicate `find_gicc_gicd_virtranges`** — `gic.rs:221-244` is one
   nine-line pipeline written twice (`.nth(1)` vs `.next()`), each ending in
   a `.map(..).unwrap_or(Err(..))?` Option-to-Result contortion whose error
   string only describes the None case. One helper called twice, using the
   `ok_or` idiom already established in `mailbox.rs:69-75`.

3. **Delete dead code instead of silencing it** — `GicdTyper` plus its Debug
   impl (gic.rs:80-99) are never constructed (no `GICD_TYPER` offset even
   exists); `// test_sysexit();` and commented vmdebug calls in main.rs;
   four new `#[allow(dead_code)]` (main.rs, reg/midr_el1.rs, vmdebug.rs, and
   a module-wide one on reg that also suppresses future dead code). Git
   remembers — delete, and re-add the day something reads GICD_TYPER.
   (When adding INTID validation later, `ITLinesNumber` is what GicdTyper
   was for — see enable_interrupt bounds in the should-fix work.)

4. **Fix doc-comment name rot** — gic.rs:196 says "must be passed to `eoi`"
   and gic.rs:270 references `ack`; the functions are `end_interrupt` and
   `try_ack_interrupt`. Either fix the docs or rename the functions to the
   shorter GIC-manual vocabulary (`ack`/`eoi`) the docs already use.

5. **Cite the spec at the registers** — GIC offsets/fields (gic.rs:23-40)
   carry comments but no document sections, unlike the repo convention
   (mailbox.rs cites its ARM doc). Add ARM IHI 0048B (GICv2) section numbers
   so `Gic::new` can be audited against the manual line by line.

6. **Document the single-core assumption** — CNTP_*, the timer PPI, and the
   GICC CPU interface are per-core banked; `TIMERS`, `GIC`, and the lock are
   global, and `arm_hardware()` arms only the calling core. One sentence in
   the timer and gic module docs marking this as per-core state saves the
   SMP port a debugging session.

7. **Ticker off-by-one** — main.rs:180: `limit = 3` fires four times
   (`n <= limit` plus the "stopping" pass); demo code, but use `n < limit`
   or say "fires limit+1 times" plainly.

8. **Minor symmetry** — main.rs:82 lost its tab separator

9. **`with_gic` helper (moot)** — the nits file mentions adding a `with_gic`
   locking helper to mirror `with_timers` in timer.rs. This is moot now:
   the GIC is no longer behind a `Lock<Option<Gic>>`; the hot path loads an
   `AtomicPtr<Gic>` and calls directly. The locking concern belongs to
   `with_timers` only.
   ("Interrupt stack:{range}") vs the aligned prints around it, looks
   accidental; and the IrqGuard/LockNode/lock triple is spelled out four
   times in gic.rs while timer.rs wraps the identical triple in
   `with_timers` — a `with_gic` helper would make the two subsystems'
   locking look as parallel as it is. (Skip `with_gic` if the
   `Lock<Option<Gic>>` should-fix lands first — the lock disappears there.)

Done when: all eight are applied or individually rejected with a note here;
`cargo xtask clippy` clean on all three arches, `cargo xtask test` passes.

Origin: panel review of 46a59c9 (clarity, simplicity, whole-system,
kernel-taste, microkernel, hardware-truth lenses).
