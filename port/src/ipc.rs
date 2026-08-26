//! In-kernel message passing: the microkernel's central primitive.
//!
//! A [`Channel`] is a bounded, **unidirectional** message queue with a wait
//! set: a sender's [`send`] enqueues a [`Message`] (or hands it straight to a
//! blocked receiver), and a receiver's [`receive`] dequeues one (or blocks
//! until one arrives).  Directionality is what makes request/reply
//! well-defined over a pair of channels: the client sends on one, the server
//! on the other, so a client never pops its own request before the server's
//! reply (a single shared FIFO would).
//!
//! The module is arch-agnostic.  It knows a process only as a [`ProcId`] and
//! schedules through an [`IpcScheduler`] (block / wake / boost).  The blocking
//! is the *scheduler's* blocking, not a mechanism this module invents: a
//! process that blocks in `receive` is put off the ready set by the arch and
//! back on it by a matching `send` — the same `resched`/`swtch` path a yield
//! uses, with a `Blocked` state in between.
//!
//! The message *moves* across the boundary: a `Message` has one owner at a
//! time — a queue slot, or the handoff to a single woken receiver — never a
//! shared reference.  That is the "share memory by communicating" discipline,
//! enforced by construction, not by a permission check.  The kernel sees the
//! opcode and the tag; the payload is opaque (a server's protocol is its own
//! encoding of `buf`).

// Re-exported from `r9x_abi`, the single source both the kernel and the
// `r9x_std` target read, so build, loader, and servers cannot drift (a
// pinning test asserts they match).  The uses below (`[u8; MSG_MAX]`, …) refer
// to this re-export.
pub use r9x_abi::MSG_MAX;
pub use r9x_abi::RECEIVE_TIMEOUT;

/// The bounded, tagged, fixed message.  `opcode` dispatches, `tag` correlates
/// a reply to its request, `buf`/`len` carry the (opaque) payload.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Message {
    pub opcode: u16,
    pub tag: u32,
    pub len: u16,
    pub buf: [u8; MSG_MAX],
}

impl Message {
    /// A message with the given opcode/tag and a payload copied from `data`
    /// (truncated to `MSG_MAX`).
    pub fn new(opcode: u16, tag: u32, data: &[u8]) -> Message {
        let mut buf = [0u8; MSG_MAX];
        let n = data.len().min(MSG_MAX);
        buf[..n].copy_from_slice(&data[..n]);
        Message { opcode, tag, len: n as u16, buf }
    }
}

/// The error a send/receive/reply can return.
///
/// `send` to a full queue *blocks* the sender (total function, no drop mode),
/// so [`Full`] surfaces only in the close-during-full edge.  [`Closed`] is a
/// send/receive/reply on a channel whose owner has gone away.  [`BadTag`] is a
/// reply whose tag no outstanding request carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IpcErr {
    /// The channel is closed (its owner died or closed it).
    Closed,
    /// The queue is full and the channel is closing, so the send cannot block
    /// for a slot that will never free.
    Full,
    /// A reply whose tag no outstanding request carries.
    BadTag,
    /// The queue is empty: a non-blocking [`try_receive`] found no message.
    Empty,
}

/// A process id, as far as the scheduler is concerned: an opaque index.
pub type ProcId = usize;

/// The scheduler seam: everything `port::ipc` needs of an arch to block, wake,
/// and priority-inherit.  The blocking is the arch's own — it puts a process
/// off the ready set and back on it; this module only decides *when* and
/// *whom*.
pub trait IpcScheduler {
    /// The process running this code, if any (`None` in kernel context).
    fn current(&self) -> Option<ProcId>;
    /// The effective priority level of `id` (QNX: 0 most urgent). Called only
    /// for a live process.
    fn priority(&self, id: ProcId) -> u8;
    /// Boost `id` to `to` (more urgent than its base). A no-op unless `to` is
    /// more urgent and the slot is not already boosted (no stacking).
    fn boost(&self, id: ProcId, to: u8);
    /// Restore `id` to its base priority (the inverse of [`boost`](Self::boost)).
    fn unboost(&self, id: ProcId);
    /// Put `id` off the ready set (blocked). Does not return until `id` is
    /// woken and rescheduled.
    fn block(&self, id: ProcId);
    /// Put `id` back on the ready set (it will be picked by the next
    /// selection).
    fn wake(&self, id: ProcId);
    /// The arch's monotonic counter, in ticks: the value a bounded wait's
    /// deadline is measured against.  A register read (no lock, no allocation),
    /// so `receive_at` can call it from the hot path.
    fn now(&self) -> u64;
    /// Put `id` off the ready set with a wake `deadline` (a counter tick):
    /// the arch records the deadline and blocks; the arch's tick wakes the
    /// process when the counter reaches the deadline (or a message arrives
    /// first, which the matching `send` wakes it with).  Does not return until
    /// `id` is woken and rescheduled.
    fn block_at(&self, id: ProcId, deadline: u64);
}

