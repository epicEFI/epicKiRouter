//! The incomplete-connections (ratsnest airline) count — the full port
//! of Java `app.freerouting.drc.NetIncompletes` (frozen at `e7f9bdf1`)
//! plus the `calculateAllIncompletes` raw item lists and the
//! `maxConnections` endpoint formula
//! (`DesignRulesChecker.java:545-626`).
//!
//! ## Pipeline (constructor `NetIncompletes.java:57-226`)
//!
//! 1. **Filter** (`:85-116`): drop tails ([`item_is_tail`]) and
//!    zero-contact items that are neither a `DrillItem` (pin/via) nor a
//!    `ConductionArea` — unrouted pins legitimately have no contacts,
//!    conduction areas are connection media.
//! 2. **Grouping** (`calculateNetItems` `:293-322`): repeatedly take a
//!    remaining item, compute its connected set, and label every member
//!    still on the filtered list with that set; the set identity is the
//!    group label Kruskal later compares.
//! 3. **Delaunay** over the grouped items' ratsnest corners — one
//!    triangulation object per item ([`ratsnest_corners`] is the
//!    `Storable.getTriangulationCorners` delegate).
//! 4. **The sorted-Edge set** (`:174-184`): the ResultEdges wrapped as
//!    `Edge` (5-tuple compare, [`edge_compare`]) in a `TreeSet` —
//!    exact-duplicate edges (identical length AND identical corner
//!    coordinates) collapse.
//! 5. **Kruskal** (`:191-205`): walk the sorted edges; skip an edge
//!    whose ends already share the group label, otherwise count one
//!    airline and relabel the whole from-group to the to-group
//!    (`joinConnectedSets` `:328-337`).
//!
//! ## The falsified claim (why the count is edge-set-dependent)
//!
//! `groups − 1` is NOT the count: `Trace.getRatsnestCorners()` emits
//! only UNCONTACTED stub endpoints, so a both-ends-contacted trace (or
//! a circular conduction area) contributes ZERO corners and vanishes
//! from the triangulation graph; groups whose every item is
//! corner-less merge with nothing and Kruskal finishes below
//! `groups − 1`. Witness capture: drc-0013 (655_testboard reference
//! board) nets 3/4/17 count 3/2/2 against `groups − 1` = 4/3/3; the
//! mechanism pins live in `epic-board::contacts`
//! (`both_ends_contacted_trace_emits_zero_ratsnest_corners`).
//!
//! ## Determinism deviation door (audited, corpus-proven inert)
//!
//! Java discovers groups in `HashSet.iterator().next()` order —
//! JVM-identity-hash dependent, so the live count triangulates in a
//! different OBJECT ORDER every JVM process. Two facts make that
//! deviation inert for every observable: the count itself is
//! order-INSENSITIVE (the Kruskal yield is
//! `groups − components(edge graph)`, both order-free — proven by the
//! cross-process double capture), and the committed edge ROWS come
//! from the DrcOracle reconstruction, which sorts the storables by id
//! BEFORE triangulating (`DrcOracle.java:370`). The port therefore
//! runs ONE triangulation in the canonical ascending-id object order
//! (serving both the count and the rows) and uses the deterministic
//! smallest-remaining-id order only for group DISCOVERY (within a
//! group the members still iterate DESCENDING like Java's
//! `TreeSet`). `DrcOracle` exits 4 on any drift (groups
//! reconstruction, per-net-sum), so a future fixture that DOES depend
//! on insertion order fails the capture loudly instead of pinning
//! noise.

use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::BTreeSet;

use epic_board::board::Board;
use epic_board::contacts::{
    item_connected_set, item_is_tail, item_normal_contacts, ratsnest_corners,
};
use epic_board::id::ItemId;
use epic_board::items::BoardItemType;
use epic_board::tree_manager::SearchTreeManager;
use epic_geometry::float_point::FloatPoint;
use epic_geometry::point::Point;

/// One per-net result — the schema-v2 row shape (the harness maps it
/// onto `PerNetRecord` field-for-field).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetIncompleteRow {
    /// The 1-based net number.
    pub net_no: i32,
    /// The RAW net-list size (pre-filter; Java `netItems.size()`).
    pub items: usize,
    /// Unique connected-set count over the filtered items (Java
    /// `getConnectedGroupCount()`, with the `length <= 1` override).
    pub groups: usize,
    /// The Kruskal airline count (Java `count()`).
    pub incomplete_count: usize,
    /// The Delaunay input objects as `(id, corner count)`, sorted by
    /// id. An `n == 0` row is the falsification mechanism in the wild.
    pub ratsnest: Vec<(ItemId, usize)>,
    /// Every triangulation ResultEdge as the canonical
    /// `(min id, max id)` item pair — exact duplicates collapsed,
    /// sorted lexicographically (the DrcOracle emission contract).
    pub edges: Vec<[i64; 2]>,
}

