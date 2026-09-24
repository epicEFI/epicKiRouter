//! The shove drivers: can a trace segment be forced into a live board,
//! and what must move out of its way (Java
//! `board/optimize/TraceShover.java`, 875 lines, ported in full).
//!
//! Java anchors: static `check` `:57-225` (the max-length probe used by
//! the maze router), instance `check` `:231-411` (the recursive shove
//! feasibility check), `insert` `:417-590` (the recursive shove that
//! MUTATES the board), `getIgnoreItemsAtTiePins` `:592-603`,
//! `springOver` `:611-818`, `springOverObstacles` `:827-874`.
//!
//! ## Port shape (the D1 free-function contract)
//!
//! Java holds `RoutingBoard board` as a field and the four driver
//! classes form a MUTUAL RECURSION CYCLE (`TraceShover.check/insert →
//! DrillItemMover.shoveVias/tryShoveViaPoints → ForcedPadRouter →
//! TraceShover`). The port breaks the object graph: each class becomes
//! a module of free functions with `(manager: &mut SearchTreeManager,
//! board: &mut Board, ...)` threading, exactly like the T10a substrate
//! ([`crate::shape_trace_entries`]).
//!
//! ## Polyline identity
//!
//! `springOver` returns a Java OBJECT — the INPUT object when nothing
//! had to move, a fresh object when the polyline was wrapped. Callers
//! test identity (`newPolyline != polyline`). The port models the
//! three outcomes as [`SpringOverResult`]; [`spring_over_obstacles`]
//! maps them back onto Java's `==`/`!=` decisions verbatim (including
//! the CW-on-reversed-input case, where "unchanged" still flows through
//! the `<=` length comparison and re-reverses to the original value).

use std::cmp::Reverse;
use std::collections::BTreeSet;

use epic_geometry::direction::Direction;
use epic_geometry::int_box::IntBox;
use epic_geometry::line_segment::LineSegment;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::tile_shape::TileShape;

use crate::board::Board;
use crate::contacts::{end_contacts, item_is_tail, items_share_net, shares_net_no, start_contacts};
use crate::drill_item_mover;
use crate::id::ItemId;
use crate::items::trace::{bounding_box as trace_bounding_box, first_corner, last_corner};
use crate::items::{BoardShape, FixedState, ItemData, ObstacleKind};
use crate::rules_surf::{AngleRestriction, BoardRules};
use crate::shape_and_entry_side::shape_and_entry_side_core;
use crate::shape_entry_side::ShapeEntrySide;
use crate::shape_trace_entries::{ShapeTraceEntries, SubstituteTracePiece};
use crate::time_limit::TimeLimit;
use crate::trace_ops::{
    StopConnectionOption, combine, get_connection_items, is_routable, is_shove_fixed, nets_equal,
    remove_item_through_repository,
};
use crate::tree_manager::SearchTreeManager;
use crate::tree_shapes::clearance_compensation_value;

/// The Java identity verdict of one `springOver` call
/// (`TraceShover.java:611-818`): [`SpringOverResult::Unchanged`] means
/// the INPUT object came back (no obstacle, or the offset shape held no
/// entries); [`SpringOverResult::Changed`] carries the fresh wrapped
/// polyline; [`SpringOverResult::Failed`] is Java's `null`.
#[derive(Debug, Clone, PartialEq)]
pub enum SpringOverResult {
    /// The input polyline object was returned unchanged.
    Unchanged,
    /// A fresh polyline object (the wrapped circuit).
    Changed(Polyline),
    /// Java `null` — the spring-over failed.
    Failed,
}

/// Java static `check` (`:57-225`) — the maze router's shove probe.
/// Returns the maximum length of the input segment for which the shove
/// succeeds WITHOUT changing the board; `2147483647.0`
/// (`Integer.MAX_VALUE` widened) on complete success, `0.0` on failure.
#[allow(clippy::too_many_arguments)] // the Java read set, kept flat
pub fn check_max_length(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    line_segment: &LineSegment,
    shove_to_the_left: bool,
    layer: i32,
    net_numbers: &[i32],
    trace_half_width: i32,
    clearance_class_index: i32,
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
) -> f64 {
    let tree = manager.default_tree();
    let (tree_oid, tree_variant, tree_class) = (
        tree.object_id(),
        tree.variant,
        tree.compensated_clearance_class,
    );
    let compensated = tree.is_clearance_compensation_used();
    let mut trace_half_width = trace_half_width;
    if compensated {
        // searchTree.clearanceCompensationValue(clearanceClassIndex, layer)
        trace_half_width +=
            clearance_compensation_value(board.rules(), clearance_class_index, tree_class, layer);
    }
    let segment_polyline = line_segment.to_polyline();
    let trace_shapes = segment_polyline.offset_shapes(
        trace_half_width,
        0,
        segment_polyline.lines.len() as i32 - 1,
    );
    if trace_shapes.len() != 1 {
        // FRLogger.warn("TraceShover.check: traceShape count 1 expected")
        // — log-only (D12).
        return 0.0;
    }
    let trace_shape = &trace_shapes[0];
    if trace_shape.is_empty() {
        // FRLogger.warn("TraceShover.check: traceShape is empty") —
        // log-only (D12).
        return 0.0;
    }
    let board_bbox = board.bounding_box().expect("post-parse bounding box");
    if !trace_shape.is_contained_in_int_box(&board_bbox) {
        return 0.0;
    }
    let from_side = ShapeEntrySide::from_line_segment(line_segment, trace_shape, shove_to_the_left);
    let mut entries = ShapeTraceEntries::new(
        trace_shape.clone(),
        layer,
        net_numbers.to_vec(),
        clearance_class_index,
        Some(from_side),
    );
    let obstacles = manager.overlapping_items_with_clearance(
        board,
        0,
        trace_shape,
        layer,
        &[],
        clearance_class_index,
    );
    // Java passes the collection straight in (no tie-pin filter here).
    if !entries.store_items(manager, board, &obstacles, false, true)
        || entries.trace_tails_in_shape()
    {
        return 0.0;
    }
    let trace_piece_count = entries.substitute_trace_count();

    if entries.stack_depth() > 1 {
        // NOTE: the static check does NOT report setShoveFailingObstacle
        // (unlike the instance check).
        return 0.0;
    }

    let start_corner_approx = line_segment.start_point_approx();
    let end_corner_approx = line_segment.end_point_approx();
    let segment_length = end_corner_approx.distance(&start_corner_approx);

    let mut result = f64::from(i32::MAX);

    // check, if the obstacle vias can be shoved

    let via_list = entries.shove_via_list.clone();
    for via_id in via_list {
        let via_nets = item_nets(board, via_id);
        if shares_net_no(&via_nets, net_numbers) {
            continue;
        }
        let mut shove_via_ok = false;
        if max_via_recursion_depth > 0 {
            let try_centers = drill_item_mover::try_shove_via_points(
                manager,
                board,
                trace_shape,
                layer,
                via_id,
                clearance_class_index,
                false,
            );
            if try_centers.is_empty() {
                return 0.0;
            }
            let via_center = board.drill_center(via_id).expect("live via center");
            let delta = Point::Int(try_centers[0]).difference_by(&via_center);
            let mut ignore_items = Vec::new();
            shove_via_ok = drill_item_mover::check(
                manager,
                board,
                via_id,
                &delta,
                max_recursion_depth,
                max_via_recursion_depth - 1,
                &mut ignore_items,
                None,
            );
        }

        if !shove_via_ok {
            let via_center_approx = board
                .drill_center(via_id)
                .expect("live via center")
                .to_float();
            let mut projection =
                start_corner_approx.scalar_product(&end_corner_approx, &via_center_approx);
            projection /= segment_length;
            let first_layer = board.item_first_layer(via_id).expect("via first layer");
            let via_tree_shape =
                board.tree_shape_precalc(via_id, tree_oid, tree_variant, tree_class)
                    [(layer - first_layer) as usize]
                    .clone()
                    .expect("Java NPE: null via tree shape");
            let via_box = via_tree_shape.bounding_box();
            let via_radius = 0.5 * via_box.max_width();
            let mut current_ok_length = projection - via_radius - f64::from(trace_half_width);
            if !compensated {
                let via_class = board.item_clearance_class(via_id).expect("live via class");
                current_ok_length -=
                    f64::from(board.clearance_value(clearance_class_index, via_class, layer));
            }
            if current_ok_length <= 0.0 {
                return 0.0;
            }
            result = result.min(current_ok_length);
        }
    }
    if trace_piece_count == 0 {
        return result;
    }
    if max_recursion_depth <= 0 {
        return 0.0;
    }

    let line_direction = line_segment.get_line().direction().clone();
    loop {
        let Some(piece) = entries.next_substitute_trace_piece(manager, board) else {
            break;
        };
        let segment_count = piece.lines.lines.len() as i32 - 2;
        for i in 0..segment_count {
            let mut current_line_segment = LineSegment::from_polyline(&piece.lines, i + 1);
            if shove_to_the_left {
                // swap the line segment to get the correct shove length
                // in case it is smaller than the length of the whole
                // line segment.
                current_line_segment = current_line_segment.opposite();
            }
            let is_in_front = current_line_segment.get_line().direction() == &line_direction;
            if is_in_front {
                let shove_ok_length = check_max_length(
                    manager,
                    board,
                    &current_line_segment,
                    shove_to_the_left,
                    layer,
                    &piece.nets,
                    piece.half_width,
                    piece.clearance_class,
                    max_recursion_depth - 1,
                    max_via_recursion_depth,
                );
                if shove_ok_length < f64::from(i32::MAX) {
                    if shove_ok_length <= 0.0 {
                        return 0.0;
                    }
                    let mut projection = start_corner_approx
                        .scalar_product(
                            &end_corner_approx,
                            &current_line_segment.start_point_approx(),
                        )
                        .min(start_corner_approx.scalar_product(
                            &end_corner_approx,
                            &current_line_segment.end_point_approx(),
                        ));
                    projection /= segment_length;
                    let mut current_ok_length = shove_ok_length + projection
                        - f64::from(trace_half_width)
                        - f64::from(piece.half_width);
                    if compensated {
                        current_ok_length -= f64::from(clearance_compensation_value(
                            board.rules(),
                            piece.clearance_class,
                            tree_class,
                            layer,
                        ));
                    } else {
                        current_ok_length -= f64::from(board.clearance_value(
                            clearance_class_index,
                            piece.clearance_class,
                            layer,
                        ));
                    }
                    if current_ok_length <= 0.0 {
                        return 0.0;
                    }
                    result = current_ok_length.min(result);
                }
                // Java breaks after the FIRST in-front segment of the
                // piece (`:220`) — the `break` sits inside the
                // `isInFront` arm.
                break;
            }
        }
    }
    result
}

