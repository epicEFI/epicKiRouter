//! The trace CONTACTS seam — the compute-on-demand port of Java
//! `Trace.getNormalContacts(Point, boolean)` and its start/end/union
//! wrappers (`Trace.java:104-203`), built on the Task 8 query surface
//! ([`SearchTreeManager::overlapping_objects`]). Consumers (the seam
//! exists for THESE, no more): Task 11 `PolylineTrace.combineAtStart`
//! (`:201-254` — `getNormalContacts(corner, false)`, drop the
//! conduction areas, require exactly one contact left) and Task 15
//! `SesWriter.snappedEndpoint` (`:426-453` — drill-item contacts only).
//! The connected-set machinery is M3 (design D26) and deliberately
//! absent.
//!
//! # The spiked definition
//!
//! Every anchor below was confirmed against the frozen jar by
//! `rust/harness/oracle/ContactsSpike.java` (fixture
//! `Issue163-pic_programmer.dsn` + the crafted trap DSNs embedded in
//! the spike; capture `/tmp/epic-t10-contacts.out`, run twice and
//! diffed — deterministic). Where the jar contradicted the recon, the
//! jar wins and the correction is recorded here.
//!
//! **1. The corner guard** (`Trace.java:174`): a `point` that is
//! neither the first nor the last corner returns the EMPTY SET — a
//! mid-corner query returns nothing even though items sit there.
//! Capture: `P_GUARD A3=10 point=35000,55000 point_form=[]` with
//! `P_MID_CANDIDATES point=35000,55000 layer=0 [11:T,10:T]` (two
//! candidates, zero contacts), and `M_GUARD X=1376
//! point=1447800,-977900 point_form=[]`.
//!
//! **2. The search shape is a degenerate IntBox** —
//! `TileShape.getInstance(point)` (`TileShape.java:58-62`) returns
//! `point.surroundingBox()`. SPIKE CORRECTION of the javadoc (which
//! claims "smallest IntOctagon"): capture
//! `X_TILESHAPE class=IntBox` / `X_BOX ll=40000,40000 ur=40000,40000`.
//! A bounding octagon of a point is the same reach, so the choice is
//! UNOBSERVABLE through contacts — membership and order are what is
//! pinned, never the shape kind.
//!
//! **3. The candidate query** (`BasicBoard.java:917-921`): the 2-arg
//! `overlappingObjects(shape, layer)` = the DEFAULT tree with
//! `ignore_nets = []`. Ported as
//! `overlapping_objects(board, DEFAULT_TREE_INDEX, &point_box, layer,
//! &[])`.
//!
//! **4. The per-candidate filter** (`Trace.java:184-186`):
//! `currentItem != this` (self-exclusion — capture
//! `P_FOREIGN_CANDIDATES point=20000,40000 layer=0 [4:T,3:P]` shows
//! the trace ITSELF as a candidate while its own contact set stays
//! empty), `sharesLayer` (`Item.java:313-318` — interval overlap
//! `max(firstLayer) <= min(lastLayer)`), and `ignoreNet ||
//! sharesNet` (`Item.java:174-189`).
//!
//! **5. The per-KIND acceptance** (`Trace.java:187-199`): the tree's
//! exact intersect test is only CANDIDACY; acceptance replaces
//! geometry entirely:
//!
//! | kind | accepts iff |
//! |---|---|
//! | Trace | point equals its FIRST or LAST corner |
//! | DrillItem (Pin/Via) | point equals `getCenter()` |
//! | ConductionArea | `getArea().contains(point)` (border counts) |
//! | everything else | NEVER — keepout/outlines stay candidates only |
//!
//! Capture: `P_KEEP_G_TRUE G=9 point_form_true=[]` with
//! `P_KEEP_CANDIDATES ... [9:T,2:K]`; `P_F 7 ... start=[6:A]`;
//! `M_X 1376 1 ... start=[] end=[1384:T]` (a mid-corner endpoint of
//! X itself still lists only real endpoint contacts).
//!
//! **6. Result order**: Java builds a `TreeSet<Item>` — DESCENDING
//! id (T60). Capture: `P_ORDER_C3 12 ... start=[13:T,5:V]` with
//! candidates `P_ORDER_CANDIDATES ... [13:T,12:T,5:V]`.
//!
//! # Compute-on-demand (the M2 plan's "STORAGE" branch — closed)
//!
//! `Trace.java` has NO contact cache (its only fields are
//! `halfWidth`/`layer`); every call re-queries the default tree.
//! Capture witness: `P_REQUERY_BEFORE C3=12 end=[]` → insert H →
//! `P_REQUERY_AFTER C3=12 end=[14:T] (a cached port would still show
//! [])`. The port matches exactly: free functions over the live
//! board, no storage, no `clear_contacts` hooks. (A re-query pin is
//! therefore LEGITIMATE here — the no-memoized-re-query rule targets
//! cached oracles, and this oracle has no cache.)
//!
//! # Net 0 is a shareable net
//!
//! `Item.sharesNetNo` is a PLAIN array intersection — net 0
//! intersects net 0 (no `<= 0` guard, unlike `containsNet`). Capture:
//! `P_NET0 ta=15 nets=[0] start=[] end=[16:T]`,
//! `P_SHARES ta.sharesNetNo([0])=true ta.sharesNetNo([])=false
//! ta.sharesNetNo([1])=false ta.isObstacle(0)=true` — the T8
//! contrast row: net 0 CONTACTS like a real net but never IGNORES.
//!
//! # Compensation invariance
//!
//! Membership cannot move with tree compensation: the query shape is
//! a point and the acceptance tests are exact corner/center
//! equality. Capture: every `S` row re-captured as `SC` and every
//! `P` row as `PC1` after `setClearanceCompensationUsed(true)` is
//! identical (`SC 1575 ... start=[1573:T,693:P] end=[1574:T]`;
//! `PC1_ORDER_C3 12 ... start=[13:T,5:V] end=[14:T]`).
//!
//! # The parse-time normalize divergence (classified, shaped around)
//!
//! Java's read path ends with `board.normalizeAllTraces()`
//! (`Wiring.java:345-353`), which MUTATES the parsed board — capture
//! section C: `C_NORMALIZE_SPLIT A=4 -> [16 17] ... C=6 removed by
//! removeIfCycle, D=7 re-inserted as id 14`. The Rust reader is
//! PRE-normalize (M2 Task 13). Classification: MISSING FEATURE (not
//! numeric drift, not tie-break) — the already-ledgered dsn-0151
//! divergence class (`harness/src/dsn_corpus.rs`,
//! `allowed_fields: stats, geometry_sha256`). No weakening, no
//! partial normalize port: the pins below run (a) on pic_programmer,
//! where normalize is provably a NO-OP (the fixture passes the 1,332
//! digest compare byte-identical; the pinned rows assert the exact
//! ids and corners a board mismatch would break), and (b) on a
//! crafted PURE board whose parse state normalize leaves untouched,
//! with the trap constructs added POST-parse via insertions that
//! mirror Java `insertTraceWithoutCleaning` (`BasicBoard.java:169` —
//! inserts WITHOUT normalizing) — same DSN, same generator state,
//! same insertion order, exact id parity (10..16).

