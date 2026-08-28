---
status: done
---

# Derive the x86-64 passing status from the value that produces it

`Arch::passing_status` (xtask/src/main.rs:78) returns a hardcoded `33` for
x86-64. `x86_64/src/qemu.rs:21` defines `PASS_STATUS: i32 = 33`, derived in
its own comment from `PASS: u32 = 0x10` as `(0x10 << 1) | 1`, and says the
two are "kept beside the value it comes from so the two cannot drift".

They can drift, because xtask does not read either of them. The doc comment
on `passing_status` claims "xtask asks the arch for this rather than
assuming zero"; it asks a copy. Change `qemu::PASS` and update `PASS_STATUS`
alongside it exactly as intended, and xtask still compares against 33 at
main.rs:1107 — every x86-64 image that passes is reported
`FAILED (exit <new status>)`, with the failure pointing at the image rather
than at the constant.

xtask is a host binary and the arch crates are `no_std` kernel crates, so
importing the constant is not free — which is presumably why it was copied.

Fix direction: either make xtask depend on the x86_64 crate for that one
constant, or keep the copy and make it self-checking — a `const _:` assert
in `x86_64/src/qemu.rs` is no use since it cannot see xtask, so the check
has to live on the xtask side or in a shared crate. Failing both, at least
make each comment name the other file and line so the pair is greppable.

Done when: changing `qemu::PASS` cannot leave xtask reporting passing
images as failures.

Origin: code review of the qemu-integration-tests branch (main...HEAD).

## Status: done

- The status constants live together in `port/src/qemu.rs` (`PASS`,
  `FAIL`, `PASS_STATUS`, `IOBASE`): the x86_64 test images write
  `port::qemu::PASS`, and xtask's `passing_status` reads
  `port::qemu::PASS_STATUS`. One copy, referenced by both sides, with
  intra-doc links naming the derivation.
- Residual: `PASS_STATUS` is still the literal `33` with the derivation
  in a comment; writing it as `((PASS as i32) << 1) | 1` would close the
  last drift path.
