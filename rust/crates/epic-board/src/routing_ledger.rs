//! Java `NetRoutingLedger` (#933, upstream 339e8bb50 — "Speed up
//! single-thread routing; PCBench fully routed 51.9% to 73.8%",
//! survivor 3 of 3; port map `logs/readiness-2026-10-01/
//! 933-survivors-intake.md`): the board-owned INCREMENTAL cache of
//! incomplete-connection counts. Java replaced the autorouter's
//! fresh `DesignRulesChecker` scan per count read (a full board walk
//! per pass × N reader sites) with a ledger that is built once,
//! updated by notes at the two item-storage chokepoints, and
//! recounted per DIRTY NET only.
//!
//! Ownership mirrors Java exactly: the core is a private
//! [`crate::board::Board`] field (Java `BasicBoard.routingLedger`,
//! lazy field), the note hooks fire inside
//! [`crate::board::Board::insert_item`] /
//! [`crate::board::Board::remove_item`] (Java hooks in
//! `BoardItemRepository` at `67984f0ad→6bf5f7153e43`), and the
//! recount driver lives in `epic_drc::routing_ledger` because a
//! recount needs `(manager, board)` — the core here is pure data by
//! construction.
//!
//! DIVERGENCES (documented, mirrored on purpose):
//! - **Undo bypasses the hooks on BOTH sides** — Java's undo restores
//!   the item list without re-running the repository insert/remove,
//!   so its ledger goes stale until the next build; the port's
//!   [`crate::undo_facade`] writes the arena maps directly for the
//!   same effect. Stale-on-undo is Java parity, not a bug to fix.
//! - **`set_item_nets` is a Rust-only hook** (no Java counterpart —
//!   Java's `changeNet` predates the ledger and its callers run
//!   pre-route, when the ledger is unbuilt): the port logs the
//!   Removed(old)/Inserted(new) pair honestly; while unbuilt both
//!   notes are no-ops, so the hook is observably inert on every
//!   current caller.
//! - **The restore seam is `reset_transient_after_restore`** (Java
//!   invalidates from `deleteAllTracksAndVias`, which has no port
//!   face; a whole-board restore is the same cache-drop semantics).
//!
//! EQUIVALENCE ARGUMENT (why the incremental lists can never disagree
//! with a fresh full walk): (1) every stage of the per-net counter
//! (`epic_drc::incompletes::net_incompletes_row`) derives its groups
//! and members from `BTreeSet`s, so its count is insensitive to the
//! input list's order; (2) even for an order-sensitive consumer, ids
//! allocate monotonically and are never reused, so a note-appended
//! list stays id-ascending — exactly the order of the
//! `iter_ascending()` full walk.

use std::collections::BTreeSet;

use crate::id::ItemId;
use crate::items::BoardItemType;

/// The Java `NetRoutingLedger` state. Pure data; the writers are the
/// note hooks below (call sites in `board.rs`) and the recount driver
/// in `epic_drc::routing_ledger` — the fields are `pub` for that
/// driver alone, every other reader goes through the accessors.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoutingLedgerCore {
    /// Java `built` — `false` until the first read and again after
    /// [`RoutingLedgerCore::invalidate`]. While `false` the note
    /// hooks are no-ops: a later build does a full walk that
    /// supersedes anything they would have recorded (Java's
    /// `if (!built) return;`).
    pub built: bool,
    /// Java `itemsByNet` — per-net on-board connectable ids. Java
    /// sizes the array `maxNet + 1` and indexes it BY net number
    /// (slot 0 wasted); the port drops the sentinel and indexes
    /// `net − 1`, the same convention as
    /// `epic_drc::incompletes::raw_net_item_lists`.
    pub items_by_net: Vec<Vec<ItemId>>,
    /// Java `incompleteByNet` — the per-net cached incomplete counts
    /// (stale for every net in [`Self::dirty_nets`]).
    pub incomplete_by_net: Vec<i32>,
    /// Java `incompleteTotal` — Σ `incompleteByNet`.
    pub incomplete_total: i32,
    /// Java `maximumConnections` — Σ per-net `max(0, endpoints − 1)`
    /// with endpoints = pins + conduction areas.
    pub maximum_connections: i32,
    /// Java `dirtyNets` (`TreeSet<Integer>`) — nets whose
    /// `incompleteByNet` entry is stale; ascending by construction,
    /// which the driver's recount relies on.
    pub dirty_nets: BTreeSet<i32>,
    /// Deterministic perf witness (Java has none): full builds run
    /// and per-net recounts run — a test asserts the incremental path
    /// is taken without timing anything.
    pub builds: u32,
    /// See [`Self::builds`].
    pub net_recounts: u32,
}

