---
status: done
commit: e29f57f
---

# Extract the triplicated CI cache steps

e022999 took the rustup-cache (`actions/cache@v4` with the hand-built
`rustup-<os>-<arch>-<hash>` key) + `Swatinem/rust-cache` (pinned SHA) step
pair from two copies to three in `.github/workflows/xtask.yml`: `checks`,
`arch-ci`, and the new `aarch64-tests`. That crosses rule-of-three: a
toolchain bump, a cache-key change, or a rust-cache version/SHA update now
has to hit three sites by hand, with no automation catching a missed one.

Two grounded fixes, pick one:

- Extract the two cache steps into a local composite action
  (`.github/actions/rust-cache/action.yml`). Note `actions/checkout`
  cannot live inside it — the repo must already be checked out for
  `./.github/actions/...` to resolve — so the action is exactly the
  8-line cache pair, which is where the pinned SHA and key expression
  live.
- Or fold `aarch64-tests` into a job with a runner matrix; the `arch-ci`
  job (`xtask.yml:59+`) already demonstrates matrix syntax in-file.

Done when: the cache-key expression and the rust-cache SHA each exist in
exactly one place in `.github/`.

Origin: code review of e022999 (2026-08-20) — "triplication" CONFIRMED,
low severity. Same steps as [ci-cache-seed-on-failure.md] — do that one
in the same change so `cache-on-failure` is set in the one extracted
place.

## Status: done (e29f57f)

Extracted the rustup-cache + rust-cache pair into
`.github/actions/rust-cache/action.yml`; the three jobs now carry one
`uses: ./.github/actions/rust-cache` line each after checkout. The key
expression and the pinned rust-cache SHA exist in exactly one place.
