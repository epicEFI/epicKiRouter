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
//! `validateEntries` (`:69-78`) and the overlap query family
//! (`ShapeSearchTree.java:390-570`, `BasicBoard.checkShape`) are
//! Task 8 — see the query section of the impl.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use epic_geometry::int_box::IntBox;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::tile_shape::TileShape;
use epic_index::{LeafEntry, NodeIdx, SearchTree, SearchTreeVariant, TreeEntry};

use crate::board::Board;
use crate::id::ItemId;
use crate::items::{Area, ItemData};
use crate::rules_surf::AngleRestriction;

/// Manages the search trees used in the auto-router (Java
/// `SearchTreeManager`): creation, per-item insertion/removal across
/// ALL trees, and the rule-change rebuilds.
#[derive(Debug)]
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
    /// fill is [`SearchTreeManager::insert_items_creation_order`],
    /// which the caller runs once after
    /// [`crate::board::Board::from_ses_board`].
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
        self.trees
            .first()
            .expect("slot 0 is always the default tree (slot_zero pin)")
    }

    /// The live trees, slot 0 first (the module's slot-0 invariant).
    #[must_use]
    pub fn trees(&self) -> &[SearchTree] {
        &self.trees
    }

    /// The live trees, mutable — the autoroute engine's room-tree
    /// insert/remove through the resolved slot (Java reaches the tree
    /// OBJECT through `autorouteSearchTree`; the port goes through the
    /// manager slot index).
    pub fn trees_mut(&mut self) -> &mut [SearchTree] {
        &mut self.trees
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
    /// lookup), and Java's `item.clearSearchTreeEntries()`
    /// (`Item.java:1078-1080`) sets `searchTreesInfo = null` — the
    /// container holding BOTH the leaf entries AND the precalculated
    /// tree shapes — so the port drops its entry map AND the item's
    /// shape cache ([`Board::clear_search_tree_shapes`]); then the
    /// flag clears. A later insert therefore RE-COMPUTES the shapes
    /// (jar-verified: the keepoutOutside flip re-inserts the outline
    /// with fresh AREA shapes, see
    /// [`SearchTreeManager::generate_keepout_outside`]).
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
        board.clear_search_tree_shapes(id);
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
        let default_class = self
            .trees
            .first()
            .expect("slot 0 is always the default tree (slot_zero pin)")
            .compensated_clearance_class;
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
        let default_class = self
            .trees
            .first()
            .expect("slot 0 is always the default tree (slot_zero pin)")
            .compensated_clearance_class;
        if class_no == default_class {
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
        // The `startReadObject` walk (`:163-170`) — DESCENDING id (the
        // ConcurrentSkipListMap order, see
        // [`SearchTreeManager::insert_all_board_items`]) and LIVE-only:
        // Java's `UndoableObjects.delete` unlinks the node from the
        // map (`objects.remove`), while the port keeps flag-only
        // entries, so the walk must filter them (a deleted-then-merged
        // trace must not re-enter a fresh tree).
        let ids: Vec<ItemId> = board
            .iter_descending()
            .filter(|entry| entry.on_the_board)
            .map(|entry| entry.id)
            .collect();
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
        let default_object_id = self
            .trees
            .first()
            .expect("slot 0 is always the default tree (slot_zero pin)")
            .object_id();
        self.trees
            .retain(|tree| tree.object_id() == default_object_id);
    }

    /// Java `BoardOutline.generateKeepoutOutside(boolean)`
    /// (`BoardOutline.java:234-244`): the same-value early-out, the
    /// flag flip, then a REMOVE + INSERT of the outline item through
    /// the manager — with NO `clearDerivedData` of its own. The
    /// re-insert still computes the AREA branch: Java's remove nulls
    /// the whole `searchTreesInfo` (`Item.java:1078-1080`), so the
    /// shape cache does not survive the round trip (the Task 7 jar
    /// capture — `/tmp/epic-t7-shapes.out`, `KEEPOUT_OUTLINE` — shows
    /// the post-flip default tree carrying the area shapes: 8 fresh
    /// octagons on Issue575 where the parse cached 8 LINE octagons of
    /// different values, and 136 vs 132 shapes on Issue054). Pinned in
    /// the tests (`keepout_outside_reinsert_computes_the_area_branch`,
    /// `remove_clears_the_tree_shape_cache_between_out_and_in`).
    pub fn generate_keepout_outside(&mut self, board: &mut Board, id: ItemId, value: bool) {
        // Java's method lives ON `BoardOutline` (`BoardOutline.java:234`)
        // — a non-outline id cannot even call it. The flag guard alone
        // would let a foreign id through to the broadcast insert (T7
        // quality review NIT-2), so the kind check restores the type
        // restriction.
        if !matches!(
            board.get(id).map(|entry| &entry.data),
            Some(ItemData::BoardOutline { .. })
        ) {
            return;
        }
        if board.outline_keepout_outside_generated(id) == Some(value) {
            // Java :235-237.
            return;
        }
        board.set_outline_keepout_outside(id, value);
        // Java :239-244 — board/manager null checks are structural
        // here (the port's manager always has its board argument).
        self.remove(board, id);
        self.insert(board, id);
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
    /// DESCENDING id, each preceded by `clearDerivedData()` (`:228`),
    /// then broadcast-inserted. The order is not a style choice: Java's
    /// `board.itemList` is a `ConcurrentSkipListMap` keyed by the item
    /// itself (`UndoableObjects.java:21,37`), and `Item.compareTo` is
    /// `other.id - id` (`Item.java:95-103`) — so the
    /// `startReadObject()` walk (`UndoableObjects.java:46-48`) yields
    /// DESCENDING id. The MinAreaTree is insertion-structured, so this
    /// walk order IS the final tree skeleton (the two-path record,
    /// [`SearchTreeManager::insert_items_creation_order`]); pinned
    /// byte-for-row by the T9 index goldens, which capture exactly
    /// this rebuild path.
    ///
    /// Java's method is private (the read path inserts item-by-item);
    /// the port keeps it public for the REBUILD callers
    /// ([`SearchTreeManager::reinsert_tree_items`],
    /// [`SearchTreeManager::set_clearance_compensation_used`],
    /// [`SearchTreeManager::clearance_value_changed`]).
    pub fn insert_all_board_items(&mut self, board: &mut Board) {
        // NOTE: deliberately UNFILTERED. Java's walk is live-only
        // (`UndoableObjects.delete` unlinks the node), but the port
        // keeps flag-only entries and this method is ALSO the parse
        // path's flag-flipping step (`reinsert_tree_items`): parsed
        // entries are born with `on_the_board == false`, so filtering
        // here would un-live the whole board. The live-only contract
        // is enforced on the POST-parse walks instead
        // ([`SearchTreeManager::get_autoroute_tree`], the harness
        // freeze loops) and by `combine`'s own tree-entry removal.
        let ids: Vec<ItemId> = board.iter_descending().map(|entry| entry.id).collect();
        for id in ids {
            board.clear_derived_data(id);
            self.insert(board, id);
        }
    }

    /// The READ-path fill — Java has no such method: there, every item
    /// enters the trees at CREATION time (the repository insert's
    /// observer callback), so a parsed board's tree is built in
    /// creation order, which for parsed boards is ASCENDING id. The
    /// port's parse creates items without inserting, so this one-shot
    /// fill (after [`crate::board::Board::from_ses_board`]) replays
    /// that ascending insertion sequence. Contrast
    /// [`SearchTreeManager::insert_all_board_items`]: Java's REBUILD
    /// walk (the `ConcurrentSkipListMap` iteration) is DESCENDING, and
    /// the two skeletons genuinely differ — the MinAreaTree is
    /// insertion-structured. The CombineSpike captures
    /// (`/tmp/epic-t11-combine.out` `B_TREE`, pinned row-for-row in
    /// the trace_ops tests) are read-path trees: they match THIS fill,
    /// not the rebuild.
    pub fn insert_items_creation_order(&mut self, board: &mut Board) {
        let ids: Vec<ItemId> = board.iter_ascending().map(|entry| entry.id).collect();
        for id in ids {
            board.clear_derived_data(id);
            self.insert(board, id);
        }
    }

    /// Java `removeAllBoardItems()` `:202-215` — every item, through
    /// the guarded [`SearchTreeManager::remove`]. Same walk order as
    /// the insert side: the `ConcurrentSkipListMap` iteration yields
    /// DESCENDING id.
    fn remove_all_board_items(&mut self, board: &mut Board) {
        let ids: Vec<ItemId> = board.iter_descending().map(|entry| entry.id).collect();
        for id in ids {
            self.remove(board, id);
        }
    }

    /// The single-tree half of the broadcast insert: fetch the item's
    /// shapes for THIS tree through the per-item per-tree SHAPE CACHE
    /// (Java `Item.getTreeShape(tree, i)` lazily computing through
    /// `getPrecalculatedTreeShapes` inside `ShapeTree.insert`,
    /// `Item.java:228-238` — the cache is identity-keyed and cleared
    /// by [`Board::clear_derived_data`]) and store the returned
    /// leaves under the tree's object id (replacing any previous
    /// entries of the pair — Java's `setSearchTreeEntries` replaces).
    /// An empty shape list stores nothing (Java's `shapeCount <= 0`
    /// early-out).
    fn insert_into_tree(&mut self, board: &mut Board, id: ItemId, tree_index: usize) {
        let (variant, compensated_class, object_id) = {
            let tree = &self.trees[tree_index];
            (
                tree.variant,
                tree.compensated_clearance_class,
                tree.object_id(),
            )
        };
        let shapes = board.tree_shape_precalc(id, object_id, variant, compensated_class);
        let leaves = self.trees[tree_index].insert(u64::from(id.get()), &shapes);
        if !leaves.is_empty() {
            self.entries
                .entry(id)
                .or_default()
                .insert(object_id, leaves);
        }
    }

    /// Java `PolylineTraceSearchTreeAdapter.hasDefaultEntries`
    /// (`:22-26`): both traces carry an entry array in the DEFAULT
    /// tree — the combine fast-path gate (PolylineTrace.java:307/:431).
    #[must_use]
    pub fn has_default_entries(&self, first: ItemId, second: ItemId) -> bool {
        let object_id = self.default_tree().object_id();
        self.tree_entries(first, object_id).is_some()
            && self.tree_entries(second, object_id).is_some()
    }

    /// The to-trace's compensated half width for ONE tree (Java
    /// `toTrace.getHalfWidth() + this.clearanceCompensationValue(
    /// toTrace.clearanceClassIndex(), toTrace.getLayer())` —
    /// ShapeSearchTree.java:194-196/:269-271) — the same arithmetic as
    /// [`crate::tree_shapes::trace_compensated_half_width`] but from
    /// the copied `compensated_clearance_class`, so the per-tree loop
    /// can compute it without holding a tree borrow.
    fn tree_half_width(&self, board: &Board, trace_id: ItemId, compensated_class: i32) -> i32 {
        let half_width = board.trace_half_width(trace_id).unwrap_or(0);
        let class = board.item_clearance_class(trace_id).unwrap_or(0);
        let layer = board.trace_layer(trace_id).unwrap_or(0);
        half_width
            + crate::tree_shapes::clearance_compensation_value(
                board.rules(),
                class,
                compensated_class,
                layer,
            )
    }

    /// Java `SearchTreeManager.mergeEntriesInFront`
    /// (`SearchTreeManager.java:237-246`) broadcasting over every live
    /// tree to `ShapeSearchTree.mergeEntriesInFront`
    /// (`ShapeSearchTree.java:170-239`): the combine-at-start fast
    /// path. Per tree the op sequence is REMOVED-then-INSERTED —
    ///
    /// 1. `removeLeaf(fromEntries[removeNo])` (`:189`; first entry
    ///    when the join REVERSES the from-trace, else its last),
    /// 2. `removeLeaf(toEntries[0])` (`:190`),
    /// 3. one `insert(toTrace, i)` per LINK shape (`:233-236`),
    ///
    /// while every SURVIVING entry is TRANSFERRED in place — its leaf
    /// keeps the exact tree position, only `(object, shapeIndex)`
    /// are re-written (`:214-215` for the from-trace half, `:221`
    /// for the to-trace half). The per-tree shape cache of the
    /// SURVIVING to-trace is rebuilt from both caches plus the link
    /// shapes and installed BEFORE the inserts (`:224-230`), because
    /// the inserts read it.
    ///
    /// The from-trace's entry arrays are left STALE here (their live
    /// slots were re-labeled away) — Java's caller clears them right
    /// after ([`Self::clear_search_tree_entries`],
    /// PolylineTrace.java:321).
    pub fn merge_entries_in_front(
        &mut self,
        board: &mut Board,
        from_trace: ItemId,
        to_trace: ItemId,
        joined: &Polyline,
        from_entry_no: i32,
        to_entry_no: i32,
    ) {
        // Java :176: `fromTrace.firstCorner().equals(toTrace.firstCorner())`.
        let change_order = {
            let from_lines = board
                .trace_polyline(from_trace)
                .cloned()
                .unwrap_or_else(|| Polyline::new(Vec::new()));
            let to_lines = board
                .trace_polyline(to_trace)
                .cloned()
                .unwrap_or_else(|| Polyline::new(Vec::new()));
            crate::items::trace::first_corner(&from_lines)
                .zip(crate::items::trace::first_corner(&to_lines))
                .is_some_and(|(a, b)| a == b)
        };
        for tree_index in 0..self.trees.len() {
            let (object_id, compensated_class) = {
                let tree = &self.trees[tree_index];
                (tree.object_id(), tree.compensated_clearance_class)
            };
            let Some(from_entries) = self
                .entries
                .get(&from_trace)
                .and_then(|by_tree| by_tree.get(&object_id))
                .cloned()
            else {
                continue;
            };
            let Some(to_entries) = self
                .entries
                .get(&to_trace)
                .and_then(|by_tree| by_tree.get(&object_id))
                .cloned()
            else {
                continue;
            };
            let to_key = u64::from(to_trace.get());
            let from_shape_count_minus_1 = from_entries.len() - 1;
            let remove_no = if change_order {
                0
            } else {
                from_shape_count_minus_1
            };
            // :189, :190 — the two replaced entries.
            self.trees[tree_index].remove(&[from_entries[remove_no], to_entries[0]]);
            let link_shapes = joined.offset_shapes(
                self.tree_half_width(board, to_trace, compensated_class),
                from_entry_no,
                to_entry_no,
            );
            let new_shape_count = from_entries.len() + link_shapes.len() + to_entries.len() - 2;
            let old_to_count = to_entries.len();
            let mut new_leaf_arr: Vec<Option<NodeIdx>> = vec![None; new_shape_count];
            let mut new_precalc: Vec<Option<TileShape>> = vec![None; new_shape_count];
            // :205-216 — the surviving FROM entries transfer to the
            // head of the new list (reversed when the join reversed).
            for i in 0..from_shape_count_minus_1 {
                let from_no = if change_order {
                    from_shape_count_minus_1 - i
                } else {
                    i
                };
                new_precalc[i] = board
                    .tree_shape_precalc_peek(from_trace, object_id)
                    .and_then(|shapes| shapes.get(from_no))
                    .and_then(|slot| slot.as_ref().cloned());
                new_leaf_arr[i] = from_entries[from_no];
                if let Some(leaf) = new_leaf_arr[i] {
                    self.trees[tree_index].relabel_leaf(leaf, to_key, i as u32);
                }
            }
            // :217-222 — the surviving TO entries (all but index 0,
            // which was removed) keep their shapes and move to the
            // tail. Index arithmetic is the Java shape: `i` offsets
            // both the peek window and the destination index.
            #[allow(clippy::needless_range_loop)]
            for i in 1..old_to_count {
                let current_ind = from_shape_count_minus_1 + link_shapes.len() + i - 1;
                new_precalc[current_ind] = board
                    .tree_shape_precalc_peek(to_trace, object_id)
                    .and_then(|shapes| shapes.get(i))
                    .and_then(|slot| slot.as_ref().cloned());
                new_leaf_arr[current_ind] = to_entries[i];
                if let Some(leaf) = new_leaf_arr[current_ind] {
                    self.trees[tree_index].relabel_leaf(leaf, to_key, current_ind as u32);
                }
            }
            // :226-230 — the link shapes enter the cache BEFORE the
            // inserts (the inserts read the cache).
            for (k, shape) in link_shapes.iter().enumerate() {
                new_precalc[from_shape_count_minus_1 + k] = Some(shape.clone());
            }
            board.set_tree_shape_precalc(to_trace, object_id, new_precalc);
            // :233-236 — the new link entries.
            for (k, shape) in link_shapes.iter().enumerate() {
                new_leaf_arr[from_shape_count_minus_1 + k] = self.trees[tree_index].insert_one(
                    to_key,
                    (from_shape_count_minus_1 + k) as u32,
                    Some(shape),
                );
            }
            self.entries
                .entry(to_trace)
                .or_default()
                .insert(object_id, new_leaf_arr);
        }
    }

    /// Java `SearchTreeManager.mergeEntriesAtEnd` (`:252-261`)
    /// broadcasting to `ShapeSearchTree.mergeEntriesAtEnd`
    /// (`ShapeSearchTree.java:245-312`): the combine-at-end fast
    /// path. Per tree: `removeLeaf(toEntries[last])` (`:258`), then
    /// `removeLeaf(fromEntries[removeNo])` (`:265`; the from-trace's
    /// LAST entry when the join reversed it, else its first), then
    /// one insert per link shape (`:307-310`); the surviving to-head
    /// keeps its slots untouched (`:279-282`) while the surviving
    /// from-entries transfer with re-written owners (`:284-296`).
    pub fn merge_entries_at_end(
        &mut self,
        board: &mut Board,
        from_trace: ItemId,
        to_trace: ItemId,
        joined: &Polyline,
        from_entry_no: i32,
        to_entry_no: i32,
    ) {
        // Java :251: `fromTrace.lastCorner().equals(toTrace.lastCorner())`.
        let change_order = {
            let from_lines = board
                .trace_polyline(from_trace)
                .cloned()
                .unwrap_or_else(|| Polyline::new(Vec::new()));
            let to_lines = board
                .trace_polyline(to_trace)
                .cloned()
                .unwrap_or_else(|| Polyline::new(Vec::new()));
            crate::items::trace::last_corner(&from_lines)
                .zip(crate::items::trace::last_corner(&to_lines))
                .is_some_and(|(a, b)| a == b)
        };
        for tree_index in 0..self.trees.len() {
            let (object_id, compensated_class) = {
                let tree = &self.trees[tree_index];
                (tree.object_id(), tree.compensated_clearance_class)
            };
            let Some(from_entries) = self
                .entries
                .get(&from_trace)
                .and_then(|by_tree| by_tree.get(&object_id))
                .cloned()
            else {
                continue;
            };
            let Some(to_entries) = self
                .entries
                .get(&to_trace)
                .and_then(|by_tree| by_tree.get(&object_id))
                .cloned()
            else {
                continue;
            };
            let to_key = u64::from(to_trace.get());
            let from_count = from_entries.len();
            let to_shape_count_minus_1 = to_entries.len() - 1;
            // :258, :265 — order matters: the TO entry first.
            let remove_no = if change_order { from_count - 1 } else { 0 };
            self.trees[tree_index].remove(&[to_entries[to_shape_count_minus_1]]);
            self.trees[tree_index].remove(&[from_entries[remove_no]]);
            let link_shapes = joined.offset_shapes(
                self.tree_half_width(board, to_trace, compensated_class),
                from_entry_no,
                to_entry_no,
            );
            let new_shape_count = from_count + link_shapes.len() + to_entries.len() - 2;
            let mut new_leaf_arr: Vec<Option<NodeIdx>> = vec![None; new_shape_count];
            let mut new_precalc: Vec<Option<TileShape>> = vec![None; new_shape_count];
            // :279-282 — the surviving TO head: same slots, same
            // owners (no re-labeling in Java either).
            for i in 0..to_shape_count_minus_1 {
                new_precalc[i] = board
                    .tree_shape_precalc_peek(to_trace, object_id)
                    .and_then(|shapes| shapes.get(i))
                    .and_then(|slot| slot.as_ref().cloned());
                new_leaf_arr[i] = to_entries[i];
            }
            // :284-296 — the surviving FROM entries transfer to the
            // tail (reversed when the join reversed the from-trace).
            for i in 1..from_count {
                let current_ind = to_shape_count_minus_1 + link_shapes.len() + i - 1;
                let from_no = if change_order { from_count - i - 1 } else { i };
                new_precalc[current_ind] = board
                    .tree_shape_precalc_peek(from_trace, object_id)
                    .and_then(|shapes| shapes.get(from_no))
                    .and_then(|slot| slot.as_ref().cloned());
                new_leaf_arr[current_ind] = from_entries[from_no];
                if let Some(leaf) = new_leaf_arr[current_ind] {
                    self.trees[tree_index].relabel_leaf(leaf, to_key, current_ind as u32);
                }
            }
            // :300-304 — link shapes into the cache before the inserts.
            for (k, shape) in link_shapes.iter().enumerate() {
                new_precalc[to_shape_count_minus_1 + k] = Some(shape.clone());
            }
            board.set_tree_shape_precalc(to_trace, object_id, new_precalc);
            // :307-310 — the new link entries.
            for (k, shape) in link_shapes.iter().enumerate() {
                new_leaf_arr[to_shape_count_minus_1 + k] = self.trees[tree_index].insert_one(
                    to_key,
                    (to_shape_count_minus_1 + k) as u32,
                    Some(shape),
                );
            }
            self.entries
                .entry(to_trace)
                .or_default()
                .insert(object_id, new_leaf_arr);
        }
    }

    /// Java `SearchTreeManager.changeEntries` (`:267-272`) broadcasting
    /// to `ShapeSearchTree.changeEntries` (`ShapeSearchTree.java:120-164`):
    /// the `PolylineTrace.change` fast path. Per tree: the KEPT head
    /// ([0, keepAtStart)) and the KEPT tail (the last keepAtEnd
    /// entries, re-indexed to the new list's tail) keep their leaves in
    /// place; the replaced middle is REMOVED (`:142-144`) before the
    /// new middle is INSERTED (`:160-162`), with the fresh shapes
    /// installed into the cache first (`:154-158`).
    pub fn change_entries(
        &mut self,
        board: &mut Board,
        obj: ItemId,
        new_polyline: &Polyline,
        keep_at_start_count: i32,
        keep_at_end_count: i32,
    ) {
        let keep_at_start = keep_at_start_count.max(0) as usize;
        let keep_at_end = keep_at_end_count.max(0) as usize;
        let new_len = i32::try_from(new_polyline.lines.len()).unwrap_or(i32::MAX);
        for tree_index in 0..self.trees.len() {
            let (object_id, compensated_class) = {
                let tree = &self.trees[tree_index];
                (tree.object_id(), tree.compensated_clearance_class)
            };
            let Some(old_entries) = self
                .entries
                .get(&obj)
                .and_then(|by_tree| by_tree.get(&object_id))
                .cloned()
            else {
                continue;
            };
            let key = u64::from(obj.get());
            // :127-132 — the replacement shapes over the new
            // polyline's middle windows.
            let changed_shapes = new_polyline.offset_shapes(
                self.tree_half_width(board, obj, compensated_class),
                keep_at_start_count,
                new_len - 1 - keep_at_end_count,
            );
            let old_shape_count = old_entries.len();
            let new_shape_count = changed_shapes.len() + keep_at_start + keep_at_end;
            let mut new_leaf_arr: Vec<Option<NodeIdx>> = vec![None; new_shape_count];
            let mut new_precalc: Vec<Option<TileShape>> = vec![None; new_shape_count];
            // :138-141 — the kept head.
            for (i, slot) in old_entries.iter().enumerate().take(keep_at_start) {
                new_leaf_arr[i] = *slot;
                new_precalc[i] = board
                    .tree_shape_precalc_peek(obj, object_id)
                    .and_then(|shapes| shapes.get(i))
                    .and_then(|shape| shape.as_ref().cloned());
            }
            // :142-144 — remove the replaced middle (BEFORE any
            // insert, in ascending index order).
            for slot in old_entries.iter().skip(keep_at_start).take(
                old_shape_count
                    .saturating_sub(keep_at_end)
                    .saturating_sub(keep_at_start),
            ) {
                self.trees[tree_index].remove(&[*slot]);
            }
            // :145-152 — the kept tail re-indexed to the new list's
            // tail (shape cache entries move with them).
            for i in 0..keep_at_end {
                let new_index = new_shape_count - keep_at_end + i;
                let old_index = old_shape_count - keep_at_end + i;
                new_leaf_arr[new_index] = old_entries[old_index];
                if let Some(leaf) = new_leaf_arr[new_index] {
                    self.trees[tree_index].relabel_leaf(leaf, key, new_index as u32);
                }
                new_precalc[new_index] = board
                    .tree_shape_precalc_peek(obj, object_id)
                    .and_then(|shapes| shapes.get(old_index))
                    .and_then(|shape| shape.as_ref().cloned());
            }
            // :154-158 — the fresh middle shapes enter the cache
            // before the inserts read it.
            for (k, shape) in changed_shapes.iter().enumerate() {
                new_precalc[keep_at_start + k] = Some(shape.clone());
            }
            board.set_tree_shape_precalc(obj, object_id, new_precalc);
            // :160-162 — the new middle entries.
            for i in keep_at_start..(new_shape_count - keep_at_end) {
                new_leaf_arr[i] = self.trees[tree_index].insert_one(
                    key,
                    i as u32,
                    Some(&changed_shapes[i - keep_at_start]),
                );
            }
            // :163 — the new entry list replaces the old.
            self.entries
                .entry(obj)
                .or_default()
                .insert(object_id, new_leaf_arr);
        }
    }

    /// Java `Item.clearSearchTreeEntries()` (`Item.java:1078-1080`)
    /// reached through the adapter (PolylineTrace.java:321/:445): the
    /// combine fast paths leave the joined-from trace's arrays stale
    /// (their live slots were re-labeled away), so the caller drops
    /// the WHOLE `searchTreesInfo` — every tree's entry list plus the
    /// shape caches. Unlike [`Self::remove`] the on-the-board flag is
    /// NOT touched (the item is removed separately right after).
    pub fn clear_search_tree_entries(&mut self, board: &mut Board, id: ItemId) {
        self.entries.remove(&id);
        board.clear_search_tree_shapes(id);
    }

    /// Java `SearchTreeManager.reuseEntriesAfterCutout`
    /// (`SearchTreeManager.java:278-284`) broadcasting over every live
    /// tree to `ShapeSearchTree.reuseEntriesAfterCutout`
    /// (`ShapeSearchTree.java:318-350`): the tree entries of
    /// `from_trace` are split onto `start_piece` / `end_piece` after a
    /// middle piece was cut out (`ShapeTraceEntries.fastCutoutTrace`).
    ///
    /// Per tree the op sequence is: the untouched head slots of
    /// `from_trace` ([0, start_len-1)) are RE-LABELED to the start
    /// piece IN PLACE (`:323-328` — no structural op), then exactly
    /// TWO leaves are inserted — the start piece's LAST and the end
    /// piece's FIRST (`:329-330`/:337), the cutline straddlers — and
    /// the untouched tail slots (the end piece's [1, end_len)) are
    /// re-labeled to the end piece (`:339-345`). The MIDDLE
    /// (cutline) leaves stay owned by `from_trace` in the tree; the
    /// caller's `removeItem(fromTrace)` drops exactly those through
    /// the nulled-out arrays (`ShapeTree.remove` skips null slots).
    ///
    /// The pieces are repository-inserted but never tree-inserted
    /// (fastCutoutTrace calls `itemList.insert` +
    /// `setOnTheBoard(true)` only), so their per-tree shape caches
    /// fill lazily at the first `getTreeShape` — the port's
    /// [`Board::tree_shape_precalc`] fill.
    pub fn reuse_entries_after_cutout(
        &mut self,
        board: &mut Board,
        from_trace: ItemId,
        start_piece: ItemId,
        end_piece: ItemId,
    ) {
        for tree_index in 0..self.trees.len() {
            let (object_id, compensated_class, variant) = {
                let tree = &self.trees[tree_index];
                (
                    tree.object_id(),
                    tree.compensated_clearance_class,
                    tree.variant,
                )
            };
            // Java :321: `fromTrace.getSearchTreeEntries(this)` — the
            // piece piece counts come from the PIECE polylines
            // (`:320`/:334), not the from-trace.
            let Some(from_entries) = self
                .entries
                .get(&from_trace)
                .and_then(|by_tree| by_tree.get(&object_id))
                .cloned()
            else {
                continue;
            };
            let start_len = board
                .trace_polyline(start_piece)
                .map(|lines| lines.lines.len().saturating_sub(2))
                .unwrap_or(0);
            let end_len = board
                .trace_polyline(end_piece)
                .map(|lines| lines.lines.len().saturating_sub(2))
                .unwrap_or(0);
            let start_key = u64::from(start_piece.get());
            let end_key = u64::from(end_piece.get());
            let mut start_leaf_arr: Vec<Option<NodeIdx>> = vec![None; start_len];
            let mut end_leaf_arr: Vec<Option<NodeIdx>> = vec![None; end_len];
            // :323-328 — the start piece's head, re-labeled in place;
            // the transferred slots NULL in from_trace's array.
            for i in 0..start_len.saturating_sub(1) {
                start_leaf_arr[i] = from_entries.get(i).copied().flatten();
                if let Some(leaf) = start_leaf_arr[i] {
                    self.trees[tree_index].relabel_leaf(leaf, start_key, i as u32);
                }
                if let Some(by_tree) = self.entries.get_mut(&from_trace)
                    && let Some(slots) = by_tree.get_mut(&object_id)
                {
                    slots[i] = None;
                }
            }
            // :329-330 — the start piece's LAST entry: one fresh
            // leaf. The insert lazily fills the piece's shape cache
            // (Java `getTreeShape` → `calculateTreeShapes`).
            if start_len > 0 {
                let shapes =
                    board.tree_shape_precalc(start_piece, object_id, variant, compensated_class);
                if let Some(Some(shape)) = shapes.get(start_len - 1) {
                    start_leaf_arr[start_len - 1] = self.trees[tree_index].insert_one(
                        start_key,
                        (start_len - 1) as u32,
                        Some(shape),
                    );
                }
            }
            // :334-337 — the end piece's FIRST entry: one fresh leaf.
            if end_len > 0 {
                let shapes =
                    board.tree_shape_precalc(end_piece, object_id, variant, compensated_class);
                if let Some(Some(shape)) = shapes.first() {
                    end_leaf_arr[0] = self.trees[tree_index].insert_one(end_key, 0, Some(shape));
                }
            }
            // :339-345 — the end piece's tail, re-labeled in place.
            // Index arithmetic is the Java shape (the transferred-slot
            // window walks from the array's tail); not iterator-shaped.
            #[allow(clippy::needless_range_loop)]
            for i in 1..end_len {
                let from_index = from_entries.len().saturating_sub(end_len).saturating_add(i);
                end_leaf_arr[i] = from_entries.get(from_index).copied().flatten();
                if let Some(leaf) = end_leaf_arr[i] {
                    self.trees[tree_index].relabel_leaf(leaf, end_key, i as u32);
                }
                if let Some(by_tree) = self.entries.get_mut(&from_trace)
                    && let Some(slots) = by_tree.get_mut(&object_id)
                    && from_index < slots.len()
                {
                    slots[from_index] = None;
                }
            }
            // :347-349 — the pieces' entry lists.
            self.entries
                .entry(start_piece)
                .or_default()
                .insert(object_id, start_leaf_arr);
            self.entries
                .entry(end_piece)
                .or_default()
                .insert(object_id, end_leaf_arr);
        }
    }

    // -----------------------------------------------------------------
    // The overlap query family (M2 Task 8; Java
    // ShapeSearchTree.java:390-570 + BasicBoard.checkShape
    // :957-981). The candidate/geometry halves live in epic-index
    // ([`SearchTree::query_candidates`],
    // [`SearchTree::entry_intersects`],
    // [`SearchTree::clearance_candidates`],
    // [`SearchTree::clearance_test`]); the filters and the
    // stored-shape fetch — the parts that read ITEMS and RULES —
    // live here (D17).
    // -----------------------------------------------------------------

    /// The tree index of a [`NodeIdx`]/key row's item — object keys
    /// ARE item ids on this manager's trees (the broadcast insert
    /// keys leaves by `u64::from(id.get())`); a key outside the u32
    /// range belongs to no item (Java's cast `(Item)` would throw) —
    /// `None`, never a sentinel id that could alias a real item.
    pub(crate) fn item_of_key(key: u64) -> Option<ItemId> {
        u32::try_from(key).ok().map(ItemId::new)
    }

    /// The caller-side half of the ENTRY-form queries: resolves a
    /// [`TreeEntry`] `object_key` to its [`ItemId`] (`None` for a
    /// non-item object). Public for the epic-drc clearance walk — the
    /// one out-of-crate consumer of the entry form.
    #[must_use]
    pub fn item_of_entry_key(key: u64) -> Option<ItemId> {
        Self::item_of_key(key)
    }

    /// Slot 0 of `trees` is the DEFAULT tree (module invariant; pinned
    /// by `slot_zero_is_the_default_tree_through_every_manager_mutation`).
    /// [`Self::default_tree`] hands out the reference; query methods
    /// that take a `tree_index` use this constant for the same slot.
    /// Public: the harness corpora replay queries against it by name.
    pub const DEFAULT_TREE_INDEX: usize = 0;

    /// Java `overlappingTreeEntries(shape, layer, ignoreNetNos, list)`
    /// (`ShapeSearchTree.java:390-434`) — the plain query on tree
    /// `tree_index`: the candidates in `TreeSet<Leaf>` order
    /// (descending item id), each kept iff
    ///
    /// * `layer < 0` OR the stored shape's layer equals `layer`
    ///   (`currentObject.shapeLayer(shapeIndex)`),
    /// * the item `isObstacle` for EVERY ignore net — note the
    ///   inverted Java loop (`:412-417`): ONE non-obstacle net
    ///   ignores the whole item, and net 0 never ignores
    ///   ([`Board::item_is_obstacle`]),
    /// * the exact test: the T53 octagon-skip when both shapes are
    ///   octagons, else `stored.intersects(query)`
    ///   ([`SearchTree::entry_intersects`]).
    ///
    /// The result order is the SORTED candidate order (descending
    /// item id, shape index ASC) — Java appends into that order
    /// (`:404`, brief correction: not raw DFS order).
    pub fn overlapping_tree_entries(
        &self,
        board: &mut Board,
        tree_index: usize,
        shape: &TileShape,
        layer: i32,
        ignore_nets: &[i32],
    ) -> Vec<TreeEntry> {
        let tree = &self.trees[tree_index];
        let Some(candidates) = tree.query_candidates(shape, Self::descending_keys) else {
            // Java :401-403: a shape not bounded in the tree's
            // directions warns and returns nothing.
            return Vec::new();
        };
        let mut shape_cache: BTreeMap<ItemId, Vec<Option<TileShape>>> = BTreeMap::new();
        let mut result = Vec::new();
        for leaf in candidates {
            let Some(id) = Self::item_of_key(leaf.object_key) else {
                continue;
            };
            if Self::leaf_filtered(board, id, leaf.shape_index_in_object, layer, ignore_nets) {
                continue;
            }
            let stored = Self::stored_shape(
                board,
                &mut shape_cache,
                id,
                tree.object_id(),
                tree.variant,
                tree.compensated_clearance_class,
                leaf.shape_index_in_object,
            );
            let Some(stored) = stored else {
                // A null slot holds no leaf, so a surviving candidate
                // always resolves — unreachable, skipped defensively.
                continue;
            };
            if SearchTree::entry_intersects(shape, &stored) {
                result.push(TreeEntry {
                    object_key: leaf.object_key,
                    shape_index_in_object: leaf.shape_index_in_object,
                });
            }
        }
        result
    }

    /// Java `overlappingObjects` (`:355-374`) — the entry query's
    /// objects, deduped in `TreeSet<SearchTreeObject>` order
    /// (descending item id). The 2-arg Java form is
    /// `ignore_nets = &[]`.
    pub fn overlapping_objects(
        &self,
        board: &mut Board,
        tree_index: usize,
        shape: &TileShape,
        layer: i32,
        ignore_nets: &[i32],
    ) -> Vec<ItemId> {
        let mut objects: BTreeSet<Reverse<ItemId>> = BTreeSet::new();
        for entry in self.overlapping_tree_entries(board, tree_index, shape, layer, ignore_nets) {
            if let Some(id) = Self::item_of_key(entry.object_key) {
                objects.insert(Reverse(id));
            }
        }
        objects.into_iter().map(|Reverse(id)| id).collect()
    }

    /// Java `BasicBoard.pickItems(Point, int, ItemSelectionFilter)`
    /// (`BasicBoard.java:1087-1100`), the null-filter form: the
    /// items whose shape on `layer` contains `location` — the query
    /// shape is `TileShape.getInstance(point)` = the point's
    /// SURROUNDING BOX (`TileShape.java:61-63`), a zero-size box.
    /// The result is a Java `TreeSet<Item>` — DESCENDING id through
    /// `Item.compareTo` — which is the [`SearchTreeManager::overlapping_objects`]
    /// order verbatim (the SplitSpike capture `PICK_*` rows pin it).
    /// The filter argument is not ported (no caller in the ported
    /// surface needs it; Java applies it after the query).
    pub fn pick_items(&self, board: &mut Board, location: &Point, layer: i32) -> Vec<ItemId> {
        let point_shape =
            TileShape::RegularTileShape(RegularTileShape::IntBox(location.surrounding_box()));
        self.overlapping_objects(board, Self::DEFAULT_TREE_INDEX, &point_shape, layer, &[])
    }

    /// Java `overlappingTreeEntriesWithClearance` — the 5-arg CORE
    /// (`:443-507`), without the compensation dispatch: candidates
    /// against the query hull OFFSET by
    /// [`max_clearance_offset`](`max_clearance_offset`)`(
    /// maxValue(clearance_class, layer))`, the same layer/ignore-net
    /// filters, each surviving candidate's clearance
    /// `getValue(clearance_class, item_class, layer, add_margin)` —
    /// then the half-clearance sift ([`SearchTree::clearance_test`]).
    ///
    /// An unbounded query falls back to the board's bounding box
    /// (Java `:458-461`).
    pub fn overlapping_tree_entries_with_clearance_core(
        &mut self,
        board: &mut Board,
        tree_index: usize,
        shape: &TileShape,
        layer: i32,
        ignore_nets: &[i32],
        clearance_class: i32,
    ) -> Vec<TreeEntry> {
        let max_clearance =
            max_clearance_offset(board.rules().clearance.max_value(clearance_class, layer));
        let fallback = board.bounding_box().map(RegularTileShape::IntBox);
        let tree = &self.trees[tree_index];
        let candidates = tree.clearance_candidates(
            shape,
            max_clearance,
            fallback.as_ref(),
            Self::descending_keys,
        );
        let mut shape_cache: BTreeMap<ItemId, Vec<Option<TileShape>>> = BTreeMap::new();
        let mut rows: Vec<(LeafEntry, i32, TileShape)> = Vec::new();
        for leaf in candidates {
            let Some(id) = Self::item_of_key(leaf.object_key) else {
                continue;
            };
            if Self::leaf_filtered(board, id, leaf.shape_index_in_object, layer, ignore_nets) {
                continue;
            }
            let Some(item_class) = board.item_clearance_class(id) else {
                continue;
            };
            let clearance =
                board
                    .rules()
                    .clearance
                    .get_value_opt(clearance_class, item_class, layer, true);
            let stored = Self::stored_shape(
                board,
                &mut shape_cache,
                id,
                tree.object_id(),
                tree.variant,
                tree.compensated_clearance_class,
                leaf.shape_index_in_object,
            );
            let Some(stored) = stored else {
                continue;
            };
            rows.push((leaf, clearance, stored));
        }
        self.trees[tree_index].clearance_test(shape, &rows)
    }

    /// Java `overlappingTreeEntriesWithClearance` — the 4-arg
    /// DISPATCHING form (`:514-524`) — **T57**: when the TREE's
    /// compensation flag is set (`compensatedClearanceClassNo > 0`,
    /// `:95-97` — the tree property, not the manager flag) the
    /// compensation already lives in the stored shapes and the PLAIN
    /// query runs; otherwise the 5-arg core.
    pub fn overlapping_tree_entries_with_clearance(
        &mut self,
        board: &mut Board,
        tree_index: usize,
        shape: &TileShape,
        layer: i32,
        ignore_nets: &[i32],
        clearance_class: i32,
    ) -> Vec<TreeEntry> {
        if self.trees[tree_index].is_clearance_compensation_used() {
            self.overlapping_tree_entries(board, tree_index, shape, layer, ignore_nets)
        } else {
            self.overlapping_tree_entries_with_clearance_core(
                board,
                tree_index,
                shape,
                layer,
                ignore_nets,
                clearance_class,
            )
        }
    }

    /// Java `overlappingObjectsWithClearance` (`:530-549`) — the
    /// objects of the dispatching entry query, deduped descending.
    pub fn overlapping_objects_with_clearance(
        &mut self,
        board: &mut Board,
        tree_index: usize,
        shape: &TileShape,
        layer: i32,
        ignore_nets: &[i32],
        clearance_class: i32,
    ) -> Vec<ItemId> {
        let entries = self.overlapping_tree_entries_with_clearance(
            board,
            tree_index,
            shape,
            layer,
            ignore_nets,
            clearance_class,
        );
        let mut objects: BTreeSet<Reverse<ItemId>> = BTreeSet::new();
        for entry in entries {
            if let Some(id) = Self::item_of_key(entry.object_key) {
                objects.insert(Reverse(id));
            }
        }
        objects.into_iter().map(|Reverse(id)| id).collect()
    }

    /// Java `overlappingItemsWithClearance` (`:557-569`) — the
    /// instanceof-`Item` filter over the object query. Every object
    /// in these trees IS an item (the manager only inserts board
    /// items), so this is the objects form verbatim; kept as its own
    /// method for call-site parity with the Java surface.
    pub fn overlapping_items_with_clearance(
        &mut self,
        board: &mut Board,
        tree_index: usize,
        shape: &TileShape,
        layer: i32,
        ignore_nets: &[i32],
        clearance_class: i32,
    ) -> Vec<ItemId> {
        self.overlapping_objects_with_clearance(
            board,
            tree_index,
            shape,
            layer,
            ignore_nets,
            clearance_class,
        )
    }

    /// Java `BasicBoard.checkShape` (`BasicBoard.java:957-981`): can
    /// a shape with `net_numbers` and `clearance_class` go on
    /// `layer` without a clearance violation? Per convex piece:
    /// the board bounding-box containment, then the with-clearance
    /// object query on the DEFAULT tree, then Java's re-check that
    /// some obstacle stays an obstacle for EVERY net (`:972-977`).
    ///
    /// DIVERGENCE: a failed convex split (Java's null
    /// `splitToConvex`) would NPE at `tiles.length`; the port treats
    /// it as vacuously insertable — unreachable through parsed boards
    /// whose areas always split. Likewise, a `None` board bounding box
    /// (Java's `final IntBox boundingBox` field is only null before
    /// the reader sets it, and `isContainedIn(null)` would NPE) skips
    /// the containment check instead of rejecting — unreachable
    /// post-parse, where the DSN reader always assigns the box.
    pub fn check_shape(
        &mut self,
        board: &mut Board,
        shape: &Area,
        layer: i32,
        net_numbers: &[i32],
        clearance_class: i32,
    ) -> bool {
        let Some(tiles) = crate::tree_shapes::area_split_to_convex(shape) else {
            return true;
        };
        let bounding_box = board.bounding_box();
        for tile in &tiles {
            if bounding_box.is_some_and(|bounds| !tile_contained_in_box(tile, &bounds)) {
                return false;
            }
            let obstacles = self.overlapping_objects_with_clearance(
                board,
                Self::DEFAULT_TREE_INDEX,
                tile,
                layer,
                net_numbers,
                clearance_class,
            );
            for object in obstacles {
                let is_obstacle = net_numbers
                    .iter()
                    .all(|net| board.item_is_obstacle(object, *net));
                if is_obstacle {
                    return false;
                }
            }
        }
        true
    }

    /// Java `SearchTreeManager.validateEntries(Item)`
    /// (`SearchTreeManager.java:69-79`): AND over every live tree's
    /// check. An item with NO entries for a tree is SKIPPED for that
    /// tree — Java's `getSearchTreeEntries` null makes it NPE
    /// (capture `E_ABSENT id=384 → NullPointerException`); the port
    /// treats the absent list as vacuously true (documented
    /// divergence — the poisoned-slot case, which Java DOES check, is
    /// pinned below).
    pub fn validate_entries(&self, id: ItemId) -> bool {
        let mut result = true;
        for tree in &self.trees {
            let Some(entries) = self.tree_entries(id, tree.object_id()) else {
                continue;
            };
            if !tree.validate_entries(entries) {
                result = false;
            }
        }
        result
    }

    /// The layer/ignore-net filter shared by the plain and
    /// with-clearance cores (Java `:411-420` / `:477-486`).
    fn leaf_filtered(
        board: &mut Board,
        id: ItemId,
        shape_index: u32,
        layer: i32,
        ignore_nets: &[i32],
    ) -> bool {
        let index = i32::try_from(shape_index).unwrap_or(i32::MAX);
        if layer >= 0 && board.item_shape_layer(id, index) != Some(layer) {
            return true;
        }
        !ignore_nets
            .iter()
            .all(|net| board.item_is_obstacle(id, *net))
    }

    /// The stored tree shape of one candidate — through the same
    /// identity-keyed per-tree cache the insert filled
    /// (`Item.getTreeShape(tree, index)`), with a local per-call map
    /// so an item with several leaves fetches once.
    fn stored_shape(
        board: &mut Board,
        shape_cache: &mut BTreeMap<ItemId, Vec<Option<TileShape>>>,
        id: ItemId,
        tree_object_id: u64,
        variant: SearchTreeVariant,
        compensated_class: i32,
        shape_index: u32,
    ) -> Option<TileShape> {
        // Fill once per item per query, then BORROW: cloning the whole
        // Vec on every leaf hit would allocate per candidate (an item
        // with many shapes pays once per hit, not once per query).
        // `board` and `shape_cache` are disjoint borrows, so the
        // entry API composes with the `&mut board` precalc.
        let shapes = shape_cache.entry(id).or_insert_with(|| {
            board.tree_shape_precalc(id, tree_object_id, variant, compensated_class)
        });
        shapes.get(shape_index as usize).cloned().flatten()
    }

    /// The `TreeSet<Leaf>` object order: Java `Leaf.compareTo`
    /// (`ShapeTree.java:217-223`) delegates to the object's
    /// `compareTo` — `Item.compareTo` is descending id
    /// (`Item.java:95-103`) — so the board trees' candidates come
    /// out descending by item key (capture `A_LEAVES`).
    fn descending_keys(a: u64, b: u64) -> std::cmp::Ordering {
        b.cmp(&a)
    }

    /// The white-box poison seam for the validateEntries pin —
    /// swaps two entry slots of `id` in the tree with the given
    /// object id (the spike's `arr[0] ↔ arr[1]` swap; a Java test
    /// would mutate the package-private array directly).
    #[cfg(test)]
    pub(crate) fn swap_tree_entries_for_test(
        &mut self,
        id: ItemId,
        tree_object_id: u64,
        i: usize,
        j: usize,
    ) {
        if let Some(slots) = self
            .entries
            .get_mut(&id)
            .and_then(|by_tree| by_tree.get_mut(&tree_object_id))
            && i < slots.len()
            && j < slots.len()
        {
            slots.swap(i, j);
        }
    }
}

/// Java `ShapeSearchTree` `:462`: `maxClearance = (int)(1.2 *
/// clMatrix.maxValue(clearanceClassIndex, layer))` — the DOUBLE
/// product TRUNCATED toward zero. `1.2` in binary64 sits slightly
/// BELOW 1.2, but every MULTIPLE OF 5 rounds back up to the exact
/// integer (relative deficit 3.7e-17 < half an ulp), so the corpus
/// values (2000 → 2400, capture `B_FLAG`/`C_TIE_SETUP`) truncate
/// exactly; a NON-multiple discriminates: 14 → 16.8 → **16**
/// (capture `C_T56`; rounding would give 17).
///
/// Rust `as` saturates where Java's `(int)` wrap-casts at ±2^31 —
/// identical for every real clearance magnitude.
///
/// This is the query-reach side of the coupling the drivers lean on:
/// the shove probes' effective reach (shape plus this clearance
/// margin) decides which obstacles a spring-over/piece recursion can
/// even see — see SEAM "Query-reach model" (epic-router/SEAM.md) for
/// the two-way contract and the t11 edge that pins it.
#[must_use]
pub fn max_clearance_offset(max_value: i32) -> i32 {
    (1.2_f64 * f64::from(max_value)) as i32
}

/// `TileShape.isContainedIn(IntBox)` — the checkShape containment
/// (`BasicBoard.java:962`); a simplex is inside an axis box iff its
/// bounding box is (its extreme points ARE its vertices).
fn tile_contained_in_box(tile: &TileShape, r#box: &IntBox) -> bool {
    match tile {
        TileShape::RegularTileShape(regular) => regular.is_contained_in_box(r#box),
        TileShape::Simplex(_) => tile.bounding_box().is_contained_in(r#box),
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
    use crate::test_util::parse_board_from_path as parse_board;
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

    /// **NIT-2 defensive pin** (T6 quality review): Java `remove` is
    /// guarded by `isOnTheBoard()` (`SearchTreeManager.java:48-50`) —
    /// an item force-flagged OFF the board after a bulk insert is
    /// SKIPPED by the broadcast remove and keeps its leaves. Pinned so
    /// a later unguarded "fix" cannot silently change tree-contents
    /// parity.
    #[test]
    fn remove_skips_an_item_forced_off_the_board() {
        let mut board = parse_board(ISSUE575);
        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(&mut board);
        let leaves = manager.default_tree().leaf_count();
        let via = ItemId::new(815);
        board.set_on_the_board(via, false);
        manager.remove(&mut board, via);
        assert_eq!(
            manager.default_tree().leaf_count(),
            leaves,
            "the isOnTheBoard guard makes remove a no-op (Java :48-50)"
        );
    }

    /// **NIT-2 defensive pin** (T6 quality review): a DOUBLE insert
    /// leaks leaves — the second insert's `setSearchTreeEntries`
    /// (`ShapeTree.java:31-42`) OVERWRITES the stored entries, so the
    /// later remove takes out only the second set; the first insert's
    /// leaves remain. Java has the identical leak; pinned so a later
    /// "fix" cannot silently change it. Via 815 has copper on BOTH
    /// layers — two tree shapes per insert.
    #[test]
    fn double_insert_leaks_the_first_inserts_leaves() {
        let mut board = parse_board(ISSUE575);
        let mut manager = SearchTreeManager::new();
        let via = ItemId::new(815);
        manager.insert(&mut board, via);
        assert_eq!(manager.default_tree().leaf_count(), 2);
        manager.insert(&mut board, via);
        assert_eq!(manager.default_tree().leaf_count(), 4);
        manager.remove(&mut board, via);
        assert_eq!(
            manager.default_tree().leaf_count(),
            2,
            "only the second insert's entries are removed — the first set leaks"
        );
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
    /// LIVE-ONLY bulk fill (M3-T7: Java's `startReadObject` walk runs
    /// over `UndoableObjects.objects.values()`, and `delete` unlinks
    /// the node — a deleted item must not re-enter a fresh tree).
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
        // The flag pin needs an item that is NOT on the board: the
        // Rust representation keeps a flag-false ENTRY for a deleted
        // item where Java unlinks the node, so the walk filters
        // flag-false entries (the only post-parse meaning of the
        // flag: every parsed item was flipped true by
        // `reinsert_tree_items`). Via 815 is forced into that deleted
        // state through the raw flag seam, and the bulk fill must
        // NOT re-insert it.
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
        // The tree-entry drop is the DELETION contract now: the
        // flag-false item is skipped by the live-only walk (the
        // TREE's own insert still does not set the flag — that is
        // pinned by the entries of the live items above).
        assert!(!board.is_on_the_board(ItemId::new(815)));
        assert!(
            manager
                .tree_entries(ItemId::new(815), manager.trees()[1].object_id())
                .is_none(),
            "the live-only walk skips the deleted item"
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

    /// The empty-shape contract: a kind whose Java
    /// `calculateTreeShapes` returns `new TileShape[0]` (the
    /// COMPONENT outline, `ComponentOutline.java:135-137`) carries NO
    /// tree entries (Java's `shapeCount <= 0` early-out) — but the
    /// broadcast insert still flips its on-the-board flag (`:43`).
    /// Since Task 7 the OTHER non-drill kinds (trace, obstacle,
    /// outline) DO carry entries.
    #[test]
    fn empty_shape_kinds_carry_no_entries_but_still_get_the_flag() {
        let mut board = parse_board(ISSUE575);
        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(&mut board);
        // The component outline (first id 384): no shapes, no entries.
        let component_outline = ItemId::new(384);
        assert_eq!(
            board
                .get(component_outline)
                .expect("component outline 384")
                .board_item_type(),
            BoardItemType::ComponentOutline
        );
        assert!(
            manager
                .tree_entries(component_outline, manager.default_tree().object_id())
                .is_none(),
            "ComponentOutline: no shapes -> no entries"
        );
        assert!(
            board.is_on_the_board(component_outline),
            "the flag is set regardless of shape count"
        );
        // Task 7: the board outline (id 1) DOES carry entries now.
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
                .is_some(),
            "the outline's line keepout has entries since Task 7"
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

    /// Renders a tile in the spike's capture format (`oct[...]` /
    /// `box[...]`) so assertions quote `/tmp/epic-t7-shapes.out`
    /// verbatim.
    fn fmt_tile(shape: &epic_geometry::tile_shape::TileShape) -> String {
        let epic_geometry::tile_shape::TileShape::RegularTileShape(regular) = shape else {
            panic!("the outline shapes of Issue575 are regular, got {shape:?}");
        };
        epic_index::format_bounds(regular)
    }

    /// **T7 — `generateKeepoutOutside` re-inserts the AREA branch**
    /// (`BoardOutline.java:234-244`). The parse cached the outline's
    /// LINE shapes in the default tree (the read inserts every item,
    /// `BoardItemRepository.java:161`); the flip does remove+insert
    /// with NO `clearDerivedData` — and STILL the shapes come back as
    /// the keepout-AREA branch, because Java's remove nulls the whole
    /// `searchTreesInfo` (`Item.java:1078-1080`). The capture
    /// discriminates the branches by VALUE on this fixture (both are
    /// 8 shapes: lineCount 4 x 2 layers vs 4 convex ring pieces x 2
    /// layers; on Issue054 the counts also differ, 132 vs 136).
    #[test]
    fn keepout_outside_reinsert_computes_the_area_branch() {
        let mut board = parse_board(ISSUE575);
        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(&mut board);
        let outline = ItemId::new(1);
        let object_id = manager.default_tree().object_id();

        // The parse-time LINE shapes (capture: default-parse).
        let line_shapes =
            board.tree_shape_precalc(outline, object_id, SearchTreeVariant::Generic, 0);
        assert_eq!(line_shapes.len(), 8, "lineCount 4 x layerCount 2");
        assert_eq!(
            fmt_tile(line_shapes[0].as_ref().expect("shape 0")),
            "oct[1124900 -933100 1415100 -932900 2057859 2348141 191859 482141]",
            "the LINE branch top window (capture)"
        );
        assert_eq!(
            fmt_tile(line_shapes[7].as_ref().expect("shape 7")),
            "oct[1124900 -933100 1125100 -413900 1538859 2058141 191859 711141]",
            "the LINE branch left window (capture)"
        );

        manager.generate_keepout_outside(&mut board, outline, true);
        assert_eq!(
            board.outline_keepout_outside_generated(outline),
            Some(true),
            "the flag flipped"
        );

        // The SAME default tree identity now holds the AREA shapes
        // (capture: default-after-flip == fresh45-after-flip).
        let area_shapes =
            board.tree_shape_precalc(outline, object_id, SearchTreeVariant::Generic, 0);
        assert_eq!(area_shapes.len(), 8, "4 convex ring pieces x 2 layers");
        assert_eq!(
            fmt_tile(area_shapes[0].as_ref().expect("shape 0")),
            "oct[1124000 -934000 1415000 -933000 2057000 2349000 190000 482000]",
            "the AREA branch top piece (capture)"
        );
        assert_eq!(
            fmt_tile(area_shapes[1].as_ref().expect("shape 1")),
            "oct[1124000 -933000 1125000 -413000 1537000 2058000 191000 712000]",
            "the AREA branch left piece (capture)"
        );
        // The two branches differ by VALUE on this fixture — the form
        // where a port that reuses the stale line shapes fails.
        assert_ne!(
            fmt_tile(area_shapes[0].as_ref().expect("shape 0")),
            fmt_tile(line_shapes[0].as_ref().expect("shape 0"))
        );

        // Reference: a board whose flag was set BEFORE any insert —
        // the area shapes must be identical (same rules, same branch).
        let mut reference_board = parse_board(ISSUE575);
        reference_board.set_outline_keepout_outside(outline, true);
        let mut reference = SearchTreeManager::new();
        reference.insert_all_board_items(&mut reference_board);
        let reference_object_id = reference.default_tree().object_id();
        assert_eq!(
            reference_board.tree_shape_precalc(
                outline,
                reference_object_id,
                SearchTreeVariant::Generic,
                0
            ),
            area_shapes,
            "flip-after-insert == flag-before-insert (both area shapes)"
        );
        // The WHOLE default tree agrees with the flag-before-insert
        // reference: under the rebuild walk (DESCENDING id — see
        // [`SearchTreeManager::insert_all_board_items`]) both
        // histories insert the outline's area shapes LAST (lowest id
        // → last in the walk; the flip's re-insert likewise comes
        // after the fill), and the insertion-structured MinAreaTree
        // makes equal leaf-insertion sequences give equal skeletons.
        assert_eq!(
            reference.default_tree().min_area_tree().dump_lines(),
            manager.default_tree().min_area_tree().dump_lines(),
            "the whole default tree agrees with the flag-before-insert reference"
        );

        // The same-value call is the early-out (Java :235-237): the
        // entries survive untouched.
        let leaves = manager.default_tree().leaf_count();
        manager.generate_keepout_outside(&mut board, outline, true);
        assert_eq!(manager.default_tree().leaf_count(), leaves);
        assert_eq!(board.outline_keepout_outside_generated(outline), Some(true));
    }

    /// **T7 — remove drops the SHAPE cache** (`Item.java:1078-1080`,
    /// `searchTreesInfo = null`): an insert after a remove must see
    /// rule changes that happened in between. The discriminator makes
    /// the outline's compensation LAYER-SENSITIVE (`v(1, 0, 1) =
    /// 5000`): insert under the default rules, change the matrix,
    /// remove + insert the outline — a port whose remove keeps the
    /// shape cache reuses the comp-0 layer-1 LINE shapes and diverges
    /// from the reference computed under the changed rules.
    #[test]
    fn remove_clears_the_tree_shape_cache_between_out_and_in() {
        // The reference: shapes computed UNDER the changed rules.
        let mut reference_board = parse_board(ISSUE575);
        reference_board
            .rules_mut()
            .clearance
            .set_value(1, 0, 1, 5000);
        let mut reference = SearchTreeManager::new();
        reference.insert_all_board_items(&mut reference_board);
        let outline = ItemId::new(1);
        let reference_object_id = reference.default_tree().object_id();
        let expected = reference_board.tree_shape_precalc(
            outline,
            reference_object_id,
            SearchTreeVariant::Generic,
            0,
        );
        // The witness: layer 1 really is compensated differently now.
        assert_eq!(
            clearance_compensation_value(reference_board.rules(), 1, 0, 1),
            5000,
            "the discriminating compensation"
        );

        let mut board = parse_board(ISSUE575);
        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(&mut board);
        let object_id = manager.default_tree().object_id();
        // Sanity: the cached shapes are the DEFAULT-rules shapes (the
        // layer-1 windows differ from the expected list).
        let stale = board.tree_shape_precalc(outline, object_id, SearchTreeVariant::Generic, 0);
        assert_ne!(
            stale, expected,
            "the default-rules shapes must differ from the changed-rules reference"
        );

        board.rules_mut().clearance.set_value(1, 0, 1, 5000);
        manager.remove(&mut board, outline);
        manager.insert(&mut board, outline);
        assert_eq!(
            board.tree_shape_precalc(outline, object_id, SearchTreeVariant::Generic, 0),
            expected,
            "remove must drop the shape cache — the re-insert recomputes"
        );
    }

    // -----------------------------------------------------------------
    // Task 8 — the overlap query family
    // (/tmp/epic-t8-query.out, the QuerySpike capture)
    // -----------------------------------------------------------------

    /// Renders an entry list in the capture's `(id#idx,...)` form.
    fn entries_string(entries: &[TreeEntry]) -> String {
        let rows: Vec<String> = entries
            .iter()
            .map(|entry| format!("({}#{})", entry.object_key, entry.shape_index_in_object))
            .collect();
        format!("[{}]", rows.join(","))
    }

    /// The capture's via-815 query box (`A_QBOX`).
    fn via815_query() -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::from_corners(
            1_312_600, -728_275, 1_312_900, -727_975,
        )))
    }

    /// **Section A — the plain entry query** (Issue575, default tree):
    /// the candidate ORDER (descending item id, `TreeSet<Leaf>` —
    /// `A_LEAVES`), the layer filter (`A_ENTRIES layer=0/1`), the
    /// ignore-net filter including the net-0 quirk
    /// (`A_ENTRIES nets=[1]/[0]/[9999]`), and `overlappingObjects`
    /// (`A_OBJECTS`). A port returning DFS order, ascending ids, or
    /// treating net 0 as a real net fails distinct rows here.
    #[test]
    fn plain_entry_query_matches_the_capture() {
        let mut board = parse_board(ISSUE575);
        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(&mut board);
        let query = via815_query();

        assert_eq!(
            entries_string(&manager.overlapping_tree_entries(&mut board, 0, &query, -1, &[])),
            "[(815#0),(815#1),(787#0),(786#0),(785#0),(784#0),(775#0),(771#0)]",
            "A_ENTRIES box layer=-1 nets=[] — the sorted candidate order"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries(&mut board, 0, &query, 0, &[])),
            "[(815#0),(787#0),(785#0),(771#0)]",
            "A_ENTRIES box layer=0 nets=[]"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries(&mut board, 0, &query, 1, &[])),
            "[(815#1),(786#0),(784#0),(775#0)]",
            "A_ENTRIES box layer=1 nets=[] — the via's second shape and the layer-1 items"
        );
        // The ignore-net filter: net 1 ignores every candidate (they
        // all carry it); net 0 never ignores (the containsNet guard).
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries(&mut board, 0, &query, -1, &[1])),
            "[]",
            "A_ENTRIES box layer=-1 nets=[1]"
        );
        for foreign_net in [0, 9999] {
            assert_eq!(
                entries_string(&manager.overlapping_tree_entries(
                    &mut board,
                    0,
                    &query,
                    -1,
                    &[foreign_net]
                )),
                "[(815#0),(815#1),(787#0),(786#0),(785#0),(784#0),(775#0),(771#0)]",
                "A_ENTRIES box layer=-1 nets=[{foreign_net}] — net 0 is never contained"
            );
        }

        // overlappingObjects: deduped, descending id (TreeSet under
        // Item.compareTo).
        let objects: Vec<u32> = manager
            .overlapping_objects(&mut board, 0, &query, 0, &[])
            .iter()
            .map(|id| id.get())
            .collect();
        assert_eq!(objects, vec![815, 787, 785, 771], "A_OBJECTS box layer=0");
        let objects: Vec<u32> = manager
            .overlapping_objects(&mut board, 0, &query, -1, &[])
            .iter()
            .map(|id| id.get())
            .collect();
        assert_eq!(
            objects,
            vec![815, 787, 786, 785, 784, 775, 771],
            "A_OBJECTS box layer=-1 — the via's two entries collapse to one object"
        );
    }

    /// **Sections B + B2 — the T57 compensated dispatch** (Issue575,
    /// the same query): with the default class-0 tree the 4-arg form
    /// dispatches to the with-clearance CORE and agrees with it row
    /// for row (`B4_CC0_DISPATCH` == `B5_CC0_CORE`); on the autoroute
    /// class-1 tree — TREE flag true, MANAGER flag false (`B2_TREE`)
    /// — it dispatches PLAIN (`B2_4ARG_DISPATCH` == `B2_PLAIN`);
    /// after the flag flip rebuilds a class-1 default, the same 4-arg
    /// call dispatches to the PLAIN query (`B4_CC1_DISPATCH` ==
    /// `B_PLAIN_CC1`). On THIS query the plain and core result sets
    /// COINCIDE in both flag states (the capture shows all three
    /// forms equal), so these rows pin the flag VALUES and the result
    /// SETS, not the branch choice — the discriminating rows (plain
    /// != core) live in the section-G test
    /// `with_clearance_dispatch_discriminates_plain_from_core`.
    /// The clearance annotation row (`B5_CC0_CLEARANCES`, all `cls=1
    /// c=2016`) pins the margin read the core sorted by.
    #[test]
    fn with_clearance_dispatch_follows_the_tree_flag() {
        let mut board = parse_board(ISSUE575);
        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(&mut board);
        let query = via815_query();

        // The core's clearance inputs (capture B5_CC0_CLEARANCES):
        // class 1 items read getValue(1, 1, 0, add_margin) = 2016.
        assert_eq!(board.rules().clearance.get_value_opt(1, 1, 0, true), 2016);

        let expected = "[(815#0),(787#0),(785#0),(771#0)]";
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance(
                &mut board,
                0,
                &query,
                0,
                &[],
                1
            )),
            expected,
            "B4_CC0_DISPATCH — flag off: the with-clearance core"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance_core(
                &mut board,
                0,
                &query,
                0,
                &[],
                1
            )),
            expected,
            "B5_CC0_CORE"
        );
        // overlappingItemsWithClearance — the instanceof-Item filter
        // is the objects form verbatim on this manager; pinned on the
        // B query (T8 quality review MINOR: the method had no caller
        // and no pin).
        let items: Vec<u32> = manager
            .overlapping_items_with_clearance(&mut board, 0, &query, 0, &[], 1)
            .iter()
            .map(|id| id.get())
            .collect();
        assert_eq!(
            items,
            [815, 787, 785, 771],
            "the items form of the B4_CC0 query"
        );

        // B2 — the FLAG-SOURCE rows, before the flip (the flip would
        // rebuild the tree list after): the autoroute class-1 tree
        // carries a true TREE flag while the manager (default tree)
        // flag stays false. The dispatch must read the TREE's flag —
        // but all three query forms coincide on this query (capture
        // `B2_*`), so this block pins the flag values and the plain
        // SET; the branch discrimination is section G.
        let ar1 = manager.get_autoroute_tree(&mut board, 1);
        assert_ne!(ar1, 0);
        assert!(
            manager.trees[ar1].is_clearance_compensation_used(),
            "B2_TREE treeFlag — the class-1 tree's derived flag"
        );
        assert!(
            !manager.default_tree().is_clearance_compensation_used(),
            "B2_TREE managerFlag — still off"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance(
                &mut board,
                ar1,
                &query,
                0,
                &[],
                1
            )),
            expected,
            "B2_4ARG_DISPATCH — tree flag true, manager flag false"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries(&mut board, ar1, &query, 0, &[])),
            expected,
            "B2_PLAIN"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance_core(
                &mut board,
                ar1,
                &query,
                0,
                &[],
                1
            )),
            expected,
            "B2_CORE — coincides (capture); the discriminator is section G"
        );

        // The flip: a NEW class-1 default tree — the dispatch now
        // takes the plain branch and still agrees.
        assert!(!manager.default_tree().is_clearance_compensation_used());
        manager.set_clearance_compensation_used(&mut board, true);
        assert!(manager.default_tree().is_clearance_compensation_used());
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance(
                &mut board,
                0,
                &query,
                0,
                &[],
                1
            )),
            expected,
            "B4_CC1_DISPATCH — flag on: the plain query on the compensated tree"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries(&mut board, 0, &query, 0, &[])),
            expected,
            "B_PLAIN_CC1"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance_core(
                &mut board,
                0,
                &query,
                0,
                &[],
                1
            )),
            expected,
            "B5_CC1_CORE — coincides too (capture); the divergence witness is section G"
        );
    }

    /// **Section G — the DISCRIMINATING T57 dispatch rows** (crafted
    /// board, the sweep query riding the trace octagon's diagonal
    /// face; `/tmp/epic-t8-query.out`). On the section-B query plain
    /// and core coincide, so those rows cannot fail a WRONG dispatch;
    /// here the two branches genuinely diverge at the sweep flip
    /// points, on every tree state:
    ///
    /// * `G_CC0 d=90/95` (default class-0 tree, flag false): the
    ///   4-arg dispatch takes the CORE — 4ARG == CORE == `[(4#0)]`
    ///   != PLAIN == `[]`. An INVERTED condition or an always-plain
    ///   port returns `[]` and fails.
    /// * `G_AR1 d=95/100` (autoroute class-1 tree: TREE flag true,
    ///   MANAGER flag false): the dispatch takes the PLAIN branch —
    ///   4ARG == PLAIN == `[]` != CORE == `[(4#0)]`. A manager-flag
    ///   or always-core port returns `[(4#0)]` and fails.
    /// * `G_CC1 d=95/100` (default tree rebuilt class-1, both flags
    ///   true): the same plain dispatch, re-pinning the flag-on
    ///   default.
    #[test]
    fn with_clearance_dispatch_discriminates_plain_from_core() {
        let sweep = |d: i32| oct_of(40_000 + d, 40_001 + d, 40_100 + d, 40_101 + d);
        let hit = "[(4#0)]";
        let none = "[]";
        let mut board = parse_dsn_text(CRAFTED_DSN);
        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(&mut board);

        // G_T0 — default class-0 tree: treeFlag=false managerFlag=false.
        assert!(!manager.default_tree().is_clearance_compensation_used());
        // G_CC0 d=90: plain has already dropped out; the dispatch must
        // follow the CORE.
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance(
                &mut board,
                0,
                &sweep(90),
                0,
                &[],
                1
            )),
            hit,
            "G_CC0 d=90 4ARG — the core branch (flag off)"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries(&mut board, 0, &sweep(90), 0, &[])),
            none,
            "G_CC0 d=90 PLAIN — dropped out"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance_core(
                &mut board,
                0,
                &sweep(90),
                0,
                &[],
                1
            )),
            hit,
            "G_CC0 d=90 CORE — still reaches (contrast witness)"
        );
        // G_CC0 d=100: the core's own reach ends (the T56 truncation
        // flip, pinned in detail by the section-C test).
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance(
                &mut board,
                0,
                &sweep(100),
                0,
                &[],
                1
            )),
            none,
            "G_CC0 d=100 4ARG — the core reach ends"
        );

        // G_T1 — the autoroute class-1 tree: treeFlag=true,
        // managerFlag=false. The ONLY state separating the two flag
        // sources.
        let ar1 = manager.get_autoroute_tree(&mut board, 1);
        assert_ne!(ar1, 0);
        assert!(manager.trees[ar1].is_clearance_compensation_used());
        assert!(!manager.default_tree().is_clearance_compensation_used());
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance(
                &mut board,
                ar1,
                &sweep(95),
                0,
                &[],
                1
            )),
            none,
            "G_AR1 d=95 4ARG — PLAIN despite the manager flag being off"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries(&mut board, ar1, &sweep(95), 0, &[])),
            none,
            "G_AR1 d=95 PLAIN"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance_core(
                &mut board,
                ar1,
                &sweep(95),
                0,
                &[],
                1
            )),
            hit,
            "G_AR1 d=95 CORE — would hit; a manager-flag dispatch returns it"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance(
                &mut board,
                ar1,
                &sweep(100),
                0,
                &[],
                1
            )),
            none,
            "G_AR1 d=100 4ARG — same regime, more margin"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance_core(
                &mut board,
                ar1,
                &sweep(100),
                0,
                &[],
                1
            )),
            hit,
            "G_AR1 d=100 CORE"
        );
        // G_AR1 d=105: everything ends on the compensated trees.
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance(
                &mut board,
                ar1,
                &sweep(105),
                0,
                &[],
                1
            )),
            none,
            "G_AR1 d=105 4ARG"
        );

        // G_T2 — the flag flip rebuilds a class-1 DEFAULT tree; both
        // flags true, dispatch still plain.
        manager.set_clearance_compensation_used(&mut board, true);
        assert!(manager.default_tree().is_clearance_compensation_used());
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance(
                &mut board,
                0,
                &sweep(95),
                0,
                &[],
                1
            )),
            none,
            "G_CC1 d=95 4ARG — the plain branch on the rebuilt tree"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries(&mut board, 0, &sweep(95), 0, &[])),
            none,
            "G_CC1 d=95 PLAIN"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance_core(
                &mut board,
                0,
                &sweep(95),
                0,
                &[],
                1
            )),
            hit,
            "G_CC1 d=95 CORE — contrast witness on the default tree"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance(
                &mut board,
                0,
                &sweep(105),
                0,
                &[],
                1
            )),
            none,
            "G_CC1 d=105 4ARG"
        );
    }

    /// **Section E — validateEntries** (Issue575 via 815): aligned
    /// entries validate (`E_TRUE`), a slot swap poisons BOTH the
    /// tree-side and the manager-side checks (`E_POISONED`), the
    /// restore heals (`E_RESTORED`), and an item with NO entries
    /// (the component outline 384) is vacuously true — Java NPEs
    /// there (`E_ABSENT`), the port's documented divergence.
    #[test]
    fn validate_entries_poisons_and_restores() {
        let mut board = parse_board(ISSUE575);
        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(&mut board);
        let via = ItemId::new(815);
        let object_id = manager.default_tree().object_id();
        assert_eq!(
            manager
                .tree_entries(via, object_id)
                .map(<[Option<NodeIdx>]>::len),
            Some(2),
            "E_ARRAY len=2 — the via spans two layers"
        );
        assert!(manager.validate_entries(via), "E_TRUE");
        manager.swap_tree_entries_for_test(via, object_id, 0, 1);
        assert!(!manager.validate_entries(via), "E_POISONED");
        manager.swap_tree_entries_for_test(via, object_id, 0, 1);
        assert!(manager.validate_entries(via), "E_RESTORED");
        // The absent case: the component outline carries no tree
        // entries (ComponentOutline returns no shapes).
        let outline_item = ItemId::new(384);
        assert!(manager.tree_entries(outline_item, object_id).is_none());
        assert!(
            manager.validate_entries(outline_item),
            "E_ABSENT — Java NPEs; the port treats the absent list as vacuously true"
        );
    }

    /// The crafted board of sections C/D/F — the same DSN text the
    /// Java spike parsed (both readers must agree on it).
    const CRAFTED_DSN: &str = r#"(pcb t8-query.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 1)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary (rect pcb 0 0 100000 60000))
    (keepout (rect F.Cu 10000 10000 30000 20000))
    (keepout (rect F.Cu 70000 10000 90000 20000))
    (rule (width 250) (clearance 14))
  )
  (placement)
  (library)
  (network
    (net T8NET)
  )
  (wiring
    (wire (path F.Cu 250  10000 10000 40000 40000) (net T8NET))
  )
)
"#;

    /// Parses crafted DSN text through the epic-dsn reader.
    fn parse_dsn_text(dsn: &str) -> Board {
        let mut ses = SesBoard::new();
        match read_board(dsn.as_bytes(), &mut ses) {
            DsnReadResult::Success { warnings } => {
                assert!(warnings.is_empty(), "crafted DSN: {warnings:?}");
            }
            other => panic!("expected Success for the crafted DSN, got {other:?}"),
        }
        Board::from_ses_board(&ses)
    }

    /// The bounding octagon of a box (the spike's query form).
    fn oct_of(x0: i32, y0: i32, x1: i32, y1: i32) -> TileShape {
        let box_shape = TileShape::RegularTileShape(RegularTileShape::IntBox(
            IntBox::from_corners(x0, y0, x1, y1),
        ));
        TileShape::RegularTileShape(RegularTileShape::IntOctagon(
            box_shape.bounding_octagon().expect("a box is bounded"),
        ))
    }

    /// **T56 — the `(int)(1.2 * maxValue)` truncation** (capture
    /// section C): the sweep of octagon queries marching up the
    /// 45-degree trace's diagonal face flips from `[(4#0)]` to `[]`
    /// between d=99 and d=100 — exactly the TRUNCATED offset (16 →
    /// diagonal shift 23); a ROUNDED port (17 → 24) keeps the trace
    /// at d=100. The y base is shifted by 1 so the two regimes differ
    /// on this row (B = urx_trace − (x0+y0_query) even).
    #[test]
    fn clearance_offset_truncation_flips_between_two_regimes() {
        let mut board = parse_dsn_text(CRAFTED_DSN);
        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(&mut board);

        // The facts (C_FACTS / C_T56): 4 items, maxValue(1,0) = 14,
        // (int)(1.2*14) = 16 (a rounded port gives 17).
        assert_eq!(board.item_count(), 4);
        assert_eq!(board.rules().clearance.max_value(1, 0), 14);
        assert_eq!(max_clearance_offset(14), 16);
        assert_eq!(max_clearance_offset(2000), 2400, "C_T56_EXTRA");

        for (d, expected) in [(60, "[(4#0)]"), (99, "[(4#0)]"), (100, "[]"), (130, "[]")] {
            let query = oct_of(40_000 + d, 40_001 + d, 40_100 + d, 40_101 + d);
            assert_eq!(
                entries_string(&manager.overlapping_tree_entries_with_clearance_core(
                    &mut board,
                    0,
                    &query,
                    0,
                    &[],
                    1
                )),
                expected,
                "C_SWEEP d={d} — the truncation discriminator row is d=100"
            );
        }
    }

    /// **T56 arithmetic, checked value by value** (multiples of 5
    /// round back to the exact integer through binary64; the others
    /// truncate below the rounded value).
    #[test]
    fn max_clearance_offset_truncates_like_java() {
        assert_eq!(max_clearance_offset(0), 0);
        assert_eq!(max_clearance_offset(5), 6);
        assert_eq!(max_clearance_offset(14), 16, "16.7999... -> 16, not 17");
        assert_eq!(max_clearance_offset(15), 18);
        assert_eq!(
            max_clearance_offset(17),
            20,
            "20.4 -> 20 (the round agrees)"
        );
        assert_eq!(max_clearance_offset(2000), 2400, "capture C_T56_EXTRA");
        assert_eq!(max_clearance_offset(20_000), 24_000, "capture C_TIE_SETUP");
    }

    /// **The tie order and the ignore/layer filters on the crafted
    /// board** (capture `C_TIE*` and `C_IGNORE*`): with the matrix
    /// blown up to 20000 the mid-board query reaches BOTH keepouts
    /// and the outline — all class 1, equal clearances — and the
    /// result is the descending-id arrival order
    /// `[(3#0),(2#0),(1#0)]`, IDENTICALLY on the second call
    /// (`C_TIE_AGAIN`); layer 1 gives nothing. The on-trace query
    /// shows the ignore filter on a net-carrying item: the trace (net
    /// T8NET=1) drops under `nets=[1]`, stays under `nets=[0]`.
    #[test]
    fn tie_order_and_ignore_nets_match_the_capture() {
        let mut board = parse_dsn_text(CRAFTED_DSN);
        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(&mut board);
        // The spike's matrix edit AFTER the insert — the stored tree
        // shapes keep their parse-time geometry; only the query-side
        // windows and clearances change.
        board.rules_mut().clearance.set_value(1, 1, 0, 20_000);
        assert_eq!(
            board.rules().clearance.max_value(1, 0),
            20_000,
            "C_TIE_SETUP"
        );
        assert_eq!(max_clearance_offset(20_000), 24_000);

        let mid = oct_of(49_500, 14_000, 50_500, 16_000);
        let expected = "[(3#0),(2#0),(1#0)]";
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance_core(
                &mut board,
                0,
                &mid,
                0,
                &[],
                1
            )),
            expected,
            "C_TIE layer=0 nets=[] — equal clearances keep the descending-id arrival order"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance_core(
                &mut board,
                0,
                &mid,
                0,
                &[],
                1
            )),
            expected,
            "C_TIE_AGAIN — the tie order is stable across queries"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance_core(
                &mut board,
                0,
                &mid,
                1,
                &[],
                1
            )),
            "[]",
            "C_TIE layer=1 — the keepouts and the trace are layer 0"
        );

        let on_trace = oct_of(29_750, 29_750, 30_250, 30_250);
        assert_eq!(
            board.get(ItemId::new(4)).expect("the trace").nets,
            vec![1],
            "C_TRACE_NET n=1"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance_core(
                &mut board,
                0,
                &on_trace,
                0,
                &[],
                1
            )),
            "[(4#0),(2#0)]",
            "C_IGNORE nets=[] — the trace and the near keepout"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance_core(
                &mut board,
                0,
                &on_trace,
                0,
                &[1],
                1
            )),
            "[(2#0)]",
            "C_IGNORE nets=[1] — the net-1 trace is not an obstacle for its own net"
        );
        assert_eq!(
            entries_string(&manager.overlapping_tree_entries_with_clearance_core(
                &mut board,
                0,
                &on_trace,
                0,
                &[0],
                1
            )),
            "[(4#0),(2#0)]",
            "C_IGNORE nets=[0] — net 0 never ignores"
        );
    }

    /// **Section F — checkShape** (a FRESH parse — the section-C
    /// matrix edit would swallow the near/far cases): the bbox
    /// containment reject, the obstacle reject with its layer
    /// specificity, the ignore-net accept on the trace, and the
    /// free-space accept — the six `F_CASE` rows.
    #[test]
    fn check_shape_matches_the_capture() {
        let mut board = parse_dsn_text(CRAFTED_DSN);
        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(&mut board);
        let box_area = |x0: i32, y0: i32, x1: i32, y1: i32| Area {
            border: crate::items::BoardShape::Tile(TileShape::RegularTileShape(
                RegularTileShape::IntBox(IntBox::from_corners(x0, y0, x1, y1)),
            )),
            holes: Vec::new(),
        };
        assert_eq!(
            board.bounding_box(),
            Some(IntBox::from_corners(-1000, -1000, 101_000, 61_000)),
            "F_BBOX"
        );
        // Over the board edge: the containment reject.
        assert!(
            !manager.check_shape(
                &mut board,
                &box_area(99_000, 10_000, 102_000, 20_000),
                0,
                &[],
                1
            ),
            "F_CASE over-edge"
        );
        // Over the keepout: layer 0 rejects, layer 1 is free.
        assert!(
            !manager.check_shape(
                &mut board,
                &box_area(18_000, 14_000, 22_000, 16_000),
                0,
                &[],
                1
            ),
            "F_CASE over-keepout layer=0"
        );
        assert!(
            manager.check_shape(
                &mut board,
                &box_area(18_000, 14_000, 22_000, 16_000),
                1,
                &[],
                1
            ),
            "F_CASE over-keepout layer=1 — the keepout lives on F.Cu only"
        );
        // Free space.
        assert!(
            manager.check_shape(
                &mut board,
                &box_area(45_000, 30_000, 46_000, 31_000),
                0,
                &[],
                1
            ),
            "F_CASE free"
        );
        // Over the trace: a foreign net rejects, the trace's own net
        // is not an obstacle for itself.
        assert!(
            !manager.check_shape(
                &mut board,
                &box_area(30_000, 30_000, 30_500, 30_500),
                0,
                &[],
                1
            ),
            "F_CASE over-trace nets=[]"
        );
        assert!(
            manager.check_shape(
                &mut board,
                &box_area(30_000, 30_000, 30_500, 30_500),
                0,
                &[1],
                1
            ),
            "F_CASE over-trace nets=[1] — same-net overlap is allowed"
        );
    }
}
