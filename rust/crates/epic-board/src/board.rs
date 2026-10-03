//! The live board: the item arena, the id generator, and the
//! parse-IR conversion boundary.
//!
//! Java anchors: `board/facade/BoardItemRepository.java` (insert/remove
//! bookkeeping — the revision bumps at `:166` and `:198`, T69) and
//! `board/model/items/Item.java` (the item-common fields, the
//! constructor id allocation `:86-90`, the `compareTo` order `:95-103`).
//!
//! ## Enumeration contract (D25, T60)
//!
//! `Item.compareTo = item.id - id` and Java's item list is a
//! `ConcurrentSkipListMap` under natural order, so EVERY board
//! enumeration walks DESCENDING id. The arena is keyed
//! `Reverse<ItemId>`, which makes that the default iteration order;
//! every future epic-board enumeration MUST preserve it.
//!
//! ## Id and revision semantics (T61, T69)
//!
//! Insert does NOT allocate: ids are allocated at item CONSTRUCTION
//! (`Item.java:86-90`), so construct-then-discard burns an id and a
//! deleted item's id is never reused — builders call
//! [`Board::alloc_id`] first and put the id into the entry they insert.
//! `revision` bumps on EVERY successful insert AND remove
//! (`BoardItemRepository.java:166`, `:198`); the deletion-forbidden
//! early-out (`:189-191`) and the trace min/max width bookkeeping are
//! later M2 tasks (they need the rules surface).

use std::cmp::Reverse;
use std::collections::BTreeMap;

use epic_dsn::ses_board::{ItemIr, SesBoard};
use epic_dsn::shape::BoardShape as IrBoardShape;
use epic_dsn::sink::{AreaIr, FixedStateIr, KeepoutKindIr};
use epic_dsn::state::Unit as DsnUnit;
use epic_geometry::int_box::IntBox;
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;
use epic_geometry::tile_shape::TileShape;
use epic_index::SearchTreeVariant;

use crate::components::{BoardLibrary, Components, pin_center, pin_relative_location};
use crate::id::{ItemId, ItemIdGenerator};
use crate::items::drill::DrillPrecalc;
use crate::items::{Area, BoardItemType, BoardShape, FixedState, ItemData, ObstacleKind};
use crate::layers::LayerStructure;
use crate::rules_surf::BoardRules;
use crate::undo::UndoableObjects;

/// One arena entry: the Java `Item` base-class fields plus the
/// per-kind payload. `on_the_board` mirrors `Item.onTheBoard`
/// (`Item.java:63-64`) — false while constructed-but-not-inserted
/// (Java's burned-id state), true from [`Board::insert_item`] on.
///
/// Only Java's PERSISTENT item state belongs here (`Item.java:38-67`).
/// Derived state — `searchTreesInfo`, trace contacts,
/// `smallestClearance`, the drill precalculated trio — lives in
/// Board-side side tables keyed by [`ItemId`] (the `drill_precalc`
/// map), because undo snapshots clone `ItemEntry`s and Java's
/// `Item.clone` deliberately skips `searchTreesInfo` (`Item.java:263`:
/// the copy is commented out). The lazy caches are therefore
/// `&mut self` methods on [`Board`] itself (e.g.
/// [`Board::drill_min_width`]) — never a mutation path through the
/// entry.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemEntry {
    /// The item id (Java `Item.getId()`), allocated at construction.
    pub id: ItemId,
    /// The per-kind payload (the Java subclass fields).
    pub data: ItemData,
    /// Java `Item.netNumbers`.
    pub nets: Vec<i32>,
    /// Java `Item.clearanceClassIndex`.
    pub clearance_class: i32,
    /// Java `Item.componentId` (0 = belongs to no component).
    pub component_id: i32,
    /// Java `Item.fixedState`.
    pub fixed: FixedState,
    /// Java `Item.onTheBoard` — set true by [`Board::insert_item`].
    pub on_the_board: bool,
}

impl ItemEntry {
    /// Java `Item.getBoardItemType()` (`Item.java:117-146`).
    #[must_use]
    pub fn board_item_type(&self) -> BoardItemType {
        self.data.board_item_type()
    }
}

/// Java `board.communication` (`board/state/Communication.java`) —
/// the unit / resolution / host-CAD facts the parse carried. Only the
/// pieces the board model itself reads: the id generator lives in its
/// own field, the observers/host-version are GUI/API-side.
///
/// Defaults mirror Java's no-arg `Communication()` (MIL, resolution
/// 1, no host CAD); a parsed board overwrites all three from the DSN
/// `(parser ... (host_cad ...))` / `(resolution ...)` / `(unit ...)`
/// scopes.
#[derive(Clone, Debug, PartialEq)]
pub struct BoardCommunication {
    /// Java `Communication.unit`.
    pub unit: DsnUnit,
    /// Java `Communication.resolution` (in [`BoardCommunication::unit`]).
    pub resolution: i32,
    /// Java `specctraParserInfo.hostCad` (None where the parser scope
    /// carried no host — `hostCadExists()` is false then).
    pub host_cad: Option<String>,
}

impl Default for BoardCommunication {
    /// Java `Communication()` `:49-58`: `unit = Unit.MIL`,
    /// `resolution = 1`, no specctra parser info.
    fn default() -> Self {
        Self {
            unit: DsnUnit::Mil,
            resolution: 1,
            host_cad: None,
        }
    }
}

impl BoardCommunication {
    /// Java `Communication.getResolution(Unit.MIL)`
    /// (`Communication.java:94-97`): `Unit.scale(resolution, MIL,
    /// unit)` — the resolution CONVERTED to mils, the unit the
    /// search-tree threshold reads.
    #[must_use]
    pub fn resolution_mil(&self) -> f64 {
        DsnUnit::scale(f64::from(self.resolution), DsnUnit::Mil, self.unit)
    }

    /// Java `Communication.hostCadExists()`: true iff a host CAD was
    /// recorded.
    #[must_use]
    pub fn host_cad_exists(&self) -> bool {
        self.host_cad.is_some()
    }
}

/// Java `BasicBoard`'s trace half-width ACCUMULATORS — the insert-seam
/// pair (`maxTraceHalfWidth` `:107`, field-init 1000;
/// `minTraceHalfWidth` `:110`, field-init 10000). Distinct from the
/// parse-populated `BoardRules` fields of the same names
/// (`BoardRules.java:37-40`): the DSN parse fills the RULES pair and
/// never touches this one, and [`crate::trace_ops::
/// insert_trace_without_cleaning`] (`BasicBoard.java:196-200`, under
/// `netsNormal`) updates ONLY this one. Read back through the
/// `getMaxTraceHalfWidth`/`getMinTraceHalfWidth` getters
/// (`:1119-1126`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraceHalfWidthRange {
    /// Java `BasicBoard.maxTraceHalfWidth` (`:107`, init 1000).
    pub max_trace_half_width: i32,
    /// Java `BasicBoard.minTraceHalfWidth` (`:110`, init 10000).
    pub min_trace_half_width: i32,
}

impl Default for TraceHalfWidthRange {
    /// The Java field initializers — NOT zero.
    fn default() -> Self {
        Self {
            max_trace_half_width: 1000,
            min_trace_half_width: 10_000,
        }
    }
}

/// The live board model (Java `BasicBoard`'s item-list face plus the
/// rules / layers / components / library read surfaces, M2 Task 3).
#[derive(Clone, Debug, Default)]
pub struct Board {
    /// The items keyed by DESCENDING id (module docs, D25/T60).
    pub(crate) items: BTreeMap<Reverse<ItemId>, ItemEntry>,
    /// The id generator (Java `board.communication.idGenerator`).
    id_generator: ItemIdGenerator,
    /// Java `BasicBoard.revision` — bumped per insert and remove (T69).
    revision: u64,
    /// Java `board.communication` — the unit / resolution / host-CAD
    /// facts the parse carried (M2 Task 7: the search-tree shape
    /// threshold is resolution-dependent).
    communication: BoardCommunication,
    /// The per-item per-tree SHAPE cache (Java
    /// `Item.searchTreesInfo`'s `precalculatedTreeShapes`, identity-
    /// keyed per tree — M2 Task 7). Cleared per item by
    /// [`Board::clear_derived_data`], exactly Java
    /// `ItemSearchTreesInfo.clearPrecalculatedTreeShapes`.
    shape_precalc: BTreeMap<ItemId, BTreeMap<u64, Vec<Option<TileShape>>>>,
    /// Java `board.rules` (`BasicBoard.rules`, a `BoardRules`).
    rules: BoardRules,
    /// Java `BasicBoard`'s OWN `maxTraceHalfWidth`/`minTraceHalfWidth`
    /// accumulator pair (`:107`/:110, init 1000/10000) — the
    /// insert-seam target, distinct from [`BoardRules`]' same-named
    /// parse-populated fields (struct docs).
    pub(crate) trace_half_width_range: TraceHalfWidthRange,
    /// Java `board.layerStructure`.
    layers: LayerStructure,
    /// Java `board.components` (with its own undo stack, T63).
    pub(crate) components: Components,
    /// Java `board.library` — the padstack/package registries the pin
    /// resolution reads.
    library: BoardLibrary,
    /// Java `BasicBoard.boundingBox` (`:91`) — the outline bounding box
    /// enlarged by 1000 (`Structure.java:1207-1208`, T43); `None`
    /// before `create_board`. Consumed by the outline keepout-area
    /// derivation ([`crate::items::outline`]).
    bounding_box: Option<IntBox>,
    /// The `DrillItem` precalculated triple, memoized per pin/via id
    /// ([`crate::items::drill::DrillPrecalc`] — the Java field trio of
    /// `DrillItem.java:34-46` lives Board-side because the arena's
    /// `ItemEntry` is the PERSISTENT payload only; see the
    /// [`ItemEntry`] docs).
    ///
    /// Undo policy: the facade undo/redo restore path drops the
    /// restored id's entry here (Task 14) — behavior-neutral either
    /// way, because the memos are pure functions of persistent state:
    /// Java's SWAP restores bring fresh clones (`-1` sentinels
    /// recompute), while a delete-list restore keeps the SAME instance
    /// with the memo intact (`DrillItem.java:163-184`) — recompute
    /// equals keep.
    pub(crate) drill_precalc: BTreeMap<ItemId, DrillPrecalc>,
    /// Java `BasicBoard.itemList` — the `UndoableObjects` level stack
    /// (Task 14). The arena (`items`) stays the live storage; every
    /// entry change is MIRRORED into the undo node so a node value
    /// always equals the arena entry (Java shares the object, so its
    /// node is live by construction). Filled by [`Board::insert_item`]
    /// (Java `BoardItemRepository.insertItem` → `itemList.insert`),
    /// drained by [`Board::remove_item`] (→ `itemList.delete`), and
    /// replayed by the facade in [`crate::undo_facade`].
    pub(crate) item_undo: UndoableObjects<Reverse<ItemId>, ItemEntry>,
    /// Java `RoutingBoard.shoveFailingObstacle` (`RoutingBoard.java:72`)
    /// — the obstacle responsible for the last shove to fail (the
    /// MazeTraceShover failure report, T10b). Transient in Java; owned
    /// by the board here for the same reason Java puts it on
    /// RoutingBoard: the whole shove recursion writes it through the
    /// board handle.
    shove_failing_obstacle: Option<ItemId>,
    /// Java `RoutingBoard.shoveFailingLayer` (`:73`, field-init -1).
    /// `None` mirrors Java's negative/initial state (the accessor
    /// reports -1); Java never writes a negative layer after init.
    shove_failing_layer: Option<i32>,
    /// Java `RoutingBoard.changedArea` — the TRANSIENT marking session
    /// (`board/state/ChangedArea.java`, T10c). Null outside a marking
    /// session in Java; created lazily by `startMarkingChangedArea`
    /// (nested starts keep the session) and nulled by
    /// `optChangedArea`. Not part of undo (Java never snapshots it).
    pub(crate) changed_area: Option<crate::changed_area::ChangedArea>,
    /// Java `BasicBoard.normalizeSuppressedNetNos` (`:96`, `transient
    /// Set<Integer>`, field-initialized) — the per-net oscillation
    /// suppression of [`crate::normalize_all::normalize_traces_of_net`]:
    /// a net whose outer fixpoint hit the 2000-iteration cap is added
    /// here and every later call for it short-circuits false. Java
    /// never clears it on the live board (no reset site), it is unique
    /// to the PER-NET sibling (`normalizeAllTraces` has none), and T12's
    /// restore-by-copy semantics decide whether it crosses a restore —
    /// flagged for T12, ported verbatim here.
    pub(crate) normalize_suppressed_net_nos: std::collections::BTreeSet<i32>,
    /// Java `RoutingBoard.failureLog` (`RoutingBoard.java:64`, built at
    /// `:91`) — the per-item routing failure logbook
    /// ([`crate::failure_log::RoutingFailureLog`]). A non-transient
    /// field in Java, so board snapshots carry it and restores ROLL IT
    /// BACK; a plain field here gives the identical clone semantics.
    pub failure_log: crate::failure_log::RoutingFailureLog,
    /// Java `BasicBoard.preExistingClearanceViolationsCount`
    /// (`BasicBoard.java:101`, field-initialized 0) — the violation
    /// count measured on the FRESHLY LOADED board
    /// (`HeadlessBoardManager.java:789-793`); `BoardStatistics` splits
    /// `clearanceViolations.totalCount` into pre-existing vs
    /// router-introduced against it. A plain field gives the identical
    /// snapshot/restore semantics as Java's non-transient member.
    pub pre_existing_clearance_violations_count: i32,
    /// M7-T3 (beyond-Java — no Java counterpart): the tuning-regime
    /// flag. The pipeline resolves it once from
    /// `BatchSettings::tuning_active` (the input-driven activation — a
    /// net-class length declaration anywhere on the board, with
    /// `router.tuning` as the explicit override/kill-switch) and writes
    /// it here via [`Board::set_tuning_active`]; the honoring faces (the
    /// tightener's min-length gate) read it plus
    /// [`BoardRules::net_class_length_bounds`], never the CLI tri-state.
    /// `false` (the derived-Default) = the parity regime: every honoring
    /// gate is inert and all engine faces stay byte-identical. A plain
    /// field so board clones/restores carry the regime with them.
    pub tuning_active: bool,
    /// M11-T6 (upstream #931) — Java `BoardOutline.edgePinNets`, the
    /// outline's set of nets carried by an EDGE or OUTSIDE pin
    /// (`BoardOutline.getEdgePinNets`, `pre-t6` tree: a pin is edge
    /// when its center is outside the outline shapes OR any TILE pad
    /// corner on any layer is). Java keeps it as a `transient` LAZY
    /// cache on the outline item; the arena's read faces are
    /// `&self` ([`Board::item_is_trace_obstacle`]), so the port lifts
    /// it Board-side and EAGER, recomputed at exactly the seams Java
    /// invalidates: the parse tail
    /// ([`Board::from_ses_board`]), a pin net change
    /// ([`Board::set_item_nets`]), and the undo/redo side-effects tail
    /// ([`crate::undo_facade`]). Insert/remove only marks
    /// [`Self::edge_pin_nets_dirty`] (O(1)) — routing inserts no pins,
    /// so the set stays clean for the whole run and the parse stays
    /// linear.
    pub(crate) edge_pin_nets: std::collections::BTreeSet<i32>,
    /// The dirty half of the eager [`Self::edge_pin_nets`] cache:
    /// `true` after a pin/outline insert or remove that no recompute
    /// seam has covered yet. Readers are `&self` and cannot recompute;
    /// the outline arm of
    /// [`Board::item_is_trace_obstacle`] debug-asserts the cache clean
    /// so a future pin-mutating flow that misses a seam fails loudly
    /// in tests instead of answering stale.
    pub(crate) edge_pin_nets_dirty: bool,
}

