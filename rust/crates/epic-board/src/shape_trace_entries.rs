//! The shove substrate: which obstacles sit in a shape, which trace
//! pieces must be rerouted around it, and the cutout of traces from a
//! shape (Java `board/searchtree/ShapeTraceEntries.java`, 806 lines,
//! ported in full). Used by the shove functions (T10b).
//!
//! ## Port shape
//!
//! Java holds `RoutingBoard board` as a field; the port passes
//! `&mut SearchTreeManager` / `&mut Board` through the methods (the
//! D17 split — the same shape as [`crate::trace_ops`]).
//!
//! ## The entry-point list modeling
//!
//! Java's `EntryPoint` is an intrusive singly-linked list behind the
//! private `listAnchor`, and the CHAIN ORDER is observable (resort
//! rotates around an anchor, popPiece drains a middle run,
//! calculateStackLevels splices out skipped nodes — all pointer
//! surgery a caller can observe through `nextSubstituteTracePiece`).
//! The port models the chain as a `Vec<EntryPoint>` WHERE THE VEC
//! ORDER IS THE CHAIN (`work[0]` is `listAnchor`; there is no `next`
//! field). Every Java list op translates 1:1:
//!
//! * `insertEntryPoint` — the sorted walk becomes a sorted
//!   `Vec::insert`,
//! * `rotateEntryListAroundAnchor` — `work[pos..] ++ (work[..pos]
//!   with edge_index += edge_count)`,
//! * the resort dedup/trims — sliding-window removals
//!   (`Vec::remove`/`pop`/`truncate`),
//! * `calculateStackLevels` — a position walk with
//!   `drain(current+1..next)` splicing,
//! * `popPiece` — `drain(first..=last)`.
//!
//! Java's `nextSubstituteTracePiece` hands out UNINSERTED
//! `PolylineTrace` objects (id 0, never added to the board); the port
//! returns a [`SubstituteTracePiece`] value — the caller assigns the
//! id at insert time, which is exactly what Java's id-0 constructor
//! defers.

use std::cmp::Reverse;

use epic_geometry::float_point::FloatPoint;
use epic_geometry::polyline::Polyline;
use epic_geometry::tile_shape::TileShape;

use crate::board::{Board, ItemEntry};
use crate::contacts::{end_contacts, shares_net_no, start_contacts};
use crate::id::ItemId;
use crate::items::{FixedState, ItemData, ObstacleKind};
use crate::shape_entry_side::ShapeEntrySide;
use crate::trace_ops::{
    insert_trace_without_cleaning, is_routable, is_shove_fixed, nets_equal, nets_normal,
    remove_item_through_repository,
};
use crate::tree_manager::SearchTreeManager;
use crate::tree_shapes::trace_compensated_half_width;

/// Java `c_offset_add` (`:28`).
const C_OFFSET_ADD: f64 = 1.0;

/// One entry point of a trace into the shape (Java `EntryPoint`,
/// `:789-805`). Stored in [`ShapeTraceEntries::work`]; the VEC
/// POSITION is the Java `next` chain order.
#[derive(Debug, Clone, PartialEq)]
pub struct EntryPoint {
    /// The trace this entry belongs to (Java `trace`, an object
    /// reference — the port's id handle).
    pub(crate) trace_id: ItemId,
    /// The trace line crossing the border (Java `traceLineNo`).
    pub(crate) trace_line_no: i32,
    /// The crossing point (Java `entryApprox`).
    pub(crate) entry_approx: FloatPoint,
    /// The border line index (Java `edgeIndex`; MUTATED by the resort
    /// rotation, which adds `edgeCount` to prefix entries).
    pub(crate) edge_index: i32,
    /// The shove recursion depth (Java `stackLevel`; -1 = not yet
    /// calculated).
    pub(crate) stack_level: i32,
}

/// The uninserted substitute trace piece handed back by
/// [`ShapeTraceEntries::next_substitute_trace_piece`] (Java's id-0
/// `new PolylineTrace(...)` result, never added to the board by this
/// class).
#[derive(Debug, Clone)]
pub struct SubstituteTracePiece {
    /// The piece polyline (Java `PolylineTrace.lines`).
    pub lines: Polyline,
    /// The layer the shape lives on (Java `PolylineTrace.layer`).
    pub layer: i32,
    /// The half width borrowed from the shoveled trace (Java
    /// `PolylineTrace.getHalfWidth()`).
    pub half_width: i32,
    /// The nets borrowed from the shoveled trace (Java
    /// `PolylineTrace.netNumbers`).
    pub nets: Vec<i32>,
    /// The clearance class borrowed from the shoveled trace (Java
    /// `PolylineTrace.clearanceClassIndex()`).
    pub clearance_class: i32,
}

/// Java `ShapeTraceEntries` — auxiliary class used by the shove
/// functions.
pub struct ShapeTraceEntries {
    /// Java `shoveViaList` (public in Java): the vias collected by
    /// [`ShapeTraceEntries::store_items`] for the caller to shove.
    pub shove_via_list: Vec<ItemId>,
    shape: TileShape,
    layer: i32,
    own_net_nos: Vec<i32>,
    clearance_class_index: i32,
    /// Java `fromSide`; `None` where Java holds null (the ctor
    /// accepts null — `searchFromSide` then always installs a value).
    from_side: Option<ShapeEntrySide>,
    /// Java `listAnchor` + the `next` chain: THE VEC ORDER IS THE
    /// CHAIN (module docs).
    work: Vec<EntryPoint>,
    trace_piece_count: i32,
    max_stack_level: i32,
    shape_contains_trace_tails: bool,
    /// Java `foundObstacle` — NOTE the Java quirk: `storeTrace` sets
    /// it to the trace ON SUCCESS too (`:440`), so a non-None value
    /// alone does not mean failure.
    found_obstacle: Option<ItemId>,
}

impl ShapeTraceEntries {
    /// Java ctor `:46-63`. Used for shoving traces and vias out of
    /// the input shape; `from_side.no` is the side of `shape` from
    /// where the shove comes — if its `no < 0` (or it is `None`), the
    /// side is calculated internally by `searchFromSide`.
    #[must_use]
    pub fn new(
        shape: TileShape,
        layer: i32,
        own_net_nos: Vec<i32>,
        clearance_class_index: i32,
        from_side: Option<ShapeEntrySide>,
    ) -> Self {
        Self {
            shove_via_list: Vec::new(),
            shape,
            layer,
            own_net_nos,
            clearance_class_index,
            from_side,
            work: Vec::new(),
            trace_piece_count: 0,
            max_stack_level: 0,
            shape_contains_trace_tails: false,
            found_obstacle: None,
        }
    }

    /// Java `stackDepth()` (`:285-287`) — the maximum recursion depth
    /// for shoving the obstacle traces.
    #[must_use]
    pub fn stack_depth(&self) -> i32 {
        self.max_stack_level
    }

    /// Java `substituteTraceCount()` (`:290-292`) — the number of
    /// substitute trace pieces.
    #[must_use]
    pub fn substitute_trace_count(&self) -> i32 {
        self.trace_piece_count
    }

    /// Java `traceTailsInShape()` (`:298-300`) — an unconnected
    /// endpoint of a foreign-net trace is contained in the shape
    /// interior.
    #[must_use]
    pub fn trace_tails_in_shape(&self) -> bool {
        self.shape_contains_trace_tails
    }

    /// Java `getFoundObstacle()` (`:315-317`) — the item responsible
    /// for failing, IF the shove failed. Mind the quirk on the struct
    /// field: `store_trace` stores the trace on SUCCESS as well.
    #[must_use]
    pub fn found_obstacle(&self) -> Option<ItemId> {
        self.found_obstacle
    }

