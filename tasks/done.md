# Done

All completed tasks. Task files in `done/` where they exist.

## Kernel correctness (tier 1)

- **fdt-u32-iter-length** — `property_value_as_u32_iter` stops at the property's `value_end`; trailing partial cell dropped. Hand-built 6-byte-value DTB in `core/tests/fdt_test.rs`.
- **regblock-zero-len-panic** — reject at probe; `PhysRange::from_regblock` is the fallible path.
- **pl011-gpio-range-unused** — `gpiosetpull` through `gpio_virtrange`; dead_code allows gone.
- **timer-periodicity-encoding** — handler branches on `repeat`; 1→7 unit tests.
- **interrupt-depth-per-core** — per-core `[AtomicUsize; MAX_CPUS]` by MPIDR Aff0.
- **timer-smp-double-fire** — CAS on deadline, not a load.

## Decisions (tier 2)

- **timer-intid-source** — parsed from DT (`gic::timer_intid_from_dt`).
- **no-gic-target-policy** — one loud boot panic; Pi 3 out of scope.

## GIC/timer hardening (tier 3)

- **gic-dist-init-hardening** — sweeps ICENABLER/ICPENDR/IPRIORITYR before enabling.
- **gic-enable-interrupt-api** — `gic::enable_interrupt(intid)`.
- **timer-start-result** — `register`/`start` return the error.

## First user process (tier 5)

- **process-run** — `process::run` composes page setup, context, switch, resumption.
- **user-process-switch-test** — fixed mirrored saved context, SPSR_EL1H = 0x7.
- **main9-run-first-process** — kernel starts the first process.

## Preemption (tier 6)

- **svc-yield** — `svc #1` returns to the process.
- **proc-table** — PCB, proc table, frame, kstack vector, TPIDR, forkret, `resched()`.
- **tick-preemption** — tick sets flag; switch at IRQ tail after CVAL re-arm + EOI.

## Microkernel substrate (tier 9)

- **microkernel-priority-pi** — priority scheduling, `boost`/`unboost`.
- **microkernel-ipc-core** — `port::ipc`: Channel, Message, PI. SYCSEND/SYCRECEIVE/SYCREPLY.
- **aspace-struct** — per-process `Aspace`.
- **aspace-switch** — `spawn` maps into own Aspace; TTBR0 on switch.
- **aspace-fault** — EL0 fault path; FAULT_STATUS kill.
- **irq-route** — IRQ routing table, `try_send`, `SYSIRQCLAIM`.
- **irq-integration** — SPI routing integration image.
- **console-mmapmmio** — `SYS_MAP_MMIO`.
- **console-server** — stage 5 console server (one-shot test).

## User binaries (tier 10)

- **elf-reader-port** — `port::elf`.
- **process-spawn-elf** — `Image` enum, `spawn_elf`, ~19 raw sites migrated.
- **server-console-package** — `cmd/console` workspace member, xtask `ServerStep`.
- **console-server-elf-image** — ELF embedding via build.rs.

## Stage 6: nameserver + namespace

- **stage6-createchan-syscall** — `SYCCREATECHAN` (21).
- **stage6-nameserver-server** — BindTable, BIND/RESOLVE/UNBIND loop.
- **stage6-console-publishes** — console BINDs `/dev/console`.
- **stage6-namespace-test** — namespace integration image.

## r9x target (tier 11)

- **r9x-foundation** — three target crates, specs, servers in `cmd/`.
- **r9-syscall-heap** — `SYS_ALLOC`/`SYS_FREE`.
- **r9-syscall-spawn** — `SYS_SPAWN` (aarch64), image registry.
- **r9-syscall-clock-wait** — `SYS_CLOCK`, `SYS_RECEIVE_AT`, deadline mechanism.
- **r9-syscall-proc-control** — `SYS_WAIT`, `SYS_KILL`.
- **r9-syscall-sched** — `SYS_SETPRIO`, `Priority` type.
- **r9-display-server** — `cmd/display/`: framebuffer, frame loop.
- **r9-qemu-display** — Mailbox, framebuffer at `0x3c100000`.
- **r9-mailbox-server** — `cmd/mailbox/`; `SYS_FB_CONFIGURE` retired.
- **r9-systrace** — compile-gated syscall trace.
- **r9-ipc-test-coverage** — 4 new IPC unit tests (18→22, 55 total port).
- **r9-addr-types-core** — `PhysAddr`/`PhysRange`/`VirtRange` to `r9x-core`.
- **r9-nameserver-channel-race** — per-client reply channels; MAIR Attr1 fix;
  Mailbox register offsets; QEMU machine `virt`→`raspi4b`.