/// The queue depth of a channel: the slots are a fixed array, so a full queue
/// is reported (and the sender blocks), never grown.
const QUEUE_CAP: usize = 8;

/// A bounded FIFO of [`Message`].  The slots are fixed (no allocation); a full
/// queue is reported, not grown.
struct BoundedQueue {
    slots: [Option<Message>; QUEUE_CAP],
    /// Index of the next slot to pop.
    head: usize,
    len: usize,
}

impl BoundedQueue {
    const fn new() -> Self {
        BoundedQueue { slots: [None; QUEUE_CAP], head: 0, len: 0 }
    }

    fn is_full(&self) -> bool {
        self.len == QUEUE_CAP
    }

    /// Push a message; returns `false` (and leaves the queue unchanged) if
    /// full.
    fn push(&mut self, msg: Message) -> bool {
        if self.is_full() {
            return false;
        }
        let tail = (self.head + self.len) % QUEUE_CAP;
        self.slots[tail] = Some(msg);
        self.len += 1;
        true
    }

    /// Pop the head message, if any.
    fn pop(&mut self) -> Option<Message> {
        let msg = self.slots[self.head].take()?;
        self.head = (self.head + 1) % QUEUE_CAP;
        self.len -= 1;
        Some(msg)
    }
}

/// The channel's mutable state: the queue, the one receiver blocked waiting
/// for a message, the one sender blocked on a full queue, and the close flag.
/// Guarded by the channel's lock; never held across an
/// [`IpcScheduler::block`] (which switches).
struct ChannelInner {
    queue: BoundedQueue,
    /// The one receiver blocked in `receive` (stage 2: a channel serves one
    /// blocked receiver, the server).  More than one is a later concern.
    recv_waiter: Option<ProcId>,
    /// The one sender blocked on a full queue, woken when a receive frees a
    /// slot.
    send_waiter: Option<ProcId>,
    closed: bool,
}

impl ChannelInner {
    const fn new() -> Self {
        ChannelInner {
            queue: BoundedQueue::new(),
            recv_waiter: None,
            send_waiter: None,
            closed: false,
        }
    }
}

/// A channel: a bounded, unidirectional message queue plus its wait set, owned
/// by a process (it closes when the owner goes away).  The queue slots are
/// pre-allocated (a fixed array), so no allocation is reachable in
/// `send`/`receive`/`reply`.
pub struct Channel {
    inner: crate::mcslock::Lock<ChannelInner>,
    owner: ProcId,
}

impl Channel {
    /// An open channel owned by `owner`.  No allocation: the queue and wait
    /// slots are a fixed array inside the lock.
    pub const fn new(owner: ProcId) -> Channel {
        Channel { inner: crate::mcslock::Lock::new("ipc", ChannelInner::new()), owner }
    }

    /// The channel's owner.
    pub fn owner(&self) -> ProcId {
        self.owner
    }
}

/// Close the channel through `sched`: it is marked closed, a blocked receiver
/// wakes to [`IpcErr::Closed`], and a sender blocked on a full queue wakes to
/// [`IpcErr::Full`] (a slot that will never free).  Idempotent.
pub fn close<S: IpcScheduler>(sched: &S, ch: &Channel) {
    let node = crate::mcslock::LockNode::new();
    let (recv, sendw) = {
        let mut inner = ch.inner.lock(&node);
        inner.closed = true;
        (inner.recv_waiter.take(), inner.send_waiter.take())
    };
    if let Some(r) = recv {
        sched.wake(r);
    }
    if let Some(s) = sendw {
        sched.wake(s);
    }
}

/// Send a message on `ch` through `sched`.
///
/// Two shapes:
/// - **fast** — a receiver is blocked on the channel: the message is enqueued
///   and that receiver is woken; if the *sender* outranks the *receiver* (lower
///   level number, QNX convention) the **receiver** is boosted to the sender's
///   level for the exchange (server-at-client) and unboosted when it next
///   blocks.  This is where priority inheritance triggers.
/// - **slow** — no receiver blocked: the message is enqueued and the send
///   returns.
///
/// A send to a *full* queue blocks the sender (total function, no drop mode)
/// until a receive frees a slot.  A send on a closed channel returns
/// [`IpcErr::Closed`].
pub fn send<S: IpcScheduler>(
    sched: &S,
    ch: &Channel,
    msg: Message,
) -> core::result::Result<(), IpcErr> {
    let node = crate::mcslock::LockNode::new();
    loop {
        let fast = {
            let mut inner = ch.inner.lock(&node);
            if inner.closed {
                return Err(IpcErr::Closed);
            }
            if let Some(receiver) = inner.recv_waiter.take() {
                // Fast path: a receiver is blocked; hand it the message and
                // wake it.  PI (server-at-client): if the sender outranks the
                // receiver, boost the receiver to the sender's level.  The
                // channel lock is held, so the handoff is atomic.
                if let Some(sender) = sched.current()
                    && sched.priority(sender) < sched.priority(receiver)
                {
                    sched.boost(receiver, sched.priority(sender));
                }
                debug_assert!(inner.queue.push(msg));
                Some(receiver)
            } else if inner.queue.push(msg) {
                // Slow path: no receiver blocked, room in the queue.
                None
            } else {
                // Full: the sender will block; record it and switch out.
                inner.send_waiter = Some(sched.current().expect("send needs a process"));
                None
            }
        };

        match fast {
            Some(receiver) => {
                // The lock is dropped before the switch: the woken receiver's
                // own receive must not self-deadlock on the channel lock.
                sched.wake(receiver);
                return Ok(());
            }
            None => {
                // Slow path (message enqueued): done.  The full-queue case is
                // told apart by the send-waiter we just recorded.
                let blocked = {
                    let inner = ch.inner.lock(&node);
                    inner.send_waiter.is_some()
                };
                if !blocked {
                    return Ok(());
                }
                let me = sched.current().expect("send needs a process");
                // Drop the lock, block (switch away); a receive frees a slot
                // and wakes us, and we retry with room to push.
                sched.block(me);
            }
        }
    }
}

