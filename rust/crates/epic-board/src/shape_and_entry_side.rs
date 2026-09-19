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
use epic_index::{SearchTree, SearchTreeVariant};

use crate::board::Board;
use crate::id::ItemId;
use crate::rules_surf::BoardRules;
use crate::shape_entry_side::ShapeEntrySide;
use crate::shape_trace_entries::SubstituteTracePiece;
use crate::tree_manager::SearchTreeManager;
use crate::tree_shapes::{clearance_compensation_value, trace_compensated_half_width};

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
///
/// Java's ctor reads everything through the trace ITEM, so it works
/// for board traces and for UNINSERTED substitute pieces alike (the
/// item computes its tree shapes lazily, `Item.java:228-236`). The
/// port has two faces over one value core: this id-face (board
/// traces, T10a) and [`piece_shape_and_entry_side`] (substitute
/// pieces, T10b).
pub fn shape_and_entry_side(
    manager: &SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
    index: i32,
    orthogonal: bool,
    in_shove_check: bool,
) -> ShapeAndEntrySide {
    let tree = manager.default_tree();
    let lines = board
        .trace_polyline(trace_id)
        .expect("trace polyline")
        .clone();
    let compensated_half_width = trace_compensated_half_width(board, tree, trace_id);
    let variant = tree.variant;
    shape_and_entry_side_core(
        &lines,
        compensated_half_width,
        variant,
        index,
        orthogonal,
        in_shove_check,
    )
}

/// The piece-face of the Java ctor: same body, reading the UNINSERTED
/// substitute piece's own fields (`PolylineTrace` object reads in
/// Java). The compensated half width is computed from the piece's
/// class and layer exactly like [`trace_compensated_half_width`] does
/// for a stored trace.
pub fn piece_shape_and_entry_side(
    rules: &BoardRules,
    tree: &SearchTree,
    piece: &SubstituteTracePiece,
    index: i32,
    orthogonal: bool,
    in_shove_check: bool,
) -> ShapeAndEntrySide {
    let compensated_half_width = piece.half_width
        + clearance_compensation_value(
            rules,
            piece.clearance_class,
            tree.compensated_clearance_class,
            piece.layer,
        );
    shape_and_entry_side_core(
        &piece.lines,
        compensated_half_width,
        tree.variant,
        index,
        orthogonal,
        in_shove_check,
    )
}

/// The ctor body shared verbatim by both faces (value plumbing only —
/// the id-face reads (lines, compensated half width, tree variant)
/// off the board, the piece-face off the piece fields).
pub(crate) fn shape_and_entry_side_core(
    lines: &Polyline,
    compensated_half_width: i32,
    variant: SearchTreeVariant,
    index: i32,
    orthogonal: bool,
    in_shove_check: bool,
) -> ShapeAndEntrySide {
    let mut current_shape = tree_shape_of_lines(lines, variant, compensated_half_width, index);
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
        let end_cutline = calc_cutline_at_end(lines, compensated_half_width, index);
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
        let start_cutline = calc_cutline_at_start(lines, compensated_half_width, index);
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
        current_from_side = Some(ShapeEntrySide::from_entry_no(lines, index, &current_shape));
    }
    ShapeAndEntrySide {
        shape: current_shape,
        from_side: current_from_side,
    }
}

/// The per-segment tree shape of a polyline under the tree variant
/// (the `calculateTreeShapes` element — `tree_shapes.rs`
/// `trace_tree_shapes`): offset box for the ninety-degree tree,
/// offset shape otherwise.
fn tree_shape_of_lines(
    lines: &Polyline,
    variant: SearchTreeVariant,
    offset_width: i32,
    index: i32,
) -> TileShape {
    match variant {
        SearchTreeVariant::NinetyDegree => lines
            .offset_box(offset_width, index)
            .map(|box_shape| TileShape::RegularTileShape(RegularTileShape::IntBox(box_shape)))
            .expect("tree shape exists for a live segment (Java NPE site)"),
        // The base tree's offsetShape == the 45-degree subclass's (no
        // override).
        SearchTreeVariant::Generic | SearchTreeVariant::FortyfiveDegree => lines
            .offset_shape(offset_width, index)
            .expect("tree shape exists for a live segment (Java NPE site)"),
    }
}

/// Java `calcCutlineAtEnd` (`:80-100`): the cut fires when `index`
/// is the LAST trace segment or the segment end sits closer than the
/// compensated half width to the trace's last corner; the cut line is
/// the trace's final line, flipped so its LEFT side faces away from
/// the trace interior.
fn calc_cutline_at_end(lines: &Polyline, compensated_half_width: i32, index: i32) -> Option<Line> {
    let line_count = lines.lines.len() as i32;
    if index == line_count - 3
        || lines
            .corner_approx(line_count - 2)
            .distance(&lines.corner_approx(index + 1))
            < f64::from(compensated_half_width)
    {
        let current_line = &lines.lines[(line_count - 1) as usize];
        let is = lines.corner_approx(line_count - 3);
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
    lines: &Polyline,
    compensated_half_width: i32,
    index: i32,
) -> Option<Line> {
    if index == 0
        || lines.corner_approx(0).distance(&lines.corner_approx(index))
            < f64::from(compensated_half_width)
    {
        let current_line = &lines.lines[0];
        let is = lines.corner_approx(1);
        let cut_line = if current_line.side_of_float_zero(&is) == Side::Positive {
            current_line.opposite()
        } else {
            current_line.clone()
        };
        return Some(cut_line);
    }
    None
}