impl Board {
    /// An empty board.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// M7-T3: the tuning-regime write face — the pipeline resolves the
    /// activation once (input-driven + the `router.tuning` override) and
    /// writes it here before any routing/optimization pass runs.
    pub fn set_tuning_active(&mut self, tuning_active: bool) {
        self.tuning_active = tuning_active;
    }

    /// M7-T3: the honoring faces' read face — `true` only when the
    /// pipeline resolved the tuning regime ON (the derived Default is
    /// `false`, so crafted worlds and any direct construction stay in
    /// the byte-identical parity regime).
    #[must_use]
    pub fn tuning_active(&self) -> bool {
        self.tuning_active
    }

    /// M7-T3: Java `Net.getTraceLength` (`rules/Net.java:130-140`) —
    /// the sum of every trace's `lengthApprox` (the port's
    /// `length_approx_total`) over the net's connectable items (the
    /// traces carrying the net). The meander-need report's length
    /// source and the [`BoardRules::length_violation`] predicate's
    /// caller-side input.
    #[must_use]
    pub fn net_trace_length(&self, net_number: i32) -> f64 {
        self.items
            .values()
            .filter(|entry| {
                matches!(entry.data, ItemData::Trace { .. }) && entry.nets.contains(&net_number)
            })
            .map(|entry| {
                self.trace_polyline(entry.id)
                    .map_or(0.0, epic_geometry::polyline::Polyline::length_approx_total)
            })
            .sum()
    }

    /// Allocates the next item id — the item-CONSTRUCTION counterpart
    /// of Java's `board.communication.idGenerator.newId()` call inside
    /// `Item`'s constructor (`Item.java:86-90`).
    /// # Burn semantics (T61)
    ///
    /// Allocation is NOT part of insertion: an id handed out here and
    /// never inserted (construct-then-discard — Java's closed-trace
    /// drop `BasicBoard.java:192-196`, split pieces, degenerate
    /// normalize removals) stays consumed, and deletion never frees
    /// ids. The generator is strictly monotone up to the wrap at
    /// [`crate::id::MAX_ID`].
    pub fn alloc_id(&mut self) -> ItemId {
        self.id_generator.new_id()
    }

    /// Java `ItemIdGenerator.maxGeneratedId()` — the watermark the
    /// Task 14 digest reads as `next_id` (the id-burn channel).
    #[must_use]
    pub fn max_generated_id(&self) -> u32 {
        self.id_generator.max_generated_id()
    }

    /// M6-T1b (storage-only cost slice): compacts the item-undo slab to
    /// its reachable graph — [`UndoableObjects::compact`] for the exact
    /// preservation contract. The optimizer calls this ONCE at stage
    /// entry so the per-candidate worker clones stop copying the
    /// routing history's unreachable residue; Java's `deepCopy` never
    /// carries that residue (its skip-list holds only current nodes),
    /// so the compacted board is the Java-copy-shaped state. No live
    /// read surface changes: `iter_visible`/`combine_traces` walk the
    /// unchanged map order.
    pub fn compact_item_undo_history(&mut self) {
        self.item_undo.compact();
    }

    /// Inserts a constructed entry. The id must have come from
    /// [`Board::alloc_id`] (or a parse IR) and must be free — Java's
    /// map would silently replace a duplicate key, which this port
    /// treats as the programming error it is. Bumps the revision (T69)
    /// and sets the entry's `on_the_board` flag (`Item.java:63-64`).
    /// Does NOT touch the generator: insertion consumes no id.
    ///
    /// Task 14: also inserts into the undo list (Java
    /// `BoardItemRepository.insertItem` → `itemList.insert(item)`,
    /// `BoardItemRepository.java:160` — `insert` disables redo and
    /// creates the node at the current level).
    pub fn insert_item(&mut self, mut entry: ItemEntry) {
        assert!(
            !self.items.contains_key(&Reverse(entry.id)),
            "insert_item: item id {} already on the board",
            entry.id.get()
        );
        // M11-T6 (#931): Java `BasicBoard.insertItem` invalidates the
        // outline's edge-pin cache for a pin or outline insert
        // (`BasicBoard.java:1261-1268`); the port marks dirty only —
        // `from_ses_board` recomputes once at its tail (a per-insert
        // recompute would make the parse quadratic in the pin count).
        let touches_edge_pin_nets = matches!(
            entry.data,
            ItemData::Pin { .. } | ItemData::BoardOutline { .. }
        );
        entry.on_the_board = true;
        let key = Reverse(entry.id);
        self.items.insert(key, entry.clone());
        self.item_undo.insert(key, entry);
        self.revision += 1;
        if touches_edge_pin_nets {
            self.edge_pin_nets_dirty = true;
        }
    }

    /// Removes the entry with the given id, returning it (Java holds
    /// the `Item` object; the id is the port's handle). Bumps the
    /// revision ONLY when an entry was actually removed
    /// (`BoardItemRepository.java:198` sits behind the successful-removal
    /// path), and clears the entry's `on_the_board` flag — Java does
    /// that flip in `SearchTreeManager.remove`
    /// (`SearchTreeManager.java:61`, `item.setOnTheBoard(false)` behind
    /// the `isOnTheBoard()` guard); until the tree manager exists this
    /// method owns the side effect. A no-op on a foreign id.
    ///
    /// Task 14: also deletes from the undo list (Java
    /// `BoardItemRepository.removeItem` → `itemList.delete(item)`,
    /// `BoardItemRepository.java:193-194` — tree remove, then the
    /// level-dichotomy delete).
    /// By the time this runs, the tree-manager half has already
    /// flipped the flag through [`Board::set_on_the_board`], so the
    /// node value carries `on_the_board == false` into the delete
    /// list exactly like Java's live object.
    pub fn remove_item(&mut self, id: ItemId) -> Option<ItemEntry> {
        let mut removed = self.items.remove(&Reverse(id));
        if let Some(entry) = removed.as_mut() {
            // M11-T6 (#931): Java `BasicBoard.removeItem` invalidates
            // the outline's edge-pin cache for a pin or outline remove
            // (`BasicBoard.java:606-613`); dirty-mark only (see
            // [`Board::insert_item`]).
            let touches_edge_pin_nets = matches!(
                entry.data,
                ItemData::Pin { .. } | ItemData::BoardOutline { .. }
            );
            entry.on_the_board = false;
            // The per-tree SHAPE cache must not outlive the arena slot
            // (T7 quality review NIT-4): in Java the item OBJECT goes
            // unreachable with its caches when the list drops it. Ids
            // are never reused, so this is pure hygiene against a
            // future delete flow — but the DRILL-SPAN memo stays: a
            // Java item re-inserted by undo keeps its precalculated
            // span (remove does not call clearDerivedData).
            self.shape_precalc.remove(&id);
            self.item_undo.delete(&Reverse(id));
            self.revision += 1;
            if touches_edge_pin_nets {
                self.edge_pin_nets_dirty = true;
            }
        }
        removed
    }

    /// The item with the given id, if present.
    #[must_use]
    pub fn get(&self, id: ItemId) -> Option<&ItemEntry> {
        self.items.get(&Reverse(id))
    }

    /// EVERY public enumeration walks DESCENDING id (D25/T60 — Java's
    /// `ConcurrentSkipListMap` under `Item.compareTo`).
    pub fn iter_descending(&self) -> impl Iterator<Item = &ItemEntry> {
        self.items.values()
    }

    /// The one ASCENDING-id walk: creation order. Java's READ path
    /// inserts each item into the trees at CREATION time (ascending id
    /// on parsed boards), unlike the rebuild walk
    /// ([`crate::tree_manager::SearchTreeManager::insert_all_board_items`],
    /// the `ConcurrentSkipListMap` iteration, which descends). The
    /// MinAreaTree skeleton depends on the walk order, so the two
    /// paths build different skeletons — see
    /// [`crate::tree_manager::SearchTreeManager::insert_items_creation_order`].
    pub fn iter_ascending(&self) -> impl DoubleEndedIterator<Item = &ItemEntry> {
        self.items.values().rev()
    }

    /// The number of live items.
    #[must_use]
    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    /// Java `BasicBoard.getConnectableItems(netNumber)`
    /// (`BasicBoard.java:609-611` → `BoardConnectivityQueries.java:23-37`):
    /// the connectable items carrying the net, DESCENDING-id ordered.
    /// ADDED at M3-T3 (the pure-SMD test in `AutorouteControl::rebuild_via_info`
    /// and `getUnconnectedSet` both consume it).
    #[must_use]
    pub fn get_connectable_items(&self, net_number: i32) -> Vec<ItemId> {
        crate::contacts::connectable_items(self, net_number)
    }

    /// The board revision — bumped on every insert and remove (T69).
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Java `board.rules`.
    #[must_use]
    pub fn rules(&self) -> &BoardRules {
        &self.rules
    }

    /// Java `BasicBoard.getMaxTraceHalfWidth()` (`:1119-1122`) — the
    /// BOARD accumulator (`TraceHalfWidthRange`), not
    /// [`BoardRules::max_trace_half_width`].
    #[must_use]
    pub fn max_trace_half_width(&self) -> i32 {
        self.trace_half_width_range.max_trace_half_width
    }

    /// Java `BasicBoard.getMinTraceHalfWidth()` (`:1124-1127`).
    #[must_use]
    pub fn min_trace_half_width(&self) -> i32 {
        self.trace_half_width_range.min_trace_half_width
    }

    /// Returns the minimum clearance requested between items of
    /// clearance class `class1` and `class2` on `layer` (Java
    /// `BasicBoard.clearanceValue`, `BasicBoard.java:1111-1118`): the
    /// matrix read WITH the safety margin (`getValue(..., true)`).
    /// Java's `rules == null || rules.clearanceMatrix == null` arm
    /// (returns 0) has no Rust counterpart — the port's board always
    /// carries rules with a matrix. The shove substrate consumes this
    /// for the two-step offset shapes and the via-contact diff.
    #[must_use]
    pub fn clearance_value(&self, class1: i32, class2: i32, layer: i32) -> i32 {
        self.rules
            .clearance
            .get_value_opt(class1, class2, layer, true)
    }

    /// Java `RoutingBoard.getShoveFailingObstacle` (`:1359-1361`) —
    /// the obstacle responsible for the last shove to fail.
    #[must_use]
    pub fn shove_failing_obstacle(&self) -> Option<ItemId> {
        self.shove_failing_obstacle
    }

    /// Java `RoutingBoard.setShoveFailingObstacle` (`:1363-1365`);
    /// `None` is Java's `null` (the ShapeTraceEntries found-obstacle
    /// is nullable on the failure paths that report it).
    pub fn set_shove_failing_obstacle(&mut self, item: Option<ItemId>) {
        self.shove_failing_obstacle = item;
    }

    /// The board-restore RESET face for the suppression set
    /// (`normalizeSuppressedNetNos` is `transient` in Java
    /// (`BasicBoard.java:96`) — a deserialized board comes back with
    /// the field-INITIALIZED empty set, so a restore clears every
    /// oscillation suppression). Resolves the T12 question flagged on
    /// the field docs: the set does NOT cross a restore.
    pub fn clear_normalize_suppressed_net_nos(&mut self) {
        self.normalize_suppressed_net_nos.clear();
    }

    /// The full post-deserialize TRANSIENT reset (T12's
    /// restore-by-copy model calls this after `*board =
    /// snapshot.clone()`): Java's `BasicBoard.readObject`
    /// (`:1386-1393`) resets `normalizeSuppressedNetNos` to a fresh
    /// set, and every `transient` RoutingBoard member comes back
    /// field-initialized (`RoutingBoard.java:69-75`: `changedArea`
    /// null, `shoveFailingObstacle` null, `shoveFailingLayer` -1).
    /// Deliberately NOT touched: `failureLog` (non-transient,
    /// `RoutingBoard.java:64` — snapshots carry it, restores ROLL IT
    /// BACK; the clone already did that), the id generator
    /// (non-transient inside `Communication` — the SNAPSHOT-TIME
    /// watermark, i.e. the rollback), and every persistent arena
    /// field.
    pub fn reset_transient_after_restore(&mut self) {
        self.normalize_suppressed_net_nos.clear();
        self.changed_area = None;
        self.shove_failing_obstacle = None;
        self.shove_failing_layer = None;
    }

    /// Java `RoutingBoardOperations.startMarkingChangedArea`
    /// (`:29-34`): opens the TRANSIENT marking session; a session
    /// already open is kept (the `changedArea == null` guard — nested
    /// starts never reset the accumulated box). Consumed by
    /// [`crate::routing_board_insert::opt_changed_area`].
    pub fn start_marking_changed_area(&mut self) {
        if self.changed_area.is_none() {
            self.changed_area = Some(crate::changed_area::ChangedArea::new(
                self.layers().layers.len(),
            ));
        }
    }

    /// Java `BasicBoard.getSmdPins()` (`:661-664` via
    /// `BoardItemRepository.getSmdPins` `:84-112`): the ON-BOARD pins
    /// whose (mirrored) layer span is single-layer —
    /// `firstLayer() == lastLayer()` — in `itemList` order (= ascending
    /// id; the engine.rs insertion-order convention), netless pins
    /// included exactly as in Java (the net-connected filter is
    /// BatchFanout's own, one block deeper).
    pub fn smd_pin_ids(&self) -> Vec<ItemId> {
        self.iter_ascending()
            .filter(|entry| entry.on_the_board)
            .filter(|entry| {
                let ItemData::Pin { padstack_no, .. } = &entry.data else {
                    return false;
                };
                let Some(component) = u32::try_from(entry.component_id)
                    .ok()
                    .and_then(|cid| self.components().get(cid))
                else {
                    return false;
                };
                let Some(padstack) = self.library().padstack(*padstack_no) else {
                    return false;
                };
                crate::components::pin_first_layer(component, padstack)
                    == crate::components::pin_last_layer(component, padstack)
            })
            .map(|entry| entry.id)
            .collect()
    }

