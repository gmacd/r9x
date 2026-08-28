---
status: done
---

# Collapse the near-identical find_*_physrange helpers

Every aarch64 driver hand-rolls the same device-tree probe pipeline,
differing only by compatible string, reg index and error message:
`find_gpio_physrange` and `find_pl011_physrange`
(`aarch64/src/uartpl011.rs:60,69`); `find_gpio_physrange`,
`find_aux_physrange`, `find_miniuart_physrange`
(`aarch64/src/uartmini.rs:90,101,111`); `find_mbox_physrange`
(`aarch64/src/mailbox.rs:68`). The GIC branch adds two more
(`aarch64/src/gic.rs:224,232`).

Each is roughly five lines: find nodes by compatible, take the node, take
the nth reg block, map to a `PhysRange`, `ok_or` an error string. Once
`range-by-value-sweep.md` makes `PhysRange::from` point-free, the bodies
become textually identical apart from three constants.

Fix direction: one helper in `aarch64/src/deviceutil.rs` — which already
exists for exactly this layer — taking the compatible string, reg index
and error message, returning `Result<PhysRange>`. Replace all eight (ten
after the GIC branch) call sites.

Watch the two `find_gpio_physrange` definitions: `uartmini.rs:90` and
`uartpl011.rs:60` are separate copies of the same probe and should
collapse to one call, not two.

Done when: no driver defines its own `find_*_physrange`; the shared helper
lives in `deviceutil.rs`; gates clean on all three architectures.

Sequencing: after `range-by-value-sweep.md`.

Origin: plan `tasks/plans/range-by-value.md`, task 5 (clarity lens —
the knock-on the sweep actually unlocks, which the draft's "nothing new is
built" would have foreclosed).
