# Open tasks

Ordered by priority. Done tasks are in [done.md](done.md).

Audited 2026-08-27 against the tree at `fd7e96c`, cross-checked against
Linux, Plan 9, seL4, Zircon, and Minix 3. Tasks 89, 70, 71 were found
already done and filed; task 87 was rediagnosed (see its file); tasks
92–98 were opened from audit findings.

Reviewed 2026-08-27 (later, tree at `d2cb62b`): 96, 90b, 90c landed and
filed in [done.md](done.md); 88's server half landed (9b3920a) — the
`r9x_std::console` client API is what remains.

88 then completed (the task-88 build): the `r9x_std::console` client API,
the `display` verdict via `console::write`, and the `two_clients`
serialisation test — filed in [done.md](done.md). Its build surfaced task 101
(display server nameserver-handle form).

**Architecture and correctness review 2026-08-28, tree at `f76d96a`** —
design doc [plans/architecture-review-2026-08.md](plans/architecture-review-2026-08.md).
Opened 102–127. §1 was empty and is now the largest section: the review
found ten defects that are silent corruption, an EL0 escape, or an
undiagnosable hang. Two rulings were taken and the tasks branch on them:

- **`SYS_MAP_MMIO` is to be gated / permissioned.** Task 99 unparks and
  moves to §2 as the validation half of task 120.
- **Multi-core is imminent.** Every SMP-latent race is filed at its true
  severity, and task 124 (bring the secondaries up) is wave 1 rather
  than a deferral — you cannot validate concurrent code on one core, and
  that is how a project with an explicit SMP charter accumulated a dozen
  races invisible to its own test suite.

Also corrected in that pass: `r9-mailbox-unsafe-safety.md` claimed
"Task 101", colliding with `display-ns-handle-form.md`; its header now
matches its index entry (task 13).

<!-- xtask:tasks begin -->
## 1. Correctness — the kernel is wrong today

Landing order is wave order, per the design doc. 102–108 are wave 0
(ground truth): every measurement taken before they land is measuring
through them.

104. [vm-recursive-index-guard.md](vm-recursive-index-guard.md) —
    `next_mut` is missing the `index == 511` guard its pre-MMU twin has,
    so a user-chosen VA writes a leaf into an L2 slot as a *table*
    descriptor. _Arbitrary physical read/write from EL0. The guard
    already exists twenty lines away in this repo._
105. [vm-table-publish-and-barriers.md](vm-table-publish-and-barriers.md) —
    table pages published before they are cleared (and pages are not
    zeroed), no `dsb ishst` before the TLBI, no break-before-make.
    _`vminit.rs` gets the publish order right; `vm.rs` inverts it._
106. [quickfit-alloc-arg-swap.md](quickfit-alloc-arg-swap.md) —
    `alloc_tail` passes `(size, align)` to a `(align, size)` signature;
    a 64 KB request returns a one-byte block. _Host-testable._
107. [bitmapalloc-arithmetic.md](bitmapalloc-arithmetic.md) —
    `deallocate` clears the wrong bit, plus three neighbouring
    off-by-ones. _Latent only because nothing calls `deallocate` yet;
    fold in task 9, same function._
108. [mcslock-unlock-weak-cas.md](mcslock-unlock-weak-cas.md) —
    `compare_exchange_weak` in `unlock` with no retry loop; one spurious
    failure holds the lock forever. _Land with task 97, whose loom tests
    are what catch it._
109. [proc-table-lock-irq-discipline.md](proc-table-lock-irq-discipline.md) —
    the tick takes `TABLE` in interrupt context while seven syscall
    paths hold it with IRQs unmasked. _The invariant is written down at
    `process.rs:24-27` and the code stopped honouring it._
110. [ipc-waiter-slot-protocol.md](ipc-waiter-slot-protocol.md) — single
    waiter slots strand senders; `send` can report success without
    enqueuing; `receive_at` leaves a stale waiter. _Two halves reachable
    today; the rest is subsumed by 118._