    /// The entry chain in Java's `listAnchor`-chain order (pin/test
    /// window onto the private list; also the spike-replay oracle).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn entry_chain(&self) -> &[EntryPoint] {
        &self.work
    }

    /// Java `storeItems` (`:177-220`). Stores traces and vias into
    /// the entry list; returns false if `item_list` contains
    /// obstacles that cannot be shoved aside. If `is_pad_check`, the
    /// check is for vias, otherwise for traces. If
    /// `copper_sharing_allowed`, overlaps with traces or pads of the
    /// own net are allowed.
    pub fn store_items(
        &mut self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
        item_list: &[ItemId],
        is_pad_check: bool,
        copper_sharing_allowed: bool,
    ) -> bool {
        for &current_item in item_list {
            let Some(entry) = board.get(current_item) else {
                continue; // Java would NPE; unreachable through the seam
            };
            // Java `!isPadCheck && currentItem instanceof
            // ViaObstacleArea || currentItem instanceof
            // ComponentObstacleArea` — `&&` binds TIGHTER: the
            // component kind skips regardless of isPadCheck.
            let is_via_obstacle_area = matches!(
                entry.data,
                ItemData::ObstacleArea {
                    kind: ObstacleKind::ViaObstacleArea,
                    ..
                }
            );
            let is_component_obstacle_area = matches!(
                entry.data,
                ItemData::ObstacleArea {
                    kind: ObstacleKind::ComponentObstacleArea,
                    ..
                }
            );
            if (!is_pad_check && is_via_obstacle_area) || is_component_obstacle_area {
                continue;
            }
            let contains_own_net = shares_net_no(&entry.nets, &self.own_net_nos);
            if let ItemData::ConductionArea { is_obstacle, .. } = &entry.data
                && (contains_own_net || !*is_obstacle)
            {
                continue;
            }
            if is_shove_fixed(board, current_item) && !contains_own_net {
                self.found_obstacle = Some(current_item);
                return false;
            }
            let is_via = matches!(entry.data, ItemData::Via { .. });
            let is_trace = matches!(entry.data, ItemData::Trace { .. });
            let is_pin = matches!(entry.data, ItemData::Pin { .. });
            if is_via {
                if is_pad_check || !contains_own_net {
                    self.shove_via_list.push(current_item);
                }
            } else if is_trace {
                if !self.store_trace(manager, board, current_item) {
                    return false;
                }
            } else if contains_own_net {
                if !copper_sharing_allowed {
                    self.found_obstacle = Some(current_item);
                    return false;
                }
                if is_pad_check && !(is_pin && board.pin_drill_allowed(current_item)) {
                    self.found_obstacle = Some(current_item);
                    return false;
                }
            } else {
                self.found_obstacle = Some(current_item);
                return false;
            }
        }
        self.search_from_side(board);
        self.resort(board);
        self.calculate_stack_levels(board)
    }

    /// Java `nextSubstituteTracePiece` (`:226-282`). Calculates the
    /// next substitute trace piece; `None` at the end of the
    /// substitute trace list.
    pub fn next_substitute_trace_piece(
        &mut self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
    ) -> Option<SubstituteTracePiece> {
        let (first, last) = self.pop_piece(board)?;
        let current_trace_id = first.trace_id;
        let current_half_width = board
            .trace_half_width(current_trace_id)
            .expect("entry trace exists");
        let current_class = board
            .item_clearance_class(current_trace_id)
            .expect("entry trace exists");
        let current_nets = item_nets(board, current_trace_id);
        let tree = manager.default_tree();
        let offset_shape = offset_shape(
            manager,
            board,
            &self.shape,
            current_half_width,
            current_class,
            self.clearance_class_index,
            self.layer,
            trace_compensated_half_width(board, tree, current_trace_id),
        );
        let edge_count = self.shape.border_line_count() as i32;
        let edge_diff = last.edge_index - first.edge_index;

        // calculate the polyline of the substitute trace
        let piece_len = usize::try_from(edge_diff + 3)
            .expect("Java NegativeArraySizeException for a negative edge difference");
        let mut piece_lines: Vec<epic_geometry::line::Line> = Vec::with_capacity(piece_len);
        // start with the intersecting line of the trace at the start
        // entry.
        let first_lines = board
            .trace_polyline(first.trace_id)
            .expect("entry trace exists");
        piece_lines.push(
            first_lines.lines
                [usize::try_from(first.trace_line_no).expect("trace line no in range")]
            .clone(),
        );
        // end with the intersecting line of the trace at the end entry
        let last_lines = board
            .trace_polyline(last.trace_id)
            .expect("entry trace exists");
        let end_line = last_lines.lines
            [usize::try_from(last.trace_line_no).expect("trace line no in range")]
        .clone();
        // fill the interior lines of pieceLines with the appropriate
        // edge lines of the offset shape
        let mut current_edge_no = first.edge_index % edge_count;
        for _ in 1..(piece_len - 1) {
            piece_lines.push(offset_shape.border_line(current_edge_no));
            if current_edge_no == edge_count - 1 {
                current_edge_no = 0;
            } else {
                current_edge_no += 1;
            }
        }
        piece_lines.push(end_line);
        let piece_polyline = Polyline::new(piece_lines);
        if piece_polyline.is_empty() {
            // no valid trace piece, return the next one
            return self.next_substitute_trace_piece(manager, board);
        }
        Some(SubstituteTracePiece {
            lines: piece_polyline,
            layer: self.layer,
            half_width: current_half_width,
            nets: current_nets,
            clearance_class: current_class,
        })
    }

    /// Java `cutoutTraces` (`:306-312`). Cuts out all traces in
    /// `item_list` out of the stored shape; traces sharing a net
    /// number with the own nets are ignored.
    pub fn cutout_traces(
        &mut self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
        item_list: &[ItemId],
    ) {
        for &current_item in item_list {
            let is_foreign_trace = match board.get(current_item) {
                Some(entry) => {
                    matches!(entry.data, ItemData::Trace { .. })
                        && !shares_net_no(&entry.nets, &self.own_net_nos)
                }
                None => false,
            };
            if is_foreign_trace {
                Self::cutout_trace(
                    manager,
                    board,
                    current_item,
                    &self.shape,
                    self.clearance_class_index,
                );
            }
        }
    }

    /// Java static `cutoutTrace` (`:66-107`). Cuts `trace_id` out of
    /// `shape`, enlarged by the trace pen and the clearance to
    /// `clearance_class_index`; splits the trace into the outside
    /// pieces, taking the two-piece fast path when the shape sits in
    /// the middle.
    pub fn cutout_trace(
        manager: &mut SearchTreeManager,
        board: &mut Board,
        trace_id: ItemId,
        shape: &TileShape,
        clearance_class_index: i32,
    ) {
        if !board.is_on_the_board(trace_id) {
            // Java warns "ShapeTraceEntries.cutout_trace : trace is
            // deleted" — log-only (D12), silent here.
            return;
        }
        let trace_layer = board.trace_layer(trace_id).expect("live trace has a layer");
        let trace_half_width = board
            .trace_half_width(trace_id)
            .expect("live trace has a half width");
        let trace_class = board
            .item_clearance_class(trace_id)
            .expect("live trace has a clearance class");
        let trace_nets = item_nets(board, trace_id);
        let trace_lines = board
            .trace_polyline(trace_id)
            .expect("live trace has a polyline")
            .clone();
        let tree = manager.default_tree();
        let offset_shape = if tree.is_clearance_compensation_used() {
            let current_offset =
                f64::from(trace_compensated_half_width(board, tree, trace_id)) + C_OFFSET_ADD;
            shape.offset(current_offset)
        } else {
            // enlarge the shape in 2 steps for symmetry reasons
            let cl_offset =
                f64::from(board.clearance_value(trace_class, clearance_class_index, trace_layer))
                    + C_OFFSET_ADD;
            let enlarged = shape.offset(f64::from(trace_half_width));
            enlarged.offset(cl_offset)
        };
        let pieces = offset_shape.cutout_polyline(&trace_lines);
        if pieces.len() == 1 && pieces[0] == trace_lines {
            // nothing cut off — Java compares the Polyline IDENTITY
            // (`pieces[0] == traceLines`, the completely-outside arm
            // of `TileShape.cutout(Polyline)` returns the input
            // object); a CUT piece always carries an appended border
            // line, so the value comparison cannot over-trigger.
            return;
        }
        if pieces.len() == 2
            && offset_shape.is_outside(&pieces[0].first_corner().expect("Java NPE: piece corner"))
            && offset_shape.is_outside(&pieces[1].last_corner().expect("Java NPE: piece corner"))
        {
            Self::fast_cutout_trace(
                manager,
                board,
                trace_id,
                &pieces[0],
                &pieces[1],
                trace_layer,
                trace_half_width,
                trace_class,
                trace_nets,
            );
        } else {
            remove_item_through_repository(manager, board, trace_id);
            for piece in &pieces {
                insert_trace_without_cleaning(
                    manager,
                    board,
                    piece.clone(),
                    trace_layer,
                    trace_half_width,
                    &trace_nets,
                    trace_class,
                    FixedState::Unfixed,
                );
            }
        }
    }

    /// Java static `fastCutoutTrace` (`:110-151`) — the optimized
    /// standard cutout case: the old trace is replaced by two pieces
    /// that INHERIT the tree entries of the untouched halves (no
    /// re-insert of the pieces into the trees beyond the entry
    /// transfer).
    #[allow(clippy::too_many_arguments)] // the Java read set, kept flat
    fn fast_cutout_trace(
        manager: &mut SearchTreeManager,
        board: &mut Board,
        trace_id: ItemId,
        start_piece: &Polyline,
        end_piece: &Polyline,
        layer: i32,
        half_width: i32,
        clearance_class: i32,
        nets: Vec<i32>,
    ) {
        // Java `board.additionalUpdateAfterChange(trace)`: the
        // BasicBoard implementation (`BasicBoard.java:1228`) is EMPTY;
        // the RoutingBoard override invalidates the autoroute
        // database — an engine seam that lands with the routing
        // engine's board integration (T11). Documented no-op here.
        board.item_undo.save_for_undo(&Reverse(trace_id));
        let start_id = board.alloc_id();
        board.insert_item(ItemEntry {
            id: start_id,
            data: ItemData::Trace {
                layer,
                half_width,
                lines: start_piece.clone(),
            },
            nets: nets.clone(),
            clearance_class,
            component_id: 0,
            fixed: FixedState::Unfixed,
            on_the_board: false,
        });
        let end_id = board.alloc_id();
        board.insert_item(ItemEntry {
            id: end_id,
            data: ItemData::Trace {
                layer,
                half_width,
                lines: end_piece.clone(),
            },
            nets,
            clearance_class,
            component_id: 0,
            fixed: FixedState::Unfixed,
            on_the_board: false,
        });
        manager.reuse_entries_after_cutout(board, trace_id, start_id, end_id);
        remove_item_through_repository(manager, board, trace_id);
        // Java notifies `board.communication.observers` for both new
        // traces — the headless port carries no observers; documented
        // no-op.
    }

    /// Java `storeTrace` (`:323-442`). Stores all intersection points
    /// of `trace_id` with the border of the internal shape enlarged
    /// by the half width and the clearance of the corresponding trace
    /// pen.
    fn store_trace(
        &mut self,
        manager: &mut SearchTreeManager,
        board: &mut Board,
        trace_id: ItemId,
    ) -> bool {
        let tree = manager.default_tree();
        let trace_half_width = board
            .trace_half_width(trace_id)
            .expect("stored trace exists");
        let trace_class = board
            .item_clearance_class(trace_id)
            .expect("stored trace exists");
        let trace_layer = board.trace_layer(trace_id).expect("stored trace exists");
        let trace_nets = item_nets(board, trace_id);
        let trace_lines = board
            .trace_polyline(trace_id)
            .expect("stored trace exists")
            .clone();
        let offset_shape = offset_shape(
            manager,
            board,
            &self.shape,
            trace_half_width,
            trace_class,
            self.clearance_class_index,
            trace_layer,
            trace_compensated_half_width(board, tree, trace_id),
        );

        // using enlarge here instead offset causes problems because
        // of a comparison in the constructor of class EntryPoint
        for (trace_line_no, edge_index) in offset_shape.entrance_points(&trace_lines) {
            let entry_approx = trace_lines.lines
                [usize::try_from(trace_line_no).expect("entrance line no in range")]
            .intersection_approx(&offset_shape.border_line(edge_index));
            self.insert_entry_point(trace_id, trace_line_no, edge_index, entry_approx);
        }

        // Look, if an end point of the trace lies in the interior of
        // the shape. This may be the case, if a via touches the shape
        if !shares_net_no(&trace_nets, &self.own_net_nos) {
            if !nets_normal(&trace_nets) {
                return false;
            }
            let first_corner =
                crate::items::trace::first_corner(&trace_lines).expect("Java NPE: trace corner");
            let last_corner = crate::items::trace::last_corner(&trace_lines)
                .expect("Java NPE: trace corner")
                .clone();
            let mut end_corner = first_corner;
            for i in 0..2 {
                if offset_shape.contains_point(&end_corner) {
                    let contact_list = if i == 0 {
                        start_contacts(manager, board, trace_id)
                    } else {
                        end_contacts(manager, board, trace_id)
                    };
                    let mut contact_count = 0;
                    let mut store_end_corner = true;

                    // check for contact object, which is not shovable
                    for &contact_item in &contact_list {
                        if !is_routable(board, contact_item) {
                            self.found_obstacle = Some(contact_item);
                            return false;
                        }
                        let contact_is_trace;
                        let contact_is_via;
                        let contact_half_width;
                        let contact_class;
                        {
                            let contact_entry =
                                board.get(contact_item).expect("contact item exists");
                            contact_is_trace = matches!(contact_entry.data, ItemData::Trace { .. });
                            contact_is_via = matches!(contact_entry.data, ItemData::Via { .. });
                            contact_class = contact_entry.clearance_class;
                            contact_half_width = if contact_is_trace {
                                board.trace_half_width(contact_item).expect("contact trace")
                            } else {
                                0
                            };
                        }
                        if contact_is_trace {
                            // NOTE the third clause is Java's
                            // `contactItem.clearanceClassIndex() !=
                            // contactTrace.clearanceClassIndex()` —
                            // BOTH names denote the SAME item, so the
                            // comparison is ALWAYS FALSE (the
                            // :380-381 quirk, kept verbatim).
                            #[allow(clippy::eq_op)]
                            {
                                if (is_shove_fixed(board, contact_item)
                                    || contact_half_width != trace_half_width
                                    || contact_class != contact_class)
                                    && offset_shape.contains_inside(&end_corner)
                                {
                                    self.found_obstacle = Some(contact_item);
                                    return false;
                                }
                            }
                        } else if contact_is_via {
                            let from_layer = board
                                .item_first_layer(contact_item)
                                .expect("via first layer");
                            let to_layer =
                                board.item_last_layer(contact_item).expect("via last layer");
                            if self.layer < from_layer || self.layer > to_layer {
                                // Java warns "DrillItem
                                // .getTileShapeOnLayer: layer out of
                                // range" (log-only, D12), returns null
                                // and NPEs at smallestRadius();
                                // unreachable for a via contacting the
                                // trace on this layer.
                                panic!("via shape is null on layer {} (Java NPE)", self.layer);
                            }
                            let tree = manager.default_tree();
                            let shapes = board.tree_shape_precalc(
                                contact_item,
                                tree.object_id(),
                                tree.variant,
                                tree.compensated_clearance_class,
                            );
                            let via_shape = shapes[(self.layer - from_layer) as usize]
                                .clone()
                                .expect("Java NPE: null via tree shape");
                            let mut via_trace_diff = via_shape.smallest_radius()
                                - f64::from(trace_compensated_half_width(board, tree, trace_id));
                            if !tree.is_clearance_compensation_used() {
                                let via_clearance = board.clearance_value(
                                    contact_class,
                                    self.clearance_class_index,
                                    self.layer,
                                );
                                let trace_clearance = board.clearance_value(
                                    trace_class,
                                    self.clearance_class_index,
                                    self.layer,
                                );
                                if trace_clearance > via_clearance {
                                    via_trace_diff += f64::from(via_clearance - trace_clearance);
                                }
                            }
                            if via_trace_diff < 0.0 {
                                // the via is smaller than the trace
                                self.found_obstacle = Some(contact_item);
                                return false;
                            }
                            if via_trace_diff == 0.0 && !offset_shape.contains_inside(&end_corner) {
                                // the via need not to be shoved
                                store_end_corner = false;
                            }
                        }
                        contact_count += 1;
                    }
                    if contact_count == 1 && store_end_corner {
                        let projection = offset_shape
                            .nearest_border_point(&end_corner)
                            .expect("border projection exists");
                        {
                            let projection_side =
                                offset_shape.contains_on_border_line_no(&projection);
                            // the following may not be correct because the trace may not
                            // contain a suitable line for the construction of the end line
                            // of the substitute trace.
                            let trace_line_segment_no = if i == 0 {
                                0
                            } else {
                                trace_lines.lines.len() as i32 - 1
                            };
                            if projection_side >= 0 {
                                self.insert_entry_point(
                                    trace_id,
                                    trace_line_segment_no,
                                    projection_side,
                                    projection.to_float(),
                                );
                            }
                        }
                    } else if contact_count == 0 && offset_shape.contains_inside(&end_corner) {
                        self.shape_contains_trace_tails = true;
                    }
                }
                if i == 0 {
                    end_corner = last_corner.clone();
                }
            }
        }
        // Java quirk kept: foundObstacle is set to the trace ON
        // SUCCESS too (`:440-441`).
        self.found_obstacle = Some(trace_id);
        true
    }

    /// Java `searchFromSide` (`:444-460`). If `from_side` is unset or
    /// carries `no < 0`, derives it from the FIRST own-net entry in
    /// the chain — or falls back to side 0 with no crossing point
    /// when no own-net entry exists (Java's `currentFromsideNo = 0`
    /// initial value).
    fn search_from_side(&mut self, board: &Board) {
        if self.from_side.as_ref().is_some_and(|side| side.no >= 0) {
            return; // from side is already legal
        }
        let mut current_fromside_no = 0;
        let mut current_entry_approx: Option<FloatPoint> = None;
        for node in &self.work {
            if shares_net_no(&item_nets(board, node.trace_id), &self.own_net_nos) {
                current_fromside_no = node.edge_index;
                current_entry_approx = Some(node.entry_approx);
                break;
            }
        }
        self.from_side = Some(ShapeEntrySide::new_precomputed(
            current_fromside_no,
            current_entry_approx,
        ));
    }

    /// Java `resort` (`:463-573`). Resorts the intersection points
    /// according to the from-side index and removes redundant points.
    fn resort(&mut self, board: &Board) {
        let edge_count = self.shape.border_line_count() as i32;
        let mut from_side = self
            .from_side
            .expect("Java NPE: fromSide is installed by searchFromSide before resort");
        if from_side.no < 0 || from_side.no >= edge_count {
            // Java warns "ShapeTraceEntries.resort: from side not
            // calculated" — log-only (D12), silent here.
            return;
        }
        // resort the intersection points, so that they start in the
        // middle of fromSide.
        let compare_corner1 = self
            .shape
            .corner_approx(from_side.no)
            .expect("corner of a legal side");
        let compare_corner2 = if from_side.no == edge_count - 1 {
            self.shape.corner_approx(0)
        } else {
            self.shape.corner_approx(from_side.no + 1)
        }
        .expect("corner of a legal side");
        let mut from_point_dist = 0.0;
        let mut from_point_projection: Option<FloatPoint> = None;
        if let Some(border_intersection) = from_side.border_intersection {
            let projection =
                border_intersection.projection_approx(&self.shape.border_line(from_side.no));
            from_point_projection = Some(projection);
            from_point_dist = projection.distance_square(&compare_corner1);
            if from_point_dist >= compare_corner1.distance_square(&compare_corner2) {
                from_side = ShapeEntrySide::new_precomputed(from_side.no, None);
            }
        }
        self.from_side = Some(from_side);
        // search the first intersection point between the side middle
        // and compareCorner2
        let mut break_pos: Option<usize> = None;
        for (pos, node) in self.work.iter().enumerate() {
            if node.edge_index > from_side.no {
                break_pos = Some(pos);
                break;
            }
            if node.edge_index == from_side.no {
                if let (Some(border_intersection), Some(projection)) =
                    (from_side.border_intersection, from_point_projection)
                {
                    let current_projection = node
                        .entry_approx
                        .projection_approx(&self.shape.border_line(from_side.no));
                    // Java compares the current projection against
                    // fromPointProjection (:502), the projected border
                    // intersection — not the raw crossing point.
                    let _ = border_intersection;
                    if current_projection.distance_square(&compare_corner1) >= from_point_dist
                        && current_projection.distance_square(&projection)
                            <= current_projection.distance_square(&compare_corner1)
                    {
                        break_pos = Some(pos);
                        break;
                    }
                } else if node.entry_approx.distance_square(&compare_corner2)
                    <= node.entry_approx.distance_square(&compare_corner1)
                {
                    break_pos = Some(pos);
                    break;
                }
            }
        }
        if let Some(pos) = break_pos
            && pos != 0
        {
            // Java: `current != null && current != listAnchor`
            self.rotate_entry_list_around_anchor(pos, edge_count);
        }
        // remove intersections between two other intersections of the
        // same connected set, so that only first and last intersection
        // is kept.
        if self.work.is_empty() {
            return;
        }
        if self.work.len() >= 2 {
            let mut before_prev_pos: Option<usize> = None;
            let mut prev_pos = 0usize;
            let mut current_pos = 1usize;
            while current_pos + 1 < self.work.len() {
                let next_pos = current_pos + 1;
                if net_nos_equal(
                    &item_nets(board, self.work[prev_pos].trace_id),
                    &item_nets(board, self.work[current_pos].trace_id),
                ) && net_nos_equal(
                    &item_nets(board, self.work[current_pos].trace_id),
                    &item_nets(board, self.work[next_pos].trace_id),
                ) {
                    // Java `prev.next = next` — drop the middle node;
                    // the position now holds the old next.
                    self.work.remove(current_pos);
                } else {
                    before_prev_pos = Some(prev_pos);
                    prev_pos = current_pos;
                    current_pos += 1;
                }
            }
            // exit state: current is the LAST node (position
            // work.len()-1); prev its predecessor.
            let last_pos = self.work.len() - 1;
            if net_nos_equal(
                &item_nets(board, self.work[last_pos].trace_id),
                &self.own_net_nos,
            ) {
                // Java `prev.next = null` — drop the last node.
                self.work.pop();
                if net_nos_equal(
                    &item_nets(board, self.work[prev_pos].trace_id),
                    &self.own_net_nos,
                ) {
                    if let Some(before_prev) = before_prev_pos {
                        // Java `beforePrev.next = null`.
                        self.work.truncate(before_prev + 1);
                    } else {
                        // Java `listAnchor = null`.
                        self.work.clear();
                    }
                }
            }
        }

        // remove nodes of own net at start of the list (Java
        // :566-572 — note the ITEM `netsEqual` here, NOT the local
        // order-independent `netNosEqual`; and the two-step advance).
        if let Some(first) = self.work.first()
            && nets_equal(&item_nets(board, first.trace_id), &self.own_net_nos)
        {
            self.work.remove(0);
            if let Some(first) = self.work.first()
                && nets_equal(&item_nets(board, first.trace_id), &self.own_net_nos)
            {
                self.work.remove(0);
            }
        }
    }

    /// Java `calculateStackLevels` (`:575-665`). Returns false when
    /// the stack property fails (a set must close at the level it
    /// opened).
    fn calculate_stack_levels(&mut self, board: &Board) -> bool {
        if self.work.is_empty() {
            return true;
        }
        let mut current_pos = 0usize;
        let mut current_nets = item_nets(board, self.work[current_pos].trace_id);
        let mut current_level = if net_nos_equal(&current_nets, &self.own_net_nos) {
            // ignore own net when calculating the stack level
            0
        } else {
            1
        };

        loop {
            // the entry at current_pos
            if self.work[current_pos].stack_level < 0 {
                // not yet calculated
                self.trace_piece_count += 1;
                self.work[current_pos].stack_level = current_level;
                if current_level > self.max_stack_level {
                    if self.max_stack_level > 1 {
                        self.found_obstacle = Some(self.work[current_pos].trace_id);
                    }
                    self.max_stack_level = current_level;
                }
            }

            // set stack level for all entries of the current net;
            let mut next_index = 0i32;
            let mut index_of_next_foreign_set = 0i32;
            let mut index_of_last_occurrence_of_set = 0i32;
            let mut last_own_pos: Option<usize> = None;
            let mut first_foreign_pos: Option<usize> = None;
            for check_pos in current_pos + 1..self.work.len() {
                next_index += 1;
                let check_nets = item_nets(board, self.work[check_pos].trace_id);
                if net_nos_equal(&check_nets, &current_nets) {
                    index_of_last_occurrence_of_set = next_index;
                    last_own_pos = Some(check_pos);
                    self.work[check_pos].stack_level = self.work[current_pos].stack_level;
                } else if index_of_next_foreign_set == 0 {
                    // first occurrence of a foreign connected set
                    index_of_next_foreign_set = next_index;
                    first_foreign_pos = Some(check_pos);
                }
            }

            if next_index == 0 {
                // Java: currentEntry = null — walk ends.
                break;
            }
            let next_pos;
            if index_of_next_foreign_set != 0
                && index_of_next_foreign_set < index_of_last_occurrence_of_set
            {
                // raise level
                next_pos = first_foreign_pos.expect("a counted foreign set has a position");
                if self.work[next_pos].stack_level >= 0 {
                    // already calculated — stack property fails
                    return false;
                }
                current_level += 1;
            } else if index_of_last_occurrence_of_set != 0 {
                next_pos = last_own_pos.expect("a counted set has a position");
            } else {
                next_pos = first_foreign_pos.expect("a counted foreign set has a position");
                if self.work[next_pos].stack_level >= 0 {
                    // already calculated
                    current_level -= 1;
                    if self.work[next_pos].stack_level != current_level {
                        return false;
                    }
                }
            }
            current_nets = item_nets(board, self.work[next_pos].trace_id);
            // remove all entries between currentEntry and nextEntry,
            // because they are irrelevant;
            self.work.drain(current_pos + 1..next_pos);
            current_pos += 1;
        }
        if current_level != 1 {
            // Java warns "ShapeTraceEntries.calculate_stack_levels:
            // currentLevel inconsistent" — log-only (D12), silent
            // here.
            return false;
        }
        true
    }

    /// Java `popPiece` (`:672-726`). Pops the next piece with MAXIMAL
    /// stack level from the intersection list (the Java doc says
    /// "minimal" but the code searches `stackLevel ==
    /// maxStackLevel` — the doc-comment bug kept, not fixed).
    /// Returns `None` if the stack is empty; the pair is (first
    /// entry point, last entry point of the piece).
    fn pop_piece(&mut self, board: &Board) -> Option<(EntryPoint, EntryPoint)> {
        if self.work.is_empty() {
            if self.trace_piece_count != 0 {
                // Java warns "ShapeTraceEntries: tracePieceCount is
                // inconsistent" — log-only (D12), silent here.
            }
            return None;
        }
        let first_pos = match self
            .work
            .iter()
            .position(|entry| entry.stack_level == self.max_stack_level)
        {
            Some(pos) => pos,
            None => {
                // Java warns "ShapeTraceEntries: maxStackLevel not
                // found" — log-only (D12), silent here.
                return None;
            }
        };
        let mut last_pos = first_pos;
        while last_pos + 1 < self.work.len()
            && self.work[last_pos + 1].stack_level == self.max_stack_level
            && nets_equal(
                &item_nets(board, self.work[last_pos + 1].trace_id),
                &item_nets(board, self.work[first_pos].trace_id),
            )
        {
            last_pos += 1;
        }
        let drained: Vec<EntryPoint> = self.work.drain(first_pos..=last_pos).collect();
        let first = drained
            .first()
            .cloned()
            .expect("drained run is never empty");
        let last = drained.last().cloned().expect("drained run is never empty");

        // recalculate maxStackLevel;
        self.max_stack_level = self
            .work
            .iter()
            .map(|entry| entry.stack_level)
            .max()
            .unwrap_or(0);
        self.trace_piece_count -= 1;
        if nets_equal(&item_nets(board, first.trace_id), &self.own_net_nos) {
            // own net is ignored and may occur only at the lowest
            // level
            return self.pop_piece(board);
        }
        Some((first, last))
    }

    /// Java `insertEntryPoint` (`:728-762`). Inserts the new entry
    /// into the sorted list; equal edge indices order by the scalar
    /// projection comparison (`prevCorner.scalarProduct(entryApprox,
    /// nextCorner)` — the LATER insertion wins ties, since the break
    /// on `<=` splices the new entry BEFORE the incumbent, Java's
    /// `newEntry.next = currentNext`).
    fn insert_entry_point(
        &mut self,
        trace_id: ItemId,
        trace_line_no: i32,
        edge_index: i32,
        entry_approx: FloatPoint,
    ) {
        let new_entry = EntryPoint {
            trace_id,
            trace_line_no,
            entry_approx,
            edge_index,
            stack_level: -1, // not yet calculated
        };
        let mut insert_pos = self.work.len();
        for (pos, node) in self.work.iter().enumerate() {
            if node.edge_index > new_entry.edge_index {
                // Java breaks with currentPrev behind and currentNext at
                // `pos`; the new entry is spliced BETWEEN them, i.e. at
                // `pos`. The position must be materialized here — the
                // initial work.len() value is only correct when the walk
                // falls through the whole list.
                insert_pos = pos;
                break;
            }
            if node.edge_index == new_entry.edge_index {
                let prev_corner = self
                    .shape
                    .corner_approx(edge_index)
                    .expect("corner of the entry edge");
                let next_corner = if edge_index == self.shape.border_line_count() as i32 - 1 {
                    self.shape.corner_approx(0)
                } else {
                    self.shape.corner_approx(new_entry.edge_index + 1)
                }
                .expect("corner of the entry edge");
                // than the projection of the line from prevCorner to
                // next.entryApprox onto the same line.
                if prev_corner.scalar_product(&entry_approx, &next_corner)
                    <= prev_corner.scalar_product(&node.entry_approx, &next_corner)
                {
                    insert_pos = pos;
                    break;
                }
            }
        }
        self.work.insert(insert_pos, new_entry);
    }

    /// Java `rotateEntryListAroundAnchor` (`:765-783`). Rotates the
    /// entry list so that `new_anchor_pos` becomes the list head; the
    /// rotated prefix gains `edge_count` on every edge index to
    /// differentiate points before and after the middle of fromSide.
    fn rotate_entry_list_around_anchor(&mut self, new_anchor_pos: usize, edge_count: i32) {
        let mut rotated: Vec<EntryPoint> = self.work[new_anchor_pos..].to_vec();
        let mut prefix: Vec<EntryPoint> = self.work[..new_anchor_pos].to_vec();
        for node in &mut prefix {
            node.edge_index += edge_count;
        }
        rotated.extend(prefix);
        self.work = rotated;
    }
}