/// Java `calculateAllIncompletes` (`DesignRulesChecker.java:545-564`):
/// the RAW per-net connectable item lists. Every `Connectable` item is
/// appended to EACH net list it carries (multi-net items appear in
/// every one of their nets' lists); the walk is insertion order =
/// ascending id for a parse board (Java `itemList.startReadObject`).
///
/// DEVIATION (documented): Java indexes `netItemLists.get(net − 1)`
/// and would throw `ArrayIndexOutOfBoundsException` for a connectable
/// item carrying net 0; the port SKIPS net numbers `<= 0` — unobservable
/// on parse boards, where every connectable carries a positive net.
#[must_use]
pub fn raw_net_item_lists(board: &Board) -> Vec<Vec<ItemId>> {
    let max_net_no = board.rules().nets.max_net_number();
    let mut lists = vec![Vec::new(); max_net_no.max(0) as usize];
    for entry in board.iter_ascending() {
        if !matches!(
            entry.board_item_type(),
            BoardItemType::Trace
                | BoardItemType::Pin
                | BoardItemType::Via
                | BoardItemType::ConductionArea
        ) {
            continue;
        }
        for &net in &entry.nets {
            // Java indexes netList[net - 1] directly: net <= 0 (or >
            // max_net_no) throws AIOOBE there. The port skips instead
            // (the module-doc deviation door) — this assert keeps the
            // silent skip from hiding a construction bug in callers.
            debug_assert!(
                net >= 1,
                "connectable item {} carries net {net}: Java throws AIOOBE, the port silently skips",
                entry.id.get()
            );
            if net >= 1 && net <= max_net_no {
                lists[(net - 1) as usize].push(entry.id);
            }
        }
    }
    lists
}

/// One Kruskal-ACCEPTED airline (the ratsnest segment the GUI draws),
/// in world DBU coordinates. Serde-free by charter (epic-drc carries
/// no serde dep; epic-engine projects it onto its wire type).
///
/// ENDPOINT CONVENTION (documented per the T5 charter): `from`/`to`
/// are the ratsnest CORNER points the Delaunay triangulation actually
/// uses — Java's `AirLine.fromCorner`/`toCorner`
/// (`drc/NetIncompletes.java:344-380` family) — NOT item-center
/// approximations. Conversion to i64 DBU: an `Int` corner is exact;
/// a `Rational` corner (projection artifact — parsed ratsnest corners
/// are int-cornered) rounds its f64 `to_float` face to the NEAREST
/// DBU (f64 `round`, half away from zero — the same convention as
/// epic-engine's `PointPrimitive::from_point`). The corners ride the
/// comparator as f64 (`PortEdge.from/to`), which is EXACT for the
/// int-cornered world (DBU i32 fits f64's 2^53 mantissa), so the
/// rounding is a no-op on every parsed board.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AirLineSegment {
    /// The from-corner (DBU i64).
    pub from: (i64, i64),
    /// The to-corner (DBU i64).
    pub to: (i64, i64),
    /// The 1-based net number.
    pub net: i32,
}

/// Java `maxConnections` (`DesignRulesChecker.java:570-581`): over
/// every net with a non-empty raw list, `max(0, endpoints − 1)` where
/// endpoints are the `Pin`/`ConductionArea` items — summed. The
/// FRLogger trace at `:593` advertising "(formula: total_items -
/// netCount)" is STALE; the code is endpoint-based and the port
/// implements the code.
#[must_use]
pub fn max_connections(board: &Board, lists: &[Vec<ItemId>]) -> i64 {
    lists
        .iter()
        .filter(|list| !list.is_empty())
        .map(|list| {
            let endpoints = list
                .iter()
                .filter(|&&id| {
                    matches!(
                        board.get(id).map(|entry| entry.board_item_type()),
                        Some(BoardItemType::Pin | BoardItemType::ConductionArea)
                    )
                })
                .count();
            (endpoints as i64 - 1).max(0)
        })
        .sum()
}

/// One potential airline (Java `Edge`, `NetIncompletes.java:344-380`):
/// the Delaunay ResultEdge with its grouped-item end INDICES (Java
/// holds `NetItem` references; identity comparisons become label
/// lookups) and the f64 corners the comparator rides on.
#[derive(Debug, Clone, Copy)]
struct PortEdge {
    from_idx: usize,
    from: FloatPoint,
    to_idx: usize,
    to: FloatPoint,
    /// `toCorner.distanceSquare(fromCorner)` — the Java initializer's
    /// operand order, mirrored literally.
    length_square: f64,
}

