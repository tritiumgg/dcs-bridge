//! The atomics, the cell, the channel and the thread the rings and the writer
//! thread are built from, or Loom's models of them.
//!
//! A test on the three CI hosts cannot establish that a lock-free structure is
//! correct. Two of the three are x86-64, where a missing acquire-release pair is
//! unobservable in principle, and x86-64 Windows is the only target that ships,
//! so the host most able to expose an ordering fault is the one that never runs
//! the product. Loom enumerates the interleavings instead, exhaustively, over a
//! ring small enough to exhaust.
//!
//! What that establishes is the ownership protocol rather than the orderings.
//! Slots change hands through read-modify-writes, and Loom gives one of those
//! more causality than the memory model promises, so a publish written
//! `Relaxed` passes its model. Miri is what reads the unsafe blocks, and the
//! argument for each ordering is written beside it.
//!
//! Loom can only see accesses that go through its own types, so the code under
//! test has to name these rather than `std`'s. The shape is Loom's: a cell hands
//! out its pointer inside a closure, because that is where Loom records the
//! access. The plain build compiles that down to what it would have written
//! anyway.
//!
//! Building with Loom is `RUSTFLAGS="--cfg loom"`, which `mise run loom` sets.

#[cfg(loom)]
pub use loom::sync::atomic::{AtomicBool, AtomicU64, Ordering, fence};
#[cfg(loom)]
pub use loom::sync::{Arc, mpsc};
#[cfg(loom)]
pub use loom::thread;
#[cfg(not(loom))]
pub use std::sync::atomic::{AtomicBool, AtomicU64, Ordering, fence};
#[cfg(not(loom))]
pub use std::sync::{Arc, mpsc};
#[cfg(not(loom))]
pub use std::thread;

#[cfg(loom)]
pub use loom::cell::UnsafeCell;

/// A cell whose contents one thread reaches at a time, by its own arrangement.
///
/// Carries Loom's interface so that the ring reads the same under both builds:
/// the pointer is handed to a closure rather than returned, which is how Loom
/// knows the access has ended.
#[cfg(not(loom))]
#[derive(Debug, Default)]
pub struct UnsafeCell<T>(std::cell::UnsafeCell<T>);

#[cfg(not(loom))]
impl<T> UnsafeCell<T> {
    /// Put a value in a cell.
    pub const fn new(value: T) -> Self {
        Self(std::cell::UnsafeCell::new(value))
    }

    /// Reach the value through a shared reference to the cell.
    ///
    /// Handing out the pointer is safe; using it is what carries the obligation
    /// that no other thread is touching the cell while the closure runs, and
    /// that obligation is argued where the pointer is dereferenced.
    pub fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
        f(self.0.get())
    }
}