    /// Java `BasicBoard.getSmdPins().size()` (`:661-664` via
    /// `BoardItemRepository.getSmdPins` `:94-102`): the ON-BOARD pins
    /// whose (mirrored) layer span is single-layer —
    /// `firstLayer() == lastLayer()`. The batch driver's fanout
    /// pre-pass rows read the count (`AutorouteBatchLoop.java:86-90`
    /// debug row, `:100-103` skip row); netless pins count here
    /// exactly as in Java (the net-connected filter is BatchFanout's
    /// own, one block deeper).
    pub fn smd_pin_count(&self) -> usize {
        self.smd_pin_ids().len()
    }

    /// The T12 board-hash walk over `itemList`'s reachable node graph
    /// (see [`UndoableObjects::digest_walk`]). The field is
    /// `pub(crate)`; this is the public read face the pipeline hash
    /// consumes (Java `serialize(true)` writes the whole
    /// `UndoableObjects` object — BoardSnapshotManager.java:33).
    pub fn undo_digest_walk(
        &self,
        visit: impl FnMut(crate::undo::UndoDigestNode<'_, Reverse<ItemId>, ItemEntry>),
    ) {
        self.item_undo.digest_walk(visit);
    }

    /// Java `UndoableObjects.stackLevel` — the hash walk's plain-field
    /// read (public wrapper over the `pub(crate)` field's getter).
    #[must_use]
    pub fn undo_stack_level(&self) -> usize {
        self.item_undo.stack_level()
    }

    /// Java `UndoableObjects.redoPossible` — ditto.
    #[must_use]
    pub fn undo_redo_possible(&self) -> bool {
        self.item_undo.redo_possible()
    }

    /// Java `RoutingBoard.getShoveFailingLayer` (`:1367-1369`) —
    /// -1 until a layer failure was reported.
    #[must_use]
    pub fn shove_failing_layer(&self) -> i32 {
        self.shove_failing_layer.unwrap_or(-1)
    }

    /// Java `RoutingBoard.setShoveFailingLayer` (`:1371-1373`).
    pub fn set_shove_failing_layer(&mut self, layer: i32) {
        self.shove_failing_layer = (layer >= 0).then_some(layer);
    }

    /// Java `RoutingBoard.clearShoveFailingObstacle` (`:1375-1379`) —
    /// obstacle null, layer -1.
    pub fn clear_shove_failing_obstacle(&mut self) {
        self.shove_failing_obstacle = None;
        self.shove_failing_layer = None;
    }

    /// The mutable seam onto `board.rules` — Java's `rules` is a
    /// mutable object the settings flows write through (e.g.
    /// `setHoleClearance` on the T55 drill-inflation path,
    /// `BoardRules.java:104-110`); the port's field is private, so the
    /// write side goes through here.
    pub fn rules_mut(&mut self) -> &mut BoardRules {
        &mut self.rules
    }

    /// Java `board.layerStructure`.
    #[must_use]
    pub fn layers(&self) -> &LayerStructure {
        &self.layers
    }

    /// Java `board.components`.
    #[must_use]
    pub fn components(&self) -> &Components {
        &self.components
    }

    /// Java `board.library`.
    #[must_use]
    pub fn library(&self) -> &BoardLibrary {
        &self.library
    }

    /// The mutable seam onto `board.library` — Java's `library` is a
    /// mutable object the board flows append to (`Padstacks.add` at
    /// via-insertion time, `RoutingBoard.insertVia`);
    /// [`crate::tree_shapes`] tests insert synthetic padstacks through
    /// it the same way the Task 6 jar spike did through the Java API.
    /// The production via-insertion path (padstack add + the
    /// `splitTraces` consumption of the new padstack) lives in
    /// [`crate::drill_item_mover::insert_via`].
    pub fn library_mut(&mut self) -> &mut BoardLibrary {
        &mut self.library
    }

    /// Java `board.boundingBox` (`BasicBoard.java:91`) — the outline
    /// bounds enlarged by 1000 (`Structure.java:1207-1208`, T43).
    /// `None` only before `create_board` (never after a successful
    /// parse: the IR always carries it). The outline keepout area is
    /// built over it ([`crate::items::outline::keepout_area`]).
    #[must_use]
    pub fn bounding_box(&self) -> Option<IntBox> {
        self.bounding_box
    }

    /// Java `DrillItem.firstLayer()` (`DrillItem.java:162-172`) for a
    /// pin or via item — [`crate::items::drill::via_first_layer`] for
    /// vias, [`crate::components::pin_first_layer`] for pins —
    /// memoized Board-side (Java's `precalculatedFirstLayer` sentinel
    /// `-1`), cleared only by [`Board::clear_derived_data`]. `None`
    /// for a non-drill item or an unresolvable pin.
    pub fn drill_first_layer(&mut self, id: ItemId) -> Option<i32> {
        if let Some(first_layer) = self
            .drill_precalc
            .get(&id)
            .and_then(|precalc| precalc.first_layer)
        {
            return Some(first_layer);
        }
        let first_layer = self.compute_drill_first_layer(id)?;
        self.drill_precalc.entry(id).or_default().first_layer = Some(first_layer);
        Some(first_layer)
    }

    /// Java `Item.firstLayer()` for every kind — the layer-interval
    /// lower bound backing [`crate::contacts::items_share_layer`]
    /// (`Item.sharesLayer`, `Item.java:313-318`). Per kind:
    /// Pin/Via take the memoized padstack span
    /// ([`Board::drill_first_layer`]), the flat kinds (Trace,
    /// ObstacleArea, ConductionArea, ComponentOutline) are
    /// `(layer, layer)`, and `BoardOutline` spans `0` to the layer
    /// count minus one (the `firstLayer()`/`lastLayer()` overrides
    /// hardcode that interval, `BoardOutline.java:99-108`). `None`
    /// for a missing id (Java would NPE; unreachable through the
    /// contacts seam).
    pub fn item_first_layer(&mut self, id: ItemId) -> Option<i32> {
        match &self.get(id)?.data {
            ItemData::Trace { layer, .. }
            | ItemData::ObstacleArea { layer, .. }
            | ItemData::ConductionArea { layer, .. }
            | ItemData::ComponentOutline { layer, .. } => Some(*layer),
            ItemData::Pin { .. } | ItemData::Via { .. } => self.drill_first_layer(id),
            ItemData::BoardOutline { .. } => Some(0),
            ItemData::Other => None,
        }
    }

    /// Java `Item.lastLayer()` — the interval upper bound of
    /// [`Board::item_first_layer`] (BoardOutline ends at the layer
    /// count minus one).
    pub fn item_last_layer(&mut self, id: ItemId) -> Option<i32> {
        match &self.get(id)?.data {
            ItemData::Trace { layer, .. }
            | ItemData::ObstacleArea { layer, .. }
            | ItemData::ConductionArea { layer, .. }
            | ItemData::ComponentOutline { layer, .. } => Some(*layer),
            ItemData::Pin { .. } | ItemData::Via { .. } => self.drill_last_layer(id),
            ItemData::BoardOutline { .. } => Some(self.layers().layers.len() as i32 - 1),
            ItemData::Other => None,
        }
    }

    /// Java `DrillItem.lastLayer()` (`DrillItem.java:174-185`) — the
    /// memoized counterpart of [`Board::drill_first_layer`].
    pub fn drill_last_layer(&mut self, id: ItemId) -> Option<i32> {
        if let Some(last_layer) = self
            .drill_precalc
            .get(&id)
            .and_then(|precalc| precalc.last_layer)
        {
            return Some(last_layer);
        }
        let last_layer = self.compute_drill_last_layer(id)?;
        self.drill_precalc.entry(id).or_default().last_layer = Some(last_layer);
        Some(last_layer)
    }

    /// Java `DrillItem.minWidth()` (`DrillItem.java:368-388`) for a
    /// pin or via item: the minimum bounding-box width AND height of
    /// the item's shapes over its signal layers
    /// ([`crate::items::drill::drill_min_width`], with the shapes
    /// fetched per layer through [`Board::pin_shape`] /
    /// [`crate::items::drill::via_shape`]). Memoized Board-side — and
    /// NOT cleared by [`Board::clear_derived_data`] (the Java quirk,
    /// module docs of [`crate::items::drill`]).
    pub fn drill_min_width(&mut self, id: ItemId) -> Option<f64> {
        if let Some(min_width) = self
            .drill_precalc
            .get(&id)
            .and_then(|precalc| precalc.min_width)
        {
            return Some(min_width);
        }
        let first_layer = self.drill_first_layer(id)?;
        let last_layer = self.drill_last_layer(id)?;
        let min_width = self.compute_drill_min_width(id, first_layer, last_layer)?;
        self.drill_precalc.entry(id).or_default().min_width = Some(min_width);
        Some(min_width)
    }

    /// Java `Item.clearDerivedData()` (`Item.java:1104-1109`) + the
    /// `DrillItem` override (`DrillItem.java:390-395`): drops the
    /// item's WHOLE per-tree shape cache
    /// ([`ItemSearchTreesInfo.clearPrecalculatedTreeShapes`]) and the
    /// drill LAYER SPAN. `precalculatedMinWidth` is never reset in
    /// Java (not by this, not by the geometry mutators that all route
    /// here), so a moved drill keeps its stale width; the port
    /// reproduces the asymmetry exactly (pinned white-box in the
    /// tests below — `drill_precalc_clear_derived_data_keeps_min_width`).
    /// The tree-manager side keeps LEAF entries in its own map (Java
    /// `clearSearchTreeEntries` is a separate, remove-side call).
    pub fn clear_derived_data(&mut self, id: ItemId) {
        self.shape_precalc.remove(&id);
        if let Some(precalc) = self.drill_precalc.get_mut(&id) {
            precalc.first_layer = None;
            precalc.last_layer = None;
        }
    }

    /// Mirrors the arena entry into the undo node — the value-clone
    /// counterpart of Java's SHARED-object node (Java's
    /// `UndoableObjectNode.object` IS the live item, so every field
    /// write is visible through the node by construction; the port's
    /// node is a snapshot, so every arena mutation writes through).
    /// `save_for_undo` stays at the JAVA call sites only — see
    /// [`Board::set_trace_polyline`].
    fn mirror_node(&mut self, id: ItemId) {
        if let Some(entry) = self.items.get(&Reverse(id)).cloned()
            && let Some(node) = self.item_undo.value_mut(&Reverse(id))
        {
            *node = entry;
        }
    }

    /// Java `Item.setOnTheBoard(boolean)` (`Item.java:248`): the raw
    /// flag write the search-tree manager performs after broadcasting
    /// an insert/remove (`SearchTreeManager.java:43` / `:61`). No
    /// `saveForUndo` on the Java path — the flag write rides on the
    /// live object; the node mirror keeps the port's snapshot equal.
    pub fn set_on_the_board(&mut self, id: ItemId, value: bool) {
        if let Some(entry) = self.items.get_mut(&Reverse(id)) {
            entry.on_the_board = value;
        }
        self.mirror_node(id);
    }

    /// Java `Item.setFixedState(FixedState)` (`Item.java:891-893`):
    /// the plain setter (no `saveForUndo` on the Java path).
    pub fn set_item_fixed(&mut self, id: ItemId, fixed: FixedState) {
        if let Some(entry) = self.items.get_mut(&Reverse(id)) {
            entry.fixed = fixed;
        }
        self.mirror_node(id);
    }

    /// Java `Item.isOnTheBoard()`.
    #[must_use]
    pub fn is_on_the_board(&self, id: ItemId) -> bool {
        self.items
            .get(&Reverse(id))
            .is_some_and(|entry| entry.on_the_board)
    }

    /// Java `Item.clearanceClassIndex()` — the tree-compensation input
    /// (`ShapeSearchTree.clearanceCompensationValue`'s first argument).
    #[must_use]
    pub fn item_clearance_class(&self, id: ItemId) -> Option<i32> {
        self.get(id).map(|entry| entry.clearance_class)
    }

    /// Java `Item.setClearanceClass(int)` — the plain field write
    /// (no `saveForUndo` on the Java path). Callers that change the
    /// class of on-board items must rebuild the search tree afterwards
    /// (compensated trees are keyed by class); the Java autoroute
    /// setup does this via `set_clearance_compensation_used`.
    pub fn set_item_clearance_class(&mut self, id: ItemId, class_no: i32) {
        if let Some(entry) = self.items.get_mut(&Reverse(id)) {
            entry.clearance_class = class_no;
        }
        self.mirror_node(id);
    }

    /// F1 (Rust-only, no Java counterpart — the pin auto-assignment
    /// face): replace an item's NET LIST wholesale. The plain field
    /// write + undo-mirror in the `set_item_*` family's shape. The
    /// search trees index GEOMETRY, not electrical grouping, so the
    /// caller does NOT need a tree remove/insert for a nets-only
    /// change — but [`crate::board::Board::clear_derived_data`] is
    /// still the conservative companion (the canonical mutation
    /// recipe, `apply_copper_to_edge_clearance_override`, pairs
    /// them). Nets are net NUMBERS into the append-only
    /// [`Nets`](crate::rules_surf::Nets) table — stable, never
    /// renumbered.
    pub fn set_item_nets(&mut self, id: ItemId, nets: Vec<i32>) {
        // M11-T6 (#931): Java `changeNet`'s assignment tail invalidates
        // the outline's edge-pin cache on a PIN net change
        // (`Item.java:1049-1053` — the edge-pin set carries the pin's
        // nets, so a re-netted pin changes it); the eager recompute is
        // the `&self`-reader equivalent of Java's lazy recompute.
        let mut pin_touched = false;
        if let Some(entry) = self.items.get_mut(&Reverse(id)) {
            pin_touched = matches!(entry.data, ItemData::Pin { .. });
            entry.nets = nets;
        }
        self.mirror_node(id);
        if pin_touched {
            self.recompute_edge_pin_nets();
        }
    }

    /// Java `DrillItem.getPadstack()` — the padstack of a pin or via
    /// (`None` for a non-drill item or an unresolvable padstack number).
    #[must_use]
    pub fn drill_padstack(&self, id: ItemId) -> Option<&crate::components::BoardPadstack> {
        let entry = self.get(id)?;
        let padstack_no = match &entry.data {
            ItemData::Via { padstack_no, .. } | ItemData::Pin { padstack_no, .. } => *padstack_no,
            _ => return None,
        };
        self.library.padstack(padstack_no)
    }

    /// Java `DrillItem.getCenter()` — the via's stored center or the
    /// pin's placement-resolved center (`None` for a non-drill item or
    /// an unresolvable pin).
    #[must_use]
    pub fn drill_center(&self, id: ItemId) -> Option<Point> {
        let entry = self.get(id)?;
        match &entry.data {
            ItemData::Via { center, .. } => Some(Point::Int(*center)),
            ItemData::Pin { pin_index, .. } => {
                let component_id = u32::try_from(entry.component_id).ok()?;
                pin_center(&self.components, &self.library, component_id, *pin_index)
            }
            _ => None,
        }
    }

    /// Java `DrillItem.getShape(index)` dispatched per kind: a via
    /// translates the raw padstack shape by its center
    /// ([`crate::items::drill::via_shape`]); a pin runs the full
    /// placement-resolution chain ([`crate::components::pin_shape`]).
    /// `None` = Java's null entry (a copper-less padstack layer).
    #[must_use]
    pub fn drill_shape(&self, id: ItemId, index: i32) -> Option<BoardShape> {
        let entry = self.get(id)?;
        match &entry.data {
            ItemData::Via {
                center,
                padstack_no,
                ..
            } => {
                crate::items::drill::via_shape(self.library.padstack(*padstack_no)?, *center, index)
            }
            ItemData::Pin { pin_index, .. } => {
                let component_id = u32::try_from(entry.component_id).ok()?;
                crate::components::pin_shape(
                    &self.components,
                    &self.library,
                    component_id,
                    *pin_index,
                    index,
                )
            }
            _ => None,
        }
    }

    /// Java `DrillItem.shapeLayer(index)` (`DrillItem.java:147-153`):
    /// `max(index, 0)` FIRST (`:148`), then `min(index, lastLayer -
    /// firstLayer)`, plus `firstLayer` — the clamp is against the ITEM
    /// span (memoized like Java's `firstLayer()`/`lastLayer()` calls;
    /// for a back-side pin this is NOT the padstack span). A negative
    /// index lands on the FIRST layer, never `first - 1` (T6 quality
    /// review MINOR-1: the port originally dropped the max clamp —
    /// unreachable from Task 6's call sites, but Tasks 7-8 reuse this
    /// on the shape-index path).
    pub fn drill_shape_layer(&mut self, id: ItemId, index: i32) -> Option<i32> {
        let first = self.drill_first_layer(id)?;
        let last = self.drill_last_layer(id)?;
        Some(first + index.max(0).min(last - first))
    }

    /// Java `DrillItem.tileShapeCount()` (`DrillItem.java:202-208`) —
    /// the PADSTACK span (`toLayer - fromLayer + 1`), deliberately not
    /// the item span (a back-side pin's span is mirrored).
    #[must_use]
    pub fn drill_tile_shape_count(&self, id: ItemId) -> Option<i32> {
        Some(crate::items::drill::drill_tile_shape_count(
            self.drill_padstack(id)?,
        ))
    }

    /// Crate-internal seam onto the drill-span memo — the white-box
    /// test channel for the [`Board::clear_derived_data`] staleness
    /// contract (Java's `precalculatedFirstLayer`/`LastLayer` are
    /// package-private fields the Java tests poison directly; no
    /// production caller may use this).
    #[cfg(test)]
    pub(crate) fn drill_precalc_mut(&mut self, id: ItemId) -> Option<&mut DrillPrecalc> {
        self.drill_precalc.get_mut(&id)
    }

    // -----------------------------------------------------------------
    // M2 Task 7: the per-kind shape inputs (trace / obstacle /
    // conduction / outline) and the per-tree SHAPE cache.
    // -----------------------------------------------------------------

    /// Java `board.communication` — the unit / resolution / host-CAD
    /// facts ([`BoardCommunication`]).
    #[must_use]
    pub fn communication(&self) -> &BoardCommunication {
        &self.communication
    }

    /// Java `PolylineTrace.polyline()` for a TRACE item — the stored
    /// corner polyline (`None` for a non-trace id).
    #[must_use]
    pub fn trace_polyline(&self, id: ItemId) -> Option<&Polyline> {
        let entry = self.get(id)?;
        let ItemData::Trace { lines, .. } = &entry.data else {
            return None;
        };
        Some(lines)
    }

    /// Java `Trace.get_layer()` for a TRACE item.
    #[must_use]
    pub fn trace_layer(&self, id: ItemId) -> Option<i32> {
        let entry = self.get(id)?;
        let ItemData::Trace { layer, .. } = &entry.data else {
            return None;
        };
        Some(*layer)
    }

    /// Java `Trace.get_half_width()` for a TRACE item.
    #[must_use]
    pub fn trace_half_width(&self, id: ItemId) -> Option<i32> {
        let entry = self.get(id)?;
        let ItemData::Trace { half_width, .. } = &entry.data else {
            return None;
        };
        Some(*half_width)
    }

    /// Java `PolylineTrace.setPolyline` (`PolylineTrace.java:127-130`
    /// — `this.lines = pPolyline` alone). The write half of
    /// [`Board::trace_polyline`]; the id is PRESERVED (Java mutates the
    /// object). The surrounding remove/clear/insert choreography is
    /// the adapter's — see [`crate::trace_ops::replace_geometry`]
    /// (`PolylineTraceSearchTreeAdapter.replaceGeometry`,
    /// `PolylineTraceSearchTreeAdapter.java:34-40`), the only caller
    /// in the ported surface. A non-trace id is left untouched (Java
    /// cannot even express the call — the receiver type guards it).
    pub fn set_trace_polyline(&mut self, id: ItemId, lines: Polyline) {
        // Java saves the trace at EVERY geometry-replacement site —
        // `PolylineTrace.replaceGeometry` (`:951`), `combine`'s two
        // branches (`:279` / `:403`), and the fast-cutout path
        // (`ShapeTraceEntries.java:113`) — and all of them funnel
        // through this mutator in the port. `saveForUndo` is
        // idempotent per level (`UndoableObjects.java:285-291`: a
        // no-op once `node.level == stackLevel`), so folding the save
        // HERE is behavior-identical.
        self.item_undo.save_for_undo(&Reverse(id));
        let Some(entry) = self.items.get_mut(&Reverse(id)) else {
            return;
        };
        if let ItemData::Trace {
            lines: stored_lines,
            ..
        } = &mut entry.data
        {
            *stored_lines = lines;
        }
        self.mirror_node(id);
    }

    /// M7-T4: the tuning MEANDER stage's geometry-write face — the
    /// tree-replacement core
    /// ([`crate::trace_ops::replace_geometry`], Java
    /// `PolylineTraceSearchTreeAdapter.replaceGeometry`) exposed to
    /// the pipeline: tree remove → set polyline → clear derived data →
    /// tree re-insert. The endpoints (the connection faces) are the
    /// caller's invariant — the meander wave preserves every original
    /// polyline corner outside the spliced window, so connectivity
    /// cannot change; no clearance CHECK fires here (the stage's
    /// candidate probe owns the acceptance face —
    /// `epic_router::pipeline::tuning`).
    pub fn replace_trace_geometry(
        &mut self,
        manager: &mut crate::tree_manager::SearchTreeManager,
        trace_id: ItemId,
        lines: Polyline,
    ) {
        crate::trace_ops::replace_geometry(manager, self, trace_id, lines);
    }
    /// Java `DrillItem.translateBy`'s center write (`DrillItem.java
    /// :55-65`): replaces the stored center of a pin/via and nothing
    /// else — `clear_derived_data` is the CALLER's job in Java
    /// (translateBy does both in sequence), and the undo save happens
    /// even earlier (`Item.moveBy` saves BEFORE the tree remove), so
    /// unlike [`Self::set_trace_polyline`] this setter carries NO
    /// `save_for_undo` and NO cache clearing of its own.
    pub fn set_via_center(&mut self, id: ItemId, center: IntPoint) {
        let Some(entry) = self.items.get_mut(&Reverse(id)) else {
            return;
        };
        if let ItemData::Via {
            center: stored_center,
            ..
        } = &mut entry.data
        {
            *stored_center = center;
        }
        self.mirror_node(id);
    }

    /// Java `ObstacleArea.getLayer()` for an OBSTACLE or CONDUCTION
    /// area item — the stored single layer both kinds carry (`None`
    /// for anything else, including component outlines whose layer
    /// has different semantics).
    #[must_use]
    pub fn area_layer(&self, id: ItemId) -> Option<i32> {
        let entry = self.get(id)?;
        match &entry.data {
            ItemData::ObstacleArea { layer, .. } | ItemData::ConductionArea { layer, .. } => {
                Some(*layer)
            }
            _ => None,
        }
    }

    /// Java `SearchTreeObject.shapeLayer(int)` — the layer of shape
    /// slot `index` of the item, per the kind overrides the query
    /// family's layer filter reads (`ShapeSearchTree.java:411`):
    ///
    /// * pins and vias: [`Board::drill_shape_layer`] (the span clamp,
    ///   `DrillItem.java:147-153` — memoized, hence `&mut self`),
    /// * traces: the trace's layer (`Trace.java:333-335`),
    /// * obstacle and conduction areas: the stored single layer
    ///   (`ObstacleArea.java:279-281`),
    /// * component outlines: the stored layer
    ///   (`ComponentOutline.java:125-127`),
    /// * board outlines: `index * layer_count / tileShapeCount()`
    ///   (`BoardOutline.java:70-82`) — INTEGER division over the
    ///   outline's OWN shape count (line branch: `lineCount x
    ///   layerCount`; keepout-area branch: convex pieces x
    ///   layerCount, 0 on a failed split).
    pub fn item_shape_layer(&mut self, id: ItemId, index: i32) -> Option<i32> {
        match self.get(id).map(|entry| &entry.data) {
            Some(ItemData::Pin { .. }) | Some(ItemData::Via { .. }) => {
                self.drill_shape_layer(id, index)
            }
            Some(ItemData::Trace { .. }) => self.trace_layer(id),
            Some(ItemData::ObstacleArea { .. }) | Some(ItemData::ConductionArea { .. }) => {
                self.area_layer(id)
            }
            Some(ItemData::ComponentOutline { layer, .. }) => Some(*layer),
            Some(ItemData::BoardOutline { .. }) => {
                let layer_count = self.layers().layers.len() as i32;
                let keepout_outside = self.outline_keepout_outside_generated(id)?;
                let shape_count = if keepout_outside {
                    let area = self.outline_keepout_area(id)?;
                    crate::tree_shapes::area_split_to_convex(&area)
                        .map_or(0, |pieces| pieces.len() as i32 * layer_count)
                } else {
                    let shapes = self.outline_shapes(id)?;
                    crate::items::outline::line_count(shapes) as i32 * layer_count
                };
                Some(if shape_count > 0 {
                    index * layer_count / shape_count
                } else {
                    0
                })
            }
            Some(ItemData::Other) | None => None,
        }
    }

    /// The [`Self::item_shape_layer`] read WITHOUT the drill-span memo
    /// fill: the `&self` sibling the warm-cache tree reads use (the
    /// autoroute engine's peek design — engine queries run with live
    /// `&self` borrows and must not mutate the memo caches). The drill
    /// branch substitutes the pure span halves
    /// ([`Self::compute_drill_first_layer`]
    /// /[`Self::compute_drill_last_layer`]) for the memoizing wrappers;
    /// every other branch is `&self` already, so the two faces agree
    /// whenever the memo is warm (the equality pin in the router's
    /// engine tests scans a parsed fixture end to end).
    pub fn item_shape_layer_read(&self, id: ItemId, index: i32) -> Option<i32> {
        match self.get(id).map(|entry| &entry.data) {
            Some(ItemData::Pin { .. }) | Some(ItemData::Via { .. }) => {
                let first = self.compute_drill_first_layer(id)?;
                let last = self.compute_drill_last_layer(id)?;
                Some(first + index.max(0).min(last - first))
            }
            Some(ItemData::Trace { .. }) => self.trace_layer(id),
            Some(ItemData::ObstacleArea { .. }) | Some(ItemData::ConductionArea { .. }) => {
                self.area_layer(id)
            }
            Some(ItemData::ComponentOutline { layer, .. }) => Some(*layer),
            Some(ItemData::BoardOutline { .. }) => {
                let layer_count = self.layers().layers.len() as i32;
                let keepout_outside = self.outline_keepout_outside_generated(id)?;
                let shape_count = if keepout_outside {
                    let area = self.outline_keepout_area(id)?;
                    crate::tree_shapes::area_split_to_convex(&area)
                        .map_or(0, |pieces| pieces.len() as i32 * layer_count)
                } else {
                    let shapes = self.outline_shapes(id)?;
                    crate::items::outline::line_count(shapes) as i32 * layer_count
                };
                Some(if shape_count > 0 {
                    index * layer_count / shape_count
                } else {
                    0
                })
            }
            Some(ItemData::Other) | None => None,
        }
    }

    /// Java `Item.isObstacle(int netNumber)` (`Item.java:162-164`) =
    /// `!containsNet(netNumber)`, with `containsNet`'s guard
    /// (`Item.java:149-152`): a net number `<= 0` NEVER counts as
    /// contained — so net 0 (and negatives) make EVERY item an
    /// obstacle and never ignore one (capture
    /// `A_ENTRIES ... nets=[0]` → the full list, like `nets=[9999]`).
    /// A foreign id has no nets to share — an obstacle, mirroring the
    /// net-less item.
    #[must_use]
    pub fn item_is_obstacle(&self, id: ItemId, net_number: i32) -> bool {
        if net_number <= 0 {
            return true;
        }
        match self.get(id) {
            Some(entry) => !entry.nets.contains(&net_number),
            None => true,
        }
    }

    /// Java `Item.isTraceObstacle(int)` (`Item.java:170-172`) — the
    /// VIRTUAL dispatch every maze-side consumer performs
    /// (`SearchTreeObject.isTraceObstacle`, `SearchTreeObject.java:12`;
    /// consumed at `SortedRoomNeighbours.java:223`,
    /// `Sorted45DegreeRoomNeighbours.java:123`,
    /// `SortedOrthogonalRoomNeighbours.java:154`,
    /// `CompleteFreeSpaceExpansionRoom.java:178`,
    /// `ShapeSearchTree.java:634`, `ShapeSearchTree45Degree.java:160`,
    /// `ShapeSearchTree90Degree.java:82`, and
    /// `BasicBoard.checkTraceShape:1019`). The base `Item` face is
    /// `!containsNet(netNumber)` (net `<= 0` never contained,
    /// `Item.java:149-152`), overridden by exactly three subclasses:
    ///
    /// * `ConductionArea` (`:398-400`):
    ///   `isObstacle && !containsNet(netNumber)` — every parse-time
    ///   plane/pour is inserted NON-obstacle (`Structure.java:1113`,
    ///   `:562-568`, `Wiring.java:485`), so foreign-net traces and vias
    ///   route THROUGH it (buglog 176: the Rust maze treated the
    ///   ecc83-pp_v2 GND plane as a hard blocker);
    /// * `ComponentObstacleArea` (`:71-73`) and `ViaObstacleArea`
    ///   (`:100-102`): unconditionally `false` — place keepouts and
    ///   via keepouts never block traces (a via keepout blocks DRILLS,
    ///   not traces).
    /// * `BoardOutline` (M11-T6, upstream #931,
    ///   `BoardOutline.java:138-143`): `false` for a POSITIVE net in
    ///   the outline's edge-pin set — an edge/outside pin's net may
    ///   route across the outline boundary (edge connectors,
    ///   castellated pads); everything else stays blocked.
    ///
    /// A foreign id keeps the net-less verdict `true` (`Item` with no
    /// shared net), matching [`Board::item_is_obstacle`] on unknown keys.
    #[must_use]
    pub fn item_is_trace_obstacle(&self, id: ItemId, net_number: i32) -> bool {
        match self.get(id).map(|entry| (&entry.data, &entry.nets)) {
            Some((ItemData::ConductionArea { is_obstacle, .. }, nets)) => {
                *is_obstacle && !nets.contains(&net_number)
            }
            Some((
                ItemData::ObstacleArea {
                    kind: ObstacleKind::ComponentObstacleArea | ObstacleKind::ViaObstacleArea,
                    ..
                },
                _,
            )) => false,
            Some((ItemData::BoardOutline { .. }, _)) => {
                debug_assert!(
                    !self.edge_pin_nets_dirty,
                    "edge-pin net cache read dirty — a pin/outline mutation missed its recompute seam"
                );
                !(net_number > 0 && self.edge_pin_nets.contains(&net_number))
            }
            Some((_, nets)) => !nets.contains(&net_number),
            None => true,
        }
    }

    /// Java `ConductionArea.getArea()` for a CONDUCTION item — the
    /// STORED area, verbatim: a parse-time conduction area always
    /// carries identity placement fields, so the inherited
    /// `ObstacleArea.getArea()` transform is the identity (the
    /// [`Board::obstacle_area`] docs pin the same split). `None` for
    /// a non-conduction id.
    #[must_use]
    pub fn conduction_area(&self, id: ItemId) -> Option<Area> {
        let entry = self.get(id)?;
        let ItemData::ConductionArea { area, .. } = &entry.data else {
            return None;
        };
        Some(area.clone())
    }

    /// Java `BoardOutline.getShape(i)` backing — the outline's shapes
    /// (`None` for a non-outline id).
    #[must_use]
    pub fn outline_shapes(&self, id: ItemId) -> Option<&[BoardShape]> {
        let entry = self.get(id)?;
        let ItemData::BoardOutline { shapes, .. } = &entry.data else {
            return None;
        };
        Some(shapes)
    }

    /// Java `BoardOutline.keepoutOutsideOutlineGenerated()`
    /// (`BoardOutline.java:226-228`) — `None` for a non-outline id.
    #[must_use]
    pub fn outline_keepout_outside_generated(&self, id: ItemId) -> Option<bool> {
        let entry = self.get(id)?;
        let ItemData::BoardOutline {
            keepout_outside_outline,
            ..
        } = &entry.data
        else {
            return None;
        };
        Some(*keepout_outside_outline)
    }

    /// The flag half of Java `BoardOutline.generateKeepoutOutside`
    /// (`BoardOutline.java:238`) — flips `keepoutOutsideOutline` with
    /// NO derived-data clear of its own. The remove/reinsert half lives
    /// with the tree manager
    /// ([`crate::tree_manager::SearchTreeManager::generate_keepout_outside`])
    /// and RE-COMPUTES the shapes: Java's `remove` nulls the item's
    /// whole `searchTreesInfo` (see [`Board::clear_search_tree_shapes`]),
    /// so the re-insert runs the AREA branch fresh (jar-verified on
    /// Issue575 and Issue054 — the post-flip shapes differ from the
    /// parse-time line shapes, `Item.java:1078-1080`).
    pub fn set_outline_keepout_outside(&mut self, id: ItemId, value: bool) {
        if let Some(ItemData::BoardOutline {
            keepout_outside_outline,
            ..
        }) = self
            .items
            .get_mut(&Reverse(id))
            .map(|entry| &mut entry.data)
        {
            *keepout_outside_outline = value;
        }
        self.mirror_node(id);
    }

    /// The shape half of Java `Item.clearSearchTreeEntries()`
    /// (`Item.java:1078-1080`): Java sets `searchTreesInfo = null`,
    /// dropping the container that holds BOTH the per-tree leaf
    /// entries AND the precalculated tree shapes — so the next shape
    /// query recomputes. The leaf-entry half lives manager-side
    /// ([`crate::tree_manager::SearchTreeManager::remove`], which calls
    /// this); unlike [`Board::clear_derived_data`] this is the REMOVE
    /// side's whole-container drop, not the mutator path's
    /// shapes-only clear.
    ///
    /// Divergence fixed in Task 7: the port first kept the shape cache
    /// alive across remove (a "staleness quirk" reading of
    /// `clearSearchTreeEntries` that cleared "only entries"); the jar
    /// capture disproved it — flipping `generateKeepoutOutside` on
    /// Issue054 re-inserted the outline with 136 AREA shapes where the
    /// parse had cached 132 line shapes
    /// (`/tmp/epic-t7-shapes.out`, the `KEEPOUT_OUTLINE` sequence).
    pub fn clear_search_tree_shapes(&mut self, id: ItemId) {
        self.shape_precalc.remove(&id);
    }

    /// Java `Item.getPrecalculatedTreeShapes(tree)` + the lazy
    /// `calculateTreeShapes` fill (`Item.java:228-238`): the item's
    /// tree shapes for the tree with the given OBJECT identity —
    /// served from the [`Board::shape_precalc`] cache, computed
    /// through [`crate::tree_shapes::item_tree_shapes`] on a miss.
    ///
    /// Keyed by tree identity (never by configuration): a re-created
    /// tree with the same variant and class is a CACHE MISS, exactly
    /// like Java's `tree == tree` reference walk through
    /// `ItemSearchTreesInfo`. Invalidated per item by
    /// [`Board::clear_derived_data`].
    pub fn tree_shape_precalc(
        &mut self,
        id: ItemId,
        tree_object_id: u64,
        variant: SearchTreeVariant,
        compensated_class: i32,
    ) -> Vec<Option<TileShape>> {
        if let Some(cached) = self
            .shape_precalc
            .get(&id)
            .and_then(|by_tree| by_tree.get(&tree_object_id))
        {
            return cached.clone();
        }
        let shapes = crate::tree_shapes::item_tree_shapes(self, variant, compensated_class, id);
        self.shape_precalc
            .entry(id)
            .or_default()
            .insert(tree_object_id, shapes.clone());
        shapes
    }

    /// The read-only face of [`Self::tree_shape_precalc`]: the cached
    /// shapes for `(id, tree)` WITHOUT computing on a miss. The
    /// autoroute engine's warm-cache invariant —
    /// `SearchTreeManager::get_autoroute_tree` bulk-inserts every live
    /// item through the caching path, and routing mutations flow
    /// through manager remove+insert which clears/refills it — makes a
    /// peek miss an invariant violation, so the peek answers `None`
    /// and the caller asserts with a distinctive message.
    pub fn tree_shape_precalc_peek(
        &self,
        id: ItemId,
        tree_object_id: u64,
    ) -> Option<&[Option<TileShape>]> {
        self.shape_precalc
            .get(&id)
            .and_then(|by_tree| by_tree.get(&tree_object_id))
            .map(|shapes| shapes.as_slice())
    }

    /// Java `Item.setPrecalculatedTreeShapes(shapes, tree)`
    /// (`Item.java:1066-1075`): REPLACES the cached tree shapes of
    /// `(id, tree)` — the merge/change fast paths build the new shape
    /// array (kept entries' old shapes + the fresh link/middle shapes)
    /// and install it BEFORE inserting the new leaves, because the
    /// insert reads the cache (`ShapeSearchTree.java:154-158`,
    /// :224-230, :298-304). Java guards `searchTreesInfo == null`
    /// with a warn + return; the port's insert-or-default reproduces
    /// the store (the merge callers always run on cached, on-board
    /// traces, so the null arm is unreachable there — Java only warns
    /// on a foreign tree, which the object-id keying makes a plain
    /// new slot).
    pub fn set_tree_shape_precalc(
        &mut self,
        id: ItemId,
        tree_object_id: u64,
        shapes: Vec<Option<TileShape>>,
    ) {
        self.shape_precalc
            .entry(id)
            .or_default()
            .insert(tree_object_id, shapes);
    }

    /// The fresh-computation half of [`Board::drill_first_layer`].
    fn compute_drill_first_layer(&self, id: ItemId) -> Option<i32> {
        let entry = self.get(id)?;
        match &entry.data {
            ItemData::Via { padstack_no, .. } => Some(crate::items::drill::via_first_layer(
                self.library.padstack(*padstack_no)?,
            )),
            ItemData::Pin { padstack_no, .. } => Some(crate::components::pin_first_layer(
                self.components
                    .get(u32::try_from(entry.component_id).ok()?)?,
                self.library.padstack(*padstack_no)?,
            )),
            _ => None,
        }
    }

    /// The fresh-computation half of [`Board::drill_last_layer`].
    fn compute_drill_last_layer(&self, id: ItemId) -> Option<i32> {
        let entry = self.get(id)?;
        match &entry.data {
            ItemData::Via { padstack_no, .. } => Some(crate::items::drill::via_last_layer(
                self.library.padstack(*padstack_no)?,
            )),
            ItemData::Pin { padstack_no, .. } => Some(crate::components::pin_last_layer(
                self.components
                    .get(u32::try_from(entry.component_id).ok()?)?,
                self.library.padstack(*padstack_no)?,
            )),
            _ => None,
        }
    }

    /// The fresh-computation half of [`Board::drill_min_width`] — the
    /// per-layer shape fetchers Java's `getShapeOnLayer` reaches
    /// (`DrillItem.java:263-271`: `getShape(layer - firstLayer)`).
    fn compute_drill_min_width(
        &self,
        id: ItemId,
        first_layer: i32,
        last_layer: i32,
    ) -> Option<f64> {
        let entry = self.get(id)?;
        match &entry.data {
            ItemData::Via {
                center,
                padstack_no,
                ..
            } => {
                let padstack = self.library.padstack(*padstack_no)?;
                let center = *center;
                Some(crate::items::drill::drill_min_width(
                    &self.layers,
                    first_layer,
                    last_layer,
                    |layer| crate::items::drill::via_shape(padstack, center, layer - first_layer),
                ))
            }
            ItemData::Pin { pin_index, .. } => {
                let component_id = u32::try_from(entry.component_id).ok()?;
                let pin_index = *pin_index;
                let components = &self.components;
                let library = &self.library;
                Some(crate::items::drill::drill_min_width(
                    &self.layers,
                    first_layer,
                    last_layer,
                    |layer| {
                        crate::components::pin_shape(
                            components,
                            library,
                            component_id,
                            pin_index,
                            layer - first_layer,
                        )
                    },
                ))
            }
            _ => None,
        }
    }

    /// Java `Via.getShape(index)` (`Via.java:115-132`) for a VIA item
    /// — the padstack shape at `index + firstLayer`, translated by the
    /// via center ([`crate::items::drill::via_shape`]). `None` for a
    /// non-via item.
    #[must_use]
    pub fn via_shape(&self, id: ItemId, index: i32) -> Option<crate::items::BoardShape> {
        let entry = self.get(id)?;
        let ItemData::Via {
            center,
            padstack_no,
            ..
        } = &entry.data
        else {
            return None;
        };
        crate::items::drill::via_shape(self.library.padstack(*padstack_no)?, *center, index)
    }

    /// Java `Pin.relativeLocation()` for a PIN ITEM on this board
    /// (resolves `component_id` + `pin_index` from the entry and
    /// delegates to [`crate::components::pin_relative_location`]).
    /// `None` for a non-pin item or an unresolvable pin.
    #[must_use]
    pub fn pin_relative_location(&self, id: ItemId) -> Option<epic_geometry::vector::Vector> {
        let entry = self.items.get(&Reverse(id))?;
        let ItemData::Pin { pin_index, .. } = entry.data else {
            return None;
        };
        let component_id = u32::try_from(entry.component_id).ok()?;
        pin_relative_location(&self.components, &self.library, component_id, pin_index)
    }

    /// Java `Pin.getCenter()` for a PIN ITEM on this board (`None` for
    /// a non-pin item or an unresolvable pin).
    #[must_use]
    pub fn pin_center(&self, id: ItemId) -> Option<Point> {
        let entry = self.items.get(&Reverse(id))?;
        let ItemData::Pin { pin_index, .. } = entry.data else {
            return None;
        };
        let component_id = u32::try_from(entry.component_id).ok()?;
        pin_center(&self.components, &self.library, component_id, pin_index)
    }

    /// Converts the parse-time IR into the live board (M2 plan:
    /// `SesBoard` is the one-shot parse model; this is the ONLY
    /// epic-dsn -> epic-board item path).
    ///
    /// - Every IR item is converted with its id EXACTLY preserved (no
    ///   renumbering; the IR's gaps are Java's burned ids, T61).
    /// - The id generator continues at the IR's position
    ///   (`SesBoard::last_assigned_item_id` mirrors
    ///   `ItemIdGenerator.maxGeneratedId()`), so the next
    ///   [`Board::alloc_id`] returns one past the parse.
    /// - The obstacle keepout kinds stay DISTINCT (T70): the IR's
    ///   `KeepoutKindIr` maps to the three Java `ObstacleArea`
    ///   subclasses exactly as the readers chose them
    ///   (`Structure.java:926-933`, `Network.java:1080-1149`).
    /// - Trace polylines are the IR's reader-constructed polylines
    ///   carried VERBATIM (T13): the reader builds them exactly like
    ///   Java's two read branches — `new Polyline(polygon)`
    ///   (`Wiring.java:531`) and `new Polyline(lines)` with the
    ///   parallel-line filter (`Wiring.java:562`) — so the live board
    ///   stores Java's parse-time polyline without re-derivation.
    ///   Re-deriving from the corner list would run the `Polygon` dedup,
    ///   which the `PolylinePath` branch never does: a duplicated
    ///   trailing corner (dsn-0061/0063, provably part of Java's board
    ///   state — the pre-T13 golden hash matched the verbatim corner
    ///   list) would be dropped. Corner rounding itself is the
    ///   DOCUMENTED M1b divergence (`scope/wiring.rs` module docs: Java
    ///   keeps exact rationals; the port rounds at parse), unchanged
    ///   here.
    ///
    /// # Consumer obligation (M4-T1, buglog 172)
    ///
    /// A board from this conversion is the PARSE surface, not the
    /// routing-ready one: Java's `DsnReader` fires
    /// `board.normalizeAllTraces()` at the wiring-scope tail
    /// (`Wiring.java:347`), so every consumer that routes or digests
    /// from a parsed board MUST run
    /// [`crate::normalize_all::normalize_all_traces`] after building
    /// its `SearchTreeManager`. The seven current consumers:
    /// `epic-cli/src/route.rs` (the routing path and, through it, the
    /// determinism-digest path), `epic-router/src/drill/pins.rs`, and
    /// the harness worlds `dsn_corpus.rs`, `undo_corpus.rs`,
    /// `ses_compare.rs`, `router_compare.rs`, `route_events.rs`
    /// (`parse_world_board`). Cautionary case: the events world skipped
    /// it until M4-T1 and diverged from Java at its first expansion
    /// door — Java's post-parse board had merged t7's collinear wires
    /// (19 items, no id 10) while Rust carried their phantom id 10 into
    /// the golden door labels (buglog 172).
    #[must_use]
    pub fn from_ses_board(ses: &SesBoard) -> Self {
        let mut board = Board::new();
        board
            .id_generator
            .set_next(u32::try_from(ses.last_assigned_item_id()).expect("IR ids are non-negative"));
        for item in &ses.items {
            if let Some(entry) = convert_item(item) {
                board.insert_item(entry);
            }
        }
        // The read surfaces (M2 Task 3). `layers` is None only before
        // `create_board`; the conversion of a finalized board always
        // has it (defensive: an empty structure instead of a panic).
        board.rules = BoardRules::from_ir(
            &ses.rules,
            &ses.nets,
            &ses.net_classes,
            &ses.via_infos,
            &ses.via_rules,
        );
        board.layers = match ses.layers.as_ref() {
            Some(layers) => LayerStructure::from_ir(layers),
            None => LayerStructure::default(),
        };
        board.components = Components::from_ir(
            &ses.components,
            ses.metadata.flip_style.as_deref() == Some("rotate_first"),
        );
        board.library = BoardLibrary::from_ir(&ses.padstacks, &ses.packages);
        board.communication = BoardCommunication {
            unit: ses.metadata.unit,
            resolution: ses.metadata.resolution,
            host_cad: ses.metadata.host_cad.clone(),
        };
        // The outline-derived bounding box (T43) — set by create_board
        // before any item is inserted; None only on a board without a
        // boundary (Java leaves the field null there).
        board.bounding_box = ses.bounding_box;
        // M11-T6 (#931): the edge-pin net cache's first fill — Java's
        // lazy field computes on first read after parse; the eager
        // port computes once here (every insert_item above only
        // dirtied it).
        board.recompute_edge_pin_nets();
        board
    }
}

/// Converts one IR item. `None` skips the item (defensively) — today
/// only the never-stored null-area component outline can hit it.
fn convert_item(item: &ItemIr) -> Option<ItemEntry> {
    let id = ItemId::new(u32::try_from(item.id()).expect("IR ids are positive"));
    let entry = match item {
        ItemIr::Trace { trace, .. } => ItemEntry {
            id,
            data: ItemData::Trace {
                layer: trace.layer_no,
                half_width: trace.half_width,
                // T13: the reader's Java-constructed polyline, carried
                // through the IR VERBATIM (the `TraceIr::polyline` field
                // docs). Re-deriving from the corner list would apply the
                // `Polygon` dedup, which Java's `PolylinePath` read branch
                // never does — duplicate trailing corners (dsn-0061/0063)
                // are part of Java's parse-time board state.
                lines: trace.polyline.clone(),
            },
            nets: trace.nets.clone(),
            clearance_class: trace.clearance_class,
            component_id: 0,
            fixed: fixed_from_ir(trace.fixed),
            on_the_board: false,
        },
        ItemIr::Via { via, .. } => ItemEntry {
            id,
            data: ItemData::Via {
                center: via.location,
                padstack_no: via.padstack_no,
                attach_smd_allowed: via.attach_smd_allowed,
            },
            nets: via.nets.clone(),
            clearance_class: via.clearance_class,
            component_id: 0,
            fixed: fixed_from_ir(via.fixed),
            on_the_board: false,
        },
        ItemIr::Pin { pin, .. } => ItemEntry {
            id,
            data: ItemData::Pin {
                pin_index: pin.pin_index,
                padstack_no: pin.padstack_no,
            },
            nets: pin.nets.clone(),
            clearance_class: pin.clearance_class,
            component_id: pin.component_id,
            fixed: fixed_from_ir(pin.fixed),
            on_the_board: false,
        },
        ItemIr::Keepout { keepout, .. } => ItemEntry {
            id,
            data: ItemData::ObstacleArea {
                kind: obstacle_kind_from_ir(keepout.kind),
                layer: keepout.layer_no,
                area: area_from_ir(&keepout.area),
                translation: keepout.translation,
                rotation: keepout.rotation,
                side_changed: keepout.side_changed,
                name: keepout.name.clone(),
            },
            // Parse-time keepouts never carry nets (the KeepoutIr docs).
            nets: Vec::new(),
            clearance_class: keepout.clearance_class,
            component_id: keepout.component_id,
            fixed: fixed_from_ir(keepout.fixed),
            on_the_board: false,
        },
        ItemIr::ConductionArea { area, .. } => ItemEntry {
            id,
            data: ItemData::ConductionArea {
                layer: area.layer_no,
                area: area_from_ir(&area.area),
                // Every parse-time insert passes isObstacle=false
                // (Structure.java:1108, :562; Wiring.java:485) and the
                // ctor default isFilled=true (ConductionArea.java:30).
                is_obstacle: false,
                is_filled: true,
            },
            nets: area.nets.clone(),
            clearance_class: area.clearance_class,
            component_id: 0,
            fixed: fixed_from_ir(area.fixed),
            on_the_board: false,
        },
        ItemIr::ComponentOutline { outline, .. } => {
            // The IR's sink never stores a None-area component outline
            // (`insert_component_outline` mirrors the
            // `BasicBoard.insertComponentOutline` null guard: no item,
            // no id). The skip keeps the conversion total if that ever
            // changes.
            let area = outline.area.as_ref()?;
            ItemEntry {
                id,
                data: ItemData::ComponentOutline {
                    layer: outline.layer_no,
                    area: area_from_ir(area),
                    translation: outline.translation,
                    rotation: outline.rotation,
                    is_front: outline.is_front,
                    is_courtyard: outline.is_courtyard,
                    is_fabrication: outline.is_fabrication,
                    is_closed: outline.is_closed,
                },
                nets: outline.nets.clone(),
                clearance_class: outline.clearance_class,
                component_id: outline.component_id,
                fixed: fixed_from_ir(outline.fixed),
                on_the_board: false,
            }
        }
        ItemIr::BoardOutline { outline, .. } => ItemEntry {
            id,
            data: ItemData::BoardOutline {
                shapes: outline.shapes.iter().map(board_shape_from_ir).collect(),
                // Java `keepoutOutsideOutline` starts false
                // (`generateKeepoutOutside` is a GUI/router action,
                // BoardOutline.java:234-244).
                keepout_outside_outline: false,
            },
            // Java constructs the outline with `new int[0]`
            // (`BoardOutline.java:47`).
            nets: Vec::new(),
            clearance_class: outline.clearance_class,
            component_id: 0,
            fixed: fixed_from_ir(outline.fixed),
            on_the_board: false,
        },
    };
    Some(entry)
}

/// Java `FixedState` ordinal mirror from the IR (both enums share the
/// declaration order; the IR docs pin the order as load-bearing).
fn fixed_from_ir(fixed: FixedStateIr) -> FixedState {
    match fixed {
        FixedStateIr::Unfixed => FixedState::Unfixed,
        FixedStateIr::ShoveFixed => FixedState::ShoveFixed,
        FixedStateIr::UserFixed => FixedState::UserFixed,
        FixedStateIr::SystemFixed => FixedState::SystemFixed,
    }
}

/// The readers' keepout-kind -> `ObstacleArea`-subclass choice
/// (`Structure.java:926-933`: `via_keepout` -> `ViaObstacleArea`,
/// `place_keepout` -> `ComponentObstacleArea`, else `ObstacleArea`;
/// `Network.java:1080-1149` keeps the same k=0/1/2 order for package
/// keepouts).
fn obstacle_kind_from_ir(kind: KeepoutKindIr) -> ObstacleKind {
    match kind {
        KeepoutKindIr::Keepout => ObstacleKind::ObstacleArea,
        KeepoutKindIr::ViaKeepout => ObstacleKind::ViaObstacleArea,
        KeepoutKindIr::PlaceKeepout => ObstacleKind::ComponentObstacleArea,
    }
}

/// The epic-board mirror of the IR's `BoardShape` — variants matched
/// 1:1 (an epic-dsn variant addition fails this match at compile time,
/// which is the conversion boundary doing its job).
pub(crate) fn board_shape_from_ir(shape: &IrBoardShape) -> BoardShape {
    match shape {
        IrBoardShape::Tile(tile) => BoardShape::Tile(tile.clone()),
        IrBoardShape::PolygonShape(polygon) => BoardShape::PolygonShape(polygon.clone()),
        IrBoardShape::Circle(circle) => BoardShape::Circle(*circle),
    }
}

/// The IR's `border + holes` area, converted shape-by-shape.
fn area_from_ir(area: &AreaIr) -> Area {
    Area {
        border: board_shape_from_ir(&area.border),
        holes: area.holes.iter().map(board_shape_from_ir).collect(),
    }
}

impl ItemEntry {
    /// Java `Item.isDrillable(int)` (`Item.java:895-897` base, the ONLY
    /// overrides being `Trace.java:222-224` and
    /// `ConductionArea.java:403-405`). Base items: `false` — in
    /// particular a Via does NOT override, so vias are not drillable.
    /// Traces: `containsNet(p_net_no)`. Conduction areas:
    /// `!getIsObstacle() || containsNet(p_net_no)` — a non-obstacle
    /// area (a plane routed around in no-plane mode) drills for every
    /// net; an obstacle area only for its own net(s).
    #[must_use]
    pub fn is_drillable(&self, net_number: i32) -> bool {
        match &self.data {
            ItemData::Trace { .. } => self.nets.contains(&net_number),
            ItemData::ConductionArea { is_obstacle, .. } => {
                !*is_obstacle || self.nets.contains(&net_number)
            }
            _ => false,
        }
    }
}

impl Board {
    /// Java `Pin.drillAllowed()` (`Pin.java:348-350` =
    /// `firstLayer() == lastLayer()`): true exactly for a single-layer
    /// (SMD) pin — its padstack carries a shape on one layer only. A
    /// through-hole pin spans every layer and is NOT drill allowed.
    /// The layer span resolves through the padstack (Java
    /// `Pin.firstLayer()`/`lastLayer()` delegate to
    /// `Padstack.fromLayer()`/`toLayer()`). Missing item/padstack
    /// returns `false` where Java would throw — callers only ask for
    /// live pins.
    #[must_use]
    pub fn pin_drill_allowed(&self, id: ItemId) -> bool {
        let Some(entry) = self.get(id) else {
            return false;
        };
        let ItemData::Pin { padstack_no, .. } = &entry.data else {
            return false;
        };
        let Some(padstack) = self.library.padstack(*padstack_no) else {
            return false;
        };
        padstack.from_layer() as i32 == padstack.to_layer()
    }

