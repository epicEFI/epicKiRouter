//! Crafted DSN boards + parse/lookup helpers for the epic-drc pin
//! suites (compiled for `cargo test` only). The boards reuse the
//! proven crafted-header shape of `epic-board::contacts` tests
//! (PURE_DSN): `(resolution um 1)`, empty nets legal, structure
//! keepouts legal, wiring items parsed in section order.
//!
//! Lookup discipline (cerebrum pin mode 8): NO absolute item ids in
//! pins — items are found by net membership ([`net_list`]), by kind
//! ([`ids_of_kind`]), or by ratsnest corner geometry ([`corner`]).
//! Parse order is pinned only RELATIVELY (which member sits at which
//! coordinate), never as a raw id literal.

use epic_board::board::Board;
use epic_board::id::ItemId;
use epic_board::items::{BoardItemType, ItemData};
use epic_board::tree_manager::SearchTreeManager;
use epic_dsn::reader::{DsnReadResult, read_board};
use epic_dsn::ses_board::SesBoard;

/// The main craft. Geometry (world coordinates; component CMP1 is
/// placed at (10000, 10000), so world = image offset + (10000,10000);
/// F.Cu is layer 0, rule clearance 2000, trace half width 125):
///
/// * net NQ — 4 isolated pins (PAD_R, F.Cu-only rect ±1000):
///   P1 (10000,10000), P2 (30000,10000), P3 (10000,30000),
///   P4 (70000,70000). Convex quad, diagonals P2–P3 = 8e8 vs
///   P1–P4 = 72e8 — STRICTLY non-cocircular (P3 sits inside
///   circumcircle(P1,P2,P4), incircle margin ≈ 1.6e9) — yet the
///   oracle's Delaunay edge set is the 4 hull pairs + the P1–P4
///   diagonal (NO P2–P3; bug-compat with Java's non-ideal Delaunay)
///   and `count == groups − 1 = 3` (the CONFORMING pin).
/// * net NC — two uncontacted wiring-rect CONDUCTION AREAS
///   (60000..70000 × 5000..15000 and 130000..140000 × 65000..75000):
///   CA endpoints feed `maxConnections` (2 → 1), ratsnest = 4
///   corners each, and the surface for the ConductionArea-extends-
///   ObstacleArea trap cells of the clearance matrix.
/// * net N3 — pins (100000,70000), (120000,70000) + its own open CA
///   at (130000..140000 × 65000..75000), stacked on NC's second area
///   (open CA vs open CA is non-obstacle BOTH ways, so the stack adds
///   no clearance row): 3 endpoints → 2; three groups whose Delaunay
///   graph still connects everything (count 2 == groups − 1).
/// * net NV — THE CONTRAST (count == groups − 1): an F.Cu trace
///   (200000,20000)-(210000,20000) via-contacted at BOTH ends (zero
///   ratsnest corners) whose end via also bridges to a B.Cu trace —
///   two contacts on different layer spans → NOT a tail, so its
///   drill-center corner survives — + free pin P7 (180000,45000).
///   groups = 2, the via–P7 corner edge merges them → count 1 ==
///   groups − 1. (The end-only via and the free-ended B.Cu trace are
///   tails → filtered. Java parse normalization MERGES same-net
///   endpoint-sharing same-layer traces and can absorb vias at
///   shared corners — the oracle-proven reason earlier closed-triangle
///   drafts collapsed.) The count<groups−1 witness is NOT crafted —
///   it is pinned on the real 655_testboard fixture (nets 3/4/17 vs
///   net 7) by the `epic-harness` witness test.
/// * net NZ — a zero-contact trace plus a via touching it mid-span
///   (both are tails → filtered → groups FORCED to 0, the early-exit
///   pin; raw items = 2 proves the raw-vs-filtered split).
/// * net NX / NY — two same-y traces overlapping on
///   x∈[190000,200000], F.Cu → exactly ONE deduped clearance
///   violation row.
/// * net NP — P10 (250000,70000): pad straddles the outline edge
///   x=250000 (corners on BOTH sides → the outline exemption must NOT
///   clear → violation); P11 (240000,70000): pad fully inside → the
///   exemption clears → no violation.
/// * net NP2 — P13 (world (248999,20000), pad x∈[247999,249999]):
///   all corners INSIDE the outline yet overlapping the 100-unit
///   outline-precalc border band (x ≥ 249900) → the outline
///   exemption ARM actually fires and clears (an exemption-disabled
///   mutant produces a row; interior P11 never queries the outline,
///   so its no-row is band geometry, not the exemption).
/// * net NA / NB — parallel F.Cu traces
///   (230000,110000)-(240000,110000) and
///   (230000,110600)-(240000,110600): center distance 600, half
///   width 125, bbox gap 350. Under the default matrix they
///   violate (600 < 2000); the asymmetric-matrix pin re-classes
///   them 1/2 and swaps the cells to pin the gate's argument order.
/// * structure keepout (40000..50000 × 40000..50000) on F.Cu — the
///   closed-flag ObstacleArea matrix cells.
///
/// The craft boards live as committed corpus fixtures
/// (`harness/corpus/craft/`) and are replayed by `epic-harness drc`
/// too — ONE source of truth, so every unit pin below is
/// capture-backed by the matching golden record (drc-0015..0017).
pub(crate) const DSN_MAIN: &str = include_str!("../../../harness/corpus/craft/drc-main.dsn");

