//! Java `autoroute/path/FoundConnectionInserter.java` — inserts the
//! traces and vias of the connection found by the autoroute algorithm
//! (the T11 build item; the downstream half of the T9 locator).
//!
//! The public face is [`get_instance`] (Java `getInstance`,
//! `:40-111`): walk the locator's [`ResultItem`]s target→start, insert
//! a via per layer change ([`Self::insert_via`]), the segment walk per
//! item ([`Self::insert_trace`]), the final via down to the start
//! layer, the two `connect_to_trace` splices onto EXISTING trace
//! endpoints (half widths CROSS-INDEXED: the target splice uses the
//! START layer's trace width and vice versa, `:77-106`), then the
//! per-net normalize ([`epic_board::normalize_all::
//! normalize_traces_of_net`], Java `BasicBoard.normalizeTraces(int)`).
//!
//! ## Event rows (log-only, hardcoded gates verbatim)
//!
//! Every `FRLogger` row of the Java method is reproduced natively
//! through the pluggable [`InserterEventSink`] (production: the
//! [`NullSink`] no-op; captures/tests: a collecting sink). The gates
//! are the Java ones, verbatim: the raw rows are unconditional `if
//! (true)` blocks, the five-arg structured twins are the
//! `[<method>] [<operation>] <payload>: <impactedItems>` flatten the
//! ForcedInsertProbe log4j tap emits, and the fanout diagnostics are
//! gated on `isFanout && fanoutStartPinName.startsWith("U27-")`
//! (`shouldTraceFanoutDiagnostics`, `:787-791`). Rows never affect
//! control flow (the bug-118 convention: no WARN_COUNT/digest impact).
//! Row surface:
//!
//! * `compare_trace_connection_item_raw` — per locator item (`:48-65`).
//! * `compare_trace_insert_segment_ids` — the id-burn row at SEGMENT
//!   granularity (`:189-200`).
//! * `compare_trace_insert_segment_raw` + the twin
//!   `[FoundConnectionInserter.insert_trace]
//!   [compare_trace_insert_segment] ...` — per segment decision.
//!   ADVANCE/FAIL rows carry `micro_neckdown=`; the
//!   VIOLATION_CORRECTED rows do NOT (the Java literals, `:220-343`).
//! * `compare_trace_stub_found` / the twin `compare_trace_stub_cleanup`
//!   (`:405-431`).
//! * `FANOUT_DIAG event=..., pin=..., net=..., <msg>` — the fanout
//!   diagnostics (`trace_insert_failed`, `via_mask_not_found`,
//!   `forced_via_insert_failed`,
//!   `trace_insert_micro_neckdown_{success,failed}`).
//! * the plain debug/warn messages (`insert trace failed for net #`,
//!   the via-mask faces, the null-corner `connect_to_trace` skips).
//!
//! ## Documented substitutions (all anchors-blessed)
//!
//! * **Reference → value equality on `Point`.** Every Java
//!   `okPoint == insertPolyline.lastCorner()`-style decision is a
//!   reference comparison that works because `Line` STORES its
//!   endpoints (`firstCorner()`/`lastCorner()` hand back the stored
//!   objects). Rust compares values; the anchors' analysis: a
//!   value-equal point fails the Java reference check only when the
//!   insertion RETURNED a fresh point that happens to coincide, and
//!   then both arms converge (`result = okPoint` carries the same
//!   geometry). The residual edge (a value-coincident PARTIAL-shove
//!   corner would ADVANCE in Rust where Java fails the segment) is
//!   unreachable through the sampled shorten point, which is strictly
//!   interior — banked in SEAM, revisit if a corpus compare ever
//!   shows a one-segment decision flip.
//! * **`pinEdgeToTurnDist` state parity.** The save / `set(-1)` /
//!   restore trio (`:140-141`, `:447`) ports as-is — including the
//!   restore NOT being in a finally (the single-point arm returns
//!   BEFORE the save; the FAIL `break` falls through to the restore).
//!   No Rust reader exists yet (the field is set by the pin-connection
//!   pull-tight path, T15) — the trio is state parity, not behavior.
//! * **Java-crash mappings.** A locator item with EMPTY corners
//!   crashes Java (`corners[0]` AIOOBE, `:66`); the port answers
//!   `false`/`None`. A null `lastCorner` at the final via (no segment
//!   ever inserted) crashes Java through `ForcedViaInserter` when the
//!   layers differ; the same-layer short-circuit fires first, so the
//!   port mirrors the short-circuit and maps the crash to `None`.
//! * **`clearanceClassIndex()` of an unresolvable pin** reads `-1` in
//!   the FAIL diagnostic (Java's field is never null on a live pin).
//! * **`formatPoint`'s Float branch** is unreachable for the ported
//!   callers (all polyline corners are `IntPoint`s); the placeholder
//!   string makes no parity claim.

use epic_board::board::Board;
use epic_board::contacts;
use epic_board::forced_via_inserter;
use epic_board::id::ItemId;
use epic_board::items::ItemData;
use epic_board::normalize_all::normalize_traces_of_net;
use epic_board::routing_board_insert::{
    PullTightSeam, connect_to_trace, insert_forced_trace_polyline, insert_forced_trace_segment,
};
use epic_board::routing_board_search::check_trace_segment_points;
use epic_board::rules_surf::ViaInfo;
use epic_board::time_limit::TimeLimit;
use epic_board::trace_ops::{contains_net, get_trace_tail, remove_item_through_repository};
use epic_board::tree_manager::SearchTreeManager;
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;

use crate::control::{AngleRestriction, AutorouteControl};
use crate::path::locator::{FoundConnectionLocator, ResultItem, calculate_additional_corner};

// ---------------------------------------------------------------------------
// the event sink
// ---------------------------------------------------------------------------

/// The pluggable face of the Java `FRLogger` calls in
/// `FoundConnectionInserter` (`trace`/`debug`/`warn` plus the
/// `isTraceEnabled` read the stub-cleanup row makes). Production runs
/// the [`NullSink`]; captures and pins collect with a recording sink.
/// Log-only by construction: no method returns data the algorithm
/// could branch on (the bug-118 convention).
pub trait InserterEventSink {
    /// Java `FRLogger.trace(String)` — the one-arg row.
    fn trace(&mut self, row: &str);
    /// Java `FRLogger.debug(String)`.
    fn debug(&mut self, message: &str);
    /// Java `FRLogger.warn(String)`.
    fn warn(&mut self, message: &str);
    /// Java `FRLogger.isTraceEnabled()` — read ONCE, into the
    /// `compare_trace_stub_cleanup` row's `trace_enabled=` field.
    fn is_trace_enabled(&self) -> bool;
}

/// The production sink — every row is dropped. The ported code paths
/// still run their gates and build their row strings (the strings are
/// the parity surface; dropping them here costs one dead allocation
/// per row on the NullSink face).
pub struct NullSink;

impl InserterEventSink for NullSink {
    fn trace(&mut self, _row: &str) {}
    fn debug(&mut self, _message: &str) {}
    fn warn(&mut self, _message: &str) {}
    fn is_trace_enabled(&self) -> bool {
        false
    }
}

/// The capture sink — prepends the level so a mixed stream stays
/// greppable; `is_trace_enabled` answers true (the Java capture runs
/// with tracing enabled, and the `trace_enabled=` row field must read
/// `true` to match).
#[cfg(test)]
#[derive(Default)]
pub struct CaptureSink {
    pub rows: Vec<String>,
}

