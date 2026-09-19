//! Checking and inserting forced vias (Java
//! `board/actions/ForcedViaInserter.java`, 462 lines, ported in full).
//! The fourth driver of the shove cycle: every pad check funnels into
//! [`crate::forced_pad_router`], so the whole recursion budget
//! machinery of the cycle applies here too.
//!
//! ## The verbatim quirks
//!
//! * [`check_layer`] NEVER reports a failing layer or obstacle of its
//!   own — only the delegated `checkForcedPad` runs do (contrast
//!   [`check`]/[`insert`], which set `shoveFailingLayer` on every
//!   early-out, `:180`/`:205`/`:234`/`:307`/...).
//! * [`hole_check_shape`] inflates the drill circle by
//!   `holeClearance + 10` — the trailing `+ 10` is a verbatim Java
//!   constant (`:374`).
//! * [`insert`] indexes `tracePenHalfwidthArr[i]` UNGUARDED
//!   (`:277`): where [`check`] bounds-tests
//!   `i < tracePenHalfwidthArr.length` (`:211`), the insert path
//!   throws `ArrayIndexOutOfBoundsException` on a short array. The
//!   port preserves both faces: the slice index panics with the Java
//!   exception named in the message.
//! * [`check_layer`] answers `DRILLABLE` (not NOT_DRILLABLE) when the
//!   via radius vanishes (`:43-45`).

use epic_geometry::circle::Circle;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::limits::SQRT2;
use epic_geometry::point::Point;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::side::Side;
use epic_geometry::simplex::Simplex;
use epic_geometry::tile_shape::TileShape;

use crate::board::Board;
use crate::components::BoardPadstack;
use crate::drill_item_mover::insert_via;
use crate::forced_pad_router::{CheckDrillResult, calc_from_side, check_forced_pad, forced_pad};
use crate::items::{BoardShape, FixedState};
use crate::rules_surf::{AngleRestriction, ViaInfo};
use crate::shape_entry_side::ShapeEntrySide;
use crate::tree_manager::SearchTreeManager;

/// Java static `checkLayer` (`:30-125`): is a via of radius
/// `via_radius` possible at `location` on `layer`, shoving obstacle
/// traces aside? `room_shape` (the maze drill page) feeds the from-side
/// probe; a start trace of `trace_half_width` /
/// `trace_clearance_class` is checked afterwards when positive. The
/// board is NOT changed, and — unlike [`check`]/[`insert`] — NO
/// failing layer is ever reported from this function itself.
#[allow(clippy::too_many_arguments)] // the Java read set, kept flat
pub fn check_layer(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    via_radius: f64,
    clearance_class_index: i32,
    attach_smd_allowed: bool,
    room_shape: &TileShape,
    location: &Point,
    layer: i32,
    net_numbers: &[i32],
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
    trace_half_width: i32,
    trace_clearance_class: i32,
) -> CheckDrillResult {
    if via_radius <= 0.0 {
        return CheckDrillResult::Drillable;
    }
    let int_location = match location {
        Point::Int(p) => *p,
        Point::Rational(_) => return CheckDrillResult::NotDrillable,
    };
    let via_shape = Circle::new(int_location, via_radius.ceil() as i32);

    let check_radius = via_radius
        + 0.5
            * f64::from(board.clearance_value(clearance_class_index, clearance_class_index, layer))
        + f64::from(board.min_trace_half_width());

    let is_ninety_degree = board.rules().trace_angle_restriction == AngleRestriction::NinetyDegree;
    let (tile_shape, is_90_degree) = if is_ninety_degree {
        (
            TileShape::RegularTileShape(RegularTileShape::IntBox(via_shape.bounding_box())),
            true,
        )
    } else {
        (
            TileShape::RegularTileShape(RegularTileShape::IntOctagon(via_shape.bounding_octagon())),
            false,
        )
    };

    // Java: `calculateFromSide(location.toFloat(), tileShape,
    // roomShape.toSimplex(), checkRadius, is90Degree)`.
    let from_side = calculate_from_side(
        &location.to_float(),
        &tile_shape,
        &room_shape.to_simplex(),
        check_radius,
        is_90_degree,
    );
    let Some(from_side) = from_side else {
        return CheckDrillResult::NotDrillable;
    };

    let via_result = check_forced_pad(
        manager,
        board,
        &tile_shape,
        from_side,
        layer,
        net_numbers,
        clearance_class_index,
        attach_smd_allowed,
        // Java `null` ignore items and `null` time limit.
        &[],
        max_recursion_depth,
        max_via_recursion_depth,
        false,
        None,
    );
    if via_result == CheckDrillResult::NotDrillable {
        return via_result;
    }

    if trace_half_width <= 0 {
        return via_result;
    }

    let start_trace_circle = Circle::new(int_location, trace_half_width);
    let start_trace_shape = if is_ninety_degree {
        TileShape::RegularTileShape(RegularTileShape::IntBox(start_trace_circle.bounding_box()))
    } else {
        TileShape::RegularTileShape(RegularTileShape::IntOctagon(
            start_trace_circle.bounding_octagon(),
        ))
    };

    let trace_result = check_forced_pad(
        manager,
        board,
        &start_trace_shape,
        from_side,
        layer,
        net_numbers,
        trace_clearance_class,
        // copper sharing IS allowed for the start-trace check.
        true,
        &[],
        max_recursion_depth,
        max_via_recursion_depth,
        false,
        None,
    );
    if trace_result == CheckDrillResult::NotDrillable {
        return trace_result;
    }
    if via_result == CheckDrillResult::DrillableWithAttachSmd
        || trace_result == CheckDrillResult::DrillableWithAttachSmd
    {
        return CheckDrillResult::DrillableWithAttachSmd;
    }
    CheckDrillResult::Drillable
}