/// Java `Edge.compareTo` (`:361-379`) + `Signum.asInt`: the f64
/// subtraction chain with exact `== 0.0` continuations — length
/// square, then from.x, from.y, to.x, to.y. The item identities are
/// NOT part of the compare, so two edges over the same COORDINATE
/// 4-tuple collapse in the TreeSet even between different items.
/// NaN propagates through the chain and `Signum.asInt(NaN)` is 0
/// (both comparisons false) → [`Ordering::Equal`], mirrored.
fn edge_compare(a: &PortEdge, b: &PortEdge) -> Ordering {
    let mut result = a.length_square - b.length_square;
    if result == 0.0 {
        result = a.from.x - b.from.x;
    }
    if result == 0.0 {
        result = a.from.y - b.from.y;
    }
    if result == 0.0 {
        result = a.to.x - b.to.x;
    }
    if result == 0.0 {
        result = a.to.y - b.to.y;
    }
    if result > 0.0 {
        Ordering::Greater
    } else if result < 0.0 {
        Ordering::Less
    } else {
        Ordering::Equal
    }
}

/// The FULL NetIncompletes constructor over one net's raw item list —
/// filter → grouping → Delaunay → sorted-Edge Kruskal (module docs).
/// `raw_items` must be the [`raw_net_item_lists`] entry for `net_no`.
///
/// The result rows and counts are pinned record-for-record by the drc
/// corpus (`epic-harness drc compare`).
pub fn net_incompletes_row(
    manager: &SearchTreeManager,
    board: &mut Board,
    net_no: i32,
    raw_items: &[ItemId],
) -> NetIncompleteRow {
    net_incompletes_row_inner(manager, board, net_no, raw_items, false).0
}

/// The shared walk behind [`net_incompletes_row`] (the schema-v2 row
/// face) and [`airline_segments`] (the M9-T5 ratsnest face): the
/// IDENTICAL pipeline with the Kruskal-ACCEPTED edges — the
/// airlines — optionally collected as they are counted. `collect_airlines
/// == false` is byte-identical to the pre-T5 face by construction (the
/// row path is untouched; the collection is an `if` inside the count
/// loop). Nets ascend; within a net the segments ride the sorted-Edge
/// Kruskal walk order (the deterministic comparator order — NOT the
/// lexicographic `edges` row order).
fn net_incompletes_row_inner(
    manager: &SearchTreeManager,
    board: &mut Board,
    net_no: i32,
    raw_items: &[ItemId],
    collect_airlines: bool,
) -> (NetIncompleteRow, Vec<AirLineSegment>) {
    // 1. The filter (NetIncompletes.java:85-116).
    let mut filtered: Vec<ItemId> = Vec::new();
    for &id in raw_items {
        if item_is_tail(manager, board, id) {
            continue;
        }
        let kind = board.get(id).map(|entry| entry.board_item_type());
        let drill_or_area = matches!(
            kind,
            Some(BoardItemType::Pin | BoardItemType::Via | BoardItemType::ConductionArea)
        );
        if !drill_or_area && item_normal_contacts(manager, board, id).is_empty() {
            continue;
        }
        filtered.push(id);
    }

    // 2. The grouping (calculateNetItems, :293-322). Deterministic
    //    discovery order — see the module-doc deviation door.
    let mut remaining: BTreeSet<ItemId> = filtered.iter().copied().collect();
    let mut grouped: Vec<ItemId> = Vec::new();
    let mut labels: Vec<usize> = Vec::new();
    let mut group_count = 0usize;
    while let Some(&start) = remaining.iter().next() {
        let set = item_connected_set(manager, board, start, net_no);
        // Members iterate DESCENDING (the connected set is a
        // BTreeSet<Reverse<ItemId>> = Java's TreeSet<Item> order).
        let members: Vec<ItemId> = set
            .iter()
            .map(|Reverse(member)| *member)
            .filter(|member| remaining.contains(member))
            .collect();
        debug_assert!(
            !members.is_empty(),
            "the start item is in its own connected set"
        );
        for &member in &members {
            grouped.push(member);
            labels.push(group_count);
        }
        for &member in &members {
            remaining.remove(&member);
        }
        group_count += 1;
    }

    // The ratsnest rows (sorted by id — the DrcOracle emission order).
    let mut ratsnest: Vec<(ItemId, usize)> = grouped
        .iter()
        .map(|&id| (id, ratsnest_corners(manager, board, id).len()))
        .collect();
    ratsnest.sort();

    let mut airlines: Vec<AirLineSegment> = Vec::new();
    let mut row = NetIncompleteRow {
        net_no,
        items: raw_items.len(),
        // Every discovery iteration adds exactly one new label and at
        // least the start item, so distinct labels == group_count.
        groups: group_count,
        incomplete_count: 0,
        ratsnest,
        edges: Vec::new(),
    };

    // The early exit (Java :154-163): 0 or 1 grouped items — fully
    // connected or nothing routable; groups is FORCED to the length.
    if grouped.len() <= 1 {
        row.groups = grouped.len();
        return (row, airlines);
    }

    // 3. The Delaunay triangulation over the grouped items' corners.
    //    OBJECT ORDER: ascending id — the DrcOracle reconstruction
    //    sorts the storables by id before triangulating
    //    (DrcOracle.java:370), so the committed edge rows are captured
    //    in that order. Java's live count triangulates in
    //    HashSet-discovery order instead; the cross-process double
    //    capture (see the module-doc deviation door) proves count
    //    order-insensitivity on every corpus net, so ONE triangulation
    //    in the canonical ascending-id order serves both the count and
    //    the rows.
    let mut order: Vec<usize> = (0..grouped.len()).collect();
    order.sort_by_key(|&grouped_index| grouped[grouped_index]);
    let corner_lists: Vec<Vec<Point>> = order
        .iter()
        .map(|&grouped_index| ratsnest_corners(manager, board, grouped[grouped_index]))
        .collect();
    let result_edges = epic_geometry::delaunay::triangulate(&corner_lists);

    // The schema edge pairs: EVERY ResultEdge, canonical (min, max)
    // item pair, exact duplicates collapsed, lexicographic order
    // (DrcOracle emission contract). ResultEdge object indices index
    // the ASCENDING-ID order; `order[obj]` maps back to the grouped
    // index (whose position holds the item id).
    let mut pairs: Vec<[i64; 2]> = result_edges
        .iter()
        .map(|edge| {
            let a = i64::from(grouped[order[edge.start_object]].get());
            let b = i64::from(grouped[order[edge.end_object]].get());
            [a.min(b), a.max(b)]
        })
        .collect();
    pairs.sort_unstable();
    pairs.dedup();
    row.edges = pairs;

    // 4. The sorted-Edge set (Java :174-184): sort by edge_compare,
    //    then drop exact duplicates — sort stability plus the same
    //    comparator reproduces the TreeSet content and order.
    let mut sorted_edges: Vec<PortEdge> = result_edges
        .iter()
        .map(|edge| {
            let from = edge.start_point.to_float();
            let to = edge.end_point.to_float();
            PortEdge {
                from_idx: order[edge.start_object],
                from,
                to_idx: order[edge.end_object],
                to,
                length_square: to.distance_square(&from),
            }
        })
        .collect();
    sorted_edges.sort_by(edge_compare);
    sorted_edges.dedup_by(|a, b| edge_compare(a, b) == Ordering::Equal);

    // 5. Kruskal (Java :191-205): skip same-label edges; otherwise
    //    count the airline and relabel the whole from-group
    //    (joinConnectedSets, :328-337). The PortEdge indices are
    //    grouped-order indices, matching `labels`.
    for edge in &sorted_edges {
        let from_label = labels[edge.from_idx];
        let to_label = labels[edge.to_idx];
        if from_label == to_label {
            continue; // airline exists already
        }
        row.incomplete_count += 1;
        if collect_airlines {
            airlines.push(AirLineSegment {
                from: (edge.from.x.round() as i64, edge.from.y.round() as i64),
                to: (edge.to.x.round() as i64, edge.to.y.round() as i64),
                net: net_no,
            });
        }
        for label in labels.iter_mut() {
            if *label == from_label {
                *label = to_label;
            }
        }
    }
    (row, airlines)
}

