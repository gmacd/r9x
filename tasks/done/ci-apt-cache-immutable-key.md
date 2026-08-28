---
status: done
commit: f1dd2cf
---

# The apt archives cache key is immutable, so the cache decays

The qemu apt-archives cache in `.github/workflows/xtask.yml:88` uses the
fixed key `apt-qemu-2604`. `actions/cache@v4` skips the save on an exact
key hit, so the cache is written once and never refreshed. As the
ubuntu-26.04 image's qemu package versions advance past the frozen .debs,
every run re-downloads from the mirror again — and can never save the
newer packages under the existing key. The slow-mirror protection this
step exists for (08cc076's whole point) silently decays; a slow mirror
then stalls runs into the 480s install timeout and fails the job.

Fix direction: give the key a changing component — `hashFiles` of the
package list, or a dated/versioned suffix — plus `restore-keys` with the
stable prefix so stale caches still warm the download.

Done when: bumping the qemu package set (or the image updating its
candidate versions) results in a fresh cache save on the next green run,
observable in the job's post-cache log.

Origin: code review of xtask/CI (2026-08-20, high effort) — CONFIRMED.
Same file as the rust-cache batch
([ci-cache-steps-triplicated.md], [ci-cache-seed-on-failure.md]).

## Status: done (f1dd2cf)

The apt cache key is now `apt-qemu-2604-${{ github.run_id }}` with a
`apt-qemu-2604-` restore-keys prefix: unique per run so the save is
never skipped as an exact hit, restored from the newest previous save,
so the cache tracks the image's package versions instead of freezing at
its first write.
