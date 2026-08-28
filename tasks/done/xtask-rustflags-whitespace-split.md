---
status: done
commit: 9187e7c
---

# Rustflags break on paths containing spaces or apostrophes

`xtask/src/config.rs:257` joins rustflags on spaces and passes them as a
single-quoted string via `--config build.rustflags='...'`. Cargo
whitespace-splits string-valued rustflags, so a workspace or
CARGO_TARGET_DIR path containing a space shears the
`-Clink-args=-T<path>/kernel.ld` flag in two (a checkout under
`/Volumes/My Code/r9` links against `-T/Volumes/My` plus a bogus token);
an apostrophe in the path terminates the TOML literal and cargo rejects
the `--config` value outright.

Fix direction: pass rustflags as a TOML array
(`build.rustflags=['-C','link-args=…']`), which cargo does not
whitespace-split, with proper escaping of the elements.

Done when: a checkout in a directory with a space in its path builds and
links all three arches.

Origin: code review of xtask/CI (2026-08-20, high effort) — PLAUSIBLE
(mechanism verified; needs a space/apostrophe path to trigger).

## Status: done (9187e7c)

`apply_rustflags` renders each flag as an escaped TOML basic string in a
`build.rustflags=[...]` array, which cargo passes through verbatim; the
now-stale "no spaces" constraint comment on `check_cfg` was dropped.
Verified: all three arches dist-build and the aarch64 integration tests
pass through the array form.
