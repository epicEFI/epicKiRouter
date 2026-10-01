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
/// goes through the pattern fast path. The pin holds the SEAM ROW
/// (the `global_pattern_route` observability row) plus CONNECTIVITY —
/// NOT the bend geometry: the inserted L is post-pull-tight, so the
/// 1-2-bend shape need not survive (the guides-contract note). The
/// mutant that kills this pin is the MAZE-FALLBACK face
/// (`try_pattern_route` hardwired to None — it never emits the seam
/// row); the candidate-ORDER mutant (L1<->L2) is killed by the
/// `pattern_candidate_order_l1_wins_on_clear_map` pin instead.
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
        None,
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

// ---------------------------------------------------------------------------
// M6-T8 pins — the PathFinder scheduler, the fanout congestion order,
// the region-level clamp, the digest literal (E-11), and the R-M3
// mixed-layer face.
// ---------------------------------------------------------------------------

/// The negotiated-base formula's DNR-16 boundary pins (the closed-form
/// boundary is `pressure == PRESSURE_SCALE * k` per `+start` of base,
/// and the RELATIVE cap `2 * start` — the fanout-protection-preserving
/// ceiling): the exact cap edge and one step either side, the
/// per-start scaling, and the RIPUP_CAP ceiling arm.
#[test]
fn negotiated_base_formula_boundary_pins() {
    use crate::global::history::negotiated_base;
    // start = 100 (the CLI default): the full band resolves.
    assert_eq!(negotiated_base(0, 100), 100, "no pressure = the bare start");
    assert_eq!(negotiated_base(49, 100), 149, "one below the +50% edge");
    assert_eq!(negotiated_base(50, 100), 150, "the exact +50% edge");
    assert_eq!(negotiated_base(51, 100), 151, "one above the +50% edge");
    assert_eq!(negotiated_base(99, 100), 199, "one below the cap edge");
    assert_eq!(negotiated_base(100, 100), 200, "the exact cap edge");
    assert_eq!(negotiated_base(101, 100), 200, "one above the cap edge");
    assert_eq!(negotiated_base(i64::MAX, 100), 200, "cap saturation");
    // start = 1: the band is [1, 2] in unit steps.
    assert_eq!(negotiated_base(0, 1), 1);
    assert_eq!(negotiated_base(99, 1), 1, "one below the flip");
    assert_eq!(negotiated_base(100, 1), 2, "the exact flip at the cap");
    assert_eq!(negotiated_base(i64::MAX, 1), 2, "cap saturation");
    // start = 3: integer-division steps of the start.
    assert_eq!(negotiated_base(0, 3), 3);
    assert_eq!(negotiated_base(50, 3), 4, "3 + 150/100 = 4");
    assert_eq!(negotiated_base(100, 3), 6, "the exact cap edge");
    assert_eq!(negotiated_base(0, 0), 0, "start 0 = the linear 0 face");
    assert_eq!(
        negotiated_base(i64::MAX, i64::MAX),
        i32::MAX / 100,
        "the RIPUP_CAP absolute ceiling (overflow arm)"
    );
}