/// Java instance `check` (`:231-411`). Checks if a shove with the input
/// parameters is possible without clearance violations; `dir` is used
/// internally to prevent the check from bouncing back (`None` on
/// top-level calls). Returns false if the shove failed; the failing
/// obstacle is reported through `board.setShoveFailingObstacle`.
#[allow(clippy::too_many_arguments)] // the Java read set, kept flat
pub fn check(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_shape: &TileShape,
    from_side: Option<ShapeEntrySide>,
    dir: Option<&Direction>,
    layer: i32,
    net_numbers: &[i32],
    clearance_class_index: i32,
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
    max_spring_over_recursion_depth: i32,
    time_limit: Option<&TimeLimit>,
) -> bool {
    if let Some(limit) = time_limit
        && limit.limit_exceeded()
    {
        return false;
    }

    if trace_shape.is_empty() {
        // FRLogger.warn("ShoveTraceAux.check: traceShape is empty") —
        // log-only (D12).
        return true;
    }
    let board_bbox = board.bounding_box().expect("post-parse bounding box");
    if !trace_shape.is_contained_in_int_box(&board_bbox) {
        let outline = board_outline_id(board);
        board.set_shove_failing_obstacle(outline);
        return false;
    }
    let mut entries = ShapeTraceEntries::new(
        trace_shape.clone(),
        layer,
        net_numbers.to_vec(),
        clearance_class_index,
        from_side,
    );
    let tree = manager.default_tree();
    let (_tree_oid, tree_variant, tree_class) = (
        tree.object_id(),
        tree.variant,
        tree.compensated_clearance_class,
    );
    let mut obstacles = manager.overlapping_items_with_clearance(
        board,
        0,
        trace_shape,
        layer,
        &[],
        clearance_class_index,
    );
    let ignore_set = get_ignore_items_at_tie_pins(manager, board, trace_shape, layer, net_numbers);
    // Java `obstacles.removeAll(ignoreItems)` — order-preserving.
    obstacles.retain(|id| !ignore_set.contains(id));
    if !entries.store_items(manager, board, &obstacles, false, true) {
        board.set_shove_failing_obstacle(entries.found_obstacle());
        return false;
    }
    let trace_piece_count = entries.substitute_trace_count();

    // The [shove_check_obstacles] FRLogger.trace block (`:268-303`) is
    // an oracle-native diagnostic — the spike greps the rows from the
    // Java run; the port carries no logging (D12).

    if entries.stack_depth() > 1 {
        board.set_shove_failing_obstacle(entries.found_obstacle());
        return false;
    }
    let shape_radius = 0.5 * trace_shape.bounding_box().min_width();

    // check, if the obstacle vias can be shoved

    let via_list = entries.shove_via_list.clone();
    for via_id in via_list {
        let via_nets = item_nets(board, via_id);
        if shares_net_no(&via_nets, net_numbers) {
            continue;
        }
        if max_via_recursion_depth <= 0 {
            board.set_shove_failing_obstacle(Some(via_id));
            return false;
        }
        let via_center = board.drill_center(via_id).expect("live via center");
        let current_via_center = via_center.to_float();
        let try_via_centers = drill_item_mover::try_shove_via_points(
            manager,
            board,
            trace_shape,
            layer,
            via_id,
            clearance_class_index,
            true,
        );

        // NOTE the RAW shape on layer (getShapeOnLayer), not the tree
        // shape (unlike the static check above).
        let first_layer = board.item_first_layer(via_id).expect("via first layer");
        let via_raw_shape = board
            .drill_shape(via_id, layer - first_layer)
            .expect("Java NPE: null via shape on layer");
        let max_dist = 0.5 * shape_bounding_box(&via_raw_shape).max_width() + shape_radius;
        let max_dist_square = max_dist * max_dist;
        let mut shove_via_ok = false;
        for (i, try_center) in try_via_centers.iter().enumerate() {
            if i == 0
                || current_via_center.distance_square(&try_center.to_float()) <= max_dist_square
            {
                let delta = Point::Int(*try_center).difference_by(&via_center);
                let mut ignore_items = Vec::new();
                if drill_item_mover::check(
                    manager,
                    board,
                    via_id,
                    &delta,
                    max_recursion_depth,
                    max_via_recursion_depth - 1,
                    &mut ignore_items,
                    time_limit,
                ) {
                    shove_via_ok = true;
                    break;
                }
            }
        }
        if !shove_via_ok {
            // Java returns false WITHOUT reporting a failing obstacle
            // here (`:348-350`).
            return false;
        }
    }

    if trace_piece_count == 0 {
        return true;
    }
    if max_recursion_depth <= 0 {
        board.set_shove_failing_obstacle(entries.found_obstacle());
        return false;
    }

    let is_orthogonal_mode = matches!(
        trace_shape,
        TileShape::RegularTileShape(RegularTileShape::IntBox(_))
    );
    // Java reads maxSpringOverRecursionDepth into a MUTABLE LOCAL of
    // THIS invocation; a successful spring-over decrements it (`:539`
    // here, `:384` in check) for the REST of the invocation — the NEXT
    // piece of the loop sees the decremented value at its gate and in
    // its recursive calls. The binding therefore sits ABOVE the piece
    // loop (per-invocation scope, not per-piece scope; pinned by t13).
    let mut spring_over_depth = max_spring_over_recursion_depth;
    loop {
        let Some(mut piece) = entries.next_substitute_trace_piece(manager, board) else {
            break;
        };
        if spring_over_depth > 0 {
            let compensated_half_width =
                piece_compensated_half_width(board.rules(), tree_class, &piece);
            let spring_result = spring_over(
                manager,
                board,
                &piece.lines,
                compensated_half_width,
                layer,
                &piece.nets,
                piece.clearance_class,
                false,
                spring_over_depth,
                None,
            );
            match spring_result {
                SpringOverResult::Failed => {
                    // spring_over did not work
                    return false;
                }
                SpringOverResult::Changed(new_polyline) => {
                    // spring_over changed something
                    spring_over_depth -= 1;
                    piece.lines = new_polyline;
                }
                SpringOverResult::Unchanged => {}
            }
        }
        let segment_count = piece.lines.lines.len() as i32 - 2;
        for i in 0..segment_count {
            let current_direction = piece.lines.lines[(i + 1) as usize].direction().clone();
            let is_in_front = match dir {
                None => true,
                Some(d) => *d == current_direction,
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
                if !check(
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
                    spring_over_depth,
                    time_limit,
                ) {
                    return false;
                }
            }
        }
    }
    true
}

