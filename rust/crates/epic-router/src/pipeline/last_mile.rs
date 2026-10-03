//! Java `LastMileBlockerRipup` (#933, upstream 339e8bb50 — "Speed up
//! single-thread routing; PCBench fully routed 51.9% to 73.8%",
//! survivor 2 of 3): removes a bounded set of unfixed foreign traces
//! and vias that overlap the bounding boxes of the remaining
//! airlines. Used when the autorouter has only a few incomplete
//! connections and the score has stopped improving — the batch loop
//! trigger lives in [`crate::pipeline::batch`].
//!
//! DIVERGENCE CLASS (documented, not fixed): the airline WALK ORDER
//! differs — Java `DesignRulesChecker.getAllAirlines()` walks its own
//! net/item order, this port's [`airline_segments`] face is
//! nets-ascending with the sorted-Edge Kruskal order within a net.
//! The order decides which blockers enter the [`MAX_ITEMS`]-capped
//! insertion set FIRST; it only matters on worlds with more than 24
//! candidate blockers across multiple airlines (the cap then binds).
//! Both orders are deterministic.

use epic_board::board::Board;
use epic_board::id::ItemId;
use epic_board::items::ItemData;
use epic_board::trace_ops::{is_deletion_forbidden, is_user_fixed};
use epic_board::trace_shover::remove_items;
use epic_board::tree_manager::SearchTreeManager;
use epic_drc::incompletes::airline_segments;
use epic_geometry::int_box::IntBox;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::tile_shape::TileShape;

/// Java `LastMileBlockerRipup.MAX_ITEMS` — upstream-tuned
/// (339e8bb50), not ours to re-derive.
pub const MAX_ITEMS: usize = 24;

/// Rips blockers for the current incomplete airlines. Returns the
/// number of items removed. Java `ripBlockers` (`:41-67`) 1:1: the
/// corridor of each netted airline, every overlapping item on every
/// layer (`layer = -1`), the removable blockers collected
/// insertion-ordered and deduped (Java `LinkedHashSet`) up to the
/// [`MAX_ITEMS`] cap (checked inside BOTH loops), then one bulk
/// `removeItems`.
pub fn rip_blockers(manager: &mut SearchTreeManager, board: &mut Board) -> usize {
    let airlines = airline_segments(manager, board);
    if airlines.is_empty() {
        return 0;
    }
    let mut blockers: Vec<ItemId> = Vec::new();
    for airline in &airlines {
        let corridor = corridor(airline.from, airline.to);
        let shape = TileShape::RegularTileShape(RegularTileShape::IntBox(corridor));
        let overlapping = manager.overlapping_objects(
            board,
            SearchTreeManager::DEFAULT_TREE_INDEX,
            &shape,
            -1,
            &[],
        );
        for id in overlapping {
            if blockers.len() >= MAX_ITEMS {
                break;
            }
            if !blockers.contains(&id) && is_removable_blocker(board, id, airline.net) {
                blockers.push(id);
            }
        }
        if blockers.len() >= MAX_ITEMS {
            break;
        }
    }
    if blockers.is_empty() {
        return 0;
    }
    remove_items(manager, board, &blockers);
    blockers.len()
}

/// Java `isRemovableBlocker` (`:69-84`): a Trace or Via that is not
/// deletion-forbidden, not user-fixed, carries NO net equal to the
/// airline's net, and carries at least one net.
#[must_use]
pub fn is_removable_blocker(board: &Board, id: ItemId, airline_net_number: i32) -> bool {
    let Some(entry) = board.get(id) else {
        return false;
    };
    if !matches!(entry.data, ItemData::Trace { .. } | ItemData::Via { .. }) {
        return false;
    }
    if is_deletion_forbidden(board, id) || is_user_fixed(entry) {
        return false;
    }
    if entry.nets.contains(&airline_net_number) {
        return false;
    }
    !entry.nets.is_empty()
}

