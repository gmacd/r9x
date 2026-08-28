---
status: done
---

# Export enable_interrupt; move timer knowledge out of Gic::new

Per-core bring-up (`Gic::init_cpu`, gic.rs:320-344 — written by
gic-init-ordering, which split bring-up out of `Gic::new`) hardcodes
`self.enable_interrupt(self.timer_intid)` into generic controller bring-up,
while `enable_interrupt` (gic.rs:361) has no pub wrapper although
`disable_interrupt` has one (gic.rs:411) — an asymmetric API (outsiders may
disable any interrupt but never enable one) that is exactly what forced the
hardwiring. The interrupt controller driver currently knows about, and acts
for, its client's device; the next interrupt-driven device (UART rx is the
obvious one) must either edit `init_cpu` or flip the API.

Fix: add a pub `enable_interrupt(intid)` alongside the existing pub
`disable_interrupt`; delete the timer-PPI enable from `init_cpu`; have
`timer::init` claim its own INTID, so the `timer_intid` field and the
`gic::timer_intid()` accessor move out of gic.rs into timer.rs. Consider
bounding intid against ITLinesNumber when validation lands (the once-dead
GicdTyper's purpose — see gic-timer-review-nits.md item 3).

Update: gic-init-ordering.md has landed; the hardcoded enable moved from
`Gic::new` to `init_cpu`, and `timer::init` currently only disables the
hardware (`timer_disable`) — it has no claim on the INTID to take.

Update 2026-08-22: done/timer-intid-source.md (a186968) has since resolved
the derivation half of this — the `TIMER_INTID` constant is gone, `Gic::new`
parses the INTID from the devicetree (`timer_intid_from_dt`) and exposes it
through `gic::timer_intid()`, and `init_cpu` hardcodes the enable against
that field. What remains is the API asymmetry and the controller owning its
client's line.

Interacts with: gic-init-ordering.md — timer::init enabling its line is
naturally the post-publish step that closes the bringup window.

Done when: gic.rs contains no device-specific knowledge; enable/disable
are symmetric pub API; timer::init requests its own line.

Origin: panel review of 46a59c9 (kernel-taste, clarity, whole-system,
simplicity lenses — 4-way convergence).
