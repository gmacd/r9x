//! The bind table: a fixed `name -> channel-pair` map owned by the nameserver.
//!
//! A channel is unidirectional, so a request/reply runs over a *pair*: the
//! client sends on the server's inbound channel and receives on its outbound
//! channel.  A bind entry therefore stores the pair, not one handle — a name
//! that resolved to only the inbound channel would let a client send but never
//! receive the reply.
//!
//! A linear scan over `NENT` entries (single-digit; no hash, no tree — the
//! tree is stage 7, with 9P).  It lives entirely in the nameserver's own
//! address space (user space), not the kernel.  It is pure `core` (no `std`,
//! no `alloc`) and kept in its own module so it is separable from the syscall
//! shim.  A host unit test is not hosted here: a `no_std` / `no_main` bin
//! cannot carry a `#[cfg(test)]` module (the crate's `#[panic_handler]`
//! collides with the test harness's `std`), so the table's behaviour is
//! covered end-to-end by the `namespace` image (task 4) instead.

/// A channel pair: the server's inbound channel (clients send here) and
/// outbound channel (clients receive the reply here).
#[derive(Clone, Copy, Debug)]
pub struct Pair {
    pub in_h: u32,
    pub out_h: u32,
}

/// The result codes a nameserver reply carries in its opcode.  They share the
/// opcode field with the request verbs but live in the reply message, so the
/// numbers may overlap; a client reads the reply's opcode as the result.
pub const R_OK: u16 = 0;
pub const R_ENOENT: u16 = 1;
pub const R_EFULL: u16 = 2;

/// A name's maximum length: 32 covers `/dev/console` and friends with room.
const NAME_MAX: usize = 32;
/// The bind table's size: a server's bind set is tiny, so a fixed array (no
/// alloc).
const NENT: usize = 8;

/// A bind-table entry: a name and the channel pair bound to it.
#[derive(Debug)]
struct Entry {
    name: [u8; NAME_MAX],
    namelen: u8,
    pair: Pair,
    used: bool,
}

impl Entry {
    const EMPTY: Entry =
        Entry { name: [0u8; NAME_MAX], namelen: 0, pair: Pair { in_h: 0, out_h: 0 }, used: false };

    /// True if this entry is used and bound to `name` (exact, NUL-free
    /// comparison).
    fn matches(&self, name: &[u8]) -> bool {
        self.used && self.namelen as usize == name.len() && self.name[..name.len()] == name[..]
    }
}

/// The bind table: a fixed `name -> channel-pair` map.
#[derive(Debug)]
pub struct BindTable {
    entries: [Entry; NENT],
}

impl BindTable {
    pub const fn new() -> Self {
        Self { entries: [Entry::EMPTY; NENT] }
    }

    /// `BIND`: add or replace the entry for `name` with `pair`.  Returns
    /// [`R_OK`], or [`R_EFULL`] if the table is full with no free slot.
    pub fn bind(&mut self, name: &[u8], pair: Pair) -> u16 {
        let namelen = name.len().min(NAME_MAX);
        // Replace an existing entry for the same name in place.
        for e in &mut self.entries {
            if e.used && e.namelen as usize == namelen && e.name[..namelen] == name[..namelen] {
                e.pair = pair;
                return R_OK;
            }
        }
        // No existing entry: take a free slot.
        for e in &mut self.entries {
            if !e.used {
                e.name[..namelen].copy_from_slice(&name[..namelen]);
                e.namelen = namelen as u8;
                e.pair = pair;
                e.used = true;
                return R_OK;
            }
        }
        R_EFULL
    }

    /// `RESOLVE`: the pair bound to `name`, if any.
    pub fn resolve(&self, name: &[u8]) -> Option<Pair> {
        self.entries.iter().find(|e| e.matches(name)).map(|e| e.pair)
    }

    /// `UNBIND`: clear the entry for `name`.  Returns [`R_OK`], or
    /// [`R_ENOENT`] if no entry is bound to it.
    pub fn unbind(&mut self, name: &[u8]) -> u16 {
        for e in &mut self.entries {
            if e.matches(name) {
                e.used = false;
                return R_OK;
            }
        }
        R_ENOENT
    }
}
