---
status: done
commit: 4a74cda
---

# RustupState panics without rustup and reads errors from the wrong stream

`RustupState::new()` (`xtask/src/main.rs:102`) unwraps both
`installed_rustup_targets()` and `env::var("RUSTUP_TOOLCHAIN")`:

- With a Homebrew/distro cargo (RUSTUP_TOOLCHAIN unset), every
  `cargo xtask test/check/clippy/ci` dies with a raw `NotPresent`
  backtrace instead of a message saying rustup is required (or degrading
  gracefully). `objcopy()` at `:313` already treats the same env var as
  optional, so the non-rustup state is anticipated elsewhere in the file.
- `installed_rustup_targets` reports a failing
  `rustup target list --installed` using the command's *stdout*
  (`:112`), but rustup writes its errors to stderr — so a real rustup
  failure surfaces as a blank error message.

Fix direction: return a Result (or Option) from `RustupState::new` with a
one-line "rustup not detected" diagnostic, and report command failures
from stderr.

Done when: xtask under a non-rustup toolchain prints a readable message,
and a failing rustup invocation shows rustup's actual error text.

Origin: code review of xtask/CI (2026-08-20, high effort) — CONFIRMED.

## Status: done (4a74cda)

`RustupState::new` returns Result with a "RUSTUP_TOOLCHAIN is not set"
one-liner; `installed_rustup_targets` names the rustup invocation in
spawn errors and reports failures from stderr (with the stray stdout
clones dropped). The three call sites propagate with `?`.
