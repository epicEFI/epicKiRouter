//! Java `datastructures/TimeLimit.java` — the shared shove deadline.
//!
//! Java constructs the object once per top-level shove call and hands
//! the SAME instance down the whole recursion; `limitExceeded()`
//! compares `now - timeStamp` against the limit with a STRICTLY
//! GREATER test, so a call landing exactly on the limit is not yet
//! expired. The stamp never resets: the object is a deadline, not a
//! stopwatch. Java call sites pass `null` where no limit applies
//! (every `insert` path — "the item database is already changed", so
//! a deadline would abort mid-mutation); the port models that as
//! `Option<&TimeLimit>`.

use std::time::Instant;

/// Java `TimeLimit` (`TimeLimit.java:20-46`).
#[derive(Debug, Clone)]
pub struct TimeLimit {
    /// Java `timeLimit` — the budget in milliseconds.
    limit_millis: i64,
    /// Java `timeStamp` — fixed at construction.
    time_stamp: Instant,
}

impl TimeLimit {
    /// Java ctor `TimeLimit(int p_milliSeconds)` — the stamp is NOW.
    #[must_use]
    pub fn new(milli_seconds: i64) -> Self {
        Self {
            limit_millis: milli_seconds,
            time_stamp: Instant::now(),
        }
    }

    /// Java `limitExceeded()` — `now - timeStamp > timeLimit`
    /// (strictly greater; exactly-on-limit is not an expiry).
    #[must_use]
    pub fn limit_exceeded(&self) -> bool {
        let elapsed = self.time_stamp.elapsed().as_millis() as i64;
        elapsed > self.limit_millis
    }

    /// Java `timeLimit` — the construction value (read-only face for
    /// the T12 `RouteBudget::Wall` diagnostics).
    #[must_use]
    pub fn limit_millis(&self) -> i64 {
        self.limit_millis
    }
}