/// Java static `check` (`:131-240`): is a via with the input parameters
/// possible, shoving obstacle traces aside? The board is NOT changed.
/// Every padstack layer is probed; copper-less layers fall back to the
/// hole-clearance shape at class 0, and a positive
/// `trace_pen_halfwidth_arr[i]` additionally checks room for a start
/// trace. Reports the failing layer on every early-out.
#[allow(clippy::too_many_arguments)] // the Java read set, kept flat
pub fn check(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    via_info: &ViaInfo,
    location: &Point,
    net_numbers: &[i32],
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
    trace_pen_halfwidth_arr: Option<&[i32]>,
    trace_clearance_class_index: i32,
) -> bool {
    let translate_vector = location.difference_by(&Point::ZERO);
    let calc_from_side_offset = board.min_trace_half_width();
    // Copy the padstack data out before the recursion: the pad shapes
    // are read per layer while the loop body re-borrows the board
    // mutably (Java reads them off the shared Padstack object).
    let (pad_shapes, hole_shape, from_layer, to_layer) = {
        let padstack = board
            .library()
            .padstack(via_info.padstack_no)
            .expect("Java NPE: unknown via padstack");
        (
            padstack.shapes.clone(),
            hole_check_shape(padstack, location, board),
            padstack.from_layer() as i32,
            padstack.to_layer(),
        )
    };
    let is_ninety_degree = board.rules().trace_angle_restriction == AngleRestriction::NinetyDegree;
    let mut i = from_layer;
    while i <= to_layer {
        let current_class;
        let shape_to_check: BoardShape = match &pad_shapes[i as usize] {
            None => {
                let Some(hole) = &hole_shape else {
                    i += 1;
                    continue;
                };
                // The drill hole itself must keep hole clearance from
                // copper on this layer; the shape is already built at
                // the ABSOLUTE location (no translation).
                current_class = 0;
                BoardShape::Circle(*hole)
            }
            Some(pad_shape) => {
                current_class = via_info.clearance_class;
                pad_shape.translate_by(&translate_vector)
            }
        };
        let tile_shape = if is_ninety_degree {
            TileShape::RegularTileShape(RegularTileShape::IntBox(shape_to_check.bounding_box()))
        } else {
            shape_to_check
                .bounding_octagon()
                .expect("Java NPE: unbounded padstack shape")
        };
        let from_side = calc_from_side(
            manager,
            board,
            &tile_shape,
            location,
            i,
            calc_from_side_offset,
            current_class,
        );
        if check_forced_pad(
            manager,
            board,
            &tile_shape,
            from_side,
            i,
            net_numbers,
            current_class,
            via_info.attach_smd_allowed,
            &[],
            max_recursion_depth,
            max_via_recursion_depth,
            false,
            None,
        ) == CheckDrillResult::NotDrillable
        {
            board.set_shove_failing_layer(i);
            return false;
        }
        // The drill hole must ALSO keep hole clearance from other-net
        // copper on layers where the pad exists — the pad check above
        // only enforces the (smaller) copper clearance.
        if current_class != 0
            && let Some(hole) = &hole_shape
        {
            let hole_tile = if is_ninety_degree {
                TileShape::RegularTileShape(RegularTileShape::IntBox(hole.bounding_box()))
            } else {
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(hole.bounding_octagon()))
            };
            if check_forced_pad(
                manager,
                board,
                &hole_tile,
                from_side,
                i,
                net_numbers,
                0,
                via_info.attach_smd_allowed,
                &[],
                max_recursion_depth,
                max_via_recursion_depth,
                false,
                None,
            ) == CheckDrillResult::NotDrillable
            {
                board.set_shove_failing_layer(i);
                return false;
            }
        }

        // Java `:210-213`: `tracePenHalfwidthArr != null &&
        // i < tracePenHalfwidthArr.length &&
        // tracePenHalfwidthArr[i] > 0 && location instanceof IntPoint`.
        if let Some(pen_arr) = trace_pen_halfwidth_arr
            && (i as usize) < pen_arr.len()
            && pen_arr[i as usize] > 0
            && let Point::Int(trace_point) = *location
        {
            let start_trace_circle = Circle::new(trace_point, pen_arr[i as usize]);
            let start_trace_shape = if is_ninety_degree {
                TileShape::RegularTileShape(RegularTileShape::IntBox(
                    start_trace_circle.bounding_box(),
                ))
            } else {
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(
                    start_trace_circle.bounding_octagon(),
                ))
            };
            if check_forced_pad(
                manager,
                board,
                &start_trace_shape,
                from_side,
                i,
                net_numbers,
                trace_clearance_class_index,
                true,
                &[],
                max_recursion_depth,
                max_via_recursion_depth,
                false,
                None,
            ) == CheckDrillResult::NotDrillable
            {
                board.set_shove_failing_layer(i);
                return false;
            }
        }
        i += 1;
    }
    true
}