/// Java `insert` (`:417-590`). Puts in a trace segment with the input
/// parameters and shoves obstacles out of the way. If the shove does
/// not work, the database may be damaged — call [`check`] first.
#[allow(clippy::too_many_arguments)] // the Java read set, kept flat
pub fn insert(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_shape: &TileShape,
    from_side: Option<ShapeEntrySide>,
    layer: i32,
    net_numbers: &[i32],
    clearance_class_index: i32,
    ignore_items: &[ItemId],
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
    max_spring_over_recursion_depth: i32,
) -> bool {
    if trace_shape.is_empty() {
        // FRLogger.warn("ShoveTraceAux.insert: traceShape is empty") —
        // log-only (D12).
        return true;
    }
    let board_bbox = board.bounding_box().expect("post-parse bounding box");
    if !trace_shape.is_contained_in_int_box(&board_bbox) {
        let outline = board_outline_id(board);
        board.set_shove_failing_obstacle(outline);
        return false;
    }
    if !drill_item_mover::shove_vias(
        manager,
        board,
        trace_shape,
        from_side.as_ref(),
        layer,
        net_numbers,
        clearance_class_index,
        ignore_items,
        max_recursion_depth,
        max_via_recursion_depth,
        true,
    ) {
        return false;
    }
    let mut entries = ShapeTraceEntries::new(
        trace_shape.clone(),
        layer,
        net_numbers.to_vec(),
        clearance_class_index,
        from_side,
    );
    let tree = manager.default_tree();
    let (_tree_oid_unused, tree_variant, tree_class) = (
        tree.object_id(),
        tree.variant,
        tree.compensated_clearance_class,
    );
    let mut obstacles = manager.overlapping_items_with_clearance(
        board,
        0,
        trace_shape,
        layer,
        &[],
        clearance_class_index,
    );
    let ignore_set = get_ignore_items_at_tie_pins(manager, board, trace_shape, layer, net_numbers);
    obstacles.retain(|id| !ignore_set.contains(id));
    // Java computes storeItems FIRST (its side effects collect the via
    // list), then tests the list, then the verdict.
    let obstacles_shovable = entries.store_items(manager, board, &obstacles, false, true);
    if !entries.shove_via_list.is_empty() {
        // Java tests the shove-via list BEFORE the shovable verdict and
        // reports the FIRST collected via (`:456-460`).
        board.set_shove_failing_obstacle(Some(entries.shove_via_list[0]));
        return false;
    }
    if !obstacles_shovable {
        board.set_shove_failing_obstacle(entries.found_obstacle());
        return false;
    }
    let trace_piece_count = entries.substitute_trace_count();

    // The [shove_insert_obstacles] FRLogger.trace block (`:466-502`) is
    // oracle-native — served from the Java run by the spike (D12).

    if trace_piece_count == 0 {
        return true;
    }
    if max_recursion_depth <= 0 {
        board.set_shove_failing_obstacle(entries.found_obstacle());
        return false;
    }
    let tails_exist_before = contains_trace_tails(manager, board, &obstacles, net_numbers);
    entries.cutout_traces(manager, board, &obstacles);
    let is_orthogonal_mode = matches!(
        trace_shape,
        TileShape::RegularTileShape(RegularTileShape::IntBox(_))
    );
    // Per-invocation spring-over budget, shared across the piece loop —
    // Java `:539` decrements the method-parameter local for the REST of
    // the invocation (see the matching comment in `check`).
    let mut spring_over_depth = max_spring_over_recursion_depth;
    loop {
        let Some(mut piece) = entries.next_substitute_trace_piece(manager, board) else {
            break;
        };
        // Java `:518`: degenerate pieces (both corners equal) are
        // SKIPPED — no insert, no recursion (unlike `check`, which has
        // no such arm).
        if first_corner(&piece.lines) == last_corner(&piece.lines) {
            continue;
        }
        if spring_over_depth > 0 {
            let compensated_half_width =
                piece_compensated_half_width(board.rules(), tree_class, &piece);
            let spring_result = spring_over(
                manager,
                board,
                &piece.lines,
                compensated_half_width,
                layer,
                &piece.nets,
                piece.clearance_class,
                false,
                spring_over_depth,
                None,
            );
            match spring_result {
                SpringOverResult::Failed => {
                    return false;
                }
                SpringOverResult::Changed(new_polyline) => {
                    spring_over_depth -= 1;
                    piece.lines = new_polyline;
                }
                SpringOverResult::Unchanged => {}
            }
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
            if !insert(
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
                spring_over_depth,
            ) {
                return false;
            }
        }
        // Java `:560-562`: mark the piece corners in the changed area
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
                first_corner(&piece.lines).expect("Java NPE: piece corner"),
                last_corner(&piece.lines).expect("Java NPE: piece corner"),
            ))
        } else {
            None
        };
        // Java `board.insertItem(currentSubstituteTrace)` — insert the
        // piece under its POP-TIME id, then register it in the trees.
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
            fixed: FixedState::Unfixed,
            on_the_board: false,
        });
        manager.insert(board, piece.id);

        // Java `:545-550` — the DIRECT-DEREF face of the changed-area
        // session: `currentSubstituteTrace.normalize(board.changedArea
        // .getArea(layer))` inside a swallowing try/catch. With the
        // session null the `getArea` dereference NPEs INSIDE the try
        // block and the WHOLE normalization is skipped (demoted to
        // FRLogger.error "Couldn't normalize trace."; bug-144). Port the
        // CONDITIONAL: skip-when-no-session, run-when-marking with the
        // area as the clip — normalizing with a null clip here would
        // merge the piece's perpendicular end contacts via
        // `PolylineTrace.combine()`, which ignores the clip.
        if let Some(area) = board.changed_area.as_ref() {
            let clip = area.get_area(layer);
            // Java discards the normalize verdict here; internal
            // failures surface as the caught exception path.
            let _ = crate::trace_ops::normalize(manager, board, piece.id, Some(&clip));
        }

        if let Some((start_corner, end_corner)) = end_corners {
            for location in [start_corner, end_corner] {
                let tail = get_trace_tail(manager, board, &location, layer, &current_net_numbers);
                if let Some(tail_id) = tail {
                    let connection_items =
                        get_connection_items(manager, board, tail_id, StopConnectionOption::Via);
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

/// Java `getIgnoreItemsAtTiePins` (`:592-603`): every contact of an
/// OWN-NET pin overlapping the shape is ignored by the shove — the
/// tie-pin stub exit must not block the shove of its own net. The Java
/// `TreeSet` ordering is unobservable through the `removeAll` consumer;
/// the port returns a `BTreeSet` for the same dedup semantics.
pub fn get_ignore_items_at_tie_pins(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_shape: &TileShape,
    layer: i32,
    net_numbers: &[i32],
) -> BTreeSet<ItemId> {
    let mut result = BTreeSet::new();
    let overlaps = manager.overlapping_objects(board, 0, trace_shape, layer, &[]);
    for id in overlaps {
        let is_own_net_pin = match board.get(id) {
            Some(entry) => {
                matches!(entry.data, ItemData::Pin { .. })
                    && shares_net_no(&entry.nets, net_numbers)
            }
            None => false,
        };
        if is_own_net_pin {
            result.extend(get_all_contacts_on_layer(manager, board, id, layer));
        }
    }
    result
}

/// Java `springOver` (`:611-818`). Checks if there are obstacles in the
/// way of the polyline and tries to wrap it around them in the counter
/// clockwise sense. If `contact_pins` is `Some`, all pins NOT contained
/// in the set are regarded as obstacles, even of the own net.
#[allow(clippy::too_many_arguments)] // the Java read set, kept flat
pub(crate) fn spring_over(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    polyline: &Polyline,
    half_width: i32,
    layer: i32,
    net_numbers: &[i32],
    clearance_class_index: i32,
    over_connected_pins: bool,
    recursion_depth: i32,
    contact_pins: Option<&BTreeSet<ItemId>>,
) -> SpringOverResult {
    let mut found_obstacle: Option<ItemId> = None;
    let mut found_obstacle_bounding_box = IntBox::EMPTY;
    let tree = manager.default_tree();
    let (tree_oid, tree_variant, tree_class) = (
        tree.object_id(),
        tree.variant,
        tree.compensated_clearance_class,
    );
    let compensated = tree.is_clearance_compensation_used();
    let check_net_no_arr: &[i32] = match contact_pins {
        None => net_numbers,
        Some(_) => &[],
    };
    let line_count = polyline.lines.len() as i32 - 2;
    for i in 0..line_count {
        let current_shape = polyline
            .offset_shape(half_width, i)
            .expect("offset shape of a live segment (Java NPE site)");
        let obstacles = manager.overlapping_items_with_clearance(
            board,
            0,
            &current_shape,
            layer,
            check_net_no_arr,
            clearance_class_index,
        );
        for current_item in obstacles {
            let is_obstacle =
                spring_over_is_obstacle(manager, board, current_item, net_numbers, contact_pins);

            if is_obstacle {
                let current_bounding_box = item_bounding_box(board, current_item);
                match found_obstacle {
                    None => {
                        found_obstacle = Some(current_item);
                        found_obstacle_bounding_box = current_bounding_box;
                    }
                    Some(found) if found != current_item => {
                        // check, if 1 obstacle is contained in the other
                        // obstacle and take the bigger obstacle in this
                        // case. That may happen in case of fixed vias
                        // inside of pins.
                        //
                        // Argument direction is Java's
                        // (`TraceShover.java:675-682` with
                        // `IntBox.contains(other)` ==
                        // `other.isContainedIn(this)`, IntBox.java:377):
                        // arm 1 fires when the CURRENT box CONTAINS the
                        // FOUND box — the current (bigger) obstacle wins,
                        // so the scan keeps the OUTERMOST nesting box;
                        // arm 2 fails when neither contains the other.
                        if found_obstacle_bounding_box.intersects(&current_bounding_box) {
                            if found_obstacle_bounding_box.is_contained_in(&current_bounding_box) {
                                found_obstacle = Some(current_item);
                                found_obstacle_bounding_box = current_bounding_box;
                            } else if !current_bounding_box
                                .is_contained_in(&found_obstacle_bounding_box)
                            {
                                return SpringOverResult::Failed;
                            }
                        }
                    }
                    Some(_) => {}
                }
            }
        }
        if found_obstacle.is_some() {
            break;
        }
    }
    let Some(found_obstacle) = found_obstacle else {
        // no obstacle in the way, nothing to do — the INPUT object
        // comes back (Java identity).
        return SpringOverResult::Unchanged;
    };

    let found_is_outline = matches!(
        board.get(found_obstacle).map(|e| &e.data),
        Some(ItemData::BoardOutline { .. })
    );
    let found_is_unfixed_trace = match board.get(found_obstacle).map(|e| &e.data) {
        Some(ItemData::Trace { .. }) => !is_shove_fixed(board, found_obstacle),
        _ => false,
    };
    if recursion_depth <= 0 || found_is_outline || found_is_unfixed_trace {
        board.set_shove_failing_obstacle(Some(found_obstacle));
        return SpringOverResult::Failed;
    }
    let mut try_spring_over = true;
    if !over_connected_pins {
        // Check if the obstacle has a trace contact on layer
        let contacts_on_layer = get_all_contacts_on_layer(manager, board, found_obstacle, layer);
        for contact in contacts_on_layer {
            if matches!(
                board.get(contact).map(|e| &e.data),
                Some(ItemData::Trace { .. })
            ) {
                try_spring_over = false;
                break;
            }
        }
    }
    let mut obstacle_shape: Option<TileShape> = None;
    if try_spring_over {
        let found_kind = board.get(found_obstacle).map(|e| &e.data);
        match found_kind {
            Some(ItemData::ObstacleArea { .. })
            | Some(ItemData::ConductionArea { .. })
            | Some(ItemData::Trace { .. }) => {
                // Java `instanceof ObstacleArea` covers ConductionArea
                // (a subclass); treeShapeCount is the precalc length.
                let tree_shapes =
                    board.tree_shape_precalc(found_obstacle, tree_oid, tree_variant, tree_class);
                if tree_shapes.len() == 1 {
                    obstacle_shape = Some(
                        tree_shapes[0]
                            .clone()
                            .expect("Java NPE: null single tree shape"),
                    );
                } else {
                    try_spring_over = false;
                }
            }
            Some(ItemData::Pin { .. }) | Some(ItemData::Via { .. }) => {
                let first_layer = board
                    .item_first_layer(found_obstacle)
                    .expect("drill first layer");
                obstacle_shape = Some(
                    board.tree_shape_precalc(found_obstacle, tree_oid, tree_variant, tree_class)
                        [(layer - first_layer) as usize]
                        .clone()
                        .expect("Java NPE: null drill tree shape on layer"),
                );
            }
            // Java NPE: a ComponentOutline obstacle has no branch here —
            // `obstacleShape` stays null and the cast below throws.
            _ => {}
        }
    }
    if !try_spring_over {
        board.set_shove_failing_obstacle(Some(found_obstacle));
        return SpringOverResult::Failed;
    }
    let obstacle_shape = obstacle_shape.expect("Java NPE: no obstacle shape branch");
    let offset_shape = if compensated {
        let offset = f64::from(half_width + 1);
        obstacle_shape.enlarge(offset)
    } else {
        // enlarge the shape in 2 steps for symmetry reasons
        let offset = f64::from(half_width + 1);
        let half_cl_offset = 0.5
            * f64::from(
                board.clearance_value(
                    board
                        .item_clearance_class(found_obstacle)
                        .expect("live obstacle class"),
                    clearance_class_index,
                    layer,
                ),
            );
        let enlarged = obstacle_shape.enlarge(offset + half_cl_offset);
        enlarged.enlarge(half_cl_offset)
    };
    let offset_shape = match board.rules().trace_angle_restriction {
        AngleRestriction::NinetyDegree => {
            TileShape::RegularTileShape(RegularTileShape::IntBox(offset_shape.bounding_box()))
        }
        AngleRestriction::FortyfiveDegree => {
            let octagon = offset_shape
                .bounding_octagon()
                .expect("bounding octagon of a non-empty shape");
            TileShape::RegularTileShape(RegularTileShape::IntOctagon(octagon))
        }
        AngleRestriction::None => offset_shape,
    };

    if offset_shape.contains_inside(&first_corner(polyline).expect("Java NPE: polyline corner"))
        || offset_shape.contains_inside(&last_corner(polyline).expect("Java NPE: polyline corner"))
    {
        // can happen with clearance compensation off because of
        // asymmetry in calculations with the offset shapes
        board.set_shove_failing_obstacle(Some(found_obstacle));
        return SpringOverResult::Failed;
    }
    let entries = offset_shape.entrance_points(polyline);
    if entries.is_empty() {
        return SpringOverResult::Unchanged; // no obstacle
    }
    if entries.len() < 2 {
        board.set_shove_failing_obstacle(Some(found_obstacle));
        return SpringOverResult::Failed;
    }
    let first_intersection_side_no = entries[0].1;
    let last_intersection_side_no = entries[entries.len() - 1].1;
    let first_intersection_line_no = entries[0].0;
    let last_intersection_line_no = entries[entries.len() - 1].0;
    let mut side_diff = last_intersection_side_no - first_intersection_side_no;
    let border_line_count = offset_shape.border_line_count() as i32;
    if side_diff < 0 {
        side_diff += border_line_count;
    } else if side_diff == 0 {
        let compare_corner = offset_shape
            .corner_approx(first_intersection_side_no)
            .expect("corner of a legal side");
        let first_intersection = polyline.lines[first_intersection_line_no as usize]
            .intersection_approx(&offset_shape.border_line(first_intersection_side_no));
        let second_intersection = polyline.lines[last_intersection_line_no as usize]
            .intersection_approx(&offset_shape.border_line(last_intersection_side_no));
        if compare_corner.distance(&second_intersection)
            < compare_corner.distance(&first_intersection)
        {
            side_diff += border_line_count;
        }
    }
    let piece_len = usize::try_from(side_diff + 3)
        .expect("Java NegativeArraySizeException for a negative side difference");
    let mut substitute_lines = Vec::with_capacity(piece_len);
    substitute_lines.push(polyline.lines[first_intersection_line_no as usize].clone());
    let mut current_edge_line_no = first_intersection_side_no;
    for _ in 1..=(side_diff + 1) {
        substitute_lines.push(offset_shape.border_line(current_edge_line_no));
        if current_edge_line_no == border_line_count - 1 {
            current_edge_line_no = 0;
        } else {
            current_edge_line_no += 1;
        }
    }
    substitute_lines.push(polyline.lines[last_intersection_line_no as usize].clone());
    let substitute_polyline = Polyline::new(substitute_lines);
    // build a circuit around the offsetShape in counter clock sense
    // from the first intersection point to the second intersection point
    let pieces = offset_shape.cutout_polyline(polyline);
    let mut result = substitute_polyline;
    if !pieces.is_empty() {
        result = pieces[0].combine(Some(&result));
    }
    if pieces.len() > 1 {
        result = result.combine(Some(&pieces[1]));
    }
    let inner = spring_over(
        manager,
        board,
        &result,
        half_width,
        layer,
        net_numbers,
        clearance_class_index,
        over_connected_pins,
        recursion_depth - 1,
        contact_pins,
    );
    // The outer frame ALWAYS hands back a fresh object (`result` differs
    // from the input), whatever object the recursion returned.
    match inner {
        SpringOverResult::Unchanged => SpringOverResult::Changed(result),
        SpringOverResult::Changed(value) => SpringOverResult::Changed(value),
        SpringOverResult::Failed => SpringOverResult::Failed,
    }
}

/// The obstacle classification ladder of `springOver` (`:634-664`).
/// The shove-fixed trace arm walks the item's normal contacts, which
/// needs the search-tree manager (the Rust contact surface threads it
/// explicitly where Java reads `getNormalContacts()` off the item).
fn spring_over_is_obstacle(
    manager: &SearchTreeManager,
    board: &mut Board,
    current_item: ItemId,
    net_numbers: &[i32],
    contact_pins: Option<&BTreeSet<ItemId>>,
) -> bool {
    let Some(entry) = board.get(current_item) else {
        return false; // Java would NPE; unreachable through the seam
    };
    if shares_net_no(&entry.nets, net_numbers) {
        // to avoid acid traps
        matches!(entry.data, ItemData::Pin { .. })
            && contact_pins.is_some_and(|pins| !pins.contains(&current_item))
    } else if let ItemData::ConductionArea { is_obstacle, .. } = &entry.data {
        *is_obstacle
    } else if matches!(
        entry.data,
        ItemData::ObstacleArea {
            kind: ObstacleKind::ViaObstacleArea,
            ..
        } | ItemData::ObstacleArea {
            kind: ObstacleKind::ComponentObstacleArea,
            ..
        }
    ) {
        false
    } else if matches!(entry.data, ItemData::Trace { .. }) {
        if is_shove_fixed(board, current_item) {
            // check for a shove fixed trace exit stub, which has to be
            // ignored at a tie pin.
            let mut is_obstacle = true;
            for contact in crate::contacts::item_normal_contacts(manager, board, current_item) {
                let contact_nets = item_nets(board, contact);
                if shares_net_no(&contact_nets, net_numbers) {
                    is_obstacle = false;
                }
            }
            is_obstacle
        } else {
            // an unfixed trace can be pushed aside eventually
            false
        }
    } else {
        // an unfixed via can be pushed aside eventually
        !is_routable(board, current_item)
    }
}

/// Java `springOverObstacles` (`:827-874`). Looks for the SHORTEST way
/// around the obstacles: counter clockwise first, then clockwise on the
/// reversed polyline; `<=` on the approximate lengths means CLOCKWISE
/// wins ties (the result is then the reversed CW object re-reversed).
/// `None` when both directions fail.
#[allow(clippy::too_many_arguments)] // the Java read set, kept flat
pub fn spring_over_obstacles(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    polyline: &Polyline,
    half_width: i32,
    layer: i32,
    net_numbers: &[i32],
    clearance_class_index: i32,
    contact_pins: Option<&BTreeSet<ItemId>>,
) -> Option<Polyline> {
    const MAX_SPRING_OVER_RECURSION_DEPTH: i32 = 20;
    let counter_clock_wise_result = spring_over(
        manager,
        board,
        polyline,
        half_width,
        layer,
        net_numbers,
        clearance_class_index,
        true,
        MAX_SPRING_OVER_RECURSION_DEPTH,
        contact_pins,
    );
    // Java `counterClockWiseResult == polyline` — object identity: the
    // no-obstacle early return.
    if let SpringOverResult::Unchanged = counter_clock_wise_result {
        return Some(polyline.clone()); // no obstacle
    }
    let SpringOverResult::Changed(counter_clock_wise_polyline) = counter_clock_wise_result else {
        // the CCW attempt failed; Java still runs the CW attempt and
        // its `null` propagates (both null -> null)
        let reversed = polyline.reverse();
        let clock_wise_result = spring_over(
            manager,
            board,
            &reversed,
            half_width,
            layer,
            net_numbers,
            clearance_class_index,
            true,
            MAX_SPRING_OVER_RECURSION_DEPTH,
            contact_pins,
        );
        return match clock_wise_result {
            // `clockWiseResult.reverse()` — "unchanged" means the
            // reversed INPUT object, whose reverse is the original
            // value.
            SpringOverResult::Unchanged => Some(reversed.reverse()),
            SpringOverResult::Changed(value) => Some(value.reverse()),
            SpringOverResult::Failed => None,
        };
    };

    let clock_wise_input = polyline.reverse();
    let clock_wise_result = spring_over(
        manager,
        board,
        &clock_wise_input,
        half_width,
        layer,
        net_numbers,
        clearance_class_index,
        true,
        MAX_SPRING_OVER_RECURSION_DEPTH,
        contact_pins,
    );
    match clock_wise_result {
        SpringOverResult::Changed(clock_wise_polyline) => {
            if clock_wise_polyline.length_approx_total()
                <= counter_clock_wise_polyline.length_approx_total()
            {
                Some(clock_wise_polyline.reverse())
            } else {
                Some(counter_clock_wise_polyline)
            }
        }
        // CW returned its (reversed) input object: it still flows
        // through the comparison — its length equals the input's — and
        // `CW.reverse()` restores the original geometry (the
        // CW-wins-tie rule).
        SpringOverResult::Unchanged => {
            if clock_wise_input.length_approx_total()
                <= counter_clock_wise_polyline.length_approx_total()
            {
                Some(clock_wise_input.reverse())
            } else {
                Some(counter_clock_wise_polyline)
            }
        }
        SpringOverResult::Failed => Some(counter_clock_wise_polyline),
    }
}

// ---------------------------------------------------------------------------
// Shared helpers (the Java board-facade reads the drivers lean on)
// ---------------------------------------------------------------------------

/// Java `Item.getAllContacts(layer)` (`Item.java:569-590`): the
/// connectable items sharing this item's net, overlapping one of its
/// tree shapes ON THE GIVEN LAYER. Java's `TreeSet` orders the result
/// descending by id; the port preserves that order.
pub(crate) fn get_all_contacts_on_layer(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    id: ItemId,
    layer: i32,
) -> Vec<ItemId> {
    let is_connectable = match board.get(id).map(|e| &e.data) {
        Some(ItemData::Pin { .. })
        | Some(ItemData::Via { .. })
        | Some(ItemData::Trace { .. })
        | Some(ItemData::ConductionArea { .. }) => true,
        _ => false, // Java `!(this instanceof Connectable)` -> empty
    };
    if !is_connectable {
        return Vec::new();
    }
    let tree = manager.default_tree();
    let (tree_oid, tree_variant, tree_class) = (
        tree.object_id(),
        tree.variant,
        tree.compensated_clearance_class,
    );
    let precalc = board.tree_shape_precalc(id, tree_oid, tree_variant, tree_class);
    let mut result: BTreeSet<Reverse<ItemId>> = BTreeSet::new();
    for (i, shape) in precalc.iter().enumerate() {
        if board.item_shape_layer(id, i as i32) != Some(layer) {
            continue;
        }
        let tile_shape = shape
            .clone()
            .expect("Java NPE: null tile shape in getAllContacts");
        for other in manager.overlapping_objects(board, 0, &tile_shape, layer, &[]) {
            if other == id {
                continue;
            }
            let other_connectable = matches!(
                board.get(other).map(|e| &e.data),
                Some(ItemData::Pin { .. })
                    | Some(ItemData::Via { .. })
                    | Some(ItemData::Trace { .. })
                    | Some(ItemData::ConductionArea { .. })
            );
            if other_connectable && items_share_net(board, id, other) {
                result.insert(Reverse(other));
            }
        }
    }
    result.into_iter().map(|Reverse(id)| id).collect()
}

/// Java `Item.boundingBox()` (`Item.java:297-310` region) — the box
/// containing the item geometry, dispatched per kind:
///
/// * traces: `items::trace::bounding_box` = the polyline bounds
///   enlarged by the half width (`PolylineTrace.boundingBox`, :118),
/// * pins/vias: the union of the padstack shapes' boxes over the
///   padstack span (skip null layers, `DrillItem.boundingBox` :191),
/// * obstacle/conduction/component areas: the area's border bounds,
/// * the board outline: the union of its shape bounds.
pub(crate) fn item_bounding_box(board: &Board, id: ItemId) -> IntBox {
    let Some(entry) = board.get(id) else {
        return IntBox::EMPTY; // Java would NPE; unreachable through the seam
    };
    match &entry.data {
        ItemData::Trace {
            lines, half_width, ..
        } => trace_bounding_box(lines, *half_width),
        ItemData::Pin { .. } | ItemData::Via { .. } => {
            let count = board.drill_tile_shape_count(id).unwrap_or(0).max(0);
            let mut result = IntBox::EMPTY;
            for i in 0..count {
                if let Some(shape) = board.drill_shape(id, i) {
                    result = result.union(&shape_bounding_box(&shape));
                }
            }
            result
        }
        // All three area kinds store `Area { border, holes }`; Java
        // `PolylineArea.boundingBox` is the BORDER shape's box
        // (`PolylineArea.java:69-71`) — holes never extend it.
        ItemData::ObstacleArea { area, .. }
        | ItemData::ConductionArea { area, .. }
        | ItemData::ComponentOutline { area, .. } => area.border.bounding_box(),
        ItemData::BoardOutline { shapes, .. } => {
            let mut result = IntBox::EMPTY;
            for shape in shapes {
                result = result.union(&shape_bounding_box(shape));
            }
            result
        }
        ItemData::Other => IntBox::EMPTY,
    }
}

/// The bounding box of a [`BoardShape`] (the per-variant dispatch Java
/// gets from the shared `Shape.boundingBox()` protocol).
pub(crate) fn shape_bounding_box(shape: &BoardShape) -> IntBox {
    match shape {
        BoardShape::Tile(tile) => tile.bounding_box(),
        BoardShape::PolygonShape(polygon) => polygon.bounding_box(),
        BoardShape::Circle(circle) => circle.bounding_box(),
    }
}

/// Java `BasicBoard.getOutline()` — the board's outline item (`None`
/// mirrors Java's null for a board without a boundary).
pub(crate) fn board_outline_id(board: &Board) -> Option<ItemId> {
    board
        .iter_descending()
        .find(|entry| matches!(entry.data, ItemData::BoardOutline { .. }))
        .map(|entry| entry.id)
}

/// Java `RoutingBoard.containsTraceTails(items, exceptNetNoArr)`
/// (`RoutingBoard.java:1176-1189`): any TRACE in the list whose nets
/// differ from `except_net_nos` and that carries an unconnected end.
pub(crate) fn contains_trace_tails(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    items: &[ItemId],
    except_net_nos: &[i32],
) -> bool {
    for &id in items {
        let is_foreign_tail_trace = match board.get(id).map(|e| &e.data) {
            Some(ItemData::Trace { .. }) => {
                let nets = item_nets(board, id);
                !nets_equal(&nets, except_net_nos)
            }
            _ => false,
        };
        if is_foreign_tail_trace && item_is_tail(manager, board, id) {
            return true;
        }
    }
    false
}

/// Java `BasicBoard.getTraceTail(location, layer, netNumbers)`
/// (`BasicBoard.java:1307-1330`): the same-net trace ENDING at
/// `location` (unconnected on that end) — the leftover stub to clean up
/// after a shove insert.
pub(crate) fn get_trace_tail(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    location: &Point,
    layer: i32,
    net_numbers: &[i32],
) -> Option<ItemId> {
    // Java `TileShape.getInstance(location)` = the point's
    // surrounding box.
    let point_shape =
        TileShape::RegularTileShape(RegularTileShape::IntBox(location.surrounding_box()));
    let candidates = manager.overlapping_objects(board, 0, &point_shape, layer, &[]);
    for id in candidates {
        let (is_trace, nets) = match board.get(id) {
            Some(entry) => (
                matches!(entry.data, ItemData::Trace { .. }),
                entry.nets.clone(),
            ),
            None => continue,
        };
        if !is_trace {
            continue;
        }
        if !nets_equal(&nets, net_numbers) {
            continue;
        }
        let lines = board
            .trace_polyline(id)
            .expect("live trace polyline")
            .clone();
        if first_corner(&lines).as_ref() == Some(location)
            && start_contacts(manager, board, id).is_empty()
        {
            return Some(id);
        }
        if last_corner(&lines).as_ref() == Some(location)
            && end_contacts(manager, board, id).is_empty()
        {
            return Some(id);
        }
    }
    None
}

/// Java `BasicBoard.removeItems(itemList)` (`BasicBoard.java:637-647`):
/// removes every removable item; the result is false when at least one
/// item was deletion-forbidden or user-fixed (those stay).
pub(crate) fn remove_items(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    items: &[ItemId],
) -> bool {
    let mut result = true;
    for &id in items {
        let forbidden = match board.get(id) {
            Some(entry) => {
                let user_fixed =
                    matches!(entry.fixed, FixedState::UserFixed | FixedState::SystemFixed);
                entry.component_id > 0
                    || user_fixed
                    || (matches!(entry.data, ItemData::ConductionArea { .. })
                        && !conduction_area_on_signal_layer(board, id))
            }
            None => false,
        };
        if forbidden {
            result = false;
        } else {
            remove_item_through_repository(manager, board, id);
        }
    }
    result
}

/// The `ConductionArea && !isSignalLayer(firstLayer())` clause of
/// `Item.isDeletionForbidden` (`Item.java:866-873`).
fn conduction_area_on_signal_layer(board: &Board, id: ItemId) -> bool {
    let Some(layer) = board.area_layer(id) else {
        return false;
    };
    board
        .layers()
        .layers
        .get(layer as usize)
        .is_some_and(|l| l.is_signal)
}

/// Java `BasicBoard.combineTraces(netNumber)` (`BasicBoard.java:684-708`):
/// repeatedly combines the first combinable trace of the net (walking
/// the undo list from its start), restarting after every successful
/// combine, until a full pass combines nothing.
pub fn combine_traces(manager: &mut SearchTreeManager, board: &mut Board, net_number: i32) -> bool {
    let mut result = false;
    loop {
        // Java iterates `itemList` — the UndoableObjects list, whose
        // order is INSERTION order (not the arena's descending id).
        let candidates: Vec<ItemId> = board
            .item_undo
            .iter_visible()
            .filter(|(key, entry)| {
                matches!(entry.data, ItemData::Trace { .. })
                    && entry.on_the_board
                    && (net_number < 0 || entry.nets.contains(&net_number))
                    && key.0.get() != 0
            })
            .map(|(key, _)| ItemId::new(key.0.get()))
            .collect();
        let mut something_changed = false;
        for id in candidates {
            if combine(manager, board, id) {
                something_changed = true;
                result = true;
                break;
            }
        }
        if !something_changed {
            break;
        }
    }
    result
}

/// The compensated half width of an UNINSERTED substitute piece (Java
/// `PolylineTrace.getCompensatedHalfWidth(searchTree)` reads the same
/// inputs off the object): `half_width + clearanceCompensationValue`.
pub(crate) fn piece_compensated_half_width(
    rules: &BoardRules,
    tree_class: i32,
    piece: &SubstituteTracePiece,
) -> i32 {
    piece.half_width
        + clearance_compensation_value(rules, piece.clearance_class, tree_class, piece.layer)
}

/// The nets of an item, empty for a missing id (Java would NPE;
/// unreachable through the seam).
fn item_nets(board: &Board, id: ItemId) -> Vec<i32> {
    board
        .get(id)
        .map(|entry| entry.nets.clone())
        .unwrap_or_default()
}

/// The M3-T10b SHOVER WORLD: a verbatim replay of the board mutations in
/// `rust/harness/oracle/TraceShoverProbe.main()` — the fixture parsed
/// fresh, then the probe's +-250 via padstack, the N002 trace (id 105)
/// and the N002 via (id 106) through the production insert paths, so the
/// ids replay 1:1. Captures:
/// `logs/M3-T10b/captures/trace_shover_rows.jsonl` (+ `_run2.jsonl`,
/// byte-identical double run). Shared with the `drill_item_mover` and
/// `forced_pad_router` test mods.
#[cfg(test)]
pub(crate) mod shover_world {
    use super::{
        Board, FixedState, ItemId, Polyline, SearchTreeManager, ShapeEntrySide, TileShape,
    };
    use crate::components::BoardPadstack;
    use crate::drill_item_mover::insert_via;
    use crate::items::BoardShape;
    use crate::test_util::parse_board_from_path;
    use crate::trace_ops::insert_trace_without_cleaning;
    use epic_geometry::int_box::IntBox;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::point::Point;
    use epic_geometry::regular_tile_shape::RegularTileShape;

    pub(crate) fn p(x: i32, y: i32) -> Point {
        Point::int(IntPoint::new(x, y))
    }

    pub(crate) fn ibox(x1: i32, y1: i32, x2: i32, y2: i32) -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::new(
            IntPoint::new(x1, y1),
            IntPoint::new(x2, y2),
        )))
    }

    pub(crate) struct ShoverWorld {
        #[allow(dead_code)] // captured for the pin docs
        pub cx: i32,
        #[allow(dead_code)] // captured for the pin docs
        pub cy: i32,
        pub own: i32,
        #[allow(dead_code)] // captured for the pin docs
        pub foreign: i32,
        /// The N002 trace, id 105.
        pub trace: ItemId,
        /// The N002 via at (cx, cy+600), id 106.
        pub via: ItemId,
        pub padstack_no: i32,
    }

    /// The probe's world row: cx=500000, cy=300000, ownNet=1, foreignNet=2.
    pub(crate) fn build_shover_world() -> (SearchTreeManager, Board, ShoverWorld) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../rust/harness/fixtures/locator-spike/t9_locator45.dsn");
        let mut board = parse_board_from_path(&path.to_string_lossy());
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);

        let bb = board.bounding_box().expect("fixture bbox");
        let cx = (bb.ll.x + bb.ur.x) / 2;
        let cy = (bb.ll.y + bb.ur.y) / 2;
        assert_eq!((cx, cy), (500000, 300000), "capture bbox center");
        let (own, foreign) = (1, 2);

        // `padstacks.add(new IntBox(-250,-250,250,250), 0, layerCount-1)`
        // — drillAllowed defaults to false.
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
            name: "shover_via".to_string(),
            shapes: vec![rect(); layer_count],
            drillable: false,
            placed_absolute: false,
            hole_only: false,
        });
        let padstack_no = board.library().padstacks.len() as i32;

        let trace = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(cx - 2000, cy), &p(cx + 2000, cy)),
            0,
            100,
            &[foreign],
            0,
            FixedState::Unfixed,
        )
        .expect("probe trace");
        assert_eq!(trace.get(), 105, "capture trace id");
        let via = insert_via(
            &mut manager,
            &mut board,
            padstack_no,
            IntPoint::new(cx, cy + 600),
            &[foreign],
            0,
            FixedState::Unfixed,
            false,
        );
        assert_eq!(via.get(), 106, "capture via id");

        (
            manager,
            board,
            ShoverWorld {
                cx,
                cy,
                own,
                foreign,
                trace,
                via,
                padstack_no,
            },
        )
    }

    /// The probe's mainShape: `IntBox(cx-400, cy-800, cx+400, cy+800)`.
    pub(crate) fn main_shape() -> TileShape {
        ibox(499600, 299200, 500400, 300800)
    }

    /// The probe's zeroShape: `IntBox(cx-400, cy+300, cx+400, cy+900)`.
    pub(crate) fn zero_shape() -> TileShape {
        ibox(499600, 300300, 500400, 300900)
    }

    /// `new ShapeEntrySide(new IntPoint(cx, cy-800), mainShape)` —
    /// bottom side, intersection (500000, 299200).
    pub(crate) fn main_side() -> ShapeEntrySide {
        ShapeEntrySide::from_point(p(500000, 299200), &main_shape())
    }

    /// `new ShapeEntrySide(new IntPoint(cx, cy+300), zeroShape)` —
    /// bottom side, intersection (500000, 300300).
    pub(crate) fn zero_side() -> ShapeEntrySide {
        ShapeEntrySide::from_point(p(500000, 300300), &zero_shape())
    }

    /// The SpringOverBudgetProbe world (jar capture
    /// `logs/M3-T10b/captures/spring_budget_rows_run1.jsonl` +
    /// `_run2.jsonl`, double-run byte-identical): the rig sits ~190000
    /// west of the fixture items, in the band x[308300,311500] x
    /// y[298500,301400]. Two UNFIXED 45-degree traces on DIFFERENT nets
    /// cross the probe shape (X: N002, id 105, bottom -> east; Y: N003,
    /// id 106, top -> west) and two N001 USER_FIXED ±100 vias (107 at
    /// (311150,298980), 108 at (308850,301100)) sit in the detour-arc
    /// corner corridors — every piece needs its spring-over. The probe
    /// shape is the BOUNDING OCTAGON of the rig box (45-degree mode): an
    /// IntBox would set `is_orthogonal_mode`, box-ify the recursion's
    /// segment probes, and false-fail the wrapped corner-cut diagonals
    /// (their bounding boxes overlap the via's box corner; the true
    /// octagon probes are 17 clear — see the probe header's S
    /// paragraph).
    pub(crate) struct SpringBudgetWorld {
        /// the N002 trace, id 105.
        #[allow(dead_code)] // captured for the pin docs
        pub trace_x: ItemId,
        /// the N003 trace, id 106.
        #[allow(dead_code)] // captured for the pin docs
        pub trace_y: ItemId,
        /// the N001 USER_FIXED via at (311150, 298980), id 107.
        pub vx: ItemId,
        /// the N001 USER_FIXED via at (308850, 301100), id 108.
        pub vy: ItemId,
    }

    pub(crate) fn spring_budget_world() -> (SearchTreeManager, Board, SpringBudgetWorld) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../rust/harness/fixtures/locator-spike/t9_locator45.dsn");
        let mut board = parse_board_from_path(&path.to_string_lossy());
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);

        // net numbers are capture literals (N001=1, N002=2, N003=3).
        let rect = || {
            Some(BoardShape::Tile(TileShape::RegularTileShape(
                RegularTileShape::IntBox(IntBox::new(
                    IntPoint::new(-100, -100),
                    IntPoint::new(100, 100),
                )),
            )))
        };
        let layer_count = board.library().padstacks[0].shapes.len();
        board.library_mut().padstacks.push(BoardPadstack {
            name: "spring_via".to_string(),
            shapes: vec![rect(); layer_count],
            drillable: false,
            placed_absolute: false,
            hole_only: false,
        });
        let padstack_no = board.library().padstacks.len() as i32;

        let trace_x = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(310500, 298600), &p(311400, 299500)),
            0,
            50,
            &[2],
            0,
            FixedState::Unfixed,
        )
        .expect("trace X");
        assert_eq!(trace_x.get(), 105, "capture traceX id");
        let trace_y = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(309500, 301300), &p(308400, 300200)),
            0,
            50,
            &[3],
            0,
            FixedState::Unfixed,
        )
        .expect("trace Y");
        assert_eq!(trace_y.get(), 106, "capture traceY id");
        let vx = insert_via(
            &mut manager,
            &mut board,
            padstack_no,
            IntPoint::new(311150, 298980),
            &[1],
            0,
            FixedState::UserFixed,
            false,
        );
        assert_eq!(vx.get(), 107, "capture VX id");
        let vy = insert_via(
            &mut manager,
            &mut board,
            padstack_no,
            IntPoint::new(308850, 301100),
            &[1],
            0,
            FixedState::UserFixed,
            false,
        );
        assert_eq!(vy.get(), 108, "capture VY id");

        (
            manager,
            board,
            SpringBudgetWorld {
                trace_x,
                trace_y,
                vx,
                vy,
            },
        )
    }

    /// The probe shape: the bounding octagon of the rig box
    /// [309000,299000,311000,301000] (the probe's `probeShape()`).
    pub(crate) fn spring_budget_shape() -> TileShape {
        TileShape::RegularTileShape(RegularTileShape::IntOctagon(
            IntBox::new(IntPoint::new(309000, 299000), IntPoint::new(311000, 301000))
                .bounding_octagon(),
        ))
    }

    /// `new ShapeEntrySide(new IntPoint(310000, 299000), shape)` — the
    /// interior of the bottom side (border line 0), west of X's
    /// crossing, so the resort break fires at the list head and X pops
    /// first.
    pub(crate) fn spring_budget_side() -> ShapeEntrySide {
        ShapeEntrySide::from_point(p(310000, 299000), &spring_budget_shape())
    }
}

