# Decision records

What r9x chose, why, and what lost. A record is written when a decision is
*settled*, not while it is being argued — the argument lives in the plan
(`tasks/plans/`), the conclusion lives here.

Numbered, append-only, never edited into a different decision: when a choice
changes, write a new record that says what it supersedes. Start from
[`0000-template.md`](0000-template.md).

| # | Decision | Status |
|---|---|---|
| [0001](0001-gic-routed-timer-only.md) | Require a GIC-routed generic timer PPI on aarch64 | accepted |
| [0002](0002-qnx-mechanism-plan9-interface.md) | QNX mechanism under a Plan 9 interface | accepted |
| [0003](0003-priority-scheduling-with-inheritance.md) | Priority scheduling with priority inheritance | accepted |
| [0004](0004-blocking-send-bounded-channels.md) | Channels are bounded, `send` blocks, no drop mode | accepted |
| [0005](0005-opaque-kernel-message.md) | Opaque kernel message; 9P rides on it; native opcodes are the exception | accepted |
| [0006](0006-aspace-shape-and-fault-policy.md) | `Aspace` is a page-table root; EL0 faults kill | accepted |
| [0007](0007-device-dumb-kernel.md) | Device-dumb kernel: servers map their own MMIO | accepted, partly superseded by 0010 |
| [0008](0008-irq-to-message-routing.md) | IRQs become messages: `try_send`, no inheritance, no retry | accepted |
| [0009](0009-nameserver-in-user-space.md) | Names in a user-space nameserver; kernel owns handles | accepted |
| [0010](0010-map-mmio-becomes-a-capability.md) | `SYS_MAP_MMIO` becomes a capability | accepted, **not yet implemented** |
| [0011](0011-multicore-is-imminent.md) | Multi-core is imminent; every race is a live defect | accepted (standing) |
| [0012](0012-user-binaries-are-elf.md) | User binaries are static non-PIE ELF, embedded at build time | accepted |
| [0013](0013-elf-symtab-backtrace.md) | Backtraces symbolicate from the ELF `.symtab` at spawn | accepted |
| [0014](0014-curated-r9x-std.md) | A curated `r9x_std` on `core` + `alloc`, not a fork of `std` | accepted |
| [0015](0015-display-server-in-user-space.md) | Display server in user space, pluggable sink and pacing | accepted, in progress |
| [0016](0016-first-user-process-entry-and-exit.md) | EL0 entry and return for the first user process | accepted |

## Reading them

Two are worth knowing before touching anything:
[0002](0002-qnx-mechanism-plan9-interface.md) is the shape of the whole system,
and [0010](0010-map-mmio-becomes-a-capability.md) is the one place where a
comment in the tree currently claims a property the code does not enforce.
