---
status: open
---

# gate-drift-watch: nightly drift cron and QEMU version attribution

Task 7 of 7 in the gates-hardening arc. Plan:
[plans/gates-hardening.md](plans/gates-hardening.md).

## Goal

Two drift sources, two cheap watchers: the nightly pin (the build
leans on `-Zbuild-std`, `-Zjson-target-spec`, and unstable lint
behaviour — a pin bump is where this project breaks) and the QEMU
runner image (the workflow already documents the raspi4b/QEMU
coupling; a behaviour change should be attributable to an image
bump without spelunking logs).

## Changes

- **Nightly-drift cron** (new workflow, weekly): install current
  `nightly`, override, `cargo xtask ci`. **No
  `continue-on-error`** — a scheduled workflow gates nothing, so a
  red run costs nothing and hiding it makes the job feel like
  coverage while being none. A failure step updates a pinned
  "nightly drift" issue (that push channel is what earns the keep;
  if the issue machinery is rejected in review, the honest fallback
  is cutting the job and bumping the pin on breakage).
- **QEMU version to `$GITHUB_STEP_SUMMARY`**: the existing
  `qemu --version` prints (and the documented image coupling)
  redirected into the step summary — a 3-line change to the install
  step. No version pinning — the apt-pinning machinery is not worth
  it.
- Implementation notes (2026-08-27 audit): the workflow needs
  `permissions: issues: write` and a stable way to find the pinned
  issue (label search or hardcoded number). Evidence the watch pays:
  the pin already moved once (2026-07-27 → 2026-08-21) in the month
  since the plan was written. Cheap rider: `cargo deny check
  advisories` weekly in the same cron (xtask pulls serde/clap/toml).

## Acceptance

- The cron runs on schedule; a red run surfaces on the pinned issue.
- The QEMU version is visible in the workflow step summary.

## Not in scope

Pinning QEMU or the runner image; automating the pin bump itself.
