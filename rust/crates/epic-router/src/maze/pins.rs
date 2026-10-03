//! The T6 jar-capture pins — the maze search CORE replayed against the
//! probe oracle `rust/harness/oracle/MazeSpike.java`.
//!
//! Capture: `/tmp/maze_capture_{1,2}.rows`, 182 JSON rows, run TWICE on
//! the jar with byte-identical output (`cmp` clean). Fixture:
//! `rust/harness/fixtures/maze-spike/t6_maze.dsn` (2 signal layers, a
//! keepout wall pair with a 4000-wide gap, SMD pins KA/PA and KB/PB on
//! NET_A, via VT). Build/run header lives in the oracle source.
//!
//! Protocol P (both sides reproduce it EXACTLY): a phase opens a FRESH
//! board (re-parse; a maze run mutates item start infos and the shared
//! tree — a second engine on the same board fails init), runs
//! `init`, then drains `while front non-empty && k < cap`: read the
//! front head (RAW element fields), call `occupyNextElement` ONCE,
//! record (before, after, cont); on `cont == false` record the
//! destination and BREAK (Java `findConnection` is the same loop —
//! popping past the destination re-marks it ~93 times, drain noise).
//! The Rust replay runs the same P, so head-vs-pop divergence cancels.
//!
//! The pinned rows cover:
//! * the totally ordered front — 28 MAIN pops (vias off) and 40 DRILL
//!   pops (vias on, drill pages + drills interleaved with room doors),
//!   each head a full RAW row (kind, id, section, values, backtrack,
//!   next room, shape entry) and each pop the front-size delta;
//! * the cost model — weighted distance + bend penalty (the BEND
//!   straight/diag contrast, bendCosts[0]=47) + the destination
//!   distance of the `sortingValue`;
//! * the occupancy marking — the front-size drops when an occupied
//!   section is skipped, plus a direct `is_occupied` state pin;
//! * `doorIsSmall` — the strict `<` boundary at exactly the door
//!   length (38964.0: false, 38965.0: true);
//! * the comparator tie chain — sortingValue → expansionValue → door
//!   id → section, the full-tie set dedup, the NaN fall-through and
//!   the -0.0 ≡ 0.0 tie (T1-T7).
//!
//! The T8 production `DestinationDistance`
//! (`super::destination_distance`) now backs every distance call in
//! these pins — the `TargetDistance` mirror it replaced was deleted
//! in the same commit, so the T6/T7 literals below exercise the
//! production module directly (the engine-scope differential test).
//! `LegalChecker` answers DRILLABLE because Java's real
//! `ForcedViaInserter.checkLayer` is vacuously legal on this empty
//! fixture; a mutant answering NotDrillable flips the DRILL head rows
//! (the DRILL-phase pins are its mutation coverage).

use std::collections::BTreeSet;

use epic_geometry::float_line::FloatLine;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::int_box::IntBox;
use epic_geometry::tile_shape::TileShape;

use super::destination_distance::DestinationDistance;
use super::list_element::{ExpandableObject, FrontOrder, MazeListElement, compare};
use super::search_engine::MazeSearchEngine;
use crate::control::{AngleRestriction, AutorouteControl, ExpansionCostFactor, RouterSettingsIr};
use crate::drill::pins::Harness;
use crate::drill::{
    Adjustment, CheckDrillResult, DestinationDistance as _, DrillEngine, DrillPageArray,
    ViaLayerChecker, max_drill_page_width,
};
use crate::expansion::{ExpansionDoor, NeighbourEngine};

/// The T6 spike fixture (a committed deliverable of this task).
const FIXTURE_DSN: &str = include_str!("../../../../harness/fixtures/maze-spike/t6_maze.dsn");

/// NET_A (capture `ctrl` row `netNumber:1`).
const NET_A: i32 = 1;

/// The pin item keys (capture `pins` row: startId 4, destId 5 — the
/// lower id starts).
const START_PIN: u64 = 4;
const DEST_PIN: u64 = 5;

/// bendCosts[0] forced for the run (the capture's bend probes).
const BEND_COST: f64 = 47.0;

/// Java `ForcedViaInserter.checkLayer` for this capture: the fixture
/// is empty board space, so every candidate via position is legal.
/// The DRILL-phase rows are the mutation coverage — a NotDrillable
/// answer reorders/drops them.
struct LegalChecker;

impl ViaLayerChecker for LegalChecker {
    fn check_layer(
        &mut self,
        _required_radius: f64,
        _clearance_class: i32,
        _attach_smd_allowed: bool,
        _room_shape: &TileShape,
        _location: &epic_geometry::point::Point,
        _layer: i32,
        _net_number: i32,
    ) -> CheckDrillResult {
        CheckDrillResult::Drillable
    }
}

// ---- the replay rig ----

/// Java `new RouterSettings(board)` projected to the IR — the capture
/// `ctrl` row's resolved scoring tables (2 layers, default via costs,
/// all layers active, no neckdown).
fn maze_settings_ir(vias_allowed: bool) -> RouterSettingsIr {
    RouterSettingsIr {
        trace_costs: vec![
            ExpansionCostFactor {
                horizontal: 1.0,
                vertical: 2.7,
            },
            ExpansionCostFactor {
                horizontal: 1.6,
                vertical: 1.0,
            },
        ],
        via_costs: 1,
        vias_allowed,
        bend_costs: vec![BEND_COST, 0.0],
        layer_active: vec![true, true],
        automatic_neckdown: false,
        start_ripup_costs: 1,
        fanout: Default::default(),
    }
}

/// One phase rig: a FRESH board (the capture re-parses per phase — a
/// maze run pollutes item start infos and the shared tree) with its
/// ctrl, distance, checker and page array.
fn make_run(
    vias_allowed: bool,
) -> (
    Harness,
    AutorouteControl,
    DestinationDistance,
    LegalChecker,
    DrillPageArray,
) {
    let mut harness = Harness::build_with(FIXTURE_DSN, NET_A);
    let ctrl = AutorouteControl::new(harness.board_mut(), NET_A, &maze_settings_ir(vias_allowed));
    let distance = DestinationDistance::from_ctrl(&ctrl);
    let checker = LegalChecker;
    let max_page_width = max_drill_page_width(harness.default_via_diameter());
    let pages = DrillPageArray::new(&harness, max_page_width);
    (harness, ctrl, distance, checker, pages)
}

/// The door polymorphism of a capture row (`doorKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    RoomDoor,
    TargetDoor,
    DrillPage,
    Drill,
}

fn kind_of(door: &ExpandableObject) -> Kind {
    match door {
        ExpandableObject::RoomDoor(_) => Kind::RoomDoor,
        ExpandableObject::TargetDoor(_) => Kind::TargetDoor,
        ExpandableObject::DrillPage { .. } => Kind::DrillPage,
        // Java's `ExpansionDrill` covers both the page-grid and the
        // standalone (ripped-via) drill.
        ExpandableObject::Drill { .. } | ExpandableObject::StandaloneDrill { .. } => Kind::Drill,
    }
}

// ---- literal tables (generated from /tmp/maze_capture_1.rows) ----

/// One RAW `head` row: the front element fields of the capture.
///
/// Every capture row carries `roomRipped:false, ripupCost:0,
/// alreadyChecked:false` (the no-ripup T6 world); the fields stay
/// in [`assert_head`] so the constants are pinned too.
struct RawHead {
    kind: Kind,
    door_id: i32,
    section: i32,
    exp: f64,
    sort: f64,
    back_kind: Option<Kind>,
    back_id: i32,
    back_section: i32,
    next_room_id: i32,
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
}

/// Row literal constructor — the float literals ARE the EXACT
/// `Double.toString` captures (both parses are correctly rounded, so
/// the literal is the captured f64 bit pattern).
#[allow(clippy::too_many_arguments)]
const fn h(
    kind: Kind,
    door_id: i32,
    section: i32,
    exp: f64,
    sort: f64,
    back_kind: Option<Kind>,
    back_id: i32,
    back_section: i32,
    next_room_id: i32,
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
) -> RawHead {
    // The literals ARE the capture strings: both Java's
    // Double.parseDouble and Rust's literal parse are correctly
    // rounded, so `445513.2277891202` is the exact captured f64.
    RawHead {
        kind,
        door_id,
        section,
        exp,
        sort,
        back_kind,
        back_id,
        back_section,
        next_room_id,
        ax,
        ay,
        bx,
        by,
    }
}

/// The MAIN drain `head` rows, k = 0..28 (capture order).
const MAIN_HEADS: [RawHead; 28] = [
    h(
        Kind::TargetDoor,
        125,
        0,
        0.0,
        578750.0,
        None,
        -1,
        0,
        1,
        200000.0,
        300000.0,
        200000.0,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        33,
        0,
        279663.4894122756,
        579663.4894122756,
        Some(Kind::TargetDoor),
        125,
        0,
        2,
        478750.0,
        283270.0,
        478750.0,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        33,
        1,
        279663.4894122756,
        579663.4894122756,
        Some(Kind::TargetDoor),
        125,
        0,
        2,
        478750.0,
        300000.0,
        478750.0,
        316730.0,
    ),
    h(
        Kind::RoomDoor,
        65,
        0,
        280940.2711977969,
        580208.2711977969,
        Some(Kind::RoomDoor),
        33,
        0,
        3,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        65,
        1,
        280940.2711977969,
        580208.2711977969,
        Some(Kind::RoomDoor),
        33,
        1,
        3,
        479482.0,
        300000.0,
        479482.0,
        315998.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        9,
        575651.0899450427,
        581688.0899450427,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        759584.0,
        281250.0,
        785841.1111111111,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        9,
        575651.0899450427,
        581688.0899450427,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        759584.0,
        318750.0,
        785841.1111111111,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        8,
        549534.1524044513,
        581828.1524044513,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        733326.8888888889,
        281250.0,
        759584.0,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        8,
        549534.1524044513,
        581828.1524044513,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        733326.8888888889,
        318750.0,
        759584.0,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        7,
        523447.4957723329,
        581998.4957723329,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        707069.7777777778,
        281250.0,
        733326.8888888889,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        7,
        523447.4957723329,
        581998.4957723329,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        707069.7777777778,
        318750.0,
        733326.8888888889,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        6,
        497402.0705148735,
        582210.0705148735,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        680812.6666666666,
        281250.0,
        707069.7777777778,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        6,
        497402.0705148735,
        582210.0705148735,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        680812.6666666666,
        318750.0,
        707069.7777777778,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        5,
        471414.7954530079,
        582479.7954530079,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        654555.5555555555,
        281250.0,
        680812.6666666666,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        5,
        471414.7954530079,
        582479.7954530079,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        654555.5555555555,
        318750.0,
        680812.6666666666,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        4,
        445513.2277891202,
        582836.2277891203,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        628298.4444444445,
        281250.0,
        654555.5555555555,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        4,
        445513.2277891202,
        582836.2277891203,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        628298.4444444445,
        318750.0,
        654555.5555555555,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        3,
        419745.37207271333,
        583325.3720727133,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        602041.3333333334,
        281250.0,
        628298.4444444445,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        3,
        419745.37207271333,
        583325.3720727133,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        602041.3333333334,
        318750.0,
        628298.4444444445,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        2,
        394202.5638589007,
        584039.5638589007,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        575784.2222222222,
        281250.0,
        602041.3333333334,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        2,
        394202.5638589007,
        584039.5638589007,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        575784.2222222222,
        318750.0,
        602041.3333333334,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        1,
        369080.779438006,
        585174.779438006,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        549527.1111111111,
        281250.0,
        575784.2222222222,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        1,
        369080.779438006,
        585174.779438006,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        549527.1111111111,
        318750.0,
        575784.2222222222,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        0,
        344878.59818040574,
        587229.5981804058,
        Some(Kind::RoomDoor),
        65,
        0,
        4,
        523270.0,
        281250.0,
        549527.1111111111,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        0,
        344878.59818040574,
        587229.5981804058,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        523270.0,
        318750.0,
        549527.1111111111,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        9,
        582981.0117433746,
        589018.0117433746,
        Some(Kind::RoomDoor),
        65,
        1,
        4,
        759584.0,
        281250.0,
        785841.1111111111,
        281250.0,
    ),
    h(
        Kind::RoomDoor,
        100,
        10,
        601790.912884423,
        601790.912884423,
        Some(Kind::RoomDoor),
        65,
        1,
        7,
        785841.1111111111,
        318750.0,
        812098.2222222222,
        318750.0,
    ),
    h(
        Kind::RoomDoor,
        97,
        2,
        412102.3189319887,
        601939.3189319887,
        Some(Kind::RoomDoor),
        65,
        1,
        4,
        575784.2222222222,
        281250.0,
        602041.3333333334,
        281250.0,
    ),
];

/// The MAIN drain `popMeta` rows: (before, after, cont).
const MAIN_POPS: [(i32, i32, bool); 28] = [
    (1, 2, true),
    (2, 3, true),
    (3, 4, true),
    (4, 40, true),
    (40, 75, true),
    (75, 89, true),
    (89, 103, true),
    (103, 117, true),
    (117, 131, true),
    (131, 145, true),
    (145, 159, true),
    (159, 173, true),
    (173, 187, true),
    (187, 201, true),
    (201, 215, true),
    (215, 229, true),
    (229, 243, true),
    (243, 257, true),
    (257, 271, true),
    (271, 285, true),
    (285, 299, true),
    (299, 313, true),
    (313, 327, true),
    (327, 341, true),
    (341, 355, true),
    (355, 355, true),
    (355, 369, true),
    (369, 366, false),
];

/// The DRILL drain `head` rows, k = 0..40 (capture order).
const DRILL_HEADS: [RawHead; 40] = [
    h(
        Kind::TargetDoor,
        125,
        0,
        0.0,
        578750.0,
        None,
        -1,
        0,
        1,
        200000.0,
        300000.0,
        200000.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -1950813583,
        0,
        400.0,
        579150.0,
        Some(Kind::TargetDoor),
        125,
        0,
        1,
        200000.0,
        300000.0,
        200000.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -1913489583,
        0,
        400.0,
        579150.0,
        Some(Kind::TargetDoor),
        125,
        0,
        1,
        200000.0,
        300000.0,
        200000.0,
        300000.0,
    ),
    h(
        Kind::Drill,
        -1701202883,
        0,
        29573.69021909981,
        458472.6902190998,
        Some(Kind::TargetDoor),
        125,
        0,
        -1,
        349851.0,
        304000.0,
        349851.0,
        304000.0,
    ),
    h(
        Kind::Drill,
        -1701202883,
        1,
        29573.69021909981,
        458872.6902190998,
        Some(Kind::Drill),
        -1701202883,
        0,
        9,
        349851.0,
        304000.0,
        349851.0,
        304000.0,
    ),
    h(
        Kind::RoomDoor,
        102,
        13,
        38012.63861428629,
        462666.6386142863,
        Some(Kind::Drill),
        -1701202883,
        1,
        3,
        341517.6842105263,
        300000.0,
        367472.7368421053,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -1950813581,
        1,
        38412.63861428629,
        463066.6386142863,
        Some(Kind::RoomDoor),
        102,
        13,
        3,
        341517.6842105263,
        300000.0,
        367472.7368421053,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        102,
        14,
        78695.64212314785,
        477394.64212314785,
        Some(Kind::Drill),
        -1701202883,
        1,
        3,
        367472.7368421053,
        300000.0,
        393427.7894736843,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -1950813581,
        1,
        79095.64212314785,
        477794.64212314785,
        Some(Kind::RoomDoor),
        102,
        14,
        3,
        367472.7368421053,
        300000.0,
        393427.7894736843,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -765662255,
        1,
        79095.64212314785,
        480159.2210705162,
        Some(Kind::RoomDoor),
        102,
        14,
        3,
        367472.7368421053,
        300000.0,
        393427.7894736843,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -765662253,
        1,
        38412.63861428629,
        481004.30177218106,
        Some(Kind::RoomDoor),
        102,
        13,
        3,
        341517.6842105263,
        300000.0,
        367472.7368421053,
        300000.0,
    ),
    h(
        Kind::Drill,
        -589245159,
        1,
        92414.08490269308,
        483174.08490269305,
        Some(Kind::RoomDoor),
        102,
        14,
        -1,
        388390.0,
        296000.0,
        388390.0,
        296000.0,
    ),
    h(
        Kind::Drill,
        -589245159,
        0,
        92414.08490269308,
        482774.08490269305,
        Some(Kind::Drill),
        -589245159,
        1,
        1,
        388390.0,
        296000.0,
        388390.0,
        296000.0,
    ),
    h(
        Kind::Drill,
        -589245159,
        1,
        92791.61698710383,
        483551.61698710383,
        Some(Kind::RoomDoor),
        102,
        13,
        -1,
        388390.0,
        296000.0,
        388390.0,
        296000.0,
    ),
    h(
        Kind::DrillPage,
        1604640401,
        0,
        183986.42501316947,
        483986.4250131695,
        Some(Kind::RoomDoor),
        33,
        0,
        2,
        478750.0,
        283270.0,
        478750.0,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        73,
        0,
        184863.20679869078,
        484131.2067986908,
        Some(Kind::RoomDoor),
        33,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -1799050863,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -1505175567,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -1059537551,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        -320024239,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        125613777,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        865127089,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        1310765105,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        1604640403,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::Drill,
        1716054496,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        -1,
        479482.0,
        292001.0,
        479482.0,
        292001.0,
    ),
    h(
        Kind::DrillPage,
        2050278417,
        0,
        185263.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::Drill,
        -1430797451,
        0,
        209788.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        -1,
        504007.0,
        292001.0,
        504007.0,
        292001.0,
    ),
    h(
        Kind::Drill,
        -1122813288,
        0,
        230299.20679869078,
        484531.2067986908,
        Some(Kind::RoomDoor),
        73,
        0,
        -1,
        524518.0,
        292001.0,
        524518.0,
        292001.0,
    ),
    h(
        Kind::Drill,
        1716054496,
        1,
        185263.20679869078,
        484931.2067986908,
        Some(Kind::Drill),
        1716054496,
        0,
        3,
        479482.0,
        292001.0,
        479482.0,
        292001.0,
    ),
    h(
        Kind::Drill,
        -1430797451,
        1,
        209788.20679869078,
        484931.2067986908,
        Some(Kind::Drill),
        -1430797451,
        0,
        5,
        504007.0,
        292001.0,
        504007.0,
        292001.0,
    ),
    h(
        Kind::Drill,
        -1122813288,
        1,
        230299.20679869078,
        484931.2067986908,
        Some(Kind::Drill),
        -1122813288,
        0,
        5,
        524518.0,
        292001.0,
        524518.0,
        292001.0,
    ),
    h(
        Kind::DrillPage,
        -1761726863,
        0,
        185263.20679869078,
        485333.48534766433,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        1348089105,
        0,
        185263.20679869078,
        485455.87751820916,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::DrillPage,
        1159002385,
        1,
        38412.63861428629,
        485542.37545639154,
        Some(Kind::RoomDoor),
        102,
        13,
        3,
        341517.6842105263,
        300000.0,
        367472.7368421053,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        353,
        9,
        479574.02554593654,
        485611.02554593654,
        Some(Kind::RoomDoor),
        73,
        0,
        12,
        759584.0,
        281250.0,
        785841.1111111111,
        281250.0,
    ),
    h(
        Kind::DrillPage,
        162937777,
        0,
        185263.20679869078,
        485622.22323809366,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        353,
        8,
        453457.0880053451,
        485751.0880053451,
        Some(Kind::RoomDoor),
        73,
        0,
        12,
        733326.8888888889,
        281250.0,
        759584.0,
        281250.0,
    ),
    h(
        Kind::DrillPage,
        -1022213551,
        0,
        185263.20679869078,
        485861.2721562131,
        Some(Kind::RoomDoor),
        73,
        0,
        11,
        479482.0,
        284002.0,
        479482.0,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        353,
        7,
        427370.4313732267,
        485921.4313732267,
        Some(Kind::RoomDoor),
        73,
        0,
        12,
        707069.7777777778,
        281250.0,
        733326.8888888889,
        281250.0,
    ),
    h(
        Kind::DrillPage,
        -1799050861,
        0,
        479974.02554593654,
        486011.02554593654,
        Some(Kind::RoomDoor),
        353,
        9,
        12,
        759584.0,
        281250.0,
        785841.1111111111,
        281250.0,
    ),
];

