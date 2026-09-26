//! Java `autoroute/drill` — the drill-page subsystem of the maze
//! search: the page grid ([`DrillPageArray`]), the per-page candidate
//! via enumeration ([`DrillPage::get_drills`]), the layer-change
//! expansion object ([`ExpansionDrill`]) and the via-mask layer-change
//! logic ([`expand_other_layers::expand_to_other_layers`]), plus the
//! [`MazeSearchElement`] twin both drill types embed.
//!
//! Seam boundaries (this task ports the DECISIONS, not the deep
//! machinery — see `SEAM.md`):
//! * `ViaLayerChecker` (T10) — Java `ForcedViaInserter.checkLayer`;
//!   `checkLayerWithAnyMatchingVia`'s iteration + radius arithmetic
//!   are ported around it.
//! * `DestinationDistance` (T8) — Java `search.destinationDistance`.
//! * the [`DrillMazeListElement`] emission (T6) — Java
//!   `search.mazeExpansionList.add`; the callback consumer owns the
//!   front's ordering/dedup.
//!
//! ## The two attach-SMD flows (reconciliation note)
//!
//! `ViaInfo.attach_smd_allowed` feeds TWO DIFFERENT inputs, and they
//! must not be conflated:
//! * **Per-rule-via property (control side).**
//!   `MazeExpansionEngine.checkLayerWithAnyMatchingVia:394` passes
//!   `viaInfo.attachSmdAllowed()` of EACH via of `ctrl.viaRule` into
//!   `ForcedViaInserter.checkLayer` — it decides whether a via PLACED
//!   at an attach-SMD position is legal on that layer. Consumed by
//!   [`expand_other_layers`] through [`DrillEngine::via_rule_vias`].
//! * **Net-level relaxation (enumeration side).**
//!   `DrillPage.getDrills`' `attachSmd` boolean comes from
//!   `ctrl.attachSmdAllowed` — true when ANY via of the rule allows
//!   attach (the aggregate built in `control.rs`). It relaxes the
//!   CANDIDATE ENUMERATION: drill-allowed (SMD) pins stop being
//!   cutout obstacles and their centers become drill locations
//!   (`calcPinCenterInDrill`).
//!
//! ## Deviations from Java
//!
//! * `emitDiagnostics` (DrillPageArray/DrillPage/ExpansionDrill) is
//!   omitted — the Rust engine has no diagnostic sink yet.
//! * Java wraps `completeExpansionRoom` in `catch (Exception)` →
//!   empty result; the Rust seam lets the T4 completion machinery
//!   panic (the M1a/M3 exception-to-panic discipline).
//! * Java `ViaRule.containsPadstack` is REFERENCE identity over the
//!   rule's vias; here padstack-NUMBER equality (numbers are unique
//!   per registry).

pub mod drill_page;
pub mod expand_other_layers;
pub mod expansion_drill;
pub mod maze_search_element;
pub mod page_array;

#[cfg(test)]
pub(crate) mod pins;
#[cfg(test)]
mod tests;

use std::sync::atomic::AtomicBool;

use epic_board::items::BoardShape;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::int_box::IntBox;
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;
use epic_geometry::tile_shape::TileShape;

pub use drill_page::DrillPage;
pub use expand_other_layers::{
    CheckDrillResult, DestinationDistance, DrillMazeListElement, ViaLayerChecker,
    check_layer_with_any_matching_via, expand_to_other_layers,
};
pub use expansion_drill::ExpansionDrill;
pub use maze_search_element::{Adjustment, MazeSearchElement};
pub use page_array::{DrillPageArray, max_drill_page_width};

use crate::expansion::{NeighbourEngine, TreeEntry};

/// Java `ctrl.viaRule.getVia(i)` projected to what the via-mask port
/// consumes (Java `ViaInfo`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViaRuleVia {
    /// Java `ViaInfo.getPadstack()` — the 1-based padstack number.
    pub padstack_no: i32,
    /// Java `ViaInfo.getClearanceClassIndex()`.
    pub clearance_class: i32,
    /// Java `ViaInfo.attachSmdAllowed()` — the PER-RULE-VIA property
    /// (see the module doc's two-flow note).
    pub attach_smd_allowed: bool,
}

