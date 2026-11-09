//! The `Store` port: where the event log lives.
//!
//! The log is the source of truth and the store is append-only. An
//! append is conditional on the head: the caller folded the log to some
//! `(len, head)`, applied a command, and sealed events that follow that
//! head. If another writer got there first the append is refused with
//! [`StoreError::Conflict`], and the caller reloads, re-applies and
//! tries again. That is how two processes claiming one item resolve to
//! exactly one success, with no coordinator.
//!
//! Views are the fold of the log, computed on read. At a few events a
//! second from five agents a log of thousands of events folds in
//! milliseconds; materialised views arrive with the daemon if they are
//! ever needed.

use std::fmt;

use crate::event::{ChainError, Event};
use crate::ids::Hash;
use crate::state::{FoldError, State};

/// Why the store could not do what was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// The events do not follow the stored head. Reload and retry.
    Conflict {
        /// The store's length when the append was refused.
        len: u64,
        head: Hash,
    },
    /// The stored log does not verify. Nothing is appended to a log in
    /// this condition.
    Corrupt(ChainError),
    /// The stored log verifies but refers to things it never created.
    Inconsistent(FoldError),
    /// The engine failed: I/O, locking, a schema this build cannot read.
    Backend(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict { len, .. } => {
                write!(f, "the log moved on (now {len} events); reload and retry")
            }
            Self::Corrupt(e) => write!(f, "the log does not verify: {e}"),
            Self::Inconsistent(e) => write!(f, "the log is inconsistent: {e}"),
            Self::Backend(e) => write!(f, "store: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<ChainError> for StoreError {
    fn from(e: ChainError) -> Self {
        Self::Corrupt(e)
    }
}

impl From<FoldError> for StoreError {
    fn from(e: FoldError) -> Self {
        Self::Inconsistent(e)
    }
}

/// The port. One implementation per engine; the core knows none of them.
pub trait Store {
    /// Append `events` atomically. They must be contiguous, start at
    /// `len + 1`, and the first must link to the stored head; otherwise
    /// [`StoreError::Conflict`]. An empty append is a no-op.
    fn append(&mut self, events: &[Event]) -> Result<(), StoreError>;

    /// Every event with `seq >= from`, in order.
    fn read_from(&self, from: u64) -> Result<Vec<Event>, StoreError>;

    /// The stored length and head hash, `(0, Hash::ZERO)` when empty.
    fn head(&self) -> Result<(u64, Hash), StoreError>;

    /// The whole log.
    fn read_all(&self) -> Result<Vec<Event>, StoreError> {
        self.read_from(1)
    }

    /// Verify the whole log and fold it. The state the next command is
    /// applied to.
    fn load(&self) -> Result<State, StoreError> {
        let log = self.read_all()?;
        crate::event::verify(&log)?;
        Ok(State::fold(&log)?)
    }
}

/// Check that `events` could follow `(len, head)`: the pure part of an
/// append's precondition, for adapters to call inside their transaction.
pub fn check_follows(len: u64, head: Hash, events: &[Event]) -> Result<(), StoreError> {
    let Some(first) = events.first() else {
        return Ok(());
    };
    if first.seq != len + 1 || first.prev != head {
        return Err(StoreError::Conflict { len, head });
    }
    let mut prev = head;
    for (i, e) in events.iter().enumerate() {
        if e.seq != len + 1 + i as u64 || e.prev != prev {
            return Err(StoreError::Conflict { len, head });
        }
        prev = e.hash;
    }
    Ok(())
}