/// The DRILL drain `popMeta` rows: (before, after, cont).
const DRILL_POPS: [(i32, i32, bool); 40] = [
    (1, 210, true),
    (210, 209, true),
    (209, 209, true),
    (209, 209, true),
    (209, 243, true),
    (243, 357, true),
    (357, 356, true),
    (356, 470, true),
    (470, 469, true),
    (469, 469, true),
    (469, 469, true),
    (469, 469, true),
    (469, 470, true),
    (470, 472, true),
    (472, 471, true),
    (471, 535, true),
    (535, 534, true),
    (534, 535, true),
    (535, 534, true),
    (534, 533, true),
    (533, 532, true),
    (532, 531, true),
    (531, 530, true),
    (530, 530, true),
    (530, 530, true),
    (530, 529, true),
    (529, 529, true),
    (529, 529, true),
    (529, 556, true),
    (556, 599, true),
    (599, 642, true),
    (642, 641, true),
    (641, 640, true),
    (640, 640, true),
    (640, 736, true),
    (736, 735, true),
    (735, 831, true),
    (831, 830, true),
    (830, 926, true),
    (926, 926, true),
];

/// One pinned `bendElem` row (the BEND probes' front dump).
const BEND_STRAIGHT_HEADS: [RawHead; 3] = [
    h(
        Kind::RoomDoor,
        33,
        0,
        22754.62725829493,
        322754.62725829496,
        Some(Kind::RoomDoor),
        232558643,
        0,
        2,
        478750.0,
        283270.0,
        478750.0,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        33,
        1,
        22754.62725829493,
        322754.62725829496,
        Some(Kind::RoomDoor),
        232558643,
        0,
        2,
        478750.0,
        300000.0,
        478750.0,
        316730.0,
    ),
    h(
        Kind::TargetDoor,
        125,
        0,
        277850.0,
        856600.0,
        Some(Kind::RoomDoor),
        232558643,
        0,
        -1,
        200000.0,
        300000.0,
        200000.0,
        300000.0,
    ),
];

/// The BEND_DIAG front dump — SAME seed, DIAGONAL entry chord: the
/// two sections split apart (20057.6 vs 25452.2) where the straight
/// entry gave equal values, and the target row shifts by the bend.
const BEND_DIAG_HEADS: [RawHead; 3] = [
    h(
        Kind::RoomDoor,
        33,
        0,
        20057.628072715335,
        320057.62807271536,
        Some(Kind::RoomDoor),
        232558643,
        0,
        2,
        478750.0,
        283270.0,
        478750.0,
        300000.0,
    ),
    h(
        Kind::RoomDoor,
        33,
        1,
        25452.266452855223,
        325452.2664528552,
        Some(Kind::RoomDoor),
        232558643,
        0,
        2,
        478750.0,
        300000.0,
        478750.0,
        316730.0,
    ),
    h(
        Kind::TargetDoor,
        125,
        0,
        277863.1230023165,
        856613.1230023166,
        Some(Kind::RoomDoor),
        232558643,
        0,
        -1,
        200000.0,
        300000.0,
        200000.0,
        300000.0,
    ),
];

/// The capture `ctrl` row's derived-control assertions (shared by
/// both phase rigs).
fn assert_ctrl(ctrl: &AutorouteControl, harness: &Harness, vias_allowed: bool) {
    assert_eq!(ctrl.net_number, 1, "ctrl netNumber");
    assert_eq!(ctrl.layer_count, 2, "ctrl layerCount");
    assert_eq!(ctrl.vias_allowed, vias_allowed, "ctrl viasAllowed");
    assert_eq!(ctrl.bend_costs, vec![BEND_COST, 0.0], "ctrl bendCosts");
    assert!(!ctrl.with_neckdown, "ctrl withNeckdown");
    assert!(!ctrl.is_fanout, "ctrl isFanout");
    assert!(!ctrl.ripup_allowed, "ctrl ripupAllowed");
    assert_eq!(ctrl.max_shove_trace_recursion_depth, 20, "ctrl maxShove");
    assert_eq!(
        ctrl.compensated_trace_half_width,
        [2750, 2750],
        "ctrl compensatedTraceHalfWidth"
    );
    assert_eq!(
        ctrl.trace_costs,
        vec![
            ExpansionCostFactor {
                horizontal: 1.0,
                vertical: 2.7
            },
            ExpansionCostFactor {
                horizontal: 1.6,
                vertical: 1.0
            },
        ],
        "ctrl traceCosts"
    );
    assert_eq!(ctrl.layer_active, [true, true], "ctrl layerActive");
    assert_eq!(ctrl.min_normal_via_cost, 400.0, "ctrl minNormalViaCost");
    assert_eq!(ctrl.min_cheap_via_cost, 320.0, "ctrl minCheapViaCost");
    assert_eq!(
        harness.trace_angle_restriction(),
        AngleRestriction::FortyfiveDegree,
        "ctrl angleRestriction"
    );
}

/// Asserts one front element against its capture `head` row. `what`
/// names the row for the failure message. `live_door_id` is the
/// Java-faithful id read: Java's capture prints `element.door.getId()`
/// on the LIVE door object, and `DrillPage.getId()` = `31 *
/// shape.getId() + netNumber` embeds the page's CURRENT drill-net
/// state (-1 before its first `getDrills`, the net afterwards). The
/// frozen id stored in the [`ExpandableObject::DrillPage`] variant is
/// the construction-time value — equal to the live read unless the
/// page was drilled between the element's insertion and its head read
/// (DRILL head k=10: frozen -765662255 vs live -765662253, delta =
/// +2·net). Ordering is unaffected: live-vs-frozen can only disagree
/// for |31·Δs| ≤ 2, i.e. the SAME page, where both reads are equal and
/// the comparator falls through to the section anyway.
fn assert_head(
    ctx: &Harness,
    element: &MazeListElement,
    expected: &RawHead,
    what: &str,
    live_door_id: i32,
) {
    assert_eq!(kind_of(&element.door), expected.kind, "{what} doorKind");
    assert_eq!(live_door_id, expected.door_id, "{what} doorId");
    assert_eq!(
        element.section_no_of_door, expected.section,
        "{what} section"
    );
    assert_eq!(
        element.expansion_value, expected.exp,
        "{what} expansionValue"
    );
    assert_eq!(element.sorting_value, expected.sort, "{what} sortingValue");
    let (back_kind, back_id) = match &element.backtrack_door {
        None => (None, -1),
        Some(door) => (Some(kind_of(door)), door.id()),
    };
    assert_eq!(back_kind, expected.back_kind, "{what} backKind");
    assert_eq!(back_id, expected.back_id, "{what} backId");
    assert_eq!(
        element.section_no_of_backtrack_door, expected.back_section,
        "{what} backSection"
    );
    let next_room_id = element
        .next_room_key
        .map(|key| ctx.room_id(key))
        .unwrap_or(-1);
    assert_eq!(next_room_id, expected.next_room_id, "{what} nextRoomId");
    assert_eq!(element.shape_entry.a.x, expected.ax, "{what} ax");
    assert_eq!(element.shape_entry.a.y, expected.ay, "{what} ay");
    assert_eq!(element.shape_entry.b.x, expected.bx, "{what} bx");
    assert_eq!(element.shape_entry.b.y, expected.by, "{what} by");
    // The constants every capture row carries.
    assert!(!element.room_ripped, "{what} roomRipped");
    assert_eq!(element.ripup_cost, 0, "{what} ripupCost");
    assert!(!element.already_checked, "{what} alreadyChecked");
}

/// The drain under capture protocol P: head row k, one
/// `occupy_next_element`, the (before, after, cont) literal. Returns
/// the reached destination (the `cont == false` row), if any.
fn drain(
    engine: &mut MazeSearchEngine<'_, Harness, DestinationDistance, LegalChecker>,
    heads: &[RawHead],
    pops: &[(i32, i32, bool)],
    phase: &str,
) -> Option<(ExpandableObject, i32)> {
    let mut dest = None;
    for (k, expected) in heads.iter().enumerate() {
        let head = engine
            .front
            .iter()
            .next()
            .unwrap_or_else(|| panic!("{phase}: front empty at pop {k}"))
            .clone();
        // The Java-faithful id read (see assert_head): DrillPage ids
        // resolve through the LIVE page (its net state may have flipped
        // since the element was inserted).
        let live_door_id = match &head.door {
            ExpandableObject::DrillPage { row, column, .. } => {
                engine.pages.page(*row, *column).get_id()
            }
            _ => head.door.id(),
        };
        assert_head(
            &*engine.ctx,
            &head,
            expected,
            &format!("{phase} head {k}"),
            live_door_id,
        );
        let before = i32::try_from(engine.front.len()).expect("front size");
        let cont = engine.occupy_next_element();
        let after = i32::try_from(engine.front.len()).expect("front size");
        let (eb, ea, ec) = pops[k];
        assert_eq!((before, after, cont), (eb, ea, ec), "{phase} popMeta {k}");
        if !cont {
            let reached = engine
                .destination
                .clone()
                .unwrap_or_else(|| panic!("{phase}: cont=false without a destination"));
            dest = Some(reached);
            break;
        }
    }
    dest
}

/// The MAIN phase: ctrl row, INIT row, the pre-completion door dump,
/// the destination-distance probes, the `doorIsSmall` boundary and the
/// full 28-pop drain to the destination.
#[test]
fn main_phase_protocol_pins() {
    let (mut harness, mut ctrl, mut distance, mut checker, mut pages) = make_run(false);
    assert_ctrl(&ctrl, &harness, false);

    let mut engine = MazeSearchEngine::new(
        &mut harness,
        &mut ctrl,
        &mut distance,
        &mut checker,
        &mut pages,
    );
    assert!(engine.init(&[START_PIN], &[DEST_PIN]), "MAIN init ok");

    // The INIT row: a single seed, the RAW row equal to head k=0.
    assert_eq!(engine.front.len(), 1, "init frontSize");
    let seed = engine.front.iter().next().expect("the init seed").clone();
    assert_head(
        &*engine.ctx,
        &seed,
        &MAIN_HEADS[0],
        "INIT head",
        seed.door.id(),
    );

    // The door dump (BEFORE any pop): the start room holds exactly one
    // door pre-completion; `completeNeighbourRooms` adds door 33 only
    // when the drain expands the room — the ordering is pinned.
    let start_room = seed.next_room_key.expect("the seed's room");
    let start_doors = engine.ctx.room_doors(start_room);
    assert_eq!(start_doors.len(), 1, "door dump: exactly one door");
    let start_door = &start_doors[0];
    assert_eq!(start_door.id(), 232558643, "door i=0 id");
    assert_eq!(start_door.dimension, 1, "door i=0 dim");
    {
        let (first_key, second_key, first_shape, second_shape) = {
            let ctx = &*engine.ctx;
            let fk = ctx.room_key_of_id(start_door.first_room_id);
            let sk = ctx.room_key_of_id(start_door.second_room_id);
            let fs = ctx.room_shape(fk.expect("live first room"));
            let ss = ctx.room_shape(sk.expect("live second room"));
            (fk, sk, fs, ss)
        };
        let _ = (first_key, second_key);
        let shape = ExpansionDoor::shape_between(&first_shape, &second_shape);
        assert_eq!(
            shape.bounding_box().max_width(),
            38964.0,
            "door i=0 len (IntBox maxWidth)"
        );
    }

    // The destination-distance probes (`dist` rows) — the REAL
    // heuristic, not the T5 targetless stub.
    {
        let dist = &engine.destination_distance;
        let probes = [
            (200000.0, 300000.0, 0, 578750.0),
            (200000.0, 300000.0, 1, 579150.0),
            (500000.0, 300000.0, 0, 278750.0),
            (10000.0, 10000.0, 1, 1037900.0),
        ];
        for (x, y, layer, expected) in probes {
            let value = dist.calculate(&FloatPoint::new(x, y), layer);
            assert_eq!(value, expected, "dist ({x},{y},{layer})");
        }
    }

    // The `doorIsSmall` boundary: strict `<` at exactly the door
    // length (38964.0 → NOT small; 38965.0 → small). The harness runs
    // the 90-degree arm (`bounding_box().max_width()`); the 45-degree
    // engine measures `bounding_octagon().max_width()` instead, which
    // coincides for this axis-aligned door — the boundary pins
    // transfer.
    {
        for (w, expected_small) in [(38963.0, false), (38964.0, false), (38965.0, true)] {
            let small = engine.door_is_small(start_door, w);
            assert_eq!(small, expected_small, "doorIsSmall door 232558643 w={w}");
        }
    }

    // The 28-pop drain to the destination (head rows + popMeta rows).
    let dest = drain(&mut engine, &MAIN_HEADS, &MAIN_POPS, "MAIN");
    let (dest_door, dest_section) = dest.expect("MAIN reaches the destination");
    assert_eq!(dest_door.id(), 158, "dest doorId");
    assert_eq!(dest_section, 0, "dest section");
    assert!(!engine.front.is_empty(), "drainSummary frontEmpty false");

    // The direct occupancy state pin: the door-33 section-0 element
    // (head k=1) popped at k=1 — its section is marked. The popMeta
    // front-size rows are the behavioral mirror of this state.
    // Resolved from the LIVE registry: the door instance tag is Java's
    // object identity, so a literal `(1, 2, 1)` reconstruction would
    // read a different state slot. The start room's list gained door
    // 33 (`31*1+2`) when `completeNeighbourRooms` expanded the room.
    let door33 = ExpandableObject::RoomDoor(
        engine
            .ctx
            .room_doors(start_room)
            .into_iter()
            .find(|d| d.id() == 33)
            .expect("door 33 on the start room after the drain"),
    );
    assert!(
        engine.maze_element(&door33, 0).is_occupied,
        "door 33 section 0 occupied after the drain"
    );
    assert!(
        engine.maze_element(&door33, 1).is_occupied,
        "door 33 section 1 occupied after the drain"
    );
}

/// The T8 engine-scope join row: init joins the destination pin's
/// tree-shape bounding box at layer 0 — the capture's single
/// engine-scope `ddJoin` row (isFanout=false, so the board-bounds
/// fanout joins of MazeSearchEngine.java:991-992 do NOT fire; the
/// DRILL phase ran the unspied openSearch and emits none either).
#[test]
fn t8_engine_init_join_row() {
    let (mut harness, mut ctrl, mut distance, mut checker, mut pages) = make_run(false);
    let mut engine = MazeSearchEngine::new(
        &mut harness,
        &mut ctrl,
        &mut distance,
        &mut checker,
        &mut pages,
    );
    assert!(engine.init(&[START_PIN], &[DEST_PIN]), "MAIN init ok");
    // {"type":"ddJoin","phase":"MAIN","layer":0,"llx":778750,
    //  "lly":278750,"urx":821250,"ury":321250} — the SMD dest pin's
    // tree shape, in the COMPONENT bucket (layer 0).
    assert_eq!(
        engine.destination_distance.component_side_box(),
        &IntBox::from_corners(778750, 278750, 821250, 321250),
        "ddJoin MAIN: dest tree shape in the component bucket"
    );
    // The other buckets stay empty (unjoined → the EMPTY sentinel).
    assert_eq!(
        engine.destination_distance.solder_side_box(),
        &IntBox::EMPTY,
        "solder bucket unjoined"
    );
    assert_eq!(
        engine.destination_distance.inner_side_box(),
        &IntBox::EMPTY,
        "inner bucket unjoined"
    );
}

/// The DRILL phase: vias ON — the page grid row, then the 40-pop drain
/// interleaving DrillPage, ExpansionDrill and room-door rows (the
/// drill dispatch).
#[test]
fn drill_phase_protocol_pins() {
    let (mut harness, mut ctrl, mut distance, mut checker, mut pages) = make_run(true);
    assert_ctrl(&ctrl, &harness, true);

    // The pageMeta row: the grid frame the DrillPage ids hang on.
    assert_eq!(harness.default_via_diameter(), 8000.0, "pageMeta viaDiam");
    let max_page_width = max_drill_page_width(harness.default_via_diameter());
    assert_eq!(max_page_width, 40000, "pageMeta maxPageWidth");
    assert_eq!(pages.column_count, 26, "pageMeta columnCount");
    assert_eq!(pages.row_count, 16, "pageMeta rowCount");
    assert_eq!(pages.page_width, 38539, "pageMeta pageWidth");
    assert_eq!(pages.page_height, 37625, "pageMeta pageHeight");

    let mut engine = MazeSearchEngine::new(
        &mut harness,
        &mut ctrl,
        &mut distance,
        &mut checker,
        &mut pages,
    );
    assert!(engine.init(&[START_PIN], &[DEST_PIN]), "DRILL init ok");

    // The 40-pop drain: cap reached, destination NOT reached (no
    // cont=false row in the capture).
    let dest = drain(&mut engine, &DRILL_HEADS, &DRILL_POPS, "DRILL");
    assert!(dest.is_none(), "DRILL drain hits the cap, not the dest");
    assert!(!engine.front.is_empty(), "drainSummary frontEmpty false");
    assert!(
        engine.destination.is_none(),
        "DRILL: no destination row in the capture"
    );
}

/// The BEND probes: a seeded door element with a live backtrack door
/// and a straight vs diagonal entry chord — the bend penalty +
/// weighted-distance contrast (bendCosts[0] = 47).
#[test]
fn bend_cost_contrast_pins() {
    for (diag, expected) in [(false, &BEND_STRAIGHT_HEADS), (true, &BEND_DIAG_HEADS)] {
        let (mut harness, mut ctrl, mut distance, mut checker, mut pages) = make_run(false);
        let mut engine = MazeSearchEngine::new(
            &mut harness,
            &mut ctrl,
            &mut distance,
            &mut checker,
            &mut pages,
        );
        assert!(
            engine.init(&[START_PIN], &[DEST_PIN]),
            "diag={diag} init ok"
        );

        // The init seed donates the backtrack door and the room.
        let seed_target = engine.front.iter().next().expect("the init seed").clone();
        let room_key = seed_target.next_room_key.expect("the seed's room");
        let doors = engine.ctx.room_doors(room_key);
        assert_eq!(doors.len(), 1, "bendSeed: the pre-pop door set");
        let door = &doors[0];
        assert_eq!(door.id(), 232558643, "bendSeed doorId");

        // allocateSections fires inside getSectionSegments — the
        // production precondition for reading sectionArr
        // (MazeSearchEngine.expandToDoor:715).
        let (first_key, second_key, first_shape, second_shape) = {
            let ctx = &*engine.ctx;
            let fk = ctx.room_key_of_id(door.first_room_id);
            let sk = ctx.room_key_of_id(door.second_room_id);
            let fs = ctx.room_shape(fk.expect("live first room"));
            let ss = ctx.room_shape(sk.expect("live second room"));
            (fk, sk, fs, ss)
        };
        let both_complete_free_space = first_key
            .is_some_and(|k| engine.ctx.room_is_complete_free_space(k))
            && second_key.is_some_and(|k| engine.ctx.room_is_complete_free_space(k));
        let door_shape = ExpansionDoor::shape_between(&first_shape, &second_shape);
        let seed_layer = engine.ctx.room_layer(room_key);
        let seed_half_width =
            f64::from(engine.ctrl.compensated_trace_half_width[seed_layer as usize]);
        let (section_count, _) = door.get_section_segments(
            &door_shape,
            both_complete_free_space,
            &first_shape,
            &second_shape,
            seed_half_width,
        );
        assert_eq!(section_count, 2, "bendSeed sectionCount");

        // The entry chord: straight = axis-parallel, diag = 45
        // degrees, both through the door center.
        let c = door_shape.centre_of_gravity();
        let entry = if diag {
            FloatLine::new(FloatPoint::new(c.x - 2000.0, c.y - 2000.0), c)
        } else {
            FloatLine::new(FloatPoint::new(c.x - 2000.0, c.y), c)
        };
        let seed_elem = MazeListElement::new(
            ExpandableObject::RoomDoor(door.clone()),
            0,
            Some(seed_target.door.clone()),
            seed_target.section_no_of_door,
            100.0,
            100.0,
            Some(room_key),
            entry,
            false,
            Adjustment::None,
            false,
        );
        engine.front.clear();
        assert!(engine.front.add(seed_elem), "bendSeed accepted");
        let before = engine.front.len();
        let _ = engine.occupy_next_element();
        let after = engine.front.len();
        assert_eq!(
            (before, after),
            (1, 3),
            "bendSeed before/after (diag={diag})"
        );

        // The front dump (bendElem rows).
        let elems: Vec<MazeListElement> = engine.front.iter().cloned().collect();
        assert_eq!(elems.len(), expected.len(), "bendElem count (diag={diag})");
        for (element, want) in elems.iter().zip(expected.iter()) {
            assert_head(&*engine.ctx, element, want, "bendElem", element.door.id());
        }
    }
}

