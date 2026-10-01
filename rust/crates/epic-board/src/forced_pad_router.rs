//! Checking and forcing pads (via pads and drill pads) into a live
//! board, shoving obstacle traces aside (Java
//! `board/actions/ForcedPadRouter.java`, 499 lines, ported in full).
//! The third corner of the shove recursion cycle:
//! `DrillItemMover → ForcedPadRouter → TraceShover`.
//!
//! ## The verbatim quirks
//!
//! * [`in_front_of_pad`] case 0 carries Java's `lineB.x + lineB.x`
//!   self-addition BUG in its third disjunct — reproduced verbatim
//!   (`:88`), self-addition of the x coordinate instead of
//!   `lineA.x + lineB.x`.
//! * Unlike [`crate::trace_shover::check`], [`check_forced_pad`] does
//!   NOT skip own-net vias in its ladder and checks the recursion
//!   budget BEFORE the stack depth (`:286-299`, reversed order).
//! * [`forced_pad`] calls `shoveVias` with copper sharing FORCED OFF
//!   (`:367-378`) even though it accepts the flag for its own store
//!   run.

use std::collections::BTreeSet;

use epic_geometry::line::Line;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::tile_shape::TileShape;

use crate::board::Board;
use crate::contacts::shares_net_no;
use crate::drill_item_mover;
use crate::id::ItemId;
use crate::items::{BoardShape, ItemData};
use crate::shape_and_entry_side::shape_and_entry_side_core;
use crate::shape_entry_side::ShapeEntrySide;
use crate::shape_trace_entries::ShapeTraceEntries;
use crate::time_limit::TimeLimit;
use crate::trace_ops::normalize;
use crate::trace_shover::{
    board_outline_id, combine_traces, contains_trace_tails, get_trace_tail,
    piece_compensated_half_width, remove_items,
};
use crate::tree_manager::SearchTreeManager;

/// Java `CheckDrillResult` (`:491-497`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckDrillResult {
    /// Java `DRILLABLE`.
    Drillable,
    /// Java `DRILLABLE_WITH_ATTACH_SMD` — drillable only because an
    /// own-net SMD pin may be overlapped (attach allowed).
    DrillableWithAttachSmd,
    /// Java `NOT_DRILLABLE`.
    NotDrillable,
}

/// Java `calcCheckShapeForFromSide` (`:42-54`): a thin two-line probe
/// shape jutting from `shape_center` towards `border_line` (line 0
/// along the border direction, line 1 at +90 degrees, line 2 through
/// the border projection), offset by 1 — the probe
/// [`calc_from_side`] sweeps along each border.
///
/// NOTE the Java body never reads the `shape` parameter — the port
/// keeps it for call-signature parity (Java passes `shape`, NOT the
/// offset shape, at `:489`).
pub(crate) fn calc_check_shape_for_from_side(
    _shape: &TileShape,
    shape_center: &Point,
    border_line: &Line,
) -> TileShape {
    let shape_center_float = shape_center.to_float();
    let offset_projection = shape_center_float.projection_approx(border_line);
    // Make sure, that direction restrictions are retained.
    let current_direction = border_line.direction().clone();
    let lines = vec![
        Line::new_with_direction(shape_center.clone(), current_direction.clone()),
        Line::new_with_direction(shape_center.clone(), current_direction.turn_45_degree(2)),
        Line::new_with_direction(Point::Int(offset_projection.round()), current_direction),
    ];
    let check_line = Polyline::new(lines);
    check_line
        .offset_shape(1, 0)
        .expect("Java NPE: the 3-line probe always has a first offset shape")
}