/// Send a message on `ch` through `sched` without blocking: the interrupt-
/// context variant of [`send`].  The fast path (a receiver is blocked) is the
/// same as `send`: hand the message and wake it.  No PI: the caller is the
/// kernel (not a process), so there is no client priority to inherit.  The
/// slow path (no receiver blocked, room in the queue) enqueues and returns
/// `Ok(())`.  The full-queue path returns [`IpcErr::Full`]: the message is
/// lost, not retried (the Amiga's answer — a lost display-refresh interrupt
/// is acceptable; a lost input is the server's problem, it polls the device).
///
/// This function never blocks and never allocates: it is safe to call in
/// interrupt context.
pub fn try_send<S: IpcScheduler>(
    sched: &S,
    ch: &Channel,
    msg: Message,
) -> core::result::Result<(), IpcErr> {
    let node = crate::mcslock::LockNode::new();
    let fast = {
        let mut inner = ch.inner.lock(&node);
        if inner.closed {
            return Err(IpcErr::Closed);
        }
        if let Some(receiver) = inner.recv_waiter.take() {
            // Fast path: a receiver is blocked; hand it the message and wake
            // it.  No PI (the caller is the kernel, not a process).
            debug_assert!(inner.queue.push(msg));
            Some(receiver)
        } else if inner.queue.push(msg) {
            // Slow path: no receiver blocked, room in the queue.
            None
        } else {
            // Full: the message is lost (no allocation, no blocking).
            return Err(IpcErr::Full);
        }
    };
    if let Some(receiver) = fast {
        sched.wake(receiver);
    }
    Ok(())
}

/// Receive a message from `ch` through `sched` without blocking: the
/// interrupt-context / init-context variant of [`receive`].  Dequeues a
/// message if one is queued (waking a sender blocked on a full queue),
/// returns [`IpcErr::Empty`] if the queue is empty, and
/// [`IpcErr::Closed`] if the channel is closed.  Does not require a current
/// process: the caller is the kernel, not a process.
pub fn try_receive<S: IpcScheduler>(
    sched: &S,
    ch: &Channel,
) -> core::result::Result<Message, IpcErr> {
    let node = crate::mcslock::LockNode::new();
    let (msg, sendw) = {
        let mut inner = ch.inner.lock(&node);
        if inner.closed {
            return Err(IpcErr::Closed);
        }
        match inner.queue.pop() {
            Some(m) => (Some(m), inner.send_waiter.take()),
            None => (None, None),
        }
    };
    match msg {
        Some(m) => {
            // A slot freed: wake a sender blocked on a full queue, if any.
            if let Some(s) = sendw {
                sched.wake(s);
            }
            Ok(m)
        }
        None => Err(IpcErr::Empty),
    }
}

/// Receive a message from `ch` through `sched`.
///
/// Dequeues a message if one is queued (waking a sender blocked on a full
/// queue), blocks until one arrives otherwise (the receiver is put off the
/// ready set and woken by a `send`), and returns [`IpcErr::Closed`] if the
/// channel is closed.  A receiver that blocks drops any priority-inheritance
/// boost it holds: it runs at its base priority while waiting, and is
/// re-boosted by the next `send` that outranks it.
pub fn receive<S: IpcScheduler>(sched: &S, ch: &Channel) -> core::result::Result<Message, IpcErr> {
    let node = crate::mcslock::LockNode::new();
    loop {
        let me = sched.current().expect("receive needs a process");
        let (msg, sendw, blocked) = {
            let mut inner = ch.inner.lock(&node);
            if inner.closed {
                return Err(IpcErr::Closed);
            }
            match inner.queue.pop() {
                Some(m) => (Some(m), inner.send_waiter.take(), false),
                None => {
                    // Empty: block.  Drop any PI boost (see docs) and record
                    // as the receiver.
                    sched.unboost(me);
                    inner.recv_waiter = Some(me);
                    (None, None, true)
                }
            }
        };
        if let Some(m) = msg {
            // A slot freed: wake a sender blocked on a full queue, if any.
            if let Some(s) = sendw {
                sched.wake(s);
            }
            return Ok(m);
        }
        if blocked {
            // The lock is dropped before the switch (a woken receiver's own
            // receive must not self-deadlock).  Block until a send wakes us.
            sched.block(me);
        }
    }
}