#[cfg(test)]
impl InserterEventSink for CaptureSink {
    fn trace(&mut self, row: &str) {
        self.rows.push(format!("TRACE {row}"));
    }
    fn debug(&mut self, message: &str) {
        self.rows.push(format!("DEBUG {message}"));
    }
    fn warn(&mut self, message: &str) {
        self.rows.push(format!("WARN {message}"));
    }
    fn is_trace_enabled(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// string + mapping helpers
// ---------------------------------------------------------------------------

/// Java `Point.toString()` — `IntPoint` overrides with `(x,y)`
/// (`IntPoint.java:401-403`); `FloatPoint` has no plain override, so
/// Java would print the default `Object.toString()` — unreachable for
/// the ported callers (all polyline corners are `IntPoint`s).
fn point_str(point: &Point) -> String {
    match point {
        Point::Int(p) => format!("({},{})", p.x, p.y),
        Point::Rational(_) => {
            // Java RationalPoint overrides toString with its rational
            // coordinates — unreachable for the ported callers (all
            // polyline corners are IntPoints); the placeholder makes
            // no parity claim.
            "(rational-point)".to_string()
        }
    }
}

/// Java `formatPoint(Point)` (`:113-121`): null → `"null"`, IntPoint
/// → `(x,y)`, else `toString()`.
fn point_opt_str(point: Option<&Point>) -> String {
    point.map(point_str).unwrap_or_else(|| "null".into())
}

/// Java `Polyline.firstCorner()` in a `formatPoint` context.
fn polyline_first_str(polyline: &Polyline) -> String {
    point_opt_str(polyline.first_corner().as_ref())
}

/// Java `Polyline.lastCorner()` in a `formatPoint` context.
fn polyline_last_str(polyline: &Polyline) -> String {
    point_opt_str(polyline.last_corner().as_ref())
}

/// Java `IntPoint.toString()` — the `ResultItem.corners` rows.
fn corner_str(corner: &IntPoint) -> String {
    format!("({},{})", corner.x, corner.y)
}

/// The locator's item keys are item ids widened to u64 (Java stores
/// the `Item` itself; `SearchTreeManager::item_of_key` is the same
/// narrowing, crate-private in epic-board — replicated here).
fn key_to_id(key: u64) -> Option<ItemId> {
    u32::try_from(key).ok().map(ItemId::new)
}

/// The board rules' angle restriction ([`epic_board::rules_surf::
/// AngleRestriction`]) into the router-local enum the locator's
/// [`calculate_additional_corner`] consumes — the two enums carry the
/// same three variants (Java has ONE `AngleRestriction`).
pub(crate) fn router_angle_restriction(
    board_restriction: epic_board::rules_surf::AngleRestriction,
) -> AngleRestriction {
    match board_restriction {
        epic_board::rules_surf::AngleRestriction::None => AngleRestriction::None,
        epic_board::rules_surf::AngleRestriction::NinetyDegree => AngleRestriction::NinetyDegree,
        epic_board::rules_surf::AngleRestriction::FortyfiveDegree => {
            AngleRestriction::FortyfiveDegree
        }
    }
}

/// The five-arg twin of a `compare_trace_insert_segment_raw` row, in
/// the ForcedInsertProbe flatten: `[<method>] [<operation>] <payload>:
/// <impactedItems>` with the empty `Point[0]` trailing array.
/// `micro_neckdown` is present ONLY for the ADVANCE and FAIL rows
/// (the Java VIOLATION_CORRECTED literal has no such field).
#[allow(clippy::too_many_arguments)] // the flattened Java literal
fn insert_segment_twin(
    net: i32,
    layer: i32,
    i: usize,
    from_corner_no: i32,
    decision: &str,
    neckdown: bool,
    micro_neckdown: Option<bool>,
    ok_point: &str,
    first: &str,
    last: &str,
) -> String {
    let micro = micro_neckdown
        .map(|value| format!(", micro_neckdown={value}"))
        .unwrap_or_default();
    format!(
        "[FoundConnectionInserter.insert_trace] [compare_trace_insert_segment] \
         net={net}, layer={layer}, i={i}, fromCornerNo={from_corner_no}, \
         decision={decision}, neckdown={neckdown}{micro}, okPoint={ok_point}, \
         first={first}, last={last}: Net #{net}"
    )
}

// ---------------------------------------------------------------------------
// the inserter
// ---------------------------------------------------------------------------

/// Java `FoundConnectionInserter` — built by [`get_instance`], which
/// owns the whole insertion walk. The Java private fields
/// (`firstCorner`/`lastCorner`) plus the borrowed context (`board`,
/// `ctrl`) live here; the borrows rebase per call so the instance
/// stays usable across the epic-board surfaces.
pub struct FoundConnectionInserter<'a, S: PullTightSeam, E: InserterEventSink> {
    manager: &'a mut SearchTreeManager,
    board: &'a mut Board,
    seam: &'a mut S,
    ctrl: &'a AutorouteControl,
    sink: &'a mut E,
    /// Java `lastCorner` (`:27`) — null until a segment or single
    /// point lands.
    last_corner: Option<IntPoint>,
    /// Java `firstCorner` (`:28`).
    first_corner: Option<IntPoint>,
}

/// Java `FoundConnectionInserter.getInstance` (`:40-111`). `None`
/// connection mirrors Java's null (the maze search found nothing);
/// `None` return = "the insertion did not succeed" (Java null).
#[allow(clippy::too_many_arguments)] // the Java surface, kept 1:1
pub fn get_instance<'a, S: PullTightSeam, E: InserterEventSink>(
    connection: Option<&FoundConnectionLocator>,
    manager: &'a mut SearchTreeManager,
    board: &'a mut Board,
    seam: &'a mut S,
    ctrl: &'a AutorouteControl,
    sink: &'a mut E,
) -> Option<FoundConnectionInserter<'a, S, E>> {
    // Java `:42`: `connection == null || connectionItems == null` —
    // the Vec cannot be null in Rust; the Option carries the null.
    let connection = connection?;

    let mut instance = FoundConnectionInserter {
        manager,
        board,
        seam,
        ctrl,
        sink,
        last_corner: None,
        first_corner: None,
    };

    // Java `:45`: `int currentLayer = connection.targetLayer;`
    let mut current_layer = connection.target_layer;

    for current_new_item in &connection.connection_items {
        // Java `:48-65`: the unconditional per-item row. The empty-
        // corners AIOOBE face is guarded below (Java's start/end
        // null-guard only feeds the ROW, the insertVia still indexes).
        let start_corner = current_new_item.corners.first();
        let end_corner = current_new_item.corners.last();
        instance.sink.trace(&format!(
            "compare_trace_connection_item_raw net={}, item_layer={}, cornerCount={}, start={}, end={}",
            ctrl.net_number,
            current_new_item.layer,
            current_new_item.corners.len(),
            start_corner.map(corner_str).as_deref().unwrap_or("null"),
            end_corner.map(corner_str).as_deref().unwrap_or("null"),
        ));

        // Java `:66`: `newInstance.insertVia(currentNewItem.corners[0],
        // ...)` — an empty corner list crashes Java; the port bails.
        let first_corner_of_item = *start_corner?;
        if !instance.insert_via(
            &Point::Int(first_corner_of_item),
            current_layer,
            current_new_item.layer,
        ) {
            return None;
        }
        current_layer = current_new_item.layer;
        if !instance.insert_trace(current_new_item) {
            return None;
        }
    }

    // Java `:74-76`: the final via down to the start layer, from the
    // LAST inserted corner. `lastCorner` is null when no item ran —
    // the same-layer short-circuit fires first in Java too (null is
    // never dereferenced); a layer change would NPE → the port
    // answers None (see the module docs).
    match instance.last_corner {
        Some(last) => {
            if !instance.insert_via(&Point::Int(last), current_layer, connection.start_layer) {
                return None;
            }
        }
        None => {
            if current_layer != connection.start_layer {
                return None; // Java NPE face, mapped
            }
        }
    }

    // Java `:77-91`: the TARGET splice — `targetItem instanceof
    // PolylineTrace` (every trace is a polyline trace in the port's
    // model), half width of the START layer.
    if let Some(target_key) = connection.target_item
        && let Some(to_trace) = key_to_id(target_key)
    {
        let is_trace = instance
            .board
            .get(to_trace)
            .is_some_and(|entry| matches!(entry.data, ItemData::Trace { .. }));
        if is_trace {
            match instance.first_corner {
                Some(first) => {
                    let _ = connect_to_trace(
                        instance.manager,
                        instance.board,
                        &first,
                        to_trace,
                        ctrl.trace_half_width[connection.start_layer as usize],
                        ctrl.trace_clearance_class_index,
                    );
                }
                None => instance.sink.warn(&format!(
                    "FoundConnectionInserter: firstCorner is null for net #{}, \
                         skipping connect_to_trace for target item. \
                         This may indicate a degenerate route segment.",
                    ctrl.net_number
                )),
            }
        }
    }

    // Java `:92-106`: the START splice — cross-indexed half width of
    // the TARGET layer, from `lastCorner`.
    if let Some(start_key) = connection.start_item
        && let Some(to_trace) = key_to_id(start_key)
    {
        let is_trace = instance
            .board
            .get(to_trace)
            .is_some_and(|entry| matches!(entry.data, ItemData::Trace { .. }));
        if is_trace {
            match instance.last_corner {
                Some(last) => {
                    let _ = connect_to_trace(
                        instance.manager,
                        instance.board,
                        &last,
                        to_trace,
                        ctrl.trace_half_width[connection.target_layer as usize],
                        ctrl.trace_clearance_class_index,
                    );
                }
                None => instance.sink.warn(&format!(
                    "FoundConnectionInserter: lastCorner is null for net #{}, \
                         skipping connect_to_trace for start item. \
                         This may indicate a degenerate route segment.",
                    ctrl.net_number
                )),
            }
        }
    }

    // Java `:108`: the per-net normalize
    // (`BasicBoard.normalizeTraces(int)` →
    // `epic_board::normalize_all::normalize_traces_of_net`).
    normalize_traces_of_net(instance.manager, instance.board, ctrl.net_number);

    Some(instance)
}