/// Java `inFrontOfPad` (`:57-212`): is `line` in front of `pad_shape`
/// when shoving from side `from_side`? Only implemented for integer
/// octagon pad shapes and integer endpoints — everything else answers
/// true (the conservative "check it" default).
///
/// Case 0's third disjunct reproduces Java's `lineB.x + lineB.x`
/// self-addition bug verbatim (`:78`). Every other comparison term is
/// Java's min/max-of-SUMS form (`Math.min(lineA.x + lineA.y,
/// lineB.x + lineB.y)`), NOT a sum of mins/maxes — the two agree except
/// on (1,−1)-slope segments, where Java's form is strictly larger.
pub(crate) fn in_front_of_pad(
    line: &Line,
    pad_shape: &TileShape,
    from_side: i32,
    width: i32,
    with_sides: bool,
) -> bool {
    if !pad_shape.is_int_octagon() {
        // only implemented for octagons
        return true;
    }
    let pad_octagon = pad_shape
        .bounding_octagon()
        .expect("bounding octagon of an IntOctagon shape");
    let (line_a, line_b) = match (&line.a, &line.b) {
        (Point::Int(a), Point::Int(b)) => (*a, *b),
        // not implemented
        _ => return true,
    };

    let diag_width = f64::from(width) * std::f64::consts::SQRT_2;
    let pad = &pad_octagon;

    let mut result = match from_side {
        0 => {
            // NOTE the `line_b.x + line_b.x` self-addition — Java's
            // bug at `:78`, kept verbatim.
            (line_a.y.min(line_b.y) >= pad.top_y + width)
                || (f64::from((line_a.x - line_a.y).max(line_b.x - line_b.y))
                    <= f64::from(pad.upper_left_diagonal_x) - diag_width)
                || (f64::from(line_a.x + line_a.y).min(f64::from(line_b.x + line_b.x))
                    >= f64::from(pad.upper_right_diagonal_x) + diag_width)
        }
        1 => {
            (line_a.y.min(line_b.y) >= pad.top_y + width)
                || (f64::from((line_a.x - line_a.y).max(line_b.x - line_b.y))
                    <= f64::from(pad.upper_left_diagonal_x) - diag_width)
                || (line_a.x.max(line_b.x) <= pad.left_x - width)
        }
        2 => {
            (line_a.x.max(line_b.x) <= pad.left_x - width)
                || (f64::from((line_a.x - line_a.y).max(line_b.x - line_b.y))
                    <= f64::from(pad.upper_left_diagonal_x) - diag_width)
                || (f64::from((line_a.x + line_a.y).max(line_b.x + line_b.y))
                    <= f64::from(pad.lower_left_diagonal_x) - diag_width)
        }
        3 => {
            (line_a.x.max(line_b.x) <= pad.left_x - width)
                || (line_a.y.max(line_b.y) <= pad.bottom_y - width)
                || (f64::from((line_a.x + line_a.y).max(line_b.x + line_b.y))
                    <= f64::from(pad.lower_left_diagonal_x) - diag_width)
        }
        4 => {
            (line_a.y.max(line_b.y) <= pad.bottom_y - width)
                || (f64::from((line_a.x + line_a.y).max(line_b.x + line_b.y))
                    <= f64::from(pad.lower_left_diagonal_x) - diag_width)
                || (f64::from((line_a.x - line_a.y).min(line_b.x - line_b.y))
                    >= f64::from(pad.lower_right_diagonal_x) + diag_width)
        }
        5 => {
            (line_a.y.max(line_b.y) <= pad.bottom_y - width)
                || (line_a.x.min(line_b.x) >= pad.right_x + width)
                || (f64::from((line_a.x - line_a.y).min(line_b.x - line_b.y))
                    >= f64::from(pad.lower_right_diagonal_x) + diag_width)
        }
        6 => {
            (line_a.x.min(line_b.x) >= pad.right_x + width)
                || (f64::from((line_a.x + line_a.y).min(line_b.x + line_b.y))
                    >= f64::from(pad.upper_right_diagonal_x) + diag_width)
                || (f64::from((line_a.x - line_a.y).min(line_b.x - line_b.y))
                    >= f64::from(pad.lower_right_diagonal_x) + diag_width)
        }
        7 => {
            (line_a.y.min(line_b.y) >= pad.top_y + width)
                || (f64::from((line_a.x + line_a.y).min(line_b.x + line_b.y))
                    >= f64::from(pad.upper_right_diagonal_x) + diag_width)
                || (line_a.x.min(line_b.x) >= pad.right_x + width)
        }
        _ => {
            // FRLogger.warn("ForcedPadAlgo.in_front_of_pad: fromSide
            // out of range") — log-only (D12).
            return true;
        }
    };
    if with_sides && !result {
        result = match from_side {
            0 => {
                (line_a.x.max(line_b.x) <= pad.left_x - width)
                    && (f64::from((line_a.x - line_a.y).min(line_b.x - line_b.y))
                        <= f64::from(pad.upper_left_diagonal_x) - diag_width)
                    || (line_a.x.min(line_b.x) >= pad.right_x + width)
                        && (f64::from((line_a.x + line_a.y).min(line_b.x + line_b.y))
                            >= f64::from(pad.upper_right_diagonal_x) + diag_width)
            }
            1 => {
                (line_a.x.min(line_b.x) <= pad.left_x - width)
                    && (f64::from((line_a.x + line_a.y).max(line_b.x + line_b.y))
                        <= f64::from(pad.lower_left_diagonal_x) - diag_width)
                    || (line_a.y.max(line_b.y) >= pad.top_y + width)
                        && (f64::from((line_a.x + line_a.y).min(line_b.x + line_b.y))
                            >= f64::from(pad.upper_right_diagonal_x) + diag_width)
            }
            2 => {
                (line_a.y.max(line_b.y) <= pad.bottom_y - width)
                    && (f64::from((line_a.x + line_a.y).min(line_b.x + line_b.y))
                        <= f64::from(pad.lower_left_diagonal_x) - diag_width)
                    || (line_a.y.min(line_b.y) >= pad.top_y + width)
                        && (f64::from((line_a.x - line_a.y).min(line_b.x - line_b.y))
                            <= f64::from(pad.upper_left_diagonal_x) - diag_width)
            }
            3 => {
                (line_a.y.min(line_b.y) <= pad.bottom_y - width)
                    && (f64::from((line_a.x - line_a.y).min(line_b.x - line_b.y))
                        >= f64::from(pad.lower_right_diagonal_x) + diag_width)
                    || (line_a.x.min(line_b.x) <= pad.left_x - width)
                        && (f64::from((line_a.x - line_a.y).max(line_b.x - line_b.y))
                            <= f64::from(pad.upper_left_diagonal_x) - diag_width)
            }
            4 => {
                (line_a.x.min(line_b.x) >= pad.right_x + width)
                    && (f64::from((line_a.x - line_a.y).max(line_b.x - line_b.y))
                        >= f64::from(pad.lower_right_diagonal_x) + diag_width)
                    || (line_a.x.max(line_b.x) <= pad.left_x - width)
                        && (f64::from((line_a.x + line_a.y).min(line_b.x + line_b.y))
                            <= f64::from(pad.lower_left_diagonal_x) - diag_width)
            }
            5 => {
                (line_a.x.max(line_b.x) >= pad.right_x + width)
                    && (f64::from((line_a.x + line_a.y).min(line_b.x + line_b.y))
                        >= f64::from(pad.upper_right_diagonal_x) + diag_width)
                    || (line_a.y.min(line_b.y) <= pad.bottom_y - width)
                        && (f64::from((line_a.x + line_a.y).max(line_b.x + line_b.y))
                            <= f64::from(pad.lower_left_diagonal_x) - diag_width)
            }
            6 => {
                (line_a.y.max(line_b.y) <= pad.bottom_y - width)
                    && (f64::from((line_a.x - line_a.y).max(line_b.x - line_b.y))
                        >= f64::from(pad.lower_right_diagonal_x) + diag_width)
                    || (line_a.y.min(line_b.y) >= pad.top_y + width)
                        && (f64::from((line_a.x + line_a.y).max(line_b.x + line_b.y))
                            >= f64::from(pad.upper_right_diagonal_x) + diag_width)
            }
            7 => {
                (line_a.y.max(line_b.y) >= pad.top_y + width)
                    && (f64::from((line_a.x - line_a.y).max(line_b.x - line_b.y))
                        <= f64::from(pad.upper_left_diagonal_x) - diag_width)
                    || (line_a.x.max(line_b.x) >= pad.right_x + width)
                        && (f64::from((line_a.x - line_a.y).min(line_b.x - line_b.y))
                            >= f64::from(pad.lower_right_diagonal_x) + diag_width)
            }
            _ => true,
        };
    }
    result
}