/// The tie-pin EXEMPT craft: trace TW1
/// (10000,80000)-(20000,80000) net TA and the VERTICAL trace TW2
/// (20000,80000)-(20000,90000) net TB overlap dim 2 in the 125 x 125
/// corner square WITHOUT sharing a net, and BOTH have a corner
/// exactly at pin P5 (20000,80000) — the pads' nets are TA AND TB
/// (the SAME image pin declared in both nets). Java
/// `getNormalContacts` only accepts a candidate TRACE whose
/// first/last corner EQUALS the query point (`Trace.java:186-190`),
/// so the mid-span TW2 of an earlier draft never appeared in TW1's
/// corner contacts and the exemption correctly stood — the shared
/// corner is what makes the exemption fire both ways: 0 violations.
/// Also the multi-net parse pin: P5 must appear in BOTH nets' raw
/// item lists.
pub(crate) const DSN_TIE: &str = include_str!("../../../harness/corpus/craft/drc-tie.dsn");

/// The tie-pin CONTRAST craft: TW2 shifted to
/// (19900,80200)-(30000,80200). The pair still overlaps dim 2
/// (y spans [80075,80325] vs [79875,80125]) and still shares no net,
/// but NO corner of TW1 touches TW2 and no corner of TW2's first half
/// touches TW1 (dy = 200 > half width 125) — the exemption must NOT
/// fire → exactly one violation row for the pair.
pub(crate) const DSN_TIE_CONTRAST: &str =
    include_str!("../../../harness/corpus/craft/drc-tie-contrast.dsn");

/// The #925b Pin-Pin-exemption craft (upstream 14b28b6ff, P4): five
/// components on the y=10000 row, every close pair 3000 µm apart
/// (PAD_R ±1000 → raw gap 1000 < the 2000 rule):
///
/// * CMPA (image IPA) — pins PS1/PS2, BOTH on net NS: the
///   same-component SAME-NET pair → exemption 2 clears it.
/// * CMPB + CMPB2 (BOTH placements of image IPB, 3000 apart) — the
///   single pin PB on each, both on net NS2: same net, DIFFERENT
///   components → still an obstacle (the exemption is
///   component-scoped).
/// * CMPC (image IPC) — NETLESS pins PX@1/PX@2 (referenced by no
///   net): same base name `PX` → exemption 1 (netless sub-pads of
///   one logical pad) clears it.
/// * CMPD (image IPD) — NETLESS pins PY@1/PZ@1: same component,
///   netless, but base names differ (`PY` vs `PZ`) → still an
///   obstacle (the base-name check is what fires, not mere
///   co-component netlessness).
/// * CMPE (image IPE) — PW@1 on net NS3, PW@2 NETLESS: same
///   component, same base, but exemption 1 requires BOTH netless →
///   still an obstacle.
///
/// Expected walk: exactly THREE rows — the NS2 pair, the CMPD pair,
/// the CMPE pair; no others (all cross-component distances ≥ 25000).
/// Pin names use the `@` sub-pad form (the parser accepts `@`/`#`
/// and mid-word `_`/`-` as plain word characters; the net pin-ref
/// split takes the component at the FIRST `-`).
pub(crate) const DSN_P4: &str = include_str!("../../../harness/fixtures/p4/p4-pins.dsn");

