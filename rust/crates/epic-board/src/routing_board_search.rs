//! The routing search-and-clearance queries (Java
//! `board/facade/RoutingBoardSearchFacade.java`, the search-facade
//! surface behind the thin `RoutingBoard.checkTraceSegment` facades at
//! `:198`/:223`).
//!
//! Java anchors:
//!
//! * `checkTraceSegment(Point, Point, ...)` `:29-41` — the point-pair
//!   face ([`check_trace_segment_points`]): equal corners answer 0,
//!   anything else wraps the two corners into a `Polyline` and a
//!   1-based `LineSegment` and delegates.
//! * `checkTraceSegment(LineSegment, ...)` `:43-109` — the segment
//!   face ([`check_trace_segment`]): the maximal length of the
//!   segment insertable at full width, where `Double.MAX_VALUE`…
//!   — concretely Java's `Integer.MAX_VALUE` widened to double
//!   (`2147483647.0`) — means "no conflict" and a shorter positive
//!   value is the length the nearest obstacle allows (the contract
//!   `MazeTraceShover` and `FoundConnectionInserter.tryNeckDown` read).
//!
//! ## The shortened-length arithmetic (verbatim)
//!
//! Per obstacle entry of the default tree's
//! `overlappingTreeEntriesWithClearance` query: intersect the
//! UNcompensated offset shape of the probe segment with the obstacle's
//! stored tree shape; project the nearest intersection point onto the
//! segment (`scalarProduct / lineLength`), shorten by the trace half
//! width plus the obstacle's clearance (or compensation) value plus
//! Java's `-1` safety margin, clamp at 0, keep the running minimum and
//! early-return 0 the moment a projection hits non-positive.
//!
//! The two clearance regimes mirror the tree the query ran on:
//! a compensation tree (`compensatedClearanceClassNo > 0`) bakes the
//! compensation into the STORED shapes, so the probe shape is used
//! as-is and the shortened value grows by the diagonal compensation
//! value; the plain tree offsets the probe shape by the rectangular
//! clearance value instead.

use epic_geometry::line_segment::LineSegment;
use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;

use crate::board::Board;
use crate::items::FixedState;
use crate::tree_manager::SearchTreeManager;

/// Java `RoutingBoardSearchFacade.checkTraceSegment(Point, Point, ...)`
/// (`:29-41`). `net_numbers` is never empty through real callers (the
/// engine passes the routed net); Java's null array would NPE inside
/// the tree query — the port passes slices.
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
pub fn check_trace_segment_points(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    from_point: &Point,
    to_point: &Point,
    layer: i32,
    net_numbers: &[i32],
    trace_half_width: i32,
    cl_class_no: i32,
    only_not_shovable_obstacles: bool,
) -> f64 {
    if from_point == to_point {
        return 0.0;
    }
    let current_polyline = Polyline::from_two_corners(from_point, to_point);
    let current_line_segment = LineSegment::from_polyline(&current_polyline, 1);
    check_trace_segment(
        manager,
        board,
        &current_line_segment,
        layer,
        net_numbers,
        trace_half_width,
        cl_class_no,
        only_not_shovable_obstacles,
    )
}