/// Java static `insert` (`:249-356`): shoves aside traces so that a via
/// with the input parameters can be inserted without clearance
/// violations, then inserts it (UNFIXED, attach flag from the via
/// info). If the shove failed the database may be damaged — call
/// [`check`] first.
///
/// QUIRK: `tracePenHalfwidthArr[i]` is indexed WITHOUT the bounds test
/// [`check`] has (`:277` vs `:211`) — a short array throws
/// `ArrayIndexOutOfBoundsException` in Java; the slice index here
/// panics identically.
#[allow(clippy::too_many_arguments)] // the Java read set, kept flat
pub fn insert(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    via_info: &ViaInfo,
    location: &Point,
    net_numbers: &[i32],
    trace_clearance_class_index: i32,
    trace_pen_halfwidth_arr: &[i32],
    max_recursion_depth: i32,
    max_via_recursion_depth: i32,
) -> bool {
    let translate_vector = location.difference_by(&Point::ZERO);
    let calc_from_side_offset = board.min_trace_half_width();
    // Copy the padstack data out before the recursion (see `check`):
    // the loop body re-borrows the board mutably.
    let (pad_shapes, hole_shape, from_layer, to_layer) = {
        let padstack = board
            .library()
            .padstack(via_info.padstack_no)
            .expect("Java NPE: unknown via padstack");
        (
            padstack.shapes.clone(),
            hole_check_shape(padstack, location, board),
            padstack.from_layer() as i32,
            padstack.to_layer(),
        )
    };
    let is_ninety_degree = board.rules().trace_angle_restriction == AngleRestriction::NinetyDegree;
    let mut i = from_layer;
    while i <= to_layer {
        let current_class;
        let shape_to_check: BoardShape = match &pad_shapes[i as usize] {
            None => {
                let Some(hole) = &hole_shape else {
                    i += 1;
                    continue;
                };
                current_class = 0;
                BoardShape::Circle(*hole)
            }
            Some(pad_shape) => {
                current_class = via_info.clearance_class;
                pad_shape.translate_by(&translate_vector)
            }
        };
        // Java `:277` — the UNGUARDED index (the AIOOBE quirk).
        let start_trace_circle: Option<Circle> = if trace_pen_halfwidth_arr[i as usize] > 0 {
            match location {
                Point::Int(point) => Some(Circle::new(*point, trace_pen_halfwidth_arr[i as usize])),
                Point::Rational(_) => None,
            }
        } else {
            None
        };
        let (tile_shape, start_trace_shape) = if is_ninety_degree {
            (
                TileShape::RegularTileShape(RegularTileShape::IntBox(
                    shape_to_check.bounding_box(),
                )),
                start_trace_circle.as_ref().map(|circle| {
                    TileShape::RegularTileShape(RegularTileShape::IntBox(circle.bounding_box()))
                }),
            )
        } else {
            (
                shape_to_check
                    .bounding_octagon()
                    .expect("Java NPE: unbounded padstack shape"),
                start_trace_circle.as_ref().map(|circle| {
                    TileShape::RegularTileShape(RegularTileShape::IntOctagon(
                        circle.bounding_octagon(),
                    ))
                }),
            )
        };
        let from_side = calc_from_side(
            manager,
            board,
            &tile_shape,
            location,
            i,
            calc_from_side_offset,
            current_class,
        );
        if !forced_pad(
            manager,
            board,
            &tile_shape,
            from_side,
            i,
            net_numbers,
            current_class,
            via_info.attach_smd_allowed,
            &[],
            max_recursion_depth,
            max_via_recursion_depth,
        ) {
            board.set_shove_failing_layer(i);
            return false;
        }
        if current_class != 0
            && let Some(hole) = &hole_shape
        {
            let hole_tile = if is_ninety_degree {
                TileShape::RegularTileShape(RegularTileShape::IntBox(hole.bounding_box()))
            } else {
                TileShape::RegularTileShape(RegularTileShape::IntOctagon(hole.bounding_octagon()))
            };
            if !forced_pad(
                manager,
                board,
                &hole_tile,
                from_side,
                i,
                net_numbers,
                0,
                via_info.attach_smd_allowed,
                &[],
                max_recursion_depth,
                max_via_recursion_depth,
            ) {
                board.set_shove_failing_layer(i);
                return false;
            }
        }
        if let Some(start_shape) = start_trace_shape {
            // necessary in case startTraceShape is bigger than tileShape
            if !forced_pad(
                manager,
                board,
                &start_shape,
                from_side,
                i,
                net_numbers,
                trace_clearance_class_index,
                true,
                &[],
                max_recursion_depth,
                max_via_recursion_depth,
            ) {
                board.set_shove_failing_layer(i);
                return false;
            }
        }
        i += 1;
    }
    let int_location = match location {
        Point::Int(p) => *p,
        // Java `insertVia` stores the Point datum as-is; the port's via
        // datum is an IntPoint and every production caller is integer.
        Point::Rational(_) => panic!("Java ClassCastException: non-integer via location"),
    };
    insert_via(
        manager,
        board,
        via_info.padstack_no,
        int_location,
        net_numbers,
        via_info.clearance_class,
        FixedState::Unfixed,
        via_info.attach_smd_allowed,
    );
    true
}