/// Receive a message from `ch` through `sched`, bounded by `deadline` (a
/// counter tick, measured against [`IpcScheduler::now`]).  Like [`receive`],
/// but the wait is bounded: the process is woken when a message arrives or the
/// counter reaches `deadline`, whichever is first.  On a message, returns it
/// (the message beat the deadline).  On a deadline with no message, returns a
/// message carrying [`RECEIVE_TIMEOUT`] as its opcode (the timeout beat the
/// message); a closed channel still returns [`IpcErr::Closed`].
///
/// The wait does not spin: an empty queue with a future deadline puts the
/// process off the ready set (a bounded `block`); the arch's tick wakes it at
/// the deadline, and a `send`'s fast path wakes it earlier.  A deadline that
/// has already passed (a `deadline <= now()`) never blocks: the timeout
/// returns immediately.
pub fn receive_at<S: IpcScheduler>(
    sched: &S,
    ch: &Channel,
    deadline: u64,
) -> core::result::Result<Message, IpcErr> {
    let node = crate::mcslock::LockNode::new();
    let timeout = Message { opcode: RECEIVE_TIMEOUT, tag: 0, len: 0, buf: [0; MSG_MAX] };
    loop {
        let me = sched.current().expect("receive_at needs a process");
        let (msg, sendw, blocked) = {
            let mut inner = ch.inner.lock(&node);
            if inner.closed {
                return Err(IpcErr::Closed);
            }
            match inner.queue.pop() {
                Some(m) => (Some(m), inner.send_waiter.take(), false),
                None => {
                    // Empty: a message did not beat the deadline.  A deadline
                    // already reached (or past) is an immediate timeout; a
                    // future deadline blocks until the deadline or a message.
                    if sched.now() >= deadline {
                        return Ok(timeout);
                    }
                    // Future: drop any PI boost (as `receive`) and record as
                    // the receiver.  The deadline is set by the arch when it
                    // blocks (below); the tick or a `send` clears it on wake.
                    sched.unboost(me);
                    inner.recv_waiter = Some(me);
                    (None, None, true)
                }
            }
        };
        if let Some(m) = msg {
            // A slot freed: wake a sender blocked on a full queue, if any.
            if let Some(s) = sendw {
                sched.wake(s);
            }
            return Ok(m);
        }
        if blocked {
            // The lock is dropped before the switch (a woken receiver's own
            // receive must not self-deadlock).  Block until a send or the
            // deadline wakes us.
            sched.block_at(me, deadline);
        }
    }
}