/// Java `ExpansionDoor.allocateSections` (`:193-203`) — the
/// re-segmentation contract: re-allocating with the SAME section count
/// is idempotent (marks survive), re-allocating with a DIFFERENT count
/// RESETS the section state. The T6 capture contains no
/// re-segmentation event (no door changes its section count mid-drain),
/// so the drain pins cannot discriminate the reset arm — this direct
/// contract pin closes that coverage boundary (cerebrum mode 10).
#[test]
fn allocate_sections_resegmentation_contract() {
    let (mut harness, mut ctrl, mut distance, mut checker, mut pages) = make_run(false);
    let mut engine = MazeSearchEngine::new(
        &mut harness,
        &mut ctrl,
        &mut distance,
        &mut checker,
        &mut pages,
    );
    assert!(engine.init(&[START_PIN], &[DEST_PIN]), "init ok");
    let room_key = engine
        .front
        .iter()
        .next()
        .expect("the init seed")
        .next_room_key
        .expect("the seed's room");
    let doors = engine.ctx.room_doors(room_key);
    let door = ExpandableObject::RoomDoor(doors[0].clone());

    engine.allocate_sections(&door, 2);
    engine.maze_element_mut(&door, 1).is_occupied = true;

    // Same count: idempotent, the mark SURVIVES.
    engine.allocate_sections(&door, 2);
    assert!(
        engine.maze_element(&door, 1).is_occupied,
        "same-count re-allocation keeps the section mark"
    );

    // Different count: re-segmentation RESETS the state.
    engine.allocate_sections(&door, 3);
    assert!(
        !engine.maze_element(&door, 1).is_occupied,
        "re-segmentation resets the section state"
    );
}

/// The `expandToDoor` occupancy pre-check is PER SECTION (`:715`): a
/// multi-section door expands its FREE sections — section 0 occupied
/// must not block section 1. Review mutant M2 (the pre-check reading
/// section 0 instead of `section`) survived every drain pin: the
/// captures never revisit an occupied-section door that still has a
/// free later section. Cerebrum mode 10 — direct contract pin,
/// mutation-verified (kills M2).
#[test]
fn expand_to_door_precheck_is_per_section() {
    let (mut harness, mut ctrl, mut distance, mut checker, mut pages) = make_run(false);
    let mut engine = MazeSearchEngine::new(
        &mut harness,
        &mut ctrl,
        &mut distance,
        &mut checker,
        &mut pages,
    );
    assert!(engine.init(&[START_PIN], &[DEST_PIN]), "init ok");
    let seed = engine.front.iter().next().expect("the init seed").clone();
    let from_room = seed.next_room_key.expect("the seed's room");
    let door = engine.ctx.room_doors(from_room)[0].clone();
    assert_eq!(door.id(), 232558643, "the start door");
    let door_object = ExpandableObject::RoomDoor(door.clone());

    // allocateSections fires inside getSectionSegments — the door has
    // TWO sections (the bendSeed pin pins the same arity).
    let (first_key, second_key, first_shape, second_shape) = {
        let ctx = &*engine.ctx;
        let fk = ctx.room_key_of_id(door.first_room_id);
        let sk = ctx.room_key_of_id(door.second_room_id);
        let fs = ctx.room_shape(fk.expect("live first room"));
        let ss = ctx.room_shape(sk.expect("live second room"));
        (fk, sk, fs, ss)
    };
    let both_complete_free_space = first_key
        .is_some_and(|k| engine.ctx.room_is_complete_free_space(k))
        && second_key.is_some_and(|k| engine.ctx.room_is_complete_free_space(k));
    let door_shape = ExpansionDoor::shape_between(&first_shape, &second_shape);
    let layer = engine.ctx.room_layer(from_room);
    let half_width = f64::from(engine.ctrl.compensated_trace_half_width[layer as usize]);
    let (section_count, _) = door.get_section_segments(
        &door_shape,
        both_complete_free_space,
        &first_shape,
        &second_shape,
        half_width,
    );
    assert_eq!(section_count, 2, "the start door has two sections");
    // Allocate the TRUE count BEFORE marking: a lazy `maze_element_mut`
    // resize would look like a re-segmentation (different count) and
    // reset the state inside `expand_to_door`'s `allocateSections`.
    engine.allocate_sections(&door_object, section_count);

    // The room BEYOND the door (the `nextRoom` argument of
    // expandToDoor) and the element's from-room (the seed's room).
    let from_room_id = engine.ctx.room_id(from_room);
    let to_room_id = door
        .other_room_id(from_room_id)
        .expect("the start door's other room");
    let to_room_key = engine
        .ctx
        .room_key_of_id(to_room_id)
        .expect("the to-room is registered");
    let c = door_shape.centre_of_gravity();
    let from_elem = MazeListElement::new(
        door_object.clone(),
        0,
        None,
        0,
        100.0,
        100.0,
        Some(from_room),
        FloatLine::new(c, c),
        false,
        Adjustment::None,
        false,
    );

    // Section 0 occupied, section 1 free → section 1 EXPANDS. (The
    // init seed itself is door 232558643 section 0, so the assertions
    // run on the front DELTA.)
    engine.maze_element_mut(&door_object, 0).is_occupied = true;
    let before: Vec<MazeListElement> = engine.front.iter().cloned().collect();
    let expanded = engine.expand_to_door(
        door.clone(),
        &door_object,
        &from_elem,
        to_room_key,
        0,
        true,
        Adjustment::None,
    );
    assert!(expanded, "the free section 1 expanded");
    let is_new = |e: &MazeListElement| !before.contains(e);
    assert!(
        !engine
            .front
            .iter()
            .any(|e| is_new(e) && e.door == door_object && e.section_no_of_door == 0),
        "the occupied section 0 was skipped"
    );
    assert!(
        engine
            .front
            .iter()
            .any(|e| is_new(e) && e.door == door_object && e.section_no_of_door == 1),
        "the free section 1 produced an element"
    );

    // Both sections occupied → nothing expands.
    engine.maze_element_mut(&door_object, 1).is_occupied = true;
    let size_before = engine.front.len();
    let expanded = engine.expand_to_door(
        door,
        &door_object,
        &from_elem,
        to_room_key,
        0,
        true,
        Adjustment::None,
    );
    assert!(!expanded, "fully-occupied door expands nothing");
    assert_eq!(engine.front.len(), size_before, "no new elements");
}

/// The M5-T5 row-buffer reuse face: the RAW_SECTION rows build into
/// the engine's reusable buffer and the reuse must be INVISIBLE —
/// three consecutive emissions on ONE engine (assign → skip → the
/// same assign again) must reproduce identical texts for identical
/// inputs, carry their own template prefixes, and leave no residue of
/// a previous row in either direction. The controllable fields are
/// pinned from the world's own literals (section indexes, the
/// add-cost/adjustment/roomRipped inputs); the geometry-derived
/// fields are bounded by prefix+suffix, not re-derived. The row
/// GRAMMAR is independently judged against the Java-captured events
/// golden (`route_events compare`, 3520 rows). Mutants: a missing
/// `clear()` leaves the first assign's stale tail on the skip row
/// (suffix face dies); a take-without-restore is text-invisible —
/// caught only by the capacity witness below; a wrong-template mutant
/// dies on the prefixes.
#[test]
fn raw_row_buffer_reuse_is_invisible() {
    use crate::path::inserter::CaptureSink;

    let mut sink = CaptureSink::default();
    let rows: Vec<String>;
    {
        let (mut harness, mut ctrl, mut distance, mut checker, mut pages) = make_run(false);
        let mut engine = MazeSearchEngine::new(
            &mut harness,
            &mut ctrl,
            &mut distance,
            &mut checker,
            &mut pages,
        );
        assert!(engine.init(&[START_PIN], &[DEST_PIN]), "init ok");
        engine.trace_sink(&mut sink);

        // The precheck world: door 232558643 of the seed's room, two
        // sections. The from element is a HAND-BUILT seed (section
        // fields pinned at 0/0) so the row prefixes are world
        // literals.
        let seed = engine.front.iter().next().expect("the init seed").clone();
        let from_room = seed.next_room_key.expect("the seed's room");
        let door = engine.ctx.room_doors(from_room)[0].clone();
        assert_eq!(door.id(), 232558643, "the start door");
        let door_object = ExpandableObject::RoomDoor(door.clone());
        let (first_key, second_key, first_shape, second_shape) = {
            let ctx = &*engine.ctx;
            let fk = ctx.room_key_of_id(door.first_room_id);
            let sk = ctx.room_key_of_id(door.second_room_id);
            let fs = ctx.room_shape(fk.expect("live first room"));
            let ss = ctx.room_shape(sk.expect("live second room"));
            (fk, sk, fs, ss)
        };
        let both_complete_free_space = first_key
            .is_some_and(|k| engine.ctx.room_is_complete_free_space(k))
            && second_key.is_some_and(|k| engine.ctx.room_is_complete_free_space(k));
        let door_shape = ExpansionDoor::shape_between(&first_shape, &second_shape);
        let layer = engine.ctx.room_layer(from_room);
        let half_width = f64::from(engine.ctrl.compensated_trace_half_width[layer as usize]);
        let (section_count, _) = door.get_section_segments(
            &door_shape,
            both_complete_free_space,
            &first_shape,
            &second_shape,
            half_width,
        );
        assert_eq!(section_count, 2, "the start door has two sections");
        engine.allocate_sections(&door_object, section_count);

        let c = door_shape.centre_of_gravity();
        let from_elem = MazeListElement::new(
            door_object.clone(),
            0,
            None,
            0,
            100.0,
            100.0,
            Some(from_room),
            FloatLine::new(c, c),
            false,
            Adjustment::None,
            false,
        );

        // Rounds drive `expand_to_door_section` DIRECTLY (the row
        // emitters; `expand_to_door`'s own section loop `continue`s
        // over occupied sections BEFORE the section call, so a door-
        // level walk cannot reach the skip arm). The entry segment is
        // the same `FloatLine` for every round so identical inputs
        // must reproduce identical rows.
        // Round 1: section 1 free → one assign row.
        let expanded = engine.expand_to_door_section(
            door_object.clone(),
            1,
            FloatLine::new(c, c),
            &from_elem,
            0,
            Adjustment::None,
        );
        assert!(expanded, "the free section 1 expanded");

        // Round 2: section 1 occupied → one skip row.
        engine.maze_element_mut(&door_object, 1).is_occupied = true;
        let expanded = engine.expand_to_door_section(
            door_object.clone(),
            1,
            FloatLine::new(c, c),
            &from_elem,
            0,
            Adjustment::None,
        );
        assert!(!expanded, "the occupied section 1 must skip");

        // Round 3: section 1 freed again → the SAME assign row as
        // round 1 (identical inputs on the SAME buffer must reproduce
        // the same text byte-for-byte).
        engine.maze_element_mut(&door_object, 1).is_occupied = false;
        let expanded = engine.expand_to_door_section(
            door_object,
            1,
            FloatLine::new(c, c),
            &from_elem,
            0,
            Adjustment::None,
        );
        assert!(expanded, "the re-freed section 1 expanded again");

        // The REUSE face: the buffer must be retained across rows (a
        // take-without-restore leaves the engine's buffer empty and
        // re-allocates per row — text-identical, so only a capacity
        // witness catches it).
        assert!(
            engine.row_buf.capacity() > 0,
            "the row buffer must be retained across rows (a take-without-restore \
             leaves it at capacity 0, re-allocating per row)"
        );

        // The engine holds the sink borrow; the row texts are read
        // only after the engine is dropped.
        drop(engine);
        rows = sink
            .rows
            .iter()
            .filter_map(|r| r.strip_prefix("TRACE ").map(str::to_string))
            .collect();
    }
    assert_eq!(rows.len(), 3, "exactly one RAW row per round: {rows:?}");
    assert!(
        rows[0]
            .starts_with("RAW_SECTION assign selected_section=1, from_section=0, backtrack_section=0, add_costs=0, adjustment=NONE, roomRipped=false, expansionValue="),
        "row 1 template prefix drifted: {}",
        rows[0]
    );
    assert!(
        rows[1]
            .starts_with("RAW_SECTION skip selected_section=1, from_section=0, backtrack_section=0, occupied=true, shape_entry_null=false, adjustment=NONE, door="),
        "row 2 template prefix drifted: {}",
        rows[1]
    );
    assert_eq!(rows[2], rows[0], "row 3 must byte-equal row 1");
    // No stale residue in either direction: every row terminates at
    // its net field exactly once (a missing clear() leaves the
    // previous row's tail beyond the new text).
    for row in &rows {
        assert_eq!(
            row.matches(", net=").count(),
            1,
            "row does not terminate at its net field: {row}"
        );
    }
}

/// A comparator tie element — DIRECT struct literal (not
/// `MazeListElement::new`, whose finiteness debug-asserts would
/// reject the NaN probes).
fn tie_elem(id: i32, section: i32, sort: f64, exp: f64) -> MazeListElement {
    MazeListElement {
        door: ExpandableObject::DrillPage {
            row: 0,
            column: 0,
            id,
        },
        section_no_of_door: section,
        backtrack_door: None,
        section_no_of_backtrack_door: 0,
        expansion_value: exp,
        sorting_value: sort,
        next_room_key: None,
        shape_entry: FloatLine::new(FloatPoint::ZERO, FloatPoint::ZERO),
        room_ripped: false,
        adjustment: Adjustment::None,
        already_checked: false,
        ripup_cost: 0,
    }
}

/// The set order of the probe elements as (id, section) pairs.
/// `mutable_key_type` is sound to allow here: [`FrontOrder`]'s `Ord`
/// reads only plain integers (door id, section, the two values) — the
/// interior mutability clippy sees transitively is `Line`'s atomic id
/// cache inside a `Simplex`-typed `TileShape`, which the comparator
/// never touches.
#[allow(clippy::mutable_key_type)]
fn order_of(set: &BTreeSet<FrontOrder>) -> Vec<(i32, i32)> {
    set.iter()
        .map(|e| (e.0.door.id(), e.0.section_no_of_door))
        .collect()
}

/// The comparator's 4-level tie chain + dedup + NaN/-0.0 (probes
/// T1-T7): the BTreeSet front must replicate Java's
/// `<`/`>`-semantics `TreeSet` exactly. (`mutable_key_type` allowed —
/// see `order_of`.)
#[allow(clippy::mutable_key_type)]
#[test]
fn front_tie_chain_pins() {
    // T1: sortingValue decides (order [31, 7]).
    let mut ts: BTreeSet<FrontOrder> = BTreeSet::new();
    ts.insert(FrontOrder(tie_elem(31, 0, 10.0, 5.0)));
    ts.insert(FrontOrder(tie_elem(7, 0, 20.0, 5.0)));
    assert_eq!(
        order_of(&ts),
        vec![(31, 0), (7, 0)],
        "T1 sort 10 vs 20: order by sortingValue"
    );

    // T2: sort equal → expansionValue decides (order [7, 31]).
    ts.clear();
    ts.insert(FrontOrder(tie_elem(31, 0, 10.0, 6.0)));
    ts.insert(FrontOrder(tie_elem(7, 0, 10.0, 5.0)));
    assert_eq!(
        order_of(&ts),
        vec![(7, 0), (31, 0)],
        "T2 sort equal: order by expansionValue"
    );

    // T3: values equal → door id decides (order [7, 31]).
    ts.clear();
    ts.insert(FrontOrder(tie_elem(31, 0, 10.0, 5.0)));
    ts.insert(FrontOrder(tie_elem(7, 0, 10.0, 5.0)));
    assert_eq!(
        order_of(&ts),
        vec![(7, 0), (31, 0)],
        "T3 values equal: order by doorId"
    );

    // T4: id equal → section decides (order [(31,1), (31,3)]).
    ts.clear();
    ts.insert(FrontOrder(tie_elem(31, 3, 10.0, 5.0)));
    ts.insert(FrontOrder(tie_elem(31, 1, 10.0, 5.0)));
    assert_eq!(
        order_of(&ts),
        vec![(31, 1), (31, 3)],
        "T4 id equal: order by section"
    );

    // T5: full tie → the set DEDUPS (size 1, second add rejected).
    ts.clear();
    ts.insert(FrontOrder(tie_elem(31, 2, 10.0, 5.0)));
    let first = ts.insert(FrontOrder(tie_elem(31, 2, 10.0, 5.0)));
    assert_eq!(ts.len(), 1, "T5 full tie: size");
    assert!(!first, "T5 full tie: second add rejected");

    // T6: NaN falls through the value tie-breaks (pairwise verdicts).
    use std::cmp::Ordering;
    let na = tie_elem(3, 0, f64::NAN, 0.0);
    let nb = tie_elem(9, 0, 0.0, 0.0);
    let nc = tie_elem(1, 0, 1.0, 0.0);
    assert_eq!(compare(&na, &nb), Ordering::Less, "T6-na-nb");
    assert_eq!(compare(&nb, &na), Ordering::Greater, "T6-nb-na");
    assert_eq!(compare(&nb, &nc), Ordering::Less, "T6-nb-nc");
    assert_eq!(
        compare(&na, &tie_elem(3, 0, f64::NAN, 0.0)),
        Ordering::Equal,
        "T6-na-na"
    );

    // T7: -0.0 ties 0.0 (full tie with same id/section → dedup).
    let nz = tie_elem(31, 2, -0.0, 5.0);
    let pz = tie_elem(31, 2, 0.0, 5.0);
    assert_eq!(compare(&nz, &pz), Ordering::Equal, "T7-nz-pz");
    ts.clear();
    ts.insert(FrontOrder(nz));
    let second = ts.insert(FrontOrder(pz));
    assert_eq!(ts.len(), 1, "T7 -0.0 vs 0.0: size");
    assert!(!second, "T7 -0.0 vs 0.0: second add rejected");
}
// ---- T7 literal tables (generated from logs/M3-T7/ripup_capture_1.rows) ----
//
// The M3-T7 jar-capture pins — the ripup resolver + the READ-ONLY shove
// probe replayed against `rust/harness/oracle/RipupSpike.java`. Capture:
// `logs/M3-T7/ripup_capture_{1,2}.rows`, 1126 JSON rows, run TWICE on the
// jar with byte-identical output. Fixture:
// `rust/harness/fixtures/maze-spike/t7_ripup.dsn` (2 layers, NET_A pins at
// x=200000/x=1000000, six net-2 wires 9-17, vias 18-20, net-3 pin 8).
//
// Protocol T7 (both sides reproduce it EXACTLY): per phase a FRESH parse,
// ctrl {ripupAllowed, ripupCosts=seed, ripupPassNo=1, viasAllowed=false,
// removeUnconnectedVias=seed!=2}, `init` from pin 4 TO pin 4 — the spike's
// `pinNearest` picks pin 4 for BOTH x=20000 and x=100000, so the search's
// destination is its own start item and every phase drains the front EMPTY
// (destination never set; the phase_done rows pin exactly that) — then
// k < 600: peek the front head (pop row), probe the first roomRipped
// element (leaving_ripped + small_door_ripped variants), harvest the first
// element parked in an obstacle room (per obstacle id, once), probe it,
// then `occupyNextElement`. The probes mutate ctrl.ripupPassNo through
// PASSES {1,3,4,4,6,7} — the DOUBLE pass-4 is the RNG-lifetime witness.

/// The T7 spike fixture.
const T7_FIXTURE_DSN: &str = include_str!("../../../../harness/fixtures/maze-spike/t7_ripup.dsn");

/// The NET_A start pin (item dump row id 4, x=200000) — ALSO the phase
/// destination: the spike's nearest-x probes pick pin 4 twice.
const T7_PIN: u64 = 4;

/// One drain `pop` row: the front head before each `occupyNextElement`.
#[derive(Clone, Copy, Debug)]
struct T7PopRow {
    k: i32,
    door_id: i32,
    section: i32,
    room_ripped: bool,
}
const fn t7p(k: i32, door_id: i32, section: i32, room_ripped: bool) -> T7PopRow {
    T7PopRow {
        k,
        door_id,
        section,
        room_ripped,
    }
}

