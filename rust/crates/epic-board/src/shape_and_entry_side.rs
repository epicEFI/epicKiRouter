//! The shove working shape: a trace's tree shape for one segment,
//! with the dog ears at both ends cut off and the entry side derived
//! from the active cutline (Java
//! `board/model/structure/ShapeAndEntrySide.java`, 120 lines, ported
//! in full).

use epic_geometry::line::Line;
use epic_geometry::polyline::Polyline;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::side::Side;
use epic_geometry::tile_shape::TileShape;
use epic_index::SearchTree;

use crate::board::Board;
use crate::id::ItemId;
use crate::shape_entry_side::ShapeEntrySide;
use crate::tree_manager::SearchTreeManager;
use crate::tree_shapes::trace_compensated_half_width;

/// Java `ShapeAndEntrySide` — used in the shove algorithm to
/// calculate the from-side for pushing and to cut off dog ears of the
/// trace shape.
#[derive(Debug, Clone)]
pub struct ShapeAndEntrySide {
    /// The (possibly cut-down) working shape (Java `shape`).
    pub shape: TileShape,
    /// The entry side; `None` where Java leaves the field null — in
    /// `inShoveCheck` mode the fallback calculation is suppressed
    /// (Java ctor `:71-75`).
    pub from_side: Option<ShapeEntrySide>,
}

/// Java ctor `:25-78`. In the check-shove functions `in_shove_check`
/// is expected to be true; in the actual shove functions false.
/// `orthogonal` short-circuits to the bounding box (no cutlines, no
/// entry side from a cutline).
pub fn shape_and_entry_side(
    manager: &SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
    index: i32,
    orthogonal: bool,
    in_shove_check: bool,
) -> ShapeAndEntrySide {
    let tree = manager.default_tree();
    let mut current_shape = trace_tree_shape(board, tree, trace_id, index);
    let mut current_from_side: Option<ShapeEntrySide> = None;
    let mut cut_off_at_start = false;
    let mut cut_off_at_end = false;
    if orthogonal {
        // Java assigns the IntBox (a TileShape subclass) over the
        // shape variable.
        current_shape =
            TileShape::RegularTileShape(RegularTileShape::IntBox(current_shape.bounding_box()));
    } else {
        // prevent dog ears at the start and the end of the substitute
        // trace
        current_shape = TileShape::Simplex(Box::new(current_shape.to_simplex()));
        let end_cutline = calc_cutline_at_end(board, tree, trace_id, index);
        if let Some(cutline) = end_cutline.clone() {
            let cut_plane = TileShape::from_line(&cutline);
            let tmp_shape = current_shape.intersection(&cut_plane);
            // Java tests `tmpShape != currentShape` — an IDENTITY
            // comparison that is vacuously true here: `Simplex
            // .intersection` always allocates a fresh simplex (or the
            // EMPTY singleton), and `currentShape` was freshly built
            // by `toSimplex()` above. The live gate is therefore the
            // emptiness test alone.
            if !tmp_shape.is_empty() {
                current_shape = TileShape::Simplex(Box::new(tmp_shape.to_simplex()));
                cut_off_at_end = true;
            }
        }
        let start_cutline = calc_cutline_at_start(board, tree, trace_id, index);
        if let Some(cutline) = start_cutline.clone() {
            let cut_plane = TileShape::from_line(&cutline);
            let tmp_shape = current_shape.intersection(&cut_plane);
            // Vacuous-identity note as above.
            if !tmp_shape.is_empty() {
                current_shape = TileShape::Simplex(Box::new(tmp_shape.to_simplex()));
                cut_off_at_start = true;
            }
        }
        let mut from_side_index = -1;
        let mut current_cut_line: Option<Line> = None;
        if cut_off_at_start {
            let cutline = start_cutline.expect("cut_off_at_start implies a start cutline");
            from_side_index = current_shape.border_line_index(&cutline);
            current_cut_line = Some(cutline);
        }
        if from_side_index < 0 && cut_off_at_end {
            let cutline = end_cutline.expect("cut_off_at_end implies an end cutline");
            from_side_index = current_shape.border_line_index(&cutline);
            current_cut_line = Some(cutline);
        }
        if from_side_index >= 0 {
            let cutline = current_cut_line.expect("from_side_index >= 0 implies a cutline");
            let border_intersection =
                cutline.intersection_approx(&current_shape.border_line(from_side_index));
            current_from_side = Some(ShapeEntrySide::new_precomputed(
                from_side_index,
                Some(border_intersection),
            ));
        }
    }
    if current_from_side.is_none() && !in_shove_check {
        // In inShoveCheck, using this calculation may produce an
        // undesired stackLevel > 1 in ShapeTraceEntries.
        let polyline = board.trace_polyline(trace_id).expect("trace polyline");
        current_from_side = Some(ShapeEntrySide::from_entry_no(
            polyline,
            index,
            &current_shape,
        ));
    }
    ShapeAndEntrySide {
        shape: current_shape,
        from_side: current_from_side,
    }
}