impl<S: PullTightSeam, E: InserterEventSink> FoundConnectionInserter<'_, S, E> {
    /// Java `insertTrace` (`:127-453`). Inserts the item's polyline
    /// segment-by-segment with the shove/neckdown/micro-neckdown
    /// ladder; updates `first_corner`/`last_corner` and returns
    /// whether every segment advanced.
    fn insert_trace(&mut self, trace: &ResultItem) -> bool {
        if trace.corners.is_empty() {
            // Java crashes (the `corners[0]` default at `:449`); the
            // locator never produces empty corner lists — the port
            // answers false (module docs).
            return false;
        }
        if trace.corners.len() == 1 {
            // Java `:128-136`: the single-point arm — BOTH corners
            // set so `connect_to_trace` is not called with null; the
            // pinEdgeToTurnDist save/restore is never reached.
            if self.first_corner.is_none() {
                self.first_corner = Some(trace.corners[0]);
            }
            self.last_corner = Some(trace.corners[0]);
            return true;
        }

        // Java `:138-141`: switch off the pin-connection correction
        // while inserting line for line (restore at `:447`, NOT in a
        // finally — the FAIL break falls through to it).
        let saved_edge_to_turn_dist = self.board.rules().pin_edge_to_turn_dist;
        self.board.rules_mut().pin_edge_to_turn_dist = -1.0;

        // Java `:143-165`: look for pins at the START and END corner
        // in case neckdown is necessary. `currentEndCorner` is
        // REASSIGNED to the last corner at the end of EVERY i-pass
        // (the i==0 pass reads corners[0], the i==1 pass the last).
        let mut start_pin: Option<ItemId> = None;
        let mut end_pin: Option<ItemId> = None;
        if self.ctrl.with_neckdown {
            let mut current_end_corner = Point::Int(trace.corners[0]);
            for i in 0..2 {
                let picked = self
                    .manager
                    .pick_items(self.board, &current_end_corner, trace.layer);
                for current_item in picked {
                    // Java casts to Pin unconditionally (the filter
                    // guarantees PINS); the port keeps the shape gate.
                    let is_pin = self
                        .board
                        .get(current_item)
                        .is_some_and(|entry| matches!(entry.data, ItemData::Pin { .. }));
                    if !is_pin {
                        continue;
                    }
                    let nets = self
                        .board
                        .get(current_item)
                        .map(|entry| entry.nets.clone())
                        .unwrap_or_default();
                    let center_matches =
                        self.board.drill_center(current_item) == Some(current_end_corner.clone());
                    if contains_net(&nets, self.ctrl.net_number) && center_matches {
                        if i == 0 {
                            start_pin = Some(current_item);
                        } else {
                            end_pin = Some(current_item);
                        }
                    }
                }
                current_end_corner = Point::Int(trace.corners[trace.corners.len() - 1]);
            }
        }

        let net_numbers = [self.ctrl.net_number];

        // Java `:169-176`: the segment walk.
        let mut from_corner_no: i32 = 0;
        let mut result = true;
        for i in 1..trace.corners.len() {
            let current_corner_arr: Vec<Point> = trace.corners[from_corner_no as usize..=i]
                .iter()
                .map(|corner| Point::Int(*corner))
                .collect();
            let insert_polyline = Polyline::from_points(&current_corner_arr);

            // Java `:174-188`: the id-burn bookkeeping + the forced
            // polyline insertion (tidyWidth = Integer.MAX_VALUE, the
            // pull-tight seam decides whether it runs). T12 POINTER:
            // the BoardHistory id-watermark work (T12's pass/stagnation
            // bookkeeping — the bug-150 `alloc_id` burn in
            // `check_polyline_trace` is the other watermark site) keys
            // off these maxGeneratedId deltas; the `delta=` row below
            // is the capture-visible surface of that watermark.
            let max_item_id_before = self.board.max_generated_id();
            let ok_point: Option<Point> = insert_forced_trace_polyline(
                self.manager,
                self.board,
                self.seam,
                &insert_polyline,
                self.ctrl.trace_half_width[trace.layer as usize],
                trace.layer,
                Some(&net_numbers),
                self.ctrl.trace_clearance_class_index,
                self.ctrl.max_shove_trace_recursion_depth,
                self.ctrl.max_shove_via_recursion_depth,
                self.ctrl.max_spring_over_recursion_depth,
                i32::MAX,
                self.ctrl.pull_tight_accuracy,
                true,
                None::<&TimeLimit>,
            );
            let max_item_id_after = self.board.max_generated_id();
            self.sink.trace(&format!(
                "compare_trace_insert_segment_ids net={}, i={}, maxItemIdBefore={}, \
                 maxItemIdAfter={}, delta={}",
                self.ctrl.net_number,
                i,
                max_item_id_before,
                max_item_id_after,
                i64::from(max_item_id_after) - i64::from(max_item_id_before),
            ));

            // Java `:201-217`: the neckdown + micro-neckdown arms.
            // Both hinge on `okPoint != insertPolyline.lastCorner()` —
            // Java REFERENCE equality, blessed as value equality (see
            // the module docs). NOTE the micro arm has NO null guard:
            // `null != lastCorner()` is true, so a null okPoint still
            // enters (the callee substitutes the target point).
            let mut neckdown_inserted = false;
            let mut micro_neckdown_inserted = false;
            let reached_last = insert_polyline.last_corner().as_ref() == ok_point.as_ref();
            if ok_point.is_some()
                && !reached_last
                && self.ctrl.with_neckdown
                && current_corner_arr.len() == 2
                && let Some(ok) = ok_point.as_ref()
            {
                neckdown_inserted = self.insert_neckdown(
                    ok,
                    &current_corner_arr[1],
                    trace.layer,
                    start_pin,
                    end_pin,
                );
            }
            if !neckdown_inserted
                && !reached_last
                && self.ctrl.is_fanout
                && current_corner_arr.len() == 2
            {
                micro_neckdown_inserted = self.insert_fanout_micro_neckdown(
                    ok_point.clone(),
                    &current_corner_arr[1],
                    trace.layer,
                    &net_numbers,
                    start_pin,
                    end_pin,
                );
            }

            // The row strings (built once; raw + twin share them).
            let net = self.ctrl.net_number;
            let layer = trace.layer;
            let ok_str = point_opt_str(ok_point.as_ref());
            let first_str = polyline_first_str(&insert_polyline);
            let last_str = polyline_last_str(&insert_polyline);

            if reached_last || neckdown_inserted || micro_neckdown_inserted {
                // Java `:218-243`: ADVANCE.
                from_corner_no = i as i32;
                self.sink.trace(&format!(
                    "compare_trace_insert_segment_raw net={net}, layer={layer}, i={i}, \
                     fromCornerNo={from_corner_no}, decision=ADVANCE, \
                     neckdown={neckdown_inserted}, micro_neckdown={micro_neckdown_inserted}, \
                     okPoint={ok_str}, first={first_str}, last={last_str}"
                ));
                self.sink.trace(&insert_segment_twin(
                    net,
                    layer,
                    i,
                    from_corner_no,
                    "ADVANCE",
                    neckdown_inserted,
                    Some(micro_neckdown_inserted),
                    &ok_str,
                    &first_str,
                    &last_str,
                ));
            } else if insert_polyline.first_corner().as_ref() == ok_point.as_ref()
                && i != trace.corners.len() - 1
            {
                // Java `:244-280`: VIOLATION_CORRECTED — the spring
                // over may have failed; repeating with more distant
                // corners may correct it. `--fromCornerNo` only on the
                // FIRST correction (a 2-corner window). The row has no
                // micro_neckdown field, and the `violation corrected`
                // trace is UNCONDITIONAL.
                if from_corner_no > 0 && current_corner_arr.len() < 3 {
                    from_corner_no -= 1;
                }
                self.sink
                    .trace("FoundConnectionInserter: violation corrected");
                self.sink.trace(&format!(
                    "compare_trace_insert_segment_raw net={net}, layer={layer}, i={i}, \
                     fromCornerNo={from_corner_no}, decision=VIOLATION_CORRECTED, \
                     neckdown={neckdown_inserted}, okPoint={ok_str}, first={first_str}, \
                     last={last_str}"
                ));
                self.sink.trace(&insert_segment_twin(
                    net,
                    layer,
                    i,
                    from_corner_no,
                    "VIOLATION_CORRECTED",
                    neckdown_inserted,
                    None,
                    &ok_str,
                    &first_str,
                    &last_str,
                ));
            } else {
                // Java `:281-344`: FAIL — debug + fanout diag + both
                // rows, then `result = false; break;` (which STILL
                // reaches the pinEdgeToTurnDist restore below).
                self.sink.debug(&format!(
                    "FoundConnectionInserter: insert trace failed for net #{net} at corner \
                     {i}/{} on layer {layer}, trace width: {}, from corner: {from_corner_no}, \
                     okPoint: {ok_str}, target: {last_str}",
                    trace.corners.len() - 1,
                    self.ctrl.trace_half_width[layer as usize],
                ));
                let start_pin_class = start_pin
                    .and_then(|pin| self.board.item_clearance_class(pin))
                    .unwrap_or(-1);
                let end_pin_class = end_pin
                    .and_then(|pin| self.board.item_clearance_class(pin))
                    .unwrap_or(-1);
                self.trace_fanout_diagnostic(
                    "trace_insert_failed",
                    &format!(
                        "layer={layer}, corner_index={i}, from_corner_index={from_corner_no}, \
                         traceHalfWidth={}, traceClearanceClass={}, \
                         start_pin_clearance_class={start_pin_class}, \
                         end_pin_clearance_class={end_pin_class}, okPoint={ok_str}, \
                         target={last_str}",
                        self.ctrl.trace_half_width[layer as usize],
                        self.ctrl.trace_clearance_class_index,
                    ),
                );
                self.sink.trace(&format!(
                    "compare_trace_insert_segment_raw net={net}, layer={layer}, i={i}, \
                     fromCornerNo={from_corner_no}, decision=FAIL, \
                     neckdown={neckdown_inserted}, micro_neckdown={micro_neckdown_inserted}, \
                     okPoint={ok_str}, first={first_str}, last={last_str}"
                ));
                self.sink.trace(&insert_segment_twin(
                    net,
                    layer,
                    i,
                    from_corner_no,
                    "FAIL",
                    neckdown_inserted,
                    Some(micro_neckdown_inserted),
                    &ok_str,
                    &first_str,
                    &last_str,
                ));
                result = false;
                break;
            }
        }

        // Java `:405-431`: the stub cleanup — every corner except the
        // last is a candidate tail point.
        let mut removed_trace_stubs = 0;
        let net = self.ctrl.net_number;
        for (i, corner) in trace.corners[..trace.corners.len() - 1].iter().enumerate() {
            let corner_point = Point::Int(*corner);
            let Some(stub) = get_trace_tail(
                self.manager,
                self.board,
                &corner_point,
                trace.layer,
                &net_numbers,
            ) else {
                continue;
            };
            let stub_first = self
                .board
                .trace_polyline(stub)
                .and_then(|polyline| polyline.first_corner());
            let stub_last = self
                .board
                .trace_polyline(stub)
                .and_then(|polyline| polyline.last_corner());
            let start_count = contacts::start_contacts(self.manager, self.board, stub).len();
            let end_count = contacts::end_contacts(self.manager, self.board, stub).len();
            self.sink.trace(&format!(
                "compare_trace_stub_found net={net}, corner_idx={i}, corner={}, \
                 stub_id={}, stub_first={}, stub_last={}, startContacts={start_count}, \
                 endContacts={end_count}",
                corner_str(corner),
                stub.get(),
                stub_first
                    .map(|point| point_str(&point))
                    .as_deref()
                    .unwrap_or("null"),
                stub_last
                    .map(|point| point_str(&point))
                    .as_deref()
                    .unwrap_or("null"),
            ));
            remove_item_through_repository(self.manager, self.board, stub);
            removed_trace_stubs += 1;
        }
        let trace_enabled = self.sink.is_trace_enabled();
        self.sink.trace(&format!(
            "[FoundConnectionInserter.insert_trace] [compare_trace_stub_cleanup] \
             net={net}, layer={}, removed_stubs={removed_trace_stubs}, \
             trace_enabled={trace_enabled}: Net #{net}",
            trace.layer,
        ));

        // Java `:447-452`: restore, then the corner defaults.
        self.board.rules_mut().pin_edge_to_turn_dist = saved_edge_to_turn_dist;
        if self.first_corner.is_none() {
            self.first_corner = Some(trace.corners[0]);
        }
        self.last_corner = trace.corners.last().copied();
        result
    }

    /// Java `insertNeckdown` (`:525-537`). NOTE the SWAPPED corners in
    /// the start-pin arm (`tryNeckDown(toCorner, fromCorner, ...)`),
    /// and that a FAILED start-pin arm falls INTO the end-pin arm (no
    /// else) — the end-pin verdict is returned even then.
    fn insert_neckdown(
        &mut self,
        from_corner: &Point,
        to_corner: &Point,
        layer: i32,
        start_pin: Option<ItemId>,
        end_pin: Option<ItemId>,
    ) -> bool {
        if let Some(start_pin) = start_pin {
            let ok_point = self.try_neck_down(to_corner, from_corner, layer, start_pin, true);
            if ok_point.as_ref() == Some(from_corner) {
                return true;
            }
        }
        if let Some(end_pin) = end_pin {
            let ok_point = self.try_neck_down(from_corner, to_corner, layer, end_pin, false);
            return ok_point.as_ref() == Some(to_corner);
        }
        false
    }

    /// Java `tryNeckDown` (`:539-676`): the 4-segment neckdown ladder
    /// around a pin. `at_start` is UNUSED in Java — kept for the 1:1
    /// signature. Returns the final neckdown segment's ok point.
    #[allow(clippy::too_many_arguments)] // the inner Java call's args
    fn try_neck_down(
        &mut self,
        from_corner: &Point,
        to_corner: &Point,
        layer: i32,
        pin: ItemId,
        at_start: bool,
    ) -> Option<Point> {
        let _ = at_start; // Java's parameter is never read
        if !self.board.pin_is_on_layer(pin, layer) {
            return None;
        }
        // Java memoized `getCenter()` — never null on a live pin.
        let pin_center = self
            .board
            .drill_center(pin)
            .expect("live pin center (Java memo never null)");
        let pin_clearance_class = self
            .board
            .item_clearance_class(pin)
            .expect("live pin clearance class (Java field never null)");
        let current_clearance = f64::from(self.board.rules().clearance.get_value_opt(
            self.ctrl.trace_clearance_class_index,
            pin_clearance_class,
            layer,
            true,
        ));
        let pin_max_width = self.board.pin_max_width_on_layer(pin, layer);
        let pin_neck_down_distance = 2.0 * (0.5 * pin_max_width + current_clearance);
        if pin_center.to_float().distance(&to_corner.to_float()) >= pin_neck_down_distance {
            return None;
        }

        let neck_down_halfwidth = self.board.pin_trace_neckdown_halfwidth(pin, layer);
        if neck_down_halfwidth >= self.ctrl.trace_half_width[layer as usize] {
            return None;
        }

        let float_from_corner = from_corner.to_float();
        let float_to_corner = to_corner.to_float();

        let tolerance: i32 = 2;

        let net_numbers = [self.ctrl.net_number];

        let mut ok_length = check_trace_segment_points(
            self.manager,
            self.board,
            from_corner,
            to_corner,
            layer,
            &net_numbers,
            self.ctrl.trace_half_width[layer as usize],
            self.ctrl.trace_clearance_class_index,
            true,
        );
        if ok_length >= f64::from(i32::MAX) {
            return Some(from_corner.clone());
        }
        ok_length -= f64::from(tolerance);
        let mut neck_down_end_point: Point;
        if ok_length <= f64::from(tolerance) {
            neck_down_end_point = from_corner.clone();
        } else {
            let float_neck_down_end_point =
                float_from_corner.change_length(&float_to_corner, ok_length);
            let rounded_end_point = float_neck_down_end_point.round();
            neck_down_end_point = Point::Int(rounded_end_point);
            // add a corner in case neckDownEndPoint is not exactly on
            // the line from fromCorner to toCorner — the `>=` tie is
            // the Java literal.
            let horizontal_first = (float_from_corner.x - float_neck_down_end_point.x).abs()
                >= (float_from_corner.y - float_neck_down_end_point.y).abs();
            let angle_restriction =
                router_angle_restriction(self.board.rules().trace_angle_restriction);
            let mut add_corner = calculate_additional_corner(
                &float_from_corner,
                &float_neck_down_end_point,
                horizontal_first,
                angle_restriction,
            )
            .round();
            let full_width = self.ctrl.trace_half_width[layer as usize];
            let current_ok_point = insert_forced_trace_segment(
                self.manager,
                self.board,
                self.seam,
                from_corner,
                &Point::Int(add_corner),
                full_width,
                layer,
                Some(&net_numbers),
                self.ctrl.trace_clearance_class_index,
                self.ctrl.max_shove_trace_recursion_depth,
                self.ctrl.max_shove_via_recursion_depth,
                self.ctrl.max_spring_over_recursion_depth,
                i32::MAX,
                self.ctrl.pull_tight_accuracy,
                true,
                None,
            );
            // Java `currentOkPoint != addCorner` — reference equality;
            // a null (failed) insert fails the check in Java too.
            if current_ok_point.as_ref() != Some(&Point::Int(add_corner)) {
                return Some(from_corner.clone());
            }
            let current_ok_point = insert_forced_trace_segment(
                self.manager,
                self.board,
                self.seam,
                &Point::Int(add_corner),
                &neck_down_end_point,
                full_width,
                layer,
                Some(&net_numbers),
                self.ctrl.trace_clearance_class_index,
                self.ctrl.max_shove_trace_recursion_depth,
                self.ctrl.max_shove_via_recursion_depth,
                self.ctrl.max_spring_over_recursion_depth,
                i32::MAX,
                self.ctrl.pull_tight_accuracy,
                true,
                None,
            );
            if current_ok_point.as_ref() != Some(&neck_down_end_point) {
                return Some(from_corner.clone());
            }
            add_corner = calculate_additional_corner(
                &float_neck_down_end_point,
                &float_to_corner,
                !horizontal_first,
                angle_restriction,
            )
            .round();
            if Point::Int(add_corner) != *to_corner {
                let current_ok_point = insert_forced_trace_segment(
                    self.manager,
                    self.board,
                    self.seam,
                    &neck_down_end_point,
                    &Point::Int(add_corner),
                    full_width,
                    layer,
                    Some(&net_numbers),
                    self.ctrl.trace_clearance_class_index,
                    self.ctrl.max_shove_trace_recursion_depth,
                    self.ctrl.max_shove_via_recursion_depth,
                    self.ctrl.max_spring_over_recursion_depth,
                    i32::MAX,
                    self.ctrl.pull_tight_accuracy,
                    true,
                    None,
                );
                if current_ok_point.as_ref() != Some(&Point::Int(add_corner)) {
                    return Some(from_corner.clone());
                }
                neck_down_end_point = Point::Int(add_corner);
            }
        }

        insert_forced_trace_segment(
            self.manager,
            self.board,
            self.seam,
            &neck_down_end_point,
            to_corner,
            neck_down_halfwidth,
            layer,
            Some(&net_numbers),
            self.ctrl.trace_clearance_class_index,
            self.ctrl.max_shove_trace_recursion_depth,
            self.ctrl.max_shove_via_recursion_depth,
            self.ctrl.max_spring_over_recursion_depth,
            i32::MAX,
            self.ctrl.pull_tight_accuracy,
            true,
            None,
        )
    }

    /// Java `insertFanoutMicroNeckdown` (`:455-523`): try shrinking
    /// half widths (the two pin neckdown widths, then 3/4, 3/5, 1/2 of
    /// the base — insertion-ordered dedup, the `LinkedHashSet`) until
    /// one reaches the target. Java's `okPoint` may be NULL here (the
    /// caller's arm has no null guard) — the port carries the Option.
    fn insert_fanout_micro_neckdown(
        &mut self,
        ok_point: Option<Point>,
        target_point: &Point,
        layer: i32,
        net_numbers: &[i32],
        start_pin: Option<ItemId>,
        end_pin: Option<ItemId>,
    ) -> bool {
        let from_point = ok_point.unwrap_or_else(|| target_point.clone());
        if from_point == *target_point {
            // Java `fromPoint == null || targetPoint == null ||
            // fromPoint.equals(targetPoint)` — both nulls are
            // impossible here (fromPoint defaults to the target, the
            // target is a live corner), the equality is the gate.
            return false;
        }
        let base_half_width = self.ctrl.trace_half_width[layer as usize];
        // Java `LinkedHashSet<Integer>` — insertion order, dedup.
        let mut candidate_half_widths: Vec<i32> = Vec::new();
        if let Some(pin) = start_pin
            && self.board.pin_is_on_layer(pin, layer)
        {
            let width = self.board.pin_trace_neckdown_halfwidth(pin, layer);
            if !candidate_half_widths.contains(&width) {
                candidate_half_widths.push(width);
            }
        }
        if let Some(pin) = end_pin
            && self.board.pin_is_on_layer(pin, layer)
        {
            let width = self.board.pin_trace_neckdown_halfwidth(pin, layer);
            if !candidate_half_widths.contains(&width) {
                candidate_half_widths.push(width);
            }
        }
        for fraction in [
            (base_half_width * 3) / 4,
            (base_half_width * 3) / 5,
            base_half_width / 2,
        ] {
            let width = 1.max(fraction);
            if !candidate_half_widths.contains(&width) {
                candidate_half_widths.push(width);
            }
        }

        for &candidate_half_width in &candidate_half_widths {
            if candidate_half_width <= 0 || candidate_half_width >= base_half_width {
                continue;
            }
            let candidate_ok_point = insert_forced_trace_segment(
                self.manager,
                self.board,
                self.seam,
                &from_point,
                target_point,
                candidate_half_width,
                layer,
                Some(net_numbers),
                self.ctrl.trace_clearance_class_index,
                self.ctrl.max_shove_trace_recursion_depth,
                self.ctrl.max_shove_via_recursion_depth,
                self.ctrl.max_spring_over_recursion_depth,
                i32::MAX,
                self.ctrl.pull_tight_accuracy,
                true,
                None,
            );
            // Java `candidateOkPoint == targetPoint` — reference
            // equality, blessed as value equality (module docs).
            if candidate_ok_point.as_ref() == Some(target_point) {
                self.trace_fanout_diagnostic(
                    "trace_insert_micro_neckdown_success",
                    &format!(
                        "layer={layer}, candidate_half_width={candidate_half_width}, \
                         baseHalfWidth={base_half_width}, traceClearanceClass={}, from={}, to={}",
                        self.ctrl.trace_clearance_class_index,
                        point_str(&from_point),
                        point_str(target_point),
                    ),
                );
                return true;
            }
        }
        self.trace_fanout_diagnostic(
            "trace_insert_micro_neckdown_failed",
            &format!(
                "layer={layer}, baseHalfWidth={base_half_width}, traceClearanceClass={}, \
                 from={}, to={}",
                self.ctrl.trace_clearance_class_index,
                point_str(&from_point),
                point_str(target_point),
            ),
        );
        false
    }

    /// Java `insertVia` (`:683-785`): the via-rule ordered scan (span
    /// gate, then `ForcedViaInserter.check`), then the insert.
    fn insert_via(&mut self, location: &Point, input_from_layer: i32, input_to_layer: i32) -> bool {
        if input_from_layer == input_to_layer {
            return true; // no via necessary
        }
        // sort the input layers
        let (from_layer, to_layer) = if input_from_layer < input_to_layer {
            (input_from_layer, input_to_layer)
        } else {
            (input_to_layer, input_from_layer)
        };
        let net_numbers = [self.ctrl.net_number];
        let net = self.ctrl.net_number;

        // Java `this.ctrl.viaRule.viaCount()` — the rule is
        // engine-provided (init_net); Java would NPE without it.
        let via_rule = self
            .ctrl
            .via_rule
            .as_ref()
            .expect("engine-provided via rule (Java would NPE at viaCount())");

        let mut selected_via_info: Option<ViaInfo> = None;
        let mut found_suitable_span = false;
        for &via_index in &via_rule.via_infos {
            // Clone the candidate to release the rules borrow across
            // the mut check call (a ViaInfo is a name + three ints).
            let current_via_info = self.board.rules().via_infos[via_index as usize].clone();
            let padstack = self
                .board
                .library()
                .padstack(current_via_info.padstack_no)
                .expect("via padstack of a live via rule");
            let (padstack_from_layer, padstack_to_layer) =
                (padstack.from_layer() as i32, padstack.to_layer());
            if padstack_from_layer > from_layer || padstack_to_layer < to_layer {
                continue;
            }
            found_suitable_span = true;
            if forced_via_inserter::check(
                self.manager,
                self.board,
                &current_via_info,
                location,
                &net_numbers,
                self.ctrl.max_shove_trace_recursion_depth,
                self.ctrl.max_shove_via_recursion_depth,
                Some(&self.ctrl.trace_half_width),
                self.ctrl.trace_clearance_class_index,
            ) {
                selected_via_info = Some(current_via_info);
                break;
            }
        }

        let Some(via_info) = selected_via_info else {
            if !found_suitable_span {
                self.sink.debug(&format!(
                    "FoundConnectionInserter: via mask not found for net #{net} covering \
                     layers {from_layer} to {to_layer}"
                ));
            } else {
                self.sink.debug(&format!(
                    "FoundConnectionInserter: via placement blocked by clearance/shove \
                     limits for net #{net}"
                ));
            }
            self.trace_fanout_diagnostic(
                "via_mask_not_found",
                &format!(
                    "fromLayer={from_layer}, toLayer={to_layer}, location={}, \
                     traceClearanceClass={}, viaClearanceClass={}, \
                     trace_half_width_from={}, trace_half_width_to={}",
                    point_opt_str(Some(location)),
                    self.ctrl.trace_clearance_class_index,
                    self.ctrl.via_clearance_class,
                    self.ctrl.trace_half_width[from_layer as usize],
                    self.ctrl.trace_half_width[to_layer as usize],
                ),
            );
            return false;
        };

        // insert the via
        if !forced_via_inserter::insert(
            self.manager,
            self.board,
            &via_info,
            location,
            &net_numbers,
            self.ctrl.trace_clearance_class_index,
            &self.ctrl.trace_half_width,
            self.ctrl.max_shove_trace_recursion_depth,
            self.ctrl.max_shove_via_recursion_depth,
        ) {
            self.sink.debug(&format!(
                "FoundConnectionInserter: forced via failed for net #{net}"
            ));
            let selected_padstack_name = self
                .board
                .library()
                .padstack(via_info.padstack_no)
                .map(|padstack| padstack.name.clone())
                .unwrap_or_default();
            self.trace_fanout_diagnostic(
                "forced_via_insert_failed",
                &format!(
                    "fromLayer={from_layer}, toLayer={to_layer}, location={}, \
                     selected_via_clearance_class={}, selected_via_padstack={}, \
                     traceClearanceClass={}, trace_half_width_from={}, \
                     trace_half_width_to={}",
                    point_opt_str(Some(location)),
                    via_info.clearance_class,
                    selected_padstack_name,
                    self.ctrl.trace_clearance_class_index,
                    self.ctrl.trace_half_width[from_layer as usize],
                    self.ctrl.trace_half_width[to_layer as usize],
                ),
            );
            return false;
        }
        true
    }

    /// Java `shouldTraceFanoutDiagnostics` (`:787-791`) +
    /// `traceFanoutDiagnostic` (`:793-806`): the hardcoded U27- gate.
    fn trace_fanout_diagnostic(&mut self, event: &str, message: &str) {
        let gated = self.ctrl.is_fanout
            && self
                .ctrl
                .fanout_start_pin_name
                .as_deref()
                .is_some_and(|name| name.starts_with("U27-"));
        if !gated {
            return;
        }
        let pin = self.ctrl.fanout_start_pin_name.as_deref().unwrap_or("");
        let net = self.ctrl.net_number;
        self.sink.trace(&format!(
            "FANOUT_DIAG event={event}, pin={pin}, net={net}, {message}"
        ));
    }
}

