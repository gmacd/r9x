# Stage 6a: nameserver — names to channels (the namespace returns, minus 9P)

## Problem and constraints

Stages 2–5 built the substrate: a bounded `Message`/`Channel` with send/receive/
reply and priority inheritance (`port::ipc`), per-process `Aspace`, IRQ→message
routing, and a user-space console server that maps the PL011 and writes a byte.
But every one of those is reached by *test images that hardcode the wiring*: the
`console_server` image knows the server's PL011 base, spawns it, and reads the
result. There is no *name* for any of it, and no way for one user process to
find another. The "file metaphor" — the reason this is a Plan 9-shaped system —
is still absent.

This plan is the **narrow first slice of stage 6**: a user-space **nameserver**
that owns a `name → channel-handle` bind tree, a way for a **file server** to
publish a channel under a name, and a way for a **client** to resolve a name to
a channel handle and speak over it. The file metaphor returns in its
*namespace* form: a client sees a path (`/dev/console`), not a raw channel
handle. 9P itself, the tree structure, the boot-time `init`, and retiring the
in-kernel early console are all explicitly out of this slice (see Not building)
and are filed as follow-on tasks.

Standing constraints: warning-free across aarch64 / x86-64 / riscv64 (`cargo
xtask ci`); aarch64 is the reference implementation; minimal scoped changes; the
kernel stays device-dumb and the namespace lives in user space; a new user-space
server is a separate bare-metal Rust executable embedded in the test image (the
stage-5 `ServerStep` pattern), with no kernel file access.

## Prior art

**r9 already has the mechanism this composes.** `port::ipc` is the arch-agnostic
`Channel`/`Message`/send/receive/reply with PI. `aarch64::ipc` binds it to a
4-slot channel table (`ChannelHandle = usize`, `create()`, `channel(h)`) and the
message syscalls (`SYCSEND`/`SYCRECEIVE`/`SYCREPLY` take a handle in `x0`). The
`SYSMAPMMIO` and `SYSIRQCLAIM` syscalls already let a user process own a device
and its IRQ. What is missing is exactly this slice: a user-space process that
maps *names* to channel handles, a **user-space channel-creation syscall**
(`aarch64::ipc::create()` is kernel-side only today), and the server/client
protocol that rides the existing message syscalls.

**Plan 9** (`/Volumes/Code/repos/plan9/plan9`): `sys/man/1/bind` and `2/bind`
define the user-facing semantics — `bind new old` grafts a name (`new`) into an
existing name space at a bind point (`old`); `mount` is the same with the server
named. The nameserver (`/mnt`) is itself a file server that owns the tree of
bind points; a client walks paths through it. The essential idea reused: **a
user-space process owns the name→resource map, and a client resolves a name to a
resource by asking it.** The accretion refused for this slice: the 9P fid walk
(stage 7), the per-process name-space groups (`fork`), and the tree structure.

**Linux** (witness): `mount(2)` / the VFS superblock registry is the kernel's
name→filesystem map — the *opposite* residence (kernel-owned). It confirms the
shape (a registry of named resources, resolved to a handle) but is the monolith
shape this design refuses: the map lives in a user-space process, not the kernel.

## Hardware assumptions (required)

The nameserver and the client are **pure user-space message processes**: they
touch no MMIO and claim no IRQ. The console server (stage 5) already owns the
PL011 MMIO and, for this slice, still only transmits. So the hardware surface is
**unchanged from stage 5**:

- **aarch64 (Pi 4 / QEMU `raspi4b`)** — the PL011 MMIO and its VideoCore-mailbox
  3 MHz clock, owned by the console server via `SYSMAPMMIO`; the GIC routes the
  console server's (currently unclaimed) RX IRQ later. The nameserver and client
  add no new hardware assumption: they run as EL0 processes on the existing
  scheduler, `Aspace`, and IPC. Coherency: single coherent domain; the message
  handoff is the channel queue lock's release/acquire, no cache maintenance.
- **x86-64 / riscv64** — the nameserver is arch-agnostic user space; it builds
  against the same `port::ipc` and the arch's message syscalls. Gate-green
  (compiles, no warnings) but **exercised only on aarch64**, matching stage 5.
- **Firmware co-tenancy** — unchanged: the early console stays in-kernel (the
  retirement is a follow-on task); this slice does not demote it.
