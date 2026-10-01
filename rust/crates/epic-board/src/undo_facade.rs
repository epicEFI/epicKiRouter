//! The board-level undo/redo/snapshot FACADE — Java
//! `BasicBoard.undo(Set<Integer>)` (`BasicBoard.java:1234-1241`),
//! `redo` (`:1247-1254`), `applyUndoRedoSideEffects`
//! (`:1256-1288`), and the snapshot trio
//! `generateSnapshot`/`popSnapshot` via `BoardSnapshotManager`
//! (`BoardSnapshotManager.java:75-83`). This module is the Task 14
//! doc-of-record for the facade port; the corpus driver is
//! `rust/harness/src/undo_corpus.rs` and the Java capture oracle is
//! `rust/harness/oracle/UndoOracle.java`.
//!
//! ## The facade body (Java `:1234-1241`, verbatim)
//!
//! ```java
//! public boolean undo(Set<Integer> changedNets) {
//!     this.components.undo(this.communication.observers);
//!     Collection<UndoableObjects.Storable> cancelledObjects = new LinkedList<>();
//!     Collection<UndoableObjects.Storable> restoredObjects = new LinkedList<>();
//!     boolean result = itemList.undo(cancelledObjects, restoredObjects);
//!     applyUndoRedoSideEffects(cancelledObjects, restoredObjects, changedNets);
//!     return result;
//! }
//! ```
//!
//! The port early-returns when the item undo fails; Java instead runs
//! `applyUndoRedoSideEffects` over two EMPTY lists there — two
//! zero-iteration walks, a no-op — so the shapes are equivalent.
//!
//! The order QUIRK is load-bearing and pinned: `components.undo` runs
//! FIRST and its return value is ignored — on an empty ITEM stack the
//! components level STILL MOVES while the board reports `false` (the
//! probe capture, `und-t01` steps n=12/n=13: `ret=false` with
//! `comp_level` 1→0). The port therefore calls
//! [`Components::undo`] before the item-list match and discards its
//! `bool`.
//!
//! ## `applyUndoRedoSideEffects` (`:1256-1288`, shared by undo AND redo)
//!
//! - **Cancelled, IN LIST ORDER**: `searchTreeManager.remove(item)`
//!   (the `isOnTheBoard()` guard lives INSIDE the manager, ported at
//!   [`SearchTreeManager::remove`]), the observer notify (no observers
//!   in the ported headless surface), and EVERY cancelled object's
//!   nets go into `changedNets` — unconditionally, on-board or not.
//! - **Restored, IN LIST ORDER**: `currentItem.board = this` (no port
//!   counterpart — the arena has no board back-pointer),
//!   `searchTreeManager.insert(item)` (which flips `onTheBoard` back
//!   — [`SearchTreeManager::insert`]), `clearAutorouteInfo()`
//!   (`Item.java:1096-1098` = `autorouteInfo = null` ONLY; the port's
//!   `ItemEntry` carries no autoroute info, so this is a no-port
//!   anchor), the observer notify, and the nets into `changedNets`.
//! - The ARENA is written BETWEEN the two walks' data source and the
//!   tree work: Java's `itemList.undo` has already mutated the map
//!   before the side-effect walk runs, so the port performs the arena
//!   swaps (cancelled entries OUT, restored entries IN — direct map
//!   writes, NOT [`Board::insert_item`]: Java does not re-run the
//!   repository, does not re-assert, and does NOT bump the revision;
//!   `BasicBoard.undo` never touches `revision`) before the Phase-R
//!   tree inserts. The restored `ItemEntry` VALUES come from the undo
//!   nodes, whose mirror discipline ([`Board::mirror_node`]) keeps
//!   them equal to the arena entries they replaced.
//! - The DRILL-SPAN memo ([`Board::drill_precalc`]) is dropped for
//!   every restored id — behavior-neutral, because the memos are
//!   pure functions of persistent state: Java's SWAP restores hand
//!   back fresh clones (`-1` sentinels recompute), but a delete-list
//!   restore returns the SAME instance with its memo intact — and
//!   recompute equals keep either way (padstack-derived spans,
//!   `DrillItem.java:163-184`).
//! - `changedNets` is a `HashSet<Integer>` in Java; the facade
//!   returns it SORTED + deduplicated ([`UndoOutcome::changed_nets`]),
//!   which is exactly the digest's normalization
//!   (`UndoOracle.java` sorts before emitting).
//!
//! ## The cancelled/restored collections
//!
//! The Java facade hides the lists; the T2 container
//! ([`UndoableObjects::undo`]/[`redo`]) returns them in EXACT Java
//! append order (map order for the swap phase — descending id — then
//! delete-list order), and an item may appear in BOTH lists (the
//! modified-this-level case: cancelled carries the current value,
//! restored the pre-change clone — the `same-node-in-both-lists`
//! pin). The sequential Phase-C-then-Phase-R processing makes the
//! both-lists case land on the restored value, exactly like Java.
//!
//! ## Snapshots
//!
//! `BoardSnapshotManager.generateSnapshot` pushes BOTH stacks
//! (`board.itemList.generateSnapshot(); board.components.generateSnapshot();`),
//! while `popSnapshot` pops the ITEM list ONLY
//! (`return board.itemList.popSnapshot();`) — the components keep
//! their level (the asymmetry the corpus pins: after pop,
//! `item_level` drops while `comp_level` stays).
use crate::board::{Board, ItemEntry};
use crate::id::ItemId;
use crate::tree_manager::SearchTreeManager;
use std::cmp::Reverse;
use std::collections::BTreeSet;