/// Java `corridor` (`:86-96`): the airlines' corner bbox expanded by
/// `max(span / 4, 1)` — INTEGER division, at least 1 — in the i32
/// board-int domain (the f64 corners round into it exactly as Java's
/// `(int) Math.round` narrowing).
#[must_use]
pub fn corridor(from: (i64, i64), to: (i64, i64)) -> IntBox {
    // Java `(int) Math.round(double)` — saturating narrowing; the
    // segment corners are integral DBU, so this is an exact cast on
    // every real board.
    let (x1, y1) = (from.0 as i32, from.1 as i32);
    let (x2, y2) = (to.0 as i32, to.1 as i32);
    let min_x = x1.min(x2);
    let max_x = x1.max(x2);
    let min_y = y1.min(y2);
    let max_y = y1.max(y2);
    let span = (max_x - min_x).max(max_y - min_y);
    let margin = (span / 4).max(1);
    IntBox::from_corners(
        min_x - margin,
        min_y - margin,
        max_x + margin,
        max_y + margin,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{net_no, parse};
    use epic_board::items::FixedState;
    use epic_board::trace_ops::insert_trace_without_cleaning;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::point::Point;
    use epic_geometry::polyline::Polyline;

    /// The T9/T10c locator-world fixture (2 layers, `unit um`,
    /// resolution 10): nets 33 and 98 are pin PAIRS (two airlines),
    /// 49/94 single pins (no airline) — the sibling `batch`/
    /// `board_history` suites' world.
    fn parse_fixture() -> (SearchTreeManager, Board) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../harness/fixtures/locator-spike/t9_locator45.dsn");
        let text = std::fs::read_to_string(&path).expect("fixture present");
        parse(&text)
    }

    fn pt(x: i32, y: i32) -> Point {
        Point::Int(IntPoint::new(x, y))
    }

    /// Java `:86-96`'s margin law: the corners' bbox expanded by
    /// `max(span / 4, 1)` — integer division, floor of 1.
    #[test]
    fn corridor_margin_table() {
        // dx 400 > dy 200 → span 400 → margin 100.
        let wide = corridor((100, 100), (500, 300));
        assert_eq!(
            (wide.ll.x, wide.ll.y, wide.ur.x, wide.ur.y),
            (0, 0, 600, 400),
            "the bbox + span/4 margin"
        );
        // span 2 → 2/4 = 0 → the `max(…, 1)` floor.
        let tiny = corridor((0, 0), (2, 0));
        assert_eq!(
            (tiny.ll.x, tiny.ll.y, tiny.ur.x, tiny.ur.y),
            (-1, -1, 3, 1),
            "the min-1 margin"
        );
        // The integer-division rungs: span 4 → 1, span 8 → 2.
        let four = corridor((0, 0), (4, 0));
        assert_eq!((four.ll.x, four.ur.x), (-1, 5));
        let eight = corridor((0, 0), (8, 0));
        assert_eq!((eight.ll.x, eight.ur.x), (-2, 10));
        // Corner order normalizes — either direction, one box.
        assert_eq!(
            corridor((500, 300), (100, 100)),
            corridor((100, 100), (500, 300))
        );
        // Degenerate point airline: span 0 → floor 1.
        let point = corridor((10, 10), (10, 10));
        assert_eq!(
            (point.ll.x, point.ll.y, point.ur.x, point.ur.y),
            (9, 9, 11, 11)
        );
        // The span is max(dx, dy): dy 700 drives this one (175).
        let diagonal = corridor((0, 0), (300, 700));
        assert_eq!(
            (diagonal.ll.x, diagonal.ll.y, diagonal.ur.x, diagonal.ur.y),
            (-175, -175, 475, 875)
        );
    }

    /// Java `isRemovableBlocker` (`:69-84`) — the full predicate
    /// table: kind (pin never), own net, user-fixed (the fixed AND
    /// deletion-forbidden conjuncts — `isDeletionForbidden` includes
    /// `isUserFixed`), a missing id, and the netless guard.
    #[test]
    fn is_removable_blocker_table() {
        let (mut manager, mut board) = parse_fixture();
        let net_a = net_no(&board, "NET_33");
        let net_b = net_no(&board, "NET_98");
        // Kind arm: a pin is never a blocker whatever its nets.
        let pin = board
            .iter_ascending()
            .find(|entry| matches!(entry.data, ItemData::Pin { .. }))
            .expect("fixture has pins")
            .id;
        assert!(!is_removable_blocker(&board, pin, net_b), "a pin");
        // The removable cell: an unfixed foreign trace.
        let foreign = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&pt(900_000, 900_000), &pt(950_000, 900_000)),
            0,
            1500,
            &[net_b],
            1,
            FixedState::Unfixed,
        )
        .expect("foreign insert succeeds");
        assert!(
            is_removable_blocker(&board, foreign, net_a),
            "unfixed foreign trace"
        );
        assert!(
            !is_removable_blocker(&board, foreign, net_b),
            "own net is never a blocker"
        );
        // User-fixed: rejected by BOTH the is_user_fixed conjunct and
        // is_deletion_forbidden (which subsumes it) — one row pins the
        // observable face, the fixed state.
        let fixed = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&pt(900_000, 940_000), &pt(950_000, 940_000)),
            0,
            1500,
            &[net_b],
            1,
            FixedState::UserFixed,
        )
        .expect("fixed insert succeeds");
        assert!(
            !is_removable_blocker(&board, fixed, net_a),
            "user-fixed trace"
        );
        // A missing id is not a blocker (the guard arm).
        assert!(
            !is_removable_blocker(&board, ItemId::new(u32::MAX), net_a),
            "missing id"
        );
        // Netless guard: `net_no_arr.length > 0` (when the seam
        // accepts a netless insert).
        if let Some(netless) = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&pt(900_000, 980_000), &pt(950_000, 980_000)),
            0,
            1500,
            &[],
            1,
            FixedState::Unfixed,
        ) {
            assert!(
                !is_removable_blocker(&board, netless, net_a),
                "netless trace"
            );
        }
    }

    /// The end-to-end witness: on the bare pair-net fixture, an
    /// unfixed foreign trace rides the net-33 airline's midpoint
    /// (deep inside its corridor by construction) and gets RIPPED,
    /// while an own-net trace and a user-fixed trace — placed in the
    /// target corridor's EXCLUSIVE region — survive. Placement law:
    /// the fixture's two corridors OVERLAP along the whole airline
    /// (net 98's margin swallows net 33's line, probed 2026-10-03),
    /// so an unfixed own-net survivor ON the airline would be a
    /// LEGITIMATE blocker of net 98 — the survivors must sit where
    /// only the target corridor reaches them.
    #[test]
    fn rip_witness_foreign_ripped_fixed_and_own_survive() {
        let (mut manager, mut board) = parse_fixture();
        let net_a = net_no(&board, "NET_33");
        // A real single-pin net by NUMBER (its name is not "NET_49";
        // the number is the sibling board_history suite's pinned
        // fixture fact) — no airline of its own, foreign to every
        // airline on the board.
        let net_foreign: i32 = 49;
        let airlines = airline_segments(&manager, &mut board);
        let target = airlines
            .iter()
            .find(|airline| airline.net == net_a)
            .expect("the net-33 airline");
        let other_corridors: Vec<IntBox> = airlines
            .iter()
            .filter(|airline| airline.net != net_a)
            .map(|airline| corridor(airline.from, airline.to))
            .collect();
        let corridor_a = corridor(target.from, target.to);
        let disjoint = |a: &IntBox, b: &IntBox| {
            a.ll.x > b.ur.x || b.ll.x > a.ur.x || a.ll.y > b.ur.y || b.ll.y > a.ur.y
        };

        // The blocker: the airline midpoint.
        let mid_x = ((target.from.0 + target.to.0) / 2) as i32;
        let mid_y = ((target.from.1 + target.to.1) / 2) as i32;

        // The survivors: a fractional grid inside the TARGET corridor
        // whose ±10k/±500 segment boxes stay clear of every other
        // corridor.
        const HALF_SPAN: i32 = 10_000;
        let mut survivor = None;
        for f_x in [2, 35, 5, 65, 8] {
            for f_y in [2, 35, 5, 65, 8] {
                let x = corridor_a.ll.x + f_x * (corridor_a.ur.x - corridor_a.ll.x) / 10;
                let y = corridor_a.ll.y + f_y * (corridor_a.ur.y - corridor_a.ll.y) / 10;
                let segment_box =
                    IntBox::from_corners(x - HALF_SPAN, y - 500, x + HALF_SPAN, y + 500);
                if other_corridors
                    .iter()
                    .all(|other| disjoint(&segment_box, other))
                {
                    survivor = Some((x, y));
                    break;
                }
            }
            if survivor.is_some() {
                break;
            }
        }
        let Some((s_x, s_y)) = survivor else {
            panic!("no exclusive region in the net-33 corridor (fixture drift?)");
        };

        let insert = |manager: &mut SearchTreeManager,
                      board: &mut Board,
                      x: i32,
                      y: i32,
                      nets: &[i32],
                      fixed: FixedState| {
            insert_trace_without_cleaning(
                manager,
                board,
                Polyline::from_two_corners(&pt(x - HALF_SPAN, y), &pt(x + HALF_SPAN, y)),
                0,
                500,
                nets,
                1,
                fixed,
            )
            .expect("witness insert succeeds")
        };
        let foreign = insert(
            &mut manager,
            &mut board,
            mid_x,
            mid_y,
            &[net_foreign],
            FixedState::Unfixed,
        );
        let own = insert(
            &mut manager,
            &mut board,
            s_x,
            s_y,
            &[net_a],
            FixedState::Unfixed,
        );
        let fixed = insert(
            &mut manager,
            &mut board,
            s_x,
            s_y,
            &[net_foreign],
            FixedState::UserFixed,
        );

        let removed = rip_blockers(&mut manager, &mut board);
        assert_eq!(removed, 1, "exactly the unfixed foreign trace");
        assert!(
            board.get(foreign).is_none_or(|entry| !entry.on_the_board),
            "the foreign trace was ripped"
        );
        assert!(
            board.get(own).is_some_and(|entry| entry.on_the_board),
            "the own-net trace survives"
        );
        assert!(
            board.get(fixed).is_some_and(|entry| entry.on_the_board),
            "the user-fixed trace survives"
        );
    }
}
