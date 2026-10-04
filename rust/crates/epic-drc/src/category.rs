//! Upstream #930 (`be56b5a0f`, `ClearanceViolation.java`): the
//! categorization of clearance violations by participant fixability
//! and item kinds — `isUnfixable` (NEITHER item routable, so ripping
//! up and rerouting can never resolve the pair) plus the five-way
//! `getCategory`. REPORTING ONLY: no route decision, score, or
//! golden face reads anything here; the consumers are the load-time
//! warning text and the in-memory `BoardStatistics` count.
//!
//! Port fidelity notes (both bit-relevant to the category):
//!
//! * Java's `isOutlineOrKeepout` is `instanceof BoardOutline ||
//!   ComponentOutline || ObstacleArea` — and Java's
//!   `ConductionArea` EXTENDS `ObstacleArea`, so an instanceof match
//!   sees conduction areas too. The Rust `ItemData` flattens the
//!   hierarchy, so the port spells the subclass out:
//!   `ConductionArea` is an outline-or-keepout side here.
//! * `FIXED_ROUTE` fires only for a Trace/Via inside an ALREADY
//!   unfixable pair — a routable trace is caught by
//!   `POTENTIALLY_FIXABLE` first (the category ladder checks
//!   fixability before kind).
//! * A dead id (lookup miss) is not routable and kinds as `Other` —
//!   the Rust mirror of Java's `firstItem == null` arm
//!   (`trace_ops::is_routable` already returns false for it).

use epic_board::board::Board;
use epic_board::id::ItemId;
use epic_board::items::ItemData;
use epic_board::trace_ops::is_routable;

use crate::clearance::DepthRow;

/// Java `ClearanceViolation.Category` — the declaration order is the
/// category ladder's fall-through order (most specific first).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViolationCategory {
    /// Two pins (both unfixable by construction — pins never routable).
    PinToPin,
    /// A pin against an outline/keepout/conduction area.
    PinToOutlineOrKeepout,
    /// A trace or via inside an unfixable pair (fixed routing).
    FixedRoute,
    /// Unfixable, none of the above kinds.
    OtherUnfixable,
    /// At least one item routable — the router may resolve it.
    PotentiallyFixable,
}

/// One violation side, reduced to what the category ladder asks
/// (Java's instanceof chain). `Other` is the null/dead-id arm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViolationSide {
    Pin,
    OutlineOrKeepout,
    Route,
    Other,
}

/// The per-category bucket counts over a walk's rows (the load-time
/// warning and the `BoardStatistics` unfixable count consume these).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CategoryCounts {
    pub pin_to_pin: usize,
    pub pin_to_outline_or_keepout: usize,
    pub fixed_route: usize,
    pub other_unfixable: usize,
    pub potentially_fixable: usize,
}

impl CategoryCounts {
    /// The unfixable total (Java's `totalUnfixable` — every bucket
    /// except `POTENTIALLY_FIXABLE`).
    #[must_use]
    pub fn total_unfixable(&self) -> usize {
        self.pin_to_pin + self.pin_to_outline_or_keepout + self.fixed_route + self.other_unfixable
    }

    /// The grand total (every bucket).
    #[must_use]
    pub fn total(&self) -> usize {
        self.total_unfixable() + self.potentially_fixable
    }
}

/// Java `ClearanceViolation.isUnfixable` over one side's raw id:
/// `(item == null || !item.isRoutable())` — a dead id is not
/// routable (`trace_ops::is_routable` returns false on lookup miss).
#[must_use]
pub fn side_is_routable(board: &Board, id: ItemId) -> bool {
    is_routable(board, id)
}

/// Reduces one item to its [`ViolationSide`] (the instanceof chain;
/// see the module docs for the `ConductionArea` subclass note).
#[must_use]
pub fn side_of(data: Option<&ItemData>) -> ViolationSide {
    match data {
        Some(ItemData::Pin { .. }) => ViolationSide::Pin,
        Some(
            ItemData::BoardOutline { .. }
            | ItemData::ComponentOutline { .. }
            | ItemData::ObstacleArea { .. }
            | ItemData::ConductionArea { .. },
        ) => ViolationSide::OutlineOrKeepout,
        Some(ItemData::Trace { .. } | ItemData::Via { .. }) => ViolationSide::Route,
        _ => ViolationSide::Other,
    }
}