/// A tag-correlated `send`: the reply to the request with `tag`.  The kernel
/// is opaque to the protocol and does not track outstanding tags across a
/// connection (a request and its reply ride different channels of the pair),
/// so matching a reply to its request is the application's job.  What the
/// kernel *does* enforce is the reply's internal consistency: the reply
/// message must carry the tag it is replying to.  A `reply` whose message tag
/// differs from `tag` is a [`IpcErr::BadTag`] (a malformed reply) and is not
/// sent.
pub fn reply<S: IpcScheduler>(
    sched: &S,
    ch: &Channel,
    tag: u32,
    msg: Message,
) -> core::result::Result<(), IpcErr> {
    if msg.tag != tag {
        return Err(IpcErr::BadTag);
    }
    send(sched, ch, msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A cooperative mock scheduler for host tests.  There is no real
    /// concurrency on the host, so the arch's "switch away and run the other
    /// process" is simulated: when a process blocks, the scheduler runs the
    /// test-provided `on_block` action (the *other* process's work, which
    /// wakes the blocker).  The mock owns the channel under test, so the
    /// actions reference `m.channel` (no capture of a non-'static borrow).
    /// `priority`/`boost`/`unboost` mirror the real semantics (lower level =
    /// more urgent; boost takes the lower level) and record calls so the tests
    /// can assert PI fired.
    struct Mock {
        current: RefCell<Option<ProcId>>,
        base: Vec<u8>,
        eff: RefCell<Vec<u8>>,
        blocks: RefCell<Vec<ProcId>>,
        boosts: RefCell<Vec<(ProcId, u8)>>,
        unboosts: RefCell<Vec<ProcId>>,
        /// The mock counter (`now`): a test sets it to drive a deadline.
        counter: RefCell<u64>,
        channel: Channel,
        #[allow(clippy::type_complexity)]
        on_block: RefCell<Option<Rc<dyn Fn(&Mock)>>>,
    }

    impl Mock {
        fn new(base: &[u8], channel: Channel) -> Self {
            Mock {
                current: RefCell::new(None),
                base: base.to_vec(),
                eff: RefCell::new(base.to_vec()),
                blocks: RefCell::new(vec![]),
                boosts: RefCell::new(vec![]),
                unboosts: RefCell::new(vec![]),
                counter: RefCell::new(0),
                channel,
                on_block: RefCell::new(None),
            }
        }

        /// Set the mock counter (a test advances it to pass a deadline).
        fn set_counter(&self, value: u64) {
            *self.counter.borrow_mut() = value;
        }

        /// Run `body` as process `id` (the current process for its duration).
        fn run(&self, id: ProcId, body: impl FnOnce(&Mock)) {
            let prev = self.current.replace(Some(id));
            body(self);
            *self.current.borrow_mut() = prev;
        }
    }

    impl IpcScheduler for Mock {
        fn current(&self) -> Option<ProcId> {
            *self.current.borrow()
        }
        fn priority(&self, id: ProcId) -> u8 {
            self.eff.borrow()[id]
        }
        fn boost(&self, id: ProcId, to: u8) {
            self.boosts.borrow_mut().push((id, to));
            let mut eff = self.eff.borrow_mut();
            if to < eff[id] {
                eff[id] = to;
            }
        }
        fn unboost(&self, id: ProcId) {
            self.unboosts.borrow_mut().push(id);
            self.eff.borrow_mut()[id] = self.base[id];
        }
        fn block(&self, id: ProcId) {
            self.blocks.borrow_mut().push(id);
            // Simulate the switch: run the other process's work (it wakes
            // `id`).  Clone the action out first so the borrow of `on_block`
            // does not live across the action's own borrows of `self`.
            let action = self.on_block.borrow().clone();
            if let Some(action) = action {
                action(self);
            }
        }
        fn wake(&self, id: ProcId) {
            let _ = id; // a woken process is resumed by the test's structure
        }
        fn now(&self) -> u64 {
            *self.counter.borrow()
        }
        fn block_at(&self, id: ProcId, _deadline: u64) {
            // A bounded wait is the same switch as a plain block: the test's
            // `on_block` action (or the counter) is what wakes the blocker.
            self.block(id);
        }
    }

    const CLIENT: ProcId = 0; // high urgency (low level)
    const SERVER: ProcId = 1; // low urgency (high level)

    #[test]
    fn fifo_ordering() {
        let m = Mock::new(&[128, 128, 128, 128], Channel::new(CLIENT));
        m.run(CLIENT, |m| {
            for tag in 0..3u32 {
                send(m, &m.channel, Message::new(1, tag, &[tag as u8])).unwrap();
            }
        });
        m.run(SERVER, |m| {
            for want in 0..3u32 {
                let msg = receive(m, &m.channel).unwrap();
                assert_eq!(msg.tag, want, "FIFO: tag {want} out of order");
            }
        });
    }

    #[test]
    fn full_queue_blocks_sender_until_receive_drains() {
        let m = Mock::new(&[128, 128, 128, 128], Channel::new(CLIENT));
        m.run(CLIENT, |m| {
            for tag in 0..QUEUE_CAP as u32 {
                send(m, &m.channel, Message::new(1, tag, &[])).unwrap();
            }
        });
        // One more send must block: while the client is blocked, the server's
        // receive drains a slot and the (simulated) switch resumes the client.
        m.on_block.replace(Some(Rc::new(|m: &Mock| {
            m.run(SERVER, |m| {
                let _ = receive(m, &m.channel).unwrap();
            });
        })));
        m.run(CLIENT, |m| {
            send(m, &m.channel, Message::new(1, 99, &[])).unwrap();
        });
        assert!(!m.blocks.borrow().is_empty(), "the full send must have blocked");
    }

    #[test]
    fn fast_path_hands_off_exact_bytes() {
        let m = Mock::new(&[128, 128, 128, 128], Channel::new(CLIENT));
        let payload: [u8; 16] = [0xde, 0xad, 0xbe, 0xef, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        m.on_block.replace(Some(Rc::new(move |m: &Mock| {
            m.run(CLIENT, |m| {
                send(m, &m.channel, Message::new(1, 7, &payload)).unwrap();
            });
        })));
        m.run(SERVER, |m| {
            let msg = receive(m, &m.channel).unwrap();
            assert_eq!(msg.tag, 7);
            assert_eq!(&msg.buf[..msg.len as usize], &payload, "fast path: exact bytes");
        });
    }

    #[test]
    fn boost_fires_when_sender_outranks_receiver() {
        // Client at level 16 (urgent), server at level 200.  The server blocks
        // in receive; the client's send is the fast path and boosts the server
        // to the client's level; the server's next block unboosts it.
        let m = Mock::new(&[16, 200, 128, 128], Channel::new(CLIENT));
        m.on_block.replace(Some(Rc::new(|m: &Mock| {
            m.run(CLIENT, |m| {
                send(m, &m.channel, Message::new(1, 1, &[])).unwrap();
            });
        })));
        m.run(SERVER, |m| {
            let _ = receive(m, &m.channel).unwrap(); // first: boosted to the client's level
            assert_eq!(m.priority(SERVER), 16, "server must run at the client's level");
            // The second receive blocks (queue empty): the server unboosts to
            // its base.  A close wakes it to Closed.
            m.on_block.replace(Some(Rc::new(|m: &Mock| {
                m.run(CLIENT, |m| {
                    close(m, &m.channel);
                });
            })));
            assert_eq!(receive(m, &m.channel), Err(IpcErr::Closed));
            assert_eq!(m.priority(SERVER), 200, "server must be unboosted to base");
        });
        assert!(
            m.boosts.borrow().contains(&(SERVER, 16)),
            "server must be boosted to the client's level; boosts: {:?}",
            m.boosts.borrow()
        );
        assert!(!m.unboosts.borrow().is_empty(), "server block must unboost");
    }

    #[test]
    fn no_boost_when_priorities_tie() {
        let m = Mock::new(&[128, 128, 128, 128], Channel::new(CLIENT));
        m.on_block.replace(Some(Rc::new(|m: &Mock| {
            m.run(CLIENT, |m| {
                send(m, &m.channel, Message::new(1, 1, &[])).unwrap();
            });
        })));
        m.run(SERVER, |m| {
            let _ = receive(m, &m.channel).unwrap();
        });
        assert!(m.boosts.borrow().is_empty(), "no boost when priorities tie");
    }

    #[test]
    fn reply_tag_correlation() {
        let m = Mock::new(&[128, 128, 128, 128], Channel::new(CLIENT));
        m.run(SERVER, |m| {
            reply(m, &m.channel, 42, Message::new(2, 42, &[])).unwrap();
        });
        m.run(CLIENT, |m| {
            let msg = receive(m, &m.channel).unwrap();
            assert_eq!(msg.tag, 42);
            assert_eq!(msg.opcode, 2);
        });
    }

    #[test]
    fn reply_mismatched_tag_is_bad_tag() {
        let m = Mock::new(&[128, 128, 128, 128], Channel::new(CLIENT));
        m.run(SERVER, |m| {
            assert_eq!(
                reply(m, &m.channel, 42, Message::new(2, 41, &[])),
                Err(IpcErr::BadTag),
                "a reply whose tag does not match the message's is malformed"
            );
        });
    }

    #[test]
    fn closed_channel_wakes_receiver_and_fails_send() {
        let m = Mock::new(&[128, 128, 128, 128], Channel::new(CLIENT));
        m.on_block.replace(Some(Rc::new(|m: &Mock| {
            m.run(CLIENT, |m| {
                close(m, &m.channel);
            });
        })));
        m.run(SERVER, |m| {
            assert_eq!(receive(m, &m.channel), Err(IpcErr::Closed));
        });
        m.run(CLIENT, |m| {
            assert_eq!(send(m, &m.channel, Message::new(1, 1, &[])), Err(IpcErr::Closed));
        });
    }

    #[test]
    fn try_send_slow_path_enqueues() {
        // No receiver blocked, room in the queue: the message is enqueued.
        let m = Mock::new(&[128, 128, 128, 128], Channel::new(CLIENT));
        assert_eq!(try_send(&m, &m.channel, Message::new(1, 1, &[])), Ok(()));
        m.run(SERVER, |m| {
            let msg = receive(m, &m.channel).unwrap();
            assert_eq!(msg.tag, 1);
        });
    }

    #[test]
    fn try_send_fast_path_wakes_receiver() {
        // A receiver is blocked: the message is handed to it and it is woken.
        let m = Mock::new(&[128, 128, 128, 128], Channel::new(CLIENT));
        m.on_block.replace(Some(Rc::new(|m: &Mock| {
            // The "interrupt" arrives while the server is blocked.
            assert_eq!(try_send(m, &m.channel, Message::new(1, 7, &[])), Ok(()));
        })));
        m.run(SERVER, |m| {
            let msg = receive(m, &m.channel).unwrap();
            assert_eq!(msg.tag, 7);
        });
    }

    #[test]
    fn try_send_full_queue_returns_full() {
        // Fill the queue, then try_send must return Err(Full), not block.
        let m = Mock::new(&[128, 128, 128, 128], Channel::new(CLIENT));
        m.run(CLIENT, |m| {
            for tag in 0..QUEUE_CAP as u32 {
                send(m, &m.channel, Message::new(1, tag, &[])).unwrap();
            }
        });
        // The queue is full: try_send returns Err(Full) without blocking.
        assert_eq!(
            try_send(&m, &m.channel, Message::new(1, 99, &[])),
            Err(IpcErr::Full),
            "full queue: try_send must return Full, not block"
        );
        assert!(m.blocks.borrow().is_empty(), "try_send must not block (no block calls)");
    }

    #[test]
    fn try_send_no_pi() {
        // The sender is the kernel (no current process): no boost fires.
        let m = Mock::new(&[16, 200, 128, 128], Channel::new(CLIENT));
        m.on_block.replace(Some(Rc::new(|m: &Mock| {
            // The "interrupt" arrives while the server (level 200) is blocked.
            // The client (level 16) is not the current process, so no PI.
            assert_eq!(try_send(m, &m.channel, Message::new(1, 1, &[])), Ok(()));
        })));
        m.run(SERVER, |m| {
            let _ = receive(m, &m.channel).unwrap();
        });
        assert!(
            m.boosts.borrow().is_empty(),
            "try_send must not boost (no client priority); boosts: {:?}",
            m.boosts.borrow()
        );
    }

    #[test]
    fn try_send_closed_channel_returns_closed() {
        let m = Mock::new(&[128, 128, 128, 128], Channel::new(CLIENT));
        m.run(CLIENT, |m| {
            close(m, &m.channel);
        });
        assert_eq!(
            try_send(&m, &m.channel, Message::new(1, 1, &[])),
            Err(IpcErr::Closed),
            "closed channel: try_send must return Closed"
        );
    }

    #[test]
    fn receive_at_deadline_already_passed_is_immediate_timeout() {
        // A deadline at or before the counter never blocks: the timeout
        // returns immediately, no block call.
        let m = Mock::new(&[128, 128, 128, 128], Channel::new(CLIENT));
        m.set_counter(50);
        m.run(SERVER, |m| {
            let msg = receive_at(m, &m.channel, 50).unwrap();
            assert_eq!(msg.opcode, RECEIVE_TIMEOUT, "deadline passed: immediate timeout");
            assert_eq!(msg.len, 0, "a timeout carries no payload");
        });
        assert!(m.blocks.borrow().is_empty(), "a passed deadline must not block");
    }

    #[test]
    fn receive_at_timeout_when_deadline_beats_message() {
        // The block's on_block action advances the counter past the deadline
        // (simulating the tick): on re-entry there is no message and the
        // deadline has passed, so the timeout is returned.
        let m = Mock::new(&[128, 128, 128, 128], Channel::new(CLIENT));
        m.on_block.replace(Some(Rc::new(|m: &Mock| {
            m.set_counter(200); // past the deadline of 100
        })));
        m.run(SERVER, |m| {
            let msg = receive_at(m, &m.channel, 100).unwrap();
            assert_eq!(msg.opcode, RECEIVE_TIMEOUT, "deadline beat the message");
        });
        assert_eq!(m.blocks.borrow().len(), 1, "the wait must have blocked once");
    }

    #[test]
    fn receive_at_message_beats_deadline() {
        // The block's on_block action sends a message (simulating the fast
        // path): on re-entry the message is found and returned, even though
        // the deadline is still in the future.
        let m = Mock::new(&[128, 128, 128, 128], Channel::new(CLIENT));
        m.on_block.replace(Some(Rc::new(|m: &Mock| {
            m.run(CLIENT, |m| {
                send(m, &m.channel, Message::new(1, 7, &[42])).unwrap();
            });
        })));
        m.run(SERVER, |m| {
            let msg = receive_at(m, &m.channel, 100).unwrap();
            assert_eq!(msg.opcode, 1, "the message beat the deadline");
            assert_eq!(msg.tag, 7);
            assert_eq!(msg.len, 1);
        });
    }

    #[test]
    fn receive_at_spurious_wake_reblocks() {
        // The block's on_block action does not advance the counter or send a
        // message (a spurious wake): on re-entry there is no message and the
        // deadline has not passed, so the process re-blocks.  The test
        // advances the counter and sends a message from the second on_block
        // to end the loop.
        let m = Mock::new(&[128, 128, 128, 128], Channel::new(CLIENT));
        let calls = Rc::new(RefCell::new(0));
        let calls2 = calls.clone();
        m.on_block.replace(Some(Rc::new(move |m: &Mock| {
            let n = *calls2.borrow();
            *calls2.borrow_mut() = n + 1;
            if n == 0 {
                // First block: spurious wake (no counter advance, no message).
            } else {
                // Second block: advance the counter and send a message.
                m.set_counter(200);
                m.run(CLIENT, |m| {
                    send(m, &m.channel, Message::new(1, 9, &[])).unwrap();
                });
            }
        })));
        m.run(SERVER, |m| {
            let msg = receive_at(m, &m.channel, 100).unwrap();
            assert_eq!(msg.opcode, 1, "the message arrived on the second wake");
            assert_eq!(msg.tag, 9);
        });
        assert_eq!(*calls.borrow(), 2, "the first wake was spurious: the process re-blocked");
    }

    #[test]
    fn receive_at_timeout_opcode_is_reserved() {
        // The timeout opcode is the max u16: a protocol that sends a message
        // with this opcode is ambiguous and must not.
        assert_eq!(RECEIVE_TIMEOUT, 0xffff);
    }

    // ---- priority inheritance: transitive chain ----

    #[test]
    fn pi_transitive_chain_boosts_to_highest() {
        // Three processes: A (level 10, urgent) → B (level 64) → C (level 200).
        // A sends on ch_ab; B (boosted to 10) sends on ch_bc; C must be
        // boosted to B's *effective* level (10), not B's base (64).
        //
        // The mock only tracks one channel, so we simulate the chain by
        // asserting the boost target: when B (now effective 10) sends to C,
        // the boost must be to 10, not 64.
        let m = Mock::new(&[10, 64, 200, 128], Channel::new(CLIENT));
        // Phase 1: A sends to B, boosting B to 10.
        m.on_block.replace(Some(Rc::new(|m: &Mock| {
            m.run(CLIENT, |m| {
                send(m, &m.channel, Message::new(1, 1, &[])).unwrap();
            });
        })));
        m.run(SERVER, |m| {
            let _ = receive(m, &m.channel).unwrap();
            // B is now at effective level 10 (boosted from 64).
            assert_eq!(m.priority(SERVER), 10, "B boosted to A's level");
        });
        // Phase 2: B (effective 10) sends to C.  C (level 200) blocks;
        // B's send is the fast path and must boost C to B's *effective*
        // level (10), not B's base (64).
        let m2 = Mock::new(&[10, 64, 200, 128], Channel::new(CLIENT));
        // Simulate: the "sender" is at level 10 (B's effective after phase 1).
        m2.on_block.replace(Some(Rc::new(|m: &Mock| {
            m.run(CLIENT, |m| {
                send(m, &m.channel, Message::new(1, 2, &[])).unwrap();
            });
        })));
        m2.run(SERVER, |m2| {
            let _ = receive(m2, &m2.channel).unwrap();
            // C must be boosted to 10 (the sender's level), not 64.
            assert_eq!(m2.priority(SERVER), 10, "C boosted transitively to A's level");
        });
        assert!(
            m2.boosts.borrow().contains(&(SERVER, 10)),
            "C must be boosted to 10; boosts: {:?}",
            m2.boosts.borrow()
        );
    }

    #[test]
    fn pi_revert_on_next_block_after_sender_completes() {
        // A (level 16) sends to B (level 200). B is boosted to 16.
        // A completes (no more sends). B blocks again → unboosts to 200.
        // This is the "revert on next block" semantics.
        let m = Mock::new(&[16, 200, 128, 128], Channel::new(CLIENT));
        m.on_block.replace(Some(Rc::new(|m: &Mock| {
            m.run(CLIENT, |m| {
                send(m, &m.channel, Message::new(1, 1, &[])).unwrap();
            });
        })));
        m.run(SERVER, |m| {
            let _ = receive(m, &m.channel).unwrap();
            assert_eq!(m.priority(SERVER), 16, "boosted during exchange");
            // B blocks again (queue empty). A close wakes it to Closed.
            m.on_block.replace(Some(Rc::new(|m: &Mock| {
                m.run(CLIENT, |m| {
                    close(m, &m.channel);
                });
            })));
            assert_eq!(receive(m, &m.channel), Err(IpcErr::Closed));
            assert_eq!(m.priority(SERVER), 200, "reverted to base after block");
        });
        assert!(!m.unboosts.borrow().is_empty(), "at least one unboost (revert on block)");
    }

    // ---- closed channel with queued messages ----

    #[test]
    fn closed_channel_discards_queued_messages() {
        // QNX convention: closing a channel discards any queued messages.
        // A receive after close returns Closed immediately, even if the
        // queue had messages.
        let m = Mock::new(&[128, 128, 128, 128], Channel::new(CLIENT));
        m.run(CLIENT, |m| {
            send(m, &m.channel, Message::new(1, 1, &[1])).unwrap();
            send(m, &m.channel, Message::new(1, 2, &[2])).unwrap();
        });
        m.run(CLIENT, |m| {
            close(m, &m.channel);
        });
        m.run(SERVER, |m| {
            // The channel is closed: the queued messages are discarded.
            assert_eq!(receive(m, &m.channel), Err(IpcErr::Closed));
        });
    }

    #[test]
    fn send_on_closed_channel_returns_closed() {
        let m = Mock::new(&[128, 128, 128, 128], Channel::new(CLIENT));
        m.run(CLIENT, |m| {
            close(m, &m.channel);
        });
        m.run(CLIENT, |m| {
            assert_eq!(send(m, &m.channel, Message::new(1, 1, &[])), Err(IpcErr::Closed));
        });
    }
}