/// The engine seam behind the drill subsystem: everything the Java
/// code reaches through `autorouteEngine`/`board` references. A
/// supertrait of [`NeighbourEngine`] — the drill code shares the
/// expansion graph's room registry and tree (T4).
pub trait DrillEngine: NeighbourEngine {
    /// Java `board.boundingBox` — the page-grid frame.
    fn board_bounds(&self) -> IntBox;
    /// Java `board.getLayerCount()`.
    fn layer_count(&self) -> i32;
    /// Java `autorouteEngine.stoppableThread` — the split abort flag
    /// handed to `PolylineArea.splitToConvex`; `None` disables
    /// stopping (the default for tests).
    fn stop_flag(&self) -> Option<&AtomicBool>;
    /// Java `autorouteEngine.isStopRequested` (`:287-295`) as the maze
    /// and drill walks see it. The flag-only default serves the
    /// capture/test engines; the production override (T12) consults
    /// the [`crate::engine::RouteBudget`] FIRST, mirroring Java's
    /// budget-before-flag order.
    fn is_stop_requested(&self) -> bool {
        self.stop_flag()
            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
    }

    // ---- the calcPinCenterInDrill / getDrills item reads ----
    /// Java `board.overlappingItems(area, layer)`
    /// (`BasicBoard.java:939-950`) — items overlapping the shape on
    /// the layer, in the Java `TreeSet<Item>` order (DESCENDING item
    /// id; `calcPinCenterInDrill`'s missing break makes the LAST —
    /// lowest-id — match win).
    fn overlapping_items(&self, shape: &TileShape, layer: i32) -> Vec<u64>;
    /// Java `Item.isDrillable(netNumber)`.
    fn item_is_drillable(&self, item_key: u64, net_number: i32) -> bool;
    /// Java `currentItem instanceof Pin`.
    fn item_is_pin(&self, item_key: u64) -> bool;
    /// Java `Pin.drillAllowed()`.
    fn pin_drill_allowed(&self, item_key: u64) -> bool;
    /// Java `Pin.getCenter()`.
    fn pin_center(&self, item_key: u64) -> Option<Point>;

    // ---- the ripped-via branch reads ----
    /// Java `currentObstacleItem instanceof Via`.
    fn item_is_via(&self, item_key: u64) -> bool;
    /// Java `DrillItem.getCenter()` as the via arm of
    /// `MazeTraceShover.endPointsMatching` reads it (:329-332): the
    /// STORED center — total and always present for a live via — and
    /// NEVER the lazily-computed `getAutorouteDrillInfo` transient
    /// (`Via.java:204-216`; buglog 170). `None` is unreachable for a
    /// live via; the Option is the port's defensive shape for a
    /// non-via key.
    fn via_center(&self, item_key: u64) -> Option<Point>;
    /// Java `((Via) item).getPadstack()` — the 1-based padstack number.
    fn item_padstack_no(&self, item_key: u64) -> Option<i32>;
    /// Java `Item.clearanceClassIndex()`.
    fn item_clearance_class(&self, item_key: u64) -> i32;

    // ---- the padstack / via-rule reads ----
    /// Java `Padstack.fromLayer()`/`toLayer()` — the (first, last)
    /// 0-based shape layers; first = slot count when all slots are
    /// null, last = -1 (the Rust `BoardPadstack` derivations).
    fn padstack_layer_span(&self, padstack_no: i32) -> (i32, i32);
    /// Java `Padstack.getShape(layer)`.
    fn padstack_shape(&self, padstack_no: i32, layer: i32) -> Option<BoardShape>;
    /// Java `ctrl.viaRule`'s vias in rule order.
    fn via_rule_vias(&self) -> Vec<ViaRuleVia>;

    // ---- the ExpansionDrill completion ----
    /// Java `autorouteEngine.completeExpansionRoom(room)` for the
    /// drill's fresh null-shape room
    /// (`AutorouteEngine.java:418-522`): complete the shape, calculate
    /// doors, register and insert the completed rooms; returns the
    /// completed room keys in order. `room_shape = None` mirrors the
    /// Java `IncompleteFreeSpaceExpansionRoom(null, layer, shape)`
    /// construction.
    ///
    /// T6 provides the PRODUCTION body as a provided method composing
    /// the T4 completion primitives — see
    /// [`crate::maze::completion`].
    fn complete_expansion_room(
        &mut self,
        room_shape: Option<&TileShape>,
        contained_shape: &TileShape,
        layer: i32,
    ) -> Vec<u64>
    where
        Self: Sized,
    {
        debug_assert!(
            room_shape.is_none(),
            "the drill completion seam passes the null room shape"
        );
        crate::maze::completion::complete_null_shape_room(self, contained_shape, layer)
    }