/// Java private `holeCheckShape` (`:363-375`): the drill-clearance
/// substitute shape for copper-less padstack layers — a circle at the
/// location inflated by `drillRadius + holeClearance + 10` (the
/// trailing `+ 10` is verbatim Java). `None` when the rule is off, the
/// location is non-integer, or no drill radius is known.
fn hole_check_shape(padstack: &BoardPadstack, location: &Point, board: &Board) -> Option<Circle> {
    let hole_clearance = board.rules().hole_clearance;
    if hole_clearance <= 0 {
        return None;
    }
    let center = match location {
        Point::Int(p) => *p,
        Point::Rational(_) => return None,
    };
    let drill_radius = padstack.drill_radius();
    if drill_radius <= 0.0 {
        return None;
    }
    // Inflate by the hole clearance itself and check with the null
    // clearance class (0), so the requirement is exact hole-to-copper
    // spacing regardless of the neighbor's class.
    Some(Circle::new(
        center,
        (drill_radius + f64::from(hole_clearance) + 10.0).ceil() as i32,
    ))
}

/// Java private `calculateFromSide` (`:377-461`): probes the four
/// cardinal directions (then the four diagonals, distance divided by
/// `Limits.sqrt2`) for a probe point `dist` away from the via location
/// that lies inside the room simplex; the first hit answers a
/// pre-computed [`ShapeEntrySide`] through the via box's border in that
/// direction. `None` when no probe hits (90-degree mode never tries the
/// diagonals). The side numbering: cardinal `i` maps to side `2 * i`,
/// diagonal `i` to `2 * i + 1` (side `i` directly in 90-degree mode).
#[allow(clippy::too_many_arguments)] // mirrors nothing; clarity
fn calculate_from_side(
    via_location: &FloatPoint,
    via_shape: &TileShape,
    room_shape: &Simplex,
    dist: f64,
    is_90_degree: bool,
) -> Option<ShapeEntrySide> {
    let via_box = via_shape.bounding_box();
    for i in 0..4 {
        let (check_point, border_point) = match i {
            0 => (
                FloatPoint::new(via_location.x, via_location.y - dist),
                FloatPoint::new(via_location.x, f64::from(via_box.ll.y)),
            ),
            1 => (
                FloatPoint::new(via_location.x + dist, via_location.y),
                FloatPoint::new(f64::from(via_box.ur.x), via_location.y),
            ),
            2 => (
                FloatPoint::new(via_location.x, via_location.y + dist),
                FloatPoint::new(via_location.x, f64::from(via_box.ur.y)),
            ),
            _ => (
                FloatPoint::new(via_location.x - dist, via_location.y),
                FloatPoint::new(f64::from(via_box.ll.x), via_location.y),
            ),
        };
        if simplex_contains_float(room_shape, &check_point) {
            let from_side_index = if is_90_degree { i } else { 2 * i };
            return Some(ShapeEntrySide::new_precomputed(
                from_side_index,
                Some(border_point),
            ));
        }
    }
    if is_90_degree {
        return None;
    }
    // try the diagonal directions
    let dist = dist / SQRT2;
    let border_dist = via_box.max_width() / (2.0 * SQRT2);
    for i in 0..4 {
        let (check_point, border_point) = match i {
            0 => (
                FloatPoint::new(via_location.x + dist, via_location.y - dist),
                FloatPoint::new(via_location.x + border_dist, via_location.y - border_dist),
            ),
            1 => (
                FloatPoint::new(via_location.x + dist, via_location.y + dist),
                FloatPoint::new(via_location.x + border_dist, via_location.y + border_dist),
            ),
            2 => (
                FloatPoint::new(via_location.x - dist, via_location.y + dist),
                FloatPoint::new(via_location.x - border_dist, via_location.y + border_dist),
            ),
            _ => (
                FloatPoint::new(via_location.x - dist, via_location.y - dist),
                FloatPoint::new(via_location.x - border_dist, via_location.y - border_dist),
            ),
        };
        if simplex_contains_float(room_shape, &check_point) {
            let from_side_index = 2 * i + 1;
            return Some(ShapeEntrySide::new_precomputed(
                from_side_index,
                Some(border_point),
            ));
        }
    }
    None
}

