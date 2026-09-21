//! ONE home for the process-global panic-hook swap (quality-review
//! T17b M-Q2). The hook is PROCESS-GLOBAL state and the epic-harness
//! test binary runs hook-swapping participants on cargo's
//! multi-threaded runner: [`run_detail_pass`] (router_compare.rs,
//! three watchdog pins), and `corpus::compare` (the in-suite golden
//! test). Concurrent take/silence/restore sequences race — one
//! participant restores ANOTHER's silence hook and the victim's
//! assert panics into the no-op (observed live: the sentinel FAILED
//! with its assertion message swallowed; the reviewer's Q5 mutant
//! reproduced it 3/3 with two guards removed).
//!
//! The split:
//! * [`lock`] — the caller-held serialization. Every participant in
//!   the TEST binary holds it for its whole observing window. In the
//!   BIN process the lock is uncontended (a single participant), so
//!   production call sites may take it or not without harm.
//! * [`Silenced`] — the RAII take/silence/restore swap. It does NOT
//!   lock: the sentinel pin holds [`lock`] and then calls
//!   `run_detail_pass` (which must silence), and std's `Mutex` is not
//!   reentrant — a self-locking guard would deadlock there. This is
//!   the caller contract: silence only under [`lock`] (or as the bin
//!   process's sole participant).

use std::sync::Mutex;
use std::sync::MutexGuard;

/// Serializes every panic-hook participant in this process. Held for
/// the WHOLE observing window of each participant (test body or
/// `compare` call).
static HOOK_LOCK: Mutex<()> = Mutex::new(());

/// Acquire [`HOOK_LOCK`] (poison-tolerant: a past panic in a
/// participant must not wedge the serialization for the rest of the
/// suite).
pub(crate) fn lock() -> MutexGuard<'static, ()> {
    HOOK_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

type PanicHook = Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>;

/// RAII panic silencer: takes the current hook, installs a no-op,
/// reinstalls the previous hook on drop. An early `?` or panic in the
/// guarded scope must not leave the silenced hook installed for the
/// rest of the process — including unrelated panics under cargo's
/// multithreaded test runner.
///
/// Takes NO lock (see the module docs for the re-entrancy contract).
pub(crate) struct Silenced {
    previous: Option<PanicHook>,
}

impl Silenced {
    /// Swap the process hook for a no-op; restore happens on drop.
    pub(crate) fn new() -> Self {
        let previous = Some(std::panic::take_hook());
        std::panic::set_hook(Box::new(|_| {}));
        Silenced { previous }
    }
}

impl Drop for Silenced {
    fn drop(&mut self) {
        if let Some(hook) = self.previous.take() {
            std::panic::set_hook(hook);
        }
    }
}