/// Java `ClearanceViolation.getCategory` — the pure ladder, driven by
/// one `(side, routable)` pair per participant. This is the
/// board-free core so the truth table pins it without a board.
#[must_use]
pub fn category_from_parts(
    first: (ViolationSide, bool),
    second: (ViolationSide, bool),
) -> ViolationCategory {
    // Java checks fixability FIRST: any routable participant makes
    // the pair potentially fixable, whatever the kinds.
    if first.1 || second.1 {
        return ViolationCategory::PotentiallyFixable;
    }
    match (first.0, second.0) {
        (ViolationSide::Pin, ViolationSide::Pin) => ViolationCategory::PinToPin,
        (ViolationSide::Pin, ViolationSide::OutlineOrKeepout)
        | (ViolationSide::OutlineOrKeepout, ViolationSide::Pin) => {
            ViolationCategory::PinToOutlineOrKeepout
        }
        _ => {
            if matches!(first.0, ViolationSide::Route) || matches!(second.0, ViolationSide::Route) {
                ViolationCategory::FixedRoute
            } else {
                ViolationCategory::OtherUnfixable
            }
        }
    }
}

/// The board-facing wrapper: looks both ids up and classifies.
/// A dead id kinds as `Other` and is not routable (the null arm).
#[must_use]
pub fn violation_category(board: &Board, first: ItemId, second: ItemId) -> ViolationCategory {
    let part = |id: ItemId| {
        let data = board.get(id).map(|entry| &entry.data);
        (side_of(data), side_is_routable(board, id))
    };
    category_from_parts(part(first), part(second))
}

/// The raw→[`ItemId`] clamp shared by every `DepthRow` consumer (the
/// counts walk and the warning formatter): an out-of-range raw id
/// cannot occur in a fresh walk's rows; if one ever did, it parks on
/// id 1 (the ever-present BoardOutline — never routable, never
/// pin/route) rather than panicking on a reporting path.
#[must_use]
pub fn depth_row_ids(row: &DepthRow) -> (ItemId, ItemId) {
    let id_of = |raw: i64| u32::try_from(raw.max(1)).map_or(ItemId::new(1), ItemId::new);
    (id_of(row.a), id_of(row.b))
}

/// Buckets a walk's deduped depth rows (the rows
/// `all_clearance_violation_depths` returns; `a`/`b` are the item
/// ids).
#[must_use]
pub fn categorize_depth_rows(board: &Board, rows: &[DepthRow]) -> CategoryCounts {
    let mut counts = CategoryCounts::default();
    for row in rows {
        let (first, second) = depth_row_ids(row);
        let category = violation_category(board, first, second);
        match category {
            ViolationCategory::PinToPin => counts.pin_to_pin += 1,
            ViolationCategory::PinToOutlineOrKeepout => {
                counts.pin_to_outline_or_keepout += 1;
            }
            ViolationCategory::FixedRoute => counts.fixed_route += 1,
            ViolationCategory::OtherUnfixable => counts.other_unfixable += 1,
            ViolationCategory::PotentiallyFixable => counts.potentially_fixable += 1,
        }
    }
    counts
}

// ---------------------------------------------------------------------------
// pins
// ---------------------------------------------------------------------------

#[cfg(test)]
mod pins {
    use epic_board::items::BoardItemType;
    use epic_board::items::ItemData;
    use epic_board::items::ObstacleKind;
    use epic_board::items::{Area, BoardShape};
    use epic_geometry::int_box::IntBox;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::point::Point;
    use epic_geometry::polyline::Polyline;
    use epic_geometry::regular_tile_shape::RegularTileShape;
    use epic_geometry::tile_shape::TileShape;

    use super::{
        CategoryCounts, ViolationCategory, ViolationSide, categorize_depth_rows,
        category_from_parts, depth_row_ids, side_of, violation_category,
    };
    use crate::clearance::all_clearance_violation_depths;
    use crate::test_util::{DSN_930, DSN_MAIN, DSN_P4, ids_of_kind, parse};

