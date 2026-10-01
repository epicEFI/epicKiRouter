//! M3-T4 expansion-graph pins — literal replay of the ExpansionSpike
//! capture (`logs/M3-T4/spike_run{1,2}.jsonl`, byte-identical double
//! run; the pre-digested literal rows live in `logs/M3-T4/pin-literals.md`).
//!
//! The spike protocol (rust/harness/oracle/ExpansionSpike.java, the
//! committed capture driver): the fixture
//! `rust/harness/fixtures/expansion-spike/t4_room_neighbours.dsn`
//! parses at database units = DSN units x 10 (resolution um 10). The
//! S/T batteries call `completeShape` on the whole-board seed with
//! crafted contained shapes (plain tree with net 1 for S, 45-degree
//! tree with net 2 for T); the E battery seeds the board box on the
//! plain tree (net 1, the ANY_ANGLE sorter arm) and completes two
//! rooms; the F battery does the same on the 45-degree tree (net 2,
//! the DEGREE_45 arm). The pins replay exactly that through the real
//! epic-dsn/epic-board/epic-index stack with the
//! [`super::NeighbourEngine`] implemented over the live board — no
//! synthetic worlds, no reconstructed literals (cerebrum mode 8).
//!
//! Room ids: COMPLETE free-space rooms carry engine-sequential ids
//! (Java `generateRoomIdNo` pre-increment; restarts burn an id — the
//! capture's gaps 3, 5). INCOMPLETE rooms carry the capture-proven
//! content formula `31 * shape.get_id() + layer`
//! (`crate::expansion::room`) — the captured Java hash ids (e.g.
//! -333950178) are reproduced by the Rust formula and are pinned as
//! literals; identical shapes legitimately produce identical ids (the
//! capture shows 98239 twice in the E2 list — distinct registry
//! entries, same content id).

use std::collections::{BTreeMap, BTreeSet};

use epic_board::id::ItemId;
use epic_board::items::{FixedState, ItemData};
use epic_geometry::int_box::IntBox;
use epic_geometry::int_octagon::IntOctagon;
use epic_geometry::int_point::IntPoint;
use epic_geometry::line::Line;
use epic_geometry::point::Point;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::simplex::Simplex;
use epic_geometry::tile_shape::TileShape;
use epic_index::SearchTreeVariant;
use epic_index::complete_shape::{
    CompleteShapeObjects, CompleteShapeQuery, IncompleteRoom, complete_shape,
};
use epic_index::search_tree::SearchTree;

use super::door::ExpansionDoor;
use super::neighbours::{ROOM_KEY_BASE, TreeEntry, board_item_is_trace_obstacle};
use super::room::{ExpansionRoom, RoomKind};
use super::target_door::TargetItemExpansionDoor;
use super::{CalculationMode, NeighbourEngine, select_calculation_mode};

/// The spike fixture (a committed deliverable of this task).
const FIXTURE_DSN: &str =
    include_str!("../../../../harness/fixtures/expansion-spike/t4_room_neighbours.dsn");

/// The board box in DB units (capture `meta` row:
/// bounds [-1000,-1000,1001000,601000] — the boundary rectangle plus
/// the outline's clearance treatment). Both battery seeds and every
/// `completeShape` board-bbox argument use exactly this box.
const BOARD_BOX: IntBox = IntBox {
    ll: IntPoint::new(-1000, -1000),
    ur: IntPoint::new(1001000, 601000),
};

// the battery tags used by the literal tables
const T_E1: u8 = 0;
const T_E2: u8 = 1;
const T_F1: u8 = 2;
const T_F2: u8 = 3;

// ---- shape helpers ----

fn pt(x: i32, y: i32) -> Point {
    Point::int(IntPoint::new(x, y))
}

fn box_tile(ll_x: i32, ll_y: i32, ur_x: i32, ur_y: i32) -> TileShape {
    TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::new(
        IntPoint::new(ll_x, ll_y),
        IntPoint::new(ur_x, ur_y),
    )))
}

fn board_box_tile() -> TileShape {
    TileShape::RegularTileShape(RegularTileShape::IntBox(BOARD_BOX))
}

fn board_oct_tile() -> TileShape {
    TileShape::RegularTileShape(RegularTileShape::IntOctagon(
        board_box_tile()
            .bounding_octagon()
            .expect("an IntBox always has a bounding octagon"),
    ))
}

/// The S-battery corridor contained shape (S1 head row).
fn corridor_contained() -> TileShape {
    box_tile(450000, 250000, 550000, 350000)
}

/// The F-battery seed contained shape (oracle t0 IntBox
/// [200000,450000,300000,550000]).
fn f_contained() -> TileShape {
    box_tile(200000, 450000, 300000, 550000)
}

/// Java `net.getNetClass().getTraceClearanceClass()` — the net's own
/// trace clearance class. The F-battery engine runs on the NET_B
/// class tree (`new AutorouteEngine(board, classB, true)`), whose
/// tree shapes are clearance-compensated at half the t4_slow pair
/// clearance (10000/2 = 5000) — exactly what the F capture rows
/// encode (room 3 stops 5000 from the pad/keepout edges), vs the
/// 1250 dilation of the literal class-1 tree the T probes pin.
fn net_trace_clearance_class(board: &epic_board::board::Board, net_number: i32) -> i32 {
    let rules = board.rules();
    rules
        .nets
        .get(net_number)
        .and_then(|net| rules.net_class(net.net_class))
        .map(|class| class.trace_clearance_class)
        .unwrap_or(1)
}

/// The F battery's engine class, resolved the way the oracle resolves
/// it (from the parsed fixture's NET_B row).
fn f_engine_class() -> i32 {
    let (_manager, board) = crate::test_util::parse(FIXTURE_DSN);
    net_trace_clearance_class(&board, 2)
}

/// The S3/T3 triangle contained shape (oracle corners
/// (490000,360000) (510000,360000) (500000,380000)).
fn triangle_contained() -> TileShape {
    TileShape::Simplex(Box::new(Simplex::get_instance(&[
        Line::new(pt(490000, 360000), pt(510000, 360000)),
        Line::new(pt(510000, 360000), pt(500000, 380000)),
        Line::new(pt(500000, 380000), pt(490000, 360000)),
    ])))
}

/// The capture's shape dump: class tag plus the 4 bounding
/// coordinates (Simplex/IntBox rows) or the 8 IntOctagon coordinates
/// in Java field order (left, bottom, right, top, upper-left, lower-
/// right, lower-left, upper-right diagonals).
fn shape_coords(shape: &TileShape) -> (&'static str, [i32; 8], usize) {
    match shape {
        TileShape::Simplex(_) => {
            let bb = shape.bounding_box();
            ("Simplex", pad4(bb), 4)
        }
        TileShape::RegularTileShape(RegularTileShape::IntBox(_)) => {
            let bb = shape.bounding_box();
            ("IntBox", pad4(bb), 4)
        }
        TileShape::RegularTileShape(RegularTileShape::IntOctagon(oct)) => (
            "IntOctagon",
            [
                oct.left_x,
                oct.bottom_y,
                oct.right_x,
                oct.top_y,
                oct.upper_left_diagonal_x,
                oct.lower_right_diagonal_x,
                oct.lower_left_diagonal_x,
                oct.upper_right_diagonal_x,
            ],
            8,
        ),
    }
}

fn pad4(bb: IntBox) -> [i32; 8] {
    [bb.ll.x, bb.ll.y, bb.ur.x, bb.ur.y, 0, 0, 0, 0]
}