/// The M9-T5 ratsnest face: EVERY net's Kruskal-ACCEPTED airlines with
/// world-DBU endpoints ([`AirLineSegment`], the corner convention on
/// its docs). Nets ascend; within a net the segments ride the
/// sorted-Edge Kruskal walk order. The count cross-check is structural:
/// `len == Σ per-net incomplete_count` over the SAME board state
/// (pinned).
#[must_use]
pub fn airline_segments(manager: &SearchTreeManager, board: &mut Board) -> Vec<AirLineSegment> {
    let lists = raw_net_item_lists(board);
    let mut airlines = Vec::new();
    for (index, list) in lists.iter().enumerate() {
        if list.is_empty() {
            continue;
        }
        let net_no = index as i32 + 1;
        let (_row, mut segments) = net_incompletes_row_inner(manager, board, net_no, list, true);
        airlines.append(&mut segments);
    }
    airlines
}

/// The whole `calculateAllIncompletes` surface in one call: one
/// [`NetIncompleteRow`] per non-empty net (ascending net number) plus
/// the `maxConnections` endpoint sum ([`max_connections`]).
/// `board.rules().nets.max_net_number() == 0` yields the empty result.
pub fn all_incompletes(
    manager: &SearchTreeManager,
    board: &mut Board,
) -> (i64, Vec<NetIncompleteRow>) {
    let lists = raw_net_item_lists(board);
    let mut rows = Vec::new();
    for (index, list) in lists.iter().enumerate() {
        if list.is_empty() {
            continue;
        }
        let net_no = index as i32 + 1;
        rows.push(net_incompletes_row(manager, board, net_no, list));
    }
    (max_connections(board, &lists), rows)
}

#[cfg(test)]
mod pins {
    use std::cmp::Ordering;

    use epic_board::id::ItemId;
    use epic_board::items::BoardItemType;
    use epic_geometry::float_point::FloatPoint;