/// The history update's decay + increment boundaries on the G3 map
/// (the overflow cells are closed-form: (19,31) overflows 1, (22,34)
/// overflows 2 — the T7 pins). Decay: 4*3/4 = 3 EXACT, 3*3/4 = 2
/// (truncation), 1*3/4 = 0 (sub-threshold forgetting), and the
/// HISTORY_MAX clamp decays 786432.
#[test]
fn history_decay_increment_boundary_pins() {
    use crate::global::history::{HISTORY_DECAY_DEN, HISTORY_DECAY_NUM, HISTORY_MAX, HistoryCosts};
    assert_eq!(HISTORY_DECAY_NUM, 3);
    assert_eq!(HISTORY_DECAY_DEN, 4);
    let (_m, mut board) = parse(G3);
    let map = crate::global::map::CongestionMap::build(&mut board);

    // The decay ladder at a NON-overflow cell (0,0,0).
    let mut h_exact = HistoryCosts::default();
    h_exact.entries.insert((0, 0, 0), 4);
    h_exact.update(&map);
    assert_eq!(h_exact.entry(0, 0, 0), Some(3), "4*3/4 = 3 exact");

    let mut h_trunc = HistoryCosts::default();
    h_trunc.entries.insert((0, 0, 0), 3);
    h_trunc.update(&map);
    assert_eq!(h_trunc.entry(0, 0, 0), Some(2), "3*3/4 truncates to 2");

    let mut h_forget = HistoryCosts::default();
    h_forget.entries.insert((0, 0, 0), 1);
    h_forget.update(&map);
    assert_eq!(h_forget.entry(0, 0, 0), None, "1*3/4 = 0: forgotten");

    let mut h_cap = HistoryCosts::default();
    h_cap.entries.insert((0, 0, 0), HISTORY_MAX);
    h_cap.update(&map);
    assert_eq!(
        h_cap.entry(0, 0, 0),
        Some(HISTORY_MAX * 3 / 4),
        "the cap value itself decays"
    );
    // LITERAL faces (the E-2 mutation round caught this pin deriving
    // its expectation from the const — a HISTORY_MAX + 1 mutant
    // survived until the literals landed): the tuning value is
    // exactly 2^20 and its decay exactly 786432.
    assert_eq!(HISTORY_MAX, 1 << 20, "the tuning value (E-2)");
    assert_eq!(
        h_cap.entry(0, 0, 0),
        Some(786_432),
        "2^20 * 3 / 4 truncates to 786432"
    );

    // The increments: the closed-form overflow cells.
    let mut h_inc = HistoryCosts::default();
    h_inc.update(&map);
    assert_eq!(h_inc.entry(0, 19, 31), Some(1), "overflow 1 adds 1");
    assert_eq!(h_inc.entry(0, 22, 34), Some(2), "overflow 2 adds 2");
    assert_eq!(h_inc.entry(1, 19, 31), None, "layer 1 carries no overflow");
    assert_eq!(
        h_inc.len(),
        21,
        "21 DISTINCT over-capacity cells (the T7 25 is the overflow-UNIT sum)"
    );
}

/// The guides-consumption face (the T7 preferred-layers residual):
/// the pass state carries each net's guide preference order in
/// PHYSICAL indices (G3: layer 0 carries all 25 overflow units, so the
/// preference is [1, 0]), and the maze's per-layer cost rows take the
/// `1 + rank * 0.5` bias (exact f64 literals). Mutant:
/// GUIDE_LAYER_RANK_STEP = 0 restores the identity and dies here.
#[test]
fn guides_preferred_layers_computed_and_consumed() {
    use crate::global::history::PathFinder;
    use crate::global::history::apply_preferred_layer_bias;

    let (_m, mut board) = parse(G3);
    let mut scheduler = PathFinder::new();
    let pass = scheduler.begin_pass(&mut board, 100);
    assert_eq!(
        pass.preferred_layers.get(&1),
        Some(&vec![1, 0]),
        "NA's guide preference: ordinal 1 (clear) before ordinal 0 (25 overflow units)"
    );

    // The bias on the standard 2-layer cost table: physical 0 sits at
    // preference rank 1 (x1.5), physical 1 at rank 0 (x1.0).
    let mut costs = vec![
        crate::control::ExpansionCostFactor {
            horizontal: 1.0,
            vertical: 2.7,
        },
        crate::control::ExpansionCostFactor {
            horizontal: 1.6,
            vertical: 1.0,
        },
    ];
    apply_preferred_layer_bias(&mut costs, &[1, 0]);
    assert_eq!(costs[0].horizontal, 1.5, "rank 1 -> x1.5");
    assert_eq!(
        costs[0].vertical,
        2.7 * 1.5,
        "2.7 * 1.5 (exact f64 product)"
    );
    assert_eq!(costs[1].horizontal, 1.6, "rank 0 -> x1.0");
    assert_eq!(costs[1].vertical, 1.0);

    // The exact step boundary: rank 2 -> x2.0 (1 + 2*0.5).
    let mut costs3 = vec![
        crate::control::ExpansionCostFactor {
            horizontal: 1.0,
            vertical: 1.0,
        },
        crate::control::ExpansionCostFactor {
            horizontal: 1.0,
            vertical: 1.0,
        },
        crate::control::ExpansionCostFactor {
            horizontal: 1.0,
            vertical: 1.0,
        },
    ];
    apply_preferred_layer_bias(&mut costs3, &[2, 1, 0]);
    assert_eq!(costs3[0].horizontal, 2.0, "rank 2 -> x2.0");
    assert_eq!(costs3[1].horizontal, 1.5, "rank 1 -> x1.5");
    assert_eq!(costs3[2].horizontal, 1.0, "rank 0 -> x1.0");
}