111. [proc-heap-hwm-and-alloc-page.md](proc-heap-hwm-and-alloc-page.md) —
    `heap_alloc_page` returns a VA that is not the page it mapped, and
    both heap paths rewind the watermark. _The mailbox server DMAs to an
    unrelated physical page._
112. [proc-kill-exit-slot-identity.md](proc-kill-exit-slot-identity.md) —
    `sys_kill` labels a Running process Exited without stopping it, and
    the slot is then reused under it; `exit_current` matches the first
    Running slot rather than TPIDR's.
113. [channel-close-ownership.md](channel-close-ownership.md) —
    `close_all_for` closes channels the dying process merely blocked
    *on*, so one dead client permanently bricks the nameserver.
    _Subsumed by 119's ownership model; the `owner` half can land early._
114. [server-input-hardening.md](server-input-hardening.md) — every
    server indexes its payload before length-checking it and none
    handles a closed channel. _Any client kills the nameserver with a
    one-byte message; the panic handler exits 0 so it looks clean.
    Prerequisite for 98._
115. [parser-bounds-hardening.md](parser-bounds-hardening.md) — FDT
    header offsets, ELF section bounds, and a backtrace empty-slice
    panic *inside the fault handler*. _Same fix shape, all host-testable._
116. [user-range-ok-va-aliasing.md](user-range-ok-va-aliasing.md) —
    `user_range_ok` bounds at `KZERO` not 2^48 and masks the high bits,
    so a high VA validates against a different low one. _User-triggerable
    kernel data abort._
117. [switch-out-publish-before-save.md](switch-out-publish-before-save.md) —
    the outgoing process is advertised Runnable before `swtch` saves its
    context. _Two cores on one kstack; reachable the moment 124 lands._

## 2. Architecture — the microkernel primitives

Design: [plans/architecture-review-2026-08.md](plans/architecture-review-2026-08.md).
Landing order 124 → 126 → 118 → 119 → 99+120 → 121 → 122 → 123 → 125.
118 and 119 both rewrite the IPC core; everything after depends on them,
and the cost of deferring either is linear in a server count that only
grows.

124. [smp-bringup.md](smp-bringup.md) — unpark the secondaries and add an
    SMP soak image **first**, then make the scheduler per-CPU. _Wave 1
    and deliberately small: until a second core runs, every race in §1 is
    invisible to the test suite. Expect it to fail immediately; that is
    the deliverable._
126. [abi-error-value-space.md](abi-error-value-space.md) — the error
    space overlaps the value space (`ERR_NO_SLOT = 8` is also handle 8;
    `RECEIVE_CLOSED = 2` collides with two protocol verbs). _Cheap now,
    viral once 119 densifies handles and 122 adds a protocol space._
118. [ipc-kernel-side-reply.md](ipc-kernel-side-reply.md) — reply becomes
    a kernel verb (QNX receive ids) instead of a handle carried in the
    payload. _Highest leverage on the list: it rewrites every server's
    wire format, and halves the channel budget that is currently exactly
    at the limit._
119. [handle-capabilities.md](handle-capabilities.md) — per-process
    handle tables with rights, refcounted channels, explicit handle
    transfer. _Hard prerequisite for 123: per-process namespaces are
    impossible while a handle is a global integer._
99. [r9-mmio-pa-validation.md](r9-mmio-pa-validation.md) — validate
    `SYS_MAP_MMIO` PAs against DT-derived device ranges. _**Unparked**
    by the 2026-08-28 ruling; land as the validation half of 120._
120. [device-capabilities.md](device-capabilities.md) — gate MMIO behind
    a device capability and hand drivers their FDT subtree instead of
    hardcoded physical addresses. _The authorization half 99 deferred.
    The "device-dumb, the QNX model" comment is now wrong, not merely
    incomplete._
121. [ipc-message-transport.md](ipc-message-transport.md) —
    variable-size payloads and a direct sender-to-receiver copy. _The
    console cannot atomically write a line over 252 bytes today, and 122
    has nothing to negotiate for `msize` until this lands._