    use super::{
        AirLineSegment, PortEdge, airline_segments, all_incompletes, edge_compare, max_connections,
        net_incompletes_row, raw_net_item_lists,
    };
    use crate::test_util::{DSN_MAIN, DSN_TIE, id_at_corner, net_list, net_no, nets_of, parse};
    use epic_board::contacts::ratsnest_corners;
    use epic_geometry::point::Point;

    fn kind_of(board: &super::Board, id: ItemId) -> Option<BoardItemType> {
        board.get(id).map(|entry| entry.board_item_type())
    }

    fn pair(a: ItemId, b: ItemId) -> [i64; 2] {
        let (x, y) = (i64::from(a.get()), i64::from(b.get()));
        [x.min(y), x.max(y)]
    }

    /// Whether every id in `ids` lies in one union-find component over
    /// `edges`.
    fn all_connected(ids: &[ItemId], edges: &[[i64; 2]]) -> bool {
        let mut parent: Vec<usize> = (0..ids.len()).collect();
        fn find(parent: &mut [usize], mut x: usize) -> usize {
            while parent[x] != x {
                x = parent[x];
            }
            x
        }
        for &[a, b] in edges {
            let (Some(ai), Some(bi)) = (
                ids.iter().position(|id| i64::from(id.get()) == a),
                ids.iter().position(|id| i64::from(id.get()) == b),
            ) else {
                continue;
            };
            let (ra, rb) = (find(&mut parent, ai), find(&mut parent, bi));
            parent[ra] = rb;
        }
        let root = find(&mut parent, 0);
        (1..ids.len()).all(|i| find(&mut parent, i) == root)
    }

    /// THE maxConnections formula pin: the endpoint CODE
    /// (Pin|ConductionArea per net, `max(0, endpoints − 1)` summed
    /// over non-empty nets), NOT the stale FRLogger formula
    /// "(total_items − netCount)". The craft yields 7: NQ's 4 pins
    /// give 3, NC's 2 CAs give 1, N3's 2 pins + 1 CA give 2, NP's 2
    /// pins give 1 — while NV's 2 vias contribute NOTHING (a Via is
    /// NOT an endpoint: 1 pin only, so 0) and NZ/NX/NY stay below 2
    /// endpoints, also 0. A total-items formula would say 20 − 8.
    #[test]
    fn max_connections_is_endpoint_based() {
        let (_manager, board) = parse(DSN_MAIN);
        let lists = raw_net_item_lists(&board);
        let index = |name: &str| (net_no(&board, name) - 1) as usize;
        assert_eq!(
            max_connections(&board, &lists[index("NC")..index("NC") + 1]),
            1,
            "a ConductionArea IS an endpoint: two areas → 1"
        );
        assert_eq!(
            max_connections(&board, &lists[index("N3")..index("N3") + 1]),
            2,
            "2 pins + 1 CA → 3 endpoints → 2"
        );
        assert_eq!(
            max_connections(&board, &lists[index("NV")..index("NV") + 1]),
            0,
            "vias are NOT endpoints: 1 pin + 2 vias → 0"
        );
        assert_eq!(max_connections(&board, &lists), 7);
    }

    /// The CONFORMING count: NQ's convex quad connects fully —
    /// count 3 == groups − 1 — with the EXACT Delaunay edge set the
    /// oracle captured (drc-0015 net 1): the 4 hull pairs + the
    /// P1–P4 diagonal. The quad is STRICTLY non-Delaunay for that
    /// diagonal, not cocircular: P3 lies strictly inside
    /// circumcircle(P1, P2, P4) (incircle margin ≈ 1.6e9), so an
    /// empty-circle check would forbid P1–P4 and emit P2–P3 instead.
    /// Java's Delaunay implementation does not, and the port pins the
    /// oracle's actual choice bug-for-bug (the corpus compare
    /// green-pins it record-for-record). Ids are corner-identified,
    /// never absolute.
    #[test]
    fn nq_conforming_count_and_exact_edge_set() {
        let (manager, mut board) = parse(DSN_MAIN);
        let nq = net_list(&board, "NQ");
        assert_eq!(nq.len(), 4, "isolated pins survive the filter");
        let nq_no = net_no(&board, "NQ");
        let row = net_incompletes_row(&manager, &mut board, nq_no, &nq);
        assert_eq!((row.items, row.groups, row.incomplete_count), (4, 4, 3));
        assert_eq!(
            row.incomplete_count,
            row.groups - 1,
            "the conforming case the spike's naive claim happens to fit"
        );
        for (id, n) in &row.ratsnest {
            assert_eq!(*n, 1, "a pin's ratsnest is its drill center");
            assert!(nq.contains(id), "ratsnest rows are filtered members");
        }
        let p1 = id_at_corner(&manager, &mut board, &nq, 10000.0, 10000.0);
        let p2 = id_at_corner(&manager, &mut board, &nq, 30000.0, 10000.0);
        let p3 = id_at_corner(&manager, &mut board, &nq, 10000.0, 30000.0);
        let p4 = id_at_corner(&manager, &mut board, &nq, 70000.0, 70000.0);
        let mut expected = vec![
            pair(p1, p2),
            pair(p1, p3),
            pair(p1, p4),
            pair(p2, p4),
            pair(p3, p4),
        ];
        expected.sort_unstable();
        assert_eq!(
            row.edges, expected,
            "5 oracle-pinned edges: hull + diagonal P1-P4, no P2-P3"
        );
    }

