//! The M6-T7 pin bank (charter: pins in `global/tests.rs`).
//!
//! Pin discipline (cerebrum DNR): every pin pairs a CONTRAST world
//! (a wrong implementation differs); exact boundaries carry the ±1
//! faces; the digests are capture literals verified by hand from the
//! craft's closed-form facts (cell 10015, capacity 1, grid 129x65).

use crate::test_util::parse;

/// The pattern world: NA = two front pads at (200000,320000) and
/// (1080000,480000) internal units — the diagonal 2-pin same-layer
/// pair (L/Z candidates distinct; see the fixture header).
const G1: &str = include_str!("../../../../harness/fixtures/global-spike/g1_pattern.dsn");

/// The congestion world: 4 nets — NA(1)/NB(2) same-layer pairs, NC(3)
/// a front/back pair (LayerPair), ND(4) a 3-terminal net.
const G2: &str = include_str!("../../../../harness/fixtures/global-spike/g2_congestion.dsn");

/// The overlap world: NA's (20000,32000) pad shares grid cells with
/// NB's (21000,33000) and NC's (23000,35000) pads — the only world
/// with nonzero overflow (25 cells, layer 0).
const G3: &str = include_str!("../../../../harness/fixtures/global-spike/g3_overlap.dsn");

/// G1's closed-form grid facts (probe-verified): boundary
/// 0..1280000 x 0..640000 internal (DSN x10), bbox width 1,281,920 ⇒
/// cell = 1281920/128 = 10015, grid 129x65, pitch = 2*1000+250 = 2250
/// ⇒ capacity 1 per cell per layer.
const CELL: i64 = 10_015;

#[test]
fn congestion_map_grid_facts() {
    let (_m, mut board) = parse(G1);
    let map = crate::global::map::CongestionMap::build(&mut board);
    assert_eq!(map.cell_size(), CELL);
    assert_eq!(map.grid_dims(), (129, 65));
    assert_eq!(map.capacity(0), 1);
    assert_eq!(map.capacity(1), 1);
    // The two pads alone demand one track per touched cell — no
    // overflow anywhere.
    assert_eq!(map.total_overflow(), &[0, 0]);
    // The occupancy digest (capture literal — see the probe history in
    // the task report).
    assert_eq!(
        map.occupancy_digest(),
        "de28810d09216d8f5cb7c63065c0fbe88328ca528a89c32b40a7a26f000f031d"
    );
}

/// DNR-16: the closed-form cell boundary. With the grid origin
/// (-1000,-1000) and cell 10015, column 20 spans [199300, 209315)
/// and row 32 spans [319480, 329495); ONE unit below each boundary
/// lands in the previous cell.
#[test]
fn congestion_map_cell_boundary_exact_plus_minus_one() {
    use epic_geometry::int_point::IntPoint;
    let (_m, mut board) = parse(G1);
    let map = crate::global::map::CongestionMap::build(&mut board);
    assert_eq!(map.grid_origin(), (-1000, -1000));
    assert_eq!(
        map.cell_of(IntPoint {
            x: 199_300,
            y: 321_000
        }),
        (20, 32)
    );
    assert_eq!(
        map.cell_of(IntPoint {
            x: 199_299,
            y: 321_000
        }),
        (19, 32)
    );
    assert_eq!(
        map.cell_of(IntPoint {
            x: 200_000,
            y: 319_480
        }),
        (20, 32)
    );
    assert_eq!(
        map.cell_of(IntPoint {
            x: 200_000,
            y: 319_479
        }),
        (20, 31)
    );
}