impl RoutingLedgerCore {
    /// Java `noteInserted` — the insert hook: no-op until built,
    /// Connectable kinds only, then the item joins each of its nets'
    /// lists and every net is marked dirty (the recount itself is
    /// deferred to the driver — it needs the search-tree manager).
    pub fn note_inserted(&mut self, kind: BoardItemType, id: ItemId, nets: &[i32]) {
        if !self.built || !is_ledger_kind(kind) {
            return;
        }
        for &net in nets {
            if net_in_bounds(net, self.items_by_net.len()) {
                self.items_by_net[(net - 1) as usize].push(id);
            }
        }
        self.mark_nets(nets);
    }

    /// Java `noteRemoved` — the remove hook: same gates, then the
    /// item leaves each of its nets' lists.
    pub fn note_removed(&mut self, kind: BoardItemType, id: ItemId, nets: &[i32]) {
        if !self.built || !is_ledger_kind(kind) {
            return;
        }
        for &net in nets {
            if net_in_bounds(net, self.items_by_net.len()) {
                self.items_by_net[(net - 1) as usize].retain(|&other| other != id);
            }
        }
        self.mark_nets(nets);
    }

    /// Java `invalidate()` — drops the whole cache. Called from
    /// [`crate::board::Board::reset_transient_after_restore`] (the
    /// snapshot-restore seam); Java's own caller
    /// `deleteAllTracksAndVias` has no port face.
    pub fn invalidate(&mut self) {
        *self = Self::default();
    }

    /// Java `isBuilt` — whether a build has populated the lists.
    #[must_use]
    pub fn is_built(&self) -> bool {
        self.built
    }

    /// Java `incompleteCount`'s post-flush read face.
    #[must_use]
    pub fn incomplete_total(&self) -> i32 {
        self.incomplete_total
    }

    /// Java `maximumConnections`'s read face — note the Java quirk
    /// this mirrors: that getter does NOT flush, so the value can be
    /// stale until some count read flushes (pinned downstream).
    #[must_use]
    pub fn maximum_connections(&self) -> i32 {
        self.maximum_connections
    }

    /// Java `incompleteNetNumbers`'s post-flush read face: the nets
    /// (1-based, ascending) with a strictly positive cached count.
    #[must_use]
    pub fn net_numbers_with_incompletes(&self) -> BTreeSet<i32> {
        self.incomplete_by_net
            .iter()
            .enumerate()
            .filter_map(|(index, &count)| (count > 0).then_some(index as i32 + 1))
            .collect()
    }

    fn mark_nets(&mut self, nets: &[i32]) {
        for &net in nets {
            if net_in_bounds(net, self.items_by_net.len()) {
                self.dirty_nets.insert(net);
            }
        }
    }
}

/// The Java `item instanceof Connectable` gate, as the port's one
/// kind filter — the identical set the full walk
/// (`raw_net_item_lists`) filters on, so ledger lists and a fresh
/// walk can never disagree on MEMBERSHIP (the equivalence argument's
/// other half is order, in the module docs).
fn is_ledger_kind(kind: BoardItemType) -> bool {
    matches!(
        kind,
        BoardItemType::Trace
            | BoardItemType::Pin
            | BoardItemType::Via
            | BoardItemType::ConductionArea
    )
}