/// The scheduler across a pass boundary on G3 (start = 100): pass 1
/// runs on present congestion only; end_pass folds the same congestion
/// into the history and pass 2's bases mix present + history 1:1 (150
/// = 100 + 50). Mutants: HISTORY_MIX = 0 (history ignored) keeps pass
/// 2 at pass 1's value; DECAY 4/4, INCREMENT 2, and PRESSURE_SCALE
/// drift all rotate the literals.
#[test]
fn pathfinder_bases_present_then_history_mixed() {
    use crate::global::history::PathFinder;
    let (_m, mut board) = parse(G3);
    let mut scheduler = PathFinder::new();
    let pass1 = scheduler.begin_pass(&mut board, 100);
    assert_eq!(
        pass1.bases.get(&1),
        Some(&125),
        "NA: pressure 25 (all cluster units) -> 100 + 25*100/100 = 125"
    );
    assert_eq!(
        pass1.bases.get(&2),
        Some(&118),
        "NB: pressure 18 (its region stops at row 34) -> 100 + 18 = 118"
    );
    assert_eq!(pass1.bases.len(), 2, "NC routes last (no guide)");

    scheduler.end_pass(&mut board);
    let pass2 = scheduler.begin_pass(&mut board, 100);
    assert_eq!(
        pass2.bases.get(&1),
        Some(&150),
        "NA: 25 present + 25 history = 50 -> 100 + 50 = 150 (the 1:1 mix)"
    );
    assert_eq!(
        pass2.bases.get(&2),
        Some(&136),
        "NB: 18 present + 18 history = 36 -> 100 + 36 = 136"
    );
}

/// Design :69 rides the negotiated stage's flag: with
/// `congestion_global` + `congestion_global_pathfinder` ON the map
/// feeds fanout's pin order — the QFP component's pins sort by the
/// nets' planned ranks (NA's jammed guide first: the adjacent-QFP
/// mutual-blocking fix), and the Java outer-first order holds at
/// defaults and at master-only (the committed T7 golden's face).
/// Mutant: dropping the congestion key restores the Java order and
/// kills the ON arm.
#[test]
fn fanout_pin_order_congestion_primary_key() {
    use crate::pipeline::batch::BatchSettings;
    use crate::pipeline::board_statistics::RouterSettingsScoring;
    use crate::pipeline::fanout::FanoutState;

    const G4: &str = include_str!("../../../../harness/fixtures/global-spike/g4_fanout.dsn");

    let settings = |master: bool, pathfinder: bool| {
        let mut settings = BatchSettings::new(
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
            RouterSettingsScoring::default(),
        );
        settings.congestion_global = master;
        settings.congestion_global_pathfinder = pathfinder;
        settings
    };
    let pin_names = |master: bool, pathfinder: bool| -> Vec<String> {
        let (mut manager, mut board) = parse(G4);
        let state = FanoutState::new(&mut manager, &mut board, &settings(master, pathfinder));
        assert_eq!(state.components.len(), 3, "QFP + RMT + JAM (all netted)");
        state
            .components
            .iter()
            .find(|c| c.pins.len() == 3 && c.component_id == 1)
            .expect("the QFP component row")
            .pins
            .iter()
            .map(|row| row.full_name.clone())
            .collect()
    };

    // Defaults: Java outer-first over the QFP gravity (600000, 413333):
    // QC (106667) > QA (93333) > QB (13333). The MASTER-ONLY face is
    // pinned identical (the committed T7 golden's face — the fanout
    // order must not ride the master alone).
    assert_eq!(
        pin_names(false, false),
        vec!["QFP1-QC", "QFP1-QA", "QFP1-QB"],
        "the Java outer-first order at defaults"
    );
    assert_eq!(
        pin_names(true, false),
        vec!["QFP1-QC", "QFP1-QA", "QFP1-QB"],
        "master-only keeps the Java order (the T7 golden's face)"
    );
    // Negotiated stage ON: NA's guide is jammed by the JAM pad
    // (severity > 0) -> rank(NA)=0 first; NB/NC tie at 0 severity ->
    // net-number order QB before QC.
    assert_eq!(
        pin_names(true, true),
        vec!["QFP1-QA", "QFP1-QB", "QFP1-QC"],
        "the congestion-aware order"
    );
}