    /// Java `getCategory`'s ladder, board-free. The ORDERING TRAP is
    /// the headline: fixability is checked BEFORE kind, so a ROUTABLE
    /// trace against a pin is POTENTIALLY_FIXABLE — FIXED_ROUTE fires
    /// only for a Trace/Via inside an ALREADY-unfixable pair.
    #[test]
    fn ladder_checks_fixability_before_kind() {
        let pin = (ViolationSide::Pin, false);
        let outline = (ViolationSide::OutlineOrKeepout, false);
        let route = (ViolationSide::Route, false);
        let other = (ViolationSide::Other, false);

        // Any routable participant wins, whatever the kinds.
        assert_eq!(
            category_from_parts((ViolationSide::Route, true), pin),
            ViolationCategory::PotentiallyFixable,
            "a routable trace against a pin is fixable, NOT FixedRoute/PinToPin"
        );
        assert_eq!(
            category_from_parts(pin, (ViolationSide::Route, true)),
            ViolationCategory::PotentiallyFixable,
            "order-independent"
        );
        assert_eq!(
            category_from_parts((ViolationSide::Route, true), (ViolationSide::Route, true)),
            ViolationCategory::PotentiallyFixable
        );

        // The kind ladder over unfixable pairs, most specific first.
        assert_eq!(category_from_parts(pin, pin), ViolationCategory::PinToPin);
        assert_eq!(
            category_from_parts(pin, outline),
            ViolationCategory::PinToOutlineOrKeepout
        );
        assert_eq!(
            category_from_parts(outline, pin),
            ViolationCategory::PinToOutlineOrKeepout,
            "either order"
        );
        assert_eq!(
            category_from_parts(route, outline),
            ViolationCategory::FixedRoute
        );
        assert_eq!(
            category_from_parts(outline, route),
            ViolationCategory::FixedRoute
        );
        assert_eq!(
            category_from_parts(route, route),
            ViolationCategory::FixedRoute
        );
        assert_eq!(
            category_from_parts(route, other),
            ViolationCategory::FixedRoute,
            "a trace against a dead id still kinds as a route side"
        );
        // No route side anywhere: outline x outline, dead-id arms.
        assert_eq!(
            category_from_parts(outline, outline),
            ViolationCategory::OtherUnfixable
        );
        assert_eq!(
            category_from_parts(other, other),
            ViolationCategory::OtherUnfixable
        );
        assert_eq!(
            category_from_parts(other, outline),
            ViolationCategory::OtherUnfixable
        );
        assert_eq!(
            category_from_parts(pin, other),
            ViolationCategory::OtherUnfixable,
            "pin x dead id: not pin-pin, not pin-outline, no route side"
        );
    }

    /// The instanceof chain (Java `isOutlineOrKeepout` etc.): the
    /// ConductionArea cell is the SUBCLASS trap — Java's
    /// `instanceof ObstacleArea` sees it, so it kinds as an
    /// outline-or-keepout side, never a route side.
    #[test]
    fn side_of_matches_the_java_instanceof_chain() {
        let box_area = Area::simple(BoardShape::Tile(TileShape::RegularTileShape(
            RegularTileShape::IntBox(IntBox::new(IntPoint::new(0, 0), IntPoint::new(10, 10))),
        )));
        let point = |x: i32, y: i32| Point::Int(IntPoint::new(x, y));

        assert_eq!(
            side_of(Some(&ItemData::Pin {
                pin_index: 1,
                padstack_no: 1
            })),
            ViolationSide::Pin
        );
        assert_eq!(
            side_of(Some(&ItemData::BoardOutline {
                shapes: Vec::new(),
                keepout_outside_outline: false
            })),
            ViolationSide::OutlineOrKeepout
        );
        assert_eq!(
            side_of(Some(&ItemData::ComponentOutline {
                layer: 0,
                area: box_area.clone(),
                translation: IntPoint::new(0, 0),
                rotation: 0.0,
                is_front: true,
                is_courtyard: false,
                is_fabrication: false,
                is_closed: true,
            })),
            ViolationSide::OutlineOrKeepout
        );
        assert_eq!(
            side_of(Some(&ItemData::ObstacleArea {
                kind: ObstacleKind::ObstacleArea,
                layer: 0,
                area: box_area.clone(),
                translation: IntPoint::new(0, 0),
                rotation: 0.0,
                side_changed: false,
                name: None,
            })),
            ViolationSide::OutlineOrKeepout
        );
        assert_eq!(
            side_of(Some(&ItemData::ConductionArea {
                layer: 0,
                area: box_area,
                is_obstacle: false,
                is_filled: true,
            })),
            ViolationSide::OutlineOrKeepout,
            "ConductionArea EXTENDS ObstacleArea — the subclass kinds as keepout"
        );
        assert_eq!(
            side_of(Some(&ItemData::Trace {
                layer: 0,
                half_width: 125,
                lines: Polyline::from_points(&[point(0, 0), point(1000, 0)]),
            })),
            ViolationSide::Route
        );
        assert_eq!(
            side_of(Some(&ItemData::Via {
                center: IntPoint::new(0, 0),
                padstack_no: 1,
                attach_smd_allowed: true,
            })),
            ViolationSide::Route
        );
        assert_eq!(
            side_of(Some(&ItemData::Other)),
            ViolationSide::Other,
            "the unreachable kind kinds as Other"
        );
        assert_eq!(
            side_of(None),
            ViolationSide::Other,
            "the dead-id arm mirrors Java's firstItem == null"
        );
    }