    /// Java `BoardRules.getDefaultViaDiameter()`
    /// (`BoardRules.java:408-424`): the maximum shape width of the
    /// FIRST via of the FIRST via rule (`getDefaultViaRule` =
    /// `viaRules.getFirst()`, `:242-247`) on its first and last shape
    /// layer; 0.0 when there is no rule or the rule has no vias. Java
    /// dereferences both endpoint shapes unchecked (a valid via
    /// padstack always has them); the `expect`s mirror that.
    #[must_use]
    pub fn default_via_diameter(&self) -> f64 {
        let Some(default_rule) = self.rules.via_rules.first() else {
            return 0.0;
        };
        let Some(&first_via_index) = default_rule.via_infos.first() else {
            return 0.0;
        };
        let Some(info) = self.rules.via_infos.get(first_via_index as usize) else {
            return 0.0;
        };
        let Some(padstack) = self.library.padstack(info.padstack_no) else {
            return 0.0;
        };
        let first_shape = padstack
            .get_shape(padstack.from_layer())
            .expect("valid via padstack has a first-layer shape");
        let mut result = crate::items::shape_max_width(first_shape);
        let to_index = usize::try_from(padstack.to_layer())
            .expect("a padstack with a first-layer shape has a non-negative last layer");
        let last_shape = padstack
            .get_shape(to_index)
            .expect("valid via padstack has a last-layer shape");
        result = result.max(crate::items::shape_max_width(last_shape));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::MAX_ID;
    use epic_dsn::reader::{DsnReadResult, read_board};

    /// A hand-built entry for arena tests.
    fn pin_entry(id: ItemId) -> ItemEntry {
        ItemEntry {
            id,
            data: ItemData::Pin {
                pin_index: 0,
                padstack_no: 1,
            },
            nets: vec![1],
            clearance_class: 1,
            component_id: 1,
            fixed: FixedState::Unfixed,
            on_the_board: false,
        }
    }

    /// `BoardItemRepository.java:166`/`:198`: the revision bumps on
    /// every insert AND every successful remove; a remove of a foreign
    /// id is a no-op and must NOT bump (T69).
    #[test]
    fn revision_bumps_on_insert_and_remove_but_not_on_a_missed_remove() {
        let mut board = Board::new();
        assert_eq!(board.revision(), 0);
        let one = board.alloc_id();
        board.insert_item(pin_entry(one));
        assert_eq!(board.revision(), 1, "insert bumps");
        let two = board.alloc_id();
        board.insert_item(pin_entry(two));
        assert_eq!(board.revision(), 2, "second insert bumps");
        assert!(board.remove_item(one).is_some());
        assert_eq!(board.revision(), 3, "remove bumps");
        assert!(board.remove_item(one).is_none());
        assert_eq!(board.revision(), 3, "a missed remove does not bump");
    }

    /// D25/T60: the arena enumerates DESCENDING id regardless of
    /// insertion order; removal does not disturb the order of the rest.
    /// A plain-ascending arena fails this pin.
    #[test]
    fn iteration_walks_descending_id() {
        let mut board = Board::new();
        let one = board.alloc_id();
        let two = board.alloc_id();
        let three = board.alloc_id();
        // Insert in a shuffled order — the output order is the map's
        // contract, not the insertion order.
        board.insert_item(pin_entry(two));
        board.insert_item(pin_entry(three));
        board.insert_item(pin_entry(one));
        let ids: Vec<u32> = board
            .iter_descending()
            .map(|entry| entry.id.get())
            .collect();
        assert_eq!(ids, vec![three.get(), two.get(), one.get()]);
        board.remove_item(two);
        let ids: Vec<u32> = board
            .iter_descending()
            .map(|entry| entry.id.get())
            .collect();
        assert_eq!(ids, vec![three.get(), one.get()]);
    }

    /// T61: allocation is at CONSTRUCTION, not insert — an allocated
    /// but never-inserted id stays burned, and the inserted entry keeps
    /// exactly the id it was built with.
    #[test]
    fn alloc_burns_and_insert_preserves_the_id() {
        let mut board = Board::new();
        let burned = board.alloc_id(); // constructed, never inserted
        let kept = board.alloc_id();
        board.insert_item(pin_entry(kept));
        let ids: Vec<u32> = board
            .iter_descending()
            .map(|entry| entry.id.get())
            .collect();
        assert_eq!(ids, vec![kept.get()], "the burned id is absent");
        assert_eq!(kept.get(), burned.get() + 1);
        let next = board.alloc_id();
        assert_eq!(next.get(), kept.get() + 1, "the burn is permanent");
    }

    /// T61: deletion never frees ids — the generator continues past
    /// every deleted id.
    #[test]
    fn ids_continue_monotonically_after_delete() {
        let mut board = Board::new();
        let one = board.alloc_id();
        board.insert_item(pin_entry(one));
        let two = board.alloc_id();
        board.insert_item(pin_entry(two));
        board.remove_item(one);
        let three = board.alloc_id();
        assert_eq!(three.get(), two.get() + 1, "no id reuse after delete");
        board.insert_item(pin_entry(three));
        let ids: Vec<u32> = board
            .iter_descending()
            .map(|entry| entry.id.get())
            .collect();
        assert_eq!(ids, vec![three.get(), two.get()]);
    }

    /// `Item.java:63-64`: the on-the-board flag is false at
    /// construction and true once inserted; removal clears it again
    /// (Java flips the flag in `SearchTreeManager.remove`,
    /// `SearchTreeManager.java:61` — `item.setOnTheBoard(false)` behind
    /// the `isOnTheBoard()` guard).
    #[test]
    fn on_the_board_is_set_by_insert_and_cleared_by_remove() {
        let mut board = Board::new();
        let id = board.alloc_id();
        assert!(!pin_entry(id).on_the_board, "constructed, not inserted");
        board.insert_item(pin_entry(id));
        assert!(board.get(id).expect("inserted").on_the_board);
        let removed = board.remove_item(id).expect("removed");
        assert!(!removed.on_the_board, "remove must clear the flag");
    }

    /// F1: [`Board::set_item_nets`] — the `set_item_*` family shape
    /// (plain field write + undo-node mirror). The field write, the
    /// arena read back, the mirror keeping the undo node equal to the
    /// arena entry, and the foreign-id no-op (no panic, nothing
    /// bumped). The mutation CHOREOGRAPHY (tree remove/insert around
    /// it) is pinned end-to-end by the epic-engine uncrossing test.
    #[test]
    fn set_item_nets_writes_the_field_and_mirrors_the_undo_node() {
        let mut board = Board::new();
        let id = board.alloc_id();
        board.insert_item(pin_entry(id)); // nets: vec![1]
        let revision = board.revision();

        board.set_item_nets(id, vec![7, 9]);

        assert_eq!(board.get(id).expect("entry").nets, vec![7, 9]);
        assert_eq!(
            board
                .item_undo
                .value_mut(&Reverse(id))
                .map(|node| node.nets.clone()),
            Some(vec![7, 9]),
            "the undo node carries the SAME nets (mirror_node's contract)"
        );
        assert_eq!(
            board.revision(),
            revision,
            "a field write never bumps the revision (insert/remove own it, T69)"
        );
        // A foreign id is a quiet no-op.
        board.set_item_nets(ItemId::new(999_999), vec![1]);
        assert_eq!(board.get(id).expect("entry").nets, vec![7, 9]);
    }

    /// The generator survives a forced wrap THROUGH the board seam:
    /// `alloc_id` must expose the wrap-to-1 path (T61), not skip it.
    #[test]
    fn board_alloc_id_wraps_at_max_id() {
        let mut board = Board::new();
        board.id_generator.set_next(MAX_ID);
        assert_eq!(board.alloc_id().get(), 1, "wrap to 1");
        assert_eq!(board.alloc_id().get(), 2);
    }

    /// The from_ses_board DSN: outline (id 1) -> structure keepout
    /// (id 2) -> via (id 3) -> open wire (id 4) -> a CLOSED UNFIXED
    /// wire LAST, whose id 5 is BURNED by the `BasicBoard.java:192-196`
    /// drop guard. The IR therefore holds ids {1, 2, 3, 4} with
    /// `last_assigned_item_id == 5` — the burned id sits ABOVE the max
    /// live id, so the generator pin (`alloc == 6`) discriminates
    /// `set_next(last_assigned)` from a wrong `set_next(max live id)`
    /// (which would allocate 5): anchor-blind agreement is out.
    const KEEPOUT_BURN_DSN: &str = r#"(pcb keepout-burn.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
    (keepout
      (path F.Cu 0  2000 2000  4000 2000  4000 4000  2000 4000  2000 2000)
    )
  )
  (library
    (padstack ViaPad_V
      (shape (circle F.Cu 600))
      (attach off)
    )
  )
  (network
    (net PERFECT)
  )
  (wiring
    (via ViaPad_V 5000 5000 (net PERFECT))
    (wire (path F.Cu 125  5000 1000  5000 2000) (net PERFECT))
    (wire (path F.Cu 125  1000 1000  3000 1000  3000 3000  1000 1000) (net PERFECT))
  )
)
"#;