122. [ninep-server-protocol.md](ninep-server-protocol.md) — 9P as the
    server protocol. _`OP_BIND`, `OP_WRITE`, `OP_CONFIGURE_FB` and
    `R_OK` are all 0. Every ad-hoc protocol is rewritten when 9P lands;
    land the codec once, here, and unpark 78 onto it._
123. [per-process-namespaces.md](per-process-namespaces.md) — the Plan 9
    half the project is named after. _Today: one global bind table, one
    view for everyone._
125. [threads-in-address-space.md](threads-in-address-space.md) — a
    thread is currently a whole process, so every server is a serialized
    state machine. _Write the constraint into `AGENTS.md` now: it reads
    as a design choice and is an accident, and 122's dispatch loop is
    being designed against it._

## 3. User-space — making user programs work

101. [display-ns-handle-form.md](display-ns-handle-form.md) — the display
    server reads its nameserver handles from the extra fields
    (`handle_at(2)`) but the image spawns it with them in the main
    fields; it works only because channel 0 == `ns_in`. Spawn it in the
    extra-field (for-server) form. _Spun off task 88 (the console client
    API); the new servers use the correct form._

_(88 `r9-console-server` — the `r9x_std::console` client API — landed in the
task-88 build: `write`/`println`/`reply_channel`, the `display` verdict via
`console::write`, and the `two_clients` serialisation test. Filed in
done.md.)_

## 4. Gates & hardening — build/test infrastructure

Design: [plans/gates-hardening.md](plans/gates-hardening.md) (premises
refreshed 2026-08-27). Landing order per the audit: 46 → 50 → 47 → 45
→ 51 → 49; 48 is demoted to fold-into-46 or drop.

128. [kb-check-gate.md](kb-check-gate.md) — `cargo xtask kb --check`:
    report `docs/` pages whose `covers:` code moved since their
    `verified:` commit, plus dangling paths and malformed headers.
    Reports, never blocks. _The convention exists and nothing enforces
    it; `HowItWorks.md` is what that looks like after a year._
46. [gate-frame-offsets.md](gate-frame-offsets.md) — frame-offset
    single-sourcing via build.rs `.equ` prelude. _Highest value: the
    disease is live — trap.S's comment says 288-byte frame, the code
    says 304._
47. [gate-typos.md](gate-typos.md) — crate-ci/typos + register-name
    ignore. _trap.S:47 "availalble" is the acceptance evidence._
48. [gate-unsafe-ratchet.md](gate-unsafe-ratchet.md) — `unsafe`
    ratchet. _Scope extended to abi/core/std/cmd; census refreshed._
49. [gate-symbol-manifest.md](gate-symbol-manifest.md) — structural
    post-link assertions. _Rewritten around linker-script `ASSERT()`
    (the Linux vmlinux.lds.S mechanism); llvm-nm only for `st_size`._
50. [gate-drift-watch.md](gate-drift-watch.md) — weekly nightly-vs-`ci`
    cron. _The pin already moved once since the plan was written._
51. [gate-miri.md](gate-miri.md) — `cargo miri test -p port`. _Only
    worth its slot with task 97's lock/allocator tests in place._
52. [mcslock-loom-tests.md](mcslock-loom-tests.md) — host tests for the
    MCS lock and allocator (loom or miri many-seeds). _The SMP charter
    has no concurrency gate at all today; mcslock.rs has zero tests.
    **Promoted**: task 108 is a live weak-CAS bug these tests are the
    natural detector for; land the two together._
53. [gate-assemble.md](gate-assemble.md) — assemble gate. _Demoted:
    zero new coverage, and the server-staging build.rs erodes the speed
    win; fold into 46's landing or drop._
54. [r9-vm-coverage-test.md](r9-vm-coverage-test.md) — full VM
    integration test. _Rewritten 2026-08-27: the old row-5 note had the
    ARM table-descriptor encoding inverted; matrix extended with W^X,
    live-permission, BBM, SMP, and error-path rows. Task 96 landed
    without its store-to-text kill test — the W^X row here is now the
    only planned pin of the `ro_user_text` encoding. **The BBM and SMP
    rows are tasks 105 and 124's acceptance evidence.**_