/// Java `checkForcedPad` (`:221-339`): can obstacle traces be shoved
/// aside so that a pad with the input parameters can be inserted
/// without clearance violations? The board is NOT changed.
///
/// Unlike [`crate::trace_shover::check`]: own-net vias are NOT skipped
/// in the via ladder, and the recursion budget is exhausted BEFORE the
/// stack depth is tested (`:286-299`).
#[allow(clippy::too_many_arguments)] // the Java read set, kept flat
pub fn check_forced_pad(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    pad_shape: &TileShape,
    from_side: ShapeEntrySide,
    layer: i32,
    net_numbers: &[i32],
    clearance_class_index: i32,
    copper_sharing_allowed: bool,
    ignore_items: &[ItemId],
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
    check_only_front: bool,
    time_limit: Option<&TimeLimit>,
) -> CheckDrillResult {
    // Read the side number up front: `from_side` is consumed by the
    // ShapeTraceEntries constructor below, but the in-front gate in the
    // piece loop still needs it (Java reads the field off the object).
    let from_side_no = from_side.no;
    let board_bbox = board.bounding_box().expect("post-parse bounding box");
    if !pad_shape.is_contained_in_int_box(&board_bbox) {
        let outline = board_outline_id(board);
        board.set_shove_failing_obstacle(outline);
        return CheckDrillResult::NotDrillable;
    }
    let tree = manager.default_tree();
    let (tree_oid_unused, tree_variant, tree_class) = (
        tree.object_id(),
        tree.variant,
        tree.compensated_clearance_class,
    );
    let _ = tree_oid_unused;
    let mut shape_entries = ShapeTraceEntries::new(
        pad_shape.clone(),
        layer,
        net_numbers.to_vec(),
        clearance_class_index,
        Some(from_side),
    );
    let mut obstacles = manager.overlapping_items_with_clearance(
        board,
        0,
        pad_shape,
        layer,
        &[],
        clearance_class_index,
    );

    // Java `obstacles.removeAll(ignoreItems)` — order-preserving.
    if !ignore_items.is_empty() {
        obstacles.retain(|id| !ignore_items.contains(id));
    }
    let obstacles_shovable =
        shape_entries.store_items(manager, board, &obstacles, true, copper_sharing_allowed);
    if !obstacles_shovable {
        board.set_shove_failing_obstacle(shape_entries.found_obstacle());
        return CheckDrillResult::NotDrillable;
    }

    // check, if the obstacle vias can be shoved
    // NOTE: no own-net skip here (unlike TraceShover.check).
    let via_list = shape_entries.shove_via_list.clone();
    for via_id in via_list {
        if max_via_recursion_depth <= 0 {
            board.set_shove_failing_obstacle(Some(via_id));
            return CheckDrillResult::NotDrillable;
        }
        let new_via_center = drill_item_mover::try_shove_via_points(
            manager,
            board,
            pad_shape,
            layer,
            via_id,
            clearance_class_index,
            false,
        );

        if new_via_center.is_empty() {
            board.set_shove_failing_obstacle(Some(via_id));
            return CheckDrillResult::NotDrillable;
        }
        let via_center = board
            .drill_center(via_id)
            .expect("Java NPE: via without a center");
        let delta = Point::Int(new_via_center[0]).difference_by(&via_center);
        let mut check_ignore_items = Vec::new();
        if !drill_item_mover::check(
            manager,
            board,
            via_id,
            &delta,
            max_recursion_depth,
            max_via_recursion_depth - 1,
            &mut check_ignore_items,
            time_limit,
        ) {
            return CheckDrillResult::NotDrillable;
        }
    }
    let mut result = CheckDrillResult::Drillable;
    if copper_sharing_allowed {
        for current_obstacle in &obstacles {
            if matches!(
                board.get(*current_obstacle).map(|e| &e.data),
                Some(ItemData::Pin { .. })
            ) {
                result = CheckDrillResult::DrillableWithAttachSmd;
                break;
            }
        }
    }
    let trace_piece_count = shape_entries.substitute_trace_count();
    if trace_piece_count == 0 {
        return result;
    }
    if max_recursion_depth <= 0 {
        board.set_shove_failing_obstacle(shape_entries.found_obstacle());
        return CheckDrillResult::NotDrillable;
    }
    if shape_entries.stack_depth() > 1 {
        board.set_shove_failing_obstacle(shape_entries.found_obstacle());
        return CheckDrillResult::NotDrillable;
    }
    let is_orthogonal_mode = matches!(
        pad_shape,
        TileShape::RegularTileShape(RegularTileShape::IntBox(_))
    );
    loop {
        let Some(piece) = shape_entries.next_substitute_trace_piece(manager, board) else {
            break;
        };
        let segment_count = piece.lines.lines.len() as i32 - 2;
        for i in 0..segment_count {
            let current_line = &piece.lines.lines[(i + 1) as usize];
            let current_direction = current_line.direction().clone();
            let is_in_front = if check_only_front {
                in_front_of_pad(
                    current_line,
                    pad_shape,
                    from_side_no,
                    piece.half_width,
                    true,
                )
            } else {
                true
            };
            if is_in_front {
                let current = shape_and_entry_side_core(
                    &piece.lines,
                    piece_compensated_half_width(board.rules(), tree_class, &piece),
                    tree_variant,
                    i,
                    is_orthogonal_mode,
                    true,
                );
                if !crate::trace_shover::check(
                    manager,
                    board,
                    &current.shape,
                    current.from_side,
                    Some(&current_direction),
                    layer,
                    &piece.nets,
                    piece.clearance_class,
                    max_recursion_depth - 1,
                    max_via_recursion_depth,
                    0,
                    time_limit,
                ) {
                    return CheckDrillResult::NotDrillable;
                }
            }
        }
    }
    result
}