#[cfg(test)]
mod tests {
    //! Driver pins for `TraceShover.check` / `insert` / `springOver` /
    //! `springOverObstacles`. Jar-side sources: the TraceShoverProbe
    //! replay (`shover_world`) and the ShapeTraceDebtProbe captures
    //! (`logs/M3-T10b/captures/shape_debt_rows.jsonl`).

    use super::*;
    use crate::drill_item_mover::{shove_vias, try_shove_via_points};
    use crate::shape_trace_entries::debt_world::{build_debt_world, p};
    use crate::trace_ops::insert_trace_without_cleaning;
    use epic_geometry::int_box::IntBox;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::point::Point;
    use epic_geometry::regular_tile_shape::RegularTileShape;

    /// One captured trace as (id, corners) with 1e-6 corner tolerance.
    fn assert_trace_corners(board: &Board, id: u32, expected: &[[f64; 2]]) {
        let lines = board.trace_polyline(ItemId::new(id)).expect("trace alive");
        let got: Vec<[f64; 2]> = lines
            .corner_approx_arr()
            .iter()
            .map(|c| [c.x, c.y])
            .collect();
        assert_eq!(got.len(), expected.len(), "id {id} corner count");
        for (g, w) in got.iter().zip(expected) {
            assert!(
                (g[0] - w[0]).abs() < 1e-6 && (g[1] - w[1]).abs() < 1e-6,
                "id {id} corner {g:?} vs {w:?}"
            );
        }
    }