    // ---- T6 seams (the maze engine's item/layer reads) ----
    /// Java `ItemAutorouteInfo.isStartInfo()` NEGATED — Java
    /// `TargetItemExpansionDoor.isDestinationDoor()`
    /// (`TargetItemExpansionDoor.java:45-48`).
    fn item_is_destination(&self, item_key: u64) -> bool;
    /// Java `ItemAutorouteInfo.setStartInfo(bool)` (`MazeSearchEngine
    /// .init:979/1013`).
    fn set_item_start_info(&mut self, item_key: u64, start: bool);
    /// Java `Item.treeShapeCount(tree)`.
    fn item_tree_shape_count(&self, item_key: u64) -> i32;
    /// Java `Pin.getTraceNeckdownHalfwidth(layer)` — 0.0 when the pin
    /// has no neckdown (the seam T10 sharpens; `withNeckdown` is false
    /// in Tier A so the capture never reads it).
    fn pin_neckdown_half_width(&self, item_key: u64, layer: i32) -> f64;
    /// Java `board.layerStructure.layers[layer].isSignal` — the
    /// inactive-layer occupation check (`expandToRoomDoors:398`).
    fn layer_is_signal(&self, layer: i32) -> bool;
    /// The inactive-layer drill-vs-plane probe (`:478-489`): true when
    /// a foreign-net conduction area overlaps the drill location (the
    /// `!currentItem.containsNet(ctrl.netNumber)` arm).
    fn drill_hits_foreign_conduction(&self, location: &Point, layer: i32, net_number: i32) -> bool;
    /// Java `Pin.nearestTraceExitCorner(FloatPoint, double, int)`
    /// (`expandToDrill:56-65`) — `None` is Java null (the pin has no
    /// precomputed exit corner). The T6 fixtures never reach it (the
    /// DrillPage-from-target-pin-door combination is excluded); T10
    /// owns the real port.
    fn pin_nearest_trace_exit_corner(
        &self,
        item_key: u64,
        from_point: &FloatPoint,
        trace_half_width: i32,
        layer: i32,
    ) -> Option<FloatPoint>;

    // ---- T6 seams (roomShapeIsThick + the thin-room target probe) ----
    /// Java `currentObstacleItem instanceof Trace`.
    fn item_is_trace(&self, item_key: u64) -> bool;
    /// Java `Trace.getHalfWidth()` (UNcompensated).
    fn item_trace_half_width(&self, item_key: u64) -> i32;
    /// Java `SearchTree.clearanceCompensationValue(class, layer)`.
    fn clearance_compensation_value(&self, clearance_class: i32, layer: i32) -> i32;
    /// Java `Item.shapeLayer(shapeIndex)` — the board layer of the
    /// item's i-th tree shape (`MazeSearchEngine.init`).
    fn item_shape_layer(&self, item_key: u64, shape_index: u32) -> i32;
    /// Java `Via.getTreeShapeOnLayer(tree, layer)` — `None` is the
    /// Java null (no shape of the item on that layer).
    fn item_tree_shape_on_layer(&self, item_key: u64, layer: i32) -> Option<TileShape>;
    /// Java `Via.getAutorouteDrillInfo(tree)` — the ripped-via drill
    /// the maze engine expands into (`expandToRoomDoors:613-621`).
    /// SEAM (T7): the drill construction; `None` — unreachable in the
    /// T6 fixtures (no vias) — keeps the decision site visible.
    fn via_drill_info(&self, item_key: u64) -> Option<(Point, i32, i32, TileShape)> {
        let _ = item_key;
        None
    }
    /// Java `board.checkForcedTracePolyline(polyline, halfWidth, layer,
    /// netNumbers, clearanceClass, shoveDepths...)` (`expandToTargetDoors
    /// :681-689`) — the thin-room target-door legality probe. SEAM (T7):
    /// the stub answers "legal" (empty board space); the T6 fixtures
    /// keep target expansion inside thick rooms, where Java skips the
    /// check entirely. T7 widens the signature when the real check lands.
    fn check_forced_trace_polyline(&self, from: &IntPoint, to: &IntPoint, layer: i32) -> bool {
        let _ = (from, to, layer);
        true
    }

    // ---- T7 seams (the ripup resolver + the read-only shove probe) ----