/// Java `forcedPad` (`:346-465`): shoves aside traces so that a pad
/// with the input parameters can be inserted without clearance
/// violations. The board IS changed; call [`check_forced_pad`] first.
///
/// `shoveVias` runs with copper sharing FORCED OFF (`:374`); no
/// spring-over runs here (the TraceShover recursion gets spring depth
/// 0, `:431`); degenerate pieces (both corners equal) are skipped.
#[allow(clippy::too_many_arguments)] // the Java read set, kept flat
pub fn forced_pad(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    pad_shape: &TileShape,
    from_side: ShapeEntrySide,
    layer: i32,
    net_numbers: &[i32],
    clearance_class_index: i32,
    copper_sharing_allowed: bool,
    ignore_items: &[ItemId],
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
) -> bool {
    if pad_shape.is_empty() {
        // FRLogger.warn("ShoveTraceAux.forced_pad: padShape is empty")
        // — log-only (D12).
        return true;
    }
    let board_bbox = board.bounding_box().expect("post-parse bounding box");
    if !pad_shape.is_contained_in_int_box(&board_bbox) {
        let outline = board_outline_id(board);
        board.set_shove_failing_obstacle(outline);
        return false;
    }
    if !drill_item_mover::shove_vias(
        manager,
        board,
        pad_shape,
        Some(&from_side),
        layer,
        net_numbers,
        clearance_class_index,
        ignore_items,
        max_recursion_depth,
        max_via_recursion_depth,
        // copper sharing NOT allowed here (the Java call site passes
        // the literal false, `:374`)
        false,
    ) {
        return false;
    }
    let tree = manager.default_tree();
    let (tree_oid_unused, tree_variant, tree_class) = (
        tree.object_id(),
        tree.variant,
        tree.compensated_clearance_class,
    );
    let _ = tree_oid_unused;
    let mut shape_entries = ShapeTraceEntries::new(
        pad_shape.clone(),
        layer,
        net_numbers.to_vec(),
        clearance_class_index,
        Some(from_side),
    );
    let mut obstacles = manager.overlapping_items_with_clearance(
        board,
        0,
        pad_shape,
        layer,
        &[],
        clearance_class_index,
    );
    if !ignore_items.is_empty() {
        obstacles.retain(|id| !ignore_items.contains(id));
    }
    // Java: `storeItems(...) && shoveViaList.isEmpty()` — the
    // short-circuit matters: on a failed store the via list is NOT
    // probed.
    let obstacles_shovable =
        shape_entries.store_items(manager, board, &obstacles, true, copper_sharing_allowed)
            && shape_entries.shove_via_list.is_empty();
    if !obstacles_shovable {
        board.set_shove_failing_obstacle(shape_entries.found_obstacle());
        return false;
    }
    let trace_piece_count = shape_entries.substitute_trace_count();
    if trace_piece_count == 0 {
        return true;
    }
    if max_recursion_depth <= 0 {
        board.set_shove_failing_obstacle(shape_entries.found_obstacle());
        return false;
    }
    let tails_exist_before = contains_trace_tails(manager, board, &obstacles, net_numbers);
    shape_entries.cutout_traces(manager, board, &obstacles);
    let is_orthogonal_mode = matches!(
        pad_shape,
        TileShape::RegularTileShape(RegularTileShape::IntBox(_))
    );
    loop {
        let Some(piece) = shape_entries.next_substitute_trace_piece(manager, board) else {
            break;
        };
        // Java `:411`: degenerate pieces are SKIPPED — no insert, no
        // recursion.
        if crate::items::trace::first_corner(&piece.lines)
            == crate::items::trace::last_corner(&piece.lines)
        {
            continue;
        }
        let current_net_numbers = piece.nets.clone();
        let segment_count = piece.lines.lines.len() as i32 - 2;
        for i in 0..segment_count {
            let current = shape_and_entry_side_core(
                &piece.lines,
                piece_compensated_half_width(board.rules(), tree_class, &piece),
                tree_variant,
                i,
                is_orthogonal_mode,
                false,
            );
            if !crate::trace_shover::insert(
                manager,
                board,
                &current.shape,
                current.from_side,
                layer,
                &current_net_numbers,
                piece.clearance_class,
                ignore_items,
                max_recursion_depth - 1,
                max_via_recursion_depth,
                0,
            ) {
                return false;
            }
        }
        // Java `:434-436`: mark the piece corners in the changed area
        // (a no-op while no marking session is active — Java's
        // `joinChangedArea` guards on null).
        if board.changed_area.is_some() {
            let corner_count = piece.lines.corner_count();
            for i in 0..corner_count as i32 {
                let corner = piece.lines.corner_approx(i);
                if let Some(area) = board.changed_area.as_mut() {
                    area.join_point(&corner, layer);
                }
            }
        }
        let end_corners: Option<(Point, Point)> = if !tails_exist_before {
            Some((
                crate::items::trace::first_corner(&piece.lines).expect("Java NPE: piece corner"),
                crate::items::trace::last_corner(&piece.lines).expect("Java NPE: piece corner"),
            ))
        } else {
            None
        };
        board.insert_item(crate::board::ItemEntry {
            id: piece.id,
            data: ItemData::Trace {
                layer: piece.layer,
                half_width: piece.half_width,
                lines: piece.lines.clone(),
            },
            nets: piece.nets.clone(),
            clearance_class: piece.clearance_class,
            component_id: 0,
            fixed: crate::items::FixedState::Unfixed,
            on_the_board: false,
        });
        manager.insert(board, piece.id);

        // Java `:440-450` — the NULL-CLIP face of the changed-area
        // session (unlike TraceShover.insert's direct deref): the area
        // is read into a local (`null` when the session is inactive)
        // and `normalize(optArea)` runs UNCONDITIONALLY in a swallowing
        // try/catch — the null clip means unbounded normalization.
        let clip = board.changed_area.as_ref().map(|area| area.get_area(layer));
        let _ = normalize(manager, board, piece.id, clip.as_ref());

        if let Some((start_corner, end_corner)) = end_corners {
            for location in [start_corner, end_corner] {
                let tail = get_trace_tail(manager, board, &location, layer, &current_net_numbers);
                if let Some(tail_id) = tail {
                    let connection_items = crate::trace_ops::get_connection_items(
                        manager,
                        board,
                        tail_id,
                        crate::trace_ops::StopConnectionOption::Via,
                    );
                    remove_items(manager, board, &connection_items);
                    for current_net_number in &current_net_numbers {
                        combine_traces(manager, board, *current_net_number);
                    }
                }
            }
        }
    }
    true
}