/// The SEED2 drain pops, k = 0..72.
const T7_POPS_SEED2: [T7PopRow; 72] = [
    t7p(0, 125, 0, false),
    t7p(1, 33, 3, false),
    t7p(2, 68, 0, false),
    t7p(3, 33, 2, false),
    t7p(4, 14398, 0, false),
    t7p(5, 193, 0, false),
    t7p(6, 194, 0, false),
    t7p(7, 67, 2, false),
    t7p(8, 458753, 0, true),
    t7p(9, 458753, 0, true),
    t7p(10, 458753, 0, true),
    t7p(11, 232, 0, false),
    t7p(12, 264, 0, false),
    t7p(13, 14398, 0, true),
    t7p(14, 13777, 0, false),
    t7p(15, 14554, 0, true),
    t7p(16, 14554, 0, false),
    t7p(17, 14554, 0, true),
    t7p(18, 14708, 0, true),
    t7p(19, 13808, 0, false),
    t7p(20, 14678, 0, true),
    t7p(21, 165, 0, false),
    t7p(22, 14802, 0, true),
    t7p(23, 12784, 0, false),
    t7p(24, 15856, 0, false),
    t7p(25, 14802, 0, true),
    t7p(26, 13808, 0, true),
    t7p(27, 321, 0, false),
    t7p(28, 394240, 0, true),
    t7p(29, 14709, 0, true),
    t7p(30, 361472, 0, true),
    t7p(31, 361472, 0, true),
    t7p(32, 13249, 0, true),
    t7p(33, 16228, 0, true),
    t7p(34, 394240, 0, true),
    t7p(35, 13218, 0, true),
    t7p(36, 394240, 0, true),
    t7p(37, 492544, 0, true),
    t7p(38, 494592, 0, true),
    t7p(39, 526336, 0, true),
    t7p(40, 526336, 0, true),
    t7p(41, 12225, 0, true),
    t7p(42, 17252, 0, true),
    t7p(43, 17252, 0, true),
    t7p(44, 16507, 0, true),
    t7p(45, 13249, 0, true),
    t7p(46, 13249, 0, true),
    t7p(47, 17376, 0, true),
    t7p(48, 17376, 0, true),
    t7p(49, 110, 0, false),
    t7p(50, 133, 0, false),
    t7p(51, 17314, 0, false),
    t7p(52, 17376, 0, true),
    t7p(53, 17252, 0, true),
    t7p(54, 129, 0, false),
    t7p(55, 13218, 0, true),
    t7p(56, 17314, 0, true),
    t7p(57, 17531, 0, true),
    t7p(58, 33, 1, false),
    t7p(59, 17531, 0, true),
    t7p(60, 67, 1, false),
    t7p(61, 34, 1, false),
    t7p(62, 129, 0, false),
    t7p(63, 110, 0, false),
    t7p(64, 133, 0, false),
    t7p(65, 33, 0, false),
    t7p(66, 67, 0, false),
    t7p(67, 34, 2, false),
    t7p(68, 67, 1, false),
    t7p(69, 129, 0, false),
    t7p(70, 110, 0, false),
    t7p(71, 133, 0, false),
];

/// The SEED10 drain pops, k = 0..72.
const T7_POPS_SEED10: [T7PopRow; 72] = [
    t7p(0, 125, 0, false),
    t7p(1, 33, 3, false),
    t7p(2, 68, 0, false),
    t7p(3, 33, 2, false),
    t7p(4, 14398, 0, false),
    t7p(5, 193, 0, false),
    t7p(6, 194, 0, false),
    t7p(7, 67, 2, false),
    t7p(8, 458753, 0, true),
    t7p(9, 458753, 0, true),
    t7p(10, 458753, 0, true),
    t7p(11, 232, 0, false),
    t7p(12, 264, 0, false),
    t7p(13, 14398, 0, true),
    t7p(14, 13777, 0, false),
    t7p(15, 14554, 0, true),
    t7p(16, 14554, 0, false),
    t7p(17, 14554, 0, true),
    t7p(18, 14708, 0, true),
    t7p(19, 13808, 0, false),
    t7p(20, 14678, 0, true),
    t7p(21, 165, 0, false),
    t7p(22, 14802, 0, true),
    t7p(23, 12784, 0, false),
    t7p(24, 15856, 0, false),
    t7p(25, 14802, 0, true),
    t7p(26, 13808, 0, true),
    t7p(27, 321, 0, false),
    t7p(28, 394240, 0, true),
    t7p(29, 14709, 0, true),
    t7p(30, 361472, 0, true),
    t7p(31, 361472, 0, true),
    t7p(32, 13249, 0, true),
    t7p(33, 16228, 0, true),
    t7p(34, 394240, 0, true),
    t7p(35, 13218, 0, true),
    t7p(36, 394240, 0, true),
    t7p(37, 492544, 0, true),
    t7p(38, 494592, 0, true),
    t7p(39, 526336, 0, true),
    t7p(40, 526336, 0, true),
    t7p(41, 12225, 0, true),
    t7p(42, 17252, 0, true),
    t7p(43, 17252, 0, true),
    t7p(44, 16507, 0, true),
    t7p(45, 13249, 0, true),
    t7p(46, 13249, 0, true),
    t7p(47, 17376, 0, true),
    t7p(48, 17376, 0, true),
    t7p(49, 110, 0, false),
    t7p(50, 133, 0, false),
    t7p(51, 17314, 0, false),
    t7p(52, 17376, 0, true),
    t7p(53, 17252, 0, true),
    t7p(54, 129, 0, false),
    t7p(55, 13218, 0, true),
    t7p(56, 17314, 0, true),
    t7p(57, 17531, 0, true),
    t7p(58, 33, 1, false),
    t7p(59, 17531, 0, true),
    t7p(60, 67, 1, false),
    t7p(61, 34, 1, false),
    t7p(62, 129, 0, false),
    t7p(63, 110, 0, false),
    t7p(64, 133, 0, false),
    t7p(65, 33, 0, false),
    t7p(66, 67, 0, false),
    t7p(67, 34, 2, false),
    t7p(68, 67, 1, false),
    t7p(69, 129, 0, false),
    t7p(70, 110, 0, false),
    t7p(71, 133, 0, false),
];

/// The PROBE drain pops, k = 0..72.
const T7_POPS_PROBE: [T7PopRow; 72] = [
    t7p(0, 125, 0, false),
    t7p(1, 33, 3, false),
    t7p(2, 68, 0, false),
    t7p(3, 33, 2, false),
    t7p(4, 14398, 0, false),
    t7p(5, 193, 0, false),
    t7p(6, 194, 0, false),
    t7p(7, 67, 2, false),
    t7p(8, 458753, 0, true),
    t7p(9, 458753, 0, true),
    t7p(10, 458753, 0, true),
    t7p(11, 232, 0, false),
    t7p(12, 264, 0, false),
    t7p(13, 14398, 0, true),
    t7p(14, 13777, 0, false),
    t7p(15, 14554, 0, true),
    t7p(16, 14554, 0, false),
    t7p(17, 14554, 0, true),
    t7p(18, 14708, 0, true),
    t7p(19, 13808, 0, false),
    t7p(20, 14678, 0, true),
    t7p(21, 165, 0, false),
    t7p(22, 14802, 0, true),
    t7p(23, 12784, 0, false),
    t7p(24, 15856, 0, false),
    t7p(25, 14802, 0, true),
    t7p(26, 13808, 0, true),
    t7p(27, 321, 0, false),
    t7p(28, 394240, 0, true),
    t7p(29, 14709, 0, true),
    t7p(30, 361472, 0, true),
    t7p(31, 361472, 0, true),
    t7p(32, 13249, 0, true),
    t7p(33, 16228, 0, true),
    t7p(34, 394240, 0, true),
    t7p(35, 13218, 0, true),
    t7p(36, 394240, 0, true),
    t7p(37, 492544, 0, true),
    t7p(38, 494592, 0, true),
    t7p(39, 526336, 0, true),
    t7p(40, 526336, 0, true),
    t7p(41, 12225, 0, true),
    t7p(42, 17252, 0, true),
    t7p(43, 17252, 0, true),
    t7p(44, 16507, 0, true),
    t7p(45, 13249, 0, true),
    t7p(46, 13249, 0, true),
    t7p(47, 17376, 0, true),
    t7p(48, 17376, 0, true),
    t7p(49, 110, 0, false),
    t7p(50, 133, 0, false),
    t7p(51, 17314, 0, false),
    t7p(52, 17376, 0, true),
    t7p(53, 17252, 0, true),
    t7p(54, 129, 0, false),
    t7p(55, 13218, 0, true),
    t7p(56, 17314, 0, true),
    t7p(57, 17531, 0, true),
    t7p(58, 33, 1, false),
    t7p(59, 17531, 0, true),
    t7p(60, 67, 1, false),
    t7p(61, 34, 1, false),
    t7p(62, 129, 0, false),
    t7p(63, 110, 0, false),
    t7p(64, 133, 0, false),
    t7p(65, 33, 0, false),
    t7p(66, 67, 0, false),
    t7p(67, 34, 2, false),
    t7p(68, 67, 1, false),
    t7p(69, 129, 0, false),
    t7p(70, 110, 0, false),
    t7p(71, 133, 0, false),
];

/// The three phase pop tables (SEED2, SEED10, PROBE).
const T7_POPS: [[T7PopRow; 72]; 3] = [T7_POPS_SEED2, T7_POPS_SEED10, T7_POPS_PROBE];

/// The obstacle kind of a harvest row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum T7Kind {
    Trace,
    Via,
}

/// One `harvest` row: the first front element parked in an obstacle
/// room, per obstacle id, with the RAW element fields.
#[derive(Clone, Copy, Debug)]
struct T7HarvestRow {
    k: i32,
    obstacle: u64,
    kind: T7Kind,
    corner_no: i32,
    door_id: i32,
    section: i32,
    dimension: i32,
    already_checked: bool,
    room_ripped: bool,
}
// Row-literal constructors: every parameter is one capture column.
#[allow(clippy::too_many_arguments)]
const fn t7h(
    k: i32,
    obstacle: u64,
    kind: T7Kind,
    corner_no: i32,
    door_id: i32,
    section: i32,
    dimension: i32,
    already_checked: bool,
    room_ripped: bool,
) -> T7HarvestRow {
    T7HarvestRow {
        k,
        obstacle,
        kind,
        corner_no,
        door_id,
        section,
        dimension,
        already_checked,
        room_ripped,
    }
}

/// The SEED2 harvests (order = probe order).
const T7_HARVESTS_SEED2: [T7HarvestRow; 7] = [
    t7h(2, 14, T7Kind::Trace, 0, 14398, 0, 1, false, false),
    t7h(12, 13, T7Kind::Trace, 0, 13777, 0, 1, false, false),
    t7h(20, 12, T7Kind::Trace, 0, 12784, 0, 1, false, false),
    t7h(24, 15, T7Kind::Trace, 0, 15856, 0, 1, false, false),
    t7h(25, 11, T7Kind::Trace, 0, 362496, 0, 2, false, true),
    t7h(37, 16, T7Kind::Trace, 0, 492544, 0, 2, false, true),
    t7h(38, 18, T7Kind::Via, 0, 494592, 0, 2, false, true),
];

/// The SEED10 harvests (order = probe order).
const T7_HARVESTS_SEED10: [T7HarvestRow; 7] = [
    t7h(2, 14, T7Kind::Trace, 0, 14398, 0, 1, false, false),
    t7h(12, 13, T7Kind::Trace, 0, 13777, 0, 1, false, false),
    t7h(20, 12, T7Kind::Trace, 0, 12784, 0, 1, false, false),
    t7h(24, 15, T7Kind::Trace, 0, 15856, 0, 1, false, false),
    t7h(25, 11, T7Kind::Trace, 0, 362496, 0, 2, false, true),
    t7h(37, 16, T7Kind::Trace, 0, 492544, 0, 2, false, true),
    t7h(38, 18, T7Kind::Via, 0, 494592, 0, 2, false, true),
];

/// The PROBE harvests (order = probe order).
const T7_HARVESTS_PROBE: [T7HarvestRow; 7] = [
    t7h(2, 14, T7Kind::Trace, 0, 14398, 0, 1, false, false),
    t7h(12, 13, T7Kind::Trace, 0, 13777, 0, 1, false, false),
    t7h(20, 12, T7Kind::Trace, 0, 12784, 0, 1, false, false),
    t7h(24, 15, T7Kind::Trace, 0, 15856, 0, 1, false, false),
    t7h(25, 11, T7Kind::Trace, 0, 362496, 0, 2, false, true),
    t7h(37, 16, T7Kind::Trace, 0, 492544, 0, 2, false, true),
    t7h(38, 18, T7Kind::Via, 0, 494592, 0, 2, false, true),
];

const T7_HARVESTS: [[T7HarvestRow; 7]; 3] =
    [T7_HARVESTS_SEED2, T7_HARVESTS_SEED10, T7_HARVESTS_PROBE];

/// The direct-call slot: the six PASSES entries (the two pass-4 rows
/// distinguished by order), the item-9 economics probe, the
/// ALREADY_RIPPED synthetic element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum T7Slot {
    P1,
    P3,
    P4a,
    P4b,
    P6,
    P7,
    Item9,
    Already,
}

/// The CHECK_RIPUP log payload of one direct call (the fields Java
/// logs at MazeRipupResolver.java:173-195; the gate and ALREADY_RIPPED
/// arms return BEFORE the log — `None` here is the log ABSENCE pin).
#[derive(Clone, Copy, Debug)]
struct T7TraceRow {
    items: &'static str,
    half_width: f64,
    ripup_costs: i32,
    trace_length: f64,
    min_trace_length: f64,
    item_count: i32,
    detour: f64,
    result: i32,
}
#[allow(clippy::too_many_arguments)]
const fn t7t(
    items: &'static str,
    half_width: f64,
    ripup_costs: i32,
    trace_length: f64,
    min_trace_length: f64,
    item_count: i32,
    detour: f64,
    result: i32,
) -> T7TraceRow {
    T7TraceRow {
        items,
        half_width,
        ripup_costs,
        trace_length,
        min_trace_length,
        item_count,
        detour,
        result,
    }
}

/// One direct `checkRipup` probe of the replay (marker + optional log
/// row + result row, correlated by stream position).
#[derive(Clone, Copy, Debug)]
struct T7CallRow {
    harvest: u8,
    slot: T7Slot,
    obstacle: u64,
    pass_no: i32,
    cost: i32,
    trace: Option<T7TraceRow>,
}
const fn t7c(
    harvest: u8,
    slot: T7Slot,
    obstacle: u64,
    pass_no: i32,
    cost: i32,
    trace: Option<T7TraceRow>,
) -> T7CallRow {
    T7CallRow {
        harvest,
        slot,
        obstacle,
        pass_no,
        cost,
        trace,
    }
}

