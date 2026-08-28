---
id: 101
status: open
---

# Task 101: display server reads its nameserver handles from the wrong form

## Status: open — filed from the task-88 build (console client API)

## Problem

The display server (`cmd/display`) reads its nameserver handles from the
**extra** fields, but the display image spawns it with the nameserver's pair
in the **main** fields.  The two disagree, and the server only works by a
channel-index coincidence.

`cmd/display/src/main.rs:99-100`:

```rust
let ns_in = rt::handle_at(2) as u64;
let _ns_out = rt::handle_at(3);
```

`handle_at(2)` / `handle_at(3)` are the `ns_inbound` / `ns_outbound` (extra)
fields.  But `aarch64/tests/display.rs` spawns the display server as:

```rust
let display_handles = process::Handles {
    inbound: ns_in as u32,      // main field  -> handle_at(0)
    outbound: ns_out as u32,    // main field  -> handle_at(1)
    ns_inbound: 0,
    ns_outbound: 0,
};
```

The kernel writes the `HANDLES_VA` header with `n = 2` when the extra fields
are zero (`aarch64/src/process.rs`, `spawn_elf`), so `handle_at(2)` and
`handle_at(3)` read the zeroed tail of the page — both return `0`, i.e.
channel 0.  The display server therefore sends its `RESOLVE /dev/mailbox`
and `BIND /dev/display` to **channel 0**, which happens to be `ns_in` only
because the image creates the nameserver's pair first.  If the nameserver's
inbound channel were ever created after another channel, the display server
would address the wrong channel and its resolves/binds would go nowhere.

This is pre-existing (the display test predates task 88) and out of scope
there; the new servers added in task 88 (console server, `consclient`) use
the extra-field form correctly, so they are not affected.

## Evidence

- `cmd/display/src/main.rs:99-100` reads `handle_at(2)` / `handle_at(3)`.
- `aarch64/tests/display.rs` spawns `DISPLAY_ELF` with the ns pair in the
  main fields, extra fields zero.
- `aarch64/src/process.rs` `spawn_elf`: `n = 4` only when an extra field is
  nonzero, else `n = 2`; the header is zero-padded, so `handle_at(2..4)`
  read `0`.
- It passes today because channel 0 is `ns_in` (created first).

## Fix direction

Spawn the display server in the extra-field (for-server) form, matching the
console server, mailbox server, and clients:

```rust
process::Handles {
    inbound: 0,
    outbound: 0,
    ns_inbound: ns_in as u32,
    ns_outbound: ns_out as u32,
}
```

so `handle_at(2)` / `handle_at(3)` are the real nameserver handles.  (The
display server makes its own serving pair at runtime, so the main fields stay
zero.)  Alternatively the server could read `rt::handles()` (the main pair)
instead of `handle_at(2)`, but the extra-field form is the convention every
other name-resolving server uses.

## Done-when

The display server's `ns_in` comes from a field the spawner actually wrote
(no reliance on channel 0 == `ns_in`), and the `display` integration image
still passes.

## Origin

Filed while building task 88 (`r9-console-server`, the `r9x_std::console`
client API).  The new `consclient` and console-server spawns use the
extra-field form; the audit of the display image's channel budget surfaced
the pre-existing display-server mismatch.