- **r9-mailbox-mmio-fix** (task 87) — `cmd/mailbox` mapped the wrong PA
  (`0xFE00_0000`, a raspi4b hole → synchronous external abort, DFSC 0x10) with
  the wrong register layout. Fixed to page `0xFE00B000` / regs +0x880 /
  STATUS +0x18, and — the silent second blocker — the request buffer is now
  backed by a page mapped **Device** memory (DMA-safe; a cached buffer hid the
  CPU's writes from the VC and vice-versa). 3da8c4b's display workaround
  reverted; the firmware now returns `0x3c100000`/`0x12c000`. Server exit
  branches now print their reason.

## Stage 6a: boot bringup + user print

- **stage6-init-bringup** (task 70, e6d9145) — `system::bringup()`: kernel
  spawns nameserver → mailbox → console → init; init spawns children by
  registry index via `SYS_SPAWN`. The design moved past the task file
  (SYCSPAWN was not deferred to stage 7 after all); the stage-7 supervision
  intent it held now lives in `stage7-init-supervises.md` (task 98).
- **stage6-early-console-retirement** (task 71, 34cbe80) — `CONSOLE_LIVE`
  in `port/src/devcons.rs` gates kernel `println!` after bringup; `iprint`
  stays as the panic/debug backstop (the Linux `keep_bootcon` / Zircon
  debug-UART shape). Kernel `println!` after the gate is silently dropped —
  a future logging task may ring-buffer it.
- **r9x-userspace-print** (task 89, fd7e96c) — `SYS_PRINT` (31), 256-byte
  cap through `devcons::iputb`, plus `r9x_std` `println!`/`print_str`.
  Acceptance drift, accepted: only `cmd/init` uses it so far, and the
  mailbox investigation was resolved kernel-side instead. The unchecked
  `copy_from_user` it rides is task 92's hardening.

## Batches A–E (2026-08-20 reviews)

- **Batch A** — QEMU runner: per-stream drain, x86-64 gdb-port gating.
- **Batch B** — xtask error handling: kernel.ld write, link-script panic,
  rustflags TOML array, RustupState.
- **Batch C** — workflow YAML: caching, labels (5 commits).
- **Batch D** — arch/target selection: one source of truth, loud skips (3 commits).
- **Batch E** — kernel-side hosted-test hardening (2 commits).

## Other (no prior task file)

- **timers-integration-test** (2026-08-21) — main9 ticker became asserted
  `tests/timers.rs`; fixed trap.S sp/x0 corruption.
- **allocators-integration-test** (2026-08-22) — allocation smoke-test became
  asserted `tests/allocate.rs`.
- **kernel-console-pl011** (2026-08-24) — early console PL011 (was MiniUart);
  `mailbox::init` precedes `boot::console` in all 17 aarch64 images.
- **pi3-local-intc** — resolved: out of scope (f3bb77c).
- Plus ~30 CI/xtask/trap/gic tasks (all in `done/`, listed in the provenance
  note below).

## Provenance

The `range-by-value` plan and sweep notes are no longer in the tree; the
sweep landed (PhysRange is by-value in port/src/mem.rs).

Completed and dropped from the active list (all in `done/`):
ci-qemu-packages-missing, gic-init-ordering, timer-sysreg-isb,
timer-rearm-clamp, xtask-passing-status-duplication,
xtask-undeclared-image-message, xtask-test-targets-stale,
ci-arch-tests-cross-host, trap-svc-exit, swtch-spsr-cpsr,
xtask-qemu-stderr-drain-deadlock, xtask-x86-64-qemu-gdb-port,
xtask-kernel-ld-write-unchecked, xtask-config-link-script-panic,
xtask-rustflags-whitespace-split, xtask-rustup-state-unwraps,
ci-cache-steps-triplicated, ci-cache-seed-on-failure,
ci-apt-cache-immutable-key, ci-checks-implicit-runner-arch,
ci-aarch64-comment-rationale, xtask-host-target-detection,
xtask-arch-list-single-source, xtask-test-silent-arch-skip,
ci-riscv64-zero-test-signal, irq-daif-asm-test-gating,
main9-no-mangle-gc-sections, gic-lock-free-hot-path.

### Correctness batch (branch `correctness-batch`)

- **r9-read-user-checked** (task 92) — `read_user`/`write_user` validate
  via `vm::user_range_ok` before `copy_nonoverlapping`; bad pointer is
  an error return, not a kernel data abort.
- **r9-fault-dfsc-decode** (task 93) — the EL0 fault print decodes DFSC
  (translation/permission/external abort with level) instead of raw ISS.
- **r9-mapto-error-path** (task 94) — `map_to`'s recursive-entry restore
  brackets the write unconditionally; read-back verification after
  `write_entry`. `DryPageAllocator` test forces the mid-walk failure.
- **r9-channels-close-on-exit** (task 95) — `close_all_for`: a dying
  process closes channels it is blocked on; peer wakes to `ERR_CLOSED`.
  Integration test exercises the path (two-phase run, two channels).
- **r9-syscall-exit-status** (task 100) — `SYSEXIT` passes `frame.x0`
  (exit status) instead of `frame.x8` (svc number).
- **r9-fault-backtrace** (task 90) — `backtrace::print_backtrace` walks
  the user's FP chain (`frame-pointer: always` in the target spec),
  printing return addresses. Every read via `read_user`; bounded to 32
  frames. Offline symbolication (raw addresses + llvm-symbolizer).

### Backtrace symbolication & W^X (post-audit, 2026-08-27)

- **r9-user-text-wx** (task 96) — user text is RO+X: `rw_user_text` →
  `ro_user_text` (AP AllRo); the kernel keeps writing text via the
  TTBR1 alias. `spawn_raw`'s `user_text` local renamed `ktext`. The
  store-to-text kill test is deferred to task 91's W^X matrix row.
- **r9-backtrace-symbols** (task 90b) — the fault backtrace prints
  `name+0xoffset` by parsing `.symtab`/`.strtab` from the embedded ELF
  at spawn (`SymRef` on `Process`); stripped/raw images fall back to
  raw addresses.
- **r9-demangle-v0** (task 90c) — in-kernel Rust v0 symbol demangler;
  backtrace names print as `faulttest::main`, not `_RNv...`.

### Console / mailbox routing

- **console-server-persistent** — the console server writes each
  message's payload to the PL011 and loops (no exit). The mailbox
  server resolves `/dev/console` and routes its ARM memory diagnostic
  through the console server (IPC), not the raw debug UART. Kernel boot
  diagnostics (interrupt stack, binary sections, pagealloc) print before
  `set_console_live()` (raw UART, same physical device).
- **r9-console-server** (task 88) — the `r9x_std::console` client API
  (`std/src/console.rs`): `write`/`println`, one-shot `RESOLVE /dev/console`
  cached in a `Cell` (a "thread" is a process), chunking at `MSG_MAX - 4`,
  and `ConsoleError` (`Ipc` vs `Closed`). The `display` image's "display
  passed" verdict now goes through `console::write` via a new `cmd/consclient`
  test program (the image's `main9` runs at EL1 and cannot issue `svc`, so a
  user process carries the verdict), and the new `two_clients` integration
  image proves per-client reply-channel serialisation with two concurrent
  clients. Both images use phased bringup (servers to fixpoint, then clients)
  so a client's `RESOLVE` always finds the bound name. Surfaced task 101
  (display server reads its nameserver handles from the wrong form).
