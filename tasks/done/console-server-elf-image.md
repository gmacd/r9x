---
status: done
---

# console-server-elf-image: embed the console ELF and prove it boots

Task 4 of 4 in the user-binary-loading arc. Plan:
[plans/user-binary-loading.md](plans/user-binary-loading.md).

## Status

**Done** — committed in 946a41d and merged into main (fast-forward; the whole
user-binary-loading arc, tasks 61–64, landed on main).  Gates green (18/18
integration images, 85 host tests, warning-free on all three arches),
panel-reviewed.

- `aarch64/build.rs` stages `console.elf` into `OUT_DIR`.  It uses the aarch64
  JSON-spec target name hardcoded (not `TARGET`, which varies by how the crate
  is being built: a JSON-spec build and the host-toolchain check step use
  different target names for it).
- `console_server.rs` embeds the ELF via `include_bytes!` and spawns it through
  `Image::Elf`; `SERVER_TEXT` and its `SERVER_TEXT_VA` / `SERVER_STACK_VA`
  consts are deleted.
- xtask: the `BuildStep` and `TestStep` run the `ServerStep` first (the crate's
  build.rs needs the ELF present).  On a clean tree a bare `cargo xtask test`
  on an aarch64 host now builds the server instead of panicking in the build
  script — the panel caught that the `TestStep` was the one of the six xtask
  steps that runs the aarch64 tests but not the `ServerStep`.
- Gates: `cargo xtask ci` green (18/18 integration images, 85 host tests,
  warning-free on all three arches).  Dependency proof demonstrated: touching
  `servers/console` relinks the `console_server` image (the build.rs reruns on
  the changed ELF).

## Goal

Wire the built console ELF into the `console_server` integration image and
prove, end to end, the two properties the arc exists for: (a) a **Rust-built
server ELF loads and runs with no kernel file access**, and (b) **changing the
server rebuilds the embedding image**. This task replaces the hand-assembled
`SERVER_TEXT` in the existing image with the built ELF.

Depends on tasks 1, 2, and 3.

## Changes

All in `aarch64/`.

- **`aarch64/build.rs`** (new) — stages the console ELF for the crate that
  embeds it:
  - Locate the built ELF (the path xtask's ServerStep writes — passed via an
    env var or the conventional `target/<spec>/<profile>/console.elf`).
  - `cargo:rerun-if-changed=<elf path>` so a changed server (new mtime)
    re-runs this build script and, via the included file, recompiles the
    crate.
  - Copy the ELF into `OUT_DIR` (e.g. `OUT_DIR/console.elf`).
  - **Loud failure** if the ELF is absent: `cargo:warning=`/`panic!` with
    "build the console server first: `cargo xtask build --arch aarch64`" — a
    bare `cargo build` of the image outside xtask must fail saying *why*, not
    silently (plan, decision 3).
- **`aarch64/tests/console_server.rs`**:
  - `static CONSOLE_ELF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"),
    "/console.elf"));`
  - Replace its `Image::Raw { text: &SERVER_TEXT, text_va: SERVER_TEXT_VA,
    stack_va: SERVER_STACK_VA }` (introduced by task 2's sweep) with
    `Image::Elf(CONSOLE_ELF)` — i.e. `process::spawn(&Image::Elf(CONSOLE_ELF))`.
  - **Delete** the hand-assembled `SERVER_TEXT` array and its now-unused
    `SERVER_TEXT_VA` / `SERVER_STACK_VA` consts (the point of the arc).
  - Keep the kernel-side PL011 enable (the early path) and the `status == 0`
    assert. The rest of the image is unchanged.
- The image still declares `[[test]] name = "console_server"` /
  `required-features = ["qemu-test"]` (unchanged).

## Tests

- **The boot proof** (host tests cannot make it): `cargo xtask qemu --arch
  aarch64 --image console_server` boots the image; the kernel enables the
  PL011, `spawn_elf` loads the built console ELF into a fresh process
  (segments mapped into its TTBR0, ELR = `e_entry`), the server runs
  (SYSMAPMMIO + writes `'A'` + exit 0), and the kernel asserts status 0.
- **The dependency proof** (the property the user asked for): touch/rebuild
  the `servers/console` source so `console.elf` changes mtime; the next
  `cargo xtask build`/`integration-test` of `aarch64` recompiles the
  `console_server` image (the build.rs reruns on the changed ELF). Demonstrate
  the rebuild happens (e.g. `cargo build -p aarch64 --test console_server
  --features qemu-test -v` re-runs the build script and relinks after the
  server is rebuilt).
  - The image now boots via the **unified** `process::spawn(&Image::Elf(…))` —
    the same entry point the raw images use via `Image::Raw` (plan, decision 2).

## Acceptance

- `cargo xtask ci` green, including the `console_server` integration image
  passing under QEMU (aarch64).
- `SERVER_TEXT` is gone from `console_server.rs`; the image boots a
  Rust-built ELF via `spawn_elf`.
- The dependency proof: a server change rebuilds the embedding image (the
  build.rs reruns; the image relinks).
- A bare `cargo build -p aarch64 --test console_server --features qemu-test`
  with no prior xtask server build fails **loudly** with the "build the
  server first" message (not a missing-file mystery).

## Not in scope

The kernel (`main9`) adopting the built console server (the "boot to graphics"
bridge — a separate next arc; plan, end of Tasks). Rewriting the other raw
*programs* to ELFs (the user's call — they stay hand-assembled, wrapped in
`Image::Raw`). Multiple servers in one image (a second `include_bytes!` + a
per-server layout is the obvious extension; the arc proves the single-server
path). The dependency proof is demonstrated, not turned into a CI gate, for the
arc.

## Follow-ups (pre-existing, surfaced by the panel)

Both resolved in `f1f33b9` (the loopback-verification refinement, merged right
after this task): `enable_pl011` now read-modify-writes the named bits
(`UARTEN|TXE|RXE`) and reads the CR back before spawning; the DT's PL011 base is
cross-checked against the server's hardcoded `0xfe201000`; and the server's `'A'`
is proven to have reached the device by switching the PL011 into loopback (LBE)
around the server's run and asserting the DR readback is `'A'` (masked to the
data bits). The original findings, for the record:

- **`enable_pl011` writes the wrong CR value** (`aarch64/tests/console_server.rs`).
  It writes `0x31` (UARTEN plus two reserved bits) as a full 32-bit write, which
  *clears* TXE (bit 8) and RXE (bit 9) — the reset value `0x0300` has both set.
  "Enable UART, TX, RX" is `0x0301`.  The comment ("CR bits 0, 1, 4") and the
  value both disagree with the PL011 TRM.  It passes only because QEMU's PL011
  model doesn't gate transmission on TXE.
- **The image never checks that `'A'` was received.**  The only assertion is
  `status == Some(0)`.  The kernel locates the UART via the device tree; the
  server hardcodes `0xfe20_1000`; nothing cross-checks them, and a write to a
  mapped Device page does not fault — so a wrong base maps the wrong page, the
  server writes `'A'` into nothing, and the image still passes.  The `'A'` is
  the single observable that the mapping reached the device, and the image and
  harness both look past it.