/// Java `trace.getTreeShape(searchTree, index)` — the item's
/// precalculated shape for the DEFAULT tree at `index` (through the
/// board's per-item per-tree shape cache). A `None` slot is where
/// Java would NPE; an out-of-range index is Java's AIOOBE (both
/// unreachable for a live trace segment).
fn trace_tree_shape(
    board: &mut Board,
    tree: &SearchTree,
    trace_id: ItemId,
    index: i32,
) -> TileShape {
    let shapes = board.tree_shape_precalc(
        trace_id,
        tree.object_id(),
        tree.variant,
        tree.compensated_clearance_class,
    );
    shapes[index as usize]
        .clone()
        .expect("tree shape exists for a live trace segment (Java NPE site)")
}

/// Java `calcCutlineAtEnd` (`:80-100`): the cut fires when `index`
/// is the LAST trace segment or the segment end sits closer than the
/// compensated half width to the trace's last corner; the cut line is
/// the trace's final line, flipped so its LEFT side faces away from
/// the trace interior.
fn calc_cutline_at_end(
    board: &mut Board,
    tree: &SearchTree,
    trace_id: ItemId,
    index: i32,
) -> Option<Line> {
    let trace_lines: Polyline = board
        .trace_polyline(trace_id)
        .expect("trace polyline")
        .clone();
    let line_count = trace_lines.lines.len() as i32;
    let compensated_half_width = trace_compensated_half_width(board, tree, trace_id);
    if index == line_count - 3
        || trace_lines
            .corner_approx(line_count - 2)
            .distance(&trace_lines.corner_approx(index + 1))
            < f64::from(compensated_half_width)
    {
        let current_line = &trace_lines.lines[(line_count - 1) as usize];
        let is = trace_lines.corner_approx(line_count - 3);
        let cut_line = if current_line.side_of_float_zero(&is) == Side::Positive {
            current_line.opposite()
        } else {
            current_line.clone()
        };
        return Some(cut_line);
    }
    None
}

/// Java `calcCutlineAtStart` (`:102-119`): mirror of the end cut for
/// the trace's first line and first corner.
fn calc_cutline_at_start(
    board: &mut Board,
    tree: &SearchTree,
    trace_id: ItemId,
    index: i32,
) -> Option<Line> {
    let trace_lines: Polyline = board
        .trace_polyline(trace_id)
        .expect("trace polyline")
        .clone();
    let compensated_half_width = trace_compensated_half_width(board, tree, trace_id);
    if index == 0
        || trace_lines
            .corner_approx(0)
            .distance(&trace_lines.corner_approx(index))
            < f64::from(compensated_half_width)
    {
        let current_line = &trace_lines.lines[0];
        let is = trace_lines.corner_approx(1);
        let cut_line = if current_line.side_of_float_zero(&is) == Side::Positive {
            current_line.opposite()
        } else {
            current_line.clone()
        };
        return Some(cut_line);
    }
    None
}