use std::cmp::Reverse;
use std::collections::BTreeSet;

use epic_geometry::point::Point;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::tile_shape::TileShape;

use crate::board::Board;
use crate::id::ItemId;
use crate::items::ItemData;
use crate::items::trace::{first_corner, last_corner};
use crate::tree_manager::SearchTreeManager;

/// Java `Item.sharesNetNo(int[])` (`Item.java:174-189`) — a PLAIN
/// array intersection. Unlike `containsNet` there is no `<= 0` guard:
/// net 0 intersects net 0 (capture `P_SHARES
/// ta.sharesNetNo([0])=true`), so two net-0 traces are each other's
/// contacts. Symmetric.
#[must_use]
pub fn shares_net_no(nets: &[i32], other: &[i32]) -> bool {
    nets.iter().any(|net| other.contains(net))
}

/// Java `currentItem.sharesNet(this)` — [`shares_net_no`] over the
/// two items' net arrays (`false` for a missing id, where Java would
/// NPE; unreachable through the seam).
#[must_use]
pub fn items_share_net(board: &Board, a: ItemId, b: ItemId) -> bool {
    match (board.get(a), board.get(b)) {
        (Some(entry_a), Some(entry_b)) => shares_net_no(&entry_a.nets, &entry_b.nets),
        _ => false,
    }
}

/// Java `Item.sharesLayer(Item)` (`Item.java:313-318`) — layer-interval
/// overlap: `max(firstLayer) <= min(lastLayer)`. `&mut Board` because
/// the drill span is the memoized padstack lookup.
#[must_use]
pub fn items_share_layer(board: &mut Board, a: ItemId, b: ItemId) -> bool {
    match (board.item_first_layer(a), board.item_first_layer(b)) {
        (Some(first_a), Some(first_b)) => {
            match (board.item_last_layer(a), board.item_last_layer(b)) {
                (Some(last_a), Some(last_b)) => first_a.max(first_b) <= last_a.min(last_b),
                _ => false,
            }
        }
        _ => false,
    }
}

/// Java `Trace.getNormalContacts(Point point, boolean ignoreNet)`
/// (`Trace.java:173-203`) — THE core. See the module docs for the
/// spiked anchor-by-anchor definition. Returns descending ids (T60);
/// empty for a missing id or a non-trace id (Java would throw there).
///
/// Compute-on-demand: re-queries the default tree on every call —
/// no cache, by design (see "Compute-on-demand" in the module docs).
pub fn normal_contacts(
    manager: &SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
    point: &Point,
    ignore_net: bool,
) -> Vec<ItemId> {
    // Java guards `point == null` first; a `&Point` cannot be null.
    // Copy the filter inputs up front — the tree query needs
    // `&mut Board` (shape precalc), so no entry borrow may survive it.
    let Some(entry) = board.get(trace_id) else {
        return Vec::new();
    };
    let ItemData::Trace { layer, lines, .. } = &entry.data else {
        return Vec::new();
    };
    let layer = *layer;
    let first = first_corner(lines);
    let last = last_corner(lines);

    // The corner guard: NEITHER endpoint → the empty set (a degenerate
    // trace has no corners, so the Option-None case is "not an
    // endpoint" — the same answer Java's null comparison gives).
    let is_endpoint = |corner: &Option<Point>| corner.as_ref() == Some(point);
    if !(is_endpoint(&first) || is_endpoint(&last)) {
        return Vec::new();
    }

    // The search shape: TileShape.getInstance(point) = the degenerate
    // IntBox (module docs, spike correction of the javadoc).
    let search_shape =
        TileShape::RegularTileShape(RegularTileShape::IntBox(point.surrounding_box()));

    // The 2-arg overlappingObjects: the DEFAULT tree, ignore_nets = [].
    let candidates = manager.overlapping_objects(
        board,
        SearchTreeManager::DEFAULT_TREE_INDEX,
        &search_shape,
        layer,
        &[],
    );

    // Per-candidate filter + per-kind acceptance. `candidates` is
    // already deduped descending; filtering preserves that order, so
    // the result matches Java's TreeSet iteration.
    let mut contacts = Vec::new();
    for id in candidates {
        if id == trace_id {
            continue;
        }
        if !items_share_layer(board, trace_id, id) {
            continue;
        }
        if !ignore_net && !items_share_net(board, trace_id, id) {
            continue;
        }
        if point_accepts_contact(board, id, point) {
            contacts.push(id);
        }
    }
    contacts
}

/// Java `Trace.getStartContacts()` (`Trace.java:108-110`) — the point
/// form at the first corner with `ignoreNet = false`.
pub fn start_contacts(
    manager: &SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
) -> Vec<ItemId> {
    let first = board.trace_polyline(trace_id).and_then(first_corner);
    match first {
        Some(point) => normal_contacts(manager, board, trace_id, &point, false),
        None => Vec::new(),
    }
}

