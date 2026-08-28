---
status: done
---

# r9-ipc-test-coverage: IPC unit test gap analysis (Tier 2 hardening)

Go through the IPC code (`port/src/ipc.rs` and the arch-specific channel
code) and identify unit test gaps. Use code coverage and complexity tools
to find where tests would be most valuable.

## Motivation

The IPC layer is the load-bearing mechanism of the kernel — every
inter-process interaction goes through it. The QEMU mailbox ALLOCATE bug
(a reply delivered to the wrong process) was an IPC routing issue that
went undetected because the existing tests don't cover the multi-receiver
case (two processes waiting on the same channel). The IPC state machine
(channel states: empty → has-message → received; blocking send/receive;
reply routing) is complex enough that targeted unit tests would catch
regressions before they manifest as system-level failures.

## Approach

1. **Code coverage:** run `cargo llvm-cov` (or `cargo tarpaulin`) on the
   host tests for `port` and `aarch64`. Identify branches and functions
   with zero or partial coverage. The channel state machine transitions
   (especially the multi-receiver and reply paths) are the likely gaps.

2. **Cyclomatic complexity:** use `gru` or `cargo clippy`'s
   `cognitive_complexity` to identify the most complex functions in
   `port/src/ipc.rs`. Complex functions are where bugs hide.

3. **Manual review:** read the IPC code with a focus on:
   - Channel state transitions: are all states reachable? Are all
     transitions tested?
   - The multi-receiver case: two processes waiting on the same channel —
     does the message go to exactly one? Is the other woken?
   - Reply routing: when a server replies, does the reply go to the
     specific sender (not any receiver on the channel)?
   - Blocking semantics: a blocking receive on an empty channel — does it
     block correctly? Does a send wake it?
   - Edge cases: send to a full channel, receive from a channel with no
     sender, reply with no pending request.

4. **Write the missing tests:** add host unit tests to `port/src/ipc.rs`
   (or a `tests/` submodule) that cover the identified gaps. The tests
   should be at the unit level (testing the channel state machine
   directly), not the integration level (spawning processes).

## Deliverables

- A coverage report (even if just a summary: which functions/branches are
  uncovered).
- A list of identified gaps (ordered by risk: multi-receiver routing >
  reply targeting > blocking edge cases > state transition completeness).
- New unit tests for the highest-risk gaps.
- `cargo xtask test` green with the new tests.

## Acceptance

- `cargo xtask ci` green.
- The new tests pass and would have caught the mailbox ALLOCATE routing
  bug (or a similar misdelivery).
- A coverage summary is recorded in the task file (even if just
  "branch coverage on `port/src/ipc.rs` went from X% to Y%").

## Not in scope

- Integration-level tests for IPC (those already exist: `ipc.rs`,
  `channel.rs` images).
- Fuzzing the IPC layer (a future hardening task).
- Formal verification of the channel state machine (overkill for the
  current complexity).
