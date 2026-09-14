//! The search-tree manager: which trees exist, and how items enter
//! and leave them (M2 Task 6).
//!
//! Java anchor: `board/searchtree/SearchTreeManager.java` in full —
//! the constructor `:30-36` (a SINGLE default tree: the base
//! `ShapeSearchTree` with `FortyfiveDegreeBoundingDirections`, class
//! 0), `insert` `:39-44`, `remove` `:47-62`,
//! `setClearanceCompensationUsed` `:89-108`,
//! `clearanceValueChanged` `:111-119`, `clearanceClassRemoved`
//! `:122-134`, `getAutorouteTree` `:140-172` (**T59**),
//! `resetCompensatedTrees` `:177-179`, `reinsertTreeItems`
//! `:186-200` (**T58** — the clear-between-remove-and-insert),
//! `removeAllBoardItems` `:202-215`, `insertAllBoardItems`
//! `:217-231`.
//!
//! ## Port shape
//!
//! Java's manager HOLDS the board (its trees read `board.rules` while
//! calculating shapes). Here the manager is standalone and every
//! mutating method takes `&mut Board` — the shape arithmetic already
//! lives caller-side in [`crate::tree_shapes`] (the D17 split), so
//! the trees need no board reference and the port avoids a
//! Board-owns-Manager-borrows-Board cycle.
//!
//! Per-item entries live in [`SearchTreeManager::entries`], keyed by
//! the tree's OBJECT ID (Java's `ItemSearchTreesInfo` keys by tree
//! reference equality — after `setClearanceCompensationUsed` replaces
//! the tree list, a class-keyed map would hand the NEW tree the OLD
//! tree's leaves; see [`epic_index::SearchTree::object_id`]).
//!
//! ## The slot-0 invariant
//!
//! `trees[0]` is ALWAYS the default tree: every mutation preserves it
//! — the rebuild replaces the list with a fresh single-entry list,
//! the retain-based drops keep slot 0 by construction (the default's
//! class equals itself; `clearance_class_removed` refuses the
//! default's class BEFORE dropping), and `getAutorouteTree` only
//! appends. Pinned in the tests.
//!
//! ## Not ported yet
//!
//! The trace-entry surgery members (`mergeEntriesInFront`,
//! `mergeEntriesAtEnd`, `changeEntries`, `reuseEntriesAfterCutout`,
//! `:237-284`) need the trace shape surface — Task 7.
//! `validateEntries` (`:69-78`) is a debug checker and lands with
//! the query surface (Task 8).

use std::collections::BTreeMap;

use epic_index::{NodeIdx, SearchTree, SearchTreeVariant};

use crate::board::Board;
use crate::id::ItemId;
use crate::rules_surf::AngleRestriction;

/// Manages the search trees used in the auto-router (Java
/// `SearchTreeManager`): creation, per-item insertion/removal across
/// ALL trees, and the rule-change rebuilds.
#[derive(Clone, Debug)]
pub struct SearchTreeManager {
    /// The live trees. Slot 0 is ALWAYS the default tree (module
    /// docs); Java's `compensatedSearchTrees` list with
    /// `defaultTree` first.
    trees: Vec<SearchTree>,
    /// Java `clearanceCompensationUsed` — the sticky FLAG driving
    /// tree rebuilding (the per-tree derived property is
    /// [`epic_index::SearchTree::is_clearance_compensation_used`]).
    clearance_compensation_used: bool,
    /// Per-item tree entries: item id -> tree object id -> the
    /// index-aligned leaves of that item in that tree (Java
    /// `Item.searchTreesInfo`, identity-keyed).
    entries: BTreeMap<ItemId, BTreeMap<u64, Vec<Option<NodeIdx>>>>,
}