/// Java `Trace.getEndContacts()` (`Trace.java:116-118`) — the point
/// form at the LAST corner with `ignoreNet = false`.
pub fn end_contacts(
    manager: &SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
) -> Vec<ItemId> {
    let last = board.trace_polyline(trace_id).and_then(last_corner);
    match last {
        Some(point) => normal_contacts(manager, board, trace_id, &point, false),
        None => Vec::new(),
    }
}

/// Java `Trace.getNormalContacts()` (`Trace.java:155-166`) — the UNION
/// of both endpoint contact sets, descending. Not needed by T11/T15
/// but free over the ported core; captured as the `*_ALL` rows
/// (`P_UNION C3=12 all=[14:T,13:T,5:V]`).
pub fn all_contacts(
    manager: &SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
) -> Vec<ItemId> {
    let corners = board
        .trace_polyline(trace_id)
        .map(|lines| (first_corner(lines), last_corner(lines)));
    let Some((first, last)) = corners else {
        return Vec::new();
    };
    let mut union: BTreeSet<Reverse<ItemId>> = BTreeSet::new();
    for corner in [first, last].into_iter().flatten() {
        for id in normal_contacts(manager, board, trace_id, &corner, false) {
            union.insert(Reverse(id));
        }
    }
    union.into_iter().map(|Reverse(id)| id).collect()
}

