---
status: done
---

# swtch: stop saving spsr_el1; take the resume state from the caller; give it a real wrapper

`swtch.S`'s save side does `mrs x5, spsr_el1` and stores that as the
context's `spsr`. `SPSR_EL1` at EL1 is architecturally uninitialized
until an exception happens to write it, and the hardware then clobbers
it on **every** trap taken to EL1 with the *trapped* context's state
(Arm ARM DDI 0487, exception model / SPSR_EL1 register description).
Nothing in r9 ever writes `spsr_el1` at EL1 (l.S only sets the EL3/EL2
ones), so the first `swtch` saves boot garbage, and after an EL0
exception it would save EL0 state into the "kernel" context — a later
switch-back would `eret` the kernel into EL0.

The AArch32 original's fix — reading the current state — is not
available: AArch64 has no CPSR read (`mrs x5, cpsr` does not assemble;
the first build attempt failed here). The state is known to the
**caller**, so it supplies it:

- `swtch` gains a third argument, `spsr`, recorded as the saved
  context's `spsr`; the switch-in side is unchanged (it already loads
  the *to*-context's own `spsr`).
- `swtch.rs` defines `SPSR_EL1H: u64 = 0x7` (EL1 with SP_EL1, DAIF
  unmasked, IL = 0 — the CPSR state encoding, Arm ARM DDI 0487);
  kernel callers pass it, ORing in the DAIF bits (0x3c0: D=1<<9,
  A=1<<8, I=1<<7, F=1<<6) when masking at switch-out.
- Add the `pub(crate) unsafe fn swtch` wrapper (the bare extern is
  renamed `swtch_asm` with `#[link_name = "swtch"]`); both are
  `#[cfg(target_os = "none")]` like the `global_asm` they belong to.
- Document the protocol on the wrapper with a SAFETY comment: `from`
  receives the address of the context just saved on the caller's stack
  (the caller's frame must stay live until the switch-back); `to` must
  be a context from a prior save or a starter; the call does not return
  until that context is switched back to; only x19–x30, the stack, and
  the EL/DAIF state are preserved across the switch (AArch64 PCS).
- The wrapper keeps `#[expect(dead_code)]` until process-run lands the
  first caller (prompt to drop it and make the fn `pub`).

Done when: the save side records the caller's `spsr`; the wrapper,
`SPSR_EL1H`, and the SAFETY/protocol docs exist; `cargo xtask ci` is
clean (the asm change is aarch64-crate-local; x86-64 and riscv64
untouched in the diff).

Not proven yet: the switch has no caller, so the new save side has been
assembled and linked but never executed; the first execution comes with
user-process-switch-test.

Origin: plans/first-user-process.md — finding #1; the switch must be
trustworthy before anything stands on it.

## Status: done

Landed as the first commit of the first-user-process branch.  Note the
divergence from the plan, recorded there: the plan specified
`mrs x5, cpsr`, which does not exist in AArch64 (the first build
failed exactly there); the fix that landed is the caller-supplied
`spsr` argument with `SPSR_EL1H` (0x7) and the DAIF mask 0x3c0.
`cargo xtask ci` green; the save side is assembled and linked but not
yet executed (no caller until user-process-switch-test).