impl SearchTreeManager {
    /// Java `SearchTreeManager(BasicBoard)` `:30-36` — one default
    /// tree: the GENERIC variant (the base `ShapeSearchTree`
    /// constructed with `FortyfiveDegreeBoundingDirections`), class
    /// 0. Creates NO entries: Java fills its trees item-by-item as
    /// the board is read — the port's counterpart of that read-time
    /// fill is [`SearchTreeManager::insert_all_board_items`], which
    /// the caller runs once after [`crate::board::Board::from_ses_board`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            trees: vec![SearchTree::new(SearchTreeVariant::Generic, 0)],
            clearance_compensation_used: false,
            entries: BTreeMap::new(),
        }
    }

    /// Java `getDefaultTree()` (`:65-67`).
    #[must_use]
    pub fn default_tree(&self) -> &SearchTree {
        &self.trees[0]
    }

    /// The live trees, slot 0 first (the module's slot-0 invariant).
    #[must_use]
    pub fn trees(&self) -> &[SearchTree] {
        &self.trees
    }

    /// Java `isClearanceCompensationUsed()` (`:84-86`).
    #[must_use]
    pub fn is_clearance_compensation_used(&self) -> bool {
        self.clearance_compensation_used
    }

    /// Java `Item.getSearchTreeEntries(tree)` — the stored entries of
    /// `id` in the tree with the given object id (`None` when the
    /// item was never inserted there or carries no shapes).
    #[must_use]
    pub fn tree_entries(&self, id: ItemId, tree_object_id: u64) -> Option<&[Option<NodeIdx>]> {
        self.entries
            .get(&id)
            .and_then(|by_tree| by_tree.get(&tree_object_id))
            .map(Vec::as_slice)
    }

    /// Java `insert(Item)` `:39-44`: the tree shapes of the item
    /// into EVERY live tree, then `setOnTheBoard(true)` — with no
    /// double-insert guard (Java's contract is the caller's; an item
    /// inserted twice gains duplicate leaves in Java too).
    pub fn insert(&mut self, board: &mut Board, id: ItemId) {
        for tree_index in 0..self.trees.len() {
            self.insert_into_tree(board, id, tree_index);
        }
        board.set_on_the_board(id, true);
    }

    /// Java `remove(Item)` `:47-62`. Guarded by `isOnTheBoard()`:
    /// removing an item that is not on the board is a no-op.
    /// Otherwise each live tree drops the item's entries (identity
    /// lookup), the item's WHOLE entry map goes (Java
    /// `clearSearchTreeEntries()`), and the flag clears.
    pub fn remove(&mut self, board: &mut Board, id: ItemId) {
        if !board.is_on_the_board(id) {
            return;
        }
        for tree in &mut self.trees {
            if let Some(entries) = self
                .entries
                .get(&id)
                .and_then(|by_tree| by_tree.get(&tree.object_id()))
            {
                tree.remove(entries);
            }
        }
        self.entries.remove(&id);
        board.set_on_the_board(id, false);
    }

    /// Java `setClearanceCompensationUsed(boolean)` `:89-108`. A
    /// same-value call is the early-out (`:90-92`); otherwise the
    /// flag flips, every item is removed, the tree list is REPLACED
    /// by a fresh default tree of class `1`/`0` (a NEW object — the
    /// old default's identity never comes back), and every item is
    /// re-inserted.
    pub fn set_clearance_compensation_used(&mut self, board: &mut Board, value: bool) {
        if self.clearance_compensation_used == value {
            return;
        }
        self.clearance_compensation_used = value;
        self.remove_all_board_items(board);
        self.trees.clear();
        let compensated_class = i32::from(value);
        self.trees.push(SearchTree::new(
            SearchTreeVariant::Generic,
            compensated_class,
        ));
        self.insert_all_board_items(board);
    }

    /// Java `clearanceValueChanged()` `:111-119` — a matrix cell was
    /// edited interactively. Drops every tree whose class differs
    /// from the DEFAULT's (a CLASS-NUMBER comparison, `:113-114` —
    /// not the identity comparison of `resetCompensatedTrees`), then
    /// rebuilds all items — but ONLY when compensation is on; with
    /// the flag off the surviving default tree keeps its stale
    /// shapes (Java's behavior, ported exactly).
    pub fn clearance_value_changed(&mut self, board: &mut Board) {
        let default_class = self.trees[0].compensated_clearance_class;
        self.trees
            .retain(|tree| tree.compensated_clearance_class == default_class);
        if self.clearance_compensation_used {
            self.remove_all_board_items(board);
            self.insert_all_board_items(board);
        }
    }

    /// Java `clearanceClassRemoved(int)` `:122-134`. The default
    /// tree's class is refused FIRST (Java warns through FRLogger —
    /// a log-only site, D12 — and returns); any other class's trees
    /// are dropped. Slot 0 can never be removed here.
    pub fn clearance_class_removed(&mut self, class_no: i32) {
        if class_no == self.trees[0].compensated_clearance_class {
            // Java: FRLogger.warn("...unable to remove default tree").
            return;
        }
        self.trees
            .retain(|tree| tree.compensated_clearance_class != class_no);
    }

    /// Java `getAutorouteTree(int)` `:140-172` — **T59**. Returns the
    /// INDEX (into [`SearchTreeManager::trees`]) of the tree
    /// compensated for `clearance_class_index`, creating it when
    /// absent:
    ///
    /// * the scan (`:141-145`) finds the DEFAULT tree for its own
    ///   class — class 0 with compensation off, class 1 on — so
    ///   those requests never allocate;
    /// * a new tree's variant follows the board's CURRENT angle
    ///   restriction (`:149-160`): `NINETY_DEGREE` -> the
    ///   90-degree subclass, `FORTYFIVE_DEGREE` -> the 45-degree
    ///   subclass, else the base tree with 45-degree directions —
    ///   the GENERIC variant here, whose drill dispatch reads the
    ///   restriction again at call time
    ///   ([`crate::tree_shapes::drill_tree_shapes`]);
    /// * the whole item list is bulk-inserted into the NEW TREE
    ///   ONLY (`:163-170`) — the other trees are untouched — and
    ///   WITHOUT the on-the-board flag flip of the broadcast insert
    ///   (Java calls the TREE's insert, not the manager's).
    pub fn get_autoroute_tree(&mut self, board: &mut Board, clearance_class_index: i32) -> usize {
        if let Some(index) = self
            .trees
            .iter()
            .position(|tree| tree.compensated_clearance_class == clearance_class_index)
        {
            return index;
        }
        let variant = match board.rules().trace_angle_restriction {
            AngleRestriction::NinetyDegree => SearchTreeVariant::NinetyDegree,
            AngleRestriction::FortyfiveDegree => SearchTreeVariant::FortyfiveDegree,
            AngleRestriction::None => SearchTreeVariant::Generic,
        };
        self.trees
            .push(SearchTree::new(variant, clearance_class_index));
        let index = self.trees.len() - 1;
        let ids: Vec<ItemId> = board.iter_descending().map(|entry| entry.id).collect();
        for id in ids {
            self.insert_into_tree(board, id, index);
        }
        index
    }

    /// Java `resetCompensatedTrees()` `:177-179` — every tree except
    /// the default (an IDENTITY comparison `t != defaultTree`,
    /// unlike the class comparison of `clearanceValueChanged`) is
    /// dropped. A later `getAutorouteTree` for the same class
    /// creates a NEW tree with a fresh identity.
    pub fn reset_compensated_trees(&mut self) {
        let default_object_id = self.trees[0].object_id();
        self.trees
            .retain(|tree| tree.object_id() == default_object_id);
    }

    /// Java `reinsertTreeItems()` `:186-200` — **T58**. Remove all,
    /// clear every item's derived data (the drill-span memo the tree
    /// shapes consumed — without the clear, re-insertion reuses the
    /// STALE span and a rule change never reaches the shapes), then
    /// insert all. Java walks the item list for this clear
    /// explicitly (`:191-199`) even though `insertAllBoardItems`
    /// clears again per item (`:228`) — the double clear is ported
    /// literally; see [`SearchTreeManager::insert_all_board_items`].
    ///
    /// Mutation-verified: removing BOTH clears fails both stale-span
    /// pins below (`reinsert_tree_items_clears_stale_drill_spans`,
    /// `compensation_rebuild_clears_stale_drill_spans_through_insert_all`);
    /// removing ONLY this explicit walk changes nothing observable —
    /// it is Java-redundant with `insertAllBoardItems`'s own clear,
    /// kept for structure parity with `:191-199`.
    pub fn reinsert_tree_items(&mut self, board: &mut Board) {
        self.remove_all_board_items(board);
        let ids: Vec<ItemId> = board.iter_descending().map(|entry| entry.id).collect();
        for id in &ids {
            board.clear_derived_data(*id);
        }
        self.insert_all_board_items(board);
    }

    /// Java `insertAllBoardItems()` `:217-231` — every item
    /// DESCENDING id (D25/T60), each preceded by
    /// `clearDerivedData()` (`:228`), then broadcast-inserted. Java's
    /// method is private (the read path inserts item-by-item); the
    /// port makes it public as the one-shot fill after
    /// [`crate::board::Board::from_ses_board`] until the read
    /// integration (Task 7+) drives per-item inserts.
    pub fn insert_all_board_items(&mut self, board: &mut Board) {
        let ids: Vec<ItemId> = board.iter_descending().map(|entry| entry.id).collect();
        for id in ids {
            board.clear_derived_data(id);
            self.insert(board, id);
        }
    }

    /// Java `removeAllBoardItems()` `:202-215` — every item, through
    /// the guarded [`SearchTreeManager::remove`].
    fn remove_all_board_items(&mut self, board: &mut Board) {
        let ids: Vec<ItemId> = board.iter_descending().map(|entry| entry.id).collect();
        for id in ids {
            self.remove(board, id);
        }
    }

    /// The single-tree half of the broadcast insert: compute the
    /// item's shapes for THIS tree's configuration and store the
    /// returned leaves under the tree's object id (replacing any
    /// previous entries of the pair — Java's
    /// `setSearchTreeEntries` replaces). An empty shape list stores
    /// nothing (Java's `shapeCount <= 0` early-out).
    fn insert_into_tree(&mut self, board: &mut Board, id: ItemId, tree_index: usize) {
        let (variant, compensated_class, object_id) = {
            let tree = &self.trees[tree_index];
            (
                tree.variant,
                tree.compensated_clearance_class,
                tree.object_id(),
            )
        };
        let shapes = crate::tree_shapes::item_tree_shapes(board, variant, compensated_class, id);
        let leaves = self.trees[tree_index].insert(u64::from(id.get()), &shapes);
        if !leaves.is_empty() {
            self.entries
                .entry(id)
                .or_default()
                .insert(object_id, leaves);
        }
    }
}