- **No new register or constant** is introduced by the nameserver or client. The
  only new kernel surface is a syscall number (see Interfaces), which cites the
  existing `x8` ABI, not hardware.

## Design

### Data structures

The central structure is the **bind table** in the nameserver: a fixed map of
`name → (inbound, outbound) ChannelHandle`. It is the *only* new state in the
slice, and it lives entirely in the nameserver's `Aspace` (user space), not the
kernel.

A channel is **unidirectional** (`port::ipc`), so a request/reply exchange runs
over a *pair*: the client sends on the server's *inbound* channel and receives
on its *outbound* channel; the server does the inverse. The bind entry therefore
stores the pair, not one handle — a name that resolved to only the inbound
channel would let a client send but never receive the reply.

```text
// servers/nameserver (user space, no_std)
const NENT: usize = 8;                 // fixed, no alloc — a server's bind set is tiny
struct Entry {
    name: [u8; NAME_MAX], namelen: u8,
    in:  u32,    // the server's inbound channel: clients send here
    out: u32,    // the server's outbound channel: clients receive replies here
    used: bool,
}
struct BindTable { entries: [Entry; NENT] }   // linear scan: NENT is single-digit (simplicity lens: n is small)
```

`NAME_MAX` is a const (32 covers `/dev/console` and friends with room). The
table is a linear scan, not a tree or a hash — with single-digit entries a
brute-force scan is the correct, boring choice (the tree is stage 7, with 9P).