#[test]
fn congestion_map_overflow_boundary_occupancy_cap_vs_cap_plus_one() {
    let (_m, mut board) = parse(G3);
    let map = crate::global::map::CongestionMap::build(&mut board);
    // Cap = 1 (G1's closed-form facts hold on G3 — same frame).
    assert_eq!(map.capacity(0), 1);
    // Cell (19,31): KA(NA) + KC(NB) pads cover it, KE does not —
    // occupancy 2 = cap + 1 ⇒ overflow 1. Excluding NB(2) leaves
    // one occupant ⇒ 0.
    assert_eq!(map.occupancy(19, 31, 0, None), 2);
    assert_eq!(map.overflow(19, 31, 0, None), 1);
    assert_eq!(map.overflow(19, 31, 0, Some(2)), 0);
    // Cell (22,34) is the exact KA∩KC∩KE triple: occupancy 3 = cap + 2
    // ⇒ overflow 2; excluding ANY one net still leaves two ⇒ 1.
    assert_eq!(map.occupancy(22, 34, 0, None), 3);
    assert_eq!(map.overflow(22, 34, 0, None), 2);
    assert_eq!(map.overflow(22, 34, 0, Some(1)), 1);
    assert_eq!(map.overflow(22, 34, 0, Some(2)), 1);
    assert_eq!(map.overflow(22, 34, 0, Some(3)), 1);
    // The exact cap boundary: a cell with ONE distinct net = occupancy
    // cap ⇒ overflow 0 (KB's pad alone at (107,48)).
    assert_eq!(map.occupancy(107, 48, 0, None), 1);
    assert_eq!(map.overflow(107, 48, 0, None), 0);
    // The total vector: 25 over-capacity cells, all on layer 0.
    assert_eq!(map.total_overflow(), &[25, 0]);
    assert_eq!(
        map.occupancy_digest(),
        "6766b6511f5fdb3874b6a503010ad13f611944b2222389f3bc24f125e6dc21f4"
    );
}

#[test]
fn corridor_clear_faces() {
    let (_m, mut board) = parse(G3);
    let map = crate::global::map::CongestionMap::build(&mut board);
    let a = epic_geometry::int_point::IntPoint {
        x: 200_000,
        y: 320_000,
    };
    let b = epic_geometry::int_point::IntPoint {
        x: 1_080_000,
        y: 480_000,
    };
    // The vertical-at-a.x corridor is BLOCKED: NB/NC copper sits in the
    // pad-cluster cells at columns 19..22, rows 31..36 (occupancy = cap).
    assert!(!map.corridor_clear(
        a,
        epic_geometry::int_point::IntPoint { x: 200_000, y: b.y },
        0,
        Some(1)
    ));
    // The long horizontal at row 31 is BLOCKED too (the cluster sits on it).
    assert!(!map.corridor_clear(
        a,
        epic_geometry::int_point::IntPoint { x: b.x, y: a.y },
        0,
        Some(1)
    ));
    // A free corridor: the long horizontal at row 45 (above the pads).
    assert!(map.corridor_clear(
        epic_geometry::int_point::IntPoint {
            x: 200_000,
            y: 450_000
        },
        epic_geometry::int_point::IntPoint {
            x: 1_080_000,
            y: 450_000
        },
        0,
        Some(1)
    ));
}

#[test]
fn plan_order_severity_then_net_number_tiebreak() {
    let (_m, mut board) = parse(G3);
    let plan = crate::global::plan::GlobalPlan::build(&mut board);
    let na = 1;
    let nb = 2;
    let nc = 3;
    // NA's region contains the whole cluster (rows 31..36) — most
    // overflow severity; NB's region stops at row 34; NC (1 pin) is
    // unplanned (ranks LAST).
    assert_eq!(plan.net_order(), &[na, nb]);
    assert_eq!(plan.rank_of(na), Some(0));
    assert_eq!(plan.rank_of(nb), Some(1));
    assert_eq!(plan.rank_of(nc), None);
    assert_eq!(
        plan.order_digest(),
        "376b25cfe3d9cd79edec615069dc95a78a3c61d456fe8c63ad3a7ff7a30d7bd1"
    );
}