// ---------------------------------------------------------------------------
// structural tests (the full pin battery lands with the jar probes)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::RouterSettingsIr;
    use crate::test_util::parse;
    use epic_board::items::FixedState;
    use epic_board::routing_board_insert::NoPullTight;

    /// The T10c insert-world fixture — net 94, the y=300000 corridor,
    /// the foreign N002 trace seeded at x=501000 by the worlds below.
    fn parse_fixture() -> (SearchTreeManager, Board) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../harness/fixtures/locator-spike/t9_locator45.dsn");
        let text = std::fs::read_to_string(&path).expect("fixture present");
        // test_util::parse reinserts the tree items itself.
        parse(&text)
    }

    fn settings_ir(layer_count: usize) -> RouterSettingsIr {
        RouterSettingsIr {
            trace_costs: vec![crate::control::ExpansionCostFactor::default(); layer_count],
            via_costs: 1,
            vias_allowed: true,
            bend_costs: vec![0.0; layer_count],
            layer_active: vec![true; layer_count],
            automatic_neckdown: false,
            start_ripup_costs: 1,
            fanout: Default::default(),
        }
    }

    fn corner(x: i32, y: i32) -> IntPoint {
        IntPoint::new(x, y)
    }

    /// The happy path: a single same-layer 2-corner item inserts a
    /// net-94 trace with exactly the item's corners, sets both corner
    /// fields, and emits the full row ladder (connection-item row, the
    /// id-burn row, the ADVANCE raw+twin pair, the stub-cleanup twin).
    #[test]
    fn t11_get_instance_inserts_a_single_segment() {
        let (mut manager, mut board) = parse_fixture();
        let layer_count = board.layers().layers.len();
        let ctrl = AutorouteControl::new(&mut board, 94, &settings_ir(layer_count));
        // Seed the ANCHOR trace: the locator's TARGET item. A floating
        // segment has no contact at its head corner, and the stub
        // cleanup would faithfully eat it (Java's getTraceTail: an
        // endpoint with an EMPTY contact set is a tail) — real routes
        // are anchored, so the world anchors too.
        let anchor = epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(590000, 200000)),
                &Point::Int(corner(600000, 200000)),
            ),
            0,
            1500,
            &[94],
            0,
            FixedState::Unfixed,
        )
        .expect("anchor trace");
        let mut seam = NoPullTight;
        let mut sink = CaptureSink::default();

        let locator = FoundConnectionLocator {
            connection_items: vec![ResultItem {
                corners: vec![corner(600000, 200000), corner(620000, 200000)],
                layer: 0,
            }],
            start_item: None,
            start_layer: 0,
            target_item: Some(u64::from(anchor.get())),
            target_layer: 0,
        };
        let instance = get_instance(
            Some(&locator),
            &mut manager,
            &mut board,
            &mut seam,
            &ctrl,
            &mut sink,
        );
        let ok = instance.is_some();
        let rows_dump = sink.rows.join("\n");
        assert!(ok, "the insertion succeeded. rows:\n{rows_dump}");

        // The collinear anchor + new segment MERGE under the trailing
        // normalize (Java `normalizeTraces(net)` pull-tight) — the
        // pinned postcondition is the single merged trace.
        let found = board.iter_descending().any(|entry| {
            matches!(entry.data, ItemData::Trace { .. })
                && entry.nets == [94]
                && board.trace_polyline(entry.id).is_some_and(|polyline| {
                    polyline.first_corner() == Some(Point::Int(corner(590000, 200000)))
                        && polyline.last_corner() == Some(Point::Int(corner(620000, 200000)))
                })
        });
        assert!(found, "the merged net-94 trace spans anchor + segment");

        // The row ladder — exact strings (the parity surface).
        let joined = sink.rows.join("\n");
        assert!(
            joined.contains(
                "compare_trace_connection_item_raw net=94, item_layer=0, cornerCount=2, \
                 start=(600000,200000), end=(620000,200000)"
            ),
            "connection-item row missing:\n{joined}"
        );
        assert!(
            joined.contains("compare_trace_insert_segment_ids net=94, i=1, maxItemIdBefore="),
            "id-burn row missing"
        );
        assert!(
            joined.contains(
                "decision=ADVANCE, neckdown=false, micro_neckdown=false, \
                 okPoint=(620000,200000), first=(600000,200000), last=(620000,200000)"
            ),
            "ADVANCE row missing or wrong:\n{joined}"
        );
        assert!(
            joined.contains(
                "[FoundConnectionInserter.insert_trace] [compare_trace_insert_segment] \
                 net=94, layer=0, i=1, fromCornerNo=1, decision=ADVANCE, neckdown=false, \
                 micro_neckdown=false, okPoint=(620000,200000), first=(600000,200000), \
                 last=(620000,200000): Net #94"
            ),
            "ADVANCE twin missing or wrong:\n{joined}"
        );
        assert!(
            joined.contains(
                "[FoundConnectionInserter.insert_trace] [compare_trace_stub_cleanup] \
                 net=94, layer=0, removed_stubs=0, trace_enabled=true: Net #94"
            ),
            "stub-cleanup twin missing:\n{joined}"
        );
        // The trailing per-net normalize here is a fixpoint (nothing to
        // fold). The normalize suppression set's READ face is pinned by
        // direct injection in epic-board's normalize_all tests
        // (`suppressed_net_short_circuits_before_any_walk`); its WRITE
        // face (the add-on-cap latch) is BANKED in SEAM — no crafted
        // geometry reaches the 2000-iteration cap (spec review F1, was
        // falsely claimed pinned here).
    }

    /// Java `:225-227`: a same-layer insertVia short-circuits to true
    /// before any board access — no id burn, no rows.
    #[test]
    fn t11_insert_via_same_layer_short_circuit() {
        let (mut manager, mut board) = parse_fixture();
        let layer_count = board.layers().layers.len();
        let ctrl = AutorouteControl::new(&mut board, 94, &settings_ir(layer_count));
        let mut seam = NoPullTight;
        let mut sink = CaptureSink::default();
        let mut instance = FoundConnectionInserter {
            manager: &mut manager,
            board: &mut board,
            seam: &mut seam,
            ctrl: &ctrl,
            sink: &mut sink,
            last_corner: None,
            first_corner: None,
        };
        let id_before = instance.board.max_generated_id();
        assert!(instance.insert_via(&Point::Int(corner(500000, 300000)), 1, 1));
        assert_eq!(instance.board.max_generated_id(), id_before, "no via built");
        assert!(
            sink.rows.is_empty(),
            "no rows on the short-circuit: {:?}",
            sink.rows
        );
    }

    /// The FAIL face: a USER_FIXED blocker (unshovable) makes the
    /// segment walk fail — the FAIL decision, the debug + fanout rows,
    /// `false` as the verdict, AND the pinEdgeToTurnDist restore (the
    /// Java restore sits after the FAIL break, not in a finally).
    #[test]
    fn t11_insert_trace_fail_restores_pin_edge_to_turn_dist() {
        let (mut manager, mut board) = parse_fixture();
        // Seed the unshovable blocker mid-corridor (id-agnostic).
        epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            // A full-height wall: no endpoint lies within spring-over
            // reach of the corridor, so the forced insert must FAIL
            // (a short stub would be sprung around — observed).
            Polyline::from_two_corners(
                &Point::Int(corner(610000, -100000)),
                &Point::Int(corner(610000, 700000)),
            ),
            0,
            100,
            &[2],
            0,
            FixedState::UserFixed,
        )
        .expect("blocker trace");
        let layer_count = board.layers().layers.len();
        let mut ctrl = AutorouteControl::new(&mut board, 94, &settings_ir(layer_count));
        // The fanout gate ON with the U27- prefix — the FAIL diag row
        // must appear.
        ctrl.is_fanout = true;
        ctrl.fanout_start_pin_name = Some("U27-P1".to_string());
        let mut seam = NoPullTight;
        let mut sink = CaptureSink::default();
        // A sentinel the restore must bring back (Java saves/sets/-1/restores).
        board.rules_mut().pin_edge_to_turn_dist = 5.0;
        let mut instance = FoundConnectionInserter {
            manager: &mut manager,
            board: &mut board,
            seam: &mut seam,
            ctrl: &ctrl,
            sink: &mut sink,
            last_corner: None,
            first_corner: None,
        };
        let item = ResultItem {
            corners: vec![corner(600000, 200000), corner(620000, 200000)],
            layer: 0,
        };
        assert!(
            !instance.insert_trace(&item),
            "the blocked walk fails. rows:\n{}",
            sink.rows.join("\n")
        );
        let board_rules_value = instance.board.rules().pin_edge_to_turn_dist;
        assert_eq!(board_rules_value, 5.0, "the sentinel survives the restore");
        let last_corner_after = instance.last_corner;
        let joined = sink.rows.join("\n");
        assert!(
            joined.contains("decision=FAIL"),
            "a FAIL decision row missing:\n{joined}"
        );
        assert!(
            joined.contains(
                "FoundConnectionInserter: insert trace failed for net #94 at corner 1/1 \
                 on layer 0"
            ),
            "the FAIL debug missing:\n{joined}"
        );
        assert!(
            joined.contains("FANOUT_DIAG event=trace_insert_failed, pin=U27-P1, net=94,"),
            "the gated fanout diag missing:\n{joined}"
        );
        assert!(
            joined.contains("start_pin_clearance_class=-1, end_pin_clearance_class=-1"),
            "the null-pin clearance classes missing:\n{joined}"
        );
        // The corners: last_corner NOT updated to the failed walk's
        // tail? Java updates them UNCONDITIONALLY after the loop.
        // Java updates lastCorner UNCONDITIONALLY after the walk (even
        // on FAIL) — the tail is the item's last corner.
        assert_eq!(last_corner_after, Some(corner(620000, 200000)));
    }

    /// The fanout gate itself: the same FAIL world with the prefix NOT
    /// matching U27- emits NO FANOUT_DIAG row (but the debug stays).
    #[test]
    fn t11_fanout_gate_prefix() {
        let (mut manager, mut board) = parse_fixture();
        epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            // A full-height wall: no endpoint lies within spring-over
            // reach of the corridor, so the forced insert must FAIL
            // (a short stub would be sprung around — observed).
            Polyline::from_two_corners(
                &Point::Int(corner(610000, -100000)),
                &Point::Int(corner(610000, 700000)),
            ),
            0,
            100,
            &[2],
            0,
            FixedState::UserFixed,
        )
        .expect("blocker trace");
        let layer_count = board.layers().layers.len();
        let mut ctrl = AutorouteControl::new(&mut board, 94, &settings_ir(layer_count));
        ctrl.is_fanout = true;
        ctrl.fanout_start_pin_name = Some("U28-P1".to_string());
        let mut seam = NoPullTight;
        let mut sink = CaptureSink::default();
        let mut instance = FoundConnectionInserter {
            manager: &mut manager,
            board: &mut board,
            seam: &mut seam,
            ctrl: &ctrl,
            sink: &mut sink,
            last_corner: None,
            first_corner: None,
        };
        let item = ResultItem {
            corners: vec![corner(600000, 200000), corner(620000, 200000)],
            layer: 0,
        };
        assert!(!instance.insert_trace(&item));
        let joined = sink.rows.join("\n");
        assert!(
            joined.contains("FoundConnectionInserter: insert trace failed"),
            "the debug still fires:\n{joined}"
        );
        assert!(
            !joined.contains("FANOUT_DIAG"),
            "the U28- prefix must not open the gate:\n{joined}"
        );
    }

    /// F4 (spec review): the neckdown ladder, exercised for the first
    /// time. The start-pin arm (`insert_neckdown` → `try_neck_down`,
    /// Java `:525-676`): the full-width forced insert FAILS all-or-
    /// nothing (okPoint = the from corner — the target sits inside a
    /// foreign obstacle's clearance shadow, so no detour can end
    /// there), the arm fires, and the final NARROWED insert (the pin's
    /// `pin_trace_neckdown_halfwidth` = 499 for this fixture's
    /// 1000-wide pad) lands the whole window back to the pin. The
    /// foreign class-1 pinch (surface gap 3516 below the line) blocks
    /// the full width (need 1500+2516+100 = 4116 > 3655) while the
    /// narrowed width passes (499+2516+100 = 3115 ≤ 3655). Kills the
    /// verdict comparison and an out-of-span `pin_is_on_layer` answer
    /// (the in-span `&&` boundary is NOT discriminable here: the
    /// fixture's pins span both layers, so the `&&`→`||` looseness is
    /// world-impossible — banked in SEAM).
    #[test]
    fn t11_neckdown_start_pin_narrows_blocked_target_to_pin_halfwidth() {
        let (mut manager, mut board) = parse_fixture();
        epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(668000, 16384)),
                &Point::Int(corner(672000, 16384)),
            ),
            0,
            100,
            &[2],
            1,
            FixedState::UserFixed,
        )
        .expect("pinch");
        let layer_count = board.layers().layers.len();
        let mut ctrl = AutorouteControl::new(&mut board, 94, &settings_ir(layer_count));
        ctrl.trace_half_width[0] = 1500;
        ctrl.with_neckdown = true;
        let mut seam = NoPullTight;
        let mut sink = CaptureSink::default();
        let mut instance = FoundConnectionInserter {
            manager: &mut manager,
            board: &mut board,
            seam: &mut seam,
            ctrl: &ctrl,
            sink: &mut sink,
            last_corner: None,
            first_corner: None,
        };
        let item = ResultItem {
            corners: vec![corner(663500, 20000), corner(667000, 20000)],
            layer: 0,
        };
        assert!(
            instance.insert_trace(&item),
            "the neckdown narrows through the pinch. rows:\n{}",
            sink.rows.join("\n")
        );
        let joined = sink.rows.join("\n");
        assert!(
            joined.contains(
                "decision=ADVANCE, neckdown=true, micro_neckdown=false, \
                 okPoint=(663500,20000), first=(663500,20000), last=(667000,20000)"
            ),
            "the neckdown ADVANCE row missing or wrong:\n{joined}"
        );
        assert!(
            joined.contains(
                "[FoundConnectionInserter.insert_trace] [compare_trace_insert_segment] \
                 net=94, layer=0, i=1, fromCornerNo=1, decision=ADVANCE, neckdown=true, \
                 micro_neckdown=false, okPoint=(663500,20000), first=(663500,20000), \
                 last=(667000,20000): Net #94"
            ),
            "the neckdown ADVANCE twin missing or wrong:\n{joined}"
        );
        // Exactly ONE net-94 trace survives: the narrowed one. The
        // failed full-width attempt landed nothing (all-or-nothing).
        let traces: Vec<(i32, Vec<(i32, i32)>)> = board
            .iter_descending()
            .filter(|entry| matches!(entry.data, ItemData::Trace { .. }) && entry.nets == [94])
            .map(|entry| {
                let hw = board.trace_half_width(entry.id).expect("trace hw");
                let corners = board
                    .trace_polyline(entry.id)
                    .expect("trace polyline")
                    .corners()
                    .iter()
                    .map(|c| match c {
                        Point::Int(ip) => (ip.x, ip.y),
                        Point::Rational(_) => unreachable!("integer fixture"),
                    })
                    .collect();
                (hw, corners)
            })
            .collect();
        assert_eq!(
            traces,
            vec![(499, vec![(667000, 20000), (663500, 20000)])],
            "the narrowed trace at the pin's neckdown halfwidth"
        );
        // The stub cleanup at corners[0] finds the PIN contact — no tail.
        assert!(
            joined.contains(
                "[FoundConnectionInserter.insert_trace] [compare_trace_stub_cleanup] \
                 net=94, layer=0, removed_stubs=0, trace_enabled=true: Net #94"
            ),
            "the pin contact must keep the route off the tail list:\n{joined}"
        );
    }

    /// F4: the OPEN side of the neckdown-distance gate
    /// (`pin_center.distance(to_corner) >= pin_neck_down_distance →
    /// None`, Java `:549-551`). The fixture pin's gate value is
    /// `2 * (0.5 * 1000 + 2516) = 6032`; a 3-corner item whose second
    /// window fails on an endpoint shadow hands the arm `ok = M` with
    /// `|pin → M| = 6031` — one BELOW the gate — so the ladder runs
    /// and the narrowed insert reaches the stop point (the success
    /// verdict `Some(ok) == Some(from_corner)`). The twin test below
    /// sits at EXACTLY 6032 and must reject.
    #[test]
    fn t11_neckdown_distance_gate_open_side_narrows_to_stop_point() {
        let (mut manager, mut board) = parse_fixture();
        // The foreign pinch END casts the shadow over B = (663500,32000):
        // |B → end| = 4000 < 4116 (full need) but ≥ 3115 (narrowed need),
        // and the vertical route line stays 4000 clear of the whole body.
        epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(667500, 32000)),
                &Point::Int(corner(671500, 32000)),
            ),
            0,
            100,
            &[2],
            1,
            FixedState::UserFixed,
        )
        .expect("pinch");
        let layer_count = board.layers().layers.len();
        let mut ctrl = AutorouteControl::new(&mut board, 94, &settings_ir(layer_count));
        ctrl.trace_half_width[0] = 1500;
        ctrl.with_neckdown = true;
        let mut seam = NoPullTight;
        let mut sink = CaptureSink::default();
        let mut instance = FoundConnectionInserter {
            manager: &mut manager,
            board: &mut board,
            seam: &mut seam,
            ctrl: &ctrl,
            sink: &mut sink,
            last_corner: None,
            first_corner: None,
        };
        let item = ResultItem {
            corners: vec![
                corner(663500, 20000),
                corner(663500, 26031), // |pin → M| = 6031 < 6032: gate OPEN
                corner(663500, 32000),
            ],
            layer: 0,
        };
        assert!(
            instance.insert_trace(&item),
            "the open gate narrows to the stop point. rows:\n{}",
            sink.rows.join("\n")
        );
        let joined = sink.rows.join("\n");
        assert!(
            joined.contains(
                "net=94, layer=0, i=2, fromCornerNo=2, decision=ADVANCE, neckdown=true, \
                 micro_neckdown=false, okPoint=(663500,26031)"
            ),
            "the i=2 neckdown ADVANCE row missing or wrong:\n{joined}"
        );
        let mut traces: Vec<(i32, Vec<(i32, i32)>)> = board
            .iter_descending()
            .filter(|entry| matches!(entry.data, ItemData::Trace { .. }) && entry.nets == [94])
            .map(|entry| {
                let hw = board.trace_half_width(entry.id).expect("trace hw");
                let corners = board
                    .trace_polyline(entry.id)
                    .expect("trace polyline")
                    .corners()
                    .iter()
                    .map(|c| match c {
                        Point::Int(ip) => (ip.x, ip.y),
                        Point::Rational(_) => unreachable!("integer fixture"),
                    })
                    .collect();
                (hw, corners)
            })
            .collect();
        traces.sort();
        assert_eq!(
            traces,
            vec![
                (499, vec![(663500, 32000), (663500, 26031)]),
                (1500, vec![(663500, 20000), (663500, 26031)]),
            ],
            "full-width to the stop point + narrowed through the shadow"
        );
    }

    /// F4: the CLOSED side of the neckdown-distance gate, at EXACT
    /// equality — `|pin → M| = 6032 = pin_neck_down_distance`, where
    /// Java's `>=` REFUSES (`None` from `tryNeckDown`, Java `:549`).
    /// The world is the open-side twin with M one unit higher: the
    /// verdict flips to FAIL, and NO narrowed trace may exist. The
    /// `>=` → `>` mutant opens the gate, the narrowed insert lands
    /// (4000 ≥ 3115) and the test's verdict/trace asserts fail — the
    /// boundary is the discriminator.
    #[test]
    fn t11_neckdown_distance_gate_boundary_rejects() {
        let (mut manager, mut board) = parse_fixture();
        epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(667500, 32000)),
                &Point::Int(corner(671500, 32000)),
            ),
            0,
            100,
            &[2],
            1,
            FixedState::UserFixed,
        )
        .expect("pinch");
        let layer_count = board.layers().layers.len();
        let mut ctrl = AutorouteControl::new(&mut board, 94, &settings_ir(layer_count));
        ctrl.trace_half_width[0] = 1500;
        ctrl.with_neckdown = true;
        let mut seam = NoPullTight;
        let mut sink = CaptureSink::default();
        let mut instance = FoundConnectionInserter {
            manager: &mut manager,
            board: &mut board,
            seam: &mut seam,
            ctrl: &ctrl,
            sink: &mut sink,
            last_corner: None,
            first_corner: None,
        };
        let item = ResultItem {
            corners: vec![
                corner(663500, 20000),
                corner(663500, 26032), // |pin → M| = 6032 == gate: REFUSED
                corner(663500, 32000),
            ],
            layer: 0,
        };
        assert!(
            !instance.insert_trace(&item),
            "the boundary must refuse the neckdown. rows:\n{}",
            sink.rows.join("\n")
        );
        let joined = sink.rows.join("\n");
        assert!(
            joined.contains(
                "net=94, layer=0, i=2, fromCornerNo=1, decision=FAIL, neckdown=false, \
                 micro_neckdown=false, okPoint=(663500,26032)"
            ),
            "the i=2 FAIL row missing or wrong:\n{joined}"
        );
        // No narrowed trace anywhere — the mutant's tell.
        let narrowed = board
            .iter_descending()
            .filter(|entry| matches!(entry.data, ItemData::Trace { .. }) && entry.nets == [94])
            .filter(|entry| board.trace_half_width(entry.id) == Some(499))
            .count();
        assert_eq!(narrowed, 0, "the refused gate must not narrow");
        // The faithful stub cleanup: at M the stranded full-width trace
        // is a tail (its endpoint contacts nothing) and is REMOVED —
        // the board ends with NO net-94 traces at all.
        assert!(
            joined.contains("compare_trace_stub_found net=94, corner_idx=1, corner=(663500,26032)"),
            "the stub-found row at M missing:\n{joined}"
        );
        assert!(
            joined.contains(
                "[FoundConnectionInserter.insert_trace] [compare_trace_stub_cleanup] \
                 net=94, layer=0, removed_stubs=1, trace_enabled=true: Net #94"
            ),
            "the stranded trace must be cleaned as a stub:\n{joined}"
        );
        let net94 = board
            .iter_descending()
            .filter(|entry| matches!(entry.data, ItemData::Trace { .. }) && entry.nets == [94])
            .count();
        assert_eq!(net94, 0, "the FAIL ate its own progress (stub cleanup)");
    }

    /// F4: the fanout MICRO-neckdown candidate ORDER (Java `:455-523`):
    /// the base-width insert fails all-or-nothing (the target sits in
    /// the pinch's shadow), and the FIRST succeeding candidate of the
    /// `(pin widths, 3/4, 3/5, 1/2)` insertion-ordered dedup list wins.
    /// The pinch surface gap 3642 blocks full width (need 4116) AND
    /// the 1200 that a `3/4 → 4/5` mutant would try first (need 3716),
    /// while the real first fraction candidate 1125 (need 3641) passes
    /// with 1 unit to spare. The row literal `candidate_half_width=1125`
    /// plus the hw-1125 trace kill both the reorder and the fraction
    /// mutants (each hands the win to 900).
    #[test]
    fn t11_fanout_micro_neckdown_candidate_order() {
        let (mut manager, mut board) = parse_fixture();
        epic_board::trace_ops::insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(
                &Point::Int(corner(668000, 16258)),
                &Point::Int(corner(672000, 16258)),
            ),
            0,
            100,
            &[2],
            1,
            FixedState::UserFixed,
        )
        .expect("pinch");
        let layer_count = board.layers().layers.len();
        let mut ctrl = AutorouteControl::new(&mut board, 94, &settings_ir(layer_count));
        ctrl.trace_half_width[0] = 1500;
        ctrl.is_fanout = true;
        ctrl.fanout_start_pin_name = Some("U27-P1".to_string());
        let mut seam = NoPullTight;
        let mut sink = CaptureSink::default();
        let mut instance = FoundConnectionInserter {
            manager: &mut manager,
            board: &mut board,
            seam: &mut seam,
            ctrl: &ctrl,
            sink: &mut sink,
            last_corner: None,
            first_corner: None,
        };
        let item = ResultItem {
            corners: vec![corner(663500, 20000), corner(669300, 20000)],
            layer: 0,
        };
        assert!(
            instance.insert_trace(&item),
            "the micro neckdown squeezes through. rows:\n{}",
            sink.rows.join("\n")
        );
        let joined = sink.rows.join("\n");
        assert!(
            joined.contains(
                "FANOUT_DIAG event=trace_insert_micro_neckdown_success, pin=U27-P1, net=94, \
                 layer=0, candidate_half_width=1125, baseHalfWidth=1500, traceClearanceClass=1, \
                 from=(663500,20000), to=(669300,20000)"
            ),
            "the micro success diag missing or wrong (candidate order):\n{joined}"
        );
        assert!(
            joined.contains(
                "decision=ADVANCE, neckdown=false, micro_neckdown=true, \
                 okPoint=(663500,20000), first=(663500,20000), last=(669300,20000)"
            ),
            "the micro ADVANCE row missing or wrong:\n{joined}"
        );
        let traces: Vec<(i32, Vec<(i32, i32)>)> = board
            .iter_descending()
            .filter(|entry| matches!(entry.data, ItemData::Trace { .. }) && entry.nets == [94])
            .map(|entry| {
                let hw = board.trace_half_width(entry.id).expect("trace hw");
                let corners = board
                    .trace_polyline(entry.id)
                    .expect("trace polyline")
                    .corners()
                    .iter()
                    .map(|c| match c {
                        Point::Int(ip) => (ip.x, ip.y),
                        Point::Rational(_) => unreachable!("integer fixture"),
                    })
                    .collect();
                (hw, corners)
            })
            .collect();
        assert_eq!(
            traces,
            vec![(1125, vec![(663500, 20000), (669300, 20000)])],
            "the winning candidate's trace"
        );
    }

    /// F5 (spec review): the insert_via span gate REJECTION face —
    /// a via rule whose only entry references PADD (a single-layer F.Cu padstack, padstack_no 6)  codespell:ignore
    /// cannot cover the requested 0→1
    /// span; the scan finds no suitable span, emits the debug row AND
    /// the gated `via_mask_not_found` diag, and returns false with
    /// ZERO id burn (Java `:703-720`). The inverted-gate mutant admits
    /// PADD and answers true (or hits the check) — the verdict +  codespell:ignore
    /// id-burn + row asserts all flip.
    #[test]
    fn t11_insert_via_span_gate_rejects_missing_span() {
        let (mut manager, mut board) = parse_fixture();
        let layer_count = board.layers().layers.len();
        let mut ctrl = AutorouteControl::new(&mut board, 94, &settings_ir(layer_count));
        let mut padd_no = 0;
        for no in 1..=32 {
            // (the fixture's single-layer F.Cu padstack carries the
            // codespell-flagged name the inline marker below waives;
            // the marker rides a single-line let — rustfmt leaves
            // trailing comments there intact, the T6 probe verified.)
            let name = board.library().padstack(no).map(|p| p.name.as_str());
            let is_fixture_padstack = name == Some("PADD"); // codespell:ignore
            if is_fixture_padstack {
                padd_no = no;
                break;
            }
        }
        assert_eq!(padd_no, 6, "PADD is the single-layer F.Cu padstack"); // codespell:ignore
        let new_index = board.rules().via_infos.len() as i32;
        board.rules_mut().via_infos.push(ViaInfo {
            name: "PADD_VIA".to_string(),
            padstack_no: padd_no,
            clearance_class: 0,
            attach_smd_allowed: false,
        });
        ctrl.via_rule = Some(epic_board::rules_surf::ViaRule {
            id: u32::MAX,
            name: "t11-span-gate".to_string(),
            via_infos: vec![new_index],
        });
        ctrl.is_fanout = true;
        ctrl.fanout_start_pin_name = Some("U27-P1".to_string());
        let mut seam = NoPullTight;
        let mut sink = CaptureSink::default();
        let mut instance = FoundConnectionInserter {
            manager: &mut manager,
            board: &mut board,
            seam: &mut seam,
            ctrl: &ctrl,
            sink: &mut sink,
            last_corner: None,
            first_corner: None,
        };
        let id_before = instance.board.max_generated_id();
        assert!(
            !instance.insert_via(&Point::Int(corner(700000, 30000)), 0, 1),
            "the span-missing rule must be rejected"
        );
        assert_eq!(
            instance.board.max_generated_id(),
            id_before,
            "no via may be built (zero id burn)"
        );
        let joined = sink.rows.join("\n");
        assert!(
            joined.contains(
                "DEBUG FoundConnectionInserter: via mask not found for net #94 \
                 covering layers 0 to 1"
            ),
            "the span-miss debug missing:\n{joined}"
        );
        assert!(
            joined.contains(
                "FANOUT_DIAG event=via_mask_not_found, pin=U27-P1, net=94, fromLayer=0, \
                 toLayer=1, location=(700000,30000), traceClearanceClass=1, \
                 viaClearanceClass=1, trace_half_width_from=1500, trace_half_width_to=1500"
            ),
            "the gated via_mask_not_found diag missing or wrong:\n{joined}"
        );
    }
}