Who owns what: the **nameserver** owns the bind table. A **file server** owns its
own channel *pair* (it creates both and publishes them). A **client** owns
nothing — it resolves a name to a pair and uses the handles it was given. No
fact is stored twice: each handle appears in exactly one bind entry (and in the
kernel's channel table, which is the *definition* of the handle, not a copy).

The kernel-side addition is one syscall and nothing else: `SYCCREATECHAN` returns
a fresh `ChannelHandle` (it calls the existing `aarch64::ipc::create()` and
returns the handle in `x0`). The channel table is unchanged (4 slots is enough
for the nameserver's pair + the console server's pair in the test image; the
table size is a const, grown with a decision if a later image needs more).

### Interfaces

Three verbs, all over the existing message syscalls — no new kernel protocol:

1. **Nameserver protocol** (the nameserver is a *server* on a channel pair). A
   client sends a request message, receives a reply, keyed by `tag`:
   - `BIND { name, in, out }` → the *file server* publishes: it first
     `SYCCREATECHAN` its two channels, then sends `BIND` carrying its name and
     the pair (the inbound channel clients send to, the outbound channel they
     receive replies on). Reply: `OK` / `Efull`.
   - `RESOLVE { name }` → the *client* asks. Reply: the server's `(in, out)`
     pair (or `ENoent`). A client that gets only one of the two could send but
     not receive, so the pair is the unit the table stores and returns.
   - `UNBIND { name }` → the file server withdraws. Reply: `OK` / `ENoent`.
   - `LOOKUP` is *not* built: enumerating names is a 9P `walk`/`stat` concern
     (stage 7). Recorded, not a gap.
2. **The message envelope** is the existing `Message { opcode, tag, buf, len }`:
   the opcode is the verb; the `buf` carries the NUL-free `name` for
   BIND/RESOLVE/UNBIND and the 8-byte `(in, out)` pair for a BIND request / a
   RESOLVE reply. `MSG_MAX` (256) already bounds it with room to spare; no new
   size.
3. **Day-one users**: exactly two — the console server (a file server that
   publishes `/dev/console`) and the `namespace` test image's client (which
   resolves it and round-trips a byte). One server, one client: the concrete
   thing, no abstraction layer (a second server is stage 7's 9P work).

The kernel's only new public surface is the syscall number and its `x0`-in /
`x0`-out contract — mirroring `SYSMAPMMIO`'s shape. It is a mechanism verb
("give me a channel"), not policy; the policy (what a name means) is entirely in
the nameserver.

### Init and bringup order

This slice is driven by the **test image**, not the kernel boot path (the boot-
time `init` is a follow-on task). The image's `main9` brings up exactly what
stage 5's `console_server` image does — `mailbox::init` before `boot::console`
(the PL011 console), interrupts, user page tables — and then:

1. The image creates the **nameserver's** channel pair (kernel-side
   `aarch64::ipc::create()`, as the stage-2/3 images do) and spawns the
   nameserver ELF, handing it its pair, *and* keeps a copy for the client. This
   is the one asymmetry in the slice, and it is forced, not a preference: the
   nameserver is the *first* server, so nothing exists yet that a client could
   ask to find the nameserver — the spawner must hand the nameserver's handles
   to its clients directly. Every later server (console, and stage 7's) is
   found *through* the nameserver, so it creates and publishes its own pair and
   the spawner never touches its handles.
2. It spawns the **console server** ELF (stage 5, unchanged) — which now
   `SYCCREATECHAN`s its own pair and sends `BIND("/dev/console", in, out)`.
3. It spawns/switches to the **client**, which `RESOLVE("/dev/console")`s, gets
   the `(in, out)` pair, sends a byte on `in`, and checks the console server's
   reply on `out`.

Load-bearing, stated: the nameserver must be **up and receiving** before any
server's `BIND` and before any client's `RESOLVE` (a `RESOLVE` before the name
exists is `ENoent`, a clean reply, not a hang — see Failure policy). The image
orders the spawns so this holds; the kernel does not.

### Failure policy

- **`RESOLVE` of an unknown name** replies `ENoent` — a clean, checkable result,
  not a block and not a panic. A client that races the bind table is the
  expected case (the server may not have bound yet); the image retries or checks
  `ENoent` explicitly.
- **`BIND` to a full table** replies `Efull`; **`BIND`/`UNBIND` of an absent
  name** replies `ENoent`. All `Result`-shaped, server-side safe Rust — the
  nameserver is ordinary `no_std` Rust with a spin `#[panic_handler]` it is not
  expected to reach.
- **A dead server's name**: the close-on-owner-death hook is not wired this arc
  (stage 5's stated limitation, carried). So a nameserver/console-server crash
  leaves a stale bind entry; the test image keeps its servers alive, matching
  the channel table's "lives for the program" contract. Filed as a known gap,
  not papered over — it is the same limitation the channel table already has.
- **`SYCCREATECHAN` when the table is full**: returns a distinct error in `x0`
  (no panic from a live process); the image is sized so it is not reachable in
  the test, but the path is checked, not `unwrap`ped.
- **Kernel-side init-only panics** (the image's own `create()`/spawn) are
  acceptable, matching `main9`'s style. No panic is reachable from the
  nameserver, the client, or the console server.

## Not building

- **9P over channels** — the `Fid`/`Req` protocol, `walk`/`stat`/`open`, and the
  9P opcodes as the default server interface. That is **stage 7**; this slice's
  server speaks the narrow native-opcode protocol the substrate plan already
  sanctioned for non-file servers. (Decision record 1.)
- **A tree bind table** (directories, bind *points*, `mount` vs `bind`, name-
  space groups). The slice's table is a flat `name → handle` map with absolute
  names. The tree returns with 9P (stage 7).
- **The boot-time `init` process manager** — kernel→`init`→nameserver→servers.
  This slice is driven by the test image; the real boot sequence is a follow-on
  task (it touches the kernel boot path and the "first user process is a
  server" ordering).
- **Retiring the in-kernel early console** — demoting the kernel's raw PL011
  path to debug-only once the console server is up (task 65's recorded intent).
  Follow-on task; it touches the "one permanent kernel device" decision.
- **The close-on-owner-death hook** (stale-name cleanup). Stage 5's known
  limitation, carried unchanged; a stale-entry test would need it.
- **RX (input) / the console server's RX IRQ** — the server still only
  transmits; the IRQ→message path to the server is a separate refinement.
- **A `LOOKUP`/enumeration verb** — a 9P `walk` concern (stage 7).

## Decision records

- **Decision: the namespace is a user-space nameserver process owning a flat
  `name → ChannelHandle` map, resolved by a native-opcode message — not a
  kernel registry, and not 9P yet.**
  - Alternatives: **(A) in-kernel name→channel table** (a kernel syscall
    resolves names); **(B) 9P nameserver now** (the nameserver speaks 9P and the
    client walks fids); **(C) Oberon static module** (the namespace is a static
    kernel module, not a process).
  - **(A) lost** because a kernel-owned name table is the monolith re-growing
    (microkernel lens: the moment a task says "put the registry in the kernel because it's
    easier," the microkernel dies); it also puts policy (what a name means) in
    the TCB. **(B) lost** for *this slice* because 9P is stage 7 — building the
    fid walk now to serve one native-opcode server is premature (the substrate
    plan sequences 9P after the namespace exists). **(C) lost** because it
    refuses the process boundary that is the whole point (the nameserver must be
    a user-space server so it can die without taking the kernel down).
  - Graft from B: the *names* and the *bind* vocabulary are Plan 9's
    (`bind`/`mount`, `/dev/console`), so when 9P lands in stage 7 the client's
    path is unchanged — only the protocol under it changes. Graft from A: the
    kernel does own the *handle* (the channel table), because a handle must be a
    kernel-validated index for `send`/`receive`; the split is kernel=handle,
    user=name.
  - Dissent: the whole-system lens objects to a *native opcode*
    protocol at all — the metaphor should be files from the start, and a
    `RESOLVE` that returns a raw handle is a leak. We accept the leak for this
    slice because the *client in the test image* is the only consumer and it is
    itself a test; the stage-7 9P client will see only a path. Recorded, not
    averaged away.
- **Decision: one new kernel syscall, `SYCCREATECHAN` (→ `ChannelHandle`), and
  nothing else kernel-side.**
  - Alternatives: (A) hand every server its channel handles as `spawn` arguments
    (no syscall); (B) a richer `SYCCREATECHANPAIR` that returns a connected
    pair.
  - **(A) lost** because it hardcodes the wiring back into the spawner — the
    exact thing the namespace exists to remove; a server that cannot create its
    own channel cannot be an independent server. **(B) lost** because a pair is
    a 9P/`channelcreate`-style convenience nothing in this slice needs; two
    `SYCCREATECHAN` calls are the boring, total form (whole-system lens: no speculative
    convenience).
  - Dissent: the simplicity lens notes a *second* channel verb is a new
    kernel surface. It is the *minimum* surface — a user process that cannot
    create a channel cannot participate in the namespace, and there is no smaller
    verb that does the job. One syscall, one line of contract.
- **Decision: the bind table is a fixed-size flat array with a linear scan, in
  the nameserver's user space.**
  - Alternatives: (A) a hash map (needs alloc or a fixed open-addressing table);
    (B) a tree (directories, bind points).
  - **(A) lost** because a hash table for single-digit entries is fancy
    machinery for n≈1 (simplicity lens: fancy algorithms are buggier when n is small), and
    open-addressing adds wrap/resize edge cases a linear scan does not have.
    **(B) lost** because it is stage 7 (the tree is the 9P `walk` structure).
  - Dissent: the Plan 9 lens wants the tree from the start (a flat
    map of absolute names is not a name space, it's a symbol table). We agree it
    is not the end state; the flat map is the *mechanism* (name→handle) with the
    *structure* (tree) deferred. The names are already absolute paths, so the
    tree is a later re-organisation of the same map, not a rewrite.

## Tasks

Sequenced; each is a follow-on to stage 5 and gate-green on its own.

1. `stage6-createchan-syscall.md` — the kernel slice: `SYCCREATECHAN` (→
   `ChannelHandle` in `x0`, error on a full table), bound to the existing
   `aarch64::ipc::create()`. No protocol, no server — the mechanism verb alone,
   with a host unit test of the dispatch. *Prerequisite for 2–4.*
2. `stage6-nameserver-server.md` — the `servers/nameserver` ELF (the stage-5
   `ServerStep` pattern): the `BindTable`, the `BIND`/`RESOLVE`/`UNBIND` loop
   over the message syscalls, `ENoent`/`Efull` replies. No kernel change.
3. `stage6-console-publishes.md` — extend the stage-5 console server to
   `SYCCREATECHAN` its pair and `BIND("/dev/console", handle)` after mapping the
   PL011. The server gains its name.
4. `stage6-namespace-test.md` — the `namespace` integration image: spawn
   nameserver + console server + a client; the client `RESOLVE`s, sends a byte,
   and asserts the round-trip. The file metaphor, proven.

Follow-on (this slice's explicit deferrals, each its own mini-design later):
5. `stage6-init-bringup.md` — the boot-time `init` (kernel→`init`→nameserver→
   servers); the "first user process is a server" ordering.
6. `stage6-early-console-retirement.md` — demote the in-kernel PL011 console to
   debug-only once the console server is up (task 65's recorded intent).