    /// Java `Item.getNormalContacts()` — the NO-ARG contact dispatcher
    /// (`Item.java:613-615`): the endpoint union for a trace, the
    /// center-box contacts for a drill item, the conduction-area set.
    /// DESCENDING order (Java TreeSet). Consumed by the Connection
    /// walk and the `checkRipup` via arm.
    fn item_normal_contacts(&mut self, item_key: u64) -> Vec<u64>;

    /// Java `Trace.getNormalContacts(Point, boolean ignoreNet)`
    /// (`Trace.java:173-203`).
    fn trace_normal_contacts_at(
        &mut self,
        item_key: u64,
        point: &Point,
        ignore_net: bool,
    ) -> Vec<u64>;

    /// Java `Trace.getStartContacts()` (`Trace.java:108-110`).
    fn trace_start_contacts(&mut self, item_key: u64) -> Vec<u64>;

    /// Java `Trace.getEndContacts()` (`Trace.java:116-118`).
    fn trace_end_contacts(&mut self, item_key: u64) -> Vec<u64>;

    /// Java `Item.normalContactPoint(Item)` — the exact touch point of
    /// two connectable items, or Java's null.
    fn normal_contact_point(&mut self, first_key: u64, second_key: u64) -> Option<Point>;

    /// Java `Item.firstCommonLayer(Item)` — `-1` when the items share
    /// no layer.
    fn first_common_layer(&mut self, first_key: u64, second_key: u64) -> i32;

    /// Java `Item.isUserFixed()`.
    fn item_is_user_fixed(&self, item_key: u64) -> bool;

    /// Java `Item.isShoveFixed()` — the fanout-protection arm of
    /// `calcFanoutViaRipupCostFactor`.
    fn item_is_shove_fixed(&self, item_key: u64) -> bool;

    /// Java `PolylineTrace.cornerCount()` =
    /// `PolylineTraceGeometry.cornerCount(lines)` =
    /// `lines.length - 1` (Polyline.java:178-180) — the LINE count
    /// minus 1, saturating at 0; Java's −1 on the empty polyline is
    /// unreachable for real traces. The `== 2` consumer is the
    /// fanout-via protection's 2-corner shove-fixed arm
    /// (`MazeRipupResolver.java:51-56`); counting LINES there made
    /// every 2-corner trace answer 3 and silently killed the arm (the
    /// bm06 fanout fork, buglog 181). Provided by default — every
    /// engine reads the same board model, so the body is shared
    /// verbatim through the required [`Self::item_trace_polyline`].
    fn item_trace_corner_count(&self, item_key: u64) -> i32 {
        i32::try_from(
            self.item_trace_polyline(item_key)
                .map_or(0, |polyline| polyline.lines.len().saturating_sub(1)),
        )
        .expect("corner count fits i32")
    }

    /// Java `Trace.getLength()` — the polyline length approximation.
    fn item_trace_length(&self, item_key: u64) -> f64;

    /// Java `PolylineTrace.polyline()` — `None` is unreachable for a
    /// live trace (callers pre-check `item_is_trace`).
    fn item_trace_polyline(&self, item_key: u64) -> Option<epic_geometry::polyline::Polyline>;

    /// Java `SearchTree.overlappingObjects(shape, layer, ignoreNetNos)`
    /// (`ShapeSearchTree.java:404-478`): the tree objects overlapping
    /// `shape` on `layer` that are obstacles w.r.t. EVERY ignore net
    /// (`!currentObject.isObstacle(ignoreNetNos[i])` → skip, `:412-413`
    /// — own-net objects are filtered out). Items AND room keys occur;
    /// the walk consumers re-filter by kind.
    fn overlapping_objects_ignore_nets(
        &self,
        shape: &TileShape,
        layer: i32,
        ignore_nets: &[i32],
    ) -> Vec<u64>;

    /// Java `RoutingBoard.checkTraceSegment(segment, layer, netNumbers,
    /// halfWidth, clearanceClass, cushionsEnabled)` — the READ-ONLY
    /// clearance check the shove probe consults for the free width
    /// (`MazeTraceShover.java:176-181`). SEAM (T10→T11): since the
    /// engine assembly the DEFAULT body still answers Java's
    /// `Integer.MAX_VALUE` "no shortening" verdict (capture/test
    /// engines keep it), while the production engine overrides with the
    /// real `routing_board_search::check_trace_segment` — which needs
    /// `&mut self` (the query re-derives stored shapes through the
    /// manager's memoizing face), hence the receiver widening.
    fn check_trace_segment(
        &mut self,
        line_segment: &epic_geometry::line_segment::LineSegment,
        layer: i32,
        net_numbers: &[i32],
        half_width: i32,
        clearance_class: i32,
        cushions_enabled: bool,
    ) -> f64 {
        let _ = (
            line_segment,
            layer,
            net_numbers,
            half_width,
            clearance_class,
            cushions_enabled,
        );
        2147483647.0
    }