    fn assert_int_center(board: &Board, id: ItemId, x: i32, y: i32) {
        match board.drill_center(id).expect("drill item alive") {
            Point::Int(ip) => assert_eq!((ip.x, ip.y), (x, y), "center of {id:?}"),
            Point::Rational(_) => panic!("integer center expected"),
        }
    }

    /// The full TraceShoverProbe replay (capture rows 7-21): the four
    /// driver calls in probe order, all true; the via shoved by
    /// insert_zero lands at (500000, 301168); insert_main removes trace
    /// 105 in favor of the fast-cutout pair 108/109 and the substitute
    /// 110 wrapping the (already moved) via corridor.
    #[test]
    fn t1_probe_replay_check_and_insert_match_the_jar() {
        let (mut manager, mut board, w) = shover_world::build_shover_world();
        // check_main: own nets [1] against the main shape.
        assert!(
            check(
                &mut manager,
                &mut board,
                &shover_world::main_shape(),
                Some(shover_world::main_side()),
                None,
                0,
                &[w.own],
                0,
                10,
                10,
                2,
                None,
            ),
            "capture check_main=true"
        );
        assert_eq!(board.shove_failing_layer(), -1, "capture failingLayer");
        assert_eq!(
            board.shove_failing_obstacle(),
            None,
            "capture failingObstacle"
        );
        // check_zero.
        assert!(
            check(
                &mut manager,
                &mut board,
                &shover_world::zero_shape(),
                Some(shover_world::zero_side()),
                None,
                0,
                &[w.own],
                0,
                10,
                10,
                2,
                None,
            ),
            "capture check_zero=true"
        );
        // insert_zero: moves the via north out of the zero shape.
        assert!(
            insert(
                &mut manager,
                &mut board,
                &shover_world::zero_shape(),
                Some(shover_world::zero_side()),
                0,
                &[w.own],
                0,
                &[],
                10,
                10,
                2,
            ),
            "capture insert_zero=true"
        );
        assert_int_center(&board, w.via, 500000, 301168);
        // insert_main: cuts 105 into 108/109 and inserts substitute 110.
        assert!(
            insert(
                &mut manager,
                &mut board,
                &shover_world::main_shape(),
                Some(shover_world::main_side()),
                0,
                &[w.own],
                0,
                &[],
                10,
                10,
                2,
            ),
            "capture insert_main=true"
        );
        assert!(!board.is_on_the_board(w.trace), "capture: 105 gone");
        assert_eq!(board.item_count(), 108, "capture final inventory");
        assert_trace_corners(&board, 108, &[[498000.0, 300000.0], [499483.0, 300000.0]]);
        assert_trace_corners(&board, 109, &[[500517.0, 300000.0], [502000.0, 300000.0]]);
        assert_trace_corners(
            &board,
            110,
            &[
                [500517.0, 300000.0],
                [500517.0, 300917.0],
                [499483.0, 300917.0],
                [499483.0, 300000.0],
            ],
        );
        assert_int_center(&board, w.via, 500000, 301168);
    }