const T7_CALLS_SEED2: [T7CallRow; 56] = [
    t7c(
        0,
        T7Slot::P1,
        14,
        1,
        1,
        Some(t7t("[14]", 10000.0, 2, 44000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        0,
        T7Slot::P3,
        14,
        3,
        1,
        Some(t7t("[14]", 10000.0, 2, 44000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        0,
        T7Slot::P4a,
        14,
        4,
        1,
        Some(t7t(
            "[14]",
            10000.0,
            2,
            44000.0,
            0.0,
            1,
            2221734702.386413,
            1,
        )),
    ),
    t7c(
        0,
        T7Slot::P4b,
        14,
        4,
        1,
        Some(t7t(
            "[14]",
            10000.0,
            2,
            44000.0,
            0.0,
            1,
            2818803825.9464397,
            1,
        )),
    ),
    t7c(
        0,
        T7Slot::P6,
        14,
        6,
        1,
        Some(t7t("[14]", 10000.0, 2, 44000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        0,
        T7Slot::P7,
        14,
        7,
        1,
        Some(t7t(
            "[14]",
            10000.0,
            2,
            44000.0,
            0.0,
            1,
            1603810987.4099746,
            1,
        )),
    ),
    t7c(
        0,
        T7Slot::Item9,
        9,
        7,
        21474836,
        Some(t7t(
            "[]",
            10000.0,
            2,
            0.0,
            0.0,
            0,
            1.4719533274872179,
            21474836,
        )),
    ),
    t7c(0, T7Slot::Already, 14, 7, 1, None),
    t7c(
        1,
        T7Slot::P1,
        13,
        1,
        1,
        Some(t7t("[13]", 10000.0, 2, 42000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        1,
        T7Slot::P3,
        13,
        3,
        1,
        Some(t7t("[13]", 10000.0, 2, 42000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        1,
        T7Slot::P4a,
        13,
        4,
        1,
        Some(t7t(
            "[13]",
            10000.0,
            2,
            42000.0,
            0.0,
            1,
            3167538171.484369,
            1,
        )),
    ),
    t7c(
        1,
        T7Slot::P4b,
        13,
        4,
        1,
        Some(t7t(
            "[13]",
            10000.0,
            2,
            42000.0,
            0.0,
            1,
            1185531320.6879826,
            1,
        )),
    ),
    t7c(
        1,
        T7Slot::P6,
        13,
        6,
        1,
        Some(t7t("[13]", 10000.0, 2, 42000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        1,
        T7Slot::P7,
        13,
        7,
        1,
        Some(t7t(
            "[13]",
            10000.0,
            2,
            42000.0,
            0.0,
            1,
            1085755097.6906526,
            1,
        )),
    ),
    t7c(
        1,
        T7Slot::Item9,
        9,
        7,
        21474836,
        Some(t7t(
            "[]",
            10000.0,
            2,
            0.0,
            0.0,
            0,
            1.0522833942555634,
            21474836,
        )),
    ),
    t7c(1, T7Slot::Already, 13, 7, 1, None),
    t7c(
        2,
        T7Slot::P1,
        12,
        1,
        1,
        Some(t7t("[12]", 10000.0, 2, 20000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        2,
        T7Slot::P3,
        12,
        3,
        1,
        Some(t7t("[12]", 10000.0, 2, 20000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        2,
        T7Slot::P4a,
        12,
        4,
        1,
        Some(t7t(
            "[12]",
            10000.0,
            2,
            20000.0,
            0.0,
            1,
            2631477882.7507424,
            1,
        )),
    ),
    t7c(
        2,
        T7Slot::P4b,
        12,
        4,
        1,
        Some(t7t(
            "[12]",
            10000.0,
            2,
            20000.0,
            0.0,
            1,
            1080657232.3598514,
            1,
        )),
    ),
    t7c(
        2,
        T7Slot::P6,
        12,
        6,
        1,
        Some(t7t("[12]", 10000.0, 2, 20000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        2,
        T7Slot::P7,
        12,
        7,
        1,
        Some(t7t(
            "[12]",
            10000.0,
            2,
            20000.0,
            0.0,
            1,
            1431720302.0592868,
            1,
        )),
    ),
    t7c(
        2,
        T7Slot::Item9,
        9,
        7,
        21474836,
        Some(t7t(
            "[]",
            10000.0,
            2,
            0.0,
            0.0,
            0,
            0.6241778607764639,
            21474836,
        )),
    ),
    t7c(2, T7Slot::Already, 12, 7, 1, None),
    t7c(
        3,
        T7Slot::P1,
        15,
        1,
        1,
        Some(t7t("[15]", 10000.0, 2, 36000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        3,
        T7Slot::P3,
        15,
        3,
        1,
        Some(t7t("[15]", 10000.0, 2, 36000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        3,
        T7Slot::P4a,
        15,
        4,
        1,
        Some(t7t(
            "[15]",
            10000.0,
            2,
            36000.0,
            0.0,
            1,
            1751879715.892036,
            1,
        )),
    ),
    t7c(
        3,
        T7Slot::P4b,
        15,
        4,
        1,
        Some(t7t(
            "[15]",
            10000.0,
            2,
            36000.0,
            0.0,
            1,
            1560278473.4162452,
            1,
        )),
    ),
    t7c(
        3,
        T7Slot::P6,
        15,
        6,
        1,
        Some(t7t("[15]", 10000.0, 2, 36000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        3,
        T7Slot::P7,
        15,
        7,
        1,
        Some(t7t(
            "[15]",
            10000.0,
            2,
            36000.0,
            0.0,
            1,
            1078984892.3244832,
            1,
        )),
    ),
    t7c(
        3,
        T7Slot::Item9,
        9,
        7,
        21474836,
        Some(t7t(
            "[]",
            10000.0,
            2,
            0.0,
            0.0,
            0,
            0.8390411106959199,
            21474836,
        )),
    ),
    t7c(3, T7Slot::Already, 15, 7, 1, None),
    t7c(
        4,
        T7Slot::P1,
        11,
        1,
        1,
        Some(t7t("[11]", 10000.0, 2, 18000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        4,
        T7Slot::P3,
        11,
        3,
        1,
        Some(t7t("[11]", 10000.0, 2, 18000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        4,
        T7Slot::P4a,
        11,
        4,
        1,
        Some(t7t(
            "[11]",
            10000.0,
            2,
            18000.0,
            0.0,
            1,
            1186037442.3100867,
            1,
        )),
    ),
    t7c(
        4,
        T7Slot::P4b,
        11,
        4,
        1,
        Some(t7t(
            "[11]",
            10000.0,
            2,
            18000.0,
            0.0,
            1,
            1124134482.6131048,
            1,
        )),
    ),
    t7c(
        4,
        T7Slot::P6,
        11,
        6,
        1,
        Some(t7t("[11]", 10000.0, 2, 18000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        4,
        T7Slot::P7,
        11,
        7,
        1,
        Some(t7t(
            "[11]",
            10000.0,
            2,
            18000.0,
            0.0,
            1,
            3069068038.5423145,
            1,
        )),
    ),
    t7c(
        4,
        T7Slot::Item9,
        9,
        7,
        21474836,
        Some(t7t(
            "[]",
            10000.0,
            2,
            0.0,
            0.0,
            0,
            0.5541146901111209,
            21474836,
        )),
    ),
    t7c(4, T7Slot::Already, 11, 7, 1, None),
    t7c(
        5,
        T7Slot::P1,
        16,
        1,
        1,
        Some(t7t("[16]", 10000.0, 2, 36000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        5,
        T7Slot::P3,
        16,
        3,
        1,
        Some(t7t("[16]", 10000.0, 2, 36000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        5,
        T7Slot::P4a,
        16,
        4,
        1,
        Some(t7t(
            "[16]",
            10000.0,
            2,
            36000.0,
            0.0,
            1,
            2586698508.1818995,
            1,
        )),
    ),
    t7c(
        5,
        T7Slot::P4b,
        16,
        4,
        1,
        Some(t7t(
            "[16]",
            10000.0,
            2,
            36000.0,
            0.0,
            1,
            2093183748.9418864,
            1,
        )),
    ),
    t7c(
        5,
        T7Slot::P6,
        16,
        6,
        1,
        Some(t7t("[16]", 10000.0, 2, 36000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        5,
        T7Slot::P7,
        16,
        7,
        1,
        Some(t7t(
            "[16]",
            10000.0,
            2,
            36000.0,
            0.0,
            1,
            2440093164.138213,
            1,
        )),
    ),
    t7c(
        5,
        T7Slot::Item9,
        9,
        7,
        21474836,
        Some(t7t(
            "[]",
            10000.0,
            2,
            0.0,
            0.0,
            0,
            0.5153076825124037,
            21474836,
        )),
    ),
    t7c(5, T7Slot::Already, 16, 7, 1, None),
    t7c(
        6,
        T7Slot::P1,
        18,
        1,
        1,
        Some(t7t(
            "[18,16,15]",
            5000.0,
            2,
            72000.0,
            0.0,
            3,
            2147483647.0,
            1,
        )),
    ),
    t7c(
        6,
        T7Slot::P3,
        18,
        3,
        1,
        Some(t7t(
            "[18,16,15]",
            5000.0,
            2,
            72000.0,
            0.0,
            3,
            2147483647.0,
            1,
        )),
    ),
    t7c(
        6,
        T7Slot::P4a,
        18,
        4,
        1,
        Some(t7t(
            "[18,16,15]",
            5000.0,
            2,
            72000.0,
            0.0,
            3,
            1076047074.294002,
            1,
        )),
    ),
    t7c(
        6,
        T7Slot::P4b,
        18,
        4,
        1,
        Some(t7t(
            "[18,16,15]",
            5000.0,
            2,
            72000.0,
            0.0,
            3,
            2941751118.5534143,
            1,
        )),
    ),
    t7c(
        6,
        T7Slot::P6,
        18,
        6,
        1,
        Some(t7t(
            "[18,16,15]",
            5000.0,
            2,
            72000.0,
            0.0,
            3,
            2147483647.0,
            1,
        )),
    ),
    t7c(
        6,
        T7Slot::P7,
        18,
        7,
        1,
        Some(t7t(
            "[18,16,15]",
            5000.0,
            2,
            72000.0,
            0.0,
            3,
            1226635374.471176,
            1,
        )),
    ),
    t7c(
        6,
        T7Slot::Item9,
        9,
        7,
        21474836,
        Some(t7t(
            "[]",
            10000.0,
            2,
            0.0,
            0.0,
            0,
            0.8073677176467031,
            21474836,
        )),
    ),
    t7c(6, T7Slot::Already, 18, 7, 1, None),
];

const T7_CALLS_SEED10: [T7CallRow; 49] = [
    t7c(
        0,
        T7Slot::P1,
        14,
        1,
        1,
        Some(t7t("[14]", 10000.0, 10, 44000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        0,
        T7Slot::P3,
        14,
        3,
        1,
        Some(t7t("[14]", 10000.0, 10, 44000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        0,
        T7Slot::P4a,
        14,
        4,
        1,
        Some(t7t(
            "[14]",
            10000.0,
            10,
            44000.0,
            0.0,
            1,
            2219485377.225258,
            1,
        )),
    ),
    t7c(
        0,
        T7Slot::P4b,
        14,
        4,
        1,
        Some(t7t(
            "[14]",
            10000.0,
            10,
            44000.0,
            0.0,
            1,
            1216468480.4490461,
            1,
        )),
    ),
    t7c(
        0,
        T7Slot::P6,
        14,
        6,
        1,
        Some(t7t("[14]", 10000.0, 10, 44000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        0,
        T7Slot::P7,
        14,
        7,
        1,
        Some(t7t(
            "[14]",
            10000.0,
            10,
            44000.0,
            0.0,
            1,
            1081268480.428145,
            1,
        )),
    ),
    t7c(0, T7Slot::Already, 14, 7, 1, None),
    t7c(
        1,
        T7Slot::P1,
        13,
        1,
        1,
        Some(t7t("[13]", 10000.0, 10, 42000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        1,
        T7Slot::P3,
        13,
        3,
        1,
        Some(t7t("[13]", 10000.0, 10, 42000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        1,
        T7Slot::P4a,
        13,
        4,
        1,
        Some(t7t(
            "[13]",
            10000.0,
            10,
            42000.0,
            0.0,
            1,
            2513518403.3252015,
            1,
        )),
    ),
    t7c(
        1,
        T7Slot::P4b,
        13,
        4,
        1,
        Some(t7t(
            "[13]",
            10000.0,
            10,
            42000.0,
            0.0,
            1,
            1368704300.3149726,
            1,
        )),
    ),
    t7c(
        1,
        T7Slot::P6,
        13,
        6,
        1,
        Some(t7t("[13]", 10000.0, 10, 42000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        1,
        T7Slot::P7,
        13,
        7,
        1,
        Some(t7t(
            "[13]",
            10000.0,
            10,
            42000.0,
            0.0,
            1,
            2648320773.6733613,
            1,
        )),
    ),
    t7c(1, T7Slot::Already, 13, 7, 1, None),
    t7c(
        2,
        T7Slot::P1,
        12,
        1,
        1,
        Some(t7t("[12]", 10000.0, 10, 20000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        2,
        T7Slot::P3,
        12,
        3,
        1,
        Some(t7t("[12]", 10000.0, 10, 20000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        2,
        T7Slot::P4a,
        12,
        4,
        1,
        Some(t7t(
            "[12]",
            10000.0,
            10,
            20000.0,
            0.0,
            1,
            1073888914.6037438,
            1,
        )),
    ),
    t7c(
        2,
        T7Slot::P4b,
        12,
        4,
        1,
        Some(t7t(
            "[12]",
            10000.0,
            10,
            20000.0,
            0.0,
            1,
            1490671844.2751544,
            1,
        )),
    ),
    t7c(
        2,
        T7Slot::P6,
        12,
        6,
        1,
        Some(t7t("[12]", 10000.0, 10, 20000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        2,
        T7Slot::P7,
        12,
        7,
        1,
        Some(t7t(
            "[12]",
            10000.0,
            10,
            20000.0,
            0.0,
            1,
            1075850258.0093699,
            1,
        )),
    ),
    t7c(2, T7Slot::Already, 12, 7, 1, None),
    t7c(
        3,
        T7Slot::P1,
        15,
        1,
        1,
        Some(t7t("[15]", 10000.0, 10, 36000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        3,
        T7Slot::P3,
        15,
        3,
        1,
        Some(t7t("[15]", 10000.0, 10, 36000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        3,
        T7Slot::P4a,
        15,
        4,
        1,
        Some(t7t(
            "[15]",
            10000.0,
            10,
            36000.0,
            0.0,
            1,
            3061546402.1365123,
            1,
        )),
    ),
    t7c(
        3,
        T7Slot::P4b,
        15,
        4,
        1,
        Some(t7t(
            "[15]",
            10000.0,
            10,
            36000.0,
            0.0,
            1,
            2450816237.9843135,
            1,
        )),
    ),
    t7c(
        3,
        T7Slot::P6,
        15,
        6,
        1,
        Some(t7t("[15]", 10000.0, 10, 36000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        3,
        T7Slot::P7,
        15,
        7,
        1,
        Some(t7t(
            "[15]",
            10000.0,
            10,
            36000.0,
            0.0,
            1,
            1483662280.815164,
            1,
        )),
    ),
    t7c(3, T7Slot::Already, 15, 7, 1, None),
    t7c(
        4,
        T7Slot::P1,
        11,
        1,
        1,
        Some(t7t("[11]", 10000.0, 10, 18000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        4,
        T7Slot::P3,
        11,
        3,
        1,
        Some(t7t("[11]", 10000.0, 10, 18000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        4,
        T7Slot::P4a,
        11,
        4,
        1,
        Some(t7t(
            "[11]",
            10000.0,
            10,
            18000.0,
            0.0,
            1,
            1173222882.5632799,
            1,
        )),
    ),
    t7c(
        4,
        T7Slot::P4b,
        11,
        4,
        1,
        Some(t7t(
            "[11]",
            10000.0,
            10,
            18000.0,
            0.0,
            1,
            3040572875.9398236,
            1,
        )),
    ),
    t7c(
        4,
        T7Slot::P6,
        11,
        6,
        1,
        Some(t7t("[11]", 10000.0, 10, 18000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        4,
        T7Slot::P7,
        11,
        7,
        1,
        Some(t7t(
            "[11]",
            10000.0,
            10,
            18000.0,
            0.0,
            1,
            1174003463.4172664,
            1,
        )),
    ),
    t7c(4, T7Slot::Already, 11, 7, 1, None),
    t7c(
        5,
        T7Slot::P1,
        16,
        1,
        1,
        Some(t7t("[16]", 10000.0, 10, 36000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        5,
        T7Slot::P3,
        16,
        3,
        1,
        Some(t7t("[16]", 10000.0, 10, 36000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        5,
        T7Slot::P4a,
        16,
        4,
        1,
        Some(t7t(
            "[16]",
            10000.0,
            10,
            36000.0,
            0.0,
            1,
            1379644355.159074,
            1,
        )),
    ),
    t7c(
        5,
        T7Slot::P4b,
        16,
        4,
        1,
        Some(t7t(
            "[16]",
            10000.0,
            10,
            36000.0,
            0.0,
            1,
            2002633512.8731785,
            1,
        )),
    ),
    t7c(
        5,
        T7Slot::P6,
        16,
        6,
        1,
        Some(t7t("[16]", 10000.0, 10, 36000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        5,
        T7Slot::P7,
        16,
        7,
        1,
        Some(t7t(
            "[16]",
            10000.0,
            10,
            36000.0,
            0.0,
            1,
            1215440519.6258812,
            1,
        )),
    ),
    t7c(5, T7Slot::Already, 16, 7, 1, None),
    t7c(
        6,
        T7Slot::P1,
        18,
        1,
        1,
        Some(t7t(
            "[18,16,15]",
            5000.0,
            10,
            72000.0,
            0.0,
            3,
            2147483647.0,
            1,
        )),
    ),
    t7c(
        6,
        T7Slot::P3,
        18,
        3,
        1,
        Some(t7t(
            "[18,16,15]",
            5000.0,
            10,
            72000.0,
            0.0,
            3,
            2147483647.0,
            1,
        )),
    ),
    t7c(
        6,
        T7Slot::P4a,
        18,
        4,
        1,
        Some(t7t(
            "[18,16,15]",
            5000.0,
            10,
            72000.0,
            0.0,
            3,
            2423852202.2859435,
            1,
        )),
    ),
    t7c(
        6,
        T7Slot::P4b,
        18,
        4,
        1,
        Some(t7t(
            "[18,16,15]",
            5000.0,
            10,
            72000.0,
            0.0,
            3,
            2497247948.2884383,
            1,
        )),
    ),
    t7c(
        6,
        T7Slot::P6,
        18,
        6,
        1,
        Some(t7t(
            "[18,16,15]",
            5000.0,
            10,
            72000.0,
            0.0,
            3,
            2147483647.0,
            1,
        )),
    ),
    t7c(
        6,
        T7Slot::P7,
        18,
        7,
        1,
        Some(t7t(
            "[18,16,15]",
            5000.0,
            10,
            72000.0,
            0.0,
            3,
            2615498250.3709273,
            1,
        )),
    ),
    t7c(6, T7Slot::Already, 18, 7, 1, None),
];

const T7_CALLS_PROBE: [T7CallRow; 56] = [
    t7c(
        0,
        T7Slot::P1,
        14,
        1,
        1,
        Some(t7t("[14]", 10000.0, 1000, 44000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        0,
        T7Slot::P3,
        14,
        3,
        1,
        Some(t7t("[14]", 10000.0, 1000, 44000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        0,
        T7Slot::P4a,
        14,
        4,
        1,
        Some(t7t(
            "[14]",
            10000.0,
            1000,
            44000.0,
            0.0,
            1,
            2156852259.5613413,
            1,
        )),
    ),
    t7c(
        0,
        T7Slot::P4b,
        14,
        4,
        1,
        Some(t7t(
            "[14]",
            10000.0,
            1000,
            44000.0,
            0.0,
            1,
            1783349511.7003453,
            1,
        )),
    ),
    t7c(
        0,
        T7Slot::P6,
        14,
        6,
        1,
        Some(t7t("[14]", 10000.0, 1000, 44000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        0,
        T7Slot::P7,
        14,
        7,
        1,
        Some(t7t(
            "[14]",
            10000.0,
            1000,
            44000.0,
            0.0,
            1,
            2997262940.8184934,
            1,
        )),
    ),
    t7c(
        0,
        T7Slot::Item9,
        9,
        7,
        1,
        Some(t7t(
            "[9]",
            10000.0,
            1000,
            24000.0,
            0.0,
            1,
            1077076498.8891563,
            1,
        )),
    ),
    t7c(0, T7Slot::Already, 14, 7, 1, None),
    t7c(
        1,
        T7Slot::P1,
        13,
        1,
        1,
        Some(t7t("[13]", 10000.0, 1000, 42000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        1,
        T7Slot::P3,
        13,
        3,
        1,
        Some(t7t("[13]", 10000.0, 1000, 42000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        1,
        T7Slot::P4a,
        13,
        4,
        1,
        Some(t7t(
            "[13]",
            10000.0,
            1000,
            42000.0,
            0.0,
            1,
            1500406534.0383437,
            1,
        )),
    ),
    t7c(
        1,
        T7Slot::P4b,
        13,
        4,
        1,
        Some(t7t(
            "[13]",
            10000.0,
            1000,
            42000.0,
            0.0,
            1,
            1848935190.3786967,
            1,
        )),
    ),
    t7c(
        1,
        T7Slot::P6,
        13,
        6,
        1,
        Some(t7t("[13]", 10000.0, 1000, 42000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        1,
        T7Slot::P7,
        13,
        7,
        1,
        Some(t7t(
            "[13]",
            10000.0,
            1000,
            42000.0,
            0.0,
            1,
            1724244530.427738,
            1,
        )),
    ),
    t7c(
        1,
        T7Slot::Item9,
        9,
        7,
        1,
        Some(t7t(
            "[9]",
            10000.0,
            1000,
            24000.0,
            0.0,
            1,
            2003689956.2903693,
            1,
        )),
    ),
    t7c(1, T7Slot::Already, 13, 7, 1, None),
    t7c(
        2,
        T7Slot::P1,
        12,
        1,
        1,
        Some(t7t("[12]", 10000.0, 1000, 20000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        2,
        T7Slot::P3,
        12,
        3,
        1,
        Some(t7t("[12]", 10000.0, 1000, 20000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        2,
        T7Slot::P4a,
        12,
        4,
        1,
        Some(t7t(
            "[12]",
            10000.0,
            1000,
            20000.0,
            0.0,
            1,
            1346288151.3147933,
            1,
        )),
    ),
    t7c(
        2,
        T7Slot::P4b,
        12,
        4,
        1,
        Some(t7t(
            "[12]",
            10000.0,
            1000,
            20000.0,
            0.0,
            1,
            1113574222.6839988,
            1,
        )),
    ),
    t7c(
        2,
        T7Slot::P6,
        12,
        6,
        1,
        Some(t7t("[12]", 10000.0, 1000, 20000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        2,
        T7Slot::P7,
        12,
        7,
        1,
        Some(t7t(
            "[12]",
            10000.0,
            1000,
            20000.0,
            0.0,
            1,
            1866235645.241365,
            1,
        )),
    ),
    t7c(
        2,
        T7Slot::Item9,
        9,
        7,
        1,
        Some(t7t(
            "[9]",
            10000.0,
            1000,
            24000.0,
            0.0,
            1,
            3085409176.7914524,
            1,
        )),
    ),
    t7c(2, T7Slot::Already, 12, 7, 1, None),
    t7c(
        3,
        T7Slot::P1,
        15,
        1,
        1,
        Some(t7t("[15]", 10000.0, 1000, 36000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        3,
        T7Slot::P3,
        15,
        3,
        1,
        Some(t7t("[15]", 10000.0, 1000, 36000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        3,
        T7Slot::P4a,
        15,
        4,
        1,
        Some(t7t(
            "[15]",
            10000.0,
            1000,
            36000.0,
            0.0,
            1,
            3038316210.6568184,
            1,
        )),
    ),
    t7c(
        3,
        T7Slot::P4b,
        15,
        4,
        1,
        Some(t7t(
            "[15]",
            10000.0,
            1000,
            36000.0,
            0.0,
            1,
            2621467771.072557,
            1,
        )),
    ),
    t7c(
        3,
        T7Slot::P6,
        15,
        6,
        1,
        Some(t7t("[15]", 10000.0, 1000, 36000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        3,
        T7Slot::P7,
        15,
        7,
        1,
        Some(t7t(
            "[15]",
            10000.0,
            1000,
            36000.0,
            0.0,
            1,
            1093866127.55231,
            1,
        )),
    ),
    t7c(
        3,
        T7Slot::Item9,
        9,
        7,
        1,
        Some(t7t(
            "[9]",
            10000.0,
            1000,
            24000.0,
            0.0,
            1,
            1369274755.2317076,
            1,
        )),
    ),
    t7c(3, T7Slot::Already, 15, 7, 1, None),
    t7c(
        4,
        T7Slot::P1,
        11,
        1,
        1,
        Some(t7t("[11]", 10000.0, 1000, 18000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        4,
        T7Slot::P3,
        11,
        3,
        1,
        Some(t7t("[11]", 10000.0, 1000, 18000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        4,
        T7Slot::P4a,
        11,
        4,
        1,
        Some(t7t(
            "[11]",
            10000.0,
            1000,
            18000.0,
            0.0,
            1,
            2393966447.503216,
            1,
        )),
    ),
    t7c(
        4,
        T7Slot::P4b,
        11,
        4,
        1,
        Some(t7t(
            "[11]",
            10000.0,
            1000,
            18000.0,
            0.0,
            1,
            1073924844.1267772,
            1,
        )),
    ),
    t7c(
        4,
        T7Slot::P6,
        11,
        6,
        1,
        Some(t7t("[11]", 10000.0, 1000, 18000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        4,
        T7Slot::P7,
        11,
        7,
        1,
        Some(t7t(
            "[11]",
            10000.0,
            1000,
            18000.0,
            0.0,
            1,
            2478442681.599554,
            1,
        )),
    ),
    t7c(
        4,
        T7Slot::Item9,
        9,
        7,
        1,
        Some(t7t(
            "[9]",
            10000.0,
            1000,
            24000.0,
            0.0,
            1,
            1132631066.246659,
            1,
        )),
    ),
    t7c(4, T7Slot::Already, 11, 7, 1, None),
    t7c(
        5,
        T7Slot::P1,
        16,
        1,
        1,
        Some(t7t("[16]", 10000.0, 1000, 36000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        5,
        T7Slot::P3,
        16,
        3,
        1,
        Some(t7t("[16]", 10000.0, 1000, 36000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        5,
        T7Slot::P4a,
        16,
        4,
        1,
        Some(t7t(
            "[16]",
            10000.0,
            1000,
            36000.0,
            0.0,
            1,
            1340625708.0451999,
            1,
        )),
    ),
    t7c(
        5,
        T7Slot::P4b,
        16,
        4,
        1,
        Some(t7t(
            "[16]",
            10000.0,
            1000,
            36000.0,
            0.0,
            1,
            1770806412.431135,
            1,
        )),
    ),
    t7c(
        5,
        T7Slot::P6,
        16,
        6,
        1,
        Some(t7t("[16]", 10000.0, 1000, 36000.0, 0.0, 1, 2147483647.0, 1)),
    ),
    t7c(
        5,
        T7Slot::P7,
        16,
        7,
        1,
        Some(t7t(
            "[16]",
            10000.0,
            1000,
            36000.0,
            0.0,
            1,
            3168099938.166746,
            1,
        )),
    ),
    t7c(
        5,
        T7Slot::Item9,
        9,
        7,
        1,
        Some(t7t(
            "[9]",
            10000.0,
            1000,
            24000.0,
            0.0,
            1,
            1275638343.727627,
            1,
        )),
    ),
    t7c(5, T7Slot::Already, 16, 7, 1, None),
    t7c(
        6,
        T7Slot::P1,
        18,
        1,
        1,
        Some(t7t(
            "[18,16,15]",
            5000.0,
            1000,
            72000.0,
            0.0,
            3,
            2147483647.0,
            1,
        )),
    ),
    t7c(
        6,
        T7Slot::P3,
        18,
        3,
        1,
        Some(t7t(
            "[18,16,15]",
            5000.0,
            1000,
            72000.0,
            0.0,
            3,
            2147483647.0,
            1,
        )),
    ),
    t7c(
        6,
        T7Slot::P4a,
        18,
        4,
        1,
        Some(t7t(
            "[18,16,15]",
            5000.0,
            1000,
            72000.0,
            0.0,
            3,
            1360571789.5834382,
            1,
        )),
    ),
    t7c(
        6,
        T7Slot::P4b,
        18,
        4,
        1,
        Some(t7t(
            "[18,16,15]",
            5000.0,
            1000,
            72000.0,
            0.0,
            3,
            1316107546.526542,
            1,
        )),
    ),
    t7c(
        6,
        T7Slot::P6,
        18,
        6,
        1,
        Some(t7t(
            "[18,16,15]",
            5000.0,
            1000,
            72000.0,
            0.0,
            3,
            2147483647.0,
            1,
        )),
    ),
    t7c(
        6,
        T7Slot::P7,
        18,
        7,
        1,
        Some(t7t(
            "[18,16,15]",
            5000.0,
            1000,
            72000.0,
            0.0,
            3,
            1440711189.9355729,
            1,
        )),
    ),
    t7c(
        6,
        T7Slot::Item9,
        9,
        7,
        1,
        Some(t7t(
            "[9]",
            10000.0,
            1000,
            24000.0,
            0.0,
            1,
            1516159582.4162734,
            1,
        )),
    ),
    t7c(6, T7Slot::Already, 18, 7, 1, None),
];

const T7_CALLS: [&[T7CallRow]; 3] = [&T7_CALLS_SEED2, &T7_CALLS_SEED10, &T7_CALLS_PROBE];

/// The three surgical gate probes per harvest (items 17/19/20, all
/// `-1` through the FIRST gate — the drain never reaches these arms).
const T7_GATES: [[[u64; 3]; 7]; 3] = [
    [
        [17, 19, 20],
        [17, 19, 20],
        [17, 19, 20],
        [17, 19, 20],
        [17, 19, 20],
        [17, 19, 20],
        [17, 19, 20],
    ],
    [
        [17, 19, 20],
        [17, 19, 20],
        [17, 19, 20],
        [17, 19, 20],
        [17, 19, 20],
        [17, 19, 20],
        [17, 19, 20],
    ],
    [
        [17, 19, 20],
        [17, 19, 20],
        [17, 19, 20],
        [17, 19, 20],
        [17, 19, 20],
        [17, 19, 20],
        [17, 19, 20],
    ],
];

/// One `leaving` row (the dim-1 harvests, in order):
/// `checkLeavingRippedItem` on the HARVEST element.
const T7_LEAVING: [[bool; 4]; 3] = [
    [false, false, false, false],
    [false, false, false, false],
    [false, false, false, false],
];

/// One `small_door` row: `enterThroughSmallDoor` on the harvest
/// element (ignore = the obstacle, then the destination pin 4),
/// with the Java `checkRadius` literal.
const T7_SMALL_DOOR: [[(u64, bool, f64); 8]; 3] = [
    [
        (14, true, 11252.0),
        (4, false, 11252.0),
        (13, false, 11252.0),
        (4, false, 11252.0),
        (12, true, 11252.0),
        (4, false, 11252.0),
        (15, true, 11252.0),
        (4, false, 11252.0),
    ],
    [
        (14, true, 11252.0),
        (4, false, 11252.0),
        (13, false, 11252.0),
        (4, false, 11252.0),
        (12, true, 11252.0),
        (4, false, 11252.0),
        (15, true, 11252.0),
        (4, false, 11252.0),
    ],
    [
        (14, true, 11252.0),
        (4, false, 11252.0),
        (13, false, 11252.0),
        (4, false, 11252.0),
        (12, true, 11252.0),
        (4, false, 11252.0),
        (15, true, 11252.0),
        (4, false, 11252.0),
    ],
];

/// The small-door check derivation (SEED2 harvest 0, first probe):
/// the element door shape's border corners are the capture's
/// `doorCorners` literal array.
const T7_DOOR_CORNERS: [(f64, f64); 8] = [
    (536750.0, 289340.0),
    (536750.0, 289340.0),
    (536750.0, 289340.0),
    (536750.0, 314660.0),
    (536750.0, 314660.0),
    (536750.0, 314660.0),
    (536750.0, 314660.0),
    (536750.0, 289340.0),
];

/// One `leaving_ripped` row: the :506 leaving path on the first
/// roomRipped element of the front (per door:section, once).
#[derive(Clone, Copy, Debug)]
struct T7LeavingRippedRow {
    k: i32,
    door_id: i32,
    section: i32,
    verdict: bool,
}
const fn t7lr(k: i32, door_id: i32, section: i32, verdict: bool) -> T7LeavingRippedRow {
    T7LeavingRippedRow {
        k,
        door_id,
        section,
        verdict,
    }
}

const T7_LEAVING_RIPPED_SEED2: [T7LeavingRippedRow; 22] = [
    t7lr(5, 458753, 0, false),
    t7lr(11, 14398, 0, true),
    t7lr(14, 14554, 0, false),
    t7lr(18, 14708, 0, true),
    t7lr(19, 14678, 0, false),
    t7lr(21, 14802, 0, false),
    t7lr(26, 13808, 0, true),
    t7lr(27, 394240, 0, false),
    t7lr(29, 14709, 0, true),
    t7lr(30, 361472, 0, false),
    t7lr(32, 13249, 0, true),
    t7lr(33, 16228, 0, false),
    t7lr(35, 13218, 0, true),
    t7lr(37, 492544, 0, false),
    t7lr(38, 494592, 0, false),
    t7lr(39, 526336, 0, false),
    t7lr(41, 12225, 0, true),
    t7lr(42, 17252, 0, false),
    t7lr(44, 16507, 0, false),
    t7lr(47, 17376, 0, false),
    t7lr(56, 17314, 0, true),
    t7lr(57, 17531, 0, false),
];

const T7_LEAVING_RIPPED_SEED10: [T7LeavingRippedRow; 22] = [
    t7lr(5, 458753, 0, false),
    t7lr(11, 14398, 0, true),
    t7lr(14, 14554, 0, false),
    t7lr(18, 14708, 0, true),
    t7lr(19, 14678, 0, false),
    t7lr(21, 14802, 0, false),
    t7lr(26, 13808, 0, true),
    t7lr(27, 394240, 0, false),
    t7lr(29, 14709, 0, true),
    t7lr(30, 361472, 0, false),
    t7lr(32, 13249, 0, true),
    t7lr(33, 16228, 0, false),
    t7lr(35, 13218, 0, true),
    t7lr(37, 492544, 0, false),
    t7lr(38, 494592, 0, false),
    t7lr(39, 526336, 0, false),
    t7lr(41, 12225, 0, true),
    t7lr(42, 17252, 0, false),
    t7lr(44, 16507, 0, false),
    t7lr(47, 17376, 0, false),
    t7lr(56, 17314, 0, true),
    t7lr(57, 17531, 0, false),
];

const T7_LEAVING_RIPPED_PROBE: [T7LeavingRippedRow; 22] = [
    t7lr(5, 458753, 0, false),
    t7lr(11, 14398, 0, true),
    t7lr(14, 14554, 0, false),
    t7lr(18, 14708, 0, true),
    t7lr(19, 14678, 0, false),
    t7lr(21, 14802, 0, false),
    t7lr(26, 13808, 0, true),
    t7lr(27, 394240, 0, false),
    t7lr(29, 14709, 0, true),
    t7lr(30, 361472, 0, false),
    t7lr(32, 13249, 0, true),
    t7lr(33, 16228, 0, false),
    t7lr(35, 13218, 0, true),
    t7lr(37, 492544, 0, false),
    t7lr(38, 494592, 0, false),
    t7lr(39, 526336, 0, false),
    t7lr(41, 12225, 0, true),
    t7lr(42, 17252, 0, false),
    t7lr(44, 16507, 0, false),
    t7lr(47, 17376, 0, false),
    t7lr(56, 17314, 0, true),
    t7lr(57, 17531, 0, false),
];

const T7_LEAVING_RIPPED: [[T7LeavingRippedRow; 22]; 3] = [
    T7_LEAVING_RIPPED_SEED2,
    T7_LEAVING_RIPPED_SEED10,
    T7_LEAVING_RIPPED_PROBE,
];

/// One `small_door_ripped` row: the ignore variants of the ripped
/// element (destination pin 4, then the room being left's obstacle).
#[derive(Clone, Copy, Debug)]
struct T7SmallDoorRippedRow {
    door_id: i32,
    section: i32,
    ignore_id: u64,
    verdict: bool,
}
const fn t7sr(door_id: i32, section: i32, ignore_id: u64, verdict: bool) -> T7SmallDoorRippedRow {
    T7SmallDoorRippedRow {
        door_id,
        section,
        ignore_id,
        verdict,
    }
}

const T7_SMALL_DOOR_RIPPED_SEED2: [T7SmallDoorRippedRow; 44] = [
    t7sr(458753, 0, 4, false),
    t7sr(458753, 0, 14, false),
    t7sr(14398, 0, 4, false),
    t7sr(14398, 0, 14, true),
    t7sr(14554, 0, 4, false),
    t7sr(14554, 0, 14, false),
    t7sr(14708, 0, 4, false),
    t7sr(14708, 0, 14, true),
    t7sr(14678, 0, 4, false),
    t7sr(14678, 0, 14, false),
    t7sr(14802, 0, 4, false),
    t7sr(14802, 0, 14, false),
    t7sr(13808, 0, 4, false),
    t7sr(13808, 0, 13, true),
    t7sr(394240, 0, 4, false),
    t7sr(394240, 0, 13, false),
    t7sr(14709, 0, 4, false),
    t7sr(14709, 0, 14, true),
    t7sr(361472, 0, 4, false),
    t7sr(361472, 0, 11, false),
    t7sr(13249, 0, 4, false),
    t7sr(13249, 0, 12, true),
    t7sr(16228, 0, 4, false),
    t7sr(16228, 0, 15, false),
    t7sr(13218, 0, 4, false),
    t7sr(13218, 0, 12, true),
    t7sr(492544, 0, 4, false),
    t7sr(492544, 0, 15, false),
    t7sr(494592, 0, 4, false),
    t7sr(494592, 0, 15, false),
    t7sr(526336, 0, 4, false),
    t7sr(526336, 0, 18, false),
    t7sr(12225, 0, 4, false),
    t7sr(12225, 0, 11, true),
    t7sr(17252, 0, 4, false),
    t7sr(17252, 0, 16, false),
    t7sr(16507, 0, 4, false),
    t7sr(16507, 0, 15, false),
    t7sr(17376, 0, 4, false),
    t7sr(17376, 0, 16, false),
    t7sr(17314, 0, 4, false),
    t7sr(17314, 0, 16, true),
    t7sr(17531, 0, 4, false),
    t7sr(17531, 0, 16, false),
];

const T7_SMALL_DOOR_RIPPED_SEED10: [T7SmallDoorRippedRow; 44] = [
    t7sr(458753, 0, 4, false),
    t7sr(458753, 0, 14, false),
    t7sr(14398, 0, 4, false),
    t7sr(14398, 0, 14, true),
    t7sr(14554, 0, 4, false),
    t7sr(14554, 0, 14, false),
    t7sr(14708, 0, 4, false),
    t7sr(14708, 0, 14, true),
    t7sr(14678, 0, 4, false),
    t7sr(14678, 0, 14, false),
    t7sr(14802, 0, 4, false),
    t7sr(14802, 0, 14, false),
    t7sr(13808, 0, 4, false),
    t7sr(13808, 0, 13, true),
    t7sr(394240, 0, 4, false),
    t7sr(394240, 0, 13, false),
    t7sr(14709, 0, 4, false),
    t7sr(14709, 0, 14, true),
    t7sr(361472, 0, 4, false),
    t7sr(361472, 0, 11, false),
    t7sr(13249, 0, 4, false),
    t7sr(13249, 0, 12, true),
    t7sr(16228, 0, 4, false),
    t7sr(16228, 0, 15, false),
    t7sr(13218, 0, 4, false),
    t7sr(13218, 0, 12, true),
    t7sr(492544, 0, 4, false),
    t7sr(492544, 0, 15, false),
    t7sr(494592, 0, 4, false),
    t7sr(494592, 0, 15, false),
    t7sr(526336, 0, 4, false),
    t7sr(526336, 0, 18, false),
    t7sr(12225, 0, 4, false),
    t7sr(12225, 0, 11, true),
    t7sr(17252, 0, 4, false),
    t7sr(17252, 0, 16, false),
    t7sr(16507, 0, 4, false),
    t7sr(16507, 0, 15, false),
    t7sr(17376, 0, 4, false),
    t7sr(17376, 0, 16, false),
    t7sr(17314, 0, 4, false),
    t7sr(17314, 0, 16, true),
    t7sr(17531, 0, 4, false),
    t7sr(17531, 0, 16, false),
];

const T7_SMALL_DOOR_RIPPED_PROBE: [T7SmallDoorRippedRow; 44] = [
    t7sr(458753, 0, 4, false),
    t7sr(458753, 0, 14, false),
    t7sr(14398, 0, 4, false),
    t7sr(14398, 0, 14, true),
    t7sr(14554, 0, 4, false),
    t7sr(14554, 0, 14, false),
    t7sr(14708, 0, 4, false),
    t7sr(14708, 0, 14, true),
    t7sr(14678, 0, 4, false),
    t7sr(14678, 0, 14, false),
    t7sr(14802, 0, 4, false),
    t7sr(14802, 0, 14, false),
    t7sr(13808, 0, 4, false),
    t7sr(13808, 0, 13, true),
    t7sr(394240, 0, 4, false),
    t7sr(394240, 0, 13, false),
    t7sr(14709, 0, 4, false),
    t7sr(14709, 0, 14, true),
    t7sr(361472, 0, 4, false),
    t7sr(361472, 0, 11, false),
    t7sr(13249, 0, 4, false),
    t7sr(13249, 0, 12, true),
    t7sr(16228, 0, 4, false),
    t7sr(16228, 0, 15, false),
    t7sr(13218, 0, 4, false),
    t7sr(13218, 0, 12, true),
    t7sr(492544, 0, 4, false),
    t7sr(492544, 0, 15, false),
    t7sr(494592, 0, 4, false),
    t7sr(494592, 0, 15, false),
    t7sr(526336, 0, 4, false),
    t7sr(526336, 0, 18, false),
    t7sr(12225, 0, 4, false),
    t7sr(12225, 0, 11, true),
    t7sr(17252, 0, 4, false),
    t7sr(17252, 0, 16, false),
    t7sr(16507, 0, 4, false),
    t7sr(16507, 0, 15, false),
    t7sr(17376, 0, 4, false),
    t7sr(17376, 0, 16, false),
    t7sr(17314, 0, 4, false),
    t7sr(17314, 0, 16, true),
    t7sr(17531, 0, 4, false),
    t7sr(17531, 0, 16, false),
];

const T7_SMALL_DOOR_RIPPED: [[T7SmallDoorRippedRow; 44]; 3] = [
    T7_SMALL_DOOR_RIPPED_SEED2,
    T7_SMALL_DOOR_RIPPED_SEED10,
    T7_SMALL_DOOR_RIPPED_PROBE,
];

/// One `shove` row: the READ-ONLY `checkShoveTraceLine` probe on a
/// trace harvest (both directions), with the raw gate inputs. Every
/// captured row returned an EMPTY door list (toDoors 0).
#[derive(Clone, Copy, Debug)]
struct T7ShoveRow {
    left: bool,
    verdict: bool,
    corner_no: i32,
    lines: usize,
    half_width: i32,
    ctrl_half_width: i32,
    door_max_width: f64,
}
const fn t7s(
    left: bool,
    verdict: bool,
    corner_no: i32,
    lines: usize,
    half_width: i32,
    ctrl_half_width: i32,
    door_max_width: f64,
) -> T7ShoveRow {
    T7ShoveRow {
        left,
        verdict,
        corner_no,
        lines,
        half_width,
        ctrl_half_width,
        door_max_width,
    }
}

const T7_SHOVE_SEED2: [T7ShoveRow; 12] = [
    t7s(false, true, 0, 4, 10000, 10000, 25320.0),
    t7s(true, true, 0, 4, 10000, 10000, 25320.0),
    t7s(false, false, 0, 3, 10000, 10000, 51320.0),
    t7s(true, false, 0, 3, 10000, 10000, 51320.0),
    t7s(false, true, 0, 3, 10000, 10000, 13410.0),
    t7s(true, true, 0, 3, 10000, 10000, 13410.0),
    t7s(false, true, 0, 3, 10000, 10000, 34410.0),
    t7s(true, true, 0, 3, 10000, 10000, 34410.0),
    t7s(false, true, 0, 3, 10000, 10000, 22500.13777735594),
    t7s(true, true, 0, 3, 10000, 10000, 22500.13777735594),
    t7s(false, true, 0, 3, 10000, 10000, 22500.13777735594),
    t7s(true, true, 0, 3, 10000, 10000, 22500.13777735594),
];

const T7_SHOVE_SEED10: [T7ShoveRow; 12] = [
    t7s(false, true, 0, 4, 10000, 10000, 25320.0),
    t7s(true, true, 0, 4, 10000, 10000, 25320.0),
    t7s(false, false, 0, 3, 10000, 10000, 51320.0),
    t7s(true, false, 0, 3, 10000, 10000, 51320.0),
    t7s(false, true, 0, 3, 10000, 10000, 13410.0),
    t7s(true, true, 0, 3, 10000, 10000, 13410.0),
    t7s(false, true, 0, 3, 10000, 10000, 34410.0),
    t7s(true, true, 0, 3, 10000, 10000, 34410.0),
    t7s(false, true, 0, 3, 10000, 10000, 22500.13777735594),
    t7s(true, true, 0, 3, 10000, 10000, 22500.13777735594),
    t7s(false, true, 0, 3, 10000, 10000, 22500.13777735594),
    t7s(true, true, 0, 3, 10000, 10000, 22500.13777735594),
];

const T7_SHOVE_PROBE: [T7ShoveRow; 12] = [
    t7s(false, true, 0, 4, 10000, 10000, 25320.0),
    t7s(true, true, 0, 4, 10000, 10000, 25320.0),
    t7s(false, false, 0, 3, 10000, 10000, 51320.0),
    t7s(true, false, 0, 3, 10000, 10000, 51320.0),
    t7s(false, true, 0, 3, 10000, 10000, 13410.0),
    t7s(true, true, 0, 3, 10000, 10000, 13410.0),
    t7s(false, true, 0, 3, 10000, 10000, 34410.0),
    t7s(true, true, 0, 3, 10000, 10000, 34410.0),
    t7s(false, true, 0, 3, 10000, 10000, 22500.13777735594),
    t7s(true, true, 0, 3, 10000, 10000, 22500.13777735594),
    t7s(false, true, 0, 3, 10000, 10000, 22500.13777735594),
    t7s(true, true, 0, 3, 10000, 10000, 22500.13777735594),
];

const T7_SHOVE: [[T7ShoveRow; 12]; 3] = [T7_SHOVE_SEED2, T7_SHOVE_SEED10, T7_SHOVE_PROBE];

/// The `shove_nontrace` row (the via harvest): the kind gate answers
/// TRUE (nothing to shove, do not delay occupation).
const T7_SHOVE_NONTRACE: [bool; 3] = [true, true, true];

/// The `shove_stale` row for index 5 (both captured stale rows answer
/// FALSE through the stale gate; see the test for the -1 asymmetry).
const T7_SHOVE_STALE: [bool; 3] = [false, false, false];

/// The `shove_halfwidth_gate` row: ctrl.traceHalfWidth forced to 999
/// flips the same-width gate -> TRUE.
const T7_SHOVE_HW_GATE: [bool; 3] = [true, true, true];

/// The raw `java.util.Random(seed)` probe rows (the FIRST five
/// `nextDouble` of a FRESH generator — the JavaRandom port pin).
const T7_RNG: [[f64; 5]; 3] = [
    [
        0.7311469360199058,
        0.9014476240300544,
        0.49682259343089075,
        0.9858769332362016,
        0.8571240443456863,
    ],
    [
        0.7304302967434272,
        0.2578027905957804,
        0.059201965811244595,
        0.24411725056425315,
        0.8188090228552316,
    ],
    [
        0.7101849056320707,
        0.574836350385667,
        0.9464192094792073,
        0.039405954311386604,
        0.4864098780914311,
    ],
];

/// The phase ctrl rows: (seed, removeUnconnectedVias) per phase; the
/// resolved tables are asserted in `t7_assert_ctrl`.
const T7_PHASES: [(i32, bool); 3] = [(2, false), (10, true), (1000, true)];

// ---- the T7 replay rig ----

use super::ripup::RipupTrace;
use super::shove_probe::DoorSection;

/// One T7 phase rig — the capture's per-phase fresh parse with the
/// ctrl mutations of RipupSpike.main: `ripupAllowed`, the seed as
/// `ripupCosts`, `ripupPassNo` 1, vias OFF, fanout protection ON only
/// for SEED2 (`removeUnconnectedVias = seed != 2`).
fn t7_make_run(
    seed: i32,
    remove_unconnected_vias: bool,
) -> (
    Harness,
    AutorouteControl,
    DestinationDistance,
    LegalChecker,
    DrillPageArray,
) {
    let mut harness = Harness::build_with(T7_FIXTURE_DSN, NET_A);
    let mut ctrl = AutorouteControl::new(harness.board_mut(), NET_A, &maze_settings_ir(false));
    ctrl.ripup_allowed = true;
    ctrl.ripup_costs = seed;
    ctrl.ripup_pass_no = 1;
    ctrl.vias_allowed = false;
    ctrl.remove_unconnected_vias = remove_unconnected_vias;
    let distance = DestinationDistance::from_ctrl(&ctrl);
    let checker = LegalChecker;
    let max_page_width = max_drill_page_width(harness.default_via_diameter());
    let pages = DrillPageArray::new(&harness, max_page_width);
    (harness, ctrl, distance, checker, pages)
}

/// The capture `ctrl` rows (identical resolved tables across the three
/// phases; the seed and the protection flag differ).
fn t7_assert_ctrl(ctrl: &AutorouteControl, phase_idx: usize) {
    let (seed, remove_uv) = T7_PHASES[phase_idx];
    assert_eq!(ctrl.net_number, 1, "ctrl netNumber");
    assert_eq!(ctrl.layer_count, 2, "ctrl layerCount");
    assert!(ctrl.ripup_allowed, "ctrl ripupAllowed");
    assert_eq!(ctrl.ripup_costs, seed, "ctrl ripupCosts");
    assert_eq!(ctrl.ripup_pass_no, 1, "ctrl ripupPassNo");
    assert_eq!(ctrl.settings.start_ripup_costs, 1, "ctrl startRipupCosts");
    assert_eq!(
        ctrl.remove_unconnected_vias, remove_uv,
        "ctrl removeUnconnectedVias"
    );
    assert!(!ctrl.is_fanout, "ctrl isFanout");
    assert!(!ctrl.vias_allowed, "ctrl viasAllowed");
    assert_eq!(ctrl.trace_clearance_class_index, 1, "ctrl clearanceClass");
    assert_eq!(ctrl.max_shove_trace_recursion_depth, 20, "ctrl maxShove");
    // M11-T3 rotation (2026-10-03, #931 cluster B): the depths rise
    // 5 -> 8 upstream; the Java-pre capture carried 5.
    assert_eq!(ctrl.max_shove_via_recursion_depth, 8, "ctrl maxShoveVia");
    assert_eq!(
        ctrl.compensated_trace_half_width,
        [11250, 11250],
        "ctrl compensatedTraceHalfWidth"
    );
    assert_eq!(ctrl.trace_half_width, [10000, 10000], "ctrl traceHalfWidth");
    assert_eq!(ctrl.layer_active, [true, true], "ctrl layerActive");
}

/// Asserts one direct call against its capture rows (marker + optional
/// CHECK_RIPUP log + result). The `detour` literal is the RNG-stream
/// witness: it changes only when the generator advanced.
#[allow(clippy::too_many_arguments)]
fn assert_call(
    want: &T7CallRow,
    slot: T7Slot,
    obstacle: u64,
    pass_no: i32,
    cost: i32,
    trace: Option<&RipupTrace>,
    phase: &str,
) {
    assert_eq!(want.slot, slot, "{phase} call slot obstacle {obstacle}");
    assert_eq!(want.obstacle, obstacle, "{phase} call obstacle");
    assert_eq!(
        want.pass_no, pass_no,
        "{phase} call pass obstacle {obstacle}"
    );
    assert_eq!(
        want.cost, cost,
        "{phase} call cost obstacle {obstacle} pass {pass_no}"
    );
    match (want.trace, trace) {
        (None, None) => {}
        (Some(wt), Some(rt)) => {
            assert_eq!(
                wt.items, rt.connection_item_ids,
                "{phase} connectionItems obstacle {obstacle} pass {pass_no}"
            );
            assert_eq!(
                wt.half_width, rt.half_width,
                "{phase} halfWidth obstacle {obstacle} pass {pass_no}"
            );
            assert_eq!(
                wt.ripup_costs, rt.ripup_costs,
                "{phase} ripupCosts obstacle {obstacle} pass {pass_no}"
            );
            assert_eq!(
                wt.trace_length, rt.trace_length,
                "{phase} traceLength obstacle {obstacle} pass {pass_no}"
            );
            assert_eq!(
                wt.min_trace_length, rt.min_trace_length,
                "{phase} minTraceLength obstacle {obstacle} pass {pass_no}"
            );
            assert_eq!(
                wt.item_count, rt.item_count,
                "{phase} itemCount obstacle {obstacle} pass {pass_no}"
            );
            assert_eq!(
                wt.detour, rt.detour,
                "{phase} detour obstacle {obstacle} pass {pass_no} (the RNG stream)"
            );
            assert_eq!(
                wt.result, rt.result,
                "{phase} result obstacle {obstacle} pass {pass_no}"
            );
        }
        _ => panic!("{phase}: log presence mismatch obstacle {obstacle} pass {pass_no}"),
    }
}

/// THE T7 protocol pin: the full three-phase drain replay. Every
/// expected value is a literal capture row; the call/detour tables pin
/// the whole `java.util.Random` stream (the RNG-lifetime witness lives
/// in the two SEED2 P4a/P4b detours of harvest 0: 2221734702.386413 vs
/// 2818803825.9464397 — DIFFERENT, so the engine's generator was
/// seeded once at construction, not per call; the randomize gate truth
/// table passNo 1 false / 3 false / 4 true / 6 false / 7 true follows
/// from P1/P3/P6 carrying the unrandomized 2147483647.0 detour while
/// P4a/P4b/P7 carry drawn factors).
///
/// DEVIATION (T10-gated prefix): pops 0-8 and harvest 0's full probe
/// battery are pinned EXACTLY against the capture. At pop 8 the Java
/// engine re-parks the roomRipped door-458753 element three times
/// (pops 8-10) because its INTERNAL `shoveTraceRoom` verdict for the
/// (door 458753, obstacle room 14337) pair is FALSE, while the port's
/// is TRUE — the port then expands immediately and the downstream pop
/// stream diverges (Java capture room 232 vs port 231, Δ1 id burn).
/// The ported `checkShoveTraceLine` matches Java arm-for-arm
/// (MazeTraceShover.java:32-314); the disagreement is a semantic board
/// state behind one of the dim-2 branch's three FALSE arms
/// (otherRoom-obstacle / endPointsMatching / corner distance), which
/// the capture never probed directly — only its observable (the delay)
/// is pinned. Resolving it needs the REAL `checkTraceSegment` +
/// `TraceShover.check` (the T10 remainder): with the T6/T7 seam stubs
/// (+inf / 0) the tail cannot reproduce Java's FALSE. Rows k > 8 are
/// therefore pinned by a CONTRACT, not literals: the harvest order
/// (obstacles 13, 12, 15, 11, 16, 18), their kinds and piece indices,
/// and the unrandomized P1 economics (cost 1, detour
/// 2147483647.0) — see SEAM.md and the task report.
#[test]
fn t7_ripup_protocol_pins() {
    const PASS_PLAN: [i32; 6] = [1, 3, 4, 4, 6, 7];
    const PASS_SLOTS: [T7Slot; 6] = [
        T7Slot::P1,
        T7Slot::P3,
        T7Slot::P4a,
        T7Slot::P4b,
        T7Slot::P6,
        T7Slot::P7,
    ];
    for (phase_idx, &(seed, remove_uv)) in T7_PHASES.iter().enumerate() {
        let phase = ["SEED2", "SEED10", "PROBE"][phase_idx];
        let (mut harness, mut ctrl, mut distance, mut checker, mut pages) =
            t7_make_run(seed, remove_uv);
        t7_assert_ctrl(&ctrl, phase_idx);

        // The raw-generator probes: a FRESH JavaRandom (the port pin,
        // independent of the engine's instance).
        let mut rng_probe = epic_geometry::java_random::JavaRandom::new(i64::from(seed));
        for (i, want) in T7_RNG[phase_idx].iter().enumerate() {
            assert_eq!(rng_probe.next_double(), *want, "{phase} rng draw {i}");
        }

        let mut engine = MazeSearchEngine::new(
            &mut harness,
            &mut ctrl,
            &mut distance,
            &mut checker,
            &mut pages,
        );
        // The spike's nearest-x probes pick pin 4 for BOTH endpoints —
        // the destination is the start item, so `destinationDoor` stays
        // null and every phase drains the front EMPTY (phase_done rows).
        assert!(engine.init(&[T7_PIN], &[T7_PIN]), "{phase} init ok");

        let mut harvested: BTreeSet<u64> = BTreeSet::new();
        let mut ripped_done: BTreeSet<(i32, i32)> = BTreeSet::new();
        let mut stale_done = false;
        let mut hw_done = false;
        let mut harvest_seq = 0usize;
        let mut call_iter = T7_CALLS[phase_idx].iter();
        let mut gate_iter = T7_GATES[phase_idx].iter();
        let mut leaving_seq = 0usize;
        let mut sd_seq = 0usize;
        let mut lr_iter = T7_LEAVING_RIPPED[phase_idx].iter();
        let mut sdr_iter = T7_SMALL_DOOR_RIPPED[phase_idx].iter();
        let mut shove_seq = 0usize;

        for (k, want_pop) in T7_POPS[phase_idx].iter().enumerate() {
            // The pre-divergence prefix (see the DEVIATION note): pops
            // 0-8 are literal capture rows; beyond it the port's
            // engine-internal shove verdict expands where Java delayed.
            let in_prefix = k <= 8;
            // The `pop` row: the front head before the occupation.
            // Post-divergence the port's stream is SHORTER than the
            // capture's 72 pops (the early expansion keeps the front
            // smaller); the drain ends where Java's protocol ends —
            // when the front is empty.
            let head = match engine.front.iter().next().cloned() {
                Some(head) => head,
                None => break,
            };
            if in_prefix {
                assert_eq!(want_pop.k, k as i32, "{phase} pop {k} table index");
                assert_eq!(head.door.id(), want_pop.door_id, "{phase} pop {k} doorId");
                assert_eq!(
                    head.section_no_of_door, want_pop.section,
                    "{phase} pop {k} section"
                );
                assert_eq!(
                    head.room_ripped, want_pop.room_ripped,
                    "{phase} pop {k} roomRipped"
                );
            }

            // The :506 leaving path on the first roomRipped element
            // (per door:section, once) — PREFIX-only: which elements
            // are roomRipped is pop-stream state (see DEVIATION). The
            // scan binds through a statement so the front borrow ends
            // before the probe calls.
            let ripped = engine.front.iter().find(|e| e.room_ripped).cloned();
            if in_prefix
                && let Some(r) = ripped
                && ripped_done.insert((r.door.id(), r.section_no_of_door))
            {
                let want = lr_iter.next().expect("{phase} leaving_ripped row");
                assert_eq!(
                    (want.k, want.door_id, want.section),
                    (k as i32, r.door.id(), r.section_no_of_door),
                    "{phase} leaving_ripped {k} pairing"
                );
                let verdict = engine.check_leaving_ripped_item(&r);
                assert_eq!(verdict, want.verdict, "{phase} leaving_ripped {k}");
                // Ignore variants: the destination pin, then the room
                // being left's obstacle item.
                let from_obstacle = match &r.door {
                    ExpandableObject::RoomDoor(rd) => {
                        let nid = engine
                            .ctx
                            .room_id(r.next_room_key.expect("{phase} ripped next room"));
                        rd.other_room_id(nid)
                            .and_then(|id| engine.ctx.room_key_of_id(id))
                            .filter(|key| engine.ctx.room_is_obstacle(*key))
                            .and_then(|key| engine.ctx.room_obstacle_item_key(key))
                    }
                    _ => None,
                };
                for ignore in [T7_PIN].into_iter().chain(from_obstacle) {
                    let want_sd = sdr_iter.next().expect("{phase} small_door_ripped row");
                    assert_eq!(
                        (want_sd.door_id, want_sd.section),
                        (r.door.id(), r.section_no_of_door),
                        "{phase} small_door_ripped {k} pairing"
                    );
                    assert_eq!(
                        want_sd.ignore_id, ignore,
                        "{phase} small_door_ripped {k} ignoreId"
                    );
                    let v = engine.enter_through_small_door(&r, ignore);
                    assert_eq!(
                        v, want_sd.verdict,
                        "{phase} small_door_ripped {k} ignore {ignore}"
                    );
                }
            }

            // The harvest scan over the WHOLE front (a ripup delay may
            // re-add elements; the drain mutates the set, so scan per k).
            let found = {
                let snapshot: Vec<MazeListElement> = engine.front.iter().cloned().collect();
                snapshot.into_iter().find(|e| {
                    e.next_room_key
                        .is_some_and(|key| engine.ctx.room_is_obstacle(key))
                })
            };
            if let Some(e) = found {
                let obstacle_room = e.next_room_key.expect("{phase} harvest next room");
                let obstacle = engine
                    .ctx
                    .room_obstacle_item_key(obstacle_room)
                    .expect("{phase} harvest obstacle item");
                if harvested.insert(obstacle) {
                    let want_seq = harvest_seq;
                    harvest_seq += 1;
                    // Post-divergence the HARVEST ORDER is pop-stream
                    // state too (the port parks at the obstacles in a
                    // different sequence) — the contract pairs by
                    // obstacle id, the prefix keeps the capture order.
                    let want_h = if in_prefix {
                        &T7_HARVESTS[phase_idx][want_seq]
                    } else {
                        T7_HARVESTS[phase_idx]
                            .iter()
                            .find(|row| row.obstacle == obstacle)
                            .expect("{phase} contract: obstacle in the capture harvest set")
                    };
                    if in_prefix {
                        assert_eq!(want_h.k, k as i32, "{phase} harvest k");
                    }
                    assert_eq!(want_h.obstacle, obstacle, "{phase} harvest obstacle");
                    let kind = if engine.ctx.item_is_via(obstacle) {
                        T7Kind::Via
                    } else if engine.ctx.item_is_polyline_trace(obstacle) {
                        T7Kind::Trace
                    } else {
                        panic!("{phase}: unexpected harvest kind for item {obstacle}");
                    };
                    assert_eq!(kind, want_h.kind, "{phase} harvest kind");
                    let corner_no = i32::try_from(
                        engine
                            .ctx
                            .room_obstacle_index_in_item(obstacle_room)
                            .expect("{phase} harvest corner index"),
                    )
                    .expect("corner index fits i32");
                    assert_eq!(corner_no, want_h.corner_no, "{phase} harvest cornerNo");
                    if !in_prefix {
                        // T10-gated contract (see the DEVIATION note):
                        // the parked element and the pop stream are
                        // divergent here; pin only the room-derived
                        // state above plus the unrandomized P1
                        // economics (draw-free, stream-independent).
                        engine.ctrl.ripup_pass_no = 1;
                        let (cost, trace) = engine.check_ripup_traced(&e, obstacle, false);
                        assert_eq!(cost, 1, "{phase} contract P1 cost obstacle {obstacle}");
                        let trace = trace.expect("{phase} contract P1 log row");
                        assert_eq!(
                            trace.detour, 2147483647.0,
                            "{phase} contract P1 unrandomized detour"
                        );
                        assert_eq!(trace.result, 1, "{phase} contract P1 result");
                    } else {
                        assert_eq!(e.door.id(), want_h.door_id, "{phase} harvest doorId");
                        assert_eq!(
                            e.section_no_of_door, want_h.section,
                            "{phase} harvest section"
                        );
                        assert_eq!(
                            e.door.dimension(),
                            want_h.dimension,
                            "{phase} harvest dimension"
                        );
                        assert_eq!(
                            e.already_checked, want_h.already_checked,
                            "{phase} harvest alreadyChecked"
                        );
                        assert_eq!(
                            e.room_ripped, want_h.room_ripped,
                            "{phase} harvest roomRipped"
                        );

                        // --- the PASSES loop (mutating ctrl.ripupPassNo; the
                        // double pass-4 is the RNG lifetime witness).
                        for (pi, &pass) in PASS_PLAN.iter().enumerate() {
                            engine.ctrl.ripup_pass_no = pass;
                            let (cost, trace) = engine.check_ripup_traced(&e, obstacle, false);
                            let want = call_iter.next().expect("{phase} call row");
                            assert_eq!(
                                want.harvest as usize,
                                harvest_seq - 1,
                                "{phase} call harvest"
                            );
                            assert_call(
                                want,
                                PASS_SLOTS[pi],
                                obstacle,
                                pass,
                                cost,
                                trace.as_ref(),
                                phase,
                            );
                        }
                        // --- the surgical gate probes (17/19/20, all -1
                        // through the FIRST gate; no log row = trace None).
                        let want_gates = gate_iter.next().expect("{phase} gate triple");
                        for (gi, &g) in [17u64, 19, 20].iter().enumerate() {
                            assert_eq!(want_gates[gi], g, "{phase} gate order");
                            let (cost, trace) = engine.check_ripup_traced(&e, g, false);
                            assert_eq!(cost, -1, "{phase} gate probe {g}");
                            assert!(trace.is_none(), "{phase} gate probe {g} logs nothing");
                        }
                        // --- the item-9 economics probe (SEED2: protection
                        // ON floors at the i32::MAX/100 cap; PROBE: off,
                        // result floors at 1).
                        if phase == "SEED2" || phase == "PROBE" {
                            let (cost, trace) = engine.check_ripup_traced(&e, 9, false);
                            let want = call_iter.next().expect("{phase} item9 call row");
                            assert_eq!(
                                want.harvest as usize,
                                harvest_seq - 1,
                                "{phase} item9 harvest"
                            );
                            assert_call(want, T7Slot::Item9, 9, 7, cost, trace.as_ref(), phase);
                        }
                        // --- the ALREADY_RIPPED synthetic element: the
                        // door's OTHER room is the next room of the synth,
                        // so its previous item IS the obstacle -> cost 1
                        // BEFORE any economics (no log row).
                        let door_for_synth = e.door.clone();
                        if let ExpandableObject::RoomDoor(rd) = &door_for_synth {
                            let nid = engine.ctx.room_id(obstacle_room);
                            let other = rd
                                .other_room_id(nid)
                                .and_then(|id| engine.ctx.room_key_of_id(id))
                                .filter(|key| {
                                    engine.ctx.room_is_complete_free_space(*key)
                                        || engine.ctx.room_is_obstacle(*key)
                                });
                            if let Some(other_key) = other {
                                let synth = MazeListElement::new(
                                    door_for_synth.clone(),
                                    e.section_no_of_door,
                                    None,
                                    0,
                                    0.0,
                                    0.0,
                                    Some(other_key),
                                    e.shape_entry,
                                    false,
                                    Adjustment::None,
                                    false,
                                );
                                let (cost, trace) =
                                    engine.check_ripup_traced(&synth, obstacle, false);
                                let want = call_iter.next().expect("{phase} already call row");
                                assert_eq!(
                                    want.harvest as usize,
                                    harvest_seq - 1,
                                    "{phase} already harvest"
                                );
                                assert_call(
                                    want,
                                    T7Slot::Already,
                                    obstacle,
                                    7,
                                    cost,
                                    trace.as_ref(),
                                    phase,
                                );
                            }
                        }
                        // --- the leaving + small-door probes (dim-1 doors).
                        if e.door.dimension() == 1 {
                            let verdict = engine.check_leaving_ripped_item(&e);
                            assert_eq!(
                                verdict, T7_LEAVING[phase_idx][leaving_seq],
                                "{phase} leaving"
                            );
                            leaving_seq += 1;
                            for ignore in [obstacle, T7_PIN] {
                                let want_sd = T7_SMALL_DOOR[phase_idx][sd_seq];
                                sd_seq += 1;
                                assert_eq!(want_sd.0, ignore, "{phase} small_door ignoreId");
                                let v = engine.enter_through_small_door(&e, ignore);
                                assert_eq!(v, want_sd.1, "{phase} small_door ignore {ignore}");
                                let layer = engine.ctx.room_layer(obstacle_room);
                                assert_eq!(
                                    want_sd.2,
                                    f64::from(
                                        engine.ctrl.compensated_trace_half_width[layer as usize]
                                    ) + 2.0,
                                    "{phase} small_door checkRadius"
                                );
                                // The SEED2 harvest-0 first probe pins the
                                // door-shape derivation itself (the capture
                                // `doorCorners` literal array).
                                if phase_idx == 0 && sd_seq == 1 {
                                    let ExpandableObject::RoomDoor(rd) = &e.door else {
                                        panic!("a dim-1 harvest door is a room door");
                                    };
                                    let (_, _, fs, ss) = engine.door_endpoint_shapes(rd);
                                    let shape =
                                        crate::expansion::ExpansionDoor::shape_between(&fs, &ss);
                                    assert_eq!(
                                        shape.border_line_count(),
                                        T7_DOOR_CORNERS.len(),
                                        "{phase} doorCorners count"
                                    );
                                    for (i, (wx, wy)) in T7_DOOR_CORNERS.iter().enumerate() {
                                        let c = shape
                                            .corner_approx(i as i32)
                                            .expect("{phase} door corner in range");
                                        assert_eq!(
                                            (c.x, c.y),
                                            (*wx, *wy),
                                            "{phase} doorCorners {i}"
                                        );
                                    }
                                }
                            }
                        }
                        // --- the shove probes (READ-ONLY; every captured
                        // row returned an EMPTY door list).
                        let ExpandableObject::RoomDoor(harvest_door) = &e.door else {
                            panic!("{phase} harvest door is a room door");
                        };
                        if kind == T7Kind::Trace {
                            for left in [false, true] {
                                let mut doors: Vec<DoorSection> = Vec::new();
                                let verdict = engine.check_shove_trace_line(
                                    &e,
                                    obstacle_room,
                                    left,
                                    &mut doors,
                                );
                                let want_s = T7_SHOVE[phase_idx][shove_seq];
                                shove_seq += 1;
                                assert_eq!(want_s.left, left, "{phase} shove direction");
                                assert_eq!(
                                    verdict, want_s.verdict,
                                    "{phase} shove verdict obstacle {obstacle} left {left}"
                                );
                                assert!(doors.is_empty(), "{phase} shove toDoors empty");
                                assert_eq!(want_s.corner_no, corner_no, "{phase} shove cornerNo");
                                let polyline = engine
                                    .ctx
                                    .item_trace_polyline(obstacle)
                                    .expect("{phase} obstacle polyline");
                                assert_eq!(
                                    want_s.lines,
                                    polyline.lines.len(),
                                    "{phase} shove lines"
                                );
                                assert_eq!(
                                    want_s.half_width,
                                    engine.ctx.item_trace_half_width(obstacle),
                                    "{phase} shove halfWidth"
                                );
                                let layer = engine.ctx.room_layer(obstacle_room);
                                assert_eq!(
                                    want_s.ctrl_half_width,
                                    engine.ctrl.trace_half_width[layer as usize],
                                    "{phase} shove ctrlHalfWidth"
                                );
                                let (_, _, fs, ss) = engine.door_endpoint_shapes(harvest_door);
                                assert_eq!(
                                    want_s.door_max_width,
                                    crate::expansion::ExpansionDoor::shape_between(&fs, &ss)
                                        .max_width(),
                                    "{phase} shove doorMaxWidth"
                                );
                            }
                        } else {
                            let mut doors: Vec<DoorSection> = Vec::new();
                            let verdict =
                                engine.check_shove_trace_line(&e, obstacle_room, false, &mut doors);
                            assert_eq!(
                                verdict, T7_SHOVE_NONTRACE[phase_idx],
                                "{phase} shove nontrace"
                            );
                            assert!(doors.is_empty(), "{phase} shove nontrace toDoors empty");
                        }
                        // --- the stale-index probe (once per phase, first
                        // trace harvest): a synthetic room with staleIdx 5
                        // must answer FALSE through the stale gate — and
                        // would PANIC at lines[cornerNo + 1] without it
                        // (the mutant-kill mechanism; the captured -1 row
                        // is structurally unreachable under the u32 seam,
                        // see the report).
                        if !stale_done && kind == T7Kind::Trace {
                            stale_done = true;
                            engine.ctx.force_obstacle_index_in_item(Some(5));
                            let mut doors: Vec<DoorSection> = Vec::new();
                            let verdict =
                                engine.check_shove_trace_line(&e, obstacle_room, false, &mut doors);
                            engine.ctx.force_obstacle_index_in_item(None);
                            assert_eq!(verdict, T7_SHOVE_STALE[phase_idx], "{phase} shove stale");
                            assert!(doors.is_empty(), "{phase} shove stale toDoors empty");
                        }
                        // --- the halfwidth-gate contrast (once per phase):
                        // ctrl.traceHalfWidth forced to 999 flips the
                        // same-width gate to TRUE.
                        if !hw_done {
                            hw_done = true;
                            let layer = engine.ctx.room_layer(obstacle_room);
                            let saved = engine.ctrl.trace_half_width[layer as usize];
                            engine.ctrl.trace_half_width[layer as usize] =
                                if saved == 999 { 998 } else { 999 };
                            let mut doors: Vec<DoorSection> = Vec::new();
                            let verdict =
                                engine.check_shove_trace_line(&e, obstacle_room, false, &mut doors);
                            engine.ctrl.trace_half_width[layer as usize] = saved;
                            assert_eq!(
                                verdict, T7_SHOVE_HW_GATE[phase_idx],
                                "{phase} shove hw gate"
                            );
                            assert!(doors.is_empty(), "{phase} shove hw gate toDoors empty");
                        }
                    }
                }
            }
            let _ = engine.occupy_next_element();
        }

        // The phase_done row: front drained EMPTY, destination NEVER
        // set (the start item IS the destination). The prefix battery
        // consumed exactly the harvest-0 probe rows — the next
        // unconsumed rows prove the battery boundary.
        assert_eq!(harvest_seq, 7, "{phase} harvest count");
        let next_call = call_iter.next().expect("{phase} harvest-1 P1 row");
        assert_eq!(
            (next_call.harvest as usize, next_call.slot),
            (1, T7Slot::P1),
            "{phase} battery boundary"
        );
        assert_eq!(gate_iter.count(), 6, "{phase} gate triples left unconsumed");
        assert_eq!(leaving_seq, 1, "{phase} leaving count (prefix row 0)");
        assert_eq!(sd_seq, 2, "{phase} small_door count (prefix rows 0-1)");
        let next_lr = lr_iter.next().expect("{phase} leaving_ripped row 1");
        assert_eq!(
            (next_lr.k, next_lr.door_id),
            (11, 14398),
            "{phase} leaving_ripped boundary"
        );
        let next_sdr = sdr_iter.next().expect("{phase} small_door_ripped row 2");
        assert_eq!(
            (next_sdr.door_id, next_sdr.ignore_id),
            (14398, 4),
            "{phase} small_door_ripped boundary"
        );
        assert_eq!(shove_seq, 2, "{phase} shove count (harvest 0)");
        assert!(engine.front.is_empty(), "{phase} phase_done frontSize 0");
        assert!(
            engine.destination.is_none(),
            "{phase} phase_done destination false"
        );
    }
}

/// The T10 shove-seam force contract: the probe's post-seam behavior is
/// driven by the seam verdicts. Under the capture's defaults
/// (`checkTraceSegment` = +inf, `shoveTraceCheck` = 0) the probe
/// short-circuits with an EMPTY door list — the captured shape. Forcing
/// `shove_trace_check` positive must let the door-collection loop run
/// for at least one probe (a port that drops the seam call flips this).
/// Contract-level pin: no capture literal claims the synthetic forced
/// verdicts; the assertions compare forced vs default.
#[test]
fn t7_shove_seam_force_contract() {
    let (mut harness, mut ctrl, mut distance, mut checker, mut pages) = t7_make_run(2, false);
    let mut engine = MazeSearchEngine::new(
        &mut harness,
        &mut ctrl,
        &mut distance,
        &mut checker,
        &mut pages,
    );
    assert!(engine.init(&[T7_PIN], &[T7_PIN]), "init ok");
    let mut harvested: BTreeSet<u64> = BTreeSet::new();
    let mut responsive = 0usize;
    for _ in 0..72 {
        if engine.front.is_empty() {
            break;
        }
        let found = {
            let snapshot: Vec<MazeListElement> = engine.front.iter().cloned().collect();
            snapshot.into_iter().find(|e| {
                e.next_room_key
                    .is_some_and(|key| engine.ctx.room_is_obstacle(key))
            })
        };
        if let Some(e) = found {
            let obstacle_room = e.next_room_key.expect("harvest next room");
            let obstacle = engine
                .ctx
                .room_obstacle_item_key(obstacle_room)
                .expect("harvest obstacle item");
            if harvested.insert(obstacle) && engine.ctx.item_is_polyline_trace(obstacle) {
                for left in [false, true] {
                    let mut default_doors: Vec<DoorSection> = Vec::new();
                    let default_verdict =
                        engine.check_shove_trace_line(&e, obstacle_room, left, &mut default_doors);
                    engine.ctx.force_shove_trace_check(Some(1.0e9));
                    let mut forced_doors: Vec<DoorSection> = Vec::new();
                    let forced_verdict =
                        engine.check_shove_trace_line(&e, obstacle_room, left, &mut forced_doors);
                    engine.ctx.force_shove_trace_check(None);
                    assert_eq!(default_verdict, forced_verdict, "verdict stays true");
                    if default_doors.is_empty() && !forced_doors.is_empty() {
                        responsive += 1;
                    }
                }
            }
        }
        let _ = engine.occupy_next_element();
    }
    assert!(
        responsive > 0,
        "at least one probe responds to the forced shove verdict"
    );
}

// ---- the M6-T9 push-and-shove waiver pins ----

/// The waiver's engine wiring, both flag faces on the t7 world (the
/// same harness the shove-seam contract uses). Two harness knobs make
/// the site REACHABLE — both faces run the IDENTICAL world, the flag
/// bit is the only difference:
///
/// * the compensation force (1250): the harness's default stub
///   answers 0, which keeps `room_shape_is_thick` false for the
///   same-width NET_B traces; production answers 1250 on this exact
///   fixture (it is the compensation folded into
///   `ctrl.compensated_trace_half_width` 11250 = 10000 + 1250 — the
///   T7 ctrl rows pin that table), so the forced value mirrors
///   production, not an invention.
/// * the ripup seed 2_000_000 (capture phase 1 used 2): `check_ripup`
///   clamps its result to `.max(1)`, and 1 COLLIDES with
///   `ALREADY_RIPPED_COSTS` — the t7 NET_B maze traces are
///   OPEN-ENDED connections, so their detour is
///   `DETOUR_OPEN_ENDED` (2147483647.0) and every low-seed row
///   computes exactly 1 (`seed * half_width / i32::MAX` truncates to
///   0), leaving the shove block (whose first gate is
///   `ripup_costs != ALREADY_RIPPED_COSTS`) permanently skipped. The
///   raised seed makes the charge real (~9) without touching the
///   engine.
///
/// With those set, the default `shove_trace_check` seam (0.0 → the
/// probe answers shoved=true) puts every thick obstacle encounter at
/// the waiver site. Assertions:
///
/// * OFF (the default): the budget arms at 0 and NO waiver ever fires
///   over the whole drain — the inertness face of the default-off
///   byte-identity contract.
/// * ON: the budget arms at [`PUSH_SHOVE_ROOM_BUDGET`], the waiver
///   fires, and the budget is EXACTLY EXHAUSTED by the drain — the
///   t7 world's shove opportunities exceed the budget, so
///   `budget_left` reaches 0 and the DERIVED waiver count
///   (`armed − budget_left`; NIT-Q1, M7-T2: the stored
///   `push_shove_waivers` counter was dropped as derivable from
///   exactly this subtraction) equals BUDGET. The M-BUDGET ±1 mutant
///   dies on `armed` (the const is read verbatim) and on exhaustion
///   (a larger budget leaves `budget_left > 0`).
#[test]
fn t9_push_shove_waiver_budget_bounds_and_off_inertness() {
    let drain = |push_shove_on: bool| -> (i32, i32, usize) {
        let (mut harness, mut ctrl, mut distance, mut checker, mut pages) =
            t7_make_run(2_000_000, false);
        harness.force_clearance_compensation(Some(1250));
        ctrl.push_shove = push_shove_on;
        let mut engine = MazeSearchEngine::new(
            &mut harness,
            &mut ctrl,
            &mut distance,
            &mut checker,
            &mut pages,
        );
        assert!(engine.init(&[T7_PIN], &[T7_PIN]), "init ok");
        let armed = engine.push_shove_budget_left;
        let mut steps = 0usize;
        while !engine.front.is_empty() && steps < 4096 {
            let _ = engine.occupy_next_element();
            steps += 1;
        }
        (armed, engine.push_shove_budget_left, steps)
    };
    let (off_armed, off_budget, off_steps) = drain(false);
    assert_eq!(off_armed, 0, "OFF: the budget arms at zero");
    assert_eq!(off_budget, 0, "OFF: no waiver ever fires");
    assert!(off_steps > 0, "the drain ran");
    let (on_armed, on_budget, _on_steps) = drain(true);
    assert_eq!(
        on_armed,
        super::ripup::PUSH_SHOVE_ROOM_BUDGET,
        "ON arms at the per-search const"
    );
    let on_waivers = on_armed - on_budget;
    assert!(
        on_waivers > 0,
        "ON: the waiver fires in the t7 world (else the pin judges nothing)"
    );
    assert!(
        on_waivers <= super::ripup::PUSH_SHOVE_ROOM_BUDGET,
        "ON: bounded by the per-search budget"
    );
    assert_eq!(
        on_waivers,
        super::ripup::PUSH_SHOVE_ROOM_BUDGET,
        "exact exhaustion: the t7 world's opportunities exceed the budget"
    );
}

// ---- the T17a buglog-170 pin ----

/// The via-match craft (PURE_DSN shape): net MINE carries W1
/// (40000,40000)→(30000,40000) — a corner AT the via center — W2
/// (10000,10000)→(20000,10000) — corners far from it — and the via
/// at (40000,40000). Net OTHER carries only the pin.
pub(crate) const T17_VIA_MATCH_DSN: &str = "\
(pcb t17-via-match.dsn\n\
  (parser\n\
    (string_quote \")\n\
    (space_in_quoted_tokens on)\n\
  )\n\
  (resolution um 1)\n\
  (unit um)\n\
  (structure\n\
    (layer F.Cu (type signal))\n\
    (layer B.Cu (type signal))\n\
    (boundary (rect pcb 0 0 100000 60000))\n\
    (rule (width 250) (clearance 14))\n\
  )\n\
  (placement\n\
    (component CMP1\n\
      (place CMP1 20000 40000 front 0)\n\
    )\n\
  )\n\
  (library\n\
    (padstack PAD_C600\n\
      (shape (circle F.Cu 600 0 0))\n\
    )\n\
    (image CMP1\n\
      (pin PAD_C600 P1 0 0)\n\
    )\n\
  )\n\
  (network\n\
    (net MINE)\n\
    (net OTHER (pins CMP1-P1))\n\
  )\n\
  (wiring\n\
    (wire (path F.Cu 250  40000 40000  30000 40000) (net MINE))\n\
    (via PAD_C600 40000 40000 (net MINE))\n\
    (wire (path F.Cu 250  10000 10000  20000 10000) (net MINE))\n\
  )\n\
)\n";

/// THE buglog-170 pin (T17a): `endPointsMatching`'s via arm reads the
/// via's STORED CENTER (Java `item.getCenter()`,
/// MazeTraceShover.java:329-332) — never the lazily-computed
/// `getAutorouteDrillInfo` transient (Via.java:204-216). The harness
/// engine leaves `via_drill_info` at its default `None` — exactly the
/// absent-drill-info world that panicked at the old `expect` — and the
/// arms still answer: a W1 corner AT the center matches (Java's
/// `fromCenter.equals(firstCorner)`), W2's distant corners do not,
/// and the foreign-net pin dies at the `sharesNet` gate before the
/// via arm is ever reached.
#[test]
fn t17_end_points_matching_computes_the_via_center_not_drill_info() {
    let harness = Harness::build_with(T17_VIA_MATCH_DSN, 1);
    use crate::drill::DrillEngine;
    let mut via_key = None;
    let mut w1_key = None;
    let mut w2_key = None;
    let mut pin_key = None;
    for key in 1u64..=32 {
        if harness.item_is_via(key) {
            via_key = Some(key);
        } else if harness.item_is_pin(key) {
            pin_key = Some(key);
        } else if harness.item_is_polyline_trace(key) {
            let Some(trace) = harness.item_trace_polyline(key) else {
                continue;
            };
            let near_via = [trace.first_corner(), trace.last_corner()].iter().any(
                |corner| {
                    matches!(corner, Some(epic_geometry::point::Point::Int(center)) if (center.x, center.y) == (40000, 40000))
                },
            );
            if near_via {
                w1_key = Some(key);
            } else {
                w2_key = Some(key);
            }
        }
    }
    let (via_key, w1_key, w2_key, pin_key) = (
        via_key.expect("the craft carries a via"),
        w1_key.expect("W1 ends at the via center"),
        w2_key.expect("W2 is the far trace"),
        pin_key.expect("the OTHER-net pin"),
    );
    // The arm's premise: this via's drill info is ABSENT (the default
    // seam answers None — the buglog-170 world).
    assert!(
        harness.via_drill_info(via_key).is_none(),
        "the witness world leaves drill info absent"
    );
    assert_eq!(
        harness.via_center(via_key).map(|p| p.to_float()),
        Some(FloatPoint::new(40000.0, 40000.0))
    );
    assert!(
        super::shove_probe::end_points_matching(&harness, w1_key, via_key),
        "via center == W1 end corner: Java's TRUE arm"
    );
    assert!(
        !super::shove_probe::end_points_matching(&harness, w2_key, via_key),
        "W2's corners are not at the via center: FALSE"
    );
    assert!(
        !super::shove_probe::end_points_matching(&harness, w1_key, pin_key),
        "foreign net: the sharesNet gate answers FALSE"
    );
}