    /// `totalUnfixable` = every bucket except POTENTIALLY_FIXABLE;
    /// `total` = all five.
    #[test]
    fn counts_totals_are_the_java_unfixable_split() {
        let counts = CategoryCounts {
            pin_to_pin: 2,
            pin_to_outline_or_keepout: 3,
            fixed_route: 4,
            other_unfixable: 5,
            potentially_fixable: 6,
        };
        assert_eq!(counts.total_unfixable(), 14);
        assert_eq!(counts.total(), 20);
        assert_eq!(CategoryCounts::default().total(), 0);
    }

    /// The #930 witness board end-to-end: the depth walk's eight rows
    /// bucket 6 pin-to-pin (the alternating-net adjacent pairs; the
    /// P4 exemption lattice leaves cross-net same-component pairs
    /// obstacles), 1 fixed-route (the `(type fix)` pair — SystemFixed,
    /// `is_user_fixed`'s `>=` sees it), 1 potentially-fixable (the
    /// plain pair — parse-time traces are routable). Every row buckets
    /// exactly once (counts.total() == rows.len()).
    #[test]
    fn categorize_depth_rows_buckets_the_930_witness_board() {
        let (mut manager, mut board) = parse(DSN_930);
        let (total, rows) = all_clearance_violation_depths(&mut manager, &mut board);
        assert_eq!(total, 8, "six pin pairs + two trace pairs");
        let counts = categorize_depth_rows(&board, &rows);
        assert_eq!(counts.total(), rows.len(), "every row buckets exactly once");
        assert_eq!(
            (
                counts.pin_to_pin,
                counts.pin_to_outline_or_keepout,
                counts.fixed_route,
                counts.other_unfixable,
                counts.potentially_fixable
            ),
            (6, 0, 1, 0, 1)
        );
        assert_eq!(counts.total_unfixable(), 7);
    }

    /// Integration on the pre-existing crafts: parse-time traces are
    /// UNFIXED and netted → routable → every drc-main walk row is
    /// potentially fixable (the outline x P10 row is long gone — P2's
    /// pin-gap cap collapsed it, P3's 1 µm tolerance drops its
    /// zero-shortfall); the P4 craft's three surviving rows are all
    /// pin-to-pin (pins are never routable).
    #[test]
    fn drc_main_rows_are_potentially_fixable_and_p4_rows_are_pin_to_pin() {
        let (mut manager, mut board) = parse(DSN_MAIN);
        let (_, rows) = all_clearance_violation_depths(&mut manager, &mut board);
        let counts = categorize_depth_rows(&board, &rows);
        // The raw-parse walk carries exactly ONE unfixable row — the
        // BoardOutline against the NP net's edge-STRADDLING pin
        // (P10), PinToOutlineOrKeepout. FACE NOTE: the Session-path
        // walk (what the overlay-marker golden pins) DROPS this row —
        // P2's copper-to-edge override rewrites the outline rule cell
        // for class 0 to 0 on the Session path only, so the shortfall
        // collapses to 0.0 and P3's strict gate drops it there; the
        // RAW parse walk (no override) measures the full 2000 rule
        // against the straddler and records it. Both faces are
        // correct on their own path.
        assert_eq!(
            counts.total_unfixable(),
            1,
            "the outline x straddler pin is the one unfixable drc-main row"
        );
        assert_eq!(counts.pin_to_outline_or_keepout, 1);
        let outline = ids_of_kind(&board, BoardItemType::BoardOutline)[0];
        for row in &rows {
            let (first, second) = depth_row_ids(row);
            let category = violation_category(&board, first, second);
            if category == ViolationCategory::PinToOutlineOrKeepout {
                assert!(
                    first == outline || second == outline,
                    "the unfixable row pairs the outline with the straddling pin"
                );
            } else {
                assert_eq!(
                    category,
                    ViolationCategory::PotentiallyFixable,
                    "every other drc-main row involves a routable (unfixed netted) trace"
                );
            }
        }

        let (mut manager, mut board) = parse(DSN_P4);
        let (_, rows) = all_clearance_violation_depths(&mut manager, &mut board);
        let counts = categorize_depth_rows(&board, &rows);
        // The CURRENT P4 walk law (clearance.rs:1045, post-#931
        // cluster G): TWO rows — the CMPD pair (netless, different
        // bases) and the CMPE pair (netted vs netless, same base);
        // the NS2 pair is exempt since the same-net guard dropped the
        // component scope. Both survivors are pin pairs — pins are
        // never routable — so the whole craft buckets pin-to-pin.
        assert_eq!(rows.len(), 2, "the P4 craft's two-row law (post-#931)");
        assert_eq!(
            (
                counts.pin_to_pin,
                counts.fixed_route,
                counts.potentially_fixable
            ),
            (2, 0, 0)
        );
    }
}