    /// The stack-depth gate (`:379`): the debt world's w4b store run
    /// succeeds with maxStackLevel 2, so the driver-level check must fail
    /// and report the store's foundObstacle — the :440 quirk's
    /// last-stored trace, here a genuine failure report. NOTE the id:
    /// the drivers pass `store_items` the RAW
    /// `overlappingItemsWithClearance` order, which is DESCENDING by id
    /// (`BTreeSet<Reverse<ItemId>>`, the M2 corpus-parity order — unlike
    /// the debt-world pins, which sort ASCENDING to mirror the Java
    /// probe's TreeSet). Stored 117, then 116, then 115 → the quirk's
    /// last-stored trace is 115 (w4b1). Kill target: the `> 1` → `> 2`
    /// mutant (the shove would proceed and succeed).
    #[test]
    fn t2_stack_depth_gate_fails_at_depth_two() {
        let (mut manager, mut board, w) = build_debt_world();
        let shape = shover_world::ibox(504300, 311300, 505700, 314700);
        assert!(
            !check(
                &mut manager,
                &mut board,
                &shape,
                Some(ShapeEntrySide::new_precomputed(0, None)),
                None,
                0,
                &[w.own],
                0,
                10,
                10,
                2,
                None,
            ),
            "stack depth 2 exceeds the gate"
        );
        assert_eq!(
            board.shove_failing_obstacle(),
            Some(ItemId::new(115)),
            "the failing obstacle is the :440 quirk's last-stored trace \
             under the descending raw query order (w4b1)"
        );
    }