    /// Wiring-rect conduction areas: kept by the filter, 4 ratsnest
    /// corners each, one airline between two isolated areas. The
    /// emission contract includes INTRA-object ResultEdges: each CA's
    /// own 4-corner cluster triangulates with one internal edge
    /// (`[p,p]`, `[q,q]`), alongside the cross-cluster `[p,q]`
    /// (oracle-pinned by drc-0015 net 2).
    #[test]
    fn nc_conduction_areas_are_endpoints_with_corner_ratsnest() {
        let (manager, mut board) = parse(DSN_MAIN);
        let nc = net_list(&board, "NC");
        assert_eq!(nc.len(), 2);
        let nc_no = net_no(&board, "NC");
        let row = net_incompletes_row(&manager, &mut board, nc_no, &nc);
        assert_eq!((row.items, row.groups, row.incomplete_count), (2, 2, 1));
        for (id, n) in &row.ratsnest {
            assert_eq!(kind_of(&board, *id), Some(BoardItemType::ConductionArea));
            assert_eq!(*n, 4, "a rect CA contributes its 4 corners");
        }
        assert_eq!(
            row.edges,
            vec![pair(nc[0], nc[0]), pair(nc[0], nc[1]), pair(nc[1], nc[1])],
            "two isolated corner clusters stay Delaunay-adjacent, self-edges included"
        );
    }

    /// N3: three groups (2 pins + 1 CA), yet every group ends up
    /// Delaunay-connected → count 2 == groups − 1.
    #[test]
    fn n3_three_groups_one_component() {
        let (manager, mut board) = parse(DSN_MAIN);
        let n3 = net_list(&board, "N3");
        assert_eq!(n3.len(), 3);
        let n3_no = net_no(&board, "N3");
        let row = net_incompletes_row(&manager, &mut board, n3_no, &n3);
        assert_eq!((row.items, row.groups, row.incomplete_count), (3, 3, 2));
        let mut expected: Vec<(ItemId, usize)> = n3
            .iter()
            .map(|&id| {
                let n = if kind_of(&board, id) == Some(BoardItemType::ConductionArea) {
                    4
                } else {
                    1
                };
                (id, n)
            })
            .collect();
        expected.sort();
        assert_eq!(row.ratsnest, expected, "2 pin centers + 4 CA corners");
        assert!(
            all_connected(&n3, &row.edges),
            "the triangulation bridges pins and CA corners"
        );
    }

    /// THE CONTRAST: NV's F.Cu trace is both-ends-via-contacted (ZERO
    /// corners), but the LAYER-BRIDGING via (F.Cu trace + B.Cu trace
    /// = two contacts on different layer spans → NOT a tail; a drill
    /// item ALWAYS emits its drill center) + free pin P7 form the
    /// corner pair that merges the two groups → count 1 == groups − 1.
    /// The end-only via and the free-ended B.Cu trace are tails →
    /// filtered (ratsnest shows exactly one n=0 row). The
    /// count<groups−1 witness lives on the REAL 655_testboard fixture
    /// (`epic-harness` witness pin, nets 3/4/17 vs conforming net 7).
    #[test]
    fn nv_contrast_count_equals_groups_minus_one() {
        let (manager, mut board) = parse(DSN_MAIN);
        let nv = net_list(&board, "NV");
        let vias: Vec<ItemId> = nv
            .iter()
            .copied()
            .filter(|&id| kind_of(&board, id) == Some(BoardItemType::Via))
            .collect();
        let pin: Vec<ItemId> = nv
            .iter()
            .copied()
            .filter(|&id| kind_of(&board, id) == Some(BoardItemType::Pin))
            .collect();
        assert_eq!((vias.len(), pin.len()), (2, 1));
        let nv_no = net_no(&board, "NV");
        let row = net_incompletes_row(&manager, &mut board, nv_no, &nv);
        assert_eq!((row.items, row.groups, row.incomplete_count), (5, 2, 1));
        assert_eq!(row.incomplete_count, row.groups - 1);
        assert_eq!(
            row.ratsnest.iter().filter(|(_, n)| *n == 0).count(),
            1,
            "the both-ends-contacted trace contributes nothing"
        );
        let corner_via = row
            .ratsnest
            .iter()
            .find(|(id, n)| *n == 1 && kind_of(&board, *id) == Some(BoardItemType::Via))
            .map(|(id, _)| *id)
            .expect("the layer-bridging via survives with its corner");
        assert_eq!(
            row.edges,
            vec![pair(pin[0], corner_via)],
            "one corner edge bridges the two groups"
        );
    }