/// The per-KIND acceptance of `Trace.java:187-199` — the exact test
/// from the tree is only candidacy; each kind replaces geometry with
/// its own point test (table in the module docs). Everything not in
/// the table (keepouts, obstacles, outlines) accepts NOTHING.
fn point_accepts_contact(board: &Board, id: ItemId, point: &Point) -> bool {
    let Some(entry) = board.get(id) else {
        return false;
    };
    match &entry.data {
        ItemData::Trace { lines, .. } => {
            first_corner(lines).as_ref() == Some(point)
                || last_corner(lines).as_ref() == Some(point)
        }
        ItemData::Pin { .. } | ItemData::Via { .. } => {
            board.drill_center(id).as_ref() == Some(point)
        }
        ItemData::ConductionArea { area, .. } => area.contains_point(point),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::ItemEntry;
    use crate::items::{BoardItemType, FixedState};
    use crate::test_util::{parse_board_from_path, parse_board_from_text};
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::polyline::Polyline;

    /// The spiked fixture: Issue163-pic_programmer.dsn (297 traces,
    /// 270 pins, 7 vias — trace endpoints land on pads and vias).
    /// Java's parse-time normalizeAllTraces is a NO-OP here (the
    /// fixture passes the 1,332-fixture digest compare
    /// byte-identical), so the post-parse capture rows apply to the
    /// Rust pre-normalize board as-is.
    const PIC_PROGRAMMER: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../fixtures/Issue163-pic_programmer.dsn"
    );

    /// The PURE crafted board — byte-identical to `PURE_DSN` in
    /// `rust/harness/oracle/ContactsSpike.java`. Parse items:
    /// 1=outline, 2=keepout F.Cu 70000-90000x10000-20000, 3=pin P1
    /// at (20000,40000) net OTHER, 4=E (20000,40000)-(10000,40000)
    /// net MINE, 5=via PAD_C600 at (40000,40000) net MINE, 6=AREA
    /// rect 50000,10000-60000,20000 net MINE, 7=F
    /// (55000,15000)-(55000,25000), 8=F2
    /// (50000,15000)-(50000,25000), 9=G (80000,15000)-(80000,30000).
    /// The trap insertions (A3=10, B3=11, C3=12, D3=13, H=14,
    /// ta=15, tb=16) happen POST-parse, mirroring Java
    /// `insertTraceWithoutCleaning` for exact id parity.
    const PURE_DSN: &str = "\
(pcb t10-pure.dsn\n\
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
    (keepout (rect F.Cu 70000 10000 90000 20000))\n\
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
    (wire (path F.Cu 250  20000 40000  10000 40000) (net MINE))\n\
    (via PAD_C600 40000 40000 (net MINE))\n\
    (wire (rect F.Cu 50000 10000 60000 20000) (net MINE))\n\
    (wire (path F.Cu 250  55000 15000  55000 25000) (net MINE))\n\
    (wire (path F.Cu 250  50000 15000  50000 25000) (net MINE))\n\
    (wire (path F.Cu 250  80000 15000  80000 30000) (net MINE))\n\
  )\n\
)\n";

    /// The default-tree manager over the board's items.
    fn indexed(board: &mut Board) -> SearchTreeManager {
        let mut manager = SearchTreeManager::new();
        manager.insert_all_board_items(board);
        manager
    }

    fn id(raw: u32) -> ItemId {
        ItemId::new(raw)
    }

    fn raw(contacts: &[ItemId]) -> Vec<u32> {
        contacts.iter().map(|contact| contact.get()).collect()
    }

    /// The `(id, kind)` pair list — the capture's `[1573:T,693:P]`
    /// annotation, kinds from the live board.
    fn typed(board: &Board, contacts: &[ItemId]) -> Vec<(u32, BoardItemType)> {
        contacts
            .iter()
            .map(|&contact| {
                let kind = board
                    .get(contact)
                    .map_or(BoardItemType::Other, |entry| entry.board_item_type());
                (contact.get(), kind)
            })
            .collect()
    }

    fn ip(x: i32, y: i32) -> Point {
        Point::Int(IntPoint::new(x, y))
    }

    /// Mirrors the spike's `insert.apply(...)`: a trace inserted
    /// WITHOUT normalization, layer/half-width/class copied from the
    /// parsed trace E (Java `any.getLayer()/getHalfWidth()/
    /// clearanceClassIndex()`), so both sides insert identical items
    /// and the generator hands out the same ids in the same order.
    fn insert_trace(
        board: &mut Board,
        manager: &mut SearchTreeManager,
        corners: &[Point],
        nets: &[i32],
    ) -> ItemId {
        let template = board.get(id(4)).expect("trace E on the pure board").clone();
        let ItemData::Trace {
            layer, half_width, ..
        } = template.data
        else {
            panic!("E is a trace");
        };
        let new_id = board.alloc_id();
        board.insert_item(ItemEntry {
            id: new_id,
            data: ItemData::Trace {
                layer,
                half_width,
                lines: Polyline::from_points(corners),
            },
            nets: nets.to_vec(),
            clearance_class: template.clearance_class,
            component_id: 0,
            fixed: FixedState::Unfixed,
            on_the_board: false,
        });
        manager.insert(board, new_id);
        new_id
    }

    fn first(board: &Board, trace_id: ItemId) -> Point {
        board
            .trace_polyline(trace_id)
            .and_then(first_corner)
            .expect("trace corners")
    }

    fn last(board: &Board, trace_id: ItemId) -> Point {
        board
            .trace_polyline(trace_id)
            .and_then(last_corner)
            .expect("trace corners")
    }

    // ------------------------------------------------------------------
    // pic_programmer — the S/M rows of the capture
    // ------------------------------------------------------------------

    /// The first 6 traces by descending id: exact corners, start/end
    /// contact sets as (id, kind), and the union form.
    ///
    /// Capture (`/tmp/epic-t10-contacts.out`, lines 3-14):
    /// `S 1575 1 first=1244600,-571500 last=1244600,-609600
    /// start=[1573:T,693:P] end=[1574:T]` … `S 1570 1
    /// first=1079500,-1153160 last=1257300,-1155700
    /// start=[1567:T,918:P] end=[1571:T]`, with `S_ALL` union rows.
    #[test]
    fn pic_start_end_and_union_contacts_match_the_capture() {
        let mut board = parse_board_from_path(PIC_PROGRAMMER);
        let manager = indexed(&mut board);

        let trace_count = board
            .iter_descending()
            .filter(|entry| matches!(entry.data, ItemData::Trace { .. }))
            .count();
        assert_eq!(trace_count, 297, "capture: trace count=297");

        // (id, first, last, start, end, union) straight from S/S_ALL.
        type PicRow = (u32, Point, Point, Vec<u32>, Vec<u32>, Vec<u32>);
        let rows: &[PicRow] = &[
            (
                1575,
                ip(1_244_600, -571_500),
                ip(1_244_600, -609_600),
                vec![1573, 693],
                vec![1574],
                vec![1574, 1573, 693],
            ),
            (
                1574,
                ip(1_244_600, -609_600),
                ip(1_257_300, -622_300),
                vec![1575],
                vec![65],
                vec![1575, 65],
            ),
            (
                1573,
                ip(1_181_100, -571_500),
                ip(1_244_600, -571_500),
                vec![1179],
                vec![1575, 693],
                vec![1575, 1179, 693],
            ),
            (
                1572,
                ip(1_282_700, -1_117_600),
                ip(1_282_700, -1_130_300),
                vec![1071],
                vec![1571],
                vec![1571, 1071],
            ),
            (
                1571,
                ip(1_282_700, -1_130_300),
                ip(1_257_300, -1_155_700),
                vec![1572],
                vec![1570],
                vec![1572, 1570],
            ),
            (
                1570,
                ip(1_079_500, -1_153_160),
                ip(1_257_300, -1_155_700),
                vec![1567, 918],
                vec![1571],
                vec![1571, 1567, 918],
            ),
        ];
        for &(trace_id, ref first_p, ref last_p, ref start, ref end, ref union) in rows {
            let trace_id = id(trace_id);
            assert_eq!(
                first(&board, trace_id),
                *first_p,
                "first corner of {trace_id:?}"
            );
            assert_eq!(
                last(&board, trace_id),
                *last_p,
                "last corner of {trace_id:?}"
            );
            assert_eq!(raw(&start_contacts(&manager, &mut board, trace_id)), *start);
            assert_eq!(raw(&end_contacts(&manager, &mut board, trace_id)), *end);
            assert_eq!(raw(&all_contacts(&manager, &mut board, trace_id)), *union);
            // Self-exclusion: never in its own sets.
            assert!(!start.contains(&trace_id.get()));
            assert!(!end.contains(&trace_id.get()));
        }

        // The kind annotations of the S rows (T/P).
        let contacts = start_contacts(&manager, &mut board, id(1575));
        assert_eq!(
            typed(&board, &contacts),
            vec![(1573, BoardItemType::Trace), (693, BoardItemType::Pin)]
        );
        let contacts = end_contacts(&manager, &mut board, id(1574));
        assert_eq!(typed(&board, &contacts), vec![(65, BoardItemType::Pin)]);
        let contacts = start_contacts(&manager, &mut board, id(1570));
        assert_eq!(
            typed(&board, &contacts),
            vec![(1567, BoardItemType::Trace), (918, BoardItemType::Pin)]
        );
    }

    /// The mid-corner pair: Y=1372's endpoint sits on a MID corner of
    /// X=1376 (X is also on the OTHER layer, so exclusion is
    /// over-determined there — the same-layer-only discriminator is
    /// the pure-board A3/B3 test below). The point form at X's own
    /// mid corner is the corner guard.
    ///
    /// Capture (lines 16-22):
    /// `M_PAIR Y=1372 X=1376 corner=1447800,-977900`,
    /// `M_X_CORNERS 1376 [1474470,-977900 1447800,-977900
    /// 1447800,-979170]`, `M_Y 1372 0 ... start=[1373:T] end=[840:P]`,
    /// `M_X 1376 1 ... start=[] end=[1384:T]`,
    /// `M_GUARD X=1376 point=1447800,-977900 point_form=[]`.
    #[test]
    fn pic_mid_corner_pair_and_guard() {
        let mut board = parse_board_from_path(PIC_PROGRAMMER);
        let manager = indexed(&mut board);

        let (y, x) = (id(1372), id(1376));
        // The capture pins Y's FIRST/LAST only (`M_Y 1372 0
        // first=1419860,-876300 last=1447800,-977900`) — the wire
        // keeps its mid corner (1447800,-904240): `(type protect)`
        // traces are USER_FIXED and normalize's split leaves them
        // alone, which is why pic_programmer passes the digest
        // compare. Only X's full corner list is captured
        // (`M_X_CORNERS`).
        assert_eq!(first(&board, y), ip(1_419_860, -876_300));
        assert_eq!(last(&board, y), ip(1_447_800, -977_900));
        assert_eq!(
            board.trace_polyline(x).expect("X").corners(),
            vec![
                ip(1_474_470, -977_900),
                ip(1_447_800, -977_900),
                ip(1_447_800, -979_170)
            ]
        );
        assert_eq!(raw(&start_contacts(&manager, &mut board, y)), vec![1373]);
        // X (id 1376, the LONGER trace through Y's endpoint) is NOT in
        // Y's end set — only the pin 840 is.
        let contacts = end_contacts(&manager, &mut board, y);
        assert_eq!(raw(&contacts), vec![840]);
        assert_eq!(typed(&board, &contacts), vec![(840, BoardItemType::Pin)]);
        assert_eq!(raw(&start_contacts(&manager, &mut board, x)), vec![]);
        assert_eq!(raw(&end_contacts(&manager, &mut board, x)), vec![1384]);

        // The corner guard at X's own mid corner: empty despite the
        // items sitting there.
        let mid = ip(1_447_800, -977_900);
        assert_eq!(
            normal_contacts(&manager, &mut board, x, &mid, false),
            Vec::new()
        );

        // The cross-layer span behind X's exclusion (Item.java:313-318).
        assert!(!items_share_layer(&mut board, y, x));
        assert!(items_share_layer(&mut board, y, id(840)));
    }

    // ------------------------------------------------------------------
    // the PURE board — parse parity first
    // ------------------------------------------------------------------

    /// The parse state of the PURE board matches the capture's item
    /// dump exactly (ids, kinds, layers, nets, geometry) — the
    /// precondition every insertion-parity pin below stands on.
    ///
    /// Capture (lines 58-67): `P_ITEM id=9 kind=PolylineTrace
    /// layer=0 nets=[1] corners=[80000,15000 80000,30000]` … down to
    /// `P_ITEM id=1 kind=BoardOutline layer=0 nets=[]`,
    /// `P_PARSE_IDS E=4 F=7 F2=8 G=9 AREA=6`.
    #[test]
    fn pure_board_parse_state_matches_the_capture() {
        let mut board = parse_board_from_text(PURE_DSN);

        let e = id(4);
        assert_eq!(
            board.trace_polyline(e).expect("E").corners(),
            vec![ip(20_000, 40_000), ip(10_000, 40_000)]
        );
        assert_eq!(
            board.trace_polyline(id(7)).expect("F").corners(),
            vec![ip(55_000, 15_000), ip(55_000, 25_000)]
        );
        assert_eq!(
            board.trace_polyline(id(8)).expect("F2").corners(),
            vec![ip(50_000, 15_000), ip(50_000, 25_000)]
        );
        assert_eq!(
            board.trace_polyline(id(9)).expect("G").corners(),
            vec![ip(80_000, 15_000), ip(80_000, 30_000)]
        );
        for trace_id in [4u32, 7, 8, 9] {
            let entry = board.get(id(trace_id)).expect("trace entry");
            assert!(matches!(entry.data, ItemData::Trace { .. }));
            assert_eq!(entry.nets, vec![1]);
        }
        // The via: center (40000,40000), net 1, span 0..0.
        assert_eq!(board.drill_center(id(5)), Some(ip(40_000, 40_000)));
        assert_eq!(board.drill_first_layer(id(5)), Some(0));
        assert_eq!(board.drill_last_layer(id(5)), Some(0));
        assert_eq!(board.get(id(5)).expect("via").nets, vec![1]);
        // The pin: foreign net 2, same center as E's start.
        assert_eq!(board.drill_center(id(3)), Some(ip(20_000, 40_000)));
        assert_eq!(board.get(id(3)).expect("pin").nets, vec![2]);
        // The conduction area bbox.
        let area = board.conduction_area(id(6)).expect("AREA");
        assert_eq!(
            area.border.bounding_box(),
            epic_geometry::int_box::IntBox::new(
                epic_geometry::int_point::IntPoint::new(50_000, 10_000),
                epic_geometry::int_point::IntPoint::new(60_000, 20_000)
            )
        );
        // Kinds of the parse items (2 = keepout ObstacleArea, 1 = outline).
        assert_eq!(
            board.get(id(2)).expect("keepout").board_item_type(),
            BoardItemType::ObstacleArea
        );
        assert_eq!(
            board.get(id(1)).expect("outline").board_item_type(),
            BoardItemType::BoardOutline
        );
    }

    // ------------------------------------------------------------------
    // the trap pins on the PURE board
    // ------------------------------------------------------------------

    /// The foreign-net trap: E's START sits exactly on pin P1's
    /// center, but the pin is net OTHER — not a contact with
    /// `ignore_net=false`, a contact with `ignore_net=true`. The raw
    /// candidate set (queried directly) contains BOTH E itself and
    /// the pin, so the empty false-row is the net filter's doing and
    /// the self-id proves the `!= this` filter.
    ///
    /// Capture (lines 68-71):
    /// `P_E 4 0 first=20000,40000 last=10000,40000 start=[] end=[]`,
    /// `P_FOREIGN E=4 start_false=[] start_true=[3:P]`,
    /// `P_FOREIGN_CANDIDATES point=20000,40000 layer=0 [4:T,3:P]`.
    #[test]
    fn foreign_net_contact_requires_ignore_net() {
        let mut board = parse_board_from_text(PURE_DSN);
        let manager = indexed(&mut board);

        let e = id(4);
        assert_eq!(start_contacts(&manager, &mut board, e), Vec::new());
        assert_eq!(end_contacts(&manager, &mut board, e), Vec::new());
        let corner = first(&board, e);
        assert_eq!(
            normal_contacts(&manager, &mut board, e, &corner, false),
            Vec::new()
        );
        let contacts = normal_contacts(&manager, &mut board, e, &corner, true);
        assert_eq!(typed(&board, &contacts), vec![(3, BoardItemType::Pin)]);

        // The raw candidate set at the same point: E itself IS a
        // candidate (the self-exclusion witness).
        let search = TileShape::RegularTileShape(RegularTileShape::IntBox(
            first(&board, e).surrounding_box(),
        ));
        assert_eq!(
            raw(&manager.overlapping_objects(
                &mut board,
                SearchTreeManager::DEFAULT_TREE_INDEX,
                &search,
                0,
                &[]
            )),
            vec![4, 3]
        );
    }

    /// The kind filter — conduction areas: F's start is strictly
    /// INSIDE the same-net area (a contact), F2's start sits ON the
    /// area's border (still a contact — border points count).
    ///
    /// Capture (lines 72-76):
    /// `P_F 7 0 ... start=[6:A] end=[]`,
    /// `P_F2 8 0 ... start=[6:A] end=[]`,
    /// `P_AREA_CONTAINS area=6 F=true F2=true`.
    #[test]
    fn conduction_area_contacts_include_border_points() {
        let mut board = parse_board_from_text(PURE_DSN);
        let manager = indexed(&mut board);

        let contacts = start_contacts(&manager, &mut board, id(7));
        assert_eq!(
            typed(&board, &contacts),
            vec![(6, BoardItemType::ConductionArea)]
        );
        assert_eq!(end_contacts(&manager, &mut board, id(7)), Vec::new());
        let contacts = start_contacts(&manager, &mut board, id(8));
        assert_eq!(
            typed(&board, &contacts),
            vec![(6, BoardItemType::ConductionArea)]
        );
        // The area's contains() itself: strict interior AND border.
        let area = board.conduction_area(id(6)).expect("AREA");
        assert!(
            area.contains_point(&ip(55_000, 15_000)),
            "F's start, strict interior"
        );
        assert!(
            area.contains_point(&ip(50_000, 15_000)),
            "F2's start, on the border"
        );
        assert!(
            !area.contains_point(&ip(10_000, 10_000)),
            "contrast: far outside"
        );
    }

    /// The kind filter — keepout: G's start lies in the keepout's
    /// corner region; the keepout is a tree CANDIDATE at that point
    /// but NEVER a contact, under either net flag.
    ///
    /// Capture (lines 77-80):
    /// `P_G 9 0 ... start=[] end=[]`,
    /// `P_KEEP_G_TRUE G=9 point_form_true=[]`,
    /// `P_KEEP_CANDIDATES point=80000,15000 layer=0 [9:T,2:K]`.
    #[test]
    fn keepout_is_candidate_but_never_contact() {
        let mut board = parse_board_from_text(PURE_DSN);
        let manager = indexed(&mut board);

        let g = id(9);
        assert_eq!(start_contacts(&manager, &mut board, g), Vec::new());
        assert_eq!(end_contacts(&manager, &mut board, g), Vec::new());
        let corner = first(&board, g);
        assert_eq!(
            normal_contacts(&manager, &mut board, g, &corner, true),
            Vec::new()
        );

        let search = TileShape::RegularTileShape(RegularTileShape::IntBox(
            first(&board, g).surrounding_box(),
        ));
        let candidates = manager.overlapping_objects(
            &mut board,
            SearchTreeManager::DEFAULT_TREE_INDEX,
            &search,
            0,
            &[],
        );
        assert_eq!(
            typed(&board, &candidates),
            vec![(9, BoardItemType::Trace), (2, BoardItemType::ObstacleArea)]
        );
    }

    /// The mid-corner + corner-guard traps, same layer AND same net:
    /// A3 is bent with mid corner (35000,55000); B3 STARTS exactly
    /// there. Both share layer 0 and net 1, so ONLY the per-kind
    /// corner check can exclude them from each other — the
    /// discriminating form of the trap (a port skipping the corner
    /// check returns A3 in B3's start set).
    ///
    /// Capture (lines 81-87):
    /// `P_INSERTED A3=10 B3=11`,
    /// `P_MID_B3 11 0 first=35000,55000 last=35000,60000 start=[] end=[]`,
    /// `P_MID_A3 10 0 first=30000,50000 last=40000,50000 start=[] end=[]`,
    /// `P_GUARD A3=10 point=35000,55000 point_form=[]`,
    /// `P_MID_CANDIDATES point=35000,55000 layer=0 [11:T,10:T]`.
    #[test]
    fn mid_corner_pair_is_excluded_by_the_kind_check() {
        let mut board = parse_board_from_text(PURE_DSN);
        let mut manager = indexed(&mut board);

        let a3 = insert_trace(
            &mut board,
            &mut manager,
            &[ip(30_000, 50_000), ip(35_000, 55_000), ip(40_000, 50_000)],
            &[1],
        );
        let b3 = insert_trace(
            &mut board,
            &mut manager,
            &[ip(35_000, 55_000), ip(35_000, 60_000)],
            &[1],
        );
        assert_eq!(a3.get(), 10, "P_INSERTED A3=10 (generator parity)");
        assert_eq!(b3.get(), 11, "P_INSERTED B3=11");

        assert_eq!(start_contacts(&manager, &mut board, b3), Vec::new());
        assert_eq!(end_contacts(&manager, &mut board, b3), Vec::new());
        assert_eq!(start_contacts(&manager, &mut board, a3), Vec::new());
        assert_eq!(end_contacts(&manager, &mut board, a3), Vec::new());

        // The corner guard at A3's own mid corner.
        let mid = ip(35_000, 55_000);
        assert_eq!(
            normal_contacts(&manager, &mut board, a3, &mid, false),
            Vec::new()
        );

        // Both ARE candidates at that point (same layer, same net,
        // shapes through the point) — only the kind check excludes.
        assert!(items_share_layer(&mut board, a3, b3));
        assert!(items_share_net(&board, a3, b3));
        let search = TileShape::RegularTileShape(RegularTileShape::IntBox(mid.surrounding_box()));
        assert_eq!(
            raw(&manager.overlapping_objects(
                &mut board,
                SearchTreeManager::DEFAULT_TREE_INDEX,
                &search,
                0,
                &[]
            )),
            vec![11, 10]
        );
    }

    /// The ORDER trap: C3 and D3 both touch the parse via's center
    /// (40000,40000) — C3's start lists [13:T,5:V] DESCENDING, D3's
    /// end lists [12:T,5:V], and the raw candidate set carries all
    /// three. A port emitting insertion or ascending order fails.
    ///
    /// Capture (lines 88-93):
    /// `P_INSERTED C3=12 D3=13`,
    /// `P_ORDER_C3 12 0 first=40000,40000 last=50000,40000 start=[13:T,5:V] end=[]`,
    /// `P_ORDER_D3 13 0 first=60000,40000 last=40000,40000 start=[] end=[12:T,5:V]`,
    /// `P_ORDER_CANDIDATES point=40000,40000 layer=0 [13:T,12:T,5:V]`.
    #[test]
    fn contacts_descend_by_id_with_via_and_trace_together() {
        let mut board = parse_board_from_text(PURE_DSN);
        let mut manager = indexed(&mut board);

        let (_, _, c3, d3) = insert_traps_through_d3(&mut board, &mut manager);
        assert_eq!(c3.get(), 12, "P_INSERTED C3=12");
        assert_eq!(d3.get(), 13, "P_INSERTED D3=13");

        let contacts = start_contacts(&manager, &mut board, c3);
        assert_eq!(
            typed(&board, &contacts),
            vec![(13, BoardItemType::Trace), (5, BoardItemType::Via)]
        );
        assert_eq!(end_contacts(&manager, &mut board, c3), Vec::new());
        assert_eq!(start_contacts(&manager, &mut board, d3), Vec::new());
        let contacts = end_contacts(&manager, &mut board, d3);
        assert_eq!(
            typed(&board, &contacts),
            vec![(12, BoardItemType::Trace), (5, BoardItemType::Via)]
        );

        let search = TileShape::RegularTileShape(RegularTileShape::IntBox(
            first(&board, c3).surrounding_box(),
        ));
        let candidates = manager.overlapping_objects(
            &mut board,
            SearchTreeManager::DEFAULT_TREE_INDEX,
            &search,
            0,
            &[],
        );
        assert_eq!(raw(&candidates), vec![13, 12, 5]);
    }

    /// Mirrors the spike's insertion sequence A3, B3, C3, D3 (the
    /// exact order `sectionP` inserts them) — every test that pins
    /// the ABSOLUTE ids 12..16 must replay the FULL sequence, because
    /// `alloc_id` is monotone and skipping A3/B3 shifts every later
    /// id (the burn semantics of T61, matching Java's generator).
    fn insert_traps_through_d3(
        board: &mut Board,
        manager: &mut SearchTreeManager,
    ) -> (ItemId, ItemId, ItemId, ItemId) {
        let a3 = insert_trace(
            board,
            manager,
            &[ip(30_000, 50_000), ip(35_000, 55_000), ip(40_000, 50_000)],
            &[1],
        );
        let b3 = insert_trace(
            board,
            manager,
            &[ip(35_000, 55_000), ip(35_000, 60_000)],
            &[1],
        );
        let c3 = insert_trace(
            board,
            manager,
            &[ip(40_000, 40_000), ip(50_000, 40_000)],
            &[1],
        );
        let d3 = insert_trace(
            board,
            manager,
            &[ip(60_000, 40_000), ip(40_000, 40_000)],
            &[1],
        );
        (a3, b3, c3, d3)
    }

    /// The COMPUTE-ON-DEMAND witness: C3's end is empty, H is
    /// inserted at C3's free end, and the SAME query now returns H.
    /// A re-query pin is legitimate here (unlike the memoized-oracle
    /// trap) precisely because there is no cache — that absence is
    /// what this pin certifies. Also the union form on a trace whose
    /// start and end sets differ.
    ///
    /// Capture (lines 94-98):
    /// `P_REQUERY_BEFORE C3=12 end=[]`,
    /// `P_INSERTED H=14`,
    /// `P_REQUERY_AFTER C3=12 end=[14:T] (a cached port would still show [])`,
    /// `P_H H=14 start=[12:T] end=[]`,
    /// `P_UNION C3=12 all=[14:T,13:T,5:V]`.
    #[test]
    fn contacts_recompute_after_insertion_no_cache() {
        let mut board = parse_board_from_text(PURE_DSN);
        let mut manager = indexed(&mut board);

        // The full sequence through D3 so the ids match the capture.
        let (_, _, c3, _) = insert_traps_through_d3(&mut board, &mut manager);
        assert_eq!(end_contacts(&manager, &mut board, c3), Vec::new());

        let h = insert_trace(
            &mut board,
            &mut manager,
            &[ip(50_000, 40_000), ip(45_000, 50_000)],
            &[1],
        );
        assert_eq!(h.get(), 14, "P_INSERTED H=14");
        let contacts = end_contacts(&manager, &mut board, c3);
        assert_eq!(typed(&board, &contacts), vec![(14, BoardItemType::Trace)]);
        let contacts = start_contacts(&manager, &mut board, h);
        assert_eq!(typed(&board, &contacts), vec![(12, BoardItemType::Trace)]);
        assert_eq!(end_contacts(&manager, &mut board, h), Vec::new());
        let contacts = all_contacts(&manager, &mut board, c3);
        assert_eq!(
            typed(&board, &contacts),
            vec![
                (14, BoardItemType::Trace),
                (13, BoardItemType::Trace),
                (5, BoardItemType::Via)
            ]
        );
    }

    /// The net-0 trap: `shares_net_no` is a plain intersection — 0
    /// intersects 0 — so two NET-ZERO traces contact each other, while
    /// `item_is_obstacle(0)` stays true (the T8 contrast row:
    /// contact-membership and ignore-obstacle use DIFFERENT rules).
    ///
    /// Capture (lines 99-101):
    /// `P_NET0 ta=15 nets=[0] start=[] end=[16:T]`,
    /// `P_NET0 tb=16 nets=[0] start=[15:T] end=[]`,
    /// `P_SHARES ta.sharesNetNo([0])=true ta.sharesNetNo([])=false
    /// ta.sharesNetNo([1])=false ta.isObstacle(0)=true`.
    #[test]
    fn net_zero_traces_contact_each_other() {
        let mut board = parse_board_from_text(PURE_DSN);
        let mut manager = indexed(&mut board);

        // The full sequence through H so ta/tb land on 15/16.
        insert_traps_through_d3(&mut board, &mut manager);
        insert_trace(
            &mut board,
            &mut manager,
            &[ip(50_000, 40_000), ip(45_000, 50_000)],
            &[1],
        );

        let ta = insert_trace(
            &mut board,
            &mut manager,
            &[ip(65_000, 40_000), ip(70_000, 40_000)],
            &[0],
        );
        let tb = insert_trace(
            &mut board,
            &mut manager,
            &[ip(70_000, 40_000), ip(75_000, 40_000)],
            &[0],
        );
        assert_eq!(ta.get(), 15, "P_NET0 ta=15");
        assert_eq!(tb.get(), 16, "P_NET0 tb=16");

        assert_eq!(start_contacts(&manager, &mut board, ta), Vec::new());
        assert_eq!(raw(&end_contacts(&manager, &mut board, ta)), vec![16]);
        assert_eq!(raw(&start_contacts(&manager, &mut board, tb)), vec![15]);
        assert_eq!(end_contacts(&manager, &mut board, tb), Vec::new());

        assert!(shares_net_no(&[0], &[0]));
        assert!(!shares_net_no(&[0], &[]));
        assert!(!shares_net_no(&[0], &[1]));
        assert_eq!(board.get(ta).expect("ta").nets, vec![0]);
        // The T8 contrast: the same net-0 item IS an obstacle for net 0.
        assert!(board.item_is_obstacle(ta, 0));
    }

    /// The compensation trap: after `set_clearance_compensation_used(
    /// true)` (a full tree rebuild under the compensated default
    /// tree) every pinned membership row is unchanged — the query
    /// shape is a point and the acceptance tests are exact equality.
    ///
    /// Capture: the whole SC section duplicates the S section
    /// row-for-row (lines 23-35 vs 3-14), and on the pure board
    /// `PC1_ORDER_C3 12 ... start=[13:T,5:V] end=[14:T]`,
    /// `PC1_F 7 ... start=[6:A] end=[]`,
    /// `PC1_FOREIGN_TRUE E=4 start_true=[3:P]`
    /// (lines 105-109), `PC1_MID_B3 11 ... start=[] end=[]` (103-104).
    #[test]
    fn compensation_flip_leaves_membership_unchanged() {
        let mut board = parse_board_from_text(PURE_DSN);
        let mut manager = indexed(&mut board);

        // Full sequence (A3..H, ids 10..14) via the shared helper —
        // same shape as the re-query test, same absolute ids.
        let (a3, b3, c3, d3) = insert_traps_through_d3(&mut board, &mut manager);
        let h = insert_trace(
            &mut board,
            &mut manager,
            &[ip(50_000, 40_000), ip(45_000, 50_000)],
            &[1],
        );
        assert_eq!(
            [a3.get(), b3.get(), c3.get(), d3.get(), h.get()],
            [10, 11, 12, 13, 14]
        );

        manager.set_clearance_compensation_used(&mut board, true);

        assert_eq!(start_contacts(&manager, &mut board, b3), Vec::new());
        assert_eq!(end_contacts(&manager, &mut board, b3), Vec::new());
        let contacts = start_contacts(&manager, &mut board, c3);
        assert_eq!(
            typed(&board, &contacts),
            vec![(13, BoardItemType::Trace), (5, BoardItemType::Via)]
        );
        let contacts = end_contacts(&manager, &mut board, c3);
        assert_eq!(typed(&board, &contacts), vec![(14, BoardItemType::Trace)]);
        let contacts = start_contacts(&manager, &mut board, id(7));
        assert_eq!(
            typed(&board, &contacts),
            vec![(6, BoardItemType::ConductionArea)]
        );
        let corner = first(&board, id(4));
        let contacts = normal_contacts(&manager, &mut board, id(4), &corner, true);
        assert_eq!(typed(&board, &contacts), vec![(3, BoardItemType::Pin)]);
    }

    /// The layer-span seams: `item_first_layer`/`item_last_layer`
    /// (the `sharesLayer` inputs) per kind. The pin-span row is
    /// captured indirectly (pin 3 IS a contact of E's start under
    /// `ignore_net=true`, which requires `shares_layer(pin, trace)`),
    /// the cross-layer row by M_PAIR; the outline interval
    /// `0..layerCount-1` is the `firstLayer()`/`lastLayer()`
    /// override anchor (`BoardOutline.java:99-108`) — unobservable
    /// through contacts (outlines are never contacts), pinned at the
    /// accessor level.
    #[test]
    fn item_layer_spans_by_kind() {
        let mut board = parse_board_from_text(PURE_DSN);

        // Drill span (pin 3, via 5) and the flat kinds.
        for drill_id in [3u32, 5] {
            assert_eq!(board.item_first_layer(id(drill_id)), Some(0));
            assert_eq!(board.item_last_layer(id(drill_id)), Some(0));
        }
        for flat_id in [4u32, 7, 8, 9] {
            assert_eq!(board.item_first_layer(id(flat_id)), Some(0));
            assert_eq!(board.item_last_layer(id(flat_id)), Some(0));
        }
        // The keepout and the outline.
        assert_eq!(board.item_first_layer(id(2)), Some(0));
        assert_eq!(board.item_last_layer(id(2)), Some(0));
        assert_eq!(board.item_first_layer(id(1)), Some(0));
        assert_eq!(
            board.item_last_layer(id(1)),
            Some(1),
            "2 layers: F.Cu + B.Cu"
        );

        // The interval-overlap rule (Item.java:313-318).
        assert!(items_share_layer(&mut board, id(3), id(4)));
        assert!(
            items_share_layer(&mut board, id(1), id(4)),
            "outline spans all layers"
        );
        assert!(items_share_net(&board, id(4), id(7)));
        assert!(!items_share_net(&board, id(3), id(4)), "OTHER vs MINE");
    }
}