#[test]
fn plan_order_tie_goes_to_net_number() {
    // G2: zero overflow everywhere ⇒ full severity tie ⇒ ascending net
    // numbers 1..4.
    let (_m, mut board) = parse(G2);
    let plan = crate::global::plan::GlobalPlan::build(&mut board);
    assert_eq!(plan.net_order(), &[1, 2, 3, 4]);
    assert_eq!(
        plan.order_digest(),
        "abb62a38b6e7e7b88176277478f3e245cf7dbb78975269877b016304d41c7ede"
    );
}

#[test]
fn plan_topology_classes() {
    let (_m, mut board) = parse(G2);
    let plan = crate::global::plan::GlobalPlan::build(&mut board);
    let topos: Vec<_> = plan.guides().iter().map(|g| g.topology()).collect();
    assert_eq!(
        topos,
        vec![
            crate::global::plan::NetTopology::SameLayerPair,
            crate::global::plan::NetTopology::SameLayerPair,
            crate::global::plan::NetTopology::LayerPair,
            crate::global::plan::NetTopology::MultiTerminal,
        ]
    );
}

/// The pattern candidate order: on the clear G1 map, L1 (bend at
/// (a.x, b.y)) wins; the exact corner vector is the pin.
#[test]
fn pattern_candidate_order_l1_wins_on_clear_map() {
    use epic_geometry::int_point::IntPoint;
    let (_m, mut board) = parse(G1);
    let map = crate::global::map::CongestionMap::build(&mut board);
    let a = IntPoint {
        x: 200_000,
        y: 320_000,
    };
    let b = IntPoint {
        x: 1_080_000,
        y: 480_000,
    };
    let corners = crate::global::pattern::candidate_corners(&map, 0, 1, a, b)
        .expect("clear map: L1 must clear");
    assert_eq!(
        corners,
        vec![
            IntPoint {
                x: 200_000,
                y: 320_000
            },
            IntPoint {
                x: 200_000,
                y: 480_000
            },
            IntPoint {
                x: 1_080_000,
                y: 480_000
            },
        ]
    );
}

/// The blocking face: the same a/b pair on the G3 map — NB/NC copper
/// saturates the pad-cluster cells on EVERY candidate corridor
/// (both Ls and both Zs) — no candidate clears.
#[test]
fn pattern_candidates_blocked_by_foreign_occupancy() {
    use epic_geometry::int_point::IntPoint;
    let (_m, mut board) = parse(G3);
    let map = crate::global::map::CongestionMap::build(&mut board);
    let a = IntPoint {
        x: 200_000,
        y: 320_000,
    };
    let b = IntPoint {
        x: 1_080_000,
        y: 480_000,
    };
    assert!(
        crate::global::pattern::candidate_corners(&map, 0, 1, a, b).is_none(),
        "saturated cluster cells block every L/Z corridor"
    );
}

/// The straight degenerate: G2's NB pair shares row y=160000 — the
/// L/Z fold collapses to the 2-point straight face.
#[test]
fn pattern_candidate_straight_when_aligned() {
    use epic_geometry::int_point::IntPoint;
    let (_m, mut board) = parse(G2);
    let map = crate::global::map::CongestionMap::build(&mut board);
    let net2 = 2;
    let corners = crate::global::pattern::candidate_corners(
        &map,
        0,
        net2,
        IntPoint {
            x: 400_000,
            y: 160_000,
        },
        IntPoint {
            x: 880_000,
            y: 160_000,
        },
    )
    .expect("clear map: straight must clear");
    assert_eq!(
        corners,
        vec![
            IntPoint {
                x: 400_000,
                y: 160_000
            },
            IntPoint {
                x: 880_000,
                y: 160_000
            },
        ]
    );
}