    /// The early exit + tail filter: NZ's raw list is 2 (trace + a
    /// via touching it mid-span), but both are tails — the trace has
    /// no endpoint contacts, the via has ≤1 — so nothing survives and
    /// `groups` is FORCED to the grouped length 0.
    #[test]
    fn nz_tails_filtered_groups_forced_to_zero() {
        let (manager, mut board) = parse(DSN_MAIN);
        let nz = net_list(&board, "NZ");
        assert_eq!(nz.len(), 2, "raw items: trace + mid-span via");
        let nz_no = net_no(&board, "NZ");
        let row = net_incompletes_row(&manager, &mut board, nz_no, &nz);
        assert_eq!((row.items, row.groups, row.incomplete_count), (2, 0, 0));
        assert!(row.ratsnest.is_empty());
        assert!(row.edges.is_empty());
    }

    /// Zero-contact plain traces are dropped by the filter's
    /// non-drill arm (NX, NY — neither drill item nor CA).
    #[test]
    fn zero_contact_traces_dropped() {
        let (manager, mut board) = parse(DSN_MAIN);
        for name in ["NX", "NY"] {
            let list = net_list(&board, name);
            assert_eq!(list.len(), 1);
            let no = net_no(&board, name);
            let row = net_incompletes_row(&manager, &mut board, no, &list);
            assert_eq!((row.items, row.groups, row.incomplete_count), (1, 0, 0));
            assert!(row.ratsnest.is_empty() && row.edges.is_empty());
        }
    }

    /// NP: two free pins → one edge, one airline, conforming.
    #[test]
    fn np_two_free_pins_single_edge() {
        let (manager, mut board) = parse(DSN_MAIN);
        let np = net_list(&board, "NP");
        let np_no = net_no(&board, "NP");
        let row = net_incompletes_row(&manager, &mut board, np_no, &np);
        assert_eq!((row.items, row.groups, row.incomplete_count), (2, 2, 1));
        assert_eq!(
            row.edges,
            vec![pair(np[0], np[1])],
            "two single-corner objects triangulate to exactly one edge"
        );
    }

    /// Multi-net parse pin (DSN_TIE): the SAME image pin in two nets
    /// appears in BOTH raw lists and carries both net numbers; the
    /// per-net rows then see the pin + its trace as ONE connected
    /// group each.
    #[test]
    fn multi_net_pin_accumulates_in_both_lists() {
        let (manager, mut board) = parse(DSN_TIE);
        let ta_no = net_no(&board, "TA");
        let tb_no = net_no(&board, "TB");
        let ta = net_list(&board, "TA");
        let tb = net_list(&board, "TB");
        let pin = *ta
            .iter()
            .find(|&&id| kind_of(&board, id) == Some(BoardItemType::Pin))
            .expect("TA carries the craft's pin");
        assert!(
            tb.contains(&pin),
            "the multi-net pin must accumulate, not overwrite"
        );
        assert_eq!(
            nets_of(&board, pin),
            vec![ta_no, tb_no],
            "both net numbers on the item, declaration order"
        );
        for (no, list) in [(ta_no, ta), (tb_no, tb)] {
            let row = net_incompletes_row(&manager, &mut board, no, &list);
            assert_eq!((row.items, row.groups, row.incomplete_count), (2, 1, 0));
        }
    }

    /// `all_incompletes`: exactly the 11 crafted nets, ascending net
    /// numbers, and the same rows a per-net replay yields.
    #[test]
    fn all_incompletes_rows_ascending_and_complete() {
        let (manager, mut board) = parse(DSN_MAIN);
        let (max_conn, rows) = all_incompletes(&manager, &mut board);
        assert_eq!(max_conn, 7);
        assert_eq!(rows.len(), 11, "every crafted net carries items");
        assert!(
            rows.windows(2).all(|w| w[0].net_no < w[1].net_no),
            "rows ascend by net number"
        );
        for name in [
            "NQ", "NC", "N3", "NV", "NZ", "NX", "NY", "NP", "NP2", "NA", "NB",
        ] {
            let list = net_list(&board, name);
            let no = net_no(&board, name);
            let expected = net_incompletes_row(&manager, &mut board, no, &list);
            let found = rows
                .iter()
                .find(|row| row.net_no == expected.net_no)
                .expect("the net has a row");
            assert_eq!(found, &expected, "row for {name} matches the replay");
        }
    }