/// Java `countNet`'s bounds guard (`netNumber >= 1 && netNumber <
/// itemsByNet.length` — the SILENT skip of out-of-range nets,
/// mirrored and documented; `raw_net_item_lists` skips `net <= 0`
/// with the same silent face).
fn net_in_bounds(net: i32, len: usize) -> bool {
    net >= 1 && (net as usize) <= len
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin() -> BoardItemType {
        BoardItemType::Pin
    }

    fn outline() -> BoardItemType {
        BoardItemType::BoardOutline
    }

    /// A built core over two nets, as the driver would leave it.
    fn built_core() -> RoutingLedgerCore {
        let mut core = RoutingLedgerCore {
            built: true,
            items_by_net: vec![vec![ItemId::new(1), ItemId::new(2)], vec![ItemId::new(3)]],
            incomplete_by_net: vec![1, 2],
            incomplete_total: 3,
            maximum_connections: 3,
            ..RoutingLedgerCore::default()
        };
        core.dirty_nets.clear();
        core
    }

    /// The `!built` gate: notes on an unbuilt core are no-ops (a
    /// later build supersedes them — Java's early return).
    #[test]
    fn notes_are_noops_until_built() {
        let mut core = RoutingLedgerCore::default();
        core.note_inserted(pin(), ItemId::new(7), &[1]);
        core.note_removed(pin(), ItemId::new(7), &[1]);
        assert!(core.items_by_net.is_empty());
        assert!(core.dirty_nets.is_empty());
        assert!(!core.is_built());
    }

    /// The Connectable gate: a non-connectable kind (outline) logs
    /// nothing even on a built core.
    #[test]
    fn non_connectable_kinds_are_skipped() {
        let mut core = built_core();
        let before = core.clone();
        core.note_inserted(outline(), ItemId::new(9), &[1]);
        core.note_removed(outline(), ItemId::new(1), &[1]);
        assert_eq!(core.items_by_net, before.items_by_net);
        assert!(core.dirty_nets.is_empty(), "no net marked dirty");
    }

    /// Insert/remove mutate exactly the item's nets' lists and mark
    /// exactly those nets dirty (multi-net items fan out).
    #[test]
    fn notes_touch_their_own_nets_only() {
        let mut core = built_core();
        let shared = ItemId::new(20);
        core.note_inserted(pin(), shared, &[1, 2]);
        assert_eq!(
            core.items_by_net[0],
            vec![ItemId::new(1), ItemId::new(2), shared]
        );
        assert_eq!(core.items_by_net[1], vec![ItemId::new(3), shared]);
        assert_eq!(
            core.dirty_nets.iter().copied().collect::<Vec<_>>(),
            vec![1, 2]
        );
        core.note_removed(pin(), shared, &[1, 2]);
        assert_eq!(core.items_by_net[0], vec![ItemId::new(1), ItemId::new(2)]);
        assert_eq!(core.items_by_net[1], vec![ItemId::new(3)]);
    }

    /// The Java `countNet` bounds guard: net 0 and net > max are
    /// silently skipped on both hooks (mirroring Java's guard, which
    /// returns without touching anything).
    #[test]
    fn out_of_range_nets_silently_skipped() {
        let mut core = built_core();
        let before = core.items_by_net.clone();
        core.note_inserted(pin(), ItemId::new(30), &[0, 3]);
        assert_eq!(core.items_by_net, before, "no list gained the item");
        assert!(core.dirty_nets.is_empty(), "neither net marked");
    }

    /// `invalidate` drops everything, including the built flag.
    #[test]
    fn invalidate_resets_to_default() {
        let mut core = built_core();
        core.invalidate();
        assert_eq!(core, RoutingLedgerCore::default());
    }

    /// The read faces: total, max, and the >0 net set.
    #[test]
    fn read_faces() {
        let core = built_core();
        assert_eq!(core.incomplete_total(), 3);
        assert_eq!(core.maximum_connections(), 3);
        assert_eq!(core.net_numbers_with_incompletes(), BTreeSet::from([1, 2]));
        let mut zeroed = built_core();
        zeroed.incomplete_by_net = vec![0, 2];
        assert_eq!(zeroed.net_numbers_with_incompletes(), BTreeSet::from([2]));
    }
}