fn assert_coords(
    snap: (&'static str, [i32; 8], usize),
    class: &str,
    coords: &[i32; 8],
    used: usize,
    what: &str,
) {
    assert_eq!(snap.0, class, "{what}: shape class");
    assert_eq!(
        &snap.1[..used],
        &coords[..used],
        "{what}: shape coordinates"
    );
}

// ---- literal row tables (verbatim from pin-literals.md) ----

/// (room_id, shape class, coords, used, layer)
type RoomRow = (i32, &'static str, [i32; 8], usize, i32);

const E1_COMPLETED: [RoomRow; 1] = [(
    1,
    "Simplex",
    [400000, 100, 600000, 480000, 0, 0, 0, 0],
    4,
    0,
)];
const E2_COMPLETED: [RoomRow; 2] = [
    (
        2,
        "Simplex",
        [350000, 100, 400000, 200000, 0, 0, 0, 0],
        4,
        0,
    ),
    (
        4,
        "Simplex",
        [100, 400000, 400000, 599900, 0, 0, 0, 0],
        4,
        0,
    ),
];
const F1_COMPLETED: [RoomRow; 1] = [(
    3,
    "IntOctagon",
    [
        125000, 405000, 435000, 594900, -469900, 30000, 530000, 1029900,
    ],
    8,
    0,
)];
const F2_COMPLETED: [RoomRow; 1] = [(
    6,
    "IntOctagon",
    [
        5100, 212171, 197929, 495000, -489900, -207071, 217271, 620000,
    ],
    8,
    0,
)];

const MAX8: [i32; 8] = [2147483647; 8];

const E1_INCOMPLETE: [RoomRow; 8] = [
    (
        -333950178,
        "Simplex",
        [400000, 100, 400000, 100, 0, 0, 0, 0],
        4,
        0,
    ),
    (1122139073, "Simplex", MAX8, 4, 0),
    (1469730785, "Simplex", MAX8, 4, 0),
    (98239, "Simplex", MAX8, 4, 0),
    (
        -1630793184,
        "Simplex",
        [400000, 200000, 400000, 200000, 0, 0, 0, 0],
        4,
        0,
    ),
    (
        -902178237,
        "Simplex",
        [600000, 100, 2147483647, 2147483647, 0, 0, 0, 0],
        4,
        0,
    ),
    (
        876257408,
        "Simplex",
        [560000, 400000, 600000, 480000, 0, 0, 0, 0],
        4,
        0,
    ),
    (
        -552812576,
        "Simplex",
        [400000, 400000, 440000, 480000, 0, 0, 0, 0],
        4,
        0,
    ),
];

const E2_INCOMPLETE: [RoomRow; 18] = [
    (1122139073, "Simplex", MAX8, 4, 0),
    (1469730785, "Simplex", MAX8, 4, 0),
    (98239, "Simplex", MAX8, 4, 0),
    (
        -1630793184,
        "Simplex",
        [400000, 200000, 400000, 200000, 0, 0, 0, 0],
        4,
        0,
    ),
    (
        -902178237,
        "Simplex",
        [600000, 100, 2147483647, 2147483647, 0, 0, 0, 0],
        4,
        0,
    ),
    (
        876257408,
        "Simplex",
        [560000, 400000, 600000, 480000, 0, 0, 0, 0],
        4,
        0,
    ),
    (
        -552812576,
        "Simplex",
        [400000, 400000, 440000, 480000, 0, 0, 0, 0],
        4,
        0,
    ),
    (
        -1871550178,
        "Simplex",
        [350000, 100, 350000, 100, 0, 0, 0, 0],
        4,
        0,
    ),
    (-1923300927, "Simplex", MAX8, 4, 0),
    (-336100927, "Simplex", MAX8, 4, 0),
    (98239, "Simplex", MAX8, 4, 0),
    (
        -2051752928,
        "Simplex",
        [350000, 200000, 350000, 200000, 0, 0, 0, 0],
        4,
        0,
    ),
    (
        1496506336,
        "Simplex",
        [200000, 400000, 200000, 400000, 0, 0, 0, 0],
        4,
        0,
    ),
    (3075231, "Simplex", MAX8, 4, 0),
    (595101761, "Simplex", MAX8, 4, 0),
    (-336100927, "Simplex", MAX8, 4, 0),
    (
        -1508344862,
        "Simplex",
        [100, 400000, 100, 400000, 0, 0, 0, 0],
        4,
        0,
    ),
    (
        1994004610,
        "Simplex",
        [400000, 400000, 499950, 599900, 0, 0, 0, 0],
        4,
        0,
    ),
];

const F1_INCOMPLETE: [RoomRow; 4] = [
    (
        -1706851826,
        "IntOctagon",
        [-1000, -1000, 197929, 601000, -602000, 198929, -2000, 620000],
        8,
        0,
    ),
    (
        1718166461,
        "IntOctagon",
        [
            402071, -1000, 1001000, 601000, -42929, 1002000, 401071, 1602000,
        ],
        8,
        0,
    ),
    (
        -1328282829,
        "IntOctagon",
        [
            435000, -1000, 1001000, 601000, -159900, 1002000, 977071, 1602000,
        ],
        8,
        0,
    ),
    (
        2058449700,
        "IntOctagon",
        [
            -1000, 419000, 125000, 601000, -602000, -420000, 418000, 719900,
        ],
        8,
        0,
    ),
];

const F2_INCOMPLETE: [RoomRow; 5] = [
    (
        1718166461,
        "IntOctagon",
        [
            402071, -1000, 1001000, 601000, -42929, 1002000, 401071, 1602000,
        ],
        8,
        0,
    ),
    (
        -1328282829,
        "IntOctagon",
        [
            435000, -1000, 1001000, 601000, -159900, 1002000, 977071, 1602000,
        ],
        8,
        0,
    ),
    (
        2058449700,
        "IntOctagon",
        [
            -1000, 419000, 125000, 601000, -602000, -420000, 418000, 719900,
        ],
        8,
        0,
    ),
    (
        1002583643,
        "IntOctagon",
        [
            5100, -1000, 1001000, 402071, -207071, 1002000, 4100, 1403071,
        ],
        8,
        0,
    ),
    (
        -1219568412,
        "IntOctagon",
        [
            -1000, 495000, 181000, 601000, -602000, -420000, 500100, 782000,
        ],
        8,
        0,
    ),
];

/// (room_id, door_index, other_is_complete, other_id (meaningful only
/// when complete — the capture prints -1/"null" otherwise), dimension,
/// shape class, coords, used)
type DoorRow = (i32, usize, bool, i32, i32, &'static str, [i32; 8], usize);

const E1_ROOM1_DOORS: [DoorRow; 8] = [
    (
        1,
        0,
        false,
        -1,
        1,
        "Simplex",
        [400000, 100, 400000, 400000, 0, 0, 0, 0],
        4,
    ),
    (
        1,
        1,
        false,
        -1,
        1,
        "Simplex",
        [440000, 480000, 560000, 480000, 0, 0, 0, 0],
        4,
    ),
    (
        1,
        2,
        false,
        -1,
        1,
        "Simplex",
        [600000, 100, 600000, 400000, 0, 0, 0, 0],
        4,
    ),
    (
        1,
        3,
        false,
        -1,
        1,
        "Simplex",
        [400000, 100, 600000, 100, 0, 0, 0, 0],
        4,
    ),
    (
        1,
        4,
        false,
        -1,
        1,
        "Simplex",
        [400000, 100, 400000, 200000, 0, 0, 0, 0],
        4,
    ),
    (
        1,
        5,
        false,
        -1,
        1,
        "Simplex",
        [600000, 100, 600000, 200000, 0, 0, 0, 0],
        4,
    ),
    (
        1,
        6,
        false,
        -1,
        1,
        "Simplex",
        [560000, 400000, 600000, 480000, 0, 0, 0, 0],
        4,
    ),
    (
        1,
        7,
        false,
        -1,
        1,
        "Simplex",
        [400000, 400000, 440000, 480000, 0, 0, 0, 0],
        4,
    ),
];

const E2_ROOM2_DOORS: [DoorRow; 6] = [
    (
        2,
        0,
        true,
        1,
        1,
        "Simplex",
        [400000, 100, 400000, 200000, 0, 0, 0, 0],
        4,
    ),
    (
        2,
        1,
        false,
        -1,
        1,
        "Simplex",
        [350000, 100, 350000, 200000, 0, 0, 0, 0],
        4,
    ),
    (
        2,
        2,
        false,
        -1,
        1,
        "Simplex",
        [350000, 200000, 400000, 200000, 0, 0, 0, 0],
        4,
    ),
    (
        2,
        3,
        false,
        -1,
        1,
        "Simplex",
        [400000, 100, 400000, 200000, 0, 0, 0, 0],
        4,
    ),
    (
        2,
        4,
        false,
        -1,
        1,
        "Simplex",
        [350000, 100, 400000, 100, 0, 0, 0, 0],
        4,
    ),
    (
        2,
        5,
        false,
        -1,
        1,
        "Simplex",
        [350000, 100, 350000, 200000, 0, 0, 0, 0],
        4,
    ),
];

const E2_ROOM4_DOORS: [DoorRow; 6] = [
    (
        4,
        0,
        false,
        -1,
        1,
        "Simplex",
        [100, 400000, 200000, 400000, 0, 0, 0, 0],
        4,
    ),
    (
        4,
        1,
        false,
        -1,
        1,
        "Simplex",
        [100, 400000, 100, 599900, 0, 0, 0, 0],
        4,
    ),
    (
        4,
        2,
        false,
        -1,
        1,
        "Simplex",
        [100, 599900, 400000, 599900, 0, 0, 0, 0],
        4,
    ),
    (
        4,
        3,
        false,
        -1,
        1,
        "Simplex",
        [400000, 400000, 400000, 599900, 0, 0, 0, 0],
        4,
    ),
    (
        4,
        4,
        false,
        -1,
        1,
        "Simplex",
        [100, 400000, 400000, 400000, 0, 0, 0, 0],
        4,
    ),
    (
        4,
        5,
        false,
        -1,
        1,
        "Simplex",
        [400000, 400000, 400000, 599900, 0, 0, 0, 0],
        4,
    ),
];

const F1_ROOM3_DOORS: [DoorRow; 4] = [
    (
        3,
        0,
        false,
        -1,
        2,
        "IntOctagon",
        [
            125000, 405000, 197929, 495000, -370000, -207071, 530000, 620000,
        ],
        8,
    ),
    (
        3,
        1,
        false,
        -1,
        2,
        "IntOctagon",
        [
            402071, 405000, 435000, 477929, -42929, 30000, 807071, 912929,
        ],
        8,
    ),
    (
        3,
        2,
        false,
        -1,
        1,
        "IntOctagon",
        [
            435000, 542071, 435000, 594900, -159900, -107071, 977071, 1029900,
        ],
        8,
    ),
    (
        3,
        3,
        false,
        -1,
        1,
        "IntOctagon",
        [
            125000, 545000, 125000, 594900, -469900, -420000, 670000, 719900,
        ],
        8,
    ),
];

const F2_ROOM6_DOORS: [DoorRow; 3] = [
    (
        6,
        0,
        true,
        3,
        2,
        "IntOctagon",
        [
            125000, 405000, 197929, 495000, -370000, -207071, 530000, 620000,
        ],
        8,
    ),
    (
        6,
        1,
        false,
        -1,
        1,
        "IntOctagon",
        [
            5100, 212171, 195000, 402071, -207071, -207071, 217271, 597071,
        ],
        8,
    ),
    (
        6,
        2,
        false,
        -1,
        1,
        "IntOctagon",
        [
            5100, 495000, 75000, 495000, -489900, -420000, 500100, 570000,
        ],
        8,
    ),
];

/// (tag, room_id, door_index, half_width, count, first segment,
/// second segment, second-to-last segment, last segment; each segment
/// [a.x, a.y, b.x, b.y]. For count == 1 all four segments are the
/// single degenerate segment (the digest prints first2 == last2).)
type SecRow = (
    u8,
    i32,
    usize,
    f64,
    usize,
    [f64; 4],
    [f64; 4],
    [f64; 4],
    [f64; 4],
);

#[rustfmt::skip]
const SECTION_ROWS: &[SecRow] = &[
    // E1 room 1
    (T_E1, 1, 0, 100.0, 393,
        [400000.0, 202.0, 400000.0, 1219.0381679389316],
        [400000.0, 1219.0381679389316, 400000.0, 2236.076335877863],
        [400000.0, 397863.9236641222, 400000.0, 398880.9618320611],
        [400000.0, 398880.9618320611, 400000.0, 399898.0]),
    (T_E1, 1, 0, 2000.0, 20,
        [400000.0, 2102.0, 400000.0, 21896.8],
        [400000.0, 21896.8, 400000.0, 41691.6],
        [400000.0, 358408.39999999997, 400000.0, 378203.2],
        [400000.0, 378203.2, 400000.0, 397998.0]),
    (T_E1, 1, 1, 100.0, 118,
        [440102.0, 480000.0, 441117.22033898305, 480000.0],
        [441117.22033898305, 480000.0, 442132.4406779661, 480000.0],
        [557867.5593220339, 480000.0, 558882.779661017, 480000.0],
        [558882.779661017, 480000.0, 559898.0, 480000.0]),
    (T_E1, 1, 1, 2000.0, 6,
        [442002.0, 480000.0, 461334.6666666667, 480000.0],
        [461334.6666666667, 480000.0, 480667.3333333333, 480000.0],
        [519332.6666666667, 480000.0, 538665.3333333334, 480000.0],
        [538665.3333333334, 480000.0, 557998.0, 480000.0]),
    (T_E1, 1, 2, 100.0, 393,
        [600000.0, 202.0, 600000.0, 1219.0381679389316],
        [600000.0, 1219.0381679389316, 600000.0, 2236.076335877863],
        [600000.0, 397863.9236641222, 600000.0, 398880.9618320611],
        [600000.0, 398880.9618320611, 600000.0, 399898.0]),
    (T_E1, 1, 2, 2000.0, 20,
        [600000.0, 2102.0, 600000.0, 21896.8],
        [600000.0, 21896.8, 600000.0, 41691.6],
        [600000.0, 358408.39999999997, 600000.0, 378203.2],
        [600000.0, 378203.2, 600000.0, 397998.0]),
    (T_E1, 1, 3, 100.0, 197,
        [400102.0, 100.0, 401116.192893401, 100.0],
        [401116.192893401, 100.0, 402130.38578680204, 100.0],
        [597869.614213198, 100.0, 598883.807106599, 100.0],
        [598883.807106599, 100.0, 599898.0, 100.0]),
    (T_E1, 1, 3, 2000.0, 10,
        [402002.0, 100.0, 421601.6, 100.0],
        [421601.6, 100.0, 441201.2, 100.0],
        [558798.8, 100.0, 578398.4, 100.0],
        [578398.4, 100.0, 597998.0, 100.0]),
    (T_E1, 1, 4, 100.0, 196,
        [400000.0, 202.0, 400000.0, 1220.857142857143],
        [400000.0, 1220.857142857143, 400000.0, 2239.714285714286],
        [400000.0, 197860.2857142857, 400000.0, 198879.14285714287],
        [400000.0, 198879.14285714287, 400000.0, 199898.0]),
    (T_E1, 1, 4, 2000.0, 10,
        [400000.0, 2102.0, 400000.0, 21691.6],
        [400000.0, 21691.6, 400000.0, 41281.2],
        [400000.0, 158818.8, 400000.0, 178408.4],
        [400000.0, 178408.4, 400000.0, 197998.0]),
    (T_E1, 1, 5, 100.0, 196,
        [600000.0, 202.0, 600000.0, 1220.857142857143],
        [600000.0, 1220.857142857143, 600000.0, 2239.714285714286],
        [600000.0, 197860.2857142857, 600000.0, 198879.14285714287],
        [600000.0, 198879.14285714287, 600000.0, 199898.0]),
    (T_E1, 1, 5, 2000.0, 10,
        [600000.0, 2102.0, 600000.0, 21691.6],
        [600000.0, 21691.6, 600000.0, 41281.2],
        [600000.0, 158818.8, 600000.0, 178408.4],
        [600000.0, 178408.4, 600000.0, 197998.0]),
    (T_E1, 1, 6, 100.0, 88,
        [599954.384213259, 400091.231573482, 599500.8754811394, 400998.249037721],
        [599500.8754811394, 400998.249037721, 599047.36674902, 401905.26650196005],
        [560952.63325098, 478094.7334980399, 560499.1245188606, 479001.750962279],
        [560499.1245188606, 479001.750962279, 560045.615786741, 479908.768426518]),
    (T_E1, 1, 6, 2000.0, 5,
        [599104.678381809, 401790.6432363818, 591462.8070290855, 417074.38594182907],
        [591462.8070290855, 417074.38594182907, 583820.9356763618, 432358.1286472764],
        [576179.0643236382, 447641.8713527236, 568537.1929709145, 462925.61405817093],
        [568537.1929709145, 462925.61405817093, 560895.321618191, 478209.3567636182]),
    (T_E1, 1, 7, 100.0, 88,
        [400045.615786741, 400091.231573482, 400499.12451886054, 400998.249037721],
        [400499.12451886054, 400998.249037721, 400952.63325098006, 401905.26650196005],
        [439047.36674901994, 478094.7334980399, 439500.87548113946, 479001.750962279],
        [439500.87548113946, 479001.750962279, 439954.384213259, 479908.768426518]),
    (T_E1, 1, 7, 2000.0, 5,
        [400895.3216181909, 401790.6432363818, 408537.19297091453, 417074.38594182907],
        [408537.19297091453, 417074.38594182907, 416179.06432363816, 432358.1286472764],
        [423820.93567636184, 447641.8713527236, 431462.80702908547, 462925.61405817093],
        [431462.80702908547, 462925.61405817093, 439104.6783818091, 478209.3567636182]),
    // E2 room 2
    (T_E2, 2, 0, 100.0, 196,
        [400000.0, 202.0, 400000.0, 1220.857142857143],
        [400000.0, 1220.857142857143, 400000.0, 2239.714285714286],
        [400000.0, 197860.2857142857, 400000.0, 198879.14285714287],
        [400000.0, 198879.14285714287, 400000.0, 199898.0]),
    (T_E2, 2, 0, 2000.0, 10,
        [400000.0, 2102.0, 400000.0, 21691.6],
        [400000.0, 21691.6, 400000.0, 41281.2],
        [400000.0, 158818.8, 400000.0, 178408.4],
        [400000.0, 178408.4, 400000.0, 197998.0]),
    (T_E2, 2, 1, 100.0, 196,
        [350000.0, 202.0, 350000.0, 1220.857142857143],
        [350000.0, 1220.857142857143, 350000.0, 2239.714285714286],
        [350000.0, 197860.2857142857, 350000.0, 198879.14285714287],
        [350000.0, 198879.14285714287, 350000.0, 199898.0]),
    (T_E2, 2, 1, 2000.0, 10,
        [350000.0, 2102.0, 350000.0, 21691.6],
        [350000.0, 21691.6, 350000.0, 41281.2],
        [350000.0, 158818.8, 350000.0, 178408.4],
        [350000.0, 178408.4, 350000.0, 197998.0]),
    (T_E2, 2, 2, 100.0, 50,
        [350102.0, 200000.0, 351097.92, 200000.0],
        [351097.92, 200000.0, 352093.84, 200000.0],
        [397906.16, 200000.0, 398902.08, 200000.0],
        [398902.08, 200000.0, 399898.0, 200000.0]),
    (T_E2, 2, 2, 2000.0, 3,
        [352002.0, 200000.0, 367334.0, 200000.0],
        [367334.0, 200000.0, 382666.0, 200000.0],
        [367334.0, 200000.0, 382666.0, 200000.0],
        [382666.0, 200000.0, 397998.0, 200000.0]),
    (T_E2, 2, 3, 100.0, 196,
        [400000.0, 202.0, 400000.0, 1220.857142857143],
        [400000.0, 1220.857142857143, 400000.0, 2239.714285714286],
        [400000.0, 197860.2857142857, 400000.0, 198879.14285714287],
        [400000.0, 198879.14285714287, 400000.0, 199898.0]),
    (T_E2, 2, 3, 2000.0, 10,
        [400000.0, 2102.0, 400000.0, 21691.6],
        [400000.0, 21691.6, 400000.0, 41281.2],
        [400000.0, 158818.8, 400000.0, 178408.4],
        [400000.0, 178408.4, 400000.0, 197998.0]),
    (T_E2, 2, 4, 100.0, 50,
        [350102.0, 100.0, 351097.92, 100.0],
        [351097.92, 100.0, 352093.84, 100.0],
        [397906.16, 100.0, 398902.08, 100.0],
        [398902.08, 100.0, 399898.0, 100.0]),
    (T_E2, 2, 4, 2000.0, 3,
        [352002.0, 100.0, 367334.0, 100.0],
        [367334.0, 100.0, 382666.0, 100.0],
        [367334.0, 100.0, 382666.0, 100.0],
        [382666.0, 100.0, 397998.0, 100.0]),
    (T_E2, 2, 5, 100.0, 196,
        [350000.0, 202.0, 350000.0, 1220.857142857143],
        [350000.0, 1220.857142857143, 350000.0, 2239.714285714286],
        [350000.0, 197860.2857142857, 350000.0, 198879.14285714287],
        [350000.0, 198879.14285714287, 350000.0, 199898.0]),
    (T_E2, 2, 5, 2000.0, 10,
        [350000.0, 2102.0, 350000.0, 21691.6],
        [350000.0, 21691.6, 350000.0, 41281.2],
        [350000.0, 158818.8, 350000.0, 178408.4],
        [350000.0, 178408.4, 350000.0, 197998.0]),
    // E2 room 4
    (T_E2, 4, 0, 100.0, 196,
        [202.0, 400000.0, 1220.857142857143, 400000.0],
        [1220.857142857143, 400000.0, 2239.714285714286, 400000.0],
        [197860.2857142857, 400000.0, 198879.14285714287, 400000.0],
        [198879.14285714287, 400000.0, 199898.0, 400000.0]),
    (T_E2, 4, 0, 2000.0, 10,
        [2102.0, 400000.0, 21691.6, 400000.0],
        [21691.6, 400000.0, 41281.2, 400000.0],
        [158818.8, 400000.0, 178408.4, 400000.0],
        [178408.4, 400000.0, 197998.0, 400000.0]),
    (T_E2, 4, 1, 100.0, 196,
        [100.0, 400102.0, 100.0, 401120.85714285716],
        [100.0, 401120.85714285716, 100.0, 402139.71428571426],
        [100.0, 597760.2857142857, 100.0, 598779.1428571428],
        [100.0, 598779.1428571428, 100.0, 599798.0]),
    (T_E2, 4, 1, 2000.0, 10,
        [100.0, 402002.0, 100.0, 421591.6],
        [100.0, 421591.6, 100.0, 441181.2],
        [100.0, 558718.8, 100.0, 578308.4],
        [100.0, 578308.4, 100.0, 597898.0]),
    (T_E2, 4, 2, 100.0, 393,
        [202.0, 599900.0, 1219.0381679389316, 599900.0],
        [1219.0381679389316, 599900.0, 2236.076335877863, 599900.0],
        [397863.9236641222, 599900.0, 398880.9618320611, 599900.0],
        [398880.9618320611, 599900.0, 399898.0, 599900.0]),
    (T_E2, 4, 2, 2000.0, 20,
        [2102.0, 599900.0, 21896.8, 599900.0],
        [21896.8, 599900.0, 41691.6, 599900.0],
        [358408.39999999997, 599900.0, 378203.2, 599900.0],
        [378203.2, 599900.0, 397998.0, 599900.0]),
    (T_E2, 4, 3, 100.0, 196,
        [400000.0, 400102.0, 400000.0, 401120.85714285716],
        [400000.0, 401120.85714285716, 400000.0, 402139.71428571426],
        [400000.0, 597760.2857142857, 400000.0, 598779.1428571428],
        [400000.0, 598779.1428571428, 400000.0, 599798.0]),
    (T_E2, 4, 3, 2000.0, 10,
        [400000.0, 402002.0, 400000.0, 421591.6],
        [400000.0, 421591.6, 400000.0, 441181.2],
        [400000.0, 558718.8, 400000.0, 578308.4],
        [400000.0, 578308.4, 400000.0, 597898.0]),
    (T_E2, 4, 4, 100.0, 393,
        [202.0, 400000.0, 1219.0381679389316, 400000.0],
        [1219.0381679389316, 400000.0, 2236.076335877863, 400000.0],
        [397863.9236641222, 400000.0, 398880.9618320611, 400000.0],
        [398880.9618320611, 400000.0, 399898.0, 400000.0]),
    (T_E2, 4, 4, 2000.0, 20,
        [2102.0, 400000.0, 21896.8, 400000.0],
        [21896.8, 400000.0, 41691.6, 400000.0],
        [358408.39999999997, 400000.0, 378203.2, 400000.0],
        [378203.2, 400000.0, 397998.0, 400000.0]),
    (T_E2, 4, 5, 100.0, 196,
        [400000.0, 400102.0, 400000.0, 401120.85714285716],
        [400000.0, 401120.85714285716, 400000.0, 402139.71428571426],
        [400000.0, 597760.2857142857, 400000.0, 598779.1428571428],
        [400000.0, 598779.1428571428, 400000.0, 599798.0]),
    (T_E2, 4, 5, 2000.0, 10,
        [400000.0, 402002.0, 400000.0, 421591.6],
        [400000.0, 421591.6, 400000.0, 441181.2],
        [400000.0, 558718.8, 400000.0, 578308.4],
        [400000.0, 578308.4, 400000.0, 597898.0]),
    // F1 room 3
    (T_F1, 3, 0, 100.0, 1,
        [152348.375, 440883.875, 152348.375, 440883.875],
        [152348.375, 440883.875, 152348.375, 440883.875],
        [152348.375, 440883.875, 152348.375, 440883.875],
        [152348.375, 440883.875, 152348.375, 440883.875]),
    (T_F1, 3, 0, 2000.0, 1,
        [152348.375, 440883.875, 152348.375, 440883.875],
        [152348.375, 440883.875, 152348.375, 440883.875],
        [152348.375, 440883.875, 152348.375, 440883.875],
        [152348.375, 440883.875, 152348.375, 440883.875]),
    (T_F1, 3, 1, 100.0, 1,
        [422651.625, 437348.375, 422651.625, 437348.375],
        [422651.625, 437348.375, 422651.625, 437348.375],
        [422651.625, 437348.375, 422651.625, 437348.375],
        [422651.625, 437348.375, 422651.625, 437348.375]),
    (T_F1, 3, 1, 2000.0, 1,
        [422651.625, 437348.375, 422651.625, 437348.375],
        [422651.625, 437348.375, 422651.625, 437348.375],
        [422651.625, 437348.375, 422651.625, 437348.375],
        [422651.625, 437348.375, 422651.625, 437348.375]),
    (T_F1, 3, 2, 100.0, 52,
        [435000.0, 542173.0, 435000.0, 543185.0192307692],
        [435000.0, 543185.0192307692, 435000.0, 544197.0384615385],
        [435000.0, 592773.9615384615, 435000.0, 593785.9807692308],
        [435000.0, 593785.9807692308, 435000.0, 594798.0]),
    (T_F1, 3, 2, 2000.0, 3,
        [435000.0, 544073.0, 435000.0, 560348.0],
        [435000.0, 560348.0, 435000.0, 576623.0],
        [435000.0, 560348.0, 435000.0, 576623.0],
        [435000.0, 576623.0, 435000.0, 592898.0]),
    (T_F1, 3, 3, 100.0, 49,
        [125000.0, 545102.0, 125000.0, 546116.2040816327],
        [125000.0, 546116.2040816327, 125000.0, 547130.4081632653],
        [125000.0, 592769.5918367347, 125000.0, 593783.7959183673],
        [125000.0, 593783.7959183673, 125000.0, 594798.0]),
    (T_F1, 3, 3, 2000.0, 3,
        [125000.0, 547002.0, 125000.0, 562300.6666666666],
        [125000.0, 562300.6666666666, 125000.0, 577599.3333333334],
        [125000.0, 562300.6666666666, 125000.0, 577599.3333333334],
        [125000.0, 577599.3333333334, 125000.0, 592898.0]),
    // F2 room 6
    (T_F2, 6, 0, 100.0, 114,
        [197864.78356453325, 405079.24802468164, 197226.18209848882, 405867.3313926697],
        [197226.18209848882, 405867.3313926697, 196587.58063244435, 406655.4147606577],
        [126341.41936755565, 493344.5852393423, 125702.8179015112, 494132.6686073303],
        [125702.8179015112, 494132.6686073303, 125064.21643546675, 494920.75197531836]),
    (T_F2, 6, 0, 2000.0, 6,
        [196668.59506074068, 406555.436719732, 184933.89670716046, 421036.9578131547],
        [184933.89670716046, 421036.9578131547, 173199.19835358023, 435518.47890657734],
        [149729.80164641977, 464481.52109342266, 137995.10329283954, 478963.0421868453],
        [137995.10329283954, 478963.0421868453, 126260.4049392593, 493444.563280268]),
    (T_F2, 6, 1, 100.0, 264,
        [5172.124891681028, 212243.12489168101, 5890.896672804657, 212961.89667280464],
        [5890.896672804657, 212961.89667280464, 6609.668453928285, 213680.66845392826],
        [193490.33154607174, 400561.33154607174, 194209.10332719536, 401280.1033271954],
        [194209.10332719536, 401280.1033271954, 194927.87510831899, 401998.875108319]),
    (T_F2, 6, 1, 2000.0, 14,
        [6515.627775935468, 213586.62777593546, 19877.68095080183, 226948.68095080182],
        [19877.68095080183, 226948.68095080182, 33239.73412566819, 240310.73412566818],
        [166860.26587433182, 373931.2658743318, 180222.31904919815, 387293.3190491982],
        [180222.31904919815, 387293.3190491982, 193584.37222406454, 400655.37222406454]),
    (T_F2, 6, 2, 100.0, 69,
        [5202.0, 495000.0, 6212.086956521739, 495000.0],
        [6212.086956521739, 495000.0, 7222.173913043478, 495000.0],
        [72877.82608695653, 495000.0, 73887.91304347826, 495000.0],
        [73887.91304347826, 495000.0, 74898.0, 495000.0]),
    (T_F2, 6, 2, 2000.0, 4,
        [7102.0, 495000.0, 23576.0, 495000.0],
        [23576.0, 495000.0, 40050.0, 495000.0],
        [40050.0, 495000.0, 56524.0, 495000.0],
        [56524.0, 495000.0, 72998.0, 495000.0]),
];

// ---- the replay harness ----

/// A dumped room: the capture row projection (the `used` width lives
/// on the literal row side; snapshots carry the full 8-slot array).
#[derive(Clone, Debug)]
struct RoomSnap {
    id: i32,
    class: &'static str,
    coords: [i32; 8],
    layer: i32,
}

/// A dumped door of a completed room.
#[derive(Clone, Debug)]
struct DoorSnap {
    dimension: i32,
    other_is_complete: bool,
    /// The other endpoint's registry id (the content formula id for
    /// incomplete rooms, the engine id for complete rooms).
    other_id: i32,
    class: &'static str,
    coords: [i32; 8],
    used: usize,
}

/// A dumped target door.
#[derive(Clone, Debug)]
struct TargetSnap {
    item_key: u64,
    tree_entry_no: u32,
    room_id: Option<i32>,
    class: &'static str,
    coords: [i32; 8],
    used: usize,
}

/// One completion round's outputs (the capture's per-tag rows).
struct RoundOut {
    completed_ids: Vec<i32>,
}

/// Full door context of one completed room at dump time — the capture
/// door row projection plus the endpoint shapes the section pins
/// replay through `get_section_segments`. Snapshots are taken per
/// round because later rounds CONSUME earlier rooms (E1 idx0 is
/// round 2's from-room), so lazy end-state fetches cannot reproduce
/// the per-tag dumps.
struct DoorCtx {
    room_id: i32,
    door_index: usize,
    both_complete: bool,
    door_shape: TileShape,
    first_shape: TileShape,
    second_shape: TileShape,
    door: ExpansionDoor,
    snap: DoorSnap,
}

/// Both rounds of one battery with the incomplete-room / door /
/// target-door snapshots taken after each round (the engine_state /
/// incomplete_room / door / target_door rows).
struct Battery {
    h: Harness,
    incomplete_after: Vec<Vec<RoomSnap>>,
    rounds: Vec<RoundOut>,
    /// Per round: for each completed room (in order), its door dump.
    doors_after: Vec<Vec<Vec<DoorCtx>>>,
    /// Per round: for each completed room (in order), its target dump.
    targets_after: Vec<Vec<(i32, Vec<TargetSnap>)>>,
}

/// The replay harness: the parsed fixture board + a variant-matched
/// search tree holding items AND completed rooms + the room registry
/// implementing [`NeighbourEngine`].
struct Harness {
    board: epic_board::board::Board,
    /// The manager-resolved tree variant for this battery (pin 6).
    resolved_variant: SearchTreeVariant,
    net: i32,
    /// The engine-side tree: items inserted at build (the same frozen
    /// shapes the manager's tree holds), completed rooms inserted by
    /// [`Harness::flush_completed_inserts`] after each completion
    /// round (Java inserts after `calculateDoors` returns).
    tree: SearchTree,
    /// Frozen item tree shapes per item key (the flows never mutate
    /// items; `item_tree_shapes` needs `&mut Board`, so the shapes are
    /// frozen once at build).
    item_shapes: BTreeMap<u64, Vec<Option<TileShape>>>,
    item_layers: BTreeMap<(u64, u32), i32>,
    rooms: Vec<ExpansionRoom>,
    keys: Vec<u64>,
    next_key: u64,
    id_counter: i32,
    /// Completed rooms created inside `complete()` awaiting the
    /// engine-side tree insert (abandoned restart attempts are
    /// dropped, only the returned room is inserted).
    pending_completed: Vec<u64>,
    /// Cached obstacle expansion rooms (Java
    /// `ItemAutorouteInfo.getExpansionRoom`).
    obstacle_rooms: BTreeMap<(u64, u32), u64>,
    /// The obstacle rooms whose door set is calculated
    /// (Java `ObstacleExpansionRoom.doorsCalculated`, T6 seam).
    obstacle_doors_calculated: BTreeSet<u64>,
}

impl Harness {
    fn build(net: i32, clearance_class: i32) -> Harness {
        let (mut manager, mut board) = crate::test_util::parse(FIXTURE_DSN);
        let tree_index = manager.get_autoroute_tree(&mut board, clearance_class);
        let resolved_variant = manager.trees()[tree_index].variant;
        let mut harness = Harness {
            board,
            resolved_variant,
            net,
            tree: SearchTree::new(resolved_variant, clearance_class),
            item_shapes: BTreeMap::new(),
            item_layers: BTreeMap::new(),
            rooms: Vec::new(),
            keys: Vec::new(),
            next_key: 0,
            id_counter: 0,
            pending_completed: Vec::new(),
            obstacle_rooms: BTreeMap::new(),
            obstacle_doors_calculated: BTreeSet::new(),
        };
        // Freeze the item tree shapes + layers (the same shapes
        // get_autoroute_tree just inserted into the manager's tree).
        // LIVE-only like the Java walk it mirrors (see
        // `SearchTreeManager::insert_all_board_items`).
        let ids: Vec<ItemId> = harness
            .board
            .iter_descending()
            .filter(|e| e.on_the_board)
            .map(|e| e.id)
            .collect();
        for id in ids {
            let key = u64::from(id.get());
            let shapes = epic_board::tree_shapes::item_tree_shapes(
                &mut harness.board,
                resolved_variant,
                clearance_class,
                id,
            );
            // The harness tree IS the engine tree: items first (the
            // manager's tree got the same shapes above).
            harness.tree.insert(key, &shapes);
            for (index, shape) in shapes.iter().enumerate() {
                if shape.is_some() {
                    let layer = harness
                        .board
                        .get(id)
                        .map(|e| item_data_layer(&e.data))
                        .unwrap_or(0);
                    harness.item_layers.insert((key, index as u32), layer);
                }
            }
            harness.item_shapes.insert(key, shapes);
        }
        harness
    }

    fn alloc_key(&mut self) -> u64 {
        self.next_key += 1;
        ROOM_KEY_BASE + self.next_key
    }

    fn idx_of(&self, key: u64) -> usize {
        self.keys
            .iter()
            .position(|&k| k == key)
            .unwrap_or_else(|| panic!("room key {key} not registered"))
    }

    fn room(&self, key: u64) -> &ExpansionRoom {
        &self.rooms[self.idx_of(key)]
    }

    fn room_mut(&mut self, key: u64) -> &mut ExpansionRoom {
        let i = self.idx_of(key);
        &mut self.rooms[i]
    }

    fn is_room_key(key: u64) -> bool {
        key >= ROOM_KEY_BASE
    }

    fn item_nets(&self, key: u64) -> &[i32] {
        let id = ItemId::new(u32::try_from(key).expect("item key fits u32"));
        self.board.get(id).map(|e| e.nets.as_slice()).unwrap_or(&[])
    }

    /// The most recently created room with `id` (the content formula
    /// ids of incomplete rooms legitimately collide across rounds —
    /// the capture shows 98239 twice in E2; the door endpoint is the
    /// room created last).
    fn room_key_of_id(&self, id: i32) -> Option<u64> {
        self.rooms
            .iter()
            .zip(&self.keys)
            .rev()
            .find(|(r, _)| r.id() == id)
            .map(|(_, k)| *k)
    }

    /// The door-driven exact endpoint resolution: the room of `id`
    /// whose door list holds an equal door value — the reference-
    /// faithful twin of [`Self::room_key_of_id`] (Java resolves door
    /// endpoints by object reference; the content-formula ids collide).
    fn room_key_of_door(&self, id: i32, door: &ExpansionDoor) -> Option<u64> {
        self.rooms
            .iter()
            .zip(&self.keys)
            .rev()
            .find(|(r, _)| r.id() == id && r.doors().iter().any(|d| d == door))
            .map(|(_, k)| *k)
    }

    fn first_incomplete(&self) -> Option<u64> {
        self.rooms
            .iter()
            .zip(&self.keys)
            .find(|(r, _)| r.is_incomplete())
            .map(|(_, k)| *k)
    }

    fn remove_room(&mut self, key: u64) {
        let i = self.idx_of(key);
        self.rooms.remove(i);
        self.keys.remove(i);
    }

    /// Java `engine.completeExpansionRoom` — the cell split of the
    /// from-room (with the FIRST ignore pair when the from-room has a
    /// door to a completed free-space neighbour, the E2/F2 flow), then
    /// the sorter over each cell, the from-room and consumed cells
    /// leaving the incomplete list, and the completed rooms entering
    /// the tree per cell. Returns the completed room ids in order.
    fn complete_expansion_room(&mut self, from_key: u64) -> Vec<i32> {
        let (shape, contained) = {
            let room = self.room(from_key);
            (
                room.shape().clone(),
                room.contained_shape()
                    .expect("incomplete room has a contained shape")
                    .clone(),
            )
        };
        let (ignore_object, ignore_shape) = self.first_ignore_pair(from_key);
        let cells = self.complete_shape(
            Some(&shape),
            Some(&contained),
            self.room(from_key).layer(),
            ignore_object,
            ignore_shape.as_ref(),
        );
        let mut completed_ids = Vec::new();
        let mut cell_keys = Vec::new();
        for cell in cells {
            let key = self.add_incomplete_expansion_room(
                cell.shape.clone(),
                cell.layer,
                cell.contained_shape.clone(),
            );
            cell_keys.push(key);
            let done_key = super::complete(self, key, self.resolved_variant);
            if let Some(done_key) = done_key {
                completed_ids.push(self.room(done_key).id());
            }
            // Java inserts the SURVIVING completed room after
            // calculateDoors returns; abandoned restart attempts only
            // burned an id and never reach the board/tree.
            self.flush_completed_inserts(done_key);
        }
        self.remove_room(from_key);
        for key in cell_keys {
            self.remove_room(key);
        }
        completed_ids
    }

    /// The FIRST door of `from_key` whose other endpoint is a
    /// completed free-space room — the (ignoreObject, ignoreShape)
    /// pair the capture shows flowing into the E2/F2 completeShape
    /// calls (the door shape to the completed neighbour).
    fn first_ignore_pair(&self, from_key: u64) -> (Option<u64>, Option<TileShape>) {
        let from = self.room(from_key);
        for door in from.doors() {
            if let Some(other_id) = door.other_room_id(from.id())
                && let Some(other_key) = self.room_key_of_door(other_id, door)
                && self.room(other_key).is_complete_free_space()
            {
                let shape =
                    ExpansionDoor::shape_between(from.shape(), self.room(other_key).shape());
                return (Some(other_key), Some(shape));
            }
        }
        (None, None)
    }

    /// Inserts the SURVIVING completed room of a `complete()` call and
    /// discards the abandoned restart attempts (they burned their ids —
    /// the capture's gaps 3 and 5 — but never reached the board: the
    /// F2 walk's dim-2 room entries must be room 3 ONLY, else the side
    /// marking and the completeShape restraint both diverge).
    fn flush_completed_inserts(&mut self, accepted: Option<u64>) {
        for key in std::mem::take(&mut self.pending_completed) {
            if Some(key) == accepted {
                let shape = self.room(key).shape().clone();
                self.tree.insert(key, &[Some(shape)]);
            } else {
                // drop the abandoned attempt from the registry too
                self.remove_room(key);
            }
        }
    }

    fn incomplete_snapshot(&self) -> Vec<RoomSnap> {
        self.rooms
            .iter()
            .filter(|r| r.is_incomplete())
            .map(|r| {
                let (class, coords, _) = shape_coords(r.shape());
                RoomSnap {
                    id: r.id(),
                    class,
                    coords,
                    layer: r.layer(),
                }
            })
            .collect()
    }

    fn doors_of_id(&self, room_id: i32) -> Vec<DoorCtx> {
        let key = self
            .room_key_of_id(room_id)
            .unwrap_or_else(|| panic!("room id {room_id} not registered"));
        let room = self.room(key);
        let mut out = Vec::new();
        for (door_index, door) in room.doors().iter().enumerate() {
            let other_id = door
                .other_room_id(room_id)
                .unwrap_or_else(|| panic!("door does not touch room {room_id}"));
            let other_key = self
                .room_key_of_door(other_id, door)
                .unwrap_or_else(|| panic!("door endpoint {other_id} not registered"));
            let other_complete = self.room(other_key).is_complete_free_space();
            let first_shape = room.shape().clone();
            let second_shape = self.room(other_key).shape().clone();
            let door_shape = ExpansionDoor::shape_between(&first_shape, &second_shape);
            let (class, coords, used) = shape_coords(&door_shape);
            out.push(DoorCtx {
                room_id,
                door_index,
                both_complete: other_complete,
                door_shape,
                first_shape,
                second_shape,
                door: door.clone(),
                snap: DoorSnap {
                    dimension: door.dimension,
                    other_is_complete: other_complete,
                    other_id,
                    class,
                    coords,
                    used,
                },
            });
        }
        out
    }

    fn targets_of_id(&self, room_id: i32) -> Vec<TargetSnap> {
        let key = self
            .room_key_of_id(room_id)
            .unwrap_or_else(|| panic!("room id {room_id} not registered"));
        self.room(key)
            .target_doors()
            .iter()
            .map(|t| {
                let (class, coords, used) = shape_coords(t.shape());
                TargetSnap {
                    item_key: t.item_key,
                    tree_entry_no: t.tree_entry_no,
                    room_id: t.room_id,
                    class,
                    coords,
                    used,
                }
            })
            .collect()
    }

    fn completed_shape_of_id(&self, room_id: i32) -> (&'static str, [i32; 8], usize, i32) {
        let key = self
            .room_key_of_id(room_id)
            .unwrap_or_else(|| panic!("room id {room_id} not registered"));
        let room = self.room(key);
        let (class, coords, used) = shape_coords(room.shape());
        (class, coords, used, room.layer())
    }
}

fn item_data_layer(data: &ItemData) -> i32 {
    match data {
        ItemData::Trace { layer, .. }
        | ItemData::ObstacleArea { layer, .. }
        | ItemData::ConductionArea { layer, .. }
        | ItemData::ComponentOutline { layer, .. } => *layer,
        // Pins/vias span layers; the fixture's pads live on layer 0.
        _ => 0,
    }
}

fn item_is_connectable_data(data: &ItemData) -> bool {
    matches!(
        data,
        ItemData::Trace { .. } | ItemData::Pin { .. } | ItemData::Via { .. }
    )
}

/// Runs one battery: seed + two completion rounds (the oracle's
/// E1_after_first / E2_after_second and F1/F2 flows). Door and
/// target-door dumps are taken per round — the capture tags them at
/// round time and later rounds consume earlier rooms.
fn run_battery(
    net: i32,
    clearance_class: i32,
    seed_shape: TileShape,
    seed_contained: TileShape,
) -> Battery {
    let mut h = Harness::build(net, clearance_class);
    let seed = h.add_incomplete_expansion_room(seed_shape, 0, seed_contained);
    let first = h.complete_expansion_room(seed);
    let mut battery = Battery {
        h,
        incomplete_after: Vec::new(),
        rounds: Vec::new(),
        doors_after: Vec::new(),
        targets_after: Vec::new(),
    };
    battery
        .incomplete_after
        .push(battery.h.incomplete_snapshot());
    battery
        .doors_after
        .push(first.iter().map(|&id| battery.h.doors_of_id(id)).collect());
    battery.targets_after.push(
        first
            .iter()
            .map(|&id| (id, battery.h.targets_of_id(id)))
            .collect(),
    );
    battery.rounds.push(RoundOut {
        completed_ids: first,
    });
    let from = battery
        .h
        .first_incomplete()
        .expect("an incomplete room survives round 1");
    let second = battery.h.complete_expansion_room(from);
    battery
        .incomplete_after
        .push(battery.h.incomplete_snapshot());
    battery
        .doors_after
        .push(second.iter().map(|&id| battery.h.doors_of_id(id)).collect());
    battery.targets_after.push(
        second
            .iter()
            .map(|&id| (id, battery.h.targets_of_id(id)))
            .collect(),
    );
    battery.rounds.push(RoundOut {
        completed_ids: second,
    });
    battery
}

impl NeighbourEngine for Harness {
    fn net_number(&self) -> i32 {
        self.net
    }

    fn generate_room_id_no(&mut self) -> i32 {
        self.id_counter += 1;
        self.id_counter
    }

    fn board_bounding_octagon(&self) -> IntOctagon {
        board_box_tile()
            .bounding_octagon()
            .expect("an IntBox always has a bounding octagon")
    }

    fn add_incomplete_expansion_room(
        &mut self,
        shape: TileShape,
        layer: i32,
        contained_shape: TileShape,
    ) -> u64 {
        let key = self.alloc_key();
        self.rooms
            .push(ExpansionRoom::new_incomplete(shape, layer, contained_shape));
        self.keys.push(key);
        key
    }

    fn remove_all_doors(&mut self, room_key: u64) {
        self.room_mut(room_key).clear_doors();
    }

    fn add_complete_free_space_room(&mut self, shape: TileShape, layer: i32, id: i32) -> u64 {
        let key = self.alloc_key();
        self.rooms
            .push(ExpansionRoom::new_complete_free_space(shape, layer, id));
        self.keys.push(key);
        self.pending_completed.push(key);
        key
    }

    fn overlapping_entries(&mut self, shape: &TileShape, layer: i32) -> Vec<TreeEntry> {
        let Some(leaves) = self.tree.query_candidates(shape, |a, b| a.cmp(&b)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for leaf in leaves {
            let Some(stored) =
                NeighbourEngine::tree_shape(self, leaf.object_key, leaf.shape_index_in_object)
            else {
                continue;
            };
            if layer >= 0 && self.shape_layer(leaf.object_key, leaf.shape_index_in_object) != layer
            {
                continue;
            }
            if !SearchTree::entry_intersects(shape, &stored) {
                continue;
            }
            out.push(TreeEntry {
                object_key: leaf.object_key,
                shape_index_in_object: leaf.shape_index_in_object,
            });
        }
        out
    }

    fn object_id(&self, object_key: u64) -> i32 {
        if Self::is_room_key(object_key) {
            self.room(object_key).id()
        } else {
            i32::try_from(object_key).expect("item keys are u32 ids")
        }
    }

    fn is_trace_obstacle(&self, object_key: u64, net_number: i32) -> bool {
        if Self::is_room_key(object_key) {
            let room = self.room(object_key);
            if room.is_complete_free_space() {
                // Java CompleteFreeSpaceExpansionRoom participates in
                // completeShape's restraint/ignore arms.
                return true;
            }
            if let RoomKind::Obstacle { item_key, .. } = room.kind {
                // Java queries the room's CONTAINED ITEM
                // (`SortedRoomNeighbours.java:217-223` casts the tree
                // entry to the item), so the VIRTUAL
                // `Item.isTraceObstacle` face applies (CA flag, keepout
                // kinds) via [`Board::item_is_trace_obstacle`].
                return board_item_is_trace_obstacle(&self.board, item_key, net_number);
            }
            return false;
        }
        board_item_is_trace_obstacle(&self.board, object_key, net_number)
    }

    fn tree_shape(&self, object_key: u64, shape_index: u32) -> Option<TileShape> {
        if Self::is_room_key(object_key) {
            return Some(self.room(object_key).shape().clone());
        }
        self.item_shapes
            .get(&object_key)
            .and_then(|shapes| shapes.get(shape_index as usize))
            .cloned()
            .flatten()
    }

    fn complete_shape(
        &mut self,
        room_shape: Option<&TileShape>,
        contained: Option<&TileShape>,
        layer: i32,
        ignore_object: Option<u64>,
        ignore_shape: Option<&TileShape>,
    ) -> Vec<IncompleteRoom> {
        let query = CompleteShapeQuery {
            room_shape,
            contained,
            layer,
            net_number: self.net,
            ignore_object,
            ignore_shape,
        };
        complete_shape(&self.tree, self, &query, &BOARD_BOX)
    }

    fn tree_object_room(&self, object_key: u64) -> Option<u64> {
        Self::is_room_key(object_key).then_some(object_key)
    }

    fn is_item(&self, object_key: u64) -> bool {
        !Self::is_room_key(object_key)
    }

    fn item_is_routable(&self, object_key: u64) -> bool {
        // Java `Item.isRoutable` (`Item.java:908-910`): the BASE
        // answers FALSE; only `Trace` and `Via` override with
        // `!isUserFixed() && netCount() > 0`. `Pin` has NO override,
        // so a pin answers FALSE — the walk's door arm gates
        // obstacle-door creation on this, so Java builds NO obstacle
        // door against a pin (the T4 F1 room-3 door-count witness:
        // item 5 is a pad). This MUST NOT fall back to connectability
        // (see the T6/T7 harness, `drill/pins.rs`, for the same fix).
        let id = ItemId::new(u32::try_from(object_key).expect("item key fits u32"));
        self.board
            .get(id)
            .map(|e| {
                matches!(e.data, ItemData::Trace { .. } | ItemData::Via { .. })
                    && !matches!(e.fixed, FixedState::UserFixed)
                    && !e.nets.is_empty()
            })
            .unwrap_or(false)
    }

    fn item_is_connectable(&self, object_key: u64) -> bool {
        // Java `Item.isConnectable` (`Item.java:912-914`):
        // `(this instanceof Connectable) && netCount() > 0` with
        // `Connectable` implemented by {Pin, Via, Trace,
        // ConductionArea} — INDEPENDENT of `isRoutable` (a pin is
        // connectable but NOT routable; see `item_is_routable` above).
        let id = ItemId::new(u32::try_from(object_key).expect("item key fits u32"));
        self.board
            .get(id)
            .map(|e| item_is_connectable_data(&e.data) && !e.nets.is_empty())
            .unwrap_or(false)
    }

    fn item_contains_net(&self, object_key: u64, net_number: i32) -> bool {
        self.item_nets(object_key).contains(&net_number)
    }

    fn item_shares_net(&self, first_key: u64, second_key: u64) -> bool {
        let first = self.item_nets(first_key);
        let second = self.item_nets(second_key);
        first.iter().any(|n| second.contains(n))
    }

    fn item_is_polyline_trace(&self, object_key: u64) -> bool {
        let id = ItemId::new(u32::try_from(object_key).expect("item key fits u32"));
        self.board
            .get(id)
            .map(|e| matches!(e.data, ItemData::Trace { .. }))
            .unwrap_or(false)
    }

    fn item_expansion_room(&mut self, object_key: u64, shape_index: u32) -> Option<u64> {
        if let Some(&cached) = self.obstacle_rooms.get(&(object_key, shape_index)) {
            return Some(cached);
        }
        let shape = NeighbourEngine::tree_shape(self, object_key, shape_index)?;
        let layer = self.shape_layer(object_key, shape_index);
        let key = self.alloc_key();
        self.rooms.push(ExpansionRoom::new_obstacle(
            object_key,
            shape_index,
            shape.clone(),
            layer,
        ));
        self.keys.push(key);
        // Java keeps ObstacleExpansionRoom OUT of the search tree (it is
        // a CompleteExpansionRoom, not a SearchTreeObject — it lives in
        // `ItemAutorouteInfo.expansionRoomArr`). See the T6/T7 harness
        // (`drill/pins.rs`) for the walk-pollution witness.
        self.obstacle_rooms.insert((object_key, shape_index), key);
        Some(key)
    }

    fn trace_connection_shape(&self, object_key: u64, shape_index: u32) -> Option<TileShape> {
        // NOTE: Java DrillItem.getTraceConnectionShape
        // (`DrillItem.java:358-361`) is the degenerate point box at the
        // item center, NOT the tree shape — the real four-arm dispatch is
        // ported in the T6 harness (`drill/pins.rs`); this T4-side stub
        // keeps the raw tree shape (the T4 fixture's target doors only
        // sit on pads).
        NeighbourEngine::tree_shape(self, object_key, shape_index)
    }

    fn trace_first_or_last_parallel(
        &self,
        _item_key: u64,
        _index_in_item: u32,
        _door_line: &Line,
    ) -> Option<bool> {
        // The fixture has no PolylineTrace items — the Java check
        // falls through to true (None = fall-through here).
        None
    }

    fn room_shape(&self, room_key: u64) -> TileShape {
        self.room(room_key).shape().clone()
    }

    fn room_layer(&self, room_key: u64) -> i32 {
        self.room(room_key).layer()
    }

    fn room_id(&self, room_key: u64) -> i32 {
        self.room(room_key).id()
    }

    fn room_is_incomplete(&self, room_key: u64) -> bool {
        self.room(room_key).is_incomplete()
    }

    fn room_is_obstacle(&self, room_key: u64) -> bool {
        self.room(room_key).is_obstacle()
    }

    fn room_is_complete_free_space(&self, room_key: u64) -> bool {
        self.room(room_key).is_complete_free_space()
    }

    fn room_contained_shape(&self, room_key: u64) -> Option<TileShape> {
        self.room(room_key).contained_shape().cloned()
    }

    fn room_obstacle_item_key(&self, room_key: u64) -> Option<u64> {
        match self.room(room_key).kind {
            RoomKind::Obstacle { item_key, .. } => Some(item_key),
            _ => None,
        }
    }

    fn room_obstacle_index_in_item(&self, room_key: u64) -> Option<u32> {
        match self.room(room_key).kind {
            RoomKind::Obstacle { index_in_item, .. } => Some(index_in_item),
            _ => None,
        }
    }

    fn room_has_door_to(&self, room_key: u64, other_room_id: i32) -> bool {
        self.room(room_key).door_exists(other_room_id)
    }

    fn room_doors(&self, room_key: u64) -> Vec<ExpansionDoor> {
        self.room(room_key).doors().to_vec()
    }

    fn room_key_of_id(&self, id: i32) -> Option<u64> {
        self.room_key_of_id(id)
    }

    fn room_key_of_door(&self, id: i32, door: &ExpansionDoor) -> Option<u64> {
        self.room_key_of_door(id, door)
    }

    fn set_incomplete_shape(
        &mut self,
        room_key: u64,
        shape: TileShape,
        contained_shape: TileShape,
    ) {
        let room = self.room_mut(room_key);
        room.set_shape(shape);
        room.set_contained_shape(contained_shape);
    }

    fn set_room_shape(&mut self, room_key: u64, shape: TileShape) {
        self.room_mut(room_key).set_shape(shape);
    }

    fn attach_door(&mut self, room_key: u64, door: ExpansionDoor) {
        self.room_mut(room_key).add_door(door);
    }

    fn add_target_door(&mut self, room_key: u64, door: TargetItemExpansionDoor) {
        self.room_mut(room_key).add_target_door(door);
    }

    fn set_net_dependent(&mut self, room_key: u64) {
        self.room_mut(room_key).set_net_dependent(true);
    }

    // ---- T6 seams ----
    fn tree_variant(&self) -> SearchTreeVariant {
        self.resolved_variant
    }

    fn remove_incomplete_room(&mut self, room_key: u64) {
        self.remove_room(room_key);
    }

    fn flush_completed_inserts(&mut self, accepted: Option<u64>) {
        // The inherent helper (same contract; inherent resolution
        // wins over the trait method, so this is not recursion).
        Harness::flush_completed_inserts(self, accepted);
    }

    fn room_target_doors(&self, room_key: u64) -> Vec<TargetItemExpansionDoor> {
        self.room(room_key).target_doors().to_vec()
    }

    fn room_obstacle_doors_calculated(&self, room_key: u64) -> bool {
        self.obstacle_doors_calculated.contains(&room_key)
    }

    fn set_room_doors_calculated(&mut self, room_key: u64) {
        self.obstacle_doors_calculated.insert(room_key);
    }
}

impl CompleteShapeObjects for Harness {
    fn is_trace_obstacle(&self, object_key: u64, net_number: i32) -> bool {
        NeighbourEngine::is_trace_obstacle(self, object_key, net_number)
    }

    fn shape_layer(&self, object_key: u64, shape_index: u32) -> i32 {
        if Self::is_room_key(object_key) {
            return self.room(object_key).layer();
        }
        self.item_layers
            .get(&(object_key, shape_index))
            .copied()
            .expect("live tree shapes carry a layer")
    }

    fn tree_shape(&self, object_key: u64, shape_index: u32) -> Option<TileShape> {
        NeighbourEngine::tree_shape(self, object_key, shape_index)
    }

    fn is_complete_free_space(&self, object_key: u64) -> bool {
        Self::is_room_key(object_key) && self.room(object_key).is_complete_free_space()
    }
}

// ---- shared assertion helpers ----

fn assert_room_seq(rows: &[RoomRow], snaps: &[RoomSnap], what: &str) {
    assert_eq!(snaps.len(), rows.len(), "{what}: room count");
    for (i, (snap, row)) in snaps.iter().zip(rows.iter()).enumerate() {
        let (id, class, coords, used, layer) = *row;
        assert_eq!(snap.id, id, "{what}[{i}]: room id");
        assert_eq!(snap.class, class, "{what}[{i}]: shape class");
        assert_eq!(
            &snap.coords[..used],
            &coords[..used],
            "{what}[{i}]: shape coords"
        );
        assert_eq!(snap.layer, layer, "{what}[{i}]: layer");
    }
}

fn assert_completed_seq(h: &Harness, ids: &[i32], rows: &[RoomRow], what: &str) {
    assert_eq!(ids.len(), rows.len(), "{what}: completed count");
    for (i, (id, row)) in ids.iter().zip(rows.iter()).enumerate() {
        assert_eq!(*id, row.0, "{what}[{i}]: room id");
        let (class, coords, used, layer) = h.completed_shape_of_id(*id);
        assert_eq!(class, row.1, "{what}[{i}]: shape class");
        assert_eq!(&coords[..used], &row.2[..used], "{what}[{i}]: shape coords");
        assert_eq!(layer, row.4, "{what}[{i}]: layer");
    }
}

fn assert_door_seq(ctxs: &[DoorCtx], rows: &[DoorRow], what: &str) {
    assert_eq!(ctxs.len(), rows.len(), "{what}: door count");
    for (i, (ctx, row)) in ctxs.iter().zip(rows.iter()).enumerate() {
        let (_, door_index, other_complete, other_id, dimension, class, coords, used) = *row;
        assert_eq!(i, door_index, "{what}[{i}]: door index");
        assert_eq!(ctx.snap.dimension, dimension, "{what}[{i}]: dimension");
        assert_eq!(
            ctx.snap.other_is_complete, other_complete,
            "{what}[{i}]: other endpoint kind"
        );
        if other_complete {
            assert_eq!(ctx.snap.other_id, other_id, "{what}[{i}]: other room id");
        }
        assert_eq!(ctx.snap.class, class, "{what}[{i}]: shape class");
        assert_eq!(
            &ctx.snap.coords[..used],
            &coords[..used],
            "{what}[{i}]: shape coords"
        );
    }
}

/// The door dump of one completed room from a round snapshot.
fn doors_of_round(battery: &Battery, round: usize, room_id: i32) -> &[DoorCtx] {
    battery.doors_after[round]
        .iter()
        .find(|ctxs| ctxs.first().is_some_and(|c| c.room_id == room_id))
        .unwrap_or_else(|| panic!("round {round} has no doors for room {room_id}"))
}

/// The target dump of one completed room from a round snapshot.
fn targets_of_round(battery: &Battery, round: usize, room_id: i32) -> &[TargetSnap] {
    &battery.targets_after[round]
        .iter()
        .find(|(id, _)| *id == room_id)
        .unwrap_or_else(|| panic!("round {round} has no target dump for room {room_id}"))
        .1
}

// ---- the seven pins ----

/// Pin 1a — the E battery (ANY_ANGLE sorter): net 1, engine_state row
/// `{"tag":"E1_after_first","net":1}`; completed room 1 after round 1
/// and rooms 2, 4 after round 2 (the id 3 gap is a burned restart
/// id); the incomplete_room sequences (8 + 18 rows) and the door
/// sequences of rooms 1, 2 and 4, all verbatim capture rows.
#[test]
fn neighbour_order_e_battery_base_sorter() {
    let battery = run_battery(1, 0, board_box_tile(), corridor_contained());
    assert_eq!(battery.h.net, 1, "engine_state E rows: net 1");

    // Round 1 (E1_after_first).
    assert_eq!(battery.rounds[0].completed_ids, [1], "E1 completed rooms");
    assert_completed_seq(
        &battery.h,
        &battery.rounds[0].completed_ids,
        &E1_COMPLETED,
        "E1",
    );
    assert_room_seq(
        &E1_INCOMPLETE,
        &battery.incomplete_after[0],
        "E1 incomplete",
    );
    assert_door_seq(
        doors_of_round(&battery, 0, 1),
        &E1_ROOM1_DOORS,
        "E1 room 1 doors",
    );

    // Round 2 (E2_after_second).
    assert_eq!(
        battery.rounds[1].completed_ids,
        [2, 4],
        "E2 completed rooms"
    );
    assert_completed_seq(
        &battery.h,
        &battery.rounds[1].completed_ids,
        &E2_COMPLETED,
        "E2",
    );
    assert_room_seq(
        &E2_INCOMPLETE,
        &battery.incomplete_after[1],
        "E2 incomplete",
    );
    assert_door_seq(
        doors_of_round(&battery, 1, 2),
        &E2_ROOM2_DOORS,
        "E2 room 2 doors",
    );
    assert_door_seq(
        doors_of_round(&battery, 1, 4),
        &E2_ROOM4_DOORS,
        "E2 room 4 doors",
    );
}

/// Pin 1b — the F battery (DEGREE_45 sorter): net 2, completed rooms
/// 3 and 6 (ids 1, 2 and 4, 5 burned by restarts), the incomplete
/// sequences (4 + 5 rows) and the door sequences of rooms 3 and 6.
#[test]
fn neighbour_order_f_battery_forty_five_sorter() {
    let battery = run_battery(2, f_engine_class(), board_oct_tile(), f_contained());
    assert_eq!(battery.h.net, 2, "engine_state F rows: net 2");

    assert_eq!(battery.rounds[0].completed_ids, [3], "F1 completed rooms");
    assert_completed_seq(
        &battery.h,
        &battery.rounds[0].completed_ids,
        &F1_COMPLETED,
        "F1",
    );
    assert_room_seq(
        &F1_INCOMPLETE,
        &battery.incomplete_after[0],
        "F1 incomplete",
    );
    assert_door_seq(
        doors_of_round(&battery, 0, 3),
        &F1_ROOM3_DOORS,
        "F1 room 3 doors",
    );

    assert_eq!(battery.rounds[1].completed_ids, [6], "F2 completed rooms");
    assert_completed_seq(
        &battery.h,
        &battery.rounds[1].completed_ids,
        &F2_COMPLETED,
        "F2",
    );
    assert_room_seq(
        &F2_INCOMPLETE,
        &battery.incomplete_after[1],
        "F2 incomplete",
    );
    assert_door_seq(
        doors_of_round(&battery, 1, 6),
        &F2_ROOM6_DOORS,
        "F2 room 6 doors",
    );
}

/// Pin 2 — door section segments at the captured half widths (100.0 /
/// 2000.0): first/second and second-to-last/last segments plus the
/// section count for ALL 54 digested door_sections rows, spanning the
/// three section regimes (1-dim diagonal-corner shrink, 2-dim
/// gravity-point collapse with count 1, 2-dim complete-complete
/// restraint line at F2 room 6 door 0).
#[test]
fn door_sections_at_captured_half_widths() {
    let e = run_battery(1, 0, board_box_tile(), corridor_contained());
    let f = run_battery(2, f_engine_class(), board_oct_tile(), f_contained());
    for (tag, room_id, door_index, half_width, count, fa, fb, la, lb) in
        SECTION_ROWS.iter().copied()
    {
        let battery = match tag {
            T_E1 | T_E2 => &e,
            _ => &f,
        };
        let round = match tag {
            T_E1 | T_F1 => 0,
            _ => 1,
        };
        let ctx = &doors_of_round(battery, round, room_id)[door_index];
        assert_eq!(
            ctx.door_index, door_index,
            "tag {tag} room {room_id}: door index"
        );
        let (section_count, segments) = ctx.door.get_section_segments(
            &ctx.door_shape,
            ctx.both_complete,
            &ctx.first_shape,
            &ctx.second_shape,
            half_width,
        );
        let what = format!("tag {tag} room {room_id} door {door_index} hw {half_width}");
        assert_eq!(section_count, count, "{what}: section count");
        assert_eq!(segments.len(), count, "{what}: segment list length");
        let seg = |i: usize| {
            let s = &segments[i];
            [s.a.x, s.a.y, s.b.x, s.b.y]
        };
        assert_eq!(seg(0), fa, "{what}: first segment");
        assert_eq!(seg(count - 1), lb, "{what}: last segment");
        if count >= 2 {
            assert_eq!(seg(1), fb, "{what}: second segment");
            assert_eq!(seg(count - 2), la, "{what}: second-to-last segment");
        } else {
            assert_eq!(
                seg(0),
                la,
                "{what}: single segment (digest first2 == last2)"
            );
        }
    }
}

/// Pin 3 — room completion shapes through the REAL parsed trees: the
/// S probes (plain tree, net 1) and T probes (45-degree tree, net 2)
/// verbatim from the complete_shape_room rows, plus the guard arms
/// S2_null_contained / S4_empty_tree / S5_room_shape_not_octagon
/// (result_count 0 each).
#[test]
fn room_completion_shapes() {
    // S1_corridor / S3_simplex on the plain tree.
    let mut e = Harness::build(1, 0);
    let s1 = e.complete_shape(
        Some(&board_box_tile()),
        Some(&corridor_contained()),
        0,
        None,
        None,
    );
    assert_eq!(s1.len(), 1, "S1_corridor: result_count");
    let (class, coords, used, layer) = room_triple(&s1[0]);
    assert_coords(
        (class, coords, used),
        "Simplex",
        &[400000, 100, 600000, 480000, 0, 0, 0, 0],
        4,
        "S1 shape",
    );
    assert_coords(
        shape_coords(&s1[0].contained_shape),
        "Simplex",
        &[450000, 250000, 550000, 350000, 0, 0, 0, 0],
        4,
        "S1 contained",
    );
    assert_eq!(layer, 0, "S1 layer");

    let s3 = e.complete_shape(
        Some(&board_box_tile()),
        Some(&triangle_contained()),
        0,
        None,
        None,
    );
    assert_eq!(s3.len(), 1, "S3_simplex: result_count");
    let (class, coords, used, _) = room_triple(&s3[0]);
    assert_coords(
        (class, coords, used),
        "Simplex",
        &[400000, 250000, 600000, 480000, 0, 0, 0, 0],
        4,
        "S3 shape",
    );
    assert_coords(
        shape_coords(&s3[0].contained_shape),
        "Simplex",
        &[490000, 360000, 510000, 380000, 0, 0, 0, 0],
        4,
        "S3 contained",
    );

    // T1_corridor / T3_simplex on the 45-degree tree (net 2). The
    // oracle pins these probes on the LITERAL class-1 tree45
    // (`getAutorouteTree(1)`), NOT the NET_B class tree the F battery
    // uses — the class-1 dilation (1250) is what the T capture rows
    // encode.
    let mut f = Harness::build(2, 1);
    let t1 = f.complete_shape(
        Some(&board_oct_tile()),
        Some(&corridor_contained()),
        0,
        None,
        None,
    );
    assert_eq!(t1.len(), 1, "T1_corridor: result_count");
    let (class, coords, used, layer) = room_triple(&t1[0]);
    assert_coords(
        (class, coords, used),
        "IntOctagon",
        &[
            401250, 1350, 598750, 478750, -77500, 597400, 402600, 1077500,
        ],
        8,
        "T1 shape",
    );
    assert_coords(
        shape_coords(&t1[0].contained_shape),
        "IntOctagon",
        &[
            450000, 250000, 550000, 350000, 100000, 300000, 700000, 900000,
        ],
        8,
        "T1 contained",
    );
    assert_eq!(layer, 0, "T1 layer");

    let t3 = f.complete_shape(
        Some(&board_oct_tile()),
        Some(&triangle_contained()),
        0,
        None,
        None,
    );
    assert_eq!(t3.len(), 1, "T3_simplex: result_count");
    let (class, coords, used, _) = room_triple(&t3[0]);
    assert_coords(
        (class, coords, used),
        "IntOctagon",
        &[
            401250, 239250, 598750, 478750, -77500, 359500, 640500, 1077500,
        ],
        8,
        "T3 shape",
    );
    assert_coords(
        shape_coords(&t3[0].contained_shape),
        "IntOctagon",
        &[
            490000, 360000, 510000, 380000, 120000, 150000, 850000, 880000,
        ],
        8,
        "T3 contained",
    );

    // S2_null_contained: null contained shape → empty (base arm).
    assert!(
        e.complete_shape(Some(&board_box_tile()), None, 0, None, None)
            .is_empty(),
        "S2_null_contained: result_count 0"
    );

    // S4_empty_tree: a fresh empty tree → empty (both arms).
    let empty_base = SearchTree::new(SearchTreeVariant::Generic, 0);
    let query = CompleteShapeQuery {
        room_shape: Some(&board_box_tile()),
        contained: Some(&corridor_contained()),
        layer: 0,
        net_number: 1,
        ignore_object: None,
        ignore_shape: None,
    };
    assert!(
        complete_shape(&empty_base, &e, &query, &BOARD_BOX).is_empty(),
        "S4_empty_tree (base arm): result_count 0"
    );
    let empty_45 = SearchTree::new(SearchTreeVariant::FortyfiveDegree, 1);
    let query45 = CompleteShapeQuery {
        room_shape: Some(&board_oct_tile()),
        contained: Some(&corridor_contained()),
        layer: 0,
        net_number: 2,
        ignore_object: None,
        ignore_shape: None,
    };
    assert!(
        complete_shape(&empty_45, &f, &query45, &BOARD_BOX).is_empty(),
        "S4_empty_tree (45 arm): result_count 0"
    );

    // S5_room_shape_not_octagon: a box room shape through the 45 arm
    // → empty.
    assert!(
        f.complete_shape(
            Some(&board_box_tile()),
            Some(&corridor_contained()),
            0,
            None,
            None
        )
        .is_empty(),
        "S5_room_shape_not_octagon: result_count 0"
    );
}

fn room_triple(room: &IncompleteRoom) -> (&'static str, [i32; 8], usize, i32) {
    let (class, coords, used) = shape_coords(&room.shape);
    (class, coords, used, room.layer)
}

/// Pin 4 — the 45-degree sorter's counterclockwise orientation on the
/// DIAGONAL sides (the reversed corner-sign band vs the orthogonal
/// sides, SEAM.md): room 3's and room 6's neighbour walks traverse
/// the ±45-degree border sides of their octagons — F1 doors 0/1 are
/// the dimension-2 overlaps on room 3's lower-left/lower-right
/// diagonal sides, F2 room 6 door 0 is the dimension-2 overlap on the
/// upper-left diagonal (its captured octagon carries the exact
/// -45-degree border through the normalized diagonal coordinates).
/// A sides 0-3 sign flip of the 45-degree `compareTo`
/// (neighbours_forty_five) reverses the walk order (the F2 side-1
/// tile/room3 tie) and must fail this pin; a sides 4-7 flip is
/// INVISIBLE here (no F1/F2 neighbours share a 4-7 first side) and
/// is pinned by the `compare_to_diagonal_band_signs` unit test in
/// neighbours_forty_five instead.
#[test]
fn forty_five_orientation_walk_order() {
    let battery = run_battery(2, f_engine_class(), board_oct_tile(), f_contained());

    // F1: the walk order around room 3's diagonal sides.
    assert_room_seq(
        &F1_INCOMPLETE,
        &battery.incomplete_after[0],
        "F1 incomplete",
    );
    assert_door_seq(
        doors_of_round(&battery, 0, 3),
        &F1_ROOM3_DOORS,
        "F1 room 3 doors",
    );

    // F2: the walk order around room 6's diagonal sides.
    assert_room_seq(
        &F2_INCOMPLETE,
        &battery.incomplete_after[1],
        "F2 incomplete",
    );
    assert_door_seq(
        doors_of_round(&battery, 1, 6),
        &F2_ROOM6_DOORS,
        "F2 room 6 doors",
    );
}

/// Pin 5 — E2 room 4's VALUE-IDENTICAL doors d3/d5 ([400000,400000,
/// 400000,599900] twice): both present (6 doors total), shapes equal,
/// endpoint room ids DISTINCT — a value-equality door model would
/// collapse them (cerebrum mode 7 contrast: identical values,
/// different endpoints).
#[test]
fn value_identical_doors_distinct_by_endpoints() {
    let battery = run_battery(1, 0, board_box_tile(), corridor_contained());
    let doors = doors_of_round(&battery, 1, 4);
    assert_eq!(doors.len(), 6, "room 4 door count");
    let d3 = &doors[3].snap;
    let d5 = &doors[5].snap;
    assert_eq!(
        &d3.coords[..d3.used],
        &[400000, 400000, 400000, 599900],
        "d3 shape"
    );
    assert_eq!(
        &d5.coords[..d5.used],
        &[400000, 400000, 400000, 599900],
        "d5 shape (value-identical to d3)"
    );
    assert!(
        !d3.other_is_complete && !d5.other_is_complete,
        "both endpoints incomplete"
    );
    assert_ne!(d3.other_id, d5.other_id, "endpoint room ids DISTINCT");
}

/// Pin 6 — dispatch mode selection (the capture `dispatch` row):
/// `select_calculation_mode` maps Generic → AnyAngle (the plain
/// ShapeSearchTree), FortyfiveDegree → Degree45, NinetyDegree →
/// Orthogonal; the replayed trees carry those variants. The
/// ORTHOGONAL arm is unreachable by construction on this corpus: it
/// routes to the unported `SortedOrthogonalRoomNeighbours` behind a
/// debug_assert, and no 90-degree ROUTED fixture exists (`snap_angle`
/// defaults FortyfiveDegree at epic-dsn/src/state.rs:495; NONE of the
/// 23 tier fixtures overrides it — tier A is 11 boards per
/// rust/harness/config/tiers.yaml — and the corpus's single
/// `snap_angle` override, `ninety_degree` in the index-stress fixture
/// deg90.dsn, never routes through the expansion graph).
#[test]
fn dispatch_mode_selection() {
    assert_eq!(
        select_calculation_mode(SearchTreeVariant::Generic),
        CalculationMode::AnyAngle,
        "dispatch row: tree0_class ShapeSearchTree → ANY_ANGLE"
    );
    assert_eq!(
        select_calculation_mode(SearchTreeVariant::FortyfiveDegree),
        CalculationMode::Degree45,
        "dispatch row: tree45_class ShapeSearchTree45Degree → DEGREE_45"
    );
    assert_eq!(
        select_calculation_mode(SearchTreeVariant::NinetyDegree),
        CalculationMode::Orthogonal,
        "dispatch row: 90-degree → ORTHOGONAL (unreachable on this corpus)"
    );

    let e = Harness::build(1, 0);
    assert_eq!(
        e.resolved_variant,
        SearchTreeVariant::Generic,
        "E battery resolves the plain tree"
    );
    let f = Harness::build(2, f_engine_class());
    assert_eq!(
        f.resolved_variant,
        SearchTreeVariant::FortyfiveDegree,
        "F battery resolves the 45-degree tree"
    );
}

/// Pin 7 — the E2 target door (the capture's single target_door row):
/// room 4 carries exactly one TargetItemExpansionDoor to the NET_A
/// pad (item id 5 in the capture, cross-checked by net lookup), shape
/// = pad tree shape ∩ room shape with bounding box
/// [80000,500000,120000,540000]; rooms 1 and 2 carry none.
#[test]
fn target_door_e2_room4() {
    let battery = run_battery(1, 0, board_box_tile(), corridor_contained());

    // Cross-check the capture's item id 5 against the net lookup: it
    // is the NET_A pin.
    let net_a = crate::test_util::net_no(&battery.h.board, "NET_A");
    assert_eq!(net_a, 1, "engine_state E rows: net 1");
    let id5 = ItemId::new(5);
    let entry = battery
        .h
        .board
        .get(id5)
        .expect("item id 5 exists (capture target_door item_id)");
    assert!(
        matches!(entry.data, ItemData::Pin { .. }),
        "item 5 is a pin"
    );
    assert!(entry.nets.contains(&net_a), "item 5 belongs to NET_A");

    let targets = targets_of_round(&battery, 1, 4);
    assert_eq!(targets.len(), 1, "room 4 target door count");
    let t = &targets[0];
    assert_eq!(t.item_key, 5, "target door item id");
    assert_eq!(t.tree_entry_no, 0, "target door tree entry");
    assert_eq!(t.room_id, Some(4), "target door room");
    assert_coords(
        (t.class, t.coords, t.used),
        "Simplex",
        &[80000, 500000, 120000, 540000, 0, 0, 0, 0],
        4,
        "target door shape",
    );

    assert!(
        targets_of_round(&battery, 0, 1).is_empty(),
        "room 1 has no target door"
    );
    assert!(
        targets_of_round(&battery, 1, 2).is_empty(),
        "room 2 has no target door"
    );
}