    /// The gate ORDER faces: the time limit precedes the empty-shape arm
    /// (`:328` vs `:334`); a shape outside the board fails on the outline
    /// (`:340`) and reports the BoardOutline (id 1).
    #[test]
    fn t3_time_limit_empty_and_outline_gate_order() {
        let (mut manager, mut board, w) = shover_world::build_shover_world();
        let empty = TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::EMPTY));
        let expired = TimeLimit::new(-1); // strictly elapsed: always expired
        // expiry wins over emptiness — the order pin.
        assert!(!check(
            &mut manager,
            &mut board,
            &empty,
            None,
            None,
            0,
            &[w.own],
            0,
            10,
            10,
            2,
            Some(&expired),
        ));
        // an empty shape passes without a limit.
        assert!(check(
            &mut manager,
            &mut board,
            &empty,
            None,
            None,
            0,
            &[w.own],
            0,
            10,
            10,
            2,
            None,
        ));
        // expiry fails a real probe too.
        assert!(!check(
            &mut manager,
            &mut board,
            &shover_world::zero_shape(),
            None,
            None,
            0,
            &[w.own],
            0,
            10,
            10,
            2,
            Some(&expired),
        ));
        // outside the board: the outline is reported.
        let outside = shover_world::ibox(-5000, 299000, -1000, 300000);
        assert!(!check(
            &mut manager,
            &mut board,
            &outside,
            None,
            None,
            0,
            &[w.own],
            0,
            10,
            10,
            2,
            None,
        ));
        assert_eq!(
            board.shove_failing_obstacle(),
            Some(ItemId::new(1)),
            "the BoardOutline is the failing obstacle"
        );
    }

    /// The springOver ladder arms against the debt world's SHOVE_FIXED
    /// trace v3 (id 128, N003, y = 316950, hw 100):
    /// (a) a clear polyline returns the input unchanged;
    /// (b) depth 0 fails on the obstacle before any geometry;
    /// (c) depth 5 wraps a circuit that clears v3 by EXACTLY one unit
    /// (offset total 117 = hw 100 + 1 + cl 16 vs the 116 requirement),
    /// so the inner recursion returns Unchanged and the outer maps it to
    /// Changed. NOTE this depth-5 arm pins the Changed mapping, not the
    /// enlarge arithmetic — the wrap has one unit of slack, so the
    /// two-step vs single-step enlarge rounding stays invisible HERE;
    /// t12's exact wrapped corners are what the two-step arithmetic is
    /// load-bearing for.
    #[test]
    fn t4_spring_over_ladder_arms() {
        let (mut manager, mut board, w) = build_debt_world();
        // (a) nothing in the way.
        let far = Polyline::from_two_corners(&p(504000, 316950), &p(508000, 316950));
        assert_eq!(
            spring_over(
                &mut manager,
                &mut board,
                &far,
                100,
                0,
                &[2],
                0,
                false,
                5,
                None,
            ),
            SpringOverResult::Unchanged,
            "no obstacle: the input comes back"
        );
        // (b) the depth-0 arm fails on v3.
        let cross = Polyline::from_two_corners(&p(492000, 316000), &p(492000, 318000));
        assert_eq!(
            spring_over(
                &mut manager,
                &mut board,
                &cross,
                100,
                0,
                &[2],
                0,
                false,
                0,
                None,
            ),
            SpringOverResult::Failed,
            "depth 0 fails"
        );
        assert_eq!(
            board.shove_failing_obstacle(),
            Some(w.v3),
            "the failing obstacle is the SHOVE_FIXED trace"
        );
        // (c) the wrapped circuit.
        match spring_over(
            &mut manager,
            &mut board,
            &cross,
            100,
            0,
            &[2],
            0,
            false,
            5,
            None,
        ) {
            SpringOverResult::Changed(wrapped) => {
                let corners = wrapped.corner_approx_arr();
                assert!(corners.len() > 2, "a wrap has more corners: {corners:?}");
                let max_x = corners.iter().fold(f64::MIN, |m, c| m.max(c.x));
                // springOver always walks the border in increasing side
                // order (CCW): from the bottom crossing to the top
                // crossing that is the RIGHT half around v3.
                assert!(max_x > 492000.0, "CCW wraps right, max_x {max_x}");
                assert!(max_x < 495217.0 + 1.0, "the wrap hugs the offset octagon");
            }
            other => panic!("expected a wrapped circuit, got {other:?}"),
        }
    }

    /// The springOverObstacles tie rule (`:1168-1171`, verbatim `<=`):
    /// a crossing through v3's center is symmetric, so the CCW attempt
    /// (right wrap) and the CW attempt on the reversed input (left wrap)
    /// produce DIFFERENT circuits of EXACTLY equal length — the tie picks
    /// the CW circuit (reversed back: still the left half). Kill target:
    /// `<=` → `<` flips the returned geometry to the right wrap.
    #[test]
    fn t5_spring_over_obstacles_cw_wins_the_length_tie() {
        let (mut manager, mut board, _w) = build_debt_world();
        let input = Polyline::from_two_corners(&p(492000, 316000), &p(492000, 318000));
        // both attempts succeed; assert the tie is real (equal lengths).
        let SpringOverResult::Changed(ccw) = spring_over(
            &mut manager,
            &mut board,
            &input,
            100,
            0,
            &[2],
            0,
            false,
            20,
            None,
        ) else {
            panic!("the CCW attempt wraps");
        };
        let reversed = input.reverse();
        let SpringOverResult::Changed(cw) = spring_over(
            &mut manager,
            &mut board,
            &reversed,
            100,
            0,
            &[2],
            0,
            false,
            20,
            None,
        ) else {
            panic!("the CW attempt wraps");
        };
        assert_eq!(
            cw.length_approx_total(),
            ccw.length_approx_total(),
            "the mirrored circuits tie exactly"
        );
        // the driver: the tie rule hands back the CW circuit.
        let result = spring_over_obstacles(&mut manager, &mut board, &input, 100, 0, &[2], 0, None)
            .expect("one side succeeds");
        // MIN_X is the side discriminator, NOT max_x: a springOver wrap
        // of a single vertical line always re-uses the vertical line as
        // the substitute's last line, so max_x == 492000 for BOTH the
        // left and the right wrap. The left wrap bulges to
        // min_x ~ 488783 (the offset octagon), the right wrap stays at
        // exactly 492000.
        let min_x = result
            .corner_approx_arr()
            .iter()
            .fold(f64::MAX, |m, c| m.min(c.x));
        assert!(
            min_x < 492000.0,
            "cw-wins-tie: the left wrap comes back, min_x {min_x}"
        );
    }

    /// The shoveVias true-on-failure quirks: (a) a store failure (the
    /// SHOVE_FIXED probe trace) still answers TRUE and changes nothing;
    /// (b) a spent via budget answers TRUE mid-loop with the via in
    /// place (`:302-305`).
    #[test]
    fn t6_shove_vias_true_on_store_fail_and_spent_budget() {
        let (mut manager, mut board, w) = shover_world::build_shover_world();
        let fixed = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(500680, 300000), &p(501900, 300000)),
            0,
            100,
            &[3],
            0,
            FixedState::ShoveFixed,
        )
        .expect("shove-fixed probe trace");
        // (a) the store fails on the fixed trace; the answer is still true.
        let shape = shover_world::ibox(500700, 299700, 501700, 300300);
        assert!(shove_vias(
            &mut manager,
            &mut board,
            &shape,
            None,
            0,
            &[w.own],
            0,
            &[],
            10,
            10,
            true,
        ));
        assert!(
            board.is_on_the_board(fixed),
            "the store failed: nothing moved"
        );
        // (b) maxViaRecursionDepth 0 with the foreign via in the shape.
        assert!(shove_vias(
            &mut manager,
            &mut board,
            &shover_world::zero_shape(),
            None,
            0,
            &[w.own],
            0,
            &[],
            10,
            0,
            true,
        ));
        assert_int_center(&board, w.via, 500000, 300600);
    }

    /// The `+2` shove-distance tolerance in tryShoveViaPoints (`:430`,
    /// verbatim): the IntBox obstacle runs the else branch
    /// (0.5 * cl + 2 = 10), and the north candidate lands at
    /// 300910 + 258 = 301168 — the same value the insert_zero capture
    /// produced through the production shove_vias path. Kill target:
    /// `+= 2.0` → `+= 0.0` moves the candidate to 301166.
    #[test]
    fn t7_try_shove_via_points_plus_two_constant() {
        let (mut manager, mut board, w) = shover_world::build_shover_world();
        let shape = shover_world::ibox(499600, 300200, 500400, 300900);
        let single = try_shove_via_points(&mut manager, &mut board, &shape, 0, w.via, 0, false);
        assert_eq!(
            single,
            vec![IntPoint::new(500000, 301168)],
            "the +2 tolerance puts the candidate at 301168"
        );
        let extended = try_shove_via_points(&mut manager, &mut board, &shape, 0, w.via, 0, true);
        assert_eq!(extended.len(), 4, "the extended check tries 4 points");
        assert_eq!(extended[0], IntPoint::new(500000, 301168), "nearest first");
    }

    /// The dir anti-bounce gate of the piece loop (`:497-499`): with
    /// `dir=None` every piece segment is probed; a non-matching
    /// direction probes NOTHING. World: the debt board plus an UNFIXED
    /// foreign trace T' crossing the probe shape — its substitute piece
    /// P has the segment directions UP (x=500517), LEFT (y=301017),
    /// DOWN (x=499483) — and a SHOVE_FIXED blocker X (net N003)
    /// north-east of the shape, OUTSIDE the top-level clearance reach
    /// (S expanded ends at (500416, 300916); X's expanded bbox starts
    /// at (500484, 301084)) but INSIDE the UP-segment probe box (reach
    /// 500401..500633 x, 300484..301133 y). The probe's store fails on
    /// the fixed trace. The spring-over budget is 0 to isolate the dir
    /// gate: springOver's scan shares the probe reach, so with budget
    /// left it would wrap around X and drag the whole octagon-wrap
    /// recursion into the verdict. Kill target: `None => true` ->
    /// `None => false` (the dir=None face would trivially pass).
    #[test]
    fn t8_check_dir_anti_bounce_gates_the_piece_probes() {
        let (mut manager, mut board, w) = build_debt_world();
        let _t_prime = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(498000, 300600), &p(503000, 300600)),
            0,
            100,
            &[w.foreign],
            0,
            FixedState::Unfixed,
        )
        .expect("crossing trace");
        let x = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(500550, 301150), &p(500650, 301150)),
            0,
            50,
            &[w.foreign2],
            0,
            FixedState::ShoveFixed,
        )
        .expect("shove-fixed blocker");
        let shape = shover_world::ibox(499600, 300300, 500400, 300900);
        // dir=None: every segment is probed; the UP probe's store
        // fails on X.
        assert!(!check(
            &mut manager,
            &mut board,
            &shape,
            None,
            None,
            0,
            &[w.own],
            0,
            10,
            10,
            0,
            None,
        ));
        assert_eq!(board.shove_failing_obstacle(), Some(x));
        // EAST matches no piece segment: nothing is probed, the shove
        // succeeds.
        assert!(check(
            &mut manager,
            &mut board,
            &shape,
            None,
            Some(&Direction::RIGHT),
            0,
            &[w.own],
            0,
            10,
            10,
            0,
            None,
        ));
        // WEST re-arms the failing top-segment probe (its box reaches
        // X's expanded bbox in x [500484, 500633], y [301084, 301133]).
        assert!(!check(
            &mut manager,
            &mut board,
            &shape,
            None,
            Some(&Direction::LEFT),
            0,
            &[w.own],
            0,
            10,
            10,
            0,
            None,
        ));
        assert_eq!(board.shove_failing_obstacle(), Some(x));
    }

    /// The springOver containment direction (spec-review MAJOR-1, jar
    /// capture `logs/M3-T10b/captures/spring_nest_rows_run1.jsonl` +
    /// `_run2.jsonl`, double-run byte-identical): the nest pair is
    /// fixture PIN 101
    /// (net 33, bbox [180000,280000,220000,320000]) plus a USER_FIXED
    /// +/-5000 via at its center — Java's motivating "fixed vias
    /// inside of pins" (TraceShover.java:673-674). NOTE the obstacle
    /// classes: the pin qualifies through the contactPins branch (the
    /// empty contact set makes every pin an obstacle); the via only
    /// through `!isRoutable()` (Via.isRoutable = !isUserFixed &&
    /// netCount > 0, Via.java:147-149) — a SHOVE_FIXED via is still
    /// routable and can never be the second obstacle. Java arm 1 fires
    /// when the CURRENT box CONTAINS the FOUND box (replace — the
    /// OUTERMOST nesting box wins from either encounter order;
    /// `IntBox.contains(other)` == `other.isContainedIn(this)`,
    /// IntBox.java:377-379). KILLER = the recursion BUDGET, not the
    /// final geometry: at depth 20 the swapped form still converges on
    /// the same pin circuit (it wraps the inner via first and the
    /// wrap recursion then wraps the pin — the wrong first pick
    /// HEALS), so the depth-1 `springOver` call (a real Java
    /// configuration: the instance check hands each call a
    /// decremented per-invocation budget) is the discriminator. Fixed:
    /// the pin wrap succeeds at depth 1 and the inner scan finds
    /// nothing. Swapped (F1): the budget burns on the via wrap and the
    /// pin only surfaces in the depth-0 recursion -> Failed.
    #[test]
    fn t12_spring_over_nest_keeps_the_outer_box() {
        let (mut manager, mut board, w) = shover_world::build_shover_world();
        // drift guard: the capture's pin, at the captured bbox
        let pin_bb = item_bounding_box(&board, ItemId::new(101));
        assert_eq!(
            (pin_bb.ll.x, pin_bb.ll.y, pin_bb.ur.x, pin_bb.ur.y),
            (180000, 280000, 220000, 320000),
            "capture nestPinBbox"
        );
        // the +/-5000 through-all via padstack, USER_FIXED at the pin
        // center (the world's own padstack is +/-250)
        let rect = || {
            Some(crate::items::BoardShape::Tile(TileShape::RegularTileShape(
                RegularTileShape::IntBox(IntBox::new(
                    IntPoint::new(-5000, -5000),
                    IntPoint::new(5000, 5000),
                )),
            )))
        };
        let layer_count = board.library().padstacks[0].shapes.len();
        board
            .library_mut()
            .padstacks
            .push(crate::components::BoardPadstack {
                name: "nest_via".to_string(),
                shapes: vec![rect(); layer_count],
                drillable: false,
                placed_absolute: false,
                hole_only: false,
            });
        let nest_padstack = board.library().padstacks.len() as i32;
        let _nest_via = crate::drill_item_mover::insert_via(
            &mut manager,
            &mut board,
            nest_padstack,
            IntPoint::new(200000, 300000),
            &[2],
            0,
            FixedState::UserFixed,
            false,
        );
        let spring = Polyline::from_two_corners(&p(170000, 300000), &p(230000, 300000));
        // The depth-1 attempt walks the BOTTOM half (the raw CCW wrap
        // — captured `spring_nest_rows.jsonl` row `field:"depth1"`);
        // the captured springOverObstacles answer (part b) is the TOP
        // wrap via the CW-wins-tie rule.
        let depth1_expected: Vec<[f64; 2]> = vec![
            [170000.0, 300000.0],
            [179933.0, 300000.0],
            [179933.0, 279973.0],
            [179973.0, 279933.0],
            [220027.0, 279933.0],
            [220067.0, 279973.0],
            [220067.0, 300000.0],
            [230000.0, 300000.0],
        ];
        let capture_expected: Vec<[f64; 2]> = vec![
            [170000.0, 300000.0],
            [179933.0, 300000.0],
            [179933.0, 320027.0],
            [179973.0, 320067.0],
            [220027.0, 320067.0],
            [220067.0, 320027.0],
            [220067.0, 300000.0],
            [230000.0, 300000.0],
        ];
        let assert_pin_wrap = |polyline: &Polyline, ctx: &str, expected: &[[f64; 2]]| {
            let got: Vec<[f64; 2]> = polyline
                .corner_approx_arr()
                .iter()
                .map(|c| [c.x, c.y])
                .collect();
            assert_eq!(got.len(), expected.len(), "{ctx}: capture corner count");
            for (g, e) in got.iter().zip(expected.iter()) {
                assert!(
                    (g[0] - e[0]).abs() < 1e-6 && (g[1] - e[1]).abs() < 1e-6,
                    "{ctx}: capture corner {g:?} vs {e:?}"
                );
            }
        };
        // (a) the killer: springOver at recursion depth 1.
        let depth_one = spring_over(
            &mut manager,
            &mut board,
            &spring,
            50,
            0,
            &[w.own],
            0,
            true, // overConnectedPins, as springOverObstacles passes it
            1,
            Some(&BTreeSet::new()),
        );
        let SpringOverResult::Changed(wrapped) = depth_one else {
            panic!("depth-1 springOver wraps the pin directly, got {depth_one:?}")
        };
        assert_pin_wrap(&wrapped, "depth-1", &depth1_expected);
        // (b) the public capture: springOverObstacles (depth 20) wraps
        // the same circuit end to end and reports no failure.
        let result = spring_over_obstacles(
            &mut manager,
            &mut board,
            &spring,
            50,
            0,
            &[w.own],
            0,
            Some(&BTreeSet::new()),
        )
        .expect("the capture wraps: the outer (pin) box wins");
        assert_pin_wrap(&result, "capture", &capture_expected);
        assert_eq!(
            board.shove_failing_obstacle(),
            None,
            "capture shoving-state: no failure"
        );
    }

    /// MAJOR-1 pin (jar capture `logs/M3-T10b/captures/
    /// spring_budget_rows_run1.jsonl` + `_run2.jsonl`, double-run
    /// byte-identical): the spring-over budget of ONE insert invocation
    /// is SHARED across the substitute-piece loop — Java decrements the
    /// mutable method-parameter local at `:539` (`:384` in check) for
    /// the REST of the invocation, so the SECOND piece sees the
    /// decremented value at its own gate (`:521`) and in its recursive
    /// calls (`:556`). World (spring_budget_world): the OCTAGON probe
    /// shape overlaps two traces on DIFFERENT nets (a same-net run
    /// collapses to ONE piece in ShapeTraceEntries.resort) and each
    /// trace carries a USER_FIXED via in its detour-arc corner
    /// corridor, so BOTH substitute pieces need their spring-over
    /// (pieceCount 2, stackDepth 1 in the capture's pieces-store row)
    /// and the segment recursion only passes on the wrapped circuits.
    /// X pops first (fromSide on the bottom side west of X-in).
    ///   budget 0: both gates skip; X's straight-arc probe hits VX.
    ///   budget 1: X wraps VX (1 -> 0); Y's gate then SKIPS — Java's
    ///             per-invocation scope — and Y's straight-arc probe
    ///             hits VY. The per-piece RE-ARM mutant hands Y a fresh
    ///             budget 1, wraps VY too, and answers true.
    ///   budget 2: both pieces wrap; end state = cut stubs 109-112 +
    ///             wrapped pieces 113/114 (captured corners), vias
    ///             untouched.
    #[test]
    fn t13_spring_over_budget_is_shared_across_the_piece_loop() {
        // budget 0: nothing wraps, X fails on VX.
        let (mut manager, mut board, w) = shover_world::spring_budget_world();
        assert!(!insert(
            &mut manager,
            &mut board,
            &shover_world::spring_budget_shape(),
            Some(shover_world::spring_budget_side()),
            0,
            &[1],
            0,
            &[],
            10,
            10,
            0,
        ));
        assert_eq!(
            board.shove_failing_obstacle(),
            Some(w.vx),
            "b0: X's straight arc fails on VX"
        );
        // budget 1: THE pin — the second piece must see the spent budget.
        let (mut manager, mut board, w) = shover_world::spring_budget_world();
        assert!(
            !insert(
                &mut manager,
                &mut board,
                &shover_world::spring_budget_shape(),
                Some(shover_world::spring_budget_side()),
                0,
                &[1],
                0,
                &[],
                10,
                10,
                1,
            ),
            "b1: Y's gate must skip after X spent the shared budget"
        );
        assert_eq!(
            board.shove_failing_obstacle(),
            Some(w.vy),
            "b1: Java fails the second piece on its own via"
        );
        // budget 2: both pieces wrap end to end.
        let (mut manager, mut board, w) = shover_world::spring_budget_world();
        assert!(insert(
            &mut manager,
            &mut board,
            &shover_world::spring_budget_shape(),
            Some(shover_world::spring_budget_side()),
            0,
            &[1],
            0,
            &[],
            10,
            10,
            2,
        ));
        assert_eq!(board.shove_failing_obstacle(), None, "b2: no failure");
        assert!(!board.is_on_the_board(w.trace_x), "capture: 105 cut away");
        assert!(!board.is_on_the_board(w.trace_y), "capture: 106 cut away");
        // the cut stubs.
        assert_trace_corners(&board, 109, &[[309500.0, 301300.0], [309267.0, 301067.0]]);
        assert_trace_corners(&board, 110, &[[308933.0, 300733.0], [308400.0, 300200.0]]);
        assert_trace_corners(&board, 111, &[[310500.0, 298600.0], [310833.0, 298933.0]]);
        assert_trace_corners(&board, 112, &[[311067.0, 299167.0], [311400.0, 299500.0]]);
        // the wrapped pieces (X around VX, Y around VY — the capture's
        // b2 inventory rows).
        assert_trace_corners(
            &board,
            113,
            &[
                [310833.0, 298933.0],
                [310983.0, 298933.0],
                [310983.0, 298853.0],
                [311023.0, 298813.0],
                [311277.0, 298813.0],
                [311317.0, 298853.0],
                [311317.0, 299107.0],
                [311277.0, 299147.0],
                [311067.0, 299147.0],
                [311067.0, 299167.0],
            ],
        );
        assert_trace_corners(
            &board,
            114,
            &[
                [309267.0, 301067.0],
                [309017.0, 301067.0],
                [309017.0, 301227.0],
                [308977.0, 301267.0],
                [308723.0, 301267.0],
                [308683.0, 301227.0],
                [308683.0, 300973.0],
                [308723.0, 300933.0],
                [308933.0, 300933.0],
                [308933.0, 300733.0],
            ],
        );
        assert_int_center(&board, w.vx, 311150, 298980);
        assert_int_center(&board, w.vy, 308850, 301100);
    }
}