    /// Parses [`KEEPOUT_BURN_DSN`].
    fn parse_keepout_burn() -> epic_dsn::ses_board::SesBoard {
        let mut ses = epic_dsn::ses_board::SesBoard::new();
        match read_board(KEEPOUT_BURN_DSN.as_bytes(), &mut ses) {
            DsnReadResult::Success { warnings } => {
                assert!(warnings.is_empty(), "WARN_COUNT 0, got {warnings:?}");
            }
            other => panic!("expected Success, got {other:?}"),
        }
        ses
    }

    /// `from_ses_board` preserves the IR ids EXACTLY (gap included),
    /// enumerates them DESCENDING, keeps the kinds (T70), and leaves
    /// the generator one past the parse (T61/T69).
    #[test]
    fn from_ses_board_preserves_ids_kinds_and_generator_position() {
        let ses = parse_keepout_burn();
        let ir_ids: Vec<u32> = ses.items.iter().map(|item| item.id() as u32).collect();
        assert_eq!(
            ir_ids,
            vec![1, 2, 3, 4],
            "outline, keepout, via, open trace"
        );
        assert_eq!(ses.last_assigned_item_id(), 5, "id 5 burned (last wire)");
        assert_eq!(
            ir_ids.iter().copied().max(),
            Some(4),
            "the burn sits ABOVE the max live id — the pin discriminates"
        );

        let mut board = Board::from_ses_board(&ses);
        assert_eq!(board.item_count(), ses.items.len());
        let descending: Vec<u32> = board.iter_descending().map(|e| e.id.get()).collect();
        let mut expected = ir_ids.clone();
        expected.reverse();
        assert_eq!(descending, expected, "exact ids, descending order");
        // Every converted entry is ON the board (insert_item set the flag).
        for id in &ir_ids {
            let entry = board.get(ItemId::new(*id)).expect("id preserved");
            assert!(entry.on_the_board, "entry {} was inserted", id);
        }

        // Kinds: the keepout maps to OBSTACLE_AREA (plain kind), the
        // outline to BOARD_OUTLINE.
        assert_eq!(
            board
                .get(ItemId::new(1))
                .expect("outline")
                .board_item_type(),
            BoardItemType::BoardOutline
        );
        let keepout = board.get(ItemId::new(2)).expect("keepout");
        assert_eq!(keepout.board_item_type(), BoardItemType::ObstacleArea);
        assert!(
            matches!(
                keepout.data,
                ItemData::ObstacleArea {
                    kind: ObstacleKind::ObstacleArea,
                    ..
                }
            ),
            "T70: the plain keepout keeps its distinct kind"
        );
        assert_eq!(keepout.fixed, FixedState::SystemFixed, "structure keepout");
        assert!(keepout.nets.is_empty(), "parse keepouts carry no nets");

        // The via carries the IR's fields untouched.
        let ItemIr::Via { via: ir_via, .. } = &ses.items[2] else {
            panic!("IR item 3 is the via");
        };
        let via_entry = board.get(ItemId::new(3)).expect("via");
        assert_eq!(via_entry.board_item_type(), BoardItemType::Via);
        assert_eq!(via_entry.nets, ir_via.nets, "via nets");
        assert_eq!(via_entry.component_id, 0, "vias belong to no component");
        match &via_entry.data {
            ItemData::Via {
                center,
                padstack_no,
                attach_smd_allowed,
            } => {
                assert_eq!(*center, ir_via.location);
                assert_eq!(*padstack_no, ir_via.padstack_no);
                assert_eq!(*attach_smd_allowed, ir_via.attach_smd_allowed);
            }
            other => panic!("expected a via payload, got {other:?}"),
        }

        let trace = board.get(ItemId::new(4)).expect("trace");
        assert_eq!(trace.board_item_type(), BoardItemType::Trace);
        assert_eq!(trace.nets, vec![1]);
        if let ItemData::Trace { lines, .. } = &trace.data {
            assert_eq!(lines.corner_count(), 2, "two IR corners -> two corners");
        } else {
            panic!("expected a trace payload");
        }

        // The generator continues one past the PARSE SEQUENCE (5), not
        // one past the max live id (4): 6 here fails any conversion
        // that seeds the generator from the live items instead of
        // last_assigned_item_id.
        assert_eq!(board.alloc_id().get(), 6, "next id continues the parse");
    }