## 5. Cleanup — independent, no design debate

8. [timer-callback-context-contract.md](timer-callback-context-contract.md) —
   document TimerCallback's IRQ-context contract. Docs-only. _The
   contract is already decided in practice (the scheduler tick); write
   it down before more `fire` implementors appear._
9. [mark-range-check-end-flag.md](mark-range-check-end-flag.md) — delete
   `mark_range`'s `check_end` control flag. _Fold into task 107: same
   function, and 107 changes its rounding._
10. [physrange-add-rename.md](physrange-add-rename.md) — `PhysRange::add`
    → `span`. _Types moved to `core/src/addr.rs`._
11. [physrange-with-end-test-only.md](physrange-with-end-test-only.md) —
    delete `PhysRange::with_end`. _The `#[cfg(test)]` option broke when
    the type moved to `r9x-core`._
12. [gic-timer-review-nits.md](gic-timer-review-nits.md) — nit sweep;
    status re-audited 2026-08-27 (items 1, 2, 6, 7, 9 done/moot; 3-rest,
    4, 5, 8 stand). Do last.
13. [r9-mailbox-unsafe-safety.md](r9-mailbox-unsafe-safety.md) — the
    mailbox server has ten `unsafe` blocks and zero `// SAFETY:` proofs
    (the worst of `cmd/*`). The Device-buffer change (task 87) made their
    proof load-bearing. Add `// SAFETY:` at each site.
130. [gicd-typer-bitfields.md](gicd-typer-bitfields.md) —
    `it_lines_number`/`cpu_number` read the wrong GICD_TYPER bit ranges
    (IHI 0048B.b Table 4-6); on the GIC-400 `cpu_number()` returns 7
    where the register says 3. _Unused today, but it is the field SMP
    bringup (task 124) will reach for._
## 6. Parked — deliberate deferrals

78. [r9x-std-servers.md](r9x-std-servers.md) — 9P client over
    `r9x_std::ipc`. Gated on fs/dev/net servers landing. _Notes added:
    9P message set over channels (no size[4] framing), msize vs
    MSG_MAX=256 decision, build on 88's console client. **Unpark with
    task 122**, which needs the same codec for the server side._
79. [r9x-deps-trim.md](r9x-deps-trim.md) — trim external crates.
    _Audit corrected: bitstruct is 15 invocations across 7 files, not
    "22 lines"; consider tock-registers for the register bitfields._
80. [timer-table-portability.md](timer-table-portability.md) — lift
    timer table into `port/`. Do at architecture #2 (natural trigger:
    74b), not before.
81. [stage7-init-supervises.md](stage7-init-supervises.md) — servers
    move from kernel hard-start to init-spawned with restart-on-death
    (the Minix-3 reincarnation shape). Successor to the done task 70.
    _Filed as task 98. **Gated on task 114**: supervision is pointless
    while the panic handler exits 0 and servers busy-spin on a closed
    channel rather than dying._
83. [r9-syscall-spawn-port.md](r9-syscall-spawn-port.md) — riscv64 /
    x86-64 port of `SYS_SPAWN` and the process/IPC stack under it. Not
    mechanical. _The aarch64 half is done (`done/r9-syscall-spawn.md`)._
127. [pi4-hardware-correctness.md](pi4-hardware-correctness.md) — six
    defects that only bite off QEMU: `COUNTER_FREQ` hardcoded to 1 GHz
    (54 MHz on a Pi 4, so the display paces at ~3 fps), the VideoCore
    bus address unmasked, the framebuffer size and pitch ignored, EOI
    before deassertion storming a level-triggered SPI, `sys_irq_claim`'s
    publish order, and the 4 GiB allocator cap. _Batch behind one
    hardware bring-up session; also deletes f76d96a's dead `FB_PHYS`._
<!-- xtask:tasks end -->