    /// The via-legality dispatch (T11): Java's
    /// `MazeExpansionEngine.checkLayerWithAnyMatchingVia` calls
    /// `ForcedViaInserter.checkLayer` DIRECTLY on the board; the port
    /// routes the call through the ctx so the production engine (which
    /// owns `&mut manager`+`&mut board`) can run the real probe, while
    /// capture/test engines keep the default that consults the
    /// injected [`ViaLayerChecker`] — capture behavior preserved.
    /// `Self: Sized` because the default body needs no `dyn` dispatch
    /// and every caller holds the concrete engine.
    #[allow(clippy::too_many_arguments)] // the Java parameter list
    fn via_layer_check(
        &mut self,
        ctrl: &crate::control::AutorouteControl,
        checker: &mut impl ViaLayerChecker,
        required_radius: f64,
        clearance_class: i32,
        attach_smd_allowed: bool,
        room_shape: &TileShape,
        location: &Point,
        layer: i32,
        net_number: i32,
    ) -> CheckDrillResult
    where
        Self: Sized,
    {
        let _ = ctrl;
        checker.check_layer(
            required_radius,
            clearance_class,
            attach_smd_allowed,
            room_shape,
            location,
            layer,
            net_number,
        )
    }

    /// Java `TraceShover.check(board, segment, shoveToTheLeft, layer,
    /// netNumbers, halfWidth, clearanceClass, maxShoveTraceRecursionDepth,
    /// maxShoveViaRecursionDepth)` — the recursive shove distance the
    /// probe consults (`MazeTraceShover.java:203-215`). SEAM (T10): the
    /// default 0.0 answers "nothing shovable", so the probe returns
    /// TRUE early with an EMPTY door list — the behavior of a board
    /// with nothing to shove into. Tests force capture values through
    /// a harness override of this seam.
    // Java's `TraceShover.check` takes 8 parameters; the arity is the
    // ported Java surface, not a design smell.
    #[allow(clippy::too_many_arguments)]
    fn shove_trace_check(
        &mut self,
        line_segment: &epic_geometry::line_segment::LineSegment,
        shove_to_the_left: bool,
        layer: i32,
        net_numbers: &[i32],
        half_width: i32,
        clearance_class: i32,
        max_shove_trace_recursion_depth: i32,
        max_shove_via_recursion_depth: i32,
    ) -> f64 {
        let _ = (
            line_segment,
            shove_to_the_left,
            layer,
            net_numbers,
            half_width,
            clearance_class,
            max_shove_trace_recursion_depth,
            max_shove_via_recursion_depth,
        );
        0.0
    }

    /// Java `board.rules.getTraceAngleRestriction()` — the door-length
    /// measure of `doorIsSmall` (`MazeSearchEngine.java:773-784`).
    fn trace_angle_restriction(&self) -> crate::control::AngleRestriction;
}

/// The Java tree-query iteration order for the drill walks: the search
/// tree's `TreeSet` leaf order — object id DESCENDING via the element
/// comparators (`Item.compareTo`, `Item.java:95-103`:
/// `result = item.id - id`; `CompleteFreeSpaceExpansionRoom.compareTo`,
/// `CompleteFreeSpaceExpansionRoom.java:46-54`:
/// `result = other.id - this.id`), shape index ascending secondary,
/// dedup. Room entries interleave freely (they are skipped before any
/// order-sensitive effect in both consumers).
///
/// The trailing `dedup` is a DEFENSIVE normalization, not a Java
/// behavior: Java's walk iterates a `TreeSet`, which yields each leaf
/// exactly once and cannot duplicate. The dedup pins that seam contract
/// for the Rust `overlapping_entries` implementors and is a no-op for a
/// faithful one.
pub(crate) fn java_ordered_entries(
    ctx: &mut impl NeighbourEngine,
    shape: &TileShape,
    layer: i32,
) -> Vec<TreeEntry> {
    let mut entries = ctx.overlapping_entries(shape, layer);
    entries.sort_by(|a, b| {
        ctx.object_id(b.object_key)
            .cmp(&ctx.object_id(a.object_key))
            .then(a.shape_index_in_object.cmp(&b.shape_index_in_object))
    });
    entries.dedup();
    entries
}
