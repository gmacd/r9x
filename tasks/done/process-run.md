---
status: done
---

# aarch64::process — run the first user process to its exit

Module `aarch64/src/process.rs` (plan: plans/first-user-process.md):
trap-svc-exit already landed its first half — `SYSEXIT` (0), the
kernel-return-context and exit-status statics, and the accessors; this
task adds `run`, composing what `aarch64/tests/user_process.rs`
already does step by step:

```rust
pub const SYSEXIT: u64 = 0;   // svc #0; n = ESR_EL1.ISS

pub fn run(text: &[u8], text_va: usize, stack_va: usize) -> u64
```

- Preconditions (documented, not checked): TTBR0 is the user table and
  interrupts are fully brought up.
- Allocate the text page at `text_va` and the stack page at `stack_va`
  in the user table via the existing
  `pagealloc::allocate_virtpage` (`Entry::rw_user_text` /
  `rw_user_data`, `VaMapping::Addr`); write `text` into the text page
  (the same mapping the process will execute through — same VA and
  translation at EL1 and EL0, so no cache maintenance is needed; the
  test image's read-back check proves it).
- Build the `Context` at the top of the stack page (assert it fits, as
  the test does): x30 = `text_va`, sp = the context's own address,
  spsr = 0 (EL0, SP0, DAIF unmasked, IL = 0).
- The module's kernel side is `KERNEL_SLOT`, a `*mut Context` static
  that is `swtch`'s `from` argument itself: the switch writes the
  saved kernel context's address into it, so nothing else sets it
  (the plan's separate `kernel_return_ctx` + setter collapsed into
  this; the exit trap reads the slot, `run` clears it on resumption).
  Plus `EXIT_STATUS`; SAFETY comments on the accessors, single-core
  by the l.S gate; call `swtch`; on return, read the status and clear
  the slot.

No `Process`/PCB struct, no proc table, no scheduler: with one process
and one caller, two module statics are the whole state (panel decision
3). Nothing here is panic-capable from trap context; `run` itself may
panic on allocation failure (init-only callers, plan's failure
policy).

Day-one callers: the `user_process` test image (replacing its inline
setup) and, in the following task, `main9`.

Done when: the module exists with the documented preconditions; the
test image calls `run` instead of inlining the setup; gates clean on
all three arches (the module is aarch64-crate-local).

Origin: plans/first-user-process.md.

## Done (0edcfa9, first-user-process branch)

`aarch64/src/process.rs` gained `run` exactly as specced: the two
`allocate_virtpage` calls (rw_user_text / rw_user_data, VaMapping::Addr),
the fits-in-the-stack-page assert, the context built at the top of the
stack page with x30 = text_va, sp = the context's own address, spsr = 0,
then `swtch(kernel_slot_addr(), context, SPSR_EL1H)`; on return it reads
EXIT_STATUS and clears KERNEL_SLOT.  Preconditions (TTBR0 live, interrupts
up) and the panic-on-allocation-failure policy are documented on the fn.
The `user_process` test image calls `run` instead of its inline setup
(the read-back check kept), and main9 became the second caller in the
following task (6b36a76).  Host builds get a cfg'd stub so `port` unit
tests still link.  Gates clean on all three arches.