/// The outcome of one facade undo/redo — Java's `boolean` return plus
/// the collections the facade does not expose (the oracle exposes them
/// via its second board; the port returns them directly).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoOutcome {
    /// Java's return value: false only when the ITEM stack refused
    /// (empty stack at level 0 for undo, `stackLevel >=
    /// deletedObjectsStack.size()` for redo). The components stack may
    /// still have moved (the order quirk — module docs).
    pub changed: bool,
    /// The cancelled ids IN JAVA APPEND ORDER (descending-id map order
    /// for the swap phase, then delete-list order). Ordered — never
    /// sort this.
    pub cancelled: Vec<ItemId>,
    /// The restored ids IN JAVA APPEND ORDER (undo-object-first swap
    /// values, then delete-list restores). Ordered — never sort this.
    pub restored: Vec<ItemId>,
    /// Every cancelled/restored object's nets, SORTED and deduplicated
    /// (Java collects into a `HashSet<Integer>`; the digest's
    /// normalization is the sorted set).
    pub changed_nets: Vec<i32>,
}

impl Board {
    /// Java `BasicBoard.generateSnapshot()` — pushes BOTH stacks
    /// (`BoardSnapshotManager.java:75-78`).
    pub fn generate_snapshot(&mut self) {
        self.item_undo.generate_snapshot();
        self.components.generate_snapshot();
    }

    /// Java `BasicBoard.popSnapshot()` (`BoardSnapshotManager.java:81-83`)
    /// — the ITEM list only; the components keep their level (the
    /// asymmetry pin).
    pub fn pop_snapshot(&mut self) -> bool {
        self.item_undo.pop_snapshot()
    }

    /// The item-list `stackLevel` (Java reads the private field
    /// reflectively in the oracle; the port exposes it).
    #[must_use]
    pub fn item_stack_level(&self) -> usize {
        self.item_undo.stack_level()
    }

    /// The components-list `stackLevel` (Java
    /// `Components.undoList.stackLevel`, read reflectively there).
    #[must_use]
    pub fn components_stack_level(&self) -> usize {
        self.components.stack_level()
    }

    /// Java `BasicBoard.undo(Set<Integer> changedNets)`
    /// (`:1234-1241`) — see the module docs for the verbatim body, the
    /// early-return equivalence, and the order quirk.
    pub fn undo(&mut self, manager: &mut SearchTreeManager) -> UndoOutcome {
        // Java: `this.components.undo(...)` FIRST, return ignored —
        // it runs even when the item undo then fails.
        self.components.undo();
        let Some((cancelled, restored)) = self.item_undo.undo() else {
            return UndoOutcome {
                changed: false,
                cancelled: Vec::new(),
                restored: Vec::new(),
                changed_nets: Vec::new(),
            };
        };
        self.apply_undo_redo_side_effects(manager, cancelled, restored)
    }

    /// Java `BasicBoard.redo(Set<Integer> changedNets)`
    /// (`:1247-1254`) — the mirror body with `components.redo` /
    /// `itemList.redo` and the SAME side-effect function.
    pub fn redo(&mut self, manager: &mut SearchTreeManager) -> UndoOutcome {
        self.components.redo();
        let Some((cancelled, restored)) = self.item_undo.redo() else {
            return UndoOutcome {
                changed: false,
                cancelled: Vec::new(),
                restored: Vec::new(),
                changed_nets: Vec::new(),
            };
        };
        self.apply_undo_redo_side_effects(manager, cancelled, restored)
    }