impl Default for SearchTreeManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::BoardItemType;
    use crate::tree_shapes::clearance_compensation_value;
    use epic_dsn::reader::{DsnReadResult, read_board};
    use epic_dsn::ses_board::SesBoard;

    /// The Issue575 fixture (2 layers, FORTYFIVE restriction, 815
    /// items — the tree_shapes capture board). Via 815 (class 1,
    /// `Via[0-1]_600:300_um`, copper on BOTH layers) is the probe
    /// item; the outline (id 1) is the non-drill probe.
    const ISSUE575: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../fixtures/Issue575-drc_dev-board_4_hole_clearance_violations.dsn"
    );

    /// Parses a fixture through the epic-dsn reader and converts it.
    fn parse_board(path: &str) -> Board {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let mut ses = SesBoard::new();
        match read_board(bytes.as_slice(), &mut ses) {
            DsnReadResult::Success { .. } => {}
            other => panic!("expected Success for {path}, got {other:?}"),
        }
        Board::from_ses_board(&ses)
    }

    /// Makes BOTH relevant tree classes' compensation LAYER-SENSITIVE
    /// on Issue575 — the property the stale-span pins need (a wrong
    /// shape layer must change a leaf):
    ///
    /// * class-0 tree: `comp(1, 0, l) = 0` on layer 0, `5000` on
    ///   layer 1 (`v(1,0,l)`; `cc(0,l) = 0`),
    /// * class-1 tree: `comp(1, 1, 0) = 2000 - 1000 = 1000` but
    ///   `comp(1, 1, 1) = 8000 - 4000 = 4000` (`v(1,1,1) = 8000`
    ///   moves BOTH the cell and `cc(1,1)`).
    fn make_compensation_layer_sensitive(board: &mut Board) {
        board.rules_mut().clearance.set_value(1, 0, 0, 0);
        board.rules_mut().clearance.set_value(1, 0, 1, 5000);
        board.rules_mut().clearance.set_value(1, 1, 1, 8000);
    }

    /// Poisons a drill item's memoized layer span (the T58 staleness
    /// channel — Java's package-private `precalculatedFirstLayer`
    /// fields, writable here through the crate's test seam).
    /// Requires the memo to exist: insert the item first.
    fn poison_span(board: &mut Board, id: ItemId, first: i32, last: i32) {
        let precalc = board
            .drill_precalc_mut(id)
            .expect("the span memo exists after an insert");
        precalc.first_layer = Some(first);
        precalc.last_layer = Some(last);
    }

    /// The default tree is EXACTLY Java's constructor tree — the
    /// captured key form `ShapeSearchTree_FortyfiveDegree_cc0` — and
    /// slot 0 keeps being the default through every mutation; ONLY
    /// `set_clearance_compensation_used` replaces it (with the cc1
    /// key).
    #[test]
    fn slot_zero_is_the_default_tree_through_every_manager_mutation() {
        let mut board = parse_board(ISSUE575);
        let mut manager = SearchTreeManager::new();
        assert_eq!(manager.trees().len(), 1);
        assert_eq!(
            manager.trees()[0].key(),
            "ShapeSearchTree_FortyfiveDegree_cc0"
        );

        // Autoroute trees append; the retain-based drops keep slot 0.
        manager.get_autoroute_tree(&mut board, 1);
        manager.get_autoroute_tree(&mut board, 2);
        let default_id = manager.trees()[0].object_id();
        manager.clearance_value_changed(&mut board);
        assert_eq!(manager.trees()[0].object_id(), default_id);
        manager.clearance_class_removed(1);
        assert_eq!(manager.trees()[0].object_id(), default_id);
        // The default's own class: refused before anything is dropped.
        manager.clearance_class_removed(0);
        assert_eq!(manager.trees()[0].object_id(), default_id);
        manager.reset_compensated_trees();
        assert_eq!(manager.trees()[0].object_id(), default_id);
        assert_eq!(manager.trees().len(), 1);

        // Only the compensation rebuild replaces slot 0.
        manager.set_clearance_compensation_used(&mut board, true);
        assert_ne!(manager.trees()[0].object_id(), default_id);
        assert_eq!(
            manager.trees()[0].key(),
            "ShapeSearchTree_FortyfiveDegree_cc1"
        );
        assert_eq!(manager.trees().len(), 1);
    }

    /// `setClearanceCompensationUsed` `:89-108`: the early-out, the
    /// class 1/0 switch, the fresh default identity, and the
    /// re-filled tree.
    #[test]
    fn set_clearance_compensation_used_rebuilds_with_a_fresh_default_tree() {
        let mut board = parse_board(ISSUE575);
        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(&mut board);
        let old_id = manager.default_tree().object_id();
        let old_leaves = manager.default_tree().leaf_count();
        assert_eq!(manager.default_tree().compensated_clearance_class, 0);

        manager.set_clearance_compensation_used(&mut board, true);
        assert!(manager.is_clearance_compensation_used());
        assert_eq!(manager.trees().len(), 1);
        assert_ne!(manager.default_tree().object_id(), old_id);
        assert_eq!(manager.default_tree().compensated_clearance_class, 1);
        // Same items, same shape COUNT (compensation moves bounds,
        // not counts).
        assert_eq!(manager.default_tree().leaf_count(), old_leaves);
        // The items are back on the board.
        assert!(board.is_on_the_board(ItemId::new(815)));

        // A same-value call is the early-out: same identity.
        let rebuilt_id = manager.default_tree().object_id();
        manager.set_clearance_compensation_used(&mut board, true);
        assert_eq!(manager.default_tree().object_id(), rebuilt_id);

        // Back to false: class 0, yet another identity.
        manager.set_clearance_compensation_used(&mut board, false);
        assert!(!manager.is_clearance_compensation_used());
        assert_eq!(manager.default_tree().compensated_clearance_class, 0);
        assert_ne!(manager.default_tree().object_id(), rebuilt_id);
    }

    /// `clearanceValueChanged` `:111-119`: drops non-default-CLASS
    /// trees; the default keeps its IDENTITY (unlike the
    /// compensation rebuild) and is rebuilt only when the flag is on.
    #[test]
    fn clearance_value_changed_keeps_default_identity_and_rebuilds_only_with_the_flag() {
        // Flag off: the drop happens, the rebuild does NOT.
        let mut board = parse_board(ISSUE575);
        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(&mut board);
        manager.get_autoroute_tree(&mut board, 1);
        assert_eq!(manager.trees().len(), 2);
        let default_id = manager.default_tree().object_id();
        let default_dump = manager.default_tree().min_area_tree().dump_lines().clone();
        manager.clearance_value_changed(&mut board);
        assert_eq!(manager.trees().len(), 1, "the class-1 tree is dropped");
        assert_eq!(
            manager.default_tree().object_id(),
            default_id,
            "clearance_value_changed never replaces the default"
        );
        assert_eq!(
            manager.default_tree().min_area_tree().dump_lines(),
            default_dump,
            "flag off: no rebuild (Java :115-118)"
        );

        // Flag on: the default (class 1) is REBUILT with the changed
        // matrix — same identity, fresh shapes.
        let mut board = parse_board(ISSUE575);
        let mut manager = SearchTreeManager::new();
        manager.set_clearance_compensation_used(&mut board, true);
        manager.get_autoroute_tree(&mut board, 2);
        let default_id = manager.default_tree().object_id();
        board.rules_mut().clearance.set_value(1, 1, 0, 6000);
        manager.clearance_value_changed(&mut board);
        assert_eq!(manager.trees().len(), 1, "the class-2 tree is dropped");
        assert_eq!(manager.default_tree().object_id(), default_id);
        // The rebuild used the NEW matrix: equal to a clean manager
        // built under the same rules.
        let mut clean_board = parse_board(ISSUE575);
        clean_board.rules_mut().clearance.set_value(1, 1, 0, 6000);
        let mut clean = SearchTreeManager::new();
        clean.set_clearance_compensation_used(&mut clean_board, true);
        assert_eq!(
            manager.default_tree().min_area_tree().dump_lines(),
            clean.default_tree().min_area_tree().dump_lines(),
            "flag on: the default is re-inserted under the changed matrix"
        );
    }

    /// `clearanceClassRemoved` `:122-134`: the default's class is
    /// refused FIRST (nothing is dropped), other classes' trees go,
    /// and an unknown class is a no-op.
    #[test]
    fn clearance_class_removed_refuses_the_default_class_first() {
        let mut board = parse_board(ISSUE575);
        let mut manager = SearchTreeManager::new();
        manager.get_autoroute_tree(&mut board, 2);
        assert_eq!(manager.trees().len(), 2);

        // The default's own class (0): refused, the class-2 tree
        // SURVIVES the refusal too (the check precedes the walk).
        manager.clearance_class_removed(0);
        assert_eq!(
            manager.trees().len(),
            2,
            "the default-class refusal must not drop anything else"
        );

        manager.clearance_class_removed(2);
        assert_eq!(manager.trees().len(), 1);
        assert_eq!(manager.default_tree().compensated_clearance_class, 0);

        // An unknown class: no-op.
        manager.clearance_class_removed(7);
        assert_eq!(manager.trees().len(), 1);

        // Under compensation the default's class is 1 — removing 1
        // is refused, removing the extra class 2 is not.
        manager.set_clearance_compensation_used(&mut board, true);
        manager.get_autoroute_tree(&mut board, 2);
        manager.clearance_class_removed(1);
        assert_eq!(
            manager.trees().len(),
            2,
            "the compensated default is refused"
        );
        manager.clearance_class_removed(2);
        assert_eq!(manager.trees().len(), 1);
    }

    /// `getAutorouteTree` `:140-172` — **T59**: the index return,
    /// the default-tree scan hit for its own class, the variant per
    /// restriction, the bulk insert into the NEW TREE ONLY, and the
    /// no-flag bulk insert.
    #[test]
    fn get_autoroute_tree_selects_variant_and_bulk_inserts_only_the_new_tree() {
        let mut board = parse_board(ISSUE575);
        let mut manager = SearchTreeManager::new();

        // The default (class 0) is its own autoroute tree: the scan
        // hits, nothing is created.
        assert_eq!(manager.get_autoroute_tree(&mut board, 0), 0);
        assert_eq!(manager.trees().len(), 1);

        // FORTYFIVE restriction (the parse default): a NEW 45-degree
        // tree at index 1, bulk-filled — and the DEFAULT tree stays
        // EMPTY (the bulk insert reaches the new tree only).
        //
        // The flag pin needs an item that is NOT on the board:
        // `from_ses_board` items are born on_the_board=true (the
        // post-read Java state — the DSN read inserts through the
        // manager), so via 815 is forced back into Java's
        // constructed-but-uninserted state through the raw flag seam.
        board.set_on_the_board(ItemId::new(815), false);
        assert_eq!(manager.get_autoroute_tree(&mut board, 1), 1);
        assert_eq!(
            manager.trees()[1].variant,
            SearchTreeVariant::FortyfiveDegree
        );
        assert!(
            manager.trees()[1].leaf_count() > 0,
            "the bulk insert fills the new tree"
        );
        assert_eq!(
            manager.trees()[0].leaf_count(),
            0,
            "T59: the bulk insert must NOT touch the other trees"
        );
        // The bulk insert is the TREE's insert, not the manager's:
        // entries land in the new tree WITHOUT the on-the-board flag.
        assert!(!board.is_on_the_board(ItemId::new(815)));
        assert!(
            manager
                .tree_entries(ItemId::new(815), manager.trees()[1].object_id())
                .is_some(),
            "tree entries are independent of the flag"
        );

        // A repeat request scans and returns the SAME index.
        assert_eq!(manager.get_autoroute_tree(&mut board, 1), 1);
        assert_eq!(manager.trees().len(), 2);

        // NINETY -> the 90-degree subclass; NONE -> the base tree.
        board.rules_mut().trace_angle_restriction = AngleRestriction::NinetyDegree;
        assert_eq!(manager.get_autoroute_tree(&mut board, 2), 2);
        assert_eq!(manager.trees()[2].variant, SearchTreeVariant::NinetyDegree);
        board.rules_mut().trace_angle_restriction = AngleRestriction::None;
        assert_eq!(manager.get_autoroute_tree(&mut board, 3), 3);
        assert_eq!(manager.trees()[3].variant, SearchTreeVariant::Generic);
    }

    /// `insert` `:39-44` / `remove` `:47-62`: the broadcast to ALL
    /// trees, the per-tree entry drop, the on-the-board flag, and
    /// the remove guard.
    #[test]
    fn insert_broadcasts_to_all_trees_and_remove_drops_every_entry() {
        let mut board = parse_board(ISSUE575);
        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(&mut board);
        manager.get_autoroute_tree(&mut board, 1);
        let via = ItemId::new(815);
        assert!(board.is_on_the_board(via));
        let before: Vec<usize> = manager.trees().iter().map(|t| t.leaf_count()).collect();
        assert!(before[0] > 0);
        assert_eq!(
            before[1], before[0],
            "same directions: the class-1 tree holds the same leaf count"
        );
        assert!(
            manager
                .tree_entries(via, manager.trees()[0].object_id())
                .is_some(),
            "the via has entries in the default tree"
        );

        // Remove: BOTH trees lose the via's two shapes; entries and
        // the flag go with them.
        manager.remove(&mut board, via);
        assert_eq!(manager.trees()[0].leaf_count(), before[0] - 2);
        assert_eq!(manager.trees()[1].leaf_count(), before[1] - 2);
        assert!(!board.is_on_the_board(via));
        assert!(
            manager
                .tree_entries(via, manager.trees()[0].object_id())
                .is_none()
        );

        // A second remove is the guarded no-op.
        manager.remove(&mut board, via);
        assert_eq!(manager.trees()[0].leaf_count(), before[0] - 2);

        // Insert: broadcasts into EVERY live tree.
        manager.insert(&mut board, via);
        assert_eq!(manager.trees()[0].leaf_count(), before[0]);
        assert_eq!(manager.trees()[1].leaf_count(), before[1]);
        assert!(board.is_on_the_board(via));
    }

    /// The Task-7 scope boundary: non-drill kinds carry NO tree
    /// entries (Java's `shapeCount <= 0` early-out) — but the
    /// broadcast insert still flips their on-the-board flag (`:43`).
    #[test]
    fn non_drill_items_carry_no_entries_but_still_get_the_flag() {
        let mut board = parse_board(ISSUE575);
        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(&mut board);
        let outline = ItemId::new(1);
        assert_eq!(
            board
                .get(outline)
                .expect("the outline is id 1 on every parse")
                .board_item_type(),
            BoardItemType::BoardOutline
        );
        assert!(
            manager
                .tree_entries(outline, manager.default_tree().object_id())
                .is_none(),
            "no shapes -> no entries"
        );
        assert!(
            board.is_on_the_board(outline),
            "the flag is set regardless of shape count"
        );
        // A foreign id has no entries either.
        assert!(
            manager
                .tree_entries(ItemId::new(999_999), manager.default_tree().object_id())
                .is_none()
        );
    }

    /// `resetCompensatedTrees` `:177-179` — IDENTITY comparison: only
    /// the default survives, and a re-requested class creates a NEW
    /// tree identity (the dropped one is really gone).
    #[test]
    fn reset_compensated_trees_keeps_only_the_default_identity() {
        let mut board = parse_board(ISSUE575);
        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(&mut board);
        manager.get_autoroute_tree(&mut board, 1);
        let dropped_id = manager.trees()[1].object_id();
        let default_id = manager.trees()[0].object_id();

        manager.reset_compensated_trees();
        assert_eq!(manager.trees().len(), 1);
        assert_eq!(manager.trees()[0].object_id(), default_id);

        // Re-requesting the class allocates a NEW identity.
        let index = manager.get_autoroute_tree(&mut board, 1);
        assert_eq!(index, 1);
        assert_ne!(manager.trees()[1].object_id(), dropped_id);
        assert_eq!(manager.trees().len(), 2);
    }

    /// **T58 — the clear between remove and insert.** A poisoned
    /// drill span survives `remove` (removal clears tree entries,
    /// NOT the span memo); `reinsertTreeItems` must clear it, or the
    /// re-inserted leaves carry the stale span's layer compensation.
    /// The layer-sensitive matrix makes the poison observable: shape
    /// 0 (layer 0, comp 0) vs shape 1 (layer 1, comp 5000).
    #[test]
    fn reinsert_tree_items_clears_stale_drill_spans() {
        let mut board = parse_board(ISSUE575);
        make_compensation_layer_sensitive(&mut board);
        // The branch-executing witness: the two layers' compensations
        // differ, so a wrong layer assignment changes a leaf.
        assert_eq!(
            clearance_compensation_value(board.rules(), 1, 0, 0),
            0,
            "layer 0 comp (the control)"
        );
        assert_eq!(
            clearance_compensation_value(board.rules(), 1, 0, 1),
            5000,
            "layer 1 comp — the discriminating form"
        );

        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(&mut board);
        let via = ItemId::new(815);
        let fresh = manager.default_tree().min_area_tree().dump_lines().clone();
        assert!(fresh.len() > 1, "the via contributes both shapes");

        // Poison: both shapes read layer 1 (span 1..=1).
        poison_span(&mut board, via, 1, 1);
        assert_eq!(
            board.drill_shape_layer(via, 0),
            Some(1),
            "the poison took: shape 0 now reads layer 1"
        );

        // The reinsert must come out byte-identical to the fresh
        // insert — the clear restored the true span.
        manager.reinsert_tree_items(&mut board);
        assert_eq!(
            manager.default_tree().min_area_tree().dump_lines(),
            fresh,
            "T58: the stale span must not survive into the re-inserted leaves"
        );
        // And the span itself is fresh again.
        assert_eq!(board.drill_first_layer(via), Some(0));
    }

    /// **T58, the second clear site** — `insertAllBoardItems`'s own
    /// per-item clear (`:228`): the compensation rebuild drives
    /// remove-all + insert-all WITHOUT `reinsertTreeItems`, so that
    /// clear is the only thing standing between a poisoned span and
    /// the new default tree.
    #[test]
    fn compensation_rebuild_clears_stale_drill_spans_through_insert_all() {
        let mut board = parse_board(ISSUE575);
        make_compensation_layer_sensitive(&mut board);
        // The branch-executing witness for the CLASS-1 tree (the
        // rebuild's default): its compensation differs per layer too.
        assert_eq!(
            clearance_compensation_value(board.rules(), 1, 1, 0),
            1000,
            "layer 0 comp in a class-1 tree (the control)"
        );
        assert_eq!(
            clearance_compensation_value(board.rules(), 1, 1, 1),
            4000,
            "layer 1 comp in a class-1 tree — the discriminating form"
        );
        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(&mut board);
        let via = ItemId::new(815);
        poison_span(&mut board, via, 1, 1);

        manager.set_clearance_compensation_used(&mut board, true);

        // Reference: a clean board under the same rules and the same
        // flag. If the rebuild reused the poisoned span, the via's
        // layer-0 leaf would take the layer-1 compensation.
        let mut clean_board = parse_board(ISSUE575);
        make_compensation_layer_sensitive(&mut clean_board);
        let mut clean = SearchTreeManager::new();
        clean.set_clearance_compensation_used(&mut clean_board, true);
        assert_eq!(
            manager.default_tree().min_area_tree().dump_lines(),
            clean.default_tree().min_area_tree().dump_lines(),
            "the insert_all clear must restore the fresh span before re-inserting"
        );
    }
}