/// Java static `netNosEqual` (`:153-170`) — ORDER-INDEPENDENT set
/// equality WITHOUT the `containsNet` net-0 guard. Deliberately KEPT
/// DISTINCT from [`crate::trace_ops::nets_equal`] (Java
/// `Item.netsEqual`, which routes through `containsNet` and answers
/// false for two `[0]` arrays) — `resort`/`calculateStackLevels`/
/// `popPiece` use BOTH notions exactly where Java does.
fn net_nos_equal(net_nos1: &[i32], net_nos2: &[i32]) -> bool {
    if net_nos1.len() != net_nos2.len() {
        return false;
    }
    for &current_net_no1 in net_nos1 {
        if !net_nos2.contains(&current_net_no1) {
            return false;
        }
    }
    true
}

/// The nets of an item, empty for a missing id (Java would NPE;
/// unreachable through the seam).
fn item_nets(board: &Board, id: ItemId) -> Vec<i32> {
    board
        .get(id)
        .map(|entry| entry.nets.clone())
        .unwrap_or_default()
}

/// Java's three-site offset construction (`cutoutTrace` `:74-84`,
/// `storeTrace` `:326-337`, `nextSubstituteTracePiece` `:235-245`):
/// one compensated single step or the two-step symmetric enlargement
/// (half width, then clearance + 1). The three sites differ ONLY in
/// the layer argument fed to the clearance read — cutoutTrace reads
/// the TRACE's layer, storeTrace the trace's layer, nextSubstitute
/// the SHAPE's layer — captured by the explicit `layer` parameter.
#[allow(clippy::too_many_arguments)] // the Java read set, kept flat
fn offset_shape(
    manager: &SearchTreeManager,
    board: &Board,
    shape: &TileShape,
    half_width: i32,
    trace_class: i32,
    own_class: i32,
    layer: i32,
    compensated_half_width: i32,
) -> TileShape {
    let tree = manager.default_tree();
    if tree.is_clearance_compensation_used() {
        shape.offset(f64::from(compensated_half_width) + C_OFFSET_ADD)
    } else {
        // enlarge the shape in 2 steps for symmetry reasons
        let cl_offset =
            f64::from(board.clearance_value(trace_class, own_class, layer)) + C_OFFSET_ADD;
        let enlarged = shape.offset(f64::from(half_width));
        enlarged.offset(cl_offset)
    }
}

