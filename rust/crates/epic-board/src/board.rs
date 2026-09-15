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

/// The live board model (Java `BasicBoard`'s item-list face plus the
/// rules / layers / components / library read surfaces, M2 Task 3).
#[derive(Clone, Debug, Default)]
pub struct Board {
    /// The items keyed by DESCENDING id (module docs, D25/T60).
    items: BTreeMap<Reverse<ItemId>, ItemEntry>,
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
    /// Java `board.layerStructure`.
    layers: LayerStructure,
    /// Java `board.components` (with its own undo stack, T63).
    components: Components,
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
    /// Undo policy: Java's undo restores FRESH item copies (the −1
    /// sentinels come back with the copy, so the spans recompute);
    /// this memo lives OUTSIDE the entries and would survive a
    /// restore — when Board-level undo lands, the restore path must
    /// drop the restored id's entry here.
    drill_precalc: BTreeMap<ItemId, DrillPrecalc>,
}

impl Board {
    /// An empty board.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocates the next item id — the item-CONSTRUCTION counterpart
    /// of Java's `board.communication.idGenerator.newId()` call inside
    /// `Item`'s constructor (`Item.java:86-90`).
    ///
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

    /// Inserts a constructed entry. The id must have come from
    /// [`Board::alloc_id`] (or a parse IR) and must be free — Java's
    /// map would silently replace a duplicate key, which this port
    /// treats as the programming error it is. Bumps the revision (T69)
    /// and sets the entry's `on_the_board` flag (`Item.java:63-64`).
    /// Does NOT touch the generator: insertion consumes no id.
    pub fn insert_item(&mut self, mut entry: ItemEntry) {
        assert!(
            !self.items.contains_key(&Reverse(entry.id)),
            "insert_item: item id {} already on the board",
            entry.id.get()
        );
        entry.on_the_board = true;
        self.items.insert(Reverse(entry.id), entry);
        self.revision += 1;
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
    pub fn remove_item(&mut self, id: ItemId) -> Option<ItemEntry> {
        let mut removed = self.items.remove(&Reverse(id));
        if let Some(entry) = removed.as_mut() {
            entry.on_the_board = false;
            // The per-tree SHAPE cache must not outlive the arena slot
            // (T7 quality review NIT-4): in Java the item OBJECT goes
            // unreachable with its caches when the list drops it. Ids
            // are never reused, so this is pure hygiene against a
            // future delete flow — but the DRILL-SPAN memo stays: a
            // Java item re-inserted by undo keeps its precalculated
            // span (remove does not call clearDerivedData).
            self.shape_precalc.remove(&id);
            self.revision += 1;
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

    /// The number of live items.
    #[must_use]
    pub fn item_count(&self) -> usize {
        self.items.len()
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

    /// Java `Item.setOnTheBoard(boolean)` (`Item.java:248`): the raw
    /// flag write the search-tree manager performs after broadcasting
    /// an insert/remove (`SearchTreeManager.java:43` / `:61`).
    pub fn set_on_the_board(&mut self, id: ItemId, value: bool) {
        if let Some(entry) = self.items.get_mut(&Reverse(id)) {
            entry.on_the_board = value;
        }
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
    /// - Trace polylines are rebuilt from the IR's (rounded) corner
    ///   list with `Polyline::from_points` — the inverse of Java's
    ///   `new Polyline(polygon)` (`Wiring.java:531`); for the
    ///   `PolylinePath` path (`Wiring.java:562`, `new Polyline(lines)`)
    ///   the interior corners are identical and the end lines take the
    ///   canonical perpendicular directions. Corner rounding itself is
    ///   the DOCUMENTED M1b divergence (`scope/wiring.rs` module docs:
    ///   Java keeps exact rationals; the port rounds at parse), not
    ///   re-derived here.
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
                lines: polyline_from_corners(&trace.corners),
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

/// Rebuilds a trace polyline from the IR's corner list — see
/// [`Board::from_ses_board`] for the constructor contract.
fn polyline_from_corners(
    corners: &[epic_geometry::int_point::IntPoint],
) -> epic_geometry::polyline::Polyline {
    let points: Vec<epic_geometry::point::Point> = corners
        .iter()
        .map(|&corner| epic_geometry::point::Point::Int(corner))
        .collect();
    epic_geometry::polyline::Polyline::from_points(&points)
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
}