/// Java `calcFromSide` (`:471-492`): probes a thin check shape along
/// each border of `shape` (offset by `offset`) and answers the first
/// border whose probe corridor is free — first with the full
/// clearance class, then (second sweep) with class 0. All-fail
/// answers [`ShapeEntrySide::NOT_CALCULATED`].
pub(crate) fn calc_from_side(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    shape: &TileShape,
    shape_center: &Point,
    layer: i32,
    offset: i32,
    clearance_class_index: i32,
) -> ShapeEntrySide {
    let empty_arr: [i32; 0] = [];
    let offset_shape = shape.offset(f64::from(offset));
    let border_count = offset_shape.border_line_count();
    for i in 0..border_count {
        let check_shape = calc_check_shape_for_from_side(
            shape,
            shape_center,
            &offset_shape.border_line(i as i32),
        );

        if check_trace_shape(
            manager,
            board,
            &check_shape,
            layer,
            &empty_arr,
            clearance_class_index,
            None,
        ) {
            return ShapeEntrySide::new_precomputed(i as i32, None);
        }
    }
    // try second check without clearance
    for i in 0..border_count {
        let check_shape = calc_check_shape_for_from_side(
            shape,
            shape_center,
            &offset_shape.border_line(i as i32),
        );
        if check_trace_shape(manager, board, &check_shape, layer, &empty_arr, 0, None) {
            return ShapeEntrySide::new_precomputed(i as i32, None);
        }
    }
    ShapeEntrySide::NOT_CALCULATED
}

