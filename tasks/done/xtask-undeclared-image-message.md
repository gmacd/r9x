---
status: done
---

# Distinguish a missing [[test]] entry from a missing feature

`undeclared_images` (xtask/src/main.rs:1175) reports any `tests/*.rs` whose
stem is absent from `declared`. But `declared` comes from `test_names`,
which has already filtered the manifest to targets carrying
`required-features = ["qemu-test"]` (main.rs:1161).

So a test file that *does* have a `[[test]]` stanza, but whose stanza omits
the feature, is reported as

    <arch>: tests/foo.rs has no [[test]] entry, so nothing builds it

and fails the run. The message is wrong about the cause and sends the
author to add a stanza that is already sitting in the manifest — the more
likely mistake of the two, since `harness = false` and the feature line are
easy to half-copy from a neighbouring entry.

The check itself is right to be strict: a forgotten stanza is a test that
looks like it passed, which is the whole reason this exists.

Fix: read the test targets once without the feature filter, and compare
against both sets — no stanza at all keeps today's message; a stanza
without `required-features = ["qemu-test"]` gets one that names the missing
feature.

Done when: each of the two mistakes produces a message that names it.

Origin: code review of the qemu-integration-tests branch (main...HEAD).

## Status: done

- `undeclared_images` now reads the full `[[test]]` stanza set (not the
  feature-filtered one) and distinguishes the two mistakes:
  "has no [[test]] entry, so nothing builds it" versus "has a [[test]]
  entry, but is missing the qemu-test feature".
