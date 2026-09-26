//! The per-thread scratch-buffer pool behind the M5 slice-B
//! allocation elimination: the complete-shape walk's room buffers,
//! the DFS node stacks and the with-clearance sift's sort keys are
//! REUSED across queries instead of re-grown per query/per obstacle
//! (the T1 profile's #1 allocation family — `complete_shape.rs:810`,
//! 39.2% of bm06's allocations; `min_area_tree` 1.6%).
//!
//! Pattern (panic-free by construction): each pool is a
//! `Cell<Option<T>>` thread-local; the consumer TAKES the slot for
//! the call's duration and PUTS the buffer back after. No borrow is
//! held across calls, so a hypothetical nested consumer cannot panic
//! on a double borrow — it merely allocates a fresh buffer whose
//! capacity is dropped at the outer put. Buffers never carry state
//! between uses: every consumer `clear()`s each buffer before filling
//! it, and the seven compares + the in-file reuse pin witness that
//! the reuse is invisible (a stale-buffer mutant is byte-positive).
//!
//! Why thread-local rather than per-tree state: the walk APIs take
//! `&SearchTree` (immutable) and the buffers must outlive one query,
//! not the tree. Why per-THREAD rather than a
//! global pool: buffer contents never influence results, and
//! per-thread pools stay correct under the M5-T7 deterministic
//! parallelism (each thread reuses only its own buffers — no
//! contention, no cross-thread state).
//!
//! Capacity retention is the whole point: after warmup a
//! `complete_shape` query allocates ONE `Vec` (the returned room
//! list) instead of the O(obstacles × rooms) per-obstacle /
//! per-restrain growth chains the T1 profile ranked.

use std::cell::Cell;
use std::thread::LocalKey;

use crate::complete_shape::IncompleteRoom;
use crate::shape_tree::NodeIdx;

/// The with-clearance sift's sort keys — `(clearance, entryId,
/// candidateIndex)` rows ([`crate::SearchTree::clearance_test`]).
pub(crate) type SortKeys = Vec<(i32, i32, usize)>;

/// The complete-shape walk's room double buffer (Java's
/// `result` / `newResult` pair, swapped per processed obstacle).
#[derive(Default)]
pub(crate) struct RoomBuffers {
    /// The rooms surviving so far (Java `result`).
    pub current: Vec<IncompleteRoom>,
    /// The rooms of the obstacle being processed (Java `newResult`).
    pub next: Vec<IncompleteRoom>,
}

thread_local! {
    static ROOM_BUFFERS: Cell<Option<RoomBuffers>> = const { Cell::new(None) };
    static NODE_STACKS: Cell<Option<Vec<NodeIdx>>> = const { Cell::new(None) };
    static CLEARANCE_SORT_KEYS: Cell<Option<SortKeys>> = const { Cell::new(None) };
}

/// Runs `f` with a pooled buffer: take, run, put back (capacity
/// retained). `unwrap_or_default` allocates the first buffer per
/// thread.
fn with_pooled<T: Default, R>(
    slot: &'static LocalKey<Cell<Option<T>>>,
    f: impl FnOnce(&mut T) -> R,
) -> R {
    let mut pooled = slot.with(Cell::take).unwrap_or_default();
    let result = f(&mut pooled);
    slot.with(|cell| cell.set(Some(pooled)));
    result
}

/// The complete-shape room double buffer.
pub(crate) fn with_room_buffers<R>(f: impl FnOnce(&mut RoomBuffers) -> R) -> R {
    with_pooled(&ROOM_BUFFERS, f)
}

/// A DFS node stack (the Java `ArrayStack` of the 45/90-degree
/// complete-shape walks and the [`crate::MinAreaTree::overlaps`]
/// query).
pub(crate) fn with_node_stack<R>(f: impl FnOnce(&mut Vec<NodeIdx>) -> R) -> R {
    with_pooled(&NODE_STACKS, f)
}

/// The with-clearance sift's sort keys
/// ([`crate::SearchTree::clearance_test`]).
pub(crate) fn with_clearance_sort_keys<R>(f: impl FnOnce(&mut SortKeys) -> R) -> R {
    with_pooled(&CLEARANCE_SORT_KEYS, f)
}