/// Parse a crafted DSN through the SAME reader + board build + tree
/// fill the corpus harness (`evaluate_rust`) parity-verifies.
pub(crate) fn parse(text: &str) -> (SearchTreeManager, Board) {
    let mut ses = SesBoard::new();
    let DsnReadResult::Success { warnings: _ } = read_board(text.as_bytes(), &mut ses) else {
        panic!("crafted DSN must parse");
    };
    let mut board = Board::from_ses_board(&ses);
    let mut manager = SearchTreeManager::new();
    manager.reinsert_tree_items(&mut board);
    (manager, board)
}

/// The 1-based number of the (first) net with this name.
pub(crate) fn net_no(board: &Board, name: &str) -> i32 {
    (1..=board.rules().nets.max_net_number())
        .find(|&n| {
            board
                .rules()
                .nets
                .get(n)
                .is_some_and(|net| net.name == name)
        })
        .unwrap_or_else(|| panic!("net {name} absent"))
}

/// The RAW per-net item list (calculateAllIncompletes semantics) of
/// the named net.
pub(crate) fn net_list(board: &Board, name: &str) -> Vec<ItemId> {
    crate::incompletes::raw_net_item_lists(board)[(net_no(board, name) - 1) as usize].clone()
}

/// Every item of one kind, ascending id.
pub(crate) fn ids_of_kind(board: &Board, kind: BoardItemType) -> Vec<ItemId> {
    board
        .iter_ascending()
        .filter(|entry| entry.board_item_type() == kind)
        .map(|entry| entry.id)
        .collect()
}

/// The item's FIRST ratsnest corner (for pins/vias the drill center)
/// as plain f64s.
pub(crate) fn corner(manager: &SearchTreeManager, board: &mut Board, id: ItemId) -> (f64, f64) {
    let float = epic_board::contacts::ratsnest_corners(manager, board, id)[0].to_float();
    (float.x, float.y)
}

/// The member of `ids` whose ratsnest corner sits at `(x, y)`
/// (1-unit tolerance — the crafts use integer coordinates).
pub(crate) fn id_at_corner(
    manager: &SearchTreeManager,
    board: &mut Board,
    ids: &[ItemId],
    x: f64,
    y: f64,
) -> ItemId {
    ids.iter()
        .copied()
        .find(|&id| {
            let (cx, cy) = corner(manager, board, id);
            (cx - x).abs() < 1.0 && (cy - y).abs() < 1.0
        })
        .expect("no member at the given corner")
}

/// Whether a ConductionArea parses with `isObstacle == false` (open
/// copper). A reader-fact pin helper: wiring rects are open.
pub(crate) fn ca_is_open(board: &Board, id: ItemId) -> bool {
    match &board.get(id).expect("item exists").data {
        ItemData::ConductionArea { is_obstacle, .. } => !*is_obstacle,
        _ => panic!("not a conduction area"),
    }
}

/// The net numbers an item carries.
pub(crate) fn nets_of(board: &Board, id: ItemId) -> Vec<i32> {
    board.get(id).expect("item exists").nets.clone()
}
