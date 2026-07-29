//! Set-once cells.
//!
//! The kernel is full of state that is published once during boot and
//! then only read: drivers found in the device tree, architecture
//! hooks, tables sized from firmware.  A lock is the wrong shape for
//! it — there is nothing to serialise after init, but the readers still
//! need to know whether publication has happened, and to see a fully
//! constructed value when it has.
//!
//! `Once<T>` is that shape.  Storage is inline, so a `Once` in a static
//! needs no allocator and leaks nothing; `get` is a single acquire
//! load, cheap enough for an interrupt path; and initialisation is
//! elected by a compare-exchange, so two cores racing to initialise
//! produce one winner and one `Err`, never a torn value.
//!
//! `core` has no equivalent: `OnceCell` is `!Sync` and `OnceLock` is
//! `std`-only.

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU8, Ordering};

const UNINIT: u8 = 0;
const WRITING: u8 = 1;
const READY: u8 = 2;

/// A cell that can be written once and read freely thereafter.
pub struct Once<T> {
    state: AtomicU8,
    value: UnsafeCell<MaybeUninit<T>>,
}

// SAFETY: `value` is written by exactly one core — the one that wins
// the UNINIT -> WRITING compare-exchange — and is never written again.
// A reader only forms a reference to it after an acquire load observes
// READY, which is released by the writer after the write completes, so
// no read can overlap the write.  Sharing `&T` across cores needs
// `T: Sync`; moving the value to the writing core needs `T: Send`.
unsafe impl<T: Send + Sync> Sync for Once<T> {}

impl<T> Once<T> {
    pub const fn new() -> Self {
        Self { state: AtomicU8::new(UNINIT), value: UnsafeCell::new(MaybeUninit::uninit()) }
    }

    /// Publish `value`, returning a reference to it.
    ///
    /// Returns `Err(value)` — handed back, not dropped — when the cell
    /// is not ours to write.  That covers two cases the caller cannot
    /// currently tell apart: the value was already published, and
    /// another core is publishing it right now.  In the second case a
    /// following `get` still returns `None`, so `Err` must not be read
    /// as "already visible".
    ///
    /// The contract that ambiguity narrows this type to: a `Once` is
    /// set by one core, before any other core can observe it — boot-core
    /// publication of state that secondaries later only read.  A caller
    /// that genuinely races another writer and must then use the value
    /// needs a `wait` or `get_or_init` this type does not have; add one
    /// when a second writer exists, rather than guessing the policy now.
    pub fn set(&self, value: T) -> Result<&T, T> {
        // Relaxed on both paths: this compare-exchange only elects a
        // writer.  All publication ordering is carried by the
        // release/acquire pair on READY below.
        if self
            .state
            .compare_exchange(UNINIT, WRITING, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return Err(value);
        }

        // SAFETY: we won the election, so we are the only writer, and
        // no reader can be looking: `get` requires READY, which is not
        // stored until after this write completes.
        unsafe { (*self.value.get()).write(value) };
        self.state.store(READY, Ordering::Release);

        // SAFETY: initialised immediately above.
        Ok(unsafe { (*self.value.get()).assume_init_ref() })
    }

    /// The published value, or `None` if it has not been set yet.
    pub fn get(&self) -> Option<&T> {
        if self.state.load(Ordering::Acquire) != READY {
            return None;
        }
        // SAFETY: READY was observed through an acquire load paired
        // with the release store in `set`, so the write of the value
        // happens-before this read, and nothing writes it again.
        Some(unsafe { (*self.value.get()).assume_init_ref() })
    }
}

impl<T> Default for Once<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Drop for Once<T> {
    fn drop(&mut self) {
        // Dead code for the intended use — a `Once` in a static never
        // drops — but a `Once` that silently leaked its value anywhere
        // else would be a trap.  A WRITING cell cannot be dropped: the
        // writer holds `&self` for the duration.
        if *self.state.get_mut() == READY {
            // SAFETY: READY means the value was initialised, and `&mut
            // self` means no reader can hold a reference to it.
            unsafe { self.value.get_mut().assume_init_drop() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Once;
    use alloc::boxed::Box;
    use core::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn get_before_set_is_none() {
        let once: Once<u32> = Once::new();
        assert_eq!(once.get(), None);
    }

    #[test]
    fn set_then_get() {
        let once: Once<u32> = Once::new();
        assert_eq!(once.set(42), Ok(&42));
        assert_eq!(once.get(), Some(&42));
    }

    #[test]
    fn second_set_hands_the_value_back() {
        let once: Once<u32> = Once::new();
        assert_eq!(once.set(1), Ok(&1));
        assert_eq!(once.set(2), Err(2));
        // The first value stands.
        assert_eq!(once.get(), Some(&1));
    }

    #[test]
    fn reference_from_set_and_get_alias() {
        let once: Once<u32> = Once::new();
        let from_set = once.set(7).unwrap();
        let from_get = once.get().unwrap();
        assert!(core::ptr::eq(from_set, from_get));
    }

    #[test]
    fn holds_a_type_with_no_valid_zero_value() {
        // A niche-bearing type: all-zero bytes would not be a valid
        // `Box`, so this only works because storage is `MaybeUninit`.
        let once: Once<Box<u32>> = Once::new();
        once.set(Box::new(9)).unwrap();
        assert_eq!(**once.get().unwrap(), 9);
    }

    static DROPS: AtomicUsize = AtomicUsize::new(0);

    #[derive(Debug)]
    struct CountsDrops;

    impl Drop for CountsDrops {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn dropping_a_set_cell_drops_the_value() {
        {
            let once: Once<CountsDrops> = Once::new();
            once.set(CountsDrops).unwrap();
            assert_eq!(DROPS.load(Ordering::Relaxed), 0);
        }
        assert_eq!(DROPS.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn concurrent_set_has_exactly_one_winner() {
        use std::sync::Arc;
        use std::thread;

        // The property the type exists for.  Repeated, because a single
        // run can miss the race by luck.
        for _ in 0..64 {
            let once: Arc<Once<usize>> = Arc::new(Once::new());
            let winners = Arc::new(AtomicUsize::new(0));
            let racers: Vec<_> = (0..8)
                .map(|i| {
                    let once = Arc::clone(&once);
                    let winners = Arc::clone(&winners);
                    thread::spawn(move || {
                        if once.set(i).is_ok() {
                            winners.fetch_add(1, Ordering::Relaxed);
                        }
                    })
                })
                .collect();
            for racer in racers {
                racer.join().unwrap();
            }
            assert_eq!(winners.load(Ordering::Relaxed), 1);
            // Once every writer has finished, the published value is
            // one of the values offered, and is stable.
            assert!(once.get().is_some_and(|v| *v < 8));
        }
    }

    #[test]
    fn dropping_an_unset_cell_drops_nothing() {
        // No value was ever written, so there is nothing to drop and
        // nothing to read: this must not touch the uninitialised bytes.
        let once: Once<CountsDrops> = Once::new();
        drop(once);
    }
}