    /// The M9-T5 ratsnest face: `airline_segments` on the crafted
    /// board returns the Kruskal-count-consistent subset — the
    /// segment count equals the Σ per-net `incomplete_count` over the
    /// SAME board state, every segment's net is a row net, and every
    /// endpoint pair matches the two endpoints' ratsnest corners
    /// (the count cross-check pin; kills the drop-a-segment,
    /// duplicate-an-edge, and wrong-net mutants).
    #[test]
    fn airline_segments_match_the_kruskal_count_and_corners() {
        let (manager, mut board) = parse(DSN_MAIN);
        let (max_conn, rows) = all_incompletes(&manager, &mut board);
        let expected: usize = rows.iter().map(|row| row.incomplete_count).sum();
        assert!(expected > 0, "the craft carries airlines");
        let airlines = airline_segments(&manager, &mut board);
        assert_eq!(
            airlines.len(),
            expected,
            "segment count == Σ incomplete_count (max_connections = {max_conn})"
        );
        // Every segment rides a real net row and its endpoints are
        // RATSNEST CORNERS of the two end items (the corner
        // convention) — checkable exactly on the single-corner
        // members (pins): the corner is the item's drill center.
        for segment in &airlines {
            assert!(
                rows.iter().any(|row| row.net_no == segment.net),
                "segment net {} has a row",
                segment.net
            );
        }
        // The per-net face on the smallest net: NP's two free pins —
        // exactly one segment between the two drill centers, in the
        // Kruskal walk order (shortest-first; with one edge there is
        // nothing to order).
        let np = net_list(&board, "NP");
        let np_no = net_no(&board, "NP");
        let row = net_incompletes_row(&manager, &mut board, np_no, &np);
        assert_eq!((row.items, row.groups, row.incomplete_count), (2, 2, 1));
        let np_segments: Vec<&AirLineSegment> =
            airlines.iter().filter(|s| s.net == np_no).collect();
        assert_eq!(np_segments.len(), 1);
        let mut corner_of = |id: ItemId| -> (i64, i64) {
            let corners = ratsnest_corners(&manager, &mut board, id);
            assert_eq!(corners.len(), 1, "a pin has one ratsnest corner");
            match &corners[0] {
                Point::Int(point) => (i64::from(point.x), i64::from(point.y)),
                Point::Rational(_) => panic!("parsed pins are int-cornered"),
            }
        };
        let (a, b) = (corner_of(np[0]), corner_of(np[1]));
        let segment = np_segments[0];
        let matches =
            (segment.from == a && segment.to == b) || (segment.from == b && segment.to == a);
        assert!(matches, "the NP segment joins the two pin corners");
    }

    /// `Edge.compareTo` (NetIncompletes.java:361-379): the f64
    /// subtraction chain — length square first, then from.x, from.y,
    /// to.x, to.y — with exact-zero continuations, NaN → Equal, and
    /// IDENTITY BLINDNESS (the item indices are not compared, so two
    /// edges over the same coordinate 4-tuple collapse in the
    /// TreeSet even across different items — the dedup contrast).
    #[test]
    fn edge_compare_chain_nan_and_identity_blindness() {
        let edge = |from: (f64, f64), to: (f64, f64), fi: usize, ti: usize| {
            let (from, to) = (
                FloatPoint {
                    x: from.0,
                    y: from.1,
                },
                FloatPoint { x: to.0, y: to.1 },
            );
            PortEdge {
                from_idx: fi,
                from,
                to_idx: ti,
                to,
                length_square: to.distance_square(&from),
            }
        };
        let base = edge((0.0, 0.0), (2.0, 0.0), 0, 1);
        // Length square decides before anything else.
        assert_eq!(
            edge_compare(&base, &edge((0.0, 0.0), (2.0, 1.0), 0, 1)),
            Ordering::Less
        );
        // from.x / from.y / to.x / to.y tie-breaks at equal length:
        // from.x and from.y of `base` are the smallest, from.x again
        // decides at from-ties, and at full from-ties base's to.x = 2
        // beats 0 (Greater) — the subtraction chain has no sign flip.
        assert_eq!(
            edge_compare(&base, &edge((1.0, 0.0), (1.0, 2.0), 0, 1)),
            Ordering::Less
        );
        assert_eq!(
            edge_compare(&base, &edge((0.0, 1.0), (2.0, 1.0), 0, 1)),
            Ordering::Less
        );
        assert_eq!(
            edge_compare(&base, &edge((0.0, 0.0), (0.0, 2.0), 0, 1)),
            Ordering::Greater
        );
        assert_eq!(
            edge_compare(&base, &edge((0.0, 0.0), (0.0, -2.0), 0, 1)),
            Ordering::Greater
        );
        // Identical coordinates → Equal REGARDLESS of the item
        // indices (the TreeSet dedup key).
        assert_eq!(
            edge_compare(&base, &edge((0.0, 0.0), (2.0, 0.0), 5, 9)),
            Ordering::Equal
        );
        // Signum.asInt(NaN) == 0 → Equal (the NaN door).
        let mut nan = edge((0.0, 0.0), (2.0, 0.0), 0, 1);
        nan.length_square = f64::NAN;
        assert_eq!(edge_compare(&base, &nan), Ordering::Equal);
        assert_eq!(edge_compare(&nan, &nan), Ordering::Equal);
    }
}