/// Java `TileShape.contains(FloatPoint)` (`TileShape.java:163-183`) —
/// `calculateFromSide` calls it through the `roomShape.toSimplex()`
/// face: contained iff EVERY border line has the point strictly on its
/// right side (or on the line; tolerance 0).
fn simplex_contains_float(room: &Simplex, point: &FloatPoint) -> bool {
    let line_count = room.border_line_count() as i32;
    if line_count == 0 {
        return false;
    }
    for i in 0..line_count {
        if room.border_line(i).side_of_float_zero(point) != Side::Negative {
            return false;
        }
    }
    true
}

/// The T9-via-debt checkLayer ladder: literal verdict captures from
/// `rust/harness/oracle/ShapeTraceDebtProbe.main()` (`logs/M3-T10b/
/// captures/shape_debt_rows.jsonl`, byte-identical double run), run on
/// the shared M3-T10b debt world (`shape_trace_entries::debt_world`)
/// whose w1-w8 inserts plus the v3/v5 SHOVE_FIXED N003 discriminators
/// replay ids 105-129 of the Java probe board.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::ItemId;
    use crate::items::ItemData;
    use crate::shape_trace_entries::debt_world::{build_debt_world, ibox, p};
    use crate::trace_ops::insert_trace_without_cleaning;
    use epic_geometry::int_box::IntBox;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::polyline::Polyline;

    /// Capture `v1_zero_radius`: a non-positive radius short-circuits
    /// to DRILLABLE before any board query (the room here actually
    /// contains the w5 vias — the short-circuit makes that invisible,
    /// which is exactly what Java does).
    #[test]
    fn v1_zero_radius_is_drillable() {
        let (mut manager, mut board, _w) = build_debt_world();
        let room = ibox(498000, 314800, 502000, 318800);
        let result = check_layer(
            &mut manager,
            &mut board,
            0.0,
            0,
            false,
            &room,
            &p(500000, 316000),
            0,
            &[2],
            10,
            0,
            0,
            0,
        );
        assert_eq!(result, CheckDrillResult::Drillable, "capture v1");
    }

    /// Capture `v2_pin_69` (x=340000, y=20000, nets [46]): a foreign
    /// SMD pin blocks the drill, but with attachSmdAllowed the
    /// checkForcedPad arm downgrades the verdict to
    /// DRILLABLE_WITH_ATTACH_SMD. Kill target: dropping the attach
    /// plumbing flips the with-attach face to NOT_DRILLABLE.
    #[test]
    fn v2_smd_pin_attach_ladder_matches_the_jar() {
        let (mut manager, mut board, _w) = build_debt_world();
        // drift guard: the capture row's pin identity (the row carries
        // x/y only; the nets come from the board — id 69 is net 66)
        let pin = board.get(ItemId::new(69)).expect("pin 69");
        assert!(matches!(pin.data, ItemData::Pin { .. }), "69 is a pin");
        assert_eq!(pin.nets, vec![66], "pin 69 nets");
        let room = ibox(338000, 18000, 342000, 22000);
        let with_attach = check_layer(
            &mut manager,
            &mut board,
            100.0,
            0,
            true,
            &room,
            &p(340000, 20000),
            0,
            &[66],
            10,
            0,
            0,
            0,
        );
        let no_attach = check_layer(
            &mut manager,
            &mut board,
            100.0,
            0,
            false,
            &room,
            &p(340000, 20000),
            0,
            &[66],
            10,
            0,
            0,
            0,
        );
        assert_eq!(
            with_attach,
            CheckDrillResult::DrillableWithAttachSmd,
            "capture v2_pin_69 withAttach"
        );
        assert_eq!(
            no_attach,
            CheckDrillResult::NotDrillable,
            "capture v2_pin_69 noAttach"
        );
    }

    /// Capture `v2_pin_5` (x=20000, y=20000, nets [1]): a pin of a
    /// DIFFERENT net than the probe blocks on both faces — the
    /// attach downgrade never fires. Together with v2_pin_69 this
    /// separates the same-net (downgrade) from the foreign-net (hard
    /// block) pin arms.
    #[test]
    fn v2_foreign_pin_blocks_both_faces() {
        let (mut manager, mut board, _w) = build_debt_world();
        let pin = board.get(ItemId::new(5)).expect("pin 5");
        assert!(matches!(pin.data, ItemData::Pin { .. }), "5 is a pin");
        assert_eq!(pin.nets, vec![1], "capture pin nets");
        let room = ibox(18000, 18000, 22000, 22000);
        let with_attach = check_layer(
            &mut manager,
            &mut board,
            100.0,
            0,
            true,
            &room,
            &p(20000, 20000),
            0,
            &[1],
            10,
            0,
            0,
            0,
        );
        let no_attach = check_layer(
            &mut manager,
            &mut board,
            100.0,
            0,
            false,
            &room,
            &p(20000, 20000),
            0,
            &[1],
            10,
            0,
            0,
            0,
        );
        assert_eq!(
            (with_attach, no_attach),
            (
                CheckDrillResult::NotDrillable,
                CheckDrillResult::NotDrillable
            ),
            "capture v2_pin_5"
        );
    }

    /// Captures `v3_start_trace` / `v3_via_only`: with a start-trace
    /// half width of 400 the check sweeps a circle reaching the
    /// SHOVE_FIXED N003 trace 450 above the probe point (its surface
    /// sits at 350) — the shove cannot move a SHOVE_FIXED trace, so
    /// the verdict is NOT_DRILLABLE; with half width 0 no start-trace
    /// check runs and the via octagon (~116 incl. clearance) clears
    /// it. Kill target: a mutant that ignores traceHalfWidth (both
    /// faces go DRILLABLE).
    #[test]
    fn v3_shove_fixed_trace_separates_start_trace_from_via_only() {
        let (mut manager, mut board, _w) = build_debt_world();
        let room = ibox(490000, 314800, 494000, 318800);
        let start_trace = check_layer(
            &mut manager,
            &mut board,
            100.0,
            0,
            true,
            &room,
            &p(492000, 316500),
            0,
            &[2],
            10,
            0,
            400,
            0,
        );
        let via_only = check_layer(
            &mut manager,
            &mut board,
            100.0,
            0,
            true,
            &room,
            &p(492000, 316500),
            0,
            &[2],
            10,
            0,
            0,
            0,
        );
        assert_eq!(
            start_trace,
            CheckDrillResult::NotDrillable,
            "capture v3_start_trace"
        );
        assert_eq!(via_only, CheckDrillResult::Drillable, "capture v3_via_only");
    }

    /// Capture `v4_room_excluded`: the probe location sits outside the
    /// room shape, so every from-side probe fails and the answer is
    /// NOT_DRILLABLE before any board geometry is consulted.
    #[test]
    fn v4_probe_outside_room_is_not_drillable() {
        let (mut manager, mut board, _w) = build_debt_world();
        let room = ibox(499000, 299000, 501000, 301000);
        let result = check_layer(
            &mut manager,
            &mut board,
            100.0,
            0,
            false,
            &room,
            &p(506000, 306000),
            0,
            &[2],
            10,
            0,
            0,
            0,
        );
        assert_eq!(result, CheckDrillResult::NotDrillable, "capture v4");
    }

    /// Captures `v5_ninety_clean` / `v5_ninety_trace` under the
    /// NINETY_DEGREE angle restriction (IntBox tiles, direct side
    /// numbering — the restriction is flipped and restored exactly like
    /// the probe): clean DRILLABLE, but the start-trace sweep at half
    /// width 400 hits the SHOVE_FIXED N003 trace 450 above the probe
    /// point. Kill target: an angle-branch mutant that keeps building
    /// 45-degree octagon tiles can flip the tile geometry and the
    /// from-side probes with it.
    #[test]
    fn v5_ninety_degree_branch_ladder_matches_the_jar() {
        let (mut manager, mut board, _w) = build_debt_world();
        board.rules_mut().trace_angle_restriction = AngleRestriction::NinetyDegree;
        let room = ibox(504200, 315900, 508200, 319900);
        let clean = check_layer(
            &mut manager,
            &mut board,
            150.0,
            0,
            true,
            &room,
            &p(506200, 317900),
            0,
            &[2],
            10,
            0,
            0,
            0,
        );
        let with_trace = check_layer(
            &mut manager,
            &mut board,
            150.0,
            0,
            true,
            &room,
            &p(506200, 317900),
            0,
            &[2],
            10,
            0,
            400,
            0,
        );
        board.rules_mut().trace_angle_restriction = AngleRestriction::FortyfiveDegree;
        assert_eq!(
            clean,
            CheckDrillResult::Drillable,
            "capture v5_ninety_clean"
        );
        assert_eq!(
            with_trace,
            CheckDrillResult::NotDrillable,
            "capture v5_ninety_trace"
        );
    }

    /// The second hole-check arm of the pad layers (`:196-211`) and the
    /// `holeCheckShape` +10 quirk (`:374`). World: the debt board plus a
    /// hole padstack "hole_60:30" (boxes +-60 on BOTH layers — a
    /// trailing None slot is never probed, `toLayer()` stops at the
    /// last Some) and a SHOVE_FIXED N003 blocker trace (hw 20,
    /// centerline y = 306096).
    ///
    /// Query model (the T10b lesson, drift-guarded below), three tiers:
    ///
    /// 1. STORED: the default tree is NOT clearance-compensated, so
    ///    the blocker's stored shape is its geometry +- the plain half
    ///    width (class-0 items never compensate — the `item_class <= 0`
    ///    guard). At centerline C the stored band is y in [C-20, C+20].
    /// 2. CANDIDATE PRUNE: `overlappingItemsWithClearance` offsets the
    ///    query BOUND by `(int) (1.2 * clearanceMatrix.maxValue(class,
    ///    layer))` and R-tree-overlaps the RAW stored shapes.
    ///    `maxValue(classI, layer)` reads `row[classI].maxValue` — a
    ///    zero-initialized accumulator updated only by `setValue`,
    ///    which writes `row[classJ]`; the parse targets only rows and
    ///    columns of index 1 or above (class numbers from `getNo` or
    ///    the "wire" default 1; `setDefaultValue` loops from 1). Java
    ///    parity: `maxValue(0, 0)` is 0 (row 0 never written — offset
    ///    0, RAW reach for the class-0 hole query) but `maxValue(1,
    ///    0)` is 2500 (the fixture's `(rule (clearance 250))` default
    ///    fill, parse-scaled x10 — offset 3000, the blocker is a
    ///    CANDIDATE of the class-1 pad query).
    /// 3. ACCEPTANCE: every candidate must pass `clearance_test`,
    ///    which enlarges query and stored by `cl / 2` per side where
    ///    `cl = getValue(query, item, layer, +safety margin)`; the
    ///    cells (0,0) and (1,0) are unwritten (0) and the margin is
    ///    Java's `clearance_safety_margin = 16` -> +-8 per side for
    ///    both arms. The +-8 dominates the OBSERVABLE reach: it
    ///    filters the class-1 prune's far candidates and subsumes the
    ///    class-0 prune's raw reach.
    ///
    /// Arithmetic at C = 306098 = 306000 + 98 (stored bottom 306078):
    /// pad box top 306060 + 8 = 306068 < 306078 - 8 = 306070 -> the
    /// pad query MISSES the blocker (the class-0 face proves the pad
    /// arm alone is blind); hole radius (30 + 40 + 10) = 80 -> hole
    /// octagon top 306080 >= 306078 (raw candidate) and 306088 >=
    /// 306070 (accept) -> the hole query HITS -> `storeItems`'s
    /// shove-fixed gate fires on the blocker -> NOT drillable with
    /// failingLayer 0. The window is (center+96, center+100]: below
    /// center+96 the pad ACCEPTANCE hits, above center+100 the hole
    /// PRUNE loses the blocker. Kill targets: the +10 quirk (radius
    /// 70 -> top 306068 < 306078 -> the hole query is EMPTY ->
    /// drillable — the mutant dies at the QUERY, before any
    /// entrance-point arithmetic), and the `currentClass != 0` gate
    /// (class-0 vias skip the arm entirely).
    #[test]
    fn t11_hole_clearance_second_arm_and_plus_ten_quirk() {
        let (mut manager, mut board, w) = build_debt_world();
        board.rules_mut().set_hole_clearance(40);
        let rect = || {
            Some(BoardShape::Tile(TileShape::RegularTileShape(
                RegularTileShape::IntBox(IntBox::new(
                    IntPoint::new(-60, -60),
                    IntPoint::new(60, 60),
                )),
            )))
        };
        board.library_mut().padstacks.push(BoardPadstack {
            name: "hole_60:30".to_string(),
            shapes: vec![rect(); 2],
            drillable: false,
            placed_absolute: false,
            hole_only: false,
        });
        let padstack_no = board.library().padstacks.len() as i32;
        assert!(
            (board.library().padstacks[padstack_no as usize - 1].drill_radius() - 30.0).abs()
                < 1e-9,
            "drill radius 60 * 30/60"
        );
        // drift guards: the whole face arithmetic sits on these facts.
        assert_eq!(board.clearance_value(0, 0, 0), 16, "cl00 (world guard)");
        assert_eq!(board.clearance_value(1, 0, 0), 16, "cl10 (pad-face guard)");
        assert_eq!(
            board.rules().clearance.max_value(0, 0),
            0,
            "row-0 accumulator unwritten (Java parity: the parse never writes row 0)"
        );
        assert_eq!(
            board.rules().clearance.max_value(1, 0),
            2500,
            "row-1 accumulator holds the fixture's default clearance (DSN 250, parse x10)"
        );
        assert!(
            !manager.default_tree().is_clearance_compensation_used(),
            "the default tree is NOT clearance-compensated (stored = plain geometry)"
        );
        assert_eq!(
            crate::tree_shapes::clearance_compensation_value(board.rules(), 0, 0, 0),
            0,
            "class-0 items never compensate (the item_class <= 0 guard)"
        );

        let blocker = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(503000, 306098), &p(509000, 306098)),
            0,
            20,
            &[w.foreign2],
            0,
            FixedState::ShoveFixed,
        )
        .expect("hole blocker");

        // class 0: the `currentClass != 0` gate skips the hole arm —
        // the blocker only violates the HOLE clearance, never the pad.
        let class0 = ViaInfo {
            name: "hole_probe_c0".to_string(),
            padstack_no,
            clearance_class: 0,
            attach_smd_allowed: false,
        };
        assert!(
            check(
                &mut manager,
                &mut board,
                &class0,
                &p(505000, 306000),
                &[w.own],
                10,
                10,
                None,
                0
            ),
            "class 0 skips the hole-vs-copper arm"
        );
        // class 1: the hole arm fires and fails on the SHOVE_FIXED
        // blocker at layer 0.
        let class1 = ViaInfo {
            name: "hole_probe_c1".to_string(),
            padstack_no,
            clearance_class: 1,
            attach_smd_allowed: false,
        };
        assert!(
            !check(
                &mut manager,
                &mut board,
                &class1,
                &p(505000, 306000),
                &[w.own],
                10,
                10,
                None,
                0
            ),
            "the hole arm fails on the blocker"
        );
        assert_eq!(
            board.shove_failing_layer(),
            0,
            "the failing layer is the pad layer"
        );
        assert!(
            board.is_on_the_board(blocker),
            "the check is pure: the blocker stays"
        );
        // holeClearance 0 -> holeCheckShape None -> the arm vanishes
        // and the same class-1 via drills.
        board.rules_mut().set_hole_clearance(0);
        assert!(
            check(
                &mut manager,
                &mut board,
                &class1,
                &p(505000, 306000),
                &[w.own],
                10,
                10,
                None,
                0
            ),
            "no hole rule: no hole arm"
        );
    }
}
