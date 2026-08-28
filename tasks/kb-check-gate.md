---
id: 128
status: open
---

# Task 128: `cargo xtask kb --check` — report knowledge-base pages whose code moved

## Problem

Every page in `docs/` declares the code it describes and the commit at which a
human last confirmed it:

```markdown
---
covers: aarch64/src/gic.rs, aarch64/src/irq.rs, aarch64/src/timer.rs
sources: ARM IHI 0048D (GICv2 Architecture Specification), BCM2711 ARM Peripherals
verified: f76d96a (2026-08-28)
---
```

Nothing checks it. The convention is documented in `docs/README.md` and held
only by discipline, which is exactly how the previous attempt failed:
`HowItWorks.md` carried "**Please update with any material code change!**" and
a `.claude/hooks/check-howitworks.sh` that no longer exists — the plea
survived, the mechanism did not, and the document went stale and orphaned.

The failure is already observable. `docs/hardware/gicv2.md` cited
`aarch64/src/gicv2.rs`, a file that has not existed under that name for some
time; the drift was found by hand while importing the page, not by a gate.
`docs/decisions/0012-user-binaries-are-elf.md` asserted a W+X on user text
that task 96 had already fixed in `d773a37`.

## Design

An xtask subcommand that, for each Markdown file under `docs/` carrying a
`covers:` header:

1. Parses `covers:` (comma-separated repo-relative paths or globs) and
   `verified:` (a commit, then a parenthesised date).
2. Runs `git diff --name-only <verified-commit>..HEAD -- <covers paths>`.
3. Reports each page whose covered paths changed, with the commits that
   touched them, so a reader knows what to re-check.

Also report, as distinct classes:

- a `covers:` path that no longer exists (the `gicv2.rs` case — a rename or
  deletion the page did not follow);
- a page with no `covers:` header at all, except the allowed exceptions:
  `docs/README.md`, `docs/decisions/*` (which carry `status:`) and
  `docs/reading/*` (which carry `informs:`);
- a `verified:` commit that is not an ancestor of `HEAD` (a rebased or
  invented reference).

**Report, do not block.** A blocking documentation gate gets bypassed within a
week; the point is a short honest list, printed by CI beside `fmt --check`, of
what needs a human's eye. Exit 0 with findings on stdout; reserve a non-zero
exit for malformed headers, which are a defect in the page rather than a
staleness signal.

Wire it into `.github/workflows/xtask.yml`'s `checks` job, and into
`cargo xtask ci` so the local gate matches CI.

## Tests

- Unit: header parsing — well-formed, missing fields, a bad commit, a glob.
- Unit: the ancestry and existence checks, against a fixture repo.
- Integration: run over the real `docs/` tree; every page must parse, and the
  known-good state must report no malformed headers.

## Done when

- `cargo xtask kb --check` parses every page under `docs/` and prints stale,
  dangling and malformed pages, with the commits responsible.
- CI runs it in the `checks` job; `cargo xtask ci` runs it locally.
- Running it at `HEAD` today produces a list a reader can act on, and no
  malformed-header errors.

Origin: knowledge-base scaffolding, 2026-08-28. Named as planned-but-unbuilt in
`docs/README.md`.
