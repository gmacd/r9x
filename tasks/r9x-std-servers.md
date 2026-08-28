---
status: open
---

# r9x-std-servers: the server-backed std surface (Tier 4)

Task 7 of 7 in the r9x arc. Plan:
[plans/r9x-target-std-backend.md](plans/r9x-target-std-backend.md).
Gated on the corresponding user-space servers (9P fs/dev/net) landing — **no
kernel syscall** in this task; it is pure target side. Rationale (the Plan 9 /
QNX / Amiga shape, `prior art`): `std::fs`, `std::net`, and the console `io`
are not kernel services in r9 — they are messages to user-space servers,
reached exactly as Plan 9's libc reaches its servers through `lib9p`
(`/Volumes/Code/repos/plan9/plan9/sys/src/lib9p`). The kernel stays a
message-passing broker. This tier grows `r9x_std`'s server-backed half as each
server exists; the kernel is untouched.

## Audit notes (2026-08-27 — absorb before unparking)

- Drift: `servers/` was renamed to `cmd/` (commit 916112a); read the
  references below accordingly. The console-io story is partially
  pre-empted: `SYS_PRINT` landed as the explicitly debug-only path, and
  task 88 gives the console server an `OP_WRITE` protocol with a
  cached-resolve client — the `io` section here should build on 88's
  client (and its per-client reply channel + chunking conventions), not
  re-derive it. 88's client is the template for this task's RPC shape.
- **Framing decision (record it):** this is the 9P *message set* over
  channels, not 9P *wire framing*. r9x channels are message-oriented
  with an out-of-band `(opcode, tag)` header, so `size[4]` is redundant
  and the 16-bit 9P tag rides the existing `tag: u32`. With per-client
  reply channels (540d1a2) and blocking sequential RPC,
  single-outstanding-tag suffices — the lib9p simple-client shape, not
  Linux's concurrent tag/idr machinery (`net/9p/client.c`).
- **Start set:** Tversion/Tattach/Twalk/Topen/Tread/Twrite/Tclunk +
  Rerror — what `devmnt.c` exercises on every mount. Defer Tauth,
  Tcreate/Tremove/Tstat/Twstat (second wave), and **Tflush** — Linux
  needs flush early only for signal interruption, which r9x's blocking
  `SYS_RECEIVE` doesn't have yet.
- **msize decision (record it):** `MSG_MAX = 256` (abi/src/lib.rs:41).
  Plan 9 rejects `msize < 256` outright (devmnt.c:197) and reserves
  `IOHDRSZ = 24` for the Twrite/Rread header (fcall.h:88), leaving
  ~232-byte iounits — functional but tiny. Choose: grow `MSG_MAX`, add
  a shared-page bulk path, or accept the tiny iounit initially.

## Goal

Give `r9x_std` the file-system, network, and console surface — implemented as
message-passing clients to the r9 9P servers — so an r9x binary can
`open`/`read`/`write` a file, open a network endpoint, and write to the
console, the way a Plan 9 program uses its libc, with none of it in the
kernel.

Standing constraints: warning-free for all three arches; every `r9x_std` item
in this tier is either a thin syscall (none here) or a message to one named
server (the shape rule); a server that is absent degrades to a named error
(the `R_ENOENT`/`R_ECONNREFUSED` the nameserver already returns), not a panic;
the 9P wire format is the server's protocol, mirrored in `r9x_std` the way the
console server already mirrors the nameserver protocol.

## Changes

- **A 9P client in the target repo** (a small `r9_9p` crate, or a module in
  `r9x_std`): the `attach`/`walk`/`open`/`read`/`write`/`clunk` verbs over the
  `Channel` primitives (`r9x_std::ipc`), the envelope the servers already speak
  (the nameserver's name→pair resolution is the `attach`). This is
  `lib9p`'s `fid.c`/`file.c`/`dirread.c` in Rust, over r9 channels.
- **`r9x_std::fs`:** `File`, `open(path)`, `read`/`write`, `remove`, `read_dir`
  — each a 9P sequence against the file server (resolved by name through the
  nameserver). A `Path` type. This is the Plan 9 file server as a std API.
- **`r9x_std::io` (complete the seed from Task 1):** `stdout`/`stderr` as a
  `File` on `/dev/console` (the console server, stage 5/6); `stdin` from the
  console server's input channel. The `Read`/`Write` traits from Task 1 are now
  backed by real servers, not just the console.
- **`r9x_std::net`:** `TcpListener`/`TcpStream`/`UdpSocket`-shaped handles over
  the network server's 9P interface (`/net`), resolved by name. Added only
  when the net server exists; until then the module is absent (not stubbed).
- **Naming/paths:** an `r9x_std` path is a Plan 9 absolute path (`/dev/…`,
  `/net/…`, a server's own `/…`), resolved by the nameserver — the uniform
  metaphor (whole-system lens: one way to say "a named resource," not two).

## Tests

- **Host unit tests:** the 9P envelope encode/decode (the byte layouts the
  servers speak), the name→pair→channel resolution, the degrade-to-error path
  (server absent → `R_ENOENT`, not a panic).
- **Per-server integration images (aarch64):** once the fs server lands, an
  image that `r9x_std::fs::open`s a file the fs server exposes, reads a known
  byte, and asserts it; likewise `write` + re-`read`. Once the net server
  lands, a loopback `connect`/`send`/`recv` image. Each image is the assertion
  a host test cannot make (it exercises the real server over real channels).
- **Console `io`:** extend the `namespace`/`console_server` images: a
  `r9x_std::io::stdout` write lands a byte the console server echoes.

## Acceptance

- `cargo xtask ci` green (all arches; each per-server image passes as its
  server lands).
- An r9x binary can `open`/`read`/`write` a file, and write to the console,
  with none of it a kernel syscall.
- A missing server is a named error, not a panic.
- The 9P client is the single place the wire format lives (the servers and
  `r9x_std` both read it, no per-server mirror).

## Not in scope

A kernel file-system syscall (refused — the kernel is a broker; files are a
server). A VFS or path *cache* in the kernel (the nameserver is the cache).
A sockets API beyond the 9P-backed one (the net server's 9P interface is the
interface; raw sockets are the net server's concern). `std::fs` *metadata*
(`stat`) beyond what the 9P `stat` verb gives — added as the fs server
supports it. Async/future-based I/O — the model is blocking `receive` on a
channel (a process that blocks is a process; Decision 2); an async runtime is
a large separate design, not this tier.