/// Java `RoutingBoardSearchFacade.checkTraceSegment(LineSegment, ...)`
/// (`:43-109`): returns `2147483647.0` (Java's `Integer.MAX_VALUE`
/// double widening) when no obstacle conflicts, else the maximal
/// insertable length from the start point.
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
pub fn check_trace_segment(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    line_segment: &LineSegment,
    layer: i32,
    net_numbers: &[i32],
    trace_half_width: i32,
    cl_class_no: i32,
    only_not_shovable_obstacles: bool,
) -> f64 {
    let check_polyline = line_segment.to_polyline();
    if check_polyline.lines.len() != 3 {
        return 0.0;
    }
    // Java `:54`: the offsetShape of the middle line at index 0 — for
    // a 3-line polyline the index is in range and Java's null never
    // materializes.
    let shape_to_check = check_polyline
        .offset_shape(trace_half_width, 0)
        .expect("middle-line offset shape of a 3-line polyline");
    let from_point = line_segment.start_point_approx();
    let to_point = line_segment.end_point_approx();
    let line_length = to_point.distance(&from_point);
    let mut ok_length = 2147483647.0_f64;

    // Java `:61-63`: the DEFAULT tree query (`getDefaultTree()` — the
    // slot-0 tree, [`SearchTreeManager::DEFAULT_TREE_INDEX`]). The
    // tree's identity constants are copied out before the `&mut`
    // query, which reborrows manager and board.
    let (tree_object_id, variant, tree_class, compensation_used) = {
        let tree = manager.default_tree();
        (
            tree.object_id(),
            tree.variant,
            tree.compensated_clearance_class,
            tree.is_clearance_compensation_used(),
        )
    };
    let obstacle_entries = manager.overlapping_tree_entries_with_clearance(
        board,
        SearchTreeManager::DEFAULT_TREE_INDEX,
        &shape_to_check,
        layer,
        net_numbers,
        cl_class_no,
    );

    for entry in obstacle_entries {
        // Java `:69-71`: `!(entry.object instanceof Item)` — the board
        // trees hold items only, but the mapping stays the gate.
        let Some(obstacle) = SearchTreeManager::item_of_key(entry.object_key) else {
            continue;
        };
        // Java `:72-75`: shovable obstacles (routable and not
        // shove-fixed) are skipped when the caller asks for the
        // NOT-shovable-only verdict.
        if only_not_shovable_obstacles
            && crate::trace_ops::is_routable(board, obstacle)
            && board
                .get(obstacle)
                .is_none_or(|obstacle_entry| obstacle_entry.fixed != FixedState::ShoveFixed)
        {
            continue;
        }
        // Java `:76-78`: the obstacle's STORED tree shape in the same
        // tree (the per-item per-tree cache).
        let Some(obstacle_shape) = board
            .tree_shape_precalc(obstacle, tree_object_id, variant, tree_class)
            .get(entry.shape_index_in_object as usize)
            .cloned()
            .flatten()
        else {
            // Java would NPE on a null stored shape; the port skips
            // the candidate (the convention of the manager's own
            // query cores).
            continue;
        };
        let Some(obstacle_class) = board.item_clearance_class(obstacle) else {
            // Java `clearanceClassIndex()` is never null for a live
            // item; the manager's query cores skip such candidates.
            continue;
        };
        // Java `:80-92`: the two clearance regimes.
        let (current_offset_shape, shorten_value) = if compensation_used {
            // The tree's stored shapes carry the compensation — probe
            // shape as-is, shorten by the diagonal compensation value.
            let shorten = f64::from(trace_half_width)
                + f64::from(
                    board
                        .rules()
                        .clearance
                        .clearance_compensation_value(obstacle_class, layer),
                );
            (shape_to_check.clone(), shorten)
        } else {
            let clearance_value = board.clearance_value(obstacle_class, cl_class_no, layer);
            let shorten = f64::from(trace_half_width) + f64::from(clearance_value);
            (shape_to_check.offset(f64::from(clearance_value)), shorten)
        };
        let intersection = obstacle_shape.intersection(&current_offset_shape);
        if intersection.is_empty() {
            continue;
        }
        let nearest_obstacle_point = intersection.nearest_point_approx(&from_point);
        // Java `:96-98`: the projection of the nearest point onto the
        // segment, shortened by the half width + clearance + Java's
        // safety `-1`, clamped at 0.
        let mut projection =
            from_point.scalar_product(&to_point, &nearest_obstacle_point) / line_length;
        projection = (projection - shorten_value - 1.0).max(0.0);
        if projection < ok_length {
            ok_length = projection;
            if ok_length <= 0.0 {
                return 0.0;
            }
        }
    }
    ok_length
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::parse_board_from_path;
    use crate::trace_ops::insert_trace_without_cleaning;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::polyline::Polyline;

    fn p(x: i32, y: i32) -> Point {
        Point::Int(IntPoint::new(x, y))
    }

    /// The T10c insert-world fixture (net 94, the corridor at y=300000
    /// with the foreign N002 trace at x=501000 and the N003 via at
    /// (500600,300000)) — the same `parse_fixture` seed the
    /// routing_board_insert tests use, so the id replay (105+) holds.
    fn parse_fixture() -> Board {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../rust/harness/fixtures/locator-spike/t9_locator45.dsn");
        let mut board = parse_board_from_path(&path.to_string_lossy());
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        board
    }

    /// A fresh parse seeded with the T10c insert world's two traces:
    /// net-94 own trace 105 ending at the center (500000,300000) and
    /// the foreign N002 vertical trace 106 at x=501000.
    fn seed_two_traces() -> (SearchTreeManager, Board) {
        let mut board = parse_fixture();
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let own = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(498000, 300000), &p(500000, 300000)),
            0,
            100,
            &[94],
            0,
            FixedState::Unfixed,
        )
        .expect("own trace");
        assert_eq!(own.get(), 105, "id replay");
        let cross = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(501000, 298000), &p(501000, 302000)),
            0,
            100,
            &[2],
            0,
            FixedState::Unfixed,
        )
        .expect("cross trace");
        assert_eq!(cross.get(), 106, "id replay");
        (manager, board)
    }

    /// A west-east corridor ending exactly AT the foreign trace's x:
    /// the foreign trace's offset shape blocks the last piece, so the
    /// verdict is a positive clipped length strictly below
    /// `Integer.MAX_VALUE` and the corridor cannot reach its target.
    /// The no-conflict corridor (ending short of the obstacle)
    /// answers the MAX_VALUE sentinel, and the equal-corners face
    /// answers 0.
    #[test]
    fn t11_check_trace_segment_faces() {
        let (mut manager, mut board) = seed_two_traces();

        // no-conflict face: the corridor from 499000 to 500500 stops
        // before the foreign trace at x=501000 (even its clearance
        // ring stays clear at this half width).
        let free = check_trace_segment_points(
            &mut manager,
            &mut board,
            &p(499000, 300000),
            &p(500500, 300000),
            0,
            &[94],
            100,
            0,
            false,
        );
        assert_eq!(free, 2147483647.0, "no-conflict answers MAX_VALUE");

        // clipped face: run INTO the foreign trace.
        let clipped = check_trace_segment_points(
            &mut manager,
            &mut board,
            &p(499000, 300000),
            &p(502000, 300000),
            0,
            &[94],
            100,
            0,
            false,
        );
        assert!(
            clipped > 0.0 && clipped < 2147483647.0,
            "the clipped length is a positive cut: {clipped}"
        );
        // The EXACT shortened-length arithmetic is pinned to the jar's
        // literals by `t11_check_trace_segment_jar_literals` below
        // (the probe world replay); this world only witnesses that
        // SOME obstacle cut the corridor.

        // equal-corners face.
        assert_eq!(
            check_trace_segment_points(
                &mut manager,
                &mut board,
                &p(499000, 300000),
                &p(499000, 300000),
                0,
                &[94],
                100,
                0,
                false,
            ),
            0.0
        );

        // degenerate 3-line face: the segment wrapper on a
        // non-degenerate diagonal polyline is the same query through
        // the LineSegment face.
        let polyline = Polyline::from_two_corners(&p(499000, 300000), &p(500500, 300000));
        let through_segment = check_trace_segment(
            &mut manager,
            &mut board,
            &LineSegment::from_polyline(&polyline, 1),
            0,
            &[94],
            100,
            0,
            false,
        );
        assert_eq!(
            through_segment, free,
            "both faces agree on the straight segment"
        );
    }

    /// The `onlyNotShovableObstacles` discriminator: an UNFIXED
    /// foreign trace is routable and shovable, so the flag skips it —
    /// the clipped world answers MAX_VALUE under the flag; a
    /// USER_FIXED foreign trace is NOT routable, so the flag cannot
    /// skip it and the verdict stays clipped.
    #[test]
    fn t11_check_trace_segment_not_shovable_flag() {
        let (mut manager, mut board) = seed_two_traces();

        // Shovable foreign trace: the flag SKIPS it.
        let flagged = check_trace_segment_points(
            &mut manager,
            &mut board,
            &p(499000, 300000),
            &p(502000, 300000),
            0,
            &[94],
            100,
            0,
            true,
        );
        assert_eq!(
            flagged, 2147483647.0,
            "a shovable obstacle is skipped under the flag"
        );
    }

    /// THE JAR LITERALS (AutorouteEngineProbe `check_segment` world,
    /// logs/M3-T11/captures — run1/run2 byte-identical): the probe's
    /// exact world replayed — the t9_locator45 fixture seeded with an
    /// UNFIXED net-2 wall at x=505000 and a USER_FIXED net-2 wall at
    /// x=545000 (both hw 1000, layer 0), probed along y=300000 with
    /// nets {94}, hw 1500, class 0. Java's verdicts, verbatim:
    /// clear corridor = `Integer.MAX_VALUE` widened; the unfixed wall
    /// clips to 2483.0 with the flag false and answers MAX_VALUE with
    /// the flag true (the wall is shovable, the probe skips it); the
    /// fixed wall clips to 7483.0 under BOTH flags (a USER_FIXED
    /// trace is not routable, hence never shovable). The literals
    /// exercise the full shortened-length arithmetic — offset shape,
    /// projection, clearance/compensation addend, the `-1` margin.
    #[test]
    fn t11_check_trace_segment_jar_literals() {
        let mut board = parse_fixture();
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let unfixed_wall = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(505000, 100000), &p(505000, 500000)),
            0,
            1000,
            &[2],
            0,
            FixedState::Unfixed,
        )
        .expect("unfixed wall");
        assert_eq!(unfixed_wall.get(), 105, "id replay");
        let fixed_wall = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(545000, 100000), &p(545000, 500000)),
            0,
            1000,
            &[2],
            0,
            FixedState::UserFixed,
        )
        .expect("fixed wall");
        assert_eq!(fixed_wall.get(), 106, "id replay");

        let world = |manager: &mut SearchTreeManager,
                     board: &mut Board,
                     from: (i32, i32),
                     to: (i32, i32),
                     flag: bool| {
            check_trace_segment_points(
                manager,
                board,
                &p(from.0, from.1),
                &p(to.0, to.1),
                0,
                &[94],
                1500,
                0,
                flag,
            )
        };
        // S0: the corridor between the keepout gap and the unfixed
        // wall — no conflict.
        assert_eq!(
            world(
                &mut manager,
                &mut board,
                (480000, 300000),
                (490000, 300000),
                false
            ),
            2147483647.0,
            "S0_clear_false"
        );
        // S1: the crossing of the UNFIXED wall — clipped under the
        // plain flag, MAX_VALUE under the not-shovable flag.
        assert_eq!(
            world(
                &mut manager,
                &mut board,
                (500000, 300000),
                (520000, 300000),
                false
            ),
            2483.0,
            "S1_unfixed_false"
        );
        assert_eq!(
            world(
                &mut manager,
                &mut board,
                (500000, 300000),
                (520000, 300000),
                true
            ),
            2147483647.0,
            "S1_unfixed_true"
        );
        // S2: the crossing of the USER_FIXED wall — clipped under
        // BOTH flags.
        assert_eq!(
            world(
                &mut manager,
                &mut board,
                (535000, 300000),
                (555000, 300000),
                false
            ),
            7483.0,
            "S2_fixed_false"
        );
        assert_eq!(
            world(
                &mut manager,
                &mut board,
                (535000, 300000),
                (555000, 300000),
                true
            ),
            7483.0,
            "S2_fixed_true"
        );
    }

    /// The USER_FIXED arm of the flag discriminator (a separate world:
    /// the cross trace is seeded `FixedState::UserFixed` at insert).
    #[test]
    fn t11_check_trace_segment_user_fixed_survives_the_flag() {
        let mut board = parse_fixture();
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let own = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(498000, 300000), &p(500000, 300000)),
            0,
            100,
            &[94],
            0,
            FixedState::Unfixed,
        )
        .expect("own trace");
        assert_eq!(own.get(), 105, "id replay");
        let cross = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&p(501000, 298000), &p(501000, 302000)),
            0,
            100,
            &[2],
            0,
            FixedState::UserFixed,
        )
        .expect("cross trace");
        assert_eq!(cross.get(), 106, "id replay");

        let flagged = check_trace_segment_points(
            &mut manager,
            &mut board,
            &p(499000, 300000),
            &p(502000, 300000),
            0,
            &[94],
            100,
            0,
            true,
        );
        assert!(
            flagged > 0.0 && flagged < 2147483647.0,
            "a USER_FIXED obstacle survives the flag: {flagged}"
        );
        // And the flag-off face on the same world still sees it too.
        let unflagged = check_trace_segment_points(
            &mut manager,
            &mut board,
            &p(499000, 300000),
            &p(502000, 300000),
            0,
            &[94],
            100,
            0,
            false,
        );
        assert!(
            unflagged > 0.0 && unflagged < 2147483647.0,
            "flag-off still clipped: {unflagged}"
        );
    }
}