#[cfg(test)]
mod tests {
    //! Literal captures from the jar-side spike
    //! `rust/harness/oracle/ShapeTraceEntriesProbe.java`
    //! (`logs/M3-T10a/captures/shape_trace_entries_rows.jsonl`, two
    //! byte-identical runs). The synthetic world: the
    //! `locator-spike/t9_locator45.dsn` fixture parsed fresh, then four
    //! traces (ids 105-108) plus a foreign through via (id 109)
    //! inserted through the production insert path — the SAME ids the
    //! Java probe's board handed out, so every captured id replays
    //! 1:1.

    use super::*;
    use crate::components::BoardPadstack;
    use crate::items::BoardShape;
    use crate::shape_and_entry_side::shape_and_entry_side;
    use crate::test_util::parse_board_from_path;
    use epic_geometry::int_box::IntBox;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::point::Point;
    use epic_geometry::regular_tile_shape::RegularTileShape;

    fn p(x: i32, y: i32) -> Point {
        Point::int(IntPoint::new(x, y))
    }

    fn ibox(x1: i32, y1: i32, x2: i32, y2: i32) -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::new(
            IntPoint::new(x1, y1),
            IntPoint::new(x2, y2),
        )))
    }

    /// The probe's synthetic world. The padstack mirrors Java
    /// `padstacks.add(shape, 0, layerCount - 1)` (the generated-name
    /// overload with `drillAllowed=false`); the via insert skips the
    /// `splitTraces` tail because the chosen center (cx+500, cy+1000)
    /// sits on no trace line — the same no-op the Java probe's
    /// `insertVia` ran (the initial (cx+500, cy+500) center WAS on the
    /// diagonal and `splitTraces` ate it, which moved the world).
    struct World {
        t3: ItemId,
        cx: i32,
        cy: i32,
        own: i32,
    }

    fn build_world() -> (SearchTreeManager, Board, World) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../rust/harness/fixtures/locator-spike/t9_locator45.dsn");
        let mut board = parse_board_from_path(&path.to_string_lossy());
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);

        let bb = board.bounding_box().expect("fixture bbox");
        let cx = (bb.ll.x + bb.ur.x) / 2;
        let cy = (bb.ll.y + bb.ur.y) / 2;
        // capture: the Java probe's bbox-center arithmetic
        assert_eq!(cx, 500000, "capture cx");
        assert_eq!(cy, 300000, "capture cy");
        // capture rows: ownNet=1 (N001), foreign net 2 (N002)
        assert!(
            board.rules().nets.get_by_name("N001", 1).is_some(),
            "N001 present"
        );
        assert!(
            board.rules().nets.get_by_name("N002", 1).is_some(),
            "N002 present"
        );
        let (own, foreign) = (1, 2);

        let rect = || {
            Some(BoardShape::Tile(TileShape::RegularTileShape(
                RegularTileShape::IntBox(IntBox::new(
                    IntPoint::new(-250, -250),
                    IntPoint::new(250, 250),
                )),
            )))
        };
        let layer_count = board.library().padstacks[0].shapes.len();
        board.library_mut().padstacks.push(BoardPadstack {
            name: "probe_via".to_string(),
            shapes: vec![rect(); layer_count],
            drillable: false,
            placed_absolute: false,
            hole_only: false,
        });
        let padstack_no = board.library().padstacks.len() as i32;

        let t1 = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(cx - 2000, cy), &p(cx + 2000, cy)),
            0,
            100,
            &[own],
            0,
            FixedState::Unfixed,
        )
        .expect("t1");
        let t2 = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(cx, cy - 2000), &p(cx, cy + 2000)),
            0,
            100,
            &[foreign],
            0,
            FixedState::Unfixed,
        )
        .expect("t2");
        let t3 = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(cx - 1500, cy - 1500), &p(cx + 1500, cy + 1500)),
            0,
            100,
            &[foreign],
            0,
            FixedState::Unfixed,
        )
        .expect("t3");
        let t4 = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(cx - 1000, cy + 1000), &p(cx + 1000, cy - 1000)),
            0,
            100,
            &[own],
            0,
            FixedState::Unfixed,
        )
        .expect("t4");
        // capture: ids 105..109 in insertion order
        assert_eq!(
            (t1.get(), t2.get(), t3.get(), t4.get()),
            (105, 106, 107, 108),
            "capture trace ids"
        );
        let via_id = board.alloc_id();
        board.insert_item(ItemEntry {
            id: via_id,
            data: ItemData::Via {
                center: IntPoint::new(cx + 500, cy + 1000),
                padstack_no,
                attach_smd_allowed: false,
            },
            nets: vec![foreign],
            clearance_class: 0,
            component_id: 0,
            fixed: FixedState::Unfixed,
            on_the_board: false,
        });
        manager.insert(&mut board, via_id);
        assert_eq!(via_id.get(), 109, "capture via id");

        (manager, board, World { t3, cx, cy, own })
    }

    /// One chain entry as captured: (traceId, lineNo, approx, edge, level).
    fn chain_row(entry: &EntryPoint) -> (i32, i32, [f64; 2], i32, i32) {
        (
            i32::try_from(entry.trace_id.get()).expect("positive id"),
            entry.trace_line_no,
            [entry.entry_approx.x, entry.entry_approx.y],
            entry.edge_index,
            entry.stack_level,
        )
    }

    fn assert_chain(entries: &ShapeTraceEntries, expected: &[(i32, i32, [f64; 2], i32, i32)]) {
        let rows: Vec<_> = entries.entry_chain().iter().map(chain_row).collect();
        assert_eq!(rows.len(), expected.len(), "chain length");
        for (got, want) in rows.iter().zip(expected) {
            assert_eq!(got.0, want.0, "traceId");
            assert_eq!(got.1, want.1, "lineNo");
            assert!(
                (got.2[0] - want.2[0]).abs() < 1e-6,
                "approx.x {} vs {}",
                got.2[0],
                want.2[0]
            );
            assert!(
                (got.2[1] - want.2[1]).abs() < 1e-6,
                "approx.y {} vs {}",
                got.2[1],
                want.2[1]
            );
            assert_eq!(got.3, want.3, "edgeIndex");
            assert_eq!(got.4, want.4, "stackLevel");
        }
    }

    /// The captured substitute piece list: run `nextSubstituteTracePiece`
    /// until None, pin every piece's (layer, halfWidth, class, nets,
    /// corners).
    fn assert_pieces(
        entries: &mut ShapeTraceEntries,
        manager: &mut SearchTreeManager,
        board: &mut Board,
        expected: &[(&str, Vec<[f64; 2]>)],
    ) {
        for (label, corners) in expected {
            let piece = entries
                .next_substitute_trace_piece(manager, board)
                .unwrap_or_else(|| panic!("missing piece {label}"));
            assert_eq!(piece.layer, 0, "{label} layer");
            assert_eq!(piece.half_width, 100, "{label} halfWidth");
            assert_eq!(piece.clearance_class, 0, "{label} class");
            assert_eq!(piece.nets, vec![2], "{label} nets");
            let got: Vec<[f64; 2]> = piece
                .lines
                .corner_approx_arr()
                .iter()
                .map(|c| [c.x, c.y])
                .collect();
            assert_eq!(got.len(), corners.len(), "{label} corner count");
            for (g, w) in got.iter().zip(corners) {
                assert!(
                    (g[0] - w[0]).abs() < 1e-6 && (g[1] - w[1]).abs() < 1e-6,
                    "{label} corner {g:?} vs {w:?}"
                );
            }
        }
        assert!(
            entries
                .next_substitute_trace_piece(manager, board)
                .is_none(),
            "pieces exhausted"
        );
    }

    /// s1-notcalc capture (rows 1-5): the full-shape store run. The
    /// chain pins the resort+dedup+stack-level exit state (triple dedup
    /// removed the middle of the 106 run, the own-net tail trim fired),
    /// and piece 0 is the level-1 106 run after the level-2 own-net
    /// piece was popped-and-skipped (popPiece's own-net recursion).
    #[test]
    fn s1_notcalc_chain_counts_and_piece_match_the_jar() {
        let (mut manager, mut board, world) = build_world();
        let s1 = ibox(
            world.cx - 1500,
            world.cy - 1500,
            world.cx + 1500,
            world.cy + 1500,
        );
        let mut items = manager.overlapping_objects(&mut board, 0, &s1, 0, &[]);
        // the probe sorted its storeItems input TreeSet ASCENDING by id
        items.sort_by_key(|id| id.get());
        assert_eq!(items.len(), 5, "capture items [105,106,107,108,109]");
        assert_eq!(
            items.iter().map(|id| id.get()).collect::<Vec<_>>(),
            vec![105, 106, 107, 108, 109]
        );

        let mut entries = ShapeTraceEntries::new(s1, 0, vec![world.own], 0, None);
        assert!(
            entries.store_items(&mut manager, &mut board, &items, false, false),
            "capture result=true"
        );
        assert_chain(
            &entries,
            &[
                (106, 1, [500000.0, 301617.0], 2, 1),
                (105, 1, [498383.0, 300000.0], 3, 2),
                (106, 1, [500000.0, 298383.0], 4, 1),
            ],
        );
        assert_eq!(entries.substitute_trace_count(), 2, "capture pieceCount");
        assert_eq!(entries.stack_depth(), 2, "capture maxStackLevel");
        assert!(entries.trace_tails_in_shape(), "capture tailsInShape=true");
        // the :440 quirk — foundObstacle set to a TRACE ON SUCCESS (108
        // is the last stored own-net trace, not a failure verdict).
        assert_eq!(
            entries.found_obstacle(),
            Some(ItemId::new(108)),
            "capture foundObstacle"
        );
        assert_eq!(
            entries.shove_via_list,
            vec![ItemId::new(109)],
            "capture shoveViaList"
        );
        assert_pieces(
            &mut entries,
            &mut manager,
            &mut board,
            &[(
                "p0",
                vec![
                    [500000.0, 301617.0],
                    [498383.0, 301617.0],
                    [498383.0, 298383.0],
                    [500000.0, 298383.0],
                ],
            )],
        );
    }

    /// s2-offset capture (rows 6-10): a half-plane shape grazing the
    /// diagonal's end corner; the single-entry chain and the
    /// degenerate two-identical-corners piece.
    #[test]
    fn s2_offset_single_entry_and_degenerate_piece_match_the_jar() {
        let (mut manager, mut board, world) = build_world();
        let s2 = ibox(
            world.cx - 3500,
            world.cy - 1500,
            world.cx - 500,
            world.cy + 1500,
        );
        let mut items = manager.overlapping_objects(&mut board, 0, &s2, 0, &[]);
        items.sort_by_key(|id| id.get());
        assert_eq!(
            items.iter().map(|id| id.get()).collect::<Vec<_>>(),
            vec![105, 107, 108],
            "capture items"
        );
        let mut entries = ShapeTraceEntries::new(s2, 0, vec![world.own], 0, None);
        assert!(entries.store_items(&mut manager, &mut board, &items, false, false));
        assert_chain(&entries, &[(107, 1, [499617.0, 299617.0], 5, 1)]);
        assert_eq!(entries.substitute_trace_count(), 1);
        assert_eq!(entries.stack_depth(), 1);
        assert!(entries.trace_tails_in_shape());
        assert_eq!(entries.found_obstacle(), Some(ItemId::new(108)));
        assert!(entries.shove_via_list.is_empty());
        assert_pieces(
            &mut entries,
            &mut manager,
            &mut board,
            &[("p0", vec![[499617.0, 299617.0], [499617.0, 299617.0]])],
        );
    }

    /// s3-frompoint capture (rows 11-16): ctor B from-side (no=3,
    /// intersection (499400,300000)) plus the five-entry chain with the
    /// 1/2/2/1/1 level profile and tailsInShape=false.
    #[test]
    fn s3_frompoint_side_chain_and_counts_match_the_jar() {
        let (mut manager, mut board, world) = build_world();
        let s3 = ibox(
            world.cx - 600,
            world.cy - 600,
            world.cx + 600,
            world.cy + 600,
        );
        let side = ShapeEntrySide::from_point(p(world.cx, world.cy), &s3);
        assert_eq!(side.no, 3, "capture from-side no");
        let bi = side
            .border_intersection
            .expect("capture borderIntersection");
        assert!(
            (bi.x - 499400.0).abs() < 1e-6 && (bi.y - 300000.0).abs() < 1e-6,
            "capture bi"
        );

        let mut items = manager.overlapping_objects(&mut board, 0, &s3, 0, &[]);
        items.sort_by_key(|id| id.get());
        assert_eq!(
            items.iter().map(|id| id.get()).collect::<Vec<_>>(),
            vec![105, 106, 107, 108],
            "capture items"
        );
        let mut entries = ShapeTraceEntries::new(s3, 0, vec![world.own], 0, Some(side));
        assert!(entries.store_items(&mut manager, &mut board, &items, false, false));
        assert_chain(
            &entries,
            &[
                (107, 1, [499283.0, 299283.0], 4, 1),
                (108, 1, [500717.0, 299283.0], 4, 2),
                (105, 1, [500717.0, 300000.0], 5, 2),
                (107, 1, [500717.0, 300717.0], 5, 1),
                (106, 1, [500000.0, 300717.0], 6, 1),
            ],
        );
        assert_eq!(entries.substitute_trace_count(), 2);
        assert_eq!(entries.stack_depth(), 2);
        assert!(
            !entries.trace_tails_in_shape(),
            "capture tailsInShape=false"
        );
        assert_eq!(entries.found_obstacle(), Some(ItemId::new(108)));
        assert!(
            entries.shove_via_list.is_empty(),
            "capture empty shoveViaList"
        );
        assert_pieces(
            &mut entries,
            &mut manager,
            &mut board,
            &[(
                "p0",
                vec![
                    [499283.0, 299283.0],
                    [500717.0, 299283.0],
                    [500717.0, 300717.0],
                    [500000.0, 300717.0],
                ],
            )],
        );
    }

    /// s1-padcheck capture (rows 17-22): ctor A from-side on the
    /// diagonal polyline (no=0, bi (498500,298500)) and the pad-check
    /// store run; the from-side LEGAL so resort rotates around it
    /// (edges 0,1,2) instead of leaving the natural order.
    #[test]
    fn s1_padcheck_entry_no_side_chain_and_piece_match_the_jar() {
        let (mut manager, mut board, world) = build_world();
        let t3_lines = board.trace_polyline(world.t3).expect("t3 polyline").clone();
        let s1 = ibox(
            world.cx - 1500,
            world.cy - 1500,
            world.cx + 1500,
            world.cy + 1500,
        );
        let side = ShapeEntrySide::from_entry_no(&t3_lines, 1, &s1);
        assert_eq!(side.no, 0, "capture from-side no");
        let bi = side
            .border_intersection
            .expect("capture borderIntersection");
        assert!(
            (bi.x - 498500.0).abs() < 1e-6 && (bi.y - 298500.0).abs() < 1e-6,
            "capture bi"
        );

        let mut items = manager.overlapping_objects(&mut board, 0, &s1, 0, &[]);
        items.sort_by_key(|id| id.get());
        let mut entries = ShapeTraceEntries::new(s1, 0, vec![world.own], 0, Some(side));
        assert!(
            entries.store_items(&mut manager, &mut board, &items, true, false),
            "padCheck run"
        );
        assert_chain(
            &entries,
            &[
                (106, 1, [500000.0, 298383.0], 0, 1),
                (105, 1, [501617.0, 300000.0], 1, 2),
                (106, 1, [500000.0, 301617.0], 2, 1),
            ],
        );
        assert_eq!(entries.substitute_trace_count(), 2);
        assert_eq!(entries.stack_depth(), 2);
        assert!(entries.trace_tails_in_shape());
        assert_eq!(entries.found_obstacle(), Some(ItemId::new(108)));
        assert_eq!(entries.shove_via_list, vec![ItemId::new(109)]);
        assert_pieces(
            &mut entries,
            &mut manager,
            &mut board,
            &[(
                "p0",
                vec![
                    [500000.0, 298383.0],
                    [501617.0, 298383.0],
                    [501617.0, 301617.0],
                    [500000.0, 301617.0],
                ],
            )],
        );
    }

    /// The dog-ear captures (rows 24-28): ShapeAndEntrySide over the
    /// diagonal (both cutlines at index 0) and over a 4-segment trace
    /// (id 110) inserted after the store runs — middle segment has NO
    /// cut, the inShoveCheck gate keeps fromSide null, and the
    /// non-check run falls back to ctor A.
    #[test]
    fn dog_ear_shapes_and_entry_sides_match_the_jar() {
        let (mut manager, mut board, world) = build_world();
        let (cx, cy) = (world.cx, world.cy);

        // de-107-orth: orthogonal short-circuit = the bounding box +
        // the nearest-border from-side.
        let orth = shape_and_entry_side(&manager, &mut board, world.t3, 0, true, false);
        let bb = orth.shape.bounding_box();
        assert_eq!(
            format!("{}:{}:{}:{}", bb.ll.x, bb.ll.y, bb.ur.x, bb.ur.y),
            "498400:298400:501600:301600",
            "capture de-107-orth bb"
        );
        let side = orth
            .from_side
            .expect("capture fromSide [0, 498400, 298400]");
        assert_eq!(side.no, 0);
        let bi = side.border_intersection.expect("bi");
        assert!((bi.x - 498400.0).abs() < 1e-6 && (bi.y - 298400.0).abs() < 1e-6);

        // de-107-both: both cutlines fire at index 0 of a 3-line trace.
        let both = shape_and_entry_side(&manager, &mut board, world.t3, 0, false, false);
        let bb = both.shape.bounding_box();
        assert_eq!(
            format!("{}:{}:{}:{}", bb.ll.x, bb.ll.y, bb.ur.x, bb.ur.y),
            "498429:298429:501571:301571",
            "capture de-107-both bb"
        );
        let borders: Vec<[f64; 4]> = (0..both.shape.border_line_count())
            .map(|i| {
                let line = both.shape.border_line(i as i32);
                let a = line.a.to_float();
                let b = line.b.to_float();
                [a.x, a.y, b.x, b.y]
            })
            .collect();
        let want: Vec<[f64; 4]> = vec![
            [200141.0, 0.0, 200142.0, 1.0],
            [501500.0, 301500.0, 501499.0, 301501.0],
            [199859.0, 0.0, 199858.0, -1.0],
            [498499.0, 298501.0, 498500.0, 298500.0],
        ];
        assert_eq!(borders, want, "capture de-107-both border lines");
        let side = both.from_side.expect("capture fromSide");
        assert_eq!(side.no, 3, "capture fromSide[0]");
        let bi = side.border_intersection.expect("capture fromSide bi");
        // the captured 2.147483647E9 pair — the intersection of the
        // cut line with the border line's DIRECTION form lands on the
        // int-max sentinel exactly as in the jar.
        assert!(
            (bi.x - 2147483647.0).abs() < 1.0 && (bi.y - 2147483647.0).abs() < 1.0,
            "capture bi {bi:?}"
        );

        // the 4-segment orthogonal trace (id 110, capture row 23 insert)
        let t5 = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_points(&[
                p(cx - 2600, cy + 800),
                p(cx - 2600, cy - 800),
                p(cx - 1600, cy - 800),
                p(cx - 1600, cy + 800),
            ]),
            0,
            100,
            &[world.own],
            0,
            FixedState::Unfixed,
        )
        .expect("t5");
        assert_eq!(t5.get(), 110, "capture t5 id");

        // de-5-mid-check: no cutline on the middle segment and the
        // inShoveCheck gate SUPPRESSES the from-side fallback.
        let mid_check = shape_and_entry_side(&manager, &mut board, t5, 1, false, true);
        let bb = mid_check.shape.bounding_box();
        assert_eq!(
            format!("{}:{}:{}:{}", bb.ll.x, bb.ll.y, bb.ur.x, bb.ur.y),
            "497300:299100:498500:299300",
            "capture de-5-mid bb"
        );
        assert_eq!(
            mid_check.shape.border_line_count(),
            8,
            "capture 8 border lines"
        );
        assert!(
            mid_check.from_side.is_none(),
            "capture fromSide null (gate)"
        );

        // de-5-mid: same shape, gate off — ctor A fallback fires.
        let mid = shape_and_entry_side(&manager, &mut board, t5, 1, false, false);
        let side = mid.from_side.expect("capture fromSide [4, 497400, 299300]");
        assert_eq!(side.no, 4, "capture fromSide no");
        let bi = side.border_intersection.expect("bi");
        assert!((bi.x - 497400.0).abs() < 1e-6 && (bi.y - 299300.0).abs() < 1e-6);

        // de-5-start: the start cutline at index 0.
        let start = shape_and_entry_side(&manager, &mut board, t5, 0, false, false);
        let bb = start.shape.bounding_box();
        assert_eq!(
            format!("{}:{}:{}:{}", bb.ll.x, bb.ll.y, bb.ur.x, bb.ur.y),
            "497300:299100:497500:300800",
            "capture de-5-start bb"
        );
        assert_eq!(start.shape.border_line_count(), 6, "capture 6 border lines");
        let side = start.from_side.expect("capture fromSide");
        assert_eq!(side.no, 3, "capture fromSide no");
        let bi = side.border_intersection.expect("capture bi");
        assert!(
            (bi.x - 2147483647.0).abs() < 1.0 && (bi.y - 2147483647.0).abs() < 1.0,
            "capture bi {bi:?}"
        );
    }

    /// cut-none capture (rows 29-30): a far-away cut shape leaves the
    /// trace and the item set untouched (the Polyline-identity arm of
    /// `cutoutTrace`).
    #[test]
    fn cutout_none_leaves_the_board_untouched() {
        let (mut manager, mut board, world) = build_world();
        let count_before = board.item_count();
        let far = ibox(
            world.cx + 4000,
            world.cy + 4000,
            world.cx + 4500,
            world.cy + 4500,
        );
        ShapeTraceEntries::cutout_trace(&mut manager, &mut board, world.t3, &far, 0);
        assert!(board.is_on_the_board(world.t3), "trace 107 stays");
        assert_eq!(board.item_count(), count_before, "no items added");
        let lines = board.trace_polyline(world.t3).expect("t3 polyline");
        assert_eq!(lines.corner_approx_arr().len(), 2, "uncut corner count");
        assert!((lines.corner_approx(0).x - 498500.0).abs() < 1e-6);
        assert!((lines.corner_approx(1).y - 301500.0).abs() < 1e-6);
    }

    /// cut-mid capture (rows 31-32): the FAST two-piece path. Pieces
    /// 110/111 inherit the cut trace's id sequence, carry the exact
    /// captured corner pairs, and each holds ONE transferred tree
    /// entry (the reuseEntriesAfterCutout transfer).
    #[test]
    fn cutout_middle_fast_path_splits_and_transfers_entries() {
        let (mut manager, mut board, world) = build_world();
        let mid = ibox(
            world.cx - 400,
            world.cy - 400,
            world.cx + 400,
            world.cy + 400,
        );
        ShapeTraceEntries::cutout_trace(&mut manager, &mut board, world.t3, &mid, 0);
        assert!(!board.is_on_the_board(world.t3), "107 removed");
        assert_eq!(board.item_count(), 110, "capture: pieces 110+111 added");
        let start = board.trace_polyline(ItemId::new(110)).expect("start piece");
        let got: Vec<[f64; 2]> = start
            .corner_approx_arr()
            .iter()
            .map(|c| [c.x, c.y])
            .collect();
        assert_eq!(
            got,
            vec![[498500.0, 298500.0], [499483.0, 299483.0]],
            "capture 110 corners"
        );
        let end = board.trace_polyline(ItemId::new(111)).expect("end piece");
        let got: Vec<[f64; 2]> = end.corner_approx_arr().iter().map(|c| [c.x, c.y]).collect();
        assert_eq!(
            got,
            vec![[500517.0, 300517.0], [501500.0, 301500.0]],
            "capture 111 corners"
        );
        let tree_id = manager.default_tree().object_id();
        for id in [ItemId::new(110), ItemId::new(111)] {
            let entries = manager
                .tree_entries(id, tree_id)
                .expect("transferred entries");
            assert_eq!(entries.len(), 1, "piece {id:?} has one tree entry");
            assert!(entries[0].is_some(), "piece {id:?} entry at shapeIdx 0");
        }
    }

    /// cut-end capture (rows 33-34): the SLOW remove-and-reinsert path
    /// (the shape covers the trace's end corner) — one surviving piece
    /// with the captured corners.
    #[test]
    fn cutout_end_slow_path_leaves_one_piece() {
        let (mut manager, mut board, world) = build_world();
        let end_box = ibox(
            world.cx + 700,
            world.cy + 700,
            world.cx + 2200,
            world.cy + 2200,
        );
        ShapeTraceEntries::cutout_trace(&mut manager, &mut board, world.t3, &end_box, 0);
        assert!(!board.is_on_the_board(world.t3), "107 removed");
        assert_eq!(board.item_count(), 109, "capture: one piece added");
        let piece = board.trace_polyline(ItemId::new(110)).expect("piece 110");
        let got: Vec<[f64; 2]> = piece
            .corner_approx_arr()
            .iter()
            .map(|c| [c.x, c.y])
            .collect();
        assert_eq!(
            got,
            vec![[498500.0, 298500.0], [500583.0, 300583.0]],
            "capture 110 corners"
        );
        assert!(board.is_on_the_board(ItemId::new(110)), "piece on board");
        assert!(board.trace_polyline(ItemId::new(111)).is_none(), "no 111");
    }
}