    /// The bm08 fixture path (the item-geometry pin fixture — module
    /// docs of `crate::items::outline`).
    const BM08: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm08.dsn"
    );

    /// Parses bm08 through the epic-dsn reader and converts it.
    fn bm08_board() -> Board {
        let bytes = std::fs::read(BM08).expect("bm08 fixture present");
        let mut ses = SesBoard::new();
        match read_board(bytes.as_slice(), &mut ses) {
            DsnReadResult::Success { .. } => {}
            other => panic!("expected Success, got {other:?}"),
        }
        Board::from_ses_board(&ses)
    }

    /// The bounding-box wiring: `from_ses_board` carries the IR's
    /// T43 box (capture `BOARD_BBOX 1381520 -1120380 1588500 -979694`)
    /// — the outline bounds + the 1000 margin, NOT the raw outline
    /// bounds (1382520..1587500).
    #[test]
    fn from_ses_board_carries_the_bounding_box() {
        let board = bm08_board();
        let bbox = board.bounding_box().expect("BOARD_BBOX exists");
        assert_eq!(
            (bbox.ll.x, bbox.ll.y, bbox.ur.x, bbox.ur.y),
            (1_381_520, -1_120_380, 1_588_500, -979_694),
            "BOARD_BBOX (capture)"
        );
    }

    /// **The `clearDerivedData` asymmetry, white-box.** Java
    /// `DrillItem.clearDerivedData()` (`DrillItem.java:390-395`)
    /// resets ONLY `precalculatedFirstLayer`/`precalculatedLastLayer`
    /// — `precalculatedMinWidth` is never reset anywhere in Java
    /// (the geometry mutators `:62-93` all route here and leave it).
    /// The port's memo table must reproduce the exact split: after a
    /// `drill_min_width` read (capture `DRILL kind=pin id=41 ...
    /// firstLayer=0 lastLayer=1 minWidth=15240.0`), a clear wipes the
    /// span and KEEPS the width. A "fixed" clear (also wiping the
    /// width) fails the `min_width` assertion — this is the form
    /// where wrong and right differ.
    #[test]
    fn drill_precalc_clear_derived_data_keeps_min_width() {
        let mut board = bm08_board();
        let id = ItemId::new(41);
        assert_eq!(board.drill_first_layer(id), Some(0), "firstLayer=0");
        assert_eq!(board.drill_last_layer(id), Some(1), "lastLayer=1");
        assert_eq!(
            board.drill_min_width(id),
            Some(15_240.0),
            "minWidth=15240.0"
        );
        // White-box: after the three reads the memo holds the full trio.
        let precalc = board.drill_precalc.get(&id).expect("memoized trio");
        assert_eq!(precalc.first_layer, Some(0));
        assert_eq!(precalc.last_layer, Some(1));
        assert_eq!(precalc.min_width, Some(15_240.0));

        board.clear_derived_data(id);
        let precalc = board.drill_precalc.get(&id).expect("the entry is kept");
        assert_eq!(precalc.first_layer, None, "the span is cleared");
        assert_eq!(precalc.last_layer, None, "the span is cleared");
        assert_eq!(
            precalc.min_width,
            Some(15_240.0),
            "the width SURVIVES — DrillItem.java:390-395 clears only the span"
        );

        // The public reads agree: the width is served from the
        // surviving memo; the span recomputes fresh.
        assert_eq!(board.drill_min_width(id), Some(15_240.0));
        assert_eq!(board.drill_first_layer(id), Some(0));
        assert_eq!(board.drill_last_layer(id), Some(1));
    }

    /// `DrillItem.java:148`: a NEGATIVE index clamps to the FIRST
    /// layer — `max(index, 0)` runs BEFORE the span clamp — so
    /// `shapeLayer(-1)` is layer 0, not −1; an index past the span
    /// clamps to the last. Pin 41 (span 0..1): a port without the max
    /// clamp returns −1 here.
    #[test]
    fn drill_shape_layer_clamps_negative_and_overflow_indices() {
        let mut board = bm08_board();
        let id = ItemId::new(41);
        assert_eq!(board.drill_shape_layer(id, 0), Some(0));
        assert_eq!(
            board.drill_shape_layer(id, -1),
            Some(0),
            "max(index, 0) at DrillItem.java:148"
        );
        assert_eq!(board.drill_shape_layer(id, 5), Some(1), "span clamp");
    }

    // Imports for the via wrapper pin below.
    use crate::components::{BoardLibrary, BoardPadstack};
    use epic_geometry::int_box::IntBox;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::point::Point;
    use epic_geometry::regular_tile_shape::RegularTileShape;
    use epic_geometry::tile_shape::TileShape;

    /// An `IntBox` tile shape (the synthetic via padstack rows).
    fn box_shape(x0: i32, y0: i32, x1: i32, y1: i32) -> BoardShape {
        BoardShape::Tile(TileShape::RegularTileShape(RegularTileShape::IntBox(
            IntBox::new(IntPoint::new(x0, y0), IntPoint::new(x1, y1)),
        )))
    }

    /// A tile's corners in the spike capture format (`x,y;...`).
    fn tile_corners(shape: &BoardShape) -> String {
        let BoardShape::Tile(tile) = shape else {
            panic!("expected a tile shape, got {shape:?}");
        };
        (0..tile.border_line_count())
            .map(|no| match tile.corner(no as i32) {
                Point::Int(p) => format!("{},{}", p.x, p.y),
                other => panic!("expected an integer corner, got {other:?}"),
            })
            .collect::<Vec<_>>()
            .join(";")
    }

    /// **I-1 board-path pin — `Board::via_shape`.** The drill-module
    /// tests pin the FREE function `drill::via_shape` on the spike's
    /// captured rows; this drives the item WRAPPER through a real
    /// inserted entry, pinning the entry -> padstack/center/index
    /// wiring the free-function rows cannot see. The padstack is the
    /// spike's `spike_full` (a 10000x10000 box on layer 0, a 4000x2000
    /// box on layer 1), so index 0 and index 1 return DIFFERENT
    /// captured shapes (`SYNTH_FULL_VIA_SHAPE i=0/i=1`) — a wrapper
    /// that ignores `index`, drops the center translation, or reads
    /// the wrong padstack fails here.
    #[test]
    fn via_shape_wrapper_reads_entry_padstack_center_and_index() {
        let mut board = Board::new();
        board.library = BoardLibrary {
            padstacks: vec![BoardPadstack {
                name: "spike_full".to_string(),
                shapes: vec![
                    Some(box_shape(-5000, -5000, 5000, 5000)),
                    Some(box_shape(-2000, -1000, 2000, 1000)),
                ],
                drillable: true,
                placed_absolute: false,
                hole_only: false,
            }],
            ..BoardLibrary::default()
        };
        let id = board.alloc_id();
        board.insert_item(ItemEntry {
            id,
            data: ItemData::Via {
                center: IntPoint::new(650_000, -250_000),
                padstack_no: 1,
                attach_smd_allowed: false,
            },
            nets: vec![1],
            clearance_class: 1,
            component_id: 0,
            fixed: FixedState::Unfixed,
            on_the_board: false,
        });
        assert_eq!(
            tile_corners(&board.via_shape(id, 0).expect("shape 0")),
            "645000,-255000;655000,-255000;655000,-245000;645000,-245000",
            "SYNTH_FULL_VIA_SHAPE i=0 — the layer-0 box translated by the center"
        );
        assert_eq!(
            tile_corners(&board.via_shape(id, 1).expect("shape 1")),
            "648000,-251000;652000,-251000;652000,-249000;648000,-249000",
            "SYNTH_FULL_VIA_SHAPE i=1 — the index reaches the layer arithmetic"
        );
        assert_eq!(board.via_shape(id, 2), None, "past the padstack span");
        assert_eq!(board.via_shape(id, -1), None, "below the padstack span");
        // A non-via id stays None (the wrapper's else arm).
        let other = board.alloc_id();
        board.insert_item(pin_entry(other));
        assert_eq!(board.via_shape(other, 0), None, "a pin entry is not a via");
    }

    // ---- M3-T5 additions: is_drillable / pin_drill_allowed /
    // default_via_diameter ----

    use epic_geometry::circle::Circle;

    fn circle_shape(width: i32) -> BoardShape {
        BoardShape::Circle(Circle::new(IntPoint::new(0, 0), width / 2))
    }

    fn tile_area(ll: i32, ur: i32) -> Area {
        Area::simple(BoardShape::Tile(TileShape::RegularTileShape(
            RegularTileShape::IntBox(IntBox::new(
                epic_geometry::int_point::IntPoint::new(ll, ll),
                epic_geometry::int_point::IntPoint::new(ur, ur),
            )),
        )))
    }

    fn trace_entry(id: ItemId, nets: Vec<i32>) -> ItemEntry {
        ItemEntry {
            id,
            data: ItemData::Trace {
                layer: 0,
                half_width: 50,
                lines: Polyline::from_two_corners(
                    &Point::Int(IntPoint::new(0, 0)),
                    &Point::Int(IntPoint::new(100, 0)),
                ),
            },
            nets,
            clearance_class: 1,
            component_id: 0,
            fixed: FixedState::Unfixed,
            on_the_board: false,
        }
    }

    fn conduction_entry(id: ItemId, nets: Vec<i32>, is_obstacle: bool) -> ItemEntry {
        ItemEntry {
            id,
            data: ItemData::ConductionArea {
                layer: 0,
                area: tile_area(0, 100),
                is_obstacle,
                is_filled: true,
            },
            nets,
            clearance_class: 1,
            component_id: 0,
            fixed: FixedState::Unfixed,
            on_the_board: false,
        }
    }

    /// Java `Item.isDrillable` (`Item.java:895-897` base = false; the
    /// ONLY overrides are Trace `containsNet` (`Trace.java:222-224`)
    /// and ConductionArea `!getIsObstacle() || containsNet`
    /// (`ConductionArea.java:403-405`). The non-obstacle/no-net
    /// conduction row is the discriminator against a contains-only
    /// mutant; the pin row kills an "all connectables are drillable"
    /// mutant.
    #[test]
    fn is_drillable_mirrors_the_java_overrides() {
        let mut board = Board::new();
        let pin = board.alloc_id();
        board.insert_item(pin_entry(pin));
        let trace = board.alloc_id();
        board.insert_item(trace_entry(trace, vec![1]));
        let open_plane = board.alloc_id();
        board.insert_item(conduction_entry(open_plane, vec![], false));
        let own_plane = board.alloc_id();
        board.insert_item(conduction_entry(own_plane, vec![3], true));

        assert!(!board.get(pin).expect("pin").is_drillable(1), "base false");
        assert!(
            board.get(trace).expect("trace").is_drillable(1),
            "trace containing the net"
        );
        assert!(
            !board.get(trace).expect("trace").is_drillable(2),
            "trace of a foreign net"
        );
        assert!(
            board.get(open_plane).expect("area").is_drillable(7),
            "non-obstacle area drills for ANY net"
        );
        assert!(
            board.get(own_plane).expect("area").is_drillable(3),
            "obstacle area drills for its own net"
        );
        assert!(
            !board.get(own_plane).expect("area").is_drillable(4),
            "obstacle area blocks a foreign net"
        );
    }

    /// Java `Pin.drillAllowed()` (`Pin.java:348-350`): true exactly
    /// when the padstack's first shape layer equals its last — a
    /// single-layer (SMD) pad. The two-layer padstack is the contrast
    /// witness (first-match mutants and span-ignoring mutants fail one
    /// of the two rows).
    #[test]
    fn pin_drill_allowed_is_single_layer_only() {
        let mut board = Board::new();
        board.library_mut().padstacks = vec![
            BoardPadstack {
                name: "SMD".to_string(),
                shapes: vec![Some(circle_shape(600))],
                ..Default::default()
            },
            BoardPadstack {
                name: "THRU".to_string(),
                shapes: vec![Some(circle_shape(600)), Some(circle_shape(600))],
                ..Default::default()
            },
        ];
        let smd = board.alloc_id();
        board.insert_item(ItemEntry {
            id: smd,
            data: ItemData::Pin {
                pin_index: 0,
                padstack_no: 1,
            },
            nets: vec![1],
            clearance_class: 1,
            component_id: 0,
            fixed: FixedState::Unfixed,
            on_the_board: false,
        });
        let thru = board.alloc_id();
        board.insert_item(ItemEntry {
            id: thru,
            data: ItemData::Pin {
                pin_index: 0,
                padstack_no: 2,
            },
            nets: vec![1],
            clearance_class: 1,
            component_id: 0,
            fixed: FixedState::Unfixed,
            on_the_board: false,
        });
        assert!(
            board.pin_drill_allowed(smd),
            "single-layer padstack: drill allowed"
        );
        assert!(
            !board.pin_drill_allowed(thru),
            "two-layer padstack: not drill allowed"
        );
        assert!(
            !board.pin_drill_allowed(ItemId::new(9999)),
            "a missing item is not drill allowed"
        );
    }

    /// Java `BoardRules.getDefaultViaDiameter()`
    /// (`BoardRules.java:408-424`): max width of the FIRST via of the
    /// FIRST rule over its first and last layer. The 600/800 layer
    /// pair kills a first-layer-only or last-layer-only mutant (both
    /// return 600/800, the pin wants 800); the no-rule and no-via
    /// rows pin the two 0.0 arms.
    #[test]
    fn default_via_diameter_first_via_of_first_rule() {
        let mut board = Board::new();
        assert_eq!(board.default_via_diameter(), 0.0, "no via rules");
        board.library_mut().padstacks = vec![BoardPadstack {
            name: "VIA".to_string(),
            shapes: vec![Some(circle_shape(600)), Some(circle_shape(800))],
            ..Default::default()
        }];
        board.rules_mut().via_infos = vec![crate::rules_surf::ViaInfo {
            name: "v".to_string(),
            padstack_no: 1,
            clearance_class: 1,
            attach_smd_allowed: false,
        }];
        board.rules_mut().via_rules = vec![crate::rules_surf::ViaRule {
            id: 1,
            name: "r".to_string(),
            via_infos: vec![0],
        }];
        assert_eq!(
            board.default_via_diameter(),
            800.0,
            "max over first and last layer"
        );
        board.rules_mut().via_rules[0].via_infos.clear();
        assert_eq!(board.default_via_diameter(), 0.0, "rule without vias");
    }

    /// T12 restore model, the epic-board half: `reset_transient_after_restore`
    /// mirrors Java's deserialize transients — the oscillation
    /// suppression lifts (`BasicBoard.readObject:1393` reinitializes
    /// `normalizeSuppressedNetNos`), the shove-failure report and the
    /// marking session return to their field-initialized state
    /// (`RoutingBoard.java:69-75`), while the NON-transient failure
    /// log (`RoutingBoard.java:64`) and id watermark
    /// (`Communication.java:30`) are PRESERVED — they roll back
    /// through the snapshot COPY, not through this reset. A reset that
    /// also cleared the log or the generator breaks the restore model.
    #[test]
    fn reset_transient_after_restore_clears_transients_keeps_persistent() {
        let mut board = Board::new();
        let id = board.alloc_id();
        board.insert_item(pin_entry(id));
        // poison every transient
        board.normalize_suppressed_net_nos.insert(3);
        board.changed_area = Some(crate::changed_area::ChangedArea::new(2));
        board.set_shove_failing_obstacle(Some(id));
        board.set_shove_failing_layer(1);
        // persistent state that must SURVIVE the reset
        board
            .failure_log
            .record_failure(u64::from(id.get()), 1, 3, "FAILED", None);
        let watermark = board.max_generated_id();

        board.reset_transient_after_restore();

        assert!(
            board.normalize_suppressed_net_nos.is_empty(),
            "the suppression set lifts (readObject :1393)"
        );
        assert!(board.changed_area.is_none(), "changedArea is transient");
        assert_eq!(board.shove_failing_obstacle(), None, "transient");
        assert_eq!(board.shove_failing_layer(), -1, "transient");
        assert_eq!(
            board.failure_log.failure_count(u64::from(id.get())),
            1,
            "failureLog is NON-transient — it rolls back via the copy, never here"
        );
        assert_eq!(
            board.max_generated_id(),
            watermark,
            "the generator is non-transient — the snapshot-time watermark"
        );
    }

    /// T17b (buglog 176): the VIRTUAL `Item.isTraceObstacle(int)`
    /// dispatch — [`Board::item_is_trace_obstacle`]. THE CROSSING CELL
    /// is the parity bug: a parse-time `ConductionArea` (every DSN
    /// plane/pour is inserted `isObstacle=false`,
    /// `Structure.java:1113`) is NOT a trace obstacle for a foreign
    /// net — Java's maze routes THROUGH the ecc83-pp_v2 GND plane,
    /// while the pre-fix net-membership arm blocked it and froze the
    /// board at incomplete 1. Every row and column of the
    /// kind × flag × net discriminator is pinned (mode 13: the
    /// crossing cell is the pin, not the corners): the CA flag arms
    /// (`ConductionArea.java:398-400`), the keepout arms
    /// (`ComponentObstacleArea.java:71-73`, `ViaObstacleArea.java:100-102`
    /// — both unconditionally false), the plain-keepout BASE arm
    /// (`Item.java:170-172` — no override), the net-`<= 0` faces
    /// (`Item.java:149-152` — a non-positive net is never contained,
    /// so the base face is true and the CA face degrades to its flag),
    /// and the unknown-id verdict.
    #[test]
    fn item_is_trace_obstacle_pins_the_java_virtual_dispatch() {
        let area = || {
            crate::items::Area::simple(BoardShape::Tile(TileShape::RegularTileShape(
                epic_geometry::regular_tile_shape::RegularTileShape::IntBox(IntBox::new(
                    IntPoint::new(0, 0),
                    IntPoint::new(1000, 1000),
                )),
            )))
        };
        let entry = |board: &mut Board, data: ItemData, nets: Vec<i32>| {
            let id = board.alloc_id();
            board.insert_item(ItemEntry {
                id,
                data,
                nets,
                clearance_class: 1,
                component_id: 1,
                fixed: FixedState::Unfixed,
                on_the_board: false,
            });
            id
        };
        let conduction = |is_obstacle: bool| ItemData::ConductionArea {
            layer: 1,
            area: area(),
            is_obstacle,
            is_filled: true,
        };
        let keepout = |kind: ObstacleKind| ItemData::ObstacleArea {
            kind,
            layer: 1,
            area: area(),
            translation: IntPoint::new(0, 0),
            rotation: 0.0,
            side_changed: false,
            name: None,
        };

        let mut board = Board::new();
        // THE CROSSING ROW: flag=false × foreign net → NOT a trace
        // obstacle (buglog 176 — the GND plane the maze must route
        // through).
        let plane = entry(&mut board, conduction(false), vec![2]);
        assert!(!board.item_is_trace_obstacle(plane, 6));
        // flag=false × own net → false (both terms agree).
        assert!(!board.item_is_trace_obstacle(plane, 2));
        // flag=false × net <= 0 → false (`isObstacle && !containsNet`).
        assert!(!board.item_is_trace_obstacle(plane, 0));
        // flag=true flips the foreign verdict to true — the router-set
        // obstacle face (`setIsObstacle`).
        let solid_plane = entry(&mut board, conduction(true), vec![2]);
        assert!(board.item_is_trace_obstacle(solid_plane, 6));
        // flag=true × own net stays false (containsNet wins).
        assert!(!board.item_is_trace_obstacle(solid_plane, 2));
        // flag=true × net <= 0 → true (the flag degrades to itself).
        assert!(board.item_is_trace_obstacle(solid_plane, 0));

        // Keepout arms: place/via keepouts NEVER block traces.
        let place_keepout = entry(
            &mut board,
            keepout(ObstacleKind::ComponentObstacleArea),
            vec![],
        );
        let via_keepout = entry(&mut board, keepout(ObstacleKind::ViaObstacleArea), vec![]);
        assert!(!board.item_is_trace_obstacle(place_keepout, 6));
        assert!(!board.item_is_trace_obstacle(via_keepout, 6));
        // A plain keepout has NO override — the base face applies
        // (net-less, so every positive net is foreign → true).
        let plain_keepout = entry(&mut board, keepout(ObstacleKind::ObstacleArea), vec![]);
        assert!(board.item_is_trace_obstacle(plain_keepout, 6));

        // Base face on a connectable item: own net false, foreign true,
        // net <= 0 true.
        let pin = entry(
            &mut board,
            ItemData::Pin {
                pin_index: 0,
                padstack_no: 1,
            },
            vec![6],
        );
        assert!(!board.item_is_trace_obstacle(pin, 6));
        assert!(board.item_is_trace_obstacle(pin, 7));
        assert!(board.item_is_trace_obstacle(pin, 0));

        // Unknown id: the net-less foreign verdict `true` the pre-fix
        // arm produced (`!nets.contains` on no nets).
        assert!(board.item_is_trace_obstacle(ItemId::new(999_999), 6));
    }
}