    /// Java `applyUndoRedoSideEffects` (`:1256-1288`) + the arena swap
    /// `itemList.undo/redo` already performed on the Java side. See
    /// the module docs for the phase walk.
    fn apply_undo_redo_side_effects(
        &mut self,
        manager: &mut SearchTreeManager,
        cancelled: Vec<ItemEntry>,
        restored: Vec<ItemEntry>,
    ) -> UndoOutcome {
        let mut changed_nets: BTreeSet<i32> = BTreeSet::new();
        let mut cancelled_ids = Vec::with_capacity(cancelled.len());
        let mut restored_ids = Vec::with_capacity(restored.len());

        // Phase C — cancelled, IN ORDER: nets (unconditional), tree
        // remove (the on-board guard lives in the manager), arena
        // slot out.
        for entry in &cancelled {
            cancelled_ids.push(entry.id);
            changed_nets.extend(entry.nets.iter().copied());
            manager.remove(self, entry.id);
            self.items.remove(&Reverse(entry.id));
        }

        // The arena swap Java's itemList.undo/redo already completed:
        // restored entries IN (direct map writes — no insert_item, no
        // revision bump), drill memo dropped (fresh-clone policy).
        // The undo NODE at each restored key already holds exactly
        // this value (the container put it there), so the node mirror
        // invariant survives without extra writes.
        for entry in &restored {
            self.drill_precalc.remove(&entry.id);
            self.items.insert(Reverse(entry.id), entry.clone());
        }

        // Phase R — restored, IN ORDER: nets, tree insert (flips the
        // on-board flag), after the arena is final.
        for entry in &restored {
            restored_ids.push(entry.id);
            changed_nets.extend(entry.nets.iter().copied());
            manager.insert(self, entry.id);
        }

        UndoOutcome {
            changed: true,
            cancelled: cancelled_ids,
            restored: restored_ids,
            changed_nets: changed_nets.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::{FixedState, ItemData};
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::point::Point;
    use epic_geometry::polyline::Polyline;

    /// One unfixed single-layer trace with the given nets (three
    /// corners so a geometry change is a real change).
    fn trace_entry(id: ItemId, nets: Vec<i32>, fixed: FixedState) -> ItemEntry {
        let x = i32::try_from(id.get()).expect("small ids") * 100;
        ItemEntry {
            id,
            data: ItemData::Trace {
                layer: 0,
                half_width: 100,
                lines: Polyline::from_points(&[
                    Point::Int(IntPoint::new(x, 0)),
                    Point::Int(IntPoint::new(x, 500)),
                    Point::Int(IntPoint::new(x, 1000)),
                ]),
            },
            nets,
            clearance_class: 1,
            component_id: 0,
            fixed,
            on_the_board: false,
        }
    }

    /// A board of three traces (descending ids `ids[2] > ids[1] >
    /// ids[0]`) with its trees filled — the smallest setup where the
    /// facade's tree side effects are observable.
    fn board_with_traces(nets: [[i32; 2]; 3]) -> (Board, Vec<ItemId>) {
        let mut board = Board::new();
        let ids: Vec<ItemId> = (0..3).map(|_| board.alloc_id()).collect();
        for (index, id) in ids.iter().enumerate() {
            let trimmed: Vec<i32> = nets[index].iter().copied().filter(|net| *net > 0).collect();
            board.insert_item(trace_entry(*id, trimmed, FixedState::Unfixed));
        }
        let mut manager = SearchTreeManager::new();
        manager.insert_items_creation_order(&mut board);
        (board, ids)
    }

    /// The default-tree content set — the D27 witness (`id:idx`
    /// strings, sorted).
    fn tree_pairs(manager: &SearchTreeManager) -> BTreeSet<String> {
        manager
            .default_tree()
            .min_area_tree()
            .to_array()
            .into_iter()
            .map(|leaf| format!("{}:{}", leaf.object_key, leaf.shape_index_in_object))
            .collect()
    }

    fn ids_u32(ids: &[ItemId]) -> Vec<u32> {
        ids.iter().map(|id| id.get()).collect()
    }

    /// `BoardSnapshotManager` asymmetry: `generateSnapshot` pushes
    /// BOTH stacks but `popSnapshot` pops the ITEM list only
    /// (`BoardSnapshotManager.java:75-83`) — the components keep
    /// their level. A port that pops both fails the last assert.
    #[test]
    fn pop_snapshot_pops_items_but_not_components() {
        let (mut board, _) = board_with_traces([[1, 0], [2, 0], [1, 0]]);
        board.generate_snapshot();
        assert_eq!(board.item_stack_level(), 1);
        assert_eq!(board.components_stack_level(), 1);
        assert!(board.pop_snapshot());
        assert_eq!(board.item_stack_level(), 0, "the item list popped");
        assert_eq!(
            board.components_stack_level(),
            1,
            "the components list keeps its level (the pop asymmetry)"
        );
    }

    /// The order quirk (`BasicBoard.java:1234-1241`):
    /// `components.undo` runs FIRST and its return is ignored — on an
    /// empty ITEM stack the components level STILL MOVES while the
    /// board reports `false` (oracle capture `und-t01` steps n=12).
    /// A port that calls components only on success, or checks the
    /// item stack first, fails the final assert.
    #[test]
    fn components_undo_moves_when_the_item_undo_fails() {
        let (mut board, _) = board_with_traces([[1, 0], [2, 0], [1, 0]]);
        let mut manager = SearchTreeManager::new();
        manager.insert_items_creation_order(&mut board);
        board.generate_snapshot();
        board.pop_snapshot(); // items at 0, components at 1
        let outcome = board.undo(&mut manager);
        assert!(!outcome.changed, "the ITEM stack refused (level 0)");
        assert!(outcome.cancelled.is_empty() && outcome.restored.is_empty());
        assert_eq!(
            board.components_stack_level(),
            0,
            "components STILL moved — the call precedes the item undo"
        );
        assert_eq!(board.item_stack_level(), 0);
    }

    /// The swap case: an item modified at the current level sits in
    /// BOTH lists — `cancelled` carries the current value's id,
    /// `restored` the SAME id (the pre-change clone). The sequential
    /// Phase-C-then-Phase-R walk must land the arena on the restored
    /// value (the module-docs contract). The geometry save fires
    /// through [`Board::set_trace_polyline`] — Java's
    /// `PolylineTrace.replaceGeometry` site.
    ///
    /// The REDO half is the mirror_node witness: Java's current-level
    /// undo node shares the LIVE instance, so the geometry mutation is
    /// visible through it and redo re-lands the MUTATED polyline. A
    /// port whose node stayed stale (the [`Board::mirror_node`] write
    /// dropped from [`Board::set_trace_polyline`]) re-lands the
    /// pre-mutation clone here — the undo-only asserts above cannot
    /// see that mutant (the old clone IS the correct undo landing).
    #[test]
    fn modified_item_sits_in_both_lists_and_lands_on_the_restored_value() {
        let (mut board, ids) = board_with_traces([[1, 0], [2, 0], [3, 0]]);
        let mut manager = SearchTreeManager::new();
        manager.insert_items_creation_order(&mut board);
        board.generate_snapshot();
        let original = board
            .trace_polyline(ids[2])
            .cloned()
            .expect("trace present");
        let mutated = Polyline::from_points(&[
            Point::Int(IntPoint::new(9999, 0)),
            Point::Int(IntPoint::new(9999, 1000)),
        ]);
        board.set_trace_polyline(ids[2], mutated.clone());
        let outcome = board.undo(&mut manager);
        assert!(outcome.changed);
        assert_eq!(ids_u32(&outcome.cancelled), vec![ids[2].get()]);
        assert_eq!(
            ids_u32(&outcome.restored),
            vec![ids[2].get()],
            "the same id sits in BOTH lists"
        );
        assert_eq!(
            board.trace_polyline(ids[2]),
            Some(&original),
            "the arena landed on the restored (pre-change) value"
        );
        board.redo(&mut manager);
        assert_eq!(
            board.trace_polyline(ids[2]),
            Some(&mutated),
            "redo re-lands the MUTATED geometry (the mirror_node write is load-bearing)"
        );
    }

    /// THE ORDER PIN (multi-element — the corpus cannot produce this,
    /// see `undo_corpus.rs` "coverage boundary"): the delete list
    /// preserves REMOVAL order, and undo restores it in exactly that
    /// push order. Phase 1 removes ASCENDING (restore [a, b] — kills
    /// a descending-sort mutant), phase 2 removes DESCENDING (restore
    /// [b, a] — kills an ascending-sort mutant). A port that sorts
    /// either list fails one of the two phases.
    #[test]
    fn delete_list_restore_order_is_the_java_append_order() {
        let (mut board, ids) = board_with_traces([[1, 0], [2, 0], [3, 0]]);
        let mut manager = SearchTreeManager::new();
        manager.insert_items_creation_order(&mut board);

        // Phase 1: removal order ids[0] (low) then ids[1] (high).
        board.generate_snapshot();
        crate::trace_ops::remove_item_through_repository(&mut manager, &mut board, ids[0]);
        crate::trace_ops::remove_item_through_repository(&mut manager, &mut board, ids[1]);
        let outcome = board.undo(&mut manager);
        assert!(outcome.cancelled.is_empty(), "pure restores cancel nothing");
        assert_eq!(
            ids_u32(&outcome.restored),
            vec![ids[0].get(), ids[1].get()],
            "delete-list PUSH order, not id order (phase 1)"
        );
        assert!(board.is_on_the_board(ids[0]) && board.is_on_the_board(ids[1]));

        // Phase 2: removal order ids[1] (high) then ids[0] (low).
        board.generate_snapshot();
        crate::trace_ops::remove_item_through_repository(&mut manager, &mut board, ids[1]);
        crate::trace_ops::remove_item_through_repository(&mut manager, &mut board, ids[0]);
        let outcome = board.undo(&mut manager);
        assert_eq!(
            ids_u32(&outcome.restored),
            vec![ids[1].get(), ids[0].get()],
            "delete-list PUSH order, not id order (phase 2)"
        );
    }

    /// D27: the undo/redo side effects keep the default tree CONTENT
    /// consistent — the removed item's pairs leave the set and come
    /// back with the undo, leave again with the redo. A facade that
    /// skips the tree remove (Phase C) or the tree insert (Phase R)
    /// fails the corresponding assert; the revision stays put (Java's
    /// undo path never re-runs the repository).
    #[test]
    fn undo_redo_restore_the_default_tree_content_set() {
        let (mut board, ids) = board_with_traces([[1, 0], [2, 0], [3, 0]]);
        let mut manager = SearchTreeManager::new();
        manager.insert_items_creation_order(&mut board);
        board.generate_snapshot();
        let before = tree_pairs(&manager);

        crate::trace_ops::remove_item_through_repository(&mut manager, &mut board, ids[1]);
        // The REPOSITORY remove bumps the revision (T69); the undo
        // itself must not (Java's undo path never re-runs the
        // repository).
        let revision = board.revision();
        let after_remove = tree_pairs(&manager);
        assert_ne!(before, after_remove, "the removal reached the tree");
        assert!(
            before
                .iter()
                .any(|pair| pair.starts_with(&format!("{}:", ids[1].get()))),
            "the trace had tree pairs to begin with"
        );

        let outcome = board.undo(&mut manager);
        assert_eq!(
            tree_pairs(&manager),
            before,
            "undo re-inserts the removed item's pairs (Phase R)"
        );
        assert_eq!(board.revision(), revision, "undo never bumps the revision");

        board.redo(&mut manager);
        assert_eq!(
            tree_pairs(&manager),
            after_remove,
            "redo re-removes (Phase C)"
        );

        board.undo(&mut manager);
        assert_eq!(tree_pairs(&manager), before, "second undo re-inserts again");
        assert_eq!(
            ids_u32(&outcome.restored),
            vec![ids[1].get()],
            "the pure delete-list restore carries exactly the removed id"
        );
    }

    /// `changedNets` is a Java `HashSet<Integer>`; the facade returns
    /// it SORTED and DEDUPLICATED. Two restored traces share net 1
    /// (one of them carries it with net 2) — a port that preserves
    /// encounter order or appends duplicates fails the final assert.
    #[test]
    fn changed_nets_are_sorted_and_deduplicated() {
        let (mut board, ids) = board_with_traces([[1, 0], [2, 1], [1, 0]]);
        let mut manager = SearchTreeManager::new();
        manager.insert_items_creation_order(&mut board);
        board.generate_snapshot();
        crate::trace_ops::remove_item_through_repository(&mut manager, &mut board, ids[0]);
        crate::trace_ops::remove_item_through_repository(&mut manager, &mut board, ids[1]);
        let outcome = board.undo(&mut manager);
        assert_eq!(outcome.restored.len(), 2);
        assert_eq!(
            outcome.changed_nets,
            vec![1, 2],
            "net 1 came from BOTH restored traces (dedup) and sorts before 2"
        );
    }

    /// Undo of an EMPTY boundary: Java returns TRUE (the level moved)
    /// with empty delta lists — the capture's `und-t03` step n=8.
    /// A port that treats "nothing changed" as the false-return fails.
    #[test]
    fn undo_of_an_empty_boundary_returns_true_with_empty_deltas() {
        let (mut board, _) = board_with_traces([[1, 0], [2, 0], [1, 0]]);
        let mut manager = SearchTreeManager::new();
        manager.insert_items_creation_order(&mut board);
        board.generate_snapshot();
        let outcome = board.undo(&mut manager);
        assert!(outcome.changed, "the level moved — true, not false");
        assert!(outcome.cancelled.is_empty());
        assert!(outcome.restored.is_empty());
        assert!(outcome.changed_nets.is_empty());
    }
}