/// The plan is a pure function of board state: two builds over fresh
/// parses carry identical digests (the T6 parse-determinism
/// discipline).
#[test]
fn plan_is_parse_deterministic() {
    let (_m, mut board_a) = parse(G3);
    let plan_a = crate::global::plan::GlobalPlan::build(&mut board_a);
    let (_m, mut board_b) = parse(G3);
    let plan_b = crate::global::plan::GlobalPlan::build(&mut board_b);
    assert_eq!(
        plan_a.map().occupancy_digest(),
        plan_b.map().occupancy_digest()
    );
    assert_eq!(plan_a.order_digest(), plan_b.order_digest());
}

/// End to end: with the master + pattern flags ON, routing NA's pair
/// goes through the pattern fast path — the board carries ONE new
/// trace whose bend sits at L1's corner (a.x, b.y). Mutating the
/// candidate order (L1<->L2) flips the bend coordinate and kills this
/// pin (see the mutation log).
#[test]
fn pattern_route_end_to_end_routed_at_l1() {
    use crate::pipeline::connection_router;
    use epic_board::items::ItemData;
    use std::collections::{BTreeMap, HashMap};

    let (mut manager, mut board) = parse(G1);
    let mut settings = crate::pipeline::batch::BatchSettings::new(
        crate::control::RouterSettingsIr {
            trace_costs: vec![
                crate::control::ExpansionCostFactor {
                    horizontal: 1.0,
                    vertical: 2.7,
                },
                crate::control::ExpansionCostFactor {
                    horizontal: 1.6,
                    vertical: 1.0,
                },
            ],
            via_costs: 1,
            vias_allowed: true,
            bend_costs: vec![0.0, 0.0],
            layer_active: vec![true, true],
            automatic_neckdown: false,
            start_ripup_costs: 1,
            fanout: Default::default(),
        },
        crate::pipeline::board_statistics::RouterSettingsScoring::default(),
    );
    settings.congestion_global = true;
    settings.congestion_global_pattern = true;

    let net = 1;
    let items = board.get_connectable_items(net);
    assert_eq!(items.len(), 2, "the craft's NA pair");
    let item_id = items[0];

    let mut ripped = BTreeMap::new();
    let mut ripup_costs = HashMap::new();
    let mut sink = crate::pipeline::event_sink::CaptureDriverSink::default();
    let result = connection_router::route(
        &mut manager,
        &mut board,
        &settings,
        item_id,
        net,
        &mut ripped,
        &mut ripup_costs,
        1,
        None,
        &mut sink,
    );
    assert!(
        matches!(result.state, crate::engine::AutorouteAttemptState::Routed),
        "the pattern route must route: {:?}",
        result.details
    );

    // The seam face: the fast path's observability row is present (a
    // maze-fallback mutant — try_pattern_route hardwired to None —
    // never emits it and this pin dies).
    assert!(
        sink.rows
            .iter()
            .any(|(_level, row)| row.starts_with("global_pattern_route net=1")),
        "the pattern fast path must emit its seam row"
    );

    // The board face: NA is connected — NA copper now joins the two
    // pad centers (the endpoints multiset contains BOTH centers; the
    // pull-tight pass may reshape the inserted L, so the pin holds the
    // connectivity, not the corner geometry).
    let mut endpoints: Vec<(i32, i32)> = Vec::new();
    for entry in board.iter_descending() {
        if let ItemData::Trace { lines, .. } = &entry.data
            && entry.nets.contains(&net)
        {
            for corner in [lines.first_corner(), lines.last_corner()] {
                if let Some(epic_geometry::point::Point::Int(p)) = corner {
                    endpoints.push((p.x, p.y));
                }
            }
        }
    }
    endpoints.sort();
    assert!(
        endpoints.contains(&(200_000, 320_000)),
        "KA endpoint present: {endpoints:?}"
    );
    assert!(
        endpoints.contains(&(1_080_000, 480_000)),
        "KB endpoint present: {endpoints:?}"
    );
    assert!(!endpoints.is_empty(), "NA copper exists");
}