/// Java `BasicBoard.checkTraceShape` (`BasicBoard.java:989-1053`):
/// can a trace with the input class be inserted at `shape` without
/// clearance violations? Every overlapping tree entry must be free of
/// obstacles for ALL of `net_numbers`; with `contact_pins` the pins
/// outside the set are acid-trap obstacles, and foreign-net trace
/// overlaps under a shared multi-net pin are forgiven when the
/// intersection is contained in the pin shape.
///
/// NOTE the empty-`net_numbers` semantics (the [`calc_from_side`]
/// caller): the per-net loop never clears `is_obstacle`, so ANY
/// overlapping entry answers false — the probe corridor must be
/// completely empty.
pub(crate) fn check_trace_shape(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    shape: &TileShape,
    layer: i32,
    net_numbers: &[i32],
    clearance_class_index: i32,
    contact_pins: Option<&BTreeSet<ItemId>>,
) -> bool {
    let bbox = board.bounding_box().expect("post-parse bounding box");
    if !shape.is_contained_in_int_box(&bbox) {
        return false;
    }
    let tree = manager.default_tree();
    let (tree_oid, tree_variant, tree_class) = (
        tree.object_id(),
        tree.variant,
        tree.compensated_clearance_class,
    );
    let tree_entries = if tree.is_clearance_compensation_used() {
        manager.overlapping_tree_entries(board, 0, shape, layer, &[])
    } else {
        manager.overlapping_tree_entries_with_clearance(
            board,
            0,
            shape,
            layer,
            &[],
            clearance_class_index,
        )
    };
    for tree_entry in tree_entries {
        let Some(current_id) = SearchTreeManager::item_of_key(tree_entry.object_key) else {
            continue; // Java `!(object instanceof Item)` — non-item entry
        };
        if let Some(pins) = contact_pins {
            if pins.contains(&current_id) {
                continue;
            }
            if matches!(
                board.get(current_id).map(|e| &e.data),
                Some(ItemData::Pin { .. })
            ) {
                // The contact pins of the trace should be contained in
                // ignoreItems. Other pins are handled as obstacles to
                // avoid acid traps.
                return false;
            }
        }
        let current_nets = item_nets(board, current_id);
        let mut is_obstacle = true;
        for net in net_numbers {
            // Java `if (!currentItem.isTraceObstacle(netNumbers[i]))` —
            // the VIRTUAL dispatch (`Item.isTraceObstacle` `:170-172`
            // base `!containsNet`, overridden by ConductionArea `:398`
            // `isObstacle && !containsNet`, ComponentObstacleArea `:71`
            // / ViaObstacleArea `:100` both `false`). The flag clears
            // when the item is NOT a trace obstacle for the net: an
            // own-net entry (base face), a NON-obstacle conduction area
            // (a parse-time plane/pour foreign nets route THROUGH,
            // buglog 176), or a place/via keepout. The first port read
            // only the base face — inverted on top of that (bug 149) —
            // and inverted again in T10c; this is the Java face.
            if !board.item_is_trace_obstacle(current_id, *net) {
                is_obstacle = false;
            }
        }
        if is_obstacle
            && matches!(
                board.get(current_id).map(|e| &e.data),
                Some(ItemData::Trace { .. })
            )
            && let Some(pins) = contact_pins
        {
            // check for traces of foreign nets at tie pins, which will
            // be ignored inside the pin shape
            let mut intersection: Option<TileShape> = None;
            for pin_id in pins {
                let pin_nets = item_nets(board, *pin_id);
                // Java `netCount() <= 1 || !sharesNet(currentItem)`.
                if pin_nets.len() <= 1 || !shares_net_no(&pin_nets, &current_nets) {
                    continue;
                }
                if intersection.is_none() {
                    let obstacle_trace_shape =
                        board.tree_shape_precalc(current_id, tree_oid, tree_variant, tree_class)
                            [tree_entry.shape_index_in_object as usize]
                            .clone()
                            .expect("Java IOOBE/NPE: tree shape at the entry index");
                    // Java `intersection = shape.intersection(
                    // obstacleTraceShape)` — computed lazily on the
                    // first qualifying pin.
                    intersection = Some(shape.intersection(&obstacle_trace_shape));
                }
                let pin_first_layer = board.item_first_layer(*pin_id).expect("pin first layer");
                let pin_shape = match board
                    .drill_shape(*pin_id, layer - pin_first_layer)
                    .expect("Java NPE: null pin shape on layer")
                {
                    BoardShape::Tile(tile) => tile,
                    // Java's TileShape-typed getter cannot express a
                    // polygon pin shape here.
                    _ => panic!("Java ClassCastException: non-tile pin shape"),
                };
                if pin_shape.contains_approx(intersection.as_ref().expect("set above")) {
                    is_obstacle = false;
                    break;
                }
            }
        }
        if is_obstacle {
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

#[cfg(test)]
mod tests {
    //! Pins for [`in_front_of_pad`] and the [`check_forced_pad`] via
    //! ladder. World: the TraceShoverProbe replay
    //! (`trace_shover::shover_world`; captures
    //! `logs/M3-T10b/captures/trace_shover_rows.jsonl`).

    use super::*;
    use crate::trace_shover::shover_world;

    /// `inFrontOfPad` case 0 (`ForcedPadAlgo.java:88-92`), pinned at
    /// the verbatim `lineB.x + lineB.x` self-addition bug. Pad: the
    /// bounding octagon of IntBox(0,0,4000,4000) — its diagonals pass
    /// through the box corners, so upperRight = 8000, upperLeft =
    /// -4000. The vertical line x=5000, y in [3100,3200] (a=(5000,3200),
    /// b=(5000,3100)) is "in front of" the pad ONLY through the buggy
    /// third disjunct: min(a.x+a.y, b.x+b.x) = min(8200, 10000) = 8200
    /// clears 8000 + 100*sqrt2 = 8141.4; the CORRECTED min(a.x+a.y,
    /// b.x+b.y) = 8100 would not. Kill target: the fix mutant
    /// `line_b.x + line_b.y` (verdict flips to false).
    #[test]
    fn t9_in_front_of_pad_case_zero_self_addition_bug() {
        let oct = shover_world::ibox(0, 0, 4000, 4000)
            .bounding_octagon()
            .expect("octagon");
        // drift guard for the arithmetic-comment coordinates
        assert_eq!(oct.top_y, 4000);
        assert_eq!(oct.left_x, 0);
        assert_eq!(oct.right_x, 4000);
        assert_eq!(oct.upper_right_diagonal_x, 8000);
        assert_eq!(oct.upper_left_diagonal_x, -4000);
        let pad = TileShape::RegularTileShape(RegularTileShape::IntOctagon(oct));
        let bug_line =
            Polyline::from_two_corners(&shover_world::p(5000, 3200), &shover_world::p(5000, 3100))
                .lines[1]
                .clone();
        assert!(
            in_front_of_pad(&bug_line, &pad, 0, 100, false),
            "the self-addition bug answers true"
        );
        // non-octagon pad shapes fall through to the conservative true.
        assert!(in_front_of_pad(
            &bug_line,
            &shover_world::ibox(0, 0, 4000, 4000),
            0,
            100,
            false,
        ));
    }

    /// The `withSides` second sweep of case 0 (`ForcedPadAlgo.java
    /// :130-133`): the line x=-150, y in [3900,4009] misses the main
    /// sweep (min y 3900 < top+width 4100; max(x-y) -4050 >
    /// -4000-diag; the buggy third disjunct answers -300 >= 8141 no)
    /// but clears the left-side conjunct arm: max(x) -150 <=
    /// left-width -100 AND min(x-y) -4159 <= upperLeft-diag -4141.4.
    #[test]
    fn t9_in_front_of_pad_with_sides_second_sweep() {
        let pad = TileShape::RegularTileShape(RegularTileShape::IntOctagon(
            shover_world::ibox(0, 0, 4000, 4000)
                .bounding_octagon()
                .expect("octagon"),
        ));
        let side_line =
            Polyline::from_two_corners(&shover_world::p(-150, 4009), &shover_world::p(-150, 3900))
                .lines[1]
                .clone();
        assert!(
            !in_front_of_pad(&side_line, &pad, 0, 100, false),
            "the main sweep misses the side shadow"
        );
        assert!(
            in_front_of_pad(&side_line, &pad, 0, 100, true),
            "the withSides sweep clears the left conjunct arm"
        );
    }

    /// The via ladder of `checkForcedPad` (`:317-322`): a spent
    /// maxViaRecursionDepth (0) with the shover world's foreign via
    /// (id 106) in the pad shape fails BEFORE any geometry and reports
    /// the via — the pad shape is copper-free otherwise (piece count
    /// 0), so a ladder fall-through would answer Drillable. Kill
    /// target: `<= 0` -> `< 0`.
    #[test]
    fn t10_check_forced_pad_via_ladder_spent_budget() {
        let (mut manager, mut board, w) = shover_world::build_shover_world();
        let result = check_forced_pad(
            &mut manager,
            &mut board,
            &shover_world::zero_shape(),
            shover_world::zero_side(),
            0,
            &[w.own],
            0,
            false,
            &[],
            10,
            0,
            false,
            None,
        );
        assert_eq!(result, CheckDrillResult::NotDrillable, "capture verdict");
        assert_eq!(
            board.shove_failing_obstacle(),
            Some(w.via),
            "the via is reported"
        );
    }

    /// The (1,-1)-slope discriminator for the case-6 main second
    /// disjunct (spec-review MAJOR-2; jar capture
    /// `logs/M3-T10b/captures/pad_front_rows_run1.jsonl` +
    /// `_run2.jsonl`, double-run
    /// byte-identical, reflection on the private static
    /// `inFrontOfPad`). Pad: bounding octagon of IntBox(0,0,100,100)
    /// (field drift guards below are the capture's `pad` row). On the
    /// anti-diagonal a=(0,210), b=(210,0) the sums are BOTH 210, so
    /// Java's min-of-SUMS = 210 clears 200 + 4*sqrt(2) = 205.657 while
    /// the swapped sum-of-mins = 0+0 = 0 does not — Java answers TRUE
    /// (`main6_hit`), the swapped form FALSE. `main6_miss` (180) keeps
    /// the negative polarity (both forms FALSE); `main6_hit_ws` rides
    /// the withSides=true path to the same TRUE.
    #[test]
    fn t9_in_front_of_pad_side6_min_of_sums() {
        let oct = shover_world::ibox(0, 0, 100, 100)
            .bounding_octagon()
            .expect("octagon");
        // the captured pad-row fields
        assert_eq!(oct.left_x, 0);
        assert_eq!(oct.right_x, 100);
        assert_eq!(oct.bottom_y, 0);
        assert_eq!(oct.top_y, 100);
        assert_eq!(oct.upper_left_diagonal_x, -100);
        assert_eq!(oct.lower_left_diagonal_x, 0);
        assert_eq!(oct.lower_right_diagonal_x, 100);
        assert_eq!(oct.upper_right_diagonal_x, 200);
        let pad = TileShape::RegularTileShape(RegularTileShape::IntOctagon(oct));
        let hit = Polyline::from_two_corners(&shover_world::p(0, 210), &shover_world::p(210, 0))
            .lines[1]
            .clone();
        let miss = Polyline::from_two_corners(&shover_world::p(0, 180), &shover_world::p(180, 0))
            .lines[1]
            .clone();
        assert!(
            in_front_of_pad(&hit, &pad, 6, 4, false),
            "capture main6_hit=true: min-of-sums 210 >= 205.657"
        );
        assert!(
            !in_front_of_pad(&miss, &pad, 6, 4, false),
            "capture main6_miss=false: min-of-sums 180 < 205.657"
        );
        assert!(
            in_front_of_pad(&hit, &pad, 6, 4, true),
            "capture main6_hit_ws=true"
        );
    }

    /// The withSides conj-2 max arm of case 6 (`:185`, Java
    /// max-of-SUMS; jar capture as above, row `ws6_trap`). On
    /// a=(0,220), b=(100,120) the partner conjunct min(y)=120 >=
    /// top+w=120 holds; Java's max-of-sums = max(220, 220) = 220 misses
    /// 200 + 20*sqrt(2) = 228.284 -> FALSE, while the swapped
    /// sum-of-maxes = 100 + 220 = 320 clears it -> TRUE. Kill target:
    /// the sum-of-maxes revert (F2's withSides face). `ws6_positive`
    /// (240) keeps the TRUE polarity through the main second disjunct.
    #[test]
    fn t9_in_front_of_pad_side6_ws_max_of_sums() {
        let pad = TileShape::RegularTileShape(RegularTileShape::IntOctagon(
            shover_world::ibox(0, 0, 100, 100)
                .bounding_octagon()
                .expect("octagon"),
        ));
        let trap = Polyline::from_two_corners(&shover_world::p(0, 220), &shover_world::p(100, 120))
            .lines[1]
            .clone();
        let positive =
            Polyline::from_two_corners(&shover_world::p(0, 240), &shover_world::p(100, 140)).lines
                [1]
            .clone();
        assert!(
            !in_front_of_pad(&trap, &pad, 6, 20, true),
            "capture ws6_trap=false: max-of-sums 220 < 228.284"
        );
        assert!(
            in_front_of_pad(&positive, &pad, 6, 20, true),
            "capture ws6_positive=true: main min-of-sums 240 >= 228.284"
        );
    }
    /// The T17b buglog-176 world: a parse-time `ConductionArea` (a DSN
    /// `(plane ...)` — every one is inserted `isObstacle=false`,
    /// `Structure.java:1113`) under the [`check_trace_shape`] gate.
    /// Java `BasicBoard.checkTraceShape:1017-1022` gates each
    /// overlapping entry on the VIRTUAL `currentItem.isTraceObstacle`
    /// — so a foreign NON-obstacle plane is NOT an obstacle and the
    /// trace shape is insertable THROUGH it (the pre-fix port read the
    /// base net-membership face, blocking the plane and failing the
    /// final insert segments). The flag × net discriminator at the
    /// insert gate: foreign × false → true (THE CROSSING CELL),
    /// own-net × false → true, foreign × true → false,
    /// own-net × true → true.
    #[test]
    fn t17b_check_trace_shape_routes_through_a_non_obstacle_plane() {
        use crate::tree_manager::SearchTreeManager;
        use epic_dsn::reader::{DsnReadResult, read_board};
        use epic_dsn::ses_board::SesBoard;

        const PLANE_DSN: &str = r#"(pcb t17b-plane.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer In1.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary
      (path pcb 0  0 0  10000 0  10000 10000  0 10000  0 0)
    )
    (plane GND
      (polygon In1.Cu 0  1000 1000  9000 1000  9000 9000  1000 9000)
    )
  )
  (network
    (net SIG)
    (net GND)
  )
  (wiring
    (wire (path F.Cu 125  2000 1000  3000 1000) (net SIG))
  )
)
"#;
        let mut ses = SesBoard::new();
        match read_board(PLANE_DSN.as_bytes(), &mut ses) {
            DsnReadResult::Success { warnings } => {
                assert!(warnings.is_empty(), "WARN_COUNT 0, got {warnings:?}");
            }
            other => panic!("expected Success, got {other:?}"),
        }
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);

        // The plane item (a ConductionArea) and the two net numbers.
        let plane_id = board
            .iter_descending()
            .find(|entry| matches!(entry.data, ItemData::ConductionArea { .. }))
            .expect("the plane parsed as a ConductionArea")
            .id;
        let plane_entry = board.get(plane_id).expect("plane");
        let (gnd, parse_flag) = match &plane_entry.data {
            ItemData::ConductionArea { is_obstacle, .. } => (plane_entry.nets[0], *is_obstacle),
            other => panic!("not a plane: {other:?}"),
        };
        let sig = board
            .iter_descending()
            .find(|entry| matches!(entry.data, ItemData::Trace { .. }))
            .expect("the SIG wire")
            .nets[0];
        assert_ne!(sig, gnd, "SIG and GND are different nets");
        assert!(
            !parse_flag,
            "a parse-time plane is NON-obstacle (Structure.java:1113)"
        );

        // The check shape sits INSIDE the plane polygon on In1.Cu
        // (index 1) — in INTERNAL units: `(resolution um 10)` scales
        // every DSN coordinate ×10, so the polygon [1000, 9000] DSN is
        // [10000, 90000] internal and the probe [30000, 70000] sits
        // strictly inside (the T17b debugging that produced this pin:
        // a DSN-unit probe lands in empty space and the world goes
        // vacuous).
        let over_plane = shover_world::ibox(30000, 30000, 70000, 70000);
        // THE CROSSING CELL: a FOREIGN net's trace shape over the
        // NON-obstacle plane is insertable — Java's virtual face
        // clears the obstacle flag (BasicBoard.java:1019).
        assert!(
            check_trace_shape(&mut manager, &mut board, &over_plane, 1, &[sig], 0, None),
            "a foreign non-obstacle plane must not block the trace shape"
        );
        // Own net: not an obstacle under either flag.
        assert!(check_trace_shape(
            &mut manager,
            &mut board,
            &over_plane,
            1,
            &[gnd],
            0,
            None
        ));

        // The router-set flag (`setIsObstacle`): the same foreign
        // shape becomes uninsertable, the own-net one stays clear.
        let mut entry = board.remove_item(plane_id).expect("remove the plane");
        let ItemData::ConductionArea { is_obstacle, .. } = &mut entry.data else {
            unreachable!("checked above");
        };
        *is_obstacle = true;
        board.insert_item(entry);
        assert!(
            !check_trace_shape(&mut manager, &mut board, &over_plane, 1, &[sig], 0, None),
            "an obstacle-flagged plane blocks the foreign trace shape"
        );
        assert!(check_trace_shape(
            &mut manager,
            &mut board,
            &over_plane,
            1,
            &[gnd],
            0,
            None
        ));
    }
}