/// The R-M3 fix face: on a board whose physical layer 1 is a POWER
/// plane (signal ordinals 0/1 map to physical 0/2), the B.Cu pair's
/// pattern candidates are evaluated at ORDINAL 1 — with the fix
/// reverted (the raw physical index 2) every corridor check lands past
/// the map's layer rows, answers false, and the fast path never fires.
/// The pin: with master + pattern ON, routing NA's pair emits the
/// `global_pattern_route` seam row.
#[test]
fn pattern_route_mixed_layer_ordinal_conversion() {
    use crate::pipeline::connection_router;
    use std::collections::{BTreeMap, HashMap};

    const G5: &str = include_str!("../../../../harness/fixtures/global-spike/g5_mixedlayer.dsn");

    let (mut manager, mut board) = parse(G5);
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
                crate::control::ExpansionCostFactor {
                    horizontal: 1.0,
                    vertical: 1.0,
                },
            ],
            via_costs: 1,
            vias_allowed: true,
            bend_costs: vec![0.0, 0.0, 0.0],
            layer_active: vec![true, true, true],
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
    assert_eq!(items.len(), 2, "the g5 pair");
    let item_id = items[0];

    // Rig precondition: the pins sit on physical layer 2 (B.Cu), whose
    // SIGNAL ORDINAL is 1 — the physical != ordinal board face.
    assert_eq!(board.layers().signal_layer_count(), 2, "2 signal layers");
    assert!(!board.layers().layers[1].is_signal, "physical 1 is POWER");

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
        None,
    );
    assert!(
        matches!(result.state, crate::engine::AutorouteAttemptState::Routed),
        "the mixed-layer pair must route: {:?}",
        result.details
    );
    assert!(
        sink.rows
            .iter()
            .any(|(_level, row)| row.starts_with("global_pattern_route net=1")),
        "the fast path must fire at the SIGNAL ORDINAL: {:?}",
        sink.rows
    );
}

/// M6-T8 quality Q6 / T9 bank: the congestion-rank re-sort is STABLE
/// BY CONTRACT ([`crate::pipeline::fanout::sort_by_congestion_rank`])
/// — equal ranks (including the `usize::MAX` absent-plan tie the call
/// site feeds via `unwrap_or(usize::MAX)`) keep the order the rows
/// already carry from the `sort_by(compare)` pass, and the rank alone
/// reorders. The row count (64) is past the unstable sorts'
/// insertion-sort floor, so an `sort_unstable_by_key` mutant actually
/// runs its partitioning on this input instead of coincidentally
/// stable-inserting; the tie classes are interleaved adversarially
/// (a partition swap cannot preserve them). The expectation is built
/// with the std STABLE sort on a fresh copy of the same rows.
#[test]
fn fanout_congestion_rank_sort_keeps_java_order_on_ties() {
    use crate::pipeline::fanout::sort_by_congestion_rank;

    // 64 rows over 4 rank classes (including the absent-plan MAX
    // class), interleaved so no tie class is contiguous in the input.
    let mut ranks = Vec::with_capacity(64);
    for i in 0..16 {
        ranks.push(7);
        ranks.push(3);
        ranks.push(usize::MAX);
        ranks.push(11);
        let _ = i;
    }
    // (rank, original index) — the payload an unstable partition
    // would permute within a tie class.
    let mut rows: Vec<(usize, usize)> = ranks.iter().copied().zip(0..).collect();
    sort_by_congestion_rank(&mut rows, |row| row.0);
    let mut expected: Vec<(usize, usize)> = ranks.iter().copied().zip(0..).collect();
    expected.sort_by_key(|row| row.0);
    assert_eq!(
        rows, expected,
        "ties keep the incoming Java order; only the rank reorders"
    );
    // The rank-alone-reorders face: the output IS grouped ascending
    // by rank (the interleaved input is not).
    let mut grouped = rows.clone();
    grouped.sort_by_key(|row| row.0);
    assert_eq!(rows, grouped, "the re-sort grouped the rows by rank");
    assert!(
        rows.windows(2).all(|w| w[0].0 <= w[1].0),
        "ascending by rank, MAX (absent plan) last"
    );
}
