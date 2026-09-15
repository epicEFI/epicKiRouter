//! The trace COMBINE seam (M2 Task 11): Java `PolylineTrace.combine`
//! (`PolylineTrace.java:175-192`) and its join halves, plus the
//! cycle-removal closure (`BasicBoard.removeIfCycle` /
//! `getTraceTail`, `Trace.isCycle`, `Item.isCycleRecu` /
//! `getConnectionItems`) and [`insert_trace_without_cleaning`]
//! (`BasicBoard.java:179-203`) — the insertion path every crafted
//! fixture here replays.
//!
//! Jar-pinned by `rust/harness/oracle/CombineSpike.java` (capture
//! `/tmp/epic-t11-combine.out`, plus the Y1 hang-probe capture
//! `/tmp/epic-t11-combine-hang.out`; both run twice and diffed —
//! deterministic). The tests below quote LITERAL capture rows.
//!
//! # The combine loop and its halves
//!
//! `combine()` is `while (isOnTheBoard() && (combineAtStart(true) ||
//! combineAtEnd(true)))` — the START half is tried first every
//! iteration, and the on-board recheck matters because a join can
//! remove the receiver itself (see the shrink rule below).
//! `combineAtStart` (`:201-332`): contacts at the FIRST corner via
//! `getNormalContacts(corner, false)`; conduction areas are stripped
//! when `ignoreAreas` (combine always passes `true` — capture R6);
//! anything but EXACTLY one contact refuses; the other item must be a
//! `PolylineTrace` passing the equality gate (layer, nets, half
//! width, fixed state — and NEITHER item may be
//! deletion-forbidden). The corner match decides direction:
//! `start == other.lastCorner` joins normally, `start ==
//! other.firstCorner` joins the other trace REVERSED (lines reversed
//! AND each line opposited), anything else refuses.
//! `combineAtEnd` (`:341-456`) mirrors at the LAST corner
//! (`end == other.firstCorner` normal, `== other.lastCorner`
//! reversed). The collinear-skip test is
//! `isEqualOrOpposite` between the OTHER trace's second-to-last line
//! (atStart) / the RECEIVER's second-to-last line (atEnd) and the
//! receiver's / other's second line respectively.
//!
//! Assembly reproduces Java's overlapping `arraycopy`: atStart lays
//! down `otherLines[0..len-1]`, then overwrites the slot at
//! `joinPos = otherLines.len()-1` (minus one when skipping) with
//! `aLines[1..]`; atEnd lays `aArr[0..len-1]` then the other's
//! `lines[1..]`. `Polyline::new` canonicalizes exactly like Java's
//! `new Polyline(newLines)` (D2: a 4-line input collapses to 3 lines
//! / 2 corners — capture `joinedLen=3 joinedCorners=2`).
//!
//! # D18: replaceGeometry always (documented gap)
//!
//! Java branches on `hasDefaultEntries()`: with default tree entries
//! it calls `mergeEntriesInFront/Behind` (an in-place entry-list
//! splice, capture `branch=MERGE_ENTRIES`), otherwise
//! `replaceGeometry`. The port ALWAYS takes the
//! [`replace_geometry`] path (`remove` → `set_polyline` →
//! `clear_derived_data` → `insert`,
//! `PolylineTraceSearchTreeAdapter.java:34-40`; the adapter's
//! explicit `clearSearchTreeEntries` is subsumed by
//! [`SearchTreeManager::remove`], which drops the same state).
//! Consequence, verified against the capture: the LEAF sets and leaf
//! counts match everywhere (C1 17, C2 18, C3 16, R6 16, D2 17), but
//! the I-node topology after a merge case can differ from Java's
//! `MERGE_ENTRIES` history — that is a DOCUMENTED D18 divergence, so
//! the merge-case pins compare leaf counts + survivor leaf rows
//! (geometry-derived, structure-independent), while D2 — where Java
//! itself took `branch=REPLACE_GEOMETRY` (capture
//! `D2_PRED branch=REPLACE_GEOMETRY newCount=4 joinedLen=3`) — is
//! pinned BYTE-IDENTICAL on the full 33-row tree dump.
//!
//! # The shrink rule and the loop's on-board recheck
//!
//! After replacing the geometry, Java removes the receiver when
//! `this.lines.lines.length < 3` (a single-segment leftover) and then
//! ALWAYS removes the joined trace — through the REPOSITORY
//! (`BoardItemRepository.removeItem`, `:170-199`), whose
//! deletion-forbidden skip (`:189`) comes first, so a forbidden item
//! silently stays. The `isOnTheBoard()` recheck in the combine loop
//! exists for exactly the self-removal case.
//!
//! # The connection walk (NONE-only form)
//!
//! `getConnectionItems` (`Item.java:736-815`) seeds with the item's
//! normal contacts, requires a `normalContactPoint` against the
//! receiver, applies the EXACTLY-ONE-CONTACT contract at the receiver
//! corner (trace receivers only:
//! `getNormalContacts(prevPoint, false).size() != 1` skips the
//! branch), then walks: each step demands `isRoutable`, stops at vias
//! whose `StopConnectionOption` is VIA/FANOUT_VIA (the port's surface
//! only produces NONE, which never stops — the check is unreachable
//! and absent; `isFanoutVia` is M3), and continues to the unique next
//! contact whose (layer, point) differs from the arrival pair. A NULL
//! contact point mid-walk, or a SECOND new contact, is a fork: the
//! branch ends. Arrival-skip compares against the PREVIOUS point —
//! two new contacts at the SAME point are a fork, not a skip. The
//! walk terminates through fork/dead-end detection (no visited set);
//! Y1 (capture `/tmp/epic-t11-combine-hang.out`) pins all three
//! result sets and every pairwise (layer, point) input.
//!
//! # Cycles
//!
//! `isCycle` (`Trace.java:273-320`): the `isOverlap` shortcut first
//! (start/end contact sets intersect), then a DFS from ALL start
//! contacts over [`item_normal_contacts`] with a shared visited set.
//! `isCycleRecu` (`Item.java:690-713`): conduction areas are
//! cycle-blind when the net's class says so
//! ([`NetClass::ignore_cycles_with_areas`], parse-constant false);
//! the `comeFrom` skip PRECEDES the search-item check (Y3: the seed
//! block makes the via triangle acyclic); recursion depth is
//! Java-parity (unbounded — boards are finite). The net-49 debug
//! logging in Java's `isCycle` is log-only (D12) and not ported.
//! `removeIfCycle` (`BasicBoard.java:1336-1366`) removes the
//! connection set, then — for each endpoint that had NO tail before —
//! re-queries [`get_trace_tail`] and removes THAT tail's connection
//! set too (Y7: the phase-2 pass eats the partner trace).
//!
//! # Not ported (GUI/undo anchors, T14)
//!
//! `saveForUndo` (`:279`), the observer notifications and
//! `additionalUpdateAfterChange` (`:187-189`) are GUI/undo-surface
//! side effects outside this seam; `joinChangedArea`-style DRC
//! bookkeeping belongs to the M4 checker. `mergeEntries*` itself is
//! the D18 gap above.

use std::cmp::Reverse;
use std::collections::BTreeSet;

use epic_geometry::point::Point;
use epic_geometry::polyline::Polyline;
use epic_geometry::regular_tile_shape::RegularTileShape;
use epic_geometry::tile_shape::TileShape;

use crate::board::{Board, ItemEntry};
use crate::contacts;
use crate::id::ItemId;
use crate::items::trace::{first_corner, last_corner};
use crate::items::{FixedState, ItemData};
use crate::rules_surf::NetClass;
use crate::rules_surf::is_normal_net_number;
use crate::tree_manager::SearchTreeManager;

// ---------------------------------------------------------------------------
// net predicates (Item + Nets)
// ---------------------------------------------------------------------------

/// Java `Item.containsNet(int)` (`Item.java:149-152`): the `<= 0`
/// guard — net 0 is NOT a contained net (contrast
/// [`crate::contacts::shares_net_no`], the CONTACT rule, which has no
/// guard).
#[must_use]
pub fn contains_net(nets: &[i32], net: i32) -> bool {
    net > 0 && nets.contains(&net)
}

/// Java `Item.netsEqual(int[] other)` (`Item.java:1234-1244`): same
/// length, every entry contained — through [`contains_net`], so two
/// `[0]` arrays are NOT equal.
#[must_use]
pub fn nets_equal(a: &[i32], b: &[i32]) -> bool {
    a.len() == b.len() && a.iter().all(|&net| contains_net(b, net))
}

/// Java `Item.netsNormal()`: every net number normal
/// (`Nets.isNormalNetNumber`, re-exported from
/// [`crate::rules_surf`]).
#[must_use]
pub fn nets_normal(nets: &[i32]) -> bool {
    nets.iter().all(|&net| is_normal_net_number(net))
}

/// Java `Item.isUserFixed()` (`Item.java:861-863`): the fixed state is
/// USER_FIXED or stronger (SHOVE_FIXED does NOT count — R3/R4).
#[must_use]
pub fn is_user_fixed(entry: &ItemEntry) -> bool {
    entry.fixed >= FixedState::UserFixed
}

/// Java `Item.isDeletionForbidden()` (`Item.java:866-875`): a
/// component attachment (`componentId > 0`), a user-fixed state, or a
/// conduction area on a NON-signal layer. A missing id is NOT
/// forbidden (Java would NPE; unreachable through the seam). An
/// out-of-range conduction-area layer reads as forbidden (Java would
/// throw AIOOBE — the same "cannot delete it" outcome).
#[must_use]
pub fn is_deletion_forbidden(board: &Board, id: ItemId) -> bool {
    let Some(entry) = board.get(id) else {
        return false;
    };
    if entry.component_id > 0 || is_user_fixed(entry) {
        return true;
    }
    match &entry.data {
        ItemData::ConductionArea { layer, .. } => {
            let is_signal = usize::try_from(*layer)
                .ok()
                .and_then(|index| board.layers().layers.get(index))
                .is_some_and(|layer| layer.is_signal);
            !is_signal
        }
        _ => false,
    }
}

/// Java `Item.isRoutable()` (`Item.java:855-858`):
/// `!isUserFixed() && netCount() > 0`.
fn is_routable(board: &Board, id: ItemId) -> bool {
    match board.get(id) {
        Some(entry) => !is_user_fixed(entry) && !entry.nets.is_empty(),
        None => false,
    }
}

/// Java `Item.firstCommonLayer(Item)` (`Item.java:355-370`):
/// `max(firstLayer) > min(lastLayer)` means disjoint intervals → -1,
/// else the shared first layer. A missing id yields -1 (Java would
/// NPE; unreachable through the walk).
pub fn first_common_layer(board: &mut Board, a: ItemId, b: ItemId) -> i32 {
    let Some(first_a) = board.item_first_layer(a) else {
        return -1;
    };
    let Some(first_b) = board.item_first_layer(b) else {
        return -1;
    };
    let Some(last_a) = board.item_last_layer(a) else {
        return -1;
    };
    let Some(last_b) = board.item_last_layer(b) else {
        return -1;
    };
    let max_first_layer = first_a.max(first_b);
    if max_first_layer > last_a.min(last_b) {
        -1
    } else {
        max_first_layer
    }
}

// ---------------------------------------------------------------------------
// normal contact point (the double dispatch)
// ---------------------------------------------------------------------------

/// The receiver kind classes the Java double dispatch walks: Trace
/// bodies, DrillItem (Pin/Via) bodies, and everything else bottoming
/// out at `Item.normalContactPoint(Item)` = null
/// (`Item.java:620-623`) — notably Trace ↔ ConductionArea, the R6
/// walk's fork source.
#[derive(Clone, Copy, PartialEq)]
enum ContactKind {
    Trace,
    Drill,
    Other,
}

fn contact_kind(board: &Board, id: ItemId) -> ContactKind {
    match board.get(id).map(|entry| &entry.data) {
        Some(ItemData::Trace { .. }) => ContactKind::Trace,
        Some(ItemData::Pin { .. } | ItemData::Via { .. }) => ContactKind::Drill,
        _ => ContactKind::Other,
    }
}

/// Java `Item.normalContactPoint(Item)` over the dispatch chain —
/// `null` = `None`. Per pair:
///
/// * Trace ↔ Trace (`Trace.java:138-153`): EXACT layer equality
///   first; then `contactAtFirstCorner` / `contactAtLastCorner` —
///   exactly one of the two must hold (both = null, the doubled-back
///   Y7 case; neither = null, the mid-corner Y1 case); the answer is
///   the receiver's first or last corner respectively.
/// * Drill ↔ Trace (`DrillItem.java:341-350`): sharesLayer and the
///   drill center ON a trace endpoint → the center.
/// * Drill ↔ Drill (`DrillItem.java:334-339`): sharesLayer and equal
///   centers → the receiver's center (after the dispatch swap, the
///   SECOND argument's).
/// * Anything else: `None` (the `Item` default, `:620-623`).
///
/// A missing id is `None` (Java would NPE; unreachable through the
/// walk — every id comes from a live contact set).
pub fn normal_contact_point(board: &mut Board, a: ItemId, b: ItemId) -> Option<Point> {
    match (contact_kind(board, a), contact_kind(board, b)) {
        (ContactKind::Trace, ContactKind::Trace) => {
            if board.trace_layer(a) != board.trace_layer(b) {
                return None;
            }
            let lines_a = board.trace_polyline(a)?;
            let lines_b = board.trace_polyline(b)?;
            let first = first_corner(lines_a);
            let last = last_corner(lines_a);
            let other_first = first_corner(lines_b);
            let other_last = last_corner(lines_b);
            let at_first =
                first.as_ref() == other_first.as_ref() || first.as_ref() == other_last.as_ref();
            let at_last =
                last.as_ref() == other_first.as_ref() || last.as_ref() == other_last.as_ref();
            if at_first == at_last {
                return None;
            }
            if at_first { first } else { last }
        }
        (ContactKind::Drill, ContactKind::Trace) | (ContactKind::Trace, ContactKind::Drill) => {
            let (drill, trace) = if contact_kind(board, a) == ContactKind::Drill {
                (a, b)
            } else {
                (b, a)
            };
            if !contacts::items_share_layer(board, drill, trace) {
                return None;
            }
            let center = board.drill_center(drill)?;
            let trace_lines = board.trace_polyline(trace)?;
            if first_corner(trace_lines).as_ref() == Some(&center)
                || last_corner(trace_lines).as_ref() == Some(&center)
            {
                Some(center)
            } else {
                None
            }
        }
        (ContactKind::Drill, ContactKind::Drill) => {
            if !contacts::items_share_layer(board, a, b) {
                return None;
            }
            let center_b = board.drill_center(b)?;
            if board.drill_center(a).as_ref() == Some(&center_b) {
                Some(center_b)
            } else {
                None
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// getNormalContacts() per kind + getConnectionItems
// ---------------------------------------------------------------------------

/// Java `DrillItem.getNormalContacts()` (`DrillItem.java:274-311`) —
/// the no-arg form vias (and pins) inherit: an ALL-LAYER overlap at
/// the drill center (`layer = -1`), sharesNet REQUIRED (this form has
/// no ignore flag), sharesLayer, then the per-kind acceptance at the
/// center (trace corner / drill center / area contains). Descending
/// order from the tree query.
fn drill_item_contacts(manager: &SearchTreeManager, board: &mut Board, id: ItemId) -> Vec<ItemId> {
    let Some(center) = board.drill_center(id) else {
        return Vec::new();
    };
    let search_shape =
        TileShape::RegularTileShape(RegularTileShape::IntBox(center.surrounding_box()));
    let candidates = manager.overlapping_objects(
        board,
        SearchTreeManager::DEFAULT_TREE_INDEX,
        &search_shape,
        -1,
        &[],
    );
    let mut result = Vec::new();
    for candidate in candidates {
        if candidate == id {
            continue;
        }
        if !contacts::items_share_net(board, id, candidate) {
            continue;
        }
        if !contacts::items_share_layer(board, id, candidate) {
            continue;
        }
        if contacts::point_accepts_contact(board, candidate, &center) {
            result.push(candidate);
        }
    }
    result
}

/// Java `Item.getNormalContacts()` (`Item.java:613-615` default +
/// the Trace / DrillItem overrides) — per kind: a trace queries both
/// endpoint corners ([`contacts::all_contacts`]), a drill item the
/// center ([`drill_item_contacts`]), everything else the empty set.
pub fn item_normal_contacts(
    manager: &SearchTreeManager,
    board: &mut Board,
    id: ItemId,
) -> Vec<ItemId> {
    match contact_kind(board, id) {
        ContactKind::Trace => contacts::all_contacts(manager, board, id),
        ContactKind::Drill => drill_item_contacts(manager, board, id),
        ContactKind::Other => Vec::new(),
    }
}

/// Java `Item.getConnectionItems()` (`Item.java:736-815`) — the fork-
/// terminated connected walk (module docs). Descending ids (the
/// `TreeSet` under `Item.compareTo`).
pub fn get_connection_items(
    manager: &SearchTreeManager,
    board: &mut Board,
    item_id: ItemId,
) -> Vec<ItemId> {
    let mut result: BTreeSet<Reverse<ItemId>> = BTreeSet::new();
    if is_routable(board, item_id) {
        result.insert(Reverse(item_id));
    }
    let receiver_is_trace = matches!(
        board.get(item_id).map(|entry| &entry.data),
        Some(ItemData::Trace { .. })
    );
    for mut current in item_normal_contacts(manager, board, item_id) {
        // The arrival point against the RECEIVER (:765-769).
        let Some(mut prev_contact_point) = normal_contact_point(board, item_id, current) else {
            continue;
        };
        let mut prev_contact_layer = first_common_layer(board, item_id, current);
        // The exactly-one-contact contract, TRACE receivers only
        // (:770-778).
        if receiver_is_trace
            && contacts::normal_contacts(manager, board, item_id, &prev_contact_point, false).len()
                != 1
        {
            continue;
        }
        // The walk (:780-823): forks and dead ends terminate.
        loop {
            if !is_routable(board, current) {
                break;
            }
            // The Via stop options (:783-792) are VIA / FANOUT_VIA;
            // this surface only produces NONE, which never stops —
            // the branch is unreachable and absent (isFanoutVia = M3).
            result.insert(Reverse(current));
            let mut next: Option<(ItemId, Point, i32)> = None;
            let mut fork_found = false;
            for tmp_contact in item_normal_contacts(manager, board, current) {
                let tmp_layer = first_common_layer(board, current, tmp_contact);
                if tmp_layer < 0 {
                    continue;
                }
                let Some(tmp_point) = normal_contact_point(board, current, tmp_contact) else {
                    fork_found = true;
                    break;
                };
                // The arrival-skip compares against the PREVIOUS pair;
                // a second NEW contact is a fork (:804-814).
                if prev_contact_layer != tmp_layer || prev_contact_point != tmp_point {
                    if next.is_some() {
                        fork_found = true;
                        break;
                    }
                    next = Some((tmp_contact, tmp_point, tmp_layer));
                }
            }
            if fork_found {
                break; // the fork terminates the branch (:804-814)
            }
            let Some((next_id, next_point, next_layer)) = next else {
                break; // dead end — the branch terminates
            };
            current = next_id;
            prev_contact_point = next_point;
            prev_contact_layer = next_layer;
        }
    }
    result.into_iter().map(|Reverse(id)| id).collect()
}

// ---------------------------------------------------------------------------
// cycles
// ---------------------------------------------------------------------------

/// Java `Trace.isCycle()` (`Trace.java:273-320`) — the `isOverlap`
/// shortcut (`:228-233`: start and end contact sets intersect), then
/// the DFS from ALL start contacts. A non-trace id simply has no
/// contacts and returns false (Java's receiver type guards it).
pub fn is_cycle(manager: &SearchTreeManager, board: &mut Board, trace_id: ItemId) -> bool {
    let start = contacts::start_contacts(manager, board, trace_id);
    let end = contacts::end_contacts(manager, board, trace_id);
    if start.iter().any(|id| end.contains(id)) {
        return true;
    }
    // The visited set seeds with ALL start contacts (:296-305).
    let mut visited: BTreeSet<ItemId> = start.iter().copied().collect();
    // ignoreAreas from net[0]'s class (:307-313) — null-guarded, and
    // the port's class surface carries the parse constant (false).
    let ignore_areas = board
        .get(trace_id)
        .and_then(|entry| entry.nets.first().copied())
        .and_then(|net_no| board.rules().nets.get(net_no).map(|net| net.net_class))
        .is_some_and(|class_no| {
            board
                .rules()
                .net_class(class_no)
                .is_some_and(NetClass::ignore_cycles_with_areas)
        });
    for contact in start {
        if is_cycle_recu(
            manager,
            board,
            contact,
            trace_id,
            trace_id,
            ignore_areas,
            &mut visited,
        ) {
            return true;
        }
    }
    false
}

/// Java `Item.isCycleRecu` (`Item.java:690-713`). The `comeFrom`
/// skip PRECEDES the search check — at the first level
/// `comeFrom == searchItem` is the receiver itself, so the seed block
/// cannot report a cycle through its own corner (Y3). Unbounded
/// recursion is Java-parity (the item graph is finite).
fn is_cycle_recu(
    manager: &SearchTreeManager,
    board: &mut Board,
    current: ItemId,
    search_item: ItemId,
    come_from_item: ItemId,
    ignore_areas: bool,
    visited: &mut BTreeSet<ItemId>,
) -> bool {
    if ignore_areas
        && matches!(
            board.get(current).map(|entry| &entry.data),
            Some(ItemData::ConductionArea { .. })
        )
    {
        return false;
    }
    for contact in item_normal_contacts(manager, board, current) {
        if contact == come_from_item {
            continue;
        }
        if contact == search_item {
            return true;
        }
        if visited.insert(contact)
            && is_cycle_recu(
                manager,
                board,
                contact,
                search_item,
                current,
                ignore_areas,
                visited,
            )
        {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// trace tails + removeIfCycle
// ---------------------------------------------------------------------------

/// Java `BasicBoard.getTraceTail` (`BasicBoard.java:1307-1330`): the
/// same-layer overlapping items at a point, filtered to traces of the
/// net; a trace qualifies when one of its ENDPOINTS equals the
/// location AND the matching endpoint's contact set is EMPTY (the two
/// checks are independent `if`s, so a trace with
/// `first == last == location` qualifies through the first one). A
/// mid-corner point is NOT a tail (Y5).
pub fn get_trace_tail(
    manager: &SearchTreeManager,
    board: &mut Board,
    location: &Point,
    layer: i32,
    net_numbers: &[i32],
) -> Option<ItemId> {
    let search_shape =
        TileShape::RegularTileShape(RegularTileShape::IntBox(location.surrounding_box()));
    let overlaps = manager.overlapping_objects(
        board,
        SearchTreeManager::DEFAULT_TREE_INDEX,
        &search_shape,
        layer,
        &[],
    );
    for id in overlaps {
        // `instanceof PolylineTrace` (:1321).
        let candidate = match board.get(id).map(|entry| (&entry.data, &entry.nets)) {
            Some((ItemData::Trace { lines, .. }, nets)) => {
                if !nets_equal(nets, net_numbers) {
                    continue;
                }
                Some((first_corner(lines), last_corner(lines)))
            }
            _ => None,
        };
        let Some((first, last)) = candidate else {
            continue;
        };
        if first.as_ref() == Some(location)
            && contacts::start_contacts(manager, board, id).is_empty()
        {
            return Some(id);
        }
        if last.as_ref() == Some(location) && contacts::end_contacts(manager, board, id).is_empty()
        {
            return Some(id);
        }
    }
    None
}

/// Java `BasicBoard.removeItems` + `BoardItemRepository.removeItem`
/// (`:170-199`): the deletion-forbidden skip comes FIRST (a forbidden
/// item keeps its slot AND its tree entries), then the tree removal,
/// then the list delete.
fn remove_item_through_repository(manager: &mut SearchTreeManager, board: &mut Board, id: ItemId) {
    if is_deletion_forbidden(board, id) {
        return;
    }
    manager.remove(board, id);
    board.remove_item(id);
}

fn remove_items(manager: &mut SearchTreeManager, board: &mut Board, items: &[ItemId]) {
    for id in items {
        remove_item_through_repository(manager, board, *id);
    }
}

/// Java `BasicBoard.removeIfCycle` (`BasicBoard.java:1336-1366`): the
/// on-board and cycle guards, then the connection set of the trace is
/// removed, then — per endpoint that had NO tail BEFORE the removal —
/// the (new) tail's connection set is removed too (Y7). Returns
/// whether a cycle was found and removed.
pub fn remove_if_cycle(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
) -> bool {
    if !board.is_on_the_board(trace_id) {
        return false;
    }
    if !is_cycle(manager, board, trace_id) {
        return false;
    }
    // Capture the geometry facts BEFORE anything is removed
    // (:1346-1351).
    let facts = match board.get(trace_id).map(|entry| (&entry.data, &entry.nets)) {
        Some((ItemData::Trace { layer, lines, .. }, nets)) => Some((
            *layer,
            nets.clone(),
            first_corner(lines),
            last_corner(lines),
        )),
        _ => None,
    };
    let Some((layer, nets, first, last)) = facts else {
        return false;
    };
    let endpoints = [first, last];
    let mut tail_at_endpoint_before = [false; 2];
    for (i, endpoint) in endpoints.iter().enumerate() {
        tail_at_endpoint_before[i] = endpoint
            .as_ref()
            .and_then(|point| get_trace_tail(manager, board, point, layer, &nets))
            .is_some();
    }
    let connection_items = get_connection_items(manager, board, trace_id);
    remove_items(manager, board, &connection_items);
    // The phase-2 tail pass (:1359-1366).
    for (i, endpoint) in endpoints.iter().enumerate() {
        if !tail_at_endpoint_before[i]
            && let Some(point) = endpoint
            && let Some(tail) = get_trace_tail(manager, board, point, layer, &nets)
        {
            let tail_items = get_connection_items(manager, board, tail);
            remove_items(manager, board, &tail_items);
        }
    }
    true
}

// ---------------------------------------------------------------------------
// combine
// ---------------------------------------------------------------------------

/// The trace facts one join half reads, cloned up front so no entry
/// borrow survives the tree queries.
type TraceFacts = (Polyline, i32, i32, Vec<i32>, FixedState);

fn trace_facts(board: &Board, id: ItemId) -> Option<TraceFacts> {
    let entry = board.get(id)?;
    let ItemData::Trace {
        layer,
        half_width,
        lines,
    } = &entry.data
    else {
        return None;
    };
    Some((
        lines.clone(),
        *layer,
        *half_width,
        entry.nets.clone(),
        entry.fixed,
    ))
}

/// Java `PolylineTraceSearchTreeAdapter.replaceGeometry`
/// (`:34-40`): remove → setPolyline → clearDerivedData → insert.
/// The adapter's explicit `clearSearchTreeEntries` is subsumed by
/// [`SearchTreeManager::remove`] (it drops the entry map AND the
/// shape cache). The D18 branch point — see the module docs.
fn replace_geometry(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
    polyline: Polyline,
) {
    manager.remove(board, trace_id);
    board.set_trace_polyline(trace_id, polyline);
    board.clear_derived_data(trace_id);
    manager.insert(board, trace_id);
}

/// Java `PolylineTrace.combineAtStart(ignoreAreas)` (`:201-332`) —
/// module docs for the full contract. Returns whether a join
/// happened.
fn combine_at_start(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
    ignore_areas: bool,
) -> bool {
    let Some((a_lines, a_layer, a_half_width, a_nets, a_fixed)) = trace_facts(board, trace_id)
    else {
        return false;
    };
    let Some(start_corner) = first_corner(&a_lines) else {
        return false;
    };
    let mut contact_list =
        contacts::normal_contacts(manager, board, trace_id, &start_corner, false);
    if ignore_areas {
        contact_list.retain(|id| {
            !matches!(
                board.get(*id).map(|entry| &entry.data),
                Some(ItemData::ConductionArea { .. })
            )
        });
    }
    if contact_list.len() != 1 {
        return false;
    }
    let other_id = contact_list[0];
    let Some((b_lines, b_layer, b_half_width, b_nets, b_fixed)) = trace_facts(board, other_id)
    else {
        return false;
    };
    // The equality gate (:246-252) — note the NEITHER-forbidden
    // clauses.
    if b_layer != a_layer
        || !nets_equal(&b_nets, &a_nets)
        || b_half_width != a_half_width
        || b_fixed != a_fixed
        || is_deletion_forbidden(board, other_id)
        || is_deletion_forbidden(board, trace_id)
    {
        return false;
    }
    // The corner match decides direction (:249-260).
    let other_first = first_corner(&b_lines);
    let other_last = last_corner(&b_lines);
    let reverse_order = if Some(&start_corner) == other_last.as_ref() {
        false
    } else if Some(&start_corner) == other_first.as_ref() {
        true
    } else {
        return false;
    };
    // saveForUndo (:279) — the T14 undo surface, not ported here.
    let mut other_lines = b_lines.lines.clone();
    if reverse_order {
        other_lines.reverse();
        other_lines = other_lines
            .into_iter()
            .map(|line| line.opposite())
            .collect();
    }
    let a_array = a_lines.lines.clone();
    // Java indexes aLines[1] / otherLines[oLen-2] blindly; a 2-line
    // array cannot occur for an on-board trace (insert requires >= 2
    // corners = 3 lines). The guard keeps the port total.
    if a_array.len() < 2 || other_lines.len() < 2 {
        return false;
    }
    // The collinear-skip test (:285-287).
    let skip_line = other_lines[other_lines.len() - 2].is_equal_or_opposite(&a_array[1]);
    let new_count = a_array.len() + other_lines.len() - 2 - usize::from(skip_line);
    // The overlapping-arraycopy assembly (:296-302): other's head,
    // then the receiver's tail overwriting the join slot when skipping.
    let mut new_lines: Vec<_> = other_lines[..other_lines.len() - 1].to_vec();
    if skip_line {
        new_lines.truncate(other_lines.len() - 2);
    }
    new_lines.extend_from_slice(&a_array[1..]);
    debug_assert_eq!(new_lines.len(), new_count);
    let joined = Polyline::new(new_lines);
    let joined_line_count = joined.lines.len();
    replace_geometry(manager, board, trace_id, joined);
    // The shrink rule (:304-307) — a single-segment leftover removes
    // the receiver ITSELF (through the repository, so a
    // deletion-forbidden receiver would survive).
    if joined_line_count < 3 {
        remove_item_through_repository(manager, board, trace_id);
    }
    remove_item_through_repository(manager, board, other_id);
    true
}

/// Java `PolylineTrace.combineAtEnd(ignoreAreas)` (`:341-456`) — the
/// mirror of [`combine_at_start`]: contacts at the LAST corner,
/// `end == other.firstCorner` normal / `== other.lastCorner`
/// reversed, the skip test on the RECEIVER's second-to-last line, and
/// the assembly laying the receiver's head first.
fn combine_at_end(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
    ignore_areas: bool,
) -> bool {
    let Some((a_lines, a_layer, a_half_width, a_nets, a_fixed)) = trace_facts(board, trace_id)
    else {
        return false;
    };
    let Some(end_corner) = last_corner(&a_lines) else {
        return false;
    };
    let mut contact_list = contacts::normal_contacts(manager, board, trace_id, &end_corner, false);
    if ignore_areas {
        contact_list.retain(|id| {
            !matches!(
                board.get(*id).map(|entry| &entry.data),
                Some(ItemData::ConductionArea { .. })
            )
        });
    }
    if contact_list.len() != 1 {
        return false;
    }
    let other_id = contact_list[0];
    let Some((b_lines, b_layer, b_half_width, b_nets, b_fixed)) = trace_facts(board, other_id)
    else {
        return false;
    };
    if b_layer != a_layer
        || !nets_equal(&b_nets, &a_nets)
        || b_half_width != a_half_width
        || b_fixed != a_fixed
        || is_deletion_forbidden(board, other_id)
        || is_deletion_forbidden(board, trace_id)
    {
        return false;
    }
    let other_first = first_corner(&b_lines);
    let other_last = last_corner(&b_lines);
    let reverse_order = if Some(&end_corner) == other_first.as_ref() {
        false
    } else if Some(&end_corner) == other_last.as_ref() {
        true
    } else {
        return false;
    };
    let mut other_lines = b_lines.lines.clone();
    if reverse_order {
        other_lines.reverse();
        other_lines = other_lines
            .into_iter()
            .map(|line| line.opposite())
            .collect();
    }
    let a_array = a_lines.lines.clone();
    if a_array.len() < 2 || other_lines.len() < 2 {
        return false;
    }
    // The mirrored skip test (:394-396): the RECEIVER's
    // second-to-last line against the other's second line.
    let skip_line = a_array[a_array.len() - 2].is_equal_or_opposite(&other_lines[1]);
    let new_count = a_array.len() + other_lines.len() - 2 - usize::from(skip_line);
    let mut new_lines: Vec<_> = a_array[..a_array.len() - 1].to_vec();
    if skip_line {
        new_lines.truncate(a_array.len() - 2);
    }
    new_lines.extend_from_slice(&other_lines[1..]);
    debug_assert_eq!(new_lines.len(), new_count);
    let joined = Polyline::new(new_lines);
    let joined_line_count = joined.lines.len();
    replace_geometry(manager, board, trace_id, joined);
    if joined_line_count < 3 {
        remove_item_through_repository(manager, board, trace_id);
    }
    remove_item_through_repository(manager, board, other_id);
    true
}

/// Java `PolylineTrace.combine()` (`:175-192`): the loop — start
/// first each iteration, on-board rechecked (the shrink rule can
/// remove the receiver mid-loop).
pub fn combine(manager: &mut SearchTreeManager, board: &mut Board, trace_id: ItemId) -> bool {
    let mut something_changed = false;
    while board.is_on_the_board(trace_id)
        && (combine_at_start(manager, board, trace_id, true)
            || combine_at_end(manager, board, trace_id, true))
    {
        something_changed = true;
        // observers.notifyChanged + additionalUpdateAfterChange
        // (:187-189) — GUI/autoroute-side, inert in this seam.
    }
    something_changed
}

// ---------------------------------------------------------------------------
// insertTraceWithoutCleaning
// ---------------------------------------------------------------------------

/// Java `BasicBoard.insertTraceWithoutCleaning`
/// (`BasicBoard.java:179-203`) — the NO-NORMALIZE insertion path
/// (the crafted fixtures here and the contacts seam both mirror it).
///
/// Order is parity-critical: the `< 2 corners` guard fires BEFORE the
/// item construction, so NO id is burned (`:186`); the closed-trace
/// guard fires AFTER (`:192-195`; a closed trace below USER_FIXED is
/// refused — the id IS burned, T61). The tree broadcast is the
/// repository insert's observer callback; the half-width range
/// tracking (`:198-200`) is gated on `netsNormal` and uses the PARAM
/// width, accumulated into `BasicBoard`'s OWN `maxTraceHalfWidth`
/// (`:107`, init 1000) / `minTraceHalfWidth` (`:110`, init 10000)
/// pair — NOT the parse-populated `BoardRules` fields of the same
/// names (capture `G_BB_MAX 1000 / G_BB_MIN 125`: a 250 insert does
/// not raise the 1000-ceiling max; the rules getter reads 125
/// throughout — `G_MAX_TRACE_HW 125`).
/// (The repository's clearance-class range guard `:142-145` is not
/// ported: every caller passes parse-valid classes.)
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
pub fn insert_trace_without_cleaning(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    polyline: Polyline,
    layer: i32,
    half_width: i32,
    nets: &[i32],
    clearance_class: i32,
    fixed: FixedState,
) -> Option<ItemId> {
    // Java's `p_trace_polyline.corner_count() < 2` with INT arithmetic:
    // cornerCount = lines.length - 1, so the guard fires for lengths
    // 0, 1, 2 (-1, 0, 1). The literal usize port of corner_count wraps
    // to usize::MAX for the EMPTY polyline (0 lines), which would NOT
    // fire — `lines.len() < 3` is the exact int-arithmetic equivalent
    // over the whole domain.
    if polyline.lines.len() < 3 {
        return None; // before the ctor: no id burned
    }
    let id = board.alloc_id(); // the Item ctor's generator call
    let first_pt = polyline.first_corner();
    let last_pt = polyline.last_corner();
    if first_pt == last_pt && first_pt.is_some() && fixed < FixedState::UserFixed {
        return None; // closed + not user-fixed: refused, id burned
    }
    board.insert_item(ItemEntry {
        id,
        data: ItemData::Trace {
            layer,
            half_width,
            lines: polyline,
        },
        nets: nets.to_vec(),
        clearance_class,
        component_id: 0,
        fixed,
        on_the_board: false,
    });
    manager.insert(board, id);
    if nets_normal(nets) {
        let range = &mut board.trace_half_width_range;
        range.max_trace_half_width = range.max_trace_half_width.max(half_width);
        range.min_trace_half_width = range.min_trace_half_width.min(half_width);
    }
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::parse_board_from_text;
    use epic_geometry::int_point::IntPoint;

    /// The PURE crafted board — byte-identical to `PURE_DSN` in
    /// `rust/harness/oracle/CombineSpike.java`. Parse items: 1 =
    /// outline, 2 = keepout F.Cu 70000-90000x10000-20000
    /// (SYSTEM_FIXED), 3 = pin at (20000,40000) net OTHER, 4 = trace E
    /// (20000,40000)-(10000,40000) net MINE hw 125 — the DSN wire
    /// width 250 is the FULL width, 5 = via
    /// PAD_C600 at (40000,40000) net MINE, 6 = conduction area
    /// 50000,10000-60000,20000, 7 = F (55000,15000)-(55000,25000),
    /// 8 = F2 (50000,15000)-(50000,25000), 9 = G
    /// (80000,15000)-(80000,30000). Post-parse insertions continue at
    /// id 10.
    const PURE_DSN: &str = "\
(pcb t11-pure.dsn\n\
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

    fn ip(x: i32, y: i32) -> Point {
        Point::Int(IntPoint::new(x, y))
    }

    fn poly(corners: &[(i32, i32)]) -> Polyline {
        let points: Vec<Point> = corners.iter().map(|(x, y)| ip(*x, *y)).collect();
        Polyline::from_points(&points)
    }

    /// `B_IDS tmpl=4 count=9` + the FULL `B_TREE` dump (31 rows,
    /// `/tmp/epic-t11-combine.out` `B_TREE_ROW` section) + the 16
    /// baseline leaves — the precondition every case stands on. The
    /// full-row pin is the guard for the TWO-PATH build-order record:
    /// the spike captures the READ-path tree (Java inserts each item
    /// at creation — ascending id on parsed boards), so `fresh()`
    /// replays [`SearchTreeManager::insert_items_creation_order`].
    /// Java's REBUILD walk (`insertAllBoardItems` over the
    /// `ConcurrentSkipListMap` that backs `itemList`) is DESCENDING id
    /// and yields a DIFFERENT skeleton with the same leaf set — same
    /// counts, all query pins blind to it, which is how one walk was
    /// once mistaken for the other (the T9 index goldens pin the
    /// rebuild path and pass only with the descending walk).
    fn fresh() -> (SearchTreeManager, Board) {
        let mut board = parse_board_from_text(PURE_DSN);
        assert_eq!(board.item_count(), 9, "B_IDS count=9");
        let mut manager = SearchTreeManager::new();
        manager.insert_items_creation_order(&mut board);
        let dump = manager.default_tree().min_area_tree().dump_lines();
        let expected: Vec<&str> = vec![
            "I oct[-100 -100 100100 60100 -60141 100141 -141 160141]",
            "    I oct[-100 -100 100100 100 -141 100141 -141 100141]",
            "        L obj=1 idx=0 oct[-100 -100 100100 100 -141 100141 -141 100141]",
            "        L obj=1 idx=4 oct[-100 -100 100100 100 -141 100141 -141 100141]",
            "    I oct[-100 -100 100100 60100 -60141 100141 -141 160141]",
            "        I oct[70000 -100 100100 60100 39859 100141 80000 160141]",
            "            I oct[70000 -100 100100 60100 39859 100141 80000 160141]",
            "                L obj=1 idx=1 oct[99900 -100 100100 60100 39859 100141 99859 160141]",
            "                I oct[70000 10000 90000 30125 49823 80000 80000 110177]",
            "                    L obj=2 idx=0 oct[70000 10000 90000 20000 50000 80000 80000 110000]",
            "                    L obj=9 idx=0 oct[79875 14875 80125 30125 49823 65177 94823 110177]",
            "            L obj=1 idx=5 oct[99900 -100 100100 60100 39859 100141 99859 160141]",
            "        I oct[-100 -100 100100 60100 -60141 50000 -141 160141]",
            "            I oct[-100 59900 100100 60100 -60141 40141 59859 160141]",
            "                L obj=1 idx=2 oct[-100 59900 100100 60100 -60141 40141 59859 160141]",
            "                L obj=1 idx=6 oct[-100 59900 100100 60100 -60141 40141 59859 160141]",
            "            I oct[-100 -100 60000 60100 -60141 50000 -141 80425]",
            "                I oct[-100 -100 60000 60100 -60141 50000 -141 80425]",
            "                    L obj=1 idx=3 oct[-100 -100 100 60100 -60141 141 -141 60141]",
            "                    I oct[9875 10000 60000 40300 -30177 50000 49823 80425]",
            "                        I oct[19700 10000 60000 40300 -20424 50000 59576 80425]",
            "                            L obj=3 idx=0 oct[19700 39700 20300 40300 -20424 -19575 59576 60425]",
            "                            I oct[39700 10000 60000 40300 -424 50000 60000 80425]",
            "                                L obj=5 idx=0 oct[39700 39700 40300 40300 -424 425 79576 80425]",
            "                                I oct[49875 10000 60000 25125 24823 50000 60000 80177]",
            "                                    I oct[49875 10000 60000 25125 24823 50000 60000 80000]",
            "                                        L obj=6 idx=0 oct[50000 10000 60000 20000 30000 50000 60000 80000]",
            "                                        L obj=8 idx=0 oct[49875 14875 50125 25125 24823 35177 64823 75177]",
            "                                    L obj=7 idx=0 oct[54875 14875 55125 25125 29823 40177 69823 80177]",
            "                        L obj=4 idx=0 oct[9875 39875 20125 40125 -30177 -19823 49823 60177]",
            "                L obj=1 idx=7 oct[-100 -100 100 60100 -60141 141 -141 60141]",
        ];
        assert_eq!(dump.len(), 31, "B_TREE lines=31");
        let mismatches: Vec<String> = dump
            .iter()
            .zip(expected.iter())
            .filter(|(got, want)| got.as_str() != **want)
            .map(|(got, want)| format!("got {got:?}, want {want:?}"))
            .collect();
        assert!(mismatches.is_empty(), "B_TREE byte-compare: {mismatches:?}");
        assert_eq!(
            manager.default_tree().leaf_count(),
            16,
            "B baseline leaves (4 traces + 8 outline idx + keepout + area + pin + via)"
        );
        (manager, board)
    }

    /// Mirrors the spike's `ins(board, t, corners)`: the template is
    /// the parsed trace E (id 4), inserted through the PORTED
    /// [`insert_trace_without_cleaning`] so the generator hands out
    /// the same ids as the capture (10, 11, ...). Layer, HALF WIDTH
    /// (the parse value — the DSN wire width 250 is the FULL width, so
    /// the template carries 125), class and nets are all copied from
    /// the template exactly like the spike's `tmpl.getHalfWidth()`.
    fn insert(
        manager: &mut SearchTreeManager,
        board: &mut Board,
        corners: &[(i32, i32)],
    ) -> ItemId {
        insert_with(manager, board, corners, 0, 0, FixedState::Unfixed)
    }

    /// The spike's insLayer / insWidth / insFixed variants. The `0`
    /// sentinels for `layer` and `half_width` mean "copy the template"
    /// (only `insWidth` overrides the width, at 500 in R1).
    fn insert_with(
        manager: &mut SearchTreeManager,
        board: &mut Board,
        corners: &[(i32, i32)],
        layer: i32,
        half_width: i32,
        fixed: FixedState,
    ) -> ItemId {
        let template = ItemId::new(4);
        let layer = if layer == 0 {
            board.trace_layer(template).expect("parse trace 4")
        } else {
            layer
        };
        let half_width = if half_width == 0 {
            board.trace_half_width(template).expect("parse trace 4")
        } else {
            half_width
        };
        let nets = board.get(template).expect("parse trace 4").nets.clone();
        let clearance_class = board.item_clearance_class(template).expect("parse trace 4");
        insert_trace_without_cleaning(
            manager,
            board,
            poly(corners),
            layer,
            half_width,
            &nets,
            clearance_class,
            fixed,
        )
        .expect("insert succeeded")
    }

    fn raw(contacts: &[ItemId]) -> Vec<u32> {
        contacts.iter().map(|id| id.get()).collect()
    }

    /// The capture's `corners=[x1,y1 x2,y2 ...]` body.
    fn corners_of(board: &Board, id: ItemId) -> Vec<String> {
        board
            .trace_polyline(id)
            .expect("trace alive")
            .corners()
            .iter()
            .map(|corner| match corner {
                Point::Int(p) => format!("{},{}", p.x, p.y),
                Point::Rational(_) => unreachable!("integer-board fixtures"),
            })
            .collect()
    }

    /// The survivor leaf rows (`L obj=<id> ...`) of a post-combine
    /// dump — the geometry-derived D18-safe pin set.
    fn leaf_rows_for(manager: &SearchTreeManager, id: ItemId) -> Vec<String> {
        manager
            .default_tree()
            .min_area_tree()
            .dump_lines()
            .into_iter()
            .filter(|line| line.contains(&format!(" L obj={} ", id.get())))
            .collect()
    }

    /// One `Y1_PROBE` row: (id, [first, last] corners, start, end,
    /// all contact ids).
    type ProbeRow = (
        u32,
        [&'static str; 2],
        &'static [u32],
        &'static [u32],
        &'static [u32],
    );

    /// One `Y1_PAIR` row: (x, y, firstCommonLayer, normalContactPoint
    /// as an (x, y) literal).
    type PairRow = (u32, u32, i32, Option<(i32, i32)>);

    // ------------------------------------------------------------------
    // predicates (unit level)
    // ------------------------------------------------------------------

    /// `containsNet`'s `<= 0` guard vs `sharesNetNo`'s absence;
    /// `netsEqual` through the guard; `isNormalNetNumber` boundaries.
    #[test]
    fn net_predicates_match_java_guards() {
        assert!(contains_net(&[1, 2], 2));
        assert!(!contains_net(&[1], 2));
        assert!(!contains_net(&[0], 0), "net 0 is NOT a contained net");
        assert!(nets_equal(&[1], &[1]));
        assert!(nets_equal(&[1, 2], &[2, 1]), "order-insensitive");
        assert!(!nets_equal(&[1], &[2]));
        assert!(!nets_equal(&[0], &[0]), "through the containsNet guard");
        assert!(!nets_equal(&[1], &[]));
        assert!(nets_equal(&[], &[]));
        assert!(nets_normal(&[1, 9_999_999]));
        assert!(!nets_normal(&[0]));
        assert!(!nets_normal(&[10_000_000]));
        // is_normal_net_number itself lives in rules_surf (tested
        // there); the re-export feeds nets_normal.
        assert!(is_normal_net_number(1));
    }

    /// `Item.isDeletionForbidden` (`:866-875`) per kind on the parse
    /// items: component attachment (pin 3), USER_FIXED-or-stronger
    /// (outline 1 / keepout 2 are SYSTEM_FIXED), plain conduction area
    /// 6 on a signal layer NOT forbidden, plain trace E NOT forbidden.
    /// Capture contrast: `R5_CONTACTS ... bForbidden=true`.
    #[test]
    fn deletion_forbidden_per_kind() {
        let (mut manager, mut board) = fresh();
        assert!(board.get(ItemId::new(3)).expect("pin").component_id > 0);
        assert!(
            is_deletion_forbidden(&board, ItemId::new(3)),
            "pin 3: component"
        );
        assert!(
            is_deletion_forbidden(&board, ItemId::new(1)),
            "outline: SYSTEM_FIXED >= USER_FIXED"
        );
        assert!(
            is_deletion_forbidden(&board, ItemId::new(2)),
            "keepout: SYSTEM_FIXED"
        );
        assert!(
            !is_deletion_forbidden(&board, ItemId::new(6)),
            "area 6: UNFIXED on a signal layer"
        );
        assert!(!is_deletion_forbidden(&board, ItemId::new(4)), "trace E");

        // The R5 trap shape (capture :259-261): A plain (id 10), then
        // B USER_FIXED (id 11) → forbidden → the equality gate's
        // neither-forbidden clause refuses and nothing is removed.
        let a = insert(
            &mut manager,
            &mut board,
            &[(10_000, 20_000), (30_000, 20_000)],
        );
        let b = insert_with(
            &mut manager,
            &mut board,
            &[(30_000, 20_000), (50_000, 20_000)],
            0,
            0,
            FixedState::UserFixed,
        );
        assert_eq!(a.get(), 10, "R5 A=10");
        assert_eq!(b.get(), 11, "R5 B=11");
        assert!(is_deletion_forbidden(&board, b), "R5 bForbidden=true");
        let a = ItemId::new(10);
        assert!(!combine(&mut manager, &mut board, a), "R5_COMBINE false");
        assert!(board.get(a).is_some(), "R5_A_ALIVE true");
        assert!(board.get(b).is_some(), "R5_B_ALIVE true");
    }

    /// The insert guards, pinned by the capture's G rows
    /// (`/tmp/epic-t11-combine.out`, the appended section): a 1-corner
    /// polyline refuses BEFORE construction (no id burned); the
    /// DOUBLED-BACK 3-point input COLLAPSES in Polygon construction
    /// (`B.side_of(A, A)` is collinear → the empty polyline) and is
    /// ALSO refused by the first guard — no burn (the closed-corner
    /// guard is NOT reachable from an [A,B,A] input); a REAL closed
    /// ring survives the cleanup and is refused AFTER the ctor (id
    /// burned — the witness shows the gap); the same ring USER_FIXED
    /// inserts. Absolute ids + the accumulator rows are capture
    /// literals.
    #[test]
    fn insert_trace_guards_burn_like_java() {
        let (mut manager, mut board) = fresh();
        let tmpl_hw = board
            .trace_half_width(ItemId::new(4))
            .expect("parse trace 4");
        assert_eq!(tmpl_hw, 125, "--G-- insert guards tmpl_hw=125");
        // G_ONE_CORNER null — no id burned; G_NEXT_AFTER_ONE_CORNER 10.
        assert_eq!(
            insert_trace_without_cleaning(
                &mut manager,
                &mut board,
                poly(&[(60_000, 20_000)]),
                0,
                tmpl_hw,
                &[1],
                1,
                FixedState::Unfixed
            ),
            None
        );
        let w1 = insert(
            &mut manager,
            &mut board,
            &[(60_000, 20_000), (65_000, 20_000)],
        );
        assert_eq!(w1.get(), 10, "G_NEXT_AFTER_ONE_CORNER 10");
        // G_DOUBLED_BACK null (the polygon collapse — still no burn);
        // G_NEXT_AFTER_DOUBLED_BACK 11.
        assert_eq!(
            insert_trace_without_cleaning(
                &mut manager,
                &mut board,
                poly(&[(70_000, 20_000), (75_000, 20_000), (70_000, 20_000)]),
                0,
                tmpl_hw,
                &[1],
                1,
                FixedState::Unfixed
            ),
            None
        );
        let w2 = insert(
            &mut manager,
            &mut board,
            &[(60_000, 21_000), (65_000, 21_000)],
        );
        assert_eq!(w2.get(), 11, "G_NEXT_AFTER_DOUBLED_BACK 11");
        // G_RING_UNFIXED null with id 12 BURNED; the witness shows the
        // gap (G_NEXT_AFTER_RING_UNFIXED 13) and carries the insWidth
        // param 250.
        assert_eq!(
            insert_trace_without_cleaning(
                &mut manager,
                &mut board,
                poly(&[
                    (60_000, 30_000),
                    (70_000, 30_000),
                    (70_000, 40_000),
                    (60_000, 30_000)
                ]),
                0,
                tmpl_hw,
                &[1],
                1,
                FixedState::Unfixed
            ),
            None
        );
        let w3 = insert_with(
            &mut manager,
            &mut board,
            &[(60_000, 22_000), (65_000, 22_000)],
            0,
            250,
            FixedState::Unfixed,
        );
        assert_eq!(
            w3.get(),
            13,
            "G_NEXT_AFTER_RING_UNFIXED 13 (the burned id shows as a gap)"
        );
        assert_eq!(
            board.trace_half_width(w3),
            Some(250),
            "w3 came through insWidth 250"
        );
        // G_RING_USER_FIXED id=14 hw=125 corners=[60000,30000 70000,30000
        // 70000,40000 60000,30000].
        let closed = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            poly(&[
                (60_000, 30_000),
                (70_000, 30_000),
                (70_000, 40_000),
                (60_000, 30_000),
            ]),
            0,
            tmpl_hw,
            &[1],
            1,
            FixedState::UserFixed,
        );
        assert_eq!(
            closed.map(|id| id.get()),
            Some(14),
            "G_RING_USER_FIXED id=14"
        );
        let closed = closed.expect("G_RING inserted");
        assert_eq!(board.trace_half_width(closed), Some(125), "G_RING hw=125");
        assert_eq!(
            corners_of(&board, closed),
            vec!["60000,30000", "70000,30000", "70000,40000", "60000,30000"],
            "G_RING_USER_FIXED corners"
        );
        // G_BB_MAX 1000 / G_BB_MIN 125 — the BasicBoard accumulator
        // pair (init 1000/10000): the 250 param did not raise the
        // 1000 ceiling, the 125 inserts lowered the floor; the RULES
        // getter never moved (G_MAX_TRACE_HW 125).
        assert_eq!(board.max_trace_half_width(), 1000, "G_BB_MAX 1000");
        assert_eq!(board.min_trace_half_width(), 125, "G_BB_MIN 125");
        assert_eq!(
            board.rules().max_trace_half_width,
            125,
            "G_MAX_TRACE_HW 125"
        );
    }

    // ------------------------------------------------------------------
    // C: the joins
    // ------------------------------------------------------------------

    /// C1 — collinear end-join. Capture: `C1_PRED branch=MERGE_ENTRIES
    /// atStart=false reverse=false skipLine=true newCount=3
    /// joinedLen=3 joinedCorners=2 survivor=[10000,20000 50000,20000]`,
    /// `C1_COMBINE true`, `C1_A_ALIVE true corners=[10000,20000
    /// 50000,20000]`, 17 post leaves, survivor leaf
    /// `oct[9875 19875 50125 20125 -10177 30177 29823 70177]`,
    /// `C1_WITNESS_ID 12 (no id reuse)`. Java took MERGE_ENTRIES; the
    /// port takes replaceGeometry (D18) — leaf set + count pinned, not
    /// the I-node history.
    #[test]
    fn c1_collinear_end_join() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(10_000, 20_000), (30_000, 20_000)],
        );
        let b = insert(
            &mut manager,
            &mut board,
            &[(30_000, 20_000), (50_000, 20_000)],
        );
        assert_eq!([a.get(), b.get()], [10, 11], "C1 A=10 B=11");
        assert!(combine(&mut manager, &mut board, a), "C1_COMBINE true");
        assert_eq!(
            corners_of(&board, a),
            vec!["10000,20000", "50000,20000"],
            "C1_A_ALIVE corners"
        );
        assert!(board.get(b).is_none(), "B removed");
        assert_eq!(
            manager.default_tree().leaf_count(),
            17,
            "C1 post leaves: 16 baseline + A + B - B - A + joined = 17"
        );
        let survivor = leaf_rows_for(&manager, a);
        assert_eq!(survivor.len(), 1);
        assert!(
            survivor[0].ends_with("oct[9875 19875 50125 20125 -10177 30177 29823 70177]"),
            "C1 survivor leaf row: {survivor:?}"
        );
        let w = insert(
            &mut manager,
            &mut board,
            &[(60_000, 20_000), (65_000, 20_000)],
        );
        assert_eq!(w.get(), 12, "C1_WITNESS_ID 12 (no id reuse)");
    }

    /// C2 — L end-join. Capture: `C2_PRED branch=MERGE_ENTRIES ...
    /// skipLine=false newCount=4 joinedLen=4 joinedCorners=3
    /// survivor=[10000,30000 30000,30000 30000,50000]`,
    /// `C2_A_ALIVE true corners=[10000,30000 30000,30000 30000,50000]`,
    /// 18 post leaves, survivor leaves idx=0/idx=1.
    #[test]
    fn c2_l_end_join() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(10_000, 30_000), (30_000, 30_000)],
        );
        let b = insert(
            &mut manager,
            &mut board,
            &[(30_000, 30_000), (30_000, 50_000)],
        );
        assert_eq!([a.get(), b.get()], [10, 11], "C2 A=10 B=11");
        assert!(combine(&mut manager, &mut board, a), "C2_COMBINE true");
        assert_eq!(
            corners_of(&board, a),
            vec!["10000,30000", "30000,30000", "30000,50000"],
            "C2_A_ALIVE corners"
        );
        assert!(board.get(b).is_none());
        assert_eq!(manager.default_tree().leaf_count(), 18, "C2 post leaves");
        let survivor = leaf_rows_for(&manager, a);
        assert_eq!(survivor.len(), 2, "a 3-corner trace carries 2 leaves");
    }

    /// C3 — reverse start-join, then the loop joins the parse trace E
    /// at the new end. Capture: `C3_A_AFTER_INSERT [30000,40000
    /// 10000,40000]`, `C3_B_AFTER_INSERT [30000,40000 50000,40000]`,
    /// `C3_PRED_START ... atStart=true reverse=true skipLine=true`,
    /// `C3_COMBINE true`, `C3_A_ALIVE true corners=[50000,40000
    /// 20000,40000] (TWO joins)`, 16 post leaves, survivor leaf
    /// `oct[19875 39875 50125 40125 -20177 10177 59823 90177]`.
    #[test]
    fn c3_reverse_start_join_then_loop_joins_e() {
        let (mut manager, mut board) = fresh();
        assert_eq!(
            corners_of(&board, ItemId::new(4)),
            vec!["20000,40000", "10000,40000"],
            "C3_E_CORNERS e=4"
        );
        let a = insert(
            &mut manager,
            &mut board,
            &[(30_000, 40_000), (10_000, 40_000)],
        );
        let b = insert(
            &mut manager,
            &mut board,
            &[(30_000, 40_000), (50_000, 40_000)],
        );
        assert_eq!([a.get(), b.get()], [10, 11], "C3 A=10 B=11");
        assert!(combine(&mut manager, &mut board, a), "C3_COMBINE true");
        assert_eq!(
            corners_of(&board, a),
            vec!["50000,40000", "20000,40000"],
            "C3_A_ALIVE corners after TWO joins"
        );
        assert!(board.get(b).is_none(), "B removed");
        assert!(board.get(ItemId::new(4)).is_none(), "E removed by join 2");
        assert_eq!(board.item_count(), 9, "9 parse + 2 - 2");
        assert_eq!(manager.default_tree().leaf_count(), 16, "C3 post leaves");
        let survivor = leaf_rows_for(&manager, a);
        assert_eq!(survivor.len(), 1);
        assert!(
            survivor[0].ends_with("oct[19875 39875 50125 40125 -20177 10177 59823 90177]"),
            "C3 survivor leaf row: {survivor:?}"
        );
    }

    /// C4 — chain of 3. Capture: `C4_PRED_AB branch=MERGE_ENTRIES
    /// skipLine=true`, `C4_POST_ITEM id=10 ... corners=[10000,50000
    /// 70000,50000]` — two loop iterations eat B then C.
    #[test]
    fn c4_chain_of_three() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(10_000, 50_000), (30_000, 50_000)],
        );
        let b = insert(
            &mut manager,
            &mut board,
            &[(30_000, 50_000), (50_000, 50_000)],
        );
        let c = insert(
            &mut manager,
            &mut board,
            &[(50_000, 50_000), (70_000, 50_000)],
        );
        assert_eq!([a.get(), b.get(), c.get()], [10, 11, 12], "C4 ids");
        assert!(combine(&mut manager, &mut board, a));
        assert_eq!(
            corners_of(&board, a),
            vec!["10000,50000", "70000,50000"],
            "C4_POST_ITEM id=10 corners"
        );
        assert!(board.get(b).is_none());
        assert!(board.get(c).is_none());
    }

    /// C5 — both ends. Capture: `C5_PRED_START ... survivor=[10000,20000
    /// 50000,20000]`, `C5_PRED_END ... survivor=[30000,20000
    /// 70000,20000]` (the END prediction ran against A's ORIGINAL
    /// state — a PRED artifact; the real second join starts from the
    /// once-joined trace), `C5_POST_ITEM id=10 ... corners=[10000,20000
    /// 70000,20000]`.
    #[test]
    fn c5_both_ends_start_first() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(30_000, 20_000), (50_000, 20_000)],
        );
        let b1 = insert(
            &mut manager,
            &mut board,
            &[(10_000, 20_000), (30_000, 20_000)],
        );
        let b2 = insert(
            &mut manager,
            &mut board,
            &[(50_000, 20_000), (70_000, 20_000)],
        );
        assert_eq!([a.get(), b1.get(), b2.get()], [10, 11, 12], "C5 ids");
        assert!(combine(&mut manager, &mut board, a));
        assert_eq!(
            corners_of(&board, a),
            vec!["10000,20000", "70000,20000"],
            "C5_POST_ITEM id=10 corners"
        );
        assert!(board.get(b1).is_none());
        assert!(board.get(b2).is_none());
    }

    /// C6 — two-contact refusal. Capture: `C6_END_CONTACTS
    /// [12:T,11:T]` (the exactly-one gate sees TWO candidates),
    /// `C6_COMBINE false`, and items 10/11/12 all alive.
    #[test]
    fn c6_two_contact_refusal() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(20_000, 20_000), (30_000, 20_000)],
        );
        let b1 = insert(
            &mut manager,
            &mut board,
            &[(30_000, 20_000), (40_000, 20_000)],
        );
        let b2 = insert(
            &mut manager,
            &mut board,
            &[(30_000, 20_000), (30_000, 30_000)],
        );
        assert_eq!([a.get(), b1.get(), b2.get()], [10, 11, 12], "C6 ids");
        assert_eq!(
            raw(&contacts::end_contacts(&manager, &mut board, a)),
            vec![12, 11],
            "C6_END_CONTACTS [12:T,11:T]"
        );
        assert!(!combine(&mut manager, &mut board, a), "C6_COMBINE false");
        for id in [a, b1, b2] {
            assert!(board.get(id).is_some(), "C6 POST: {id:?} alive");
        }
    }

    /// C7 — zero-contact refusal. Capture: empty start AND end contact
    /// sets, `C7_COMBINE false`, A alive
    /// `[20000,20000 30000,20000]`.
    #[test]
    fn c7_zero_contact_refusal() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(20_000, 20_000), (30_000, 20_000)],
        );
        assert_eq!(a.get(), 10, "C7 A=10");
        assert_eq!(
            raw(&contacts::start_contacts(&manager, &mut board, a)),
            Vec::<u32>::new(),
            "C7_START_CONTACTS empty"
        );
        assert_eq!(
            raw(&contacts::end_contacts(&manager, &mut board, a)),
            Vec::<u32>::new(),
            "C7_END_CONTACTS empty"
        );
        assert!(!combine(&mut manager, &mut board, a), "C7_COMBINE false");
        assert_eq!(
            corners_of(&board, a),
            vec!["20000,20000", "30000,20000"],
            "C7_A_ALIVE corners"
        );
    }

    // ------------------------------------------------------------------
    // R: the equality gate
    // ------------------------------------------------------------------

    /// R1 — width mismatch refuses (but the CONTACT exists).
    /// Capture: `R1_CONTACTS [11:T]`, `R1_COMBINE false`, both alive.
    #[test]
    fn r1_width_mismatch_refuses() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(10_000, 20_000), (30_000, 20_000)],
        );
        let b = insert_with(
            &mut manager,
            &mut board,
            &[(30_000, 20_000), (50_000, 20_000)],
            0,
            500,
            FixedState::Unfixed,
        );
        assert_eq!([a.get(), b.get()], [10, 11], "R1 A=10 B=11");
        assert_eq!(
            raw(&contacts::end_contacts(&manager, &mut board, a)),
            vec![11],
            "R1_CONTACTS [11:T]"
        );
        assert!(!combine(&mut manager, &mut board, a), "R1_COMBINE false");
        assert!(
            board.get(a).is_some() && board.get(b).is_some(),
            "R1 both alive"
        );
    }

    /// R2 — layer mismatch manifests as ZERO contacts (the tree query
    /// filters the layer before any equality gate). Capture:
    /// `R2_A_END_CONTACTS [] B_START_CONTACTS []`, `R2_COMBINE false
    /// false`, both alive.
    #[test]
    fn r2_layer_mismatch_is_zero_contacts() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(10_000, 20_000), (30_000, 20_000)],
        );
        let b = insert_with(
            &mut manager,
            &mut board,
            &[(30_000, 20_000), (50_000, 20_000)],
            1,
            0,
            FixedState::Unfixed,
        );
        assert_eq!([a.get(), b.get()], [10, 11], "R2 A=10 B=11");
        assert_eq!(
            raw(&contacts::end_contacts(&manager, &mut board, a)),
            Vec::<u32>::new(),
            "R2_A_END_CONTACTS empty"
        );
        assert_eq!(
            raw(&contacts::start_contacts(&manager, &mut board, b)),
            Vec::<u32>::new(),
            "R2_B_START_CONTACTS empty"
        );
        assert!(!combine(&mut manager, &mut board, a), "R2_COMBINE a false");
        assert!(!combine(&mut manager, &mut board, b), "R2_COMBINE b false");
        assert!(
            board.get(a).is_some() && board.get(b).is_some(),
            "R2 both alive"
        );
    }

    /// R3 — fixed mismatch refuses (SHOVE_FIXED != UNFIXED) — but a
    /// contact EXISTS. Capture: `R3_CONTACTS [11:T]`, `R3_COMBINE
    /// false`, both alive.
    #[test]
    fn r3_fixed_mismatch_refuses() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(10_000, 20_000), (30_000, 20_000)],
        );
        let b = insert_with(
            &mut manager,
            &mut board,
            &[(30_000, 20_000), (50_000, 20_000)],
            0,
            0,
            FixedState::ShoveFixed,
        );
        assert_eq!([a.get(), b.get()], [10, 11], "R3 A=10 B=11");
        assert_eq!(
            raw(&contacts::end_contacts(&manager, &mut board, a)),
            vec![11],
            "R3_CONTACTS [11:T]"
        );
        assert!(!combine(&mut manager, &mut board, a), "R3_COMBINE false");
        assert!(
            board.get(a).is_some() && board.get(b).is_some(),
            "R3 both alive"
        );
    }

    /// R4 — SAME fixed state COMBINES (the gate is equality, not
    /// forbiddenness — the R3 contrast witness). Capture:
    /// `R4_PRED branch=MERGE_ENTRIES skipLine=true`, `R4_COMBINE
    /// true`, `R4_A_ALIVE true corners=[10000,20000 50000,20000]
    /// B_ALIVE false`.
    #[test]
    fn r4_same_fixed_combines() {
        let (mut manager, mut board) = fresh();
        let a = insert_with(
            &mut manager,
            &mut board,
            &[(10_000, 20_000), (30_000, 20_000)],
            0,
            0,
            FixedState::ShoveFixed,
        );
        let b = insert_with(
            &mut manager,
            &mut board,
            &[(30_000, 20_000), (50_000, 20_000)],
            0,
            0,
            FixedState::ShoveFixed,
        );
        assert_eq!([a.get(), b.get()], [10, 11], "R4 A=10 B=11");
        assert!(combine(&mut manager, &mut board, a), "R4_COMBINE true");
        assert_eq!(
            corners_of(&board, a),
            vec!["10000,20000", "50000,20000"],
            "R4_A_ALIVE corners"
        );
        assert!(board.get(b).is_none(), "R4_B_ALIVE false");
    }

    /// R6 — the conduction-area strip: F's start contacts the area
    /// (raw size 2), `ignoreAreas=true` strips it, the join with P
    /// proceeds. Capture: `R6_F_START_RAW [10:T,6:A]`,
    /// `R6_F_START contact_size=2`, `R6_PRED_START ... atStart=true
    /// reverse=true skipLine=true survivor=[55000,5000 55000,25000]`,
    /// `R6_COMBINE true`, `R6_F_ALIVE true corners=[55000,5000
    /// 55000,25000]`, `R6_P_ALIVE false`, 16 post leaves, F's new leaf
    /// `oct[54875 4875 55125 25125 29823 50177 59823 80177]`.
    #[test]
    fn r6_area_strip_then_join() {
        let (mut manager, mut board) = fresh();
        let f = ItemId::new(7);
        let p = insert(
            &mut manager,
            &mut board,
            &[(55_000, 15_000), (55_000, 5_000)],
        );
        assert_eq!(p.get(), 10, "R6 P=10");
        let raw_start = contacts::start_contacts(&manager, &mut board, f);
        assert_eq!(raw(&raw_start), vec![10, 6], "R6_F_START_RAW [10:T,6:A]");
        assert_eq!(raw_start.len(), 2, "R6_F_START contact_size=2");
        assert!(combine(&mut manager, &mut board, f), "R6_COMBINE true");
        assert_eq!(
            corners_of(&board, f),
            vec!["55000,5000", "55000,25000"],
            "R6_F_ALIVE corners"
        );
        assert!(board.get(p).is_none(), "R6_P_ALIVE false");
        assert_eq!(manager.default_tree().leaf_count(), 16, "R6 post leaves");
        let survivor = leaf_rows_for(&manager, f);
        assert_eq!(survivor.len(), 1);
        assert!(
            survivor[0].ends_with("oct[54875 4875 55125 25125 29823 50177 59823 80177]"),
            "R6 survivor (F, id 7) leaf row: {survivor:?}"
        );
    }

    // ------------------------------------------------------------------
    // D: degenerate joins
    // ------------------------------------------------------------------

    /// D1 — partial overlap: B doubles back halfway over A; the join
    /// collapses to A's head + B's turnaround. Capture:
    /// `D1_PRED branch=MERGE_ENTRIES skipLine=true`, `D1_COMBINE
    /// true`, `D1_A_ALIVE true corners=[10000,10000 20000,10000]`,
    /// `D1_B_ALIVE false`.
    #[test]
    fn d1_partial_overlap() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(10_000, 10_000), (30_000, 10_000)],
        );
        let b = insert(
            &mut manager,
            &mut board,
            &[(30_000, 10_000), (20_000, 10_000)],
        );
        assert_eq!([a.get(), b.get()], [10, 11], "D1 A=10 B=11");
        assert!(combine(&mut manager, &mut board, a), "D1_COMBINE true");
        assert_eq!(
            corners_of(&board, a),
            vec!["10000,10000", "20000,10000"],
            "D1_A_ALIVE corners"
        );
        assert!(board.get(b).is_none(), "D1_B_ALIVE false");
    }

    /// D2 — the U-turn, the REPLACE_GEOMETRY witness: Java itself took
    /// `branch=REPLACE_GEOMETRY` here (`D2_PRED ... newCount=4
    /// joinedLen=3 joinedCorners=2`), so the port's always-replace
    /// path is tree-history-identical and the FULL post dump is pinned
    /// BYTE-IDENTICAL to the capture (33 rows, `/tmp/epic-t11-combine.out`
    /// `D2_POST_TREE_ROW` section). The STAGED dumps (`D2_AFTER_A` 35
    /// rows, `D2_AFTER_AB` 37 rows) pin the insert placement over the
    /// baseline before the combine runs: A's two leaves enter as
    /// direct siblings, B's insert then pushes A's idx=1 leaf under a
    /// new inner node beside B's own. Also: `D2_COMBINE true`,
    /// `D2_A_ALIVE true corners=[10000,10000 30000,10000]`,
    /// `D2_B_ALIVE false`, 17 post leaves — and the shrink rule does
    /// NOT fire (3 joined lines is not < 3).
    #[test]
    fn d2_u_turn_replace_geometry_dump_is_byte_identical() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(10_000, 10_000), (30_000, 10_000), (30_000, 30_000)],
        );
        let after_a = manager.default_tree().min_area_tree().dump_lines();
        let b = insert(
            &mut manager,
            &mut board,
            &[(30_000, 30_000), (30_000, 10_000)],
        );
        let after_ab = manager.default_tree().min_area_tree().dump_lines();
        assert_eq!([a.get(), b.get()], [10, 11], "D2 A=10 B=11");
        // D2_AFTER_A (35 rows): the 31-row baseline with A's two
        // leaves as direct siblings of the inner node over
        // oct[9875 9875 30125 30125 ...].
        let expected_after_a: Vec<&str> = vec![
            "I oct[-100 -100 100100 60100 -60141 100141 -141 160141]",
            "    I oct[-100 -100 100100 100 -141 100141 -141 100141]",
            "        L obj=1 idx=0 oct[-100 -100 100100 100 -141 100141 -141 100141]",
            "        L obj=1 idx=4 oct[-100 -100 100100 100 -141 100141 -141 100141]",
            "    I oct[-100 -100 100100 60100 -60141 100141 -141 160141]",
            "        I oct[70000 -100 100100 60100 39859 100141 80000 160141]",
            "            I oct[70000 -100 100100 60100 39859 100141 80000 160141]",
            "                L obj=1 idx=1 oct[99900 -100 100100 60100 39859 100141 99859 160141]",
            "                I oct[70000 10000 90000 30125 49823 80000 80000 110177]",
            "                    L obj=2 idx=0 oct[70000 10000 90000 20000 50000 80000 80000 110000]",
            "                    L obj=9 idx=0 oct[79875 14875 80125 30125 49823 65177 94823 110177]",
            "            L obj=1 idx=5 oct[99900 -100 100100 60100 39859 100141 99859 160141]",
            "        I oct[-100 -100 100100 60100 -60141 50000 -141 160141]",
            "            I oct[-100 59900 100100 60100 -60141 40141 59859 160141]",
            "                L obj=1 idx=2 oct[-100 59900 100100 60100 -60141 40141 59859 160141]",
            "                L obj=1 idx=6 oct[-100 59900 100100 60100 -60141 40141 59859 160141]",
            "            I oct[-100 -100 60000 60100 -60141 50000 -141 80425]",
            "                I oct[-100 -100 60000 60100 -60141 50000 -141 80425]",
            "                    L obj=1 idx=3 oct[-100 -100 100 60100 -60141 141 -141 60141]",
            "                    I oct[9875 9875 60000 40300 -30177 50000 19823 80425]",
            "                        I oct[19700 10000 60000 40300 -20424 50000 59576 80425]",
            "                            L obj=3 idx=0 oct[19700 39700 20300 40300 -20424 -19575 59576 60425]",
            "                            I oct[39700 10000 60000 40300 -424 50000 60000 80425]",
            "                                L obj=5 idx=0 oct[39700 39700 40300 40300 -424 425 79576 80425]",
            "                                I oct[49875 10000 60000 25125 24823 50000 60000 80177]",
            "                                    I oct[49875 10000 60000 25125 24823 50000 60000 80000]",
            "                                        L obj=6 idx=0 oct[50000 10000 60000 20000 30000 50000 60000 80000]",
            "                                        L obj=8 idx=0 oct[49875 14875 50125 25125 24823 35177 64823 75177]",
            "                                    L obj=7 idx=0 oct[54875 14875 55125 25125 29823 40177 69823 80177]",
            "                        I oct[9875 9875 30125 40125 -30177 20177 19823 60177]",
            "                            L obj=4 idx=0 oct[9875 39875 20125 40125 -30177 -19823 49823 60177]",
            "                            I oct[9875 9875 30125 30125 -177 20177 19823 60177]",
            "                                L obj=10 idx=0 oct[9875 9875 30125 10125 -177 20177 19823 40177]",
            "                                L obj=10 idx=1 oct[29875 9875 30125 30125 -177 20177 39823 60177]",
            "                L obj=1 idx=7 oct[-100 -100 100 60100 -60141 141 -141 60141]",
        ];
        assert_eq!(after_a.len(), 35, "D2_AFTER_A_TREE lines=35");
        let mismatches: Vec<String> = after_a
            .iter()
            .zip(expected_after_a.iter())
            .filter(|(got, want)| got.as_str() != **want)
            .map(|(got, want)| format!("got {got:?}, want {want:?}"))
            .collect();
        assert!(
            mismatches.is_empty(),
            "D2_AFTER_A dump byte-compare: {mismatches:?}"
        );
        // D2_AFTER_AB (37 rows): B's insert nests A's idx=1 leaf and
        // its own under a new inner node over oct[29875 ...].
        let expected_after_ab: Vec<&str> = vec![
            "I oct[-100 -100 100100 60100 -60141 100141 -141 160141]",
            "    I oct[-100 -100 100100 100 -141 100141 -141 100141]",
            "        L obj=1 idx=0 oct[-100 -100 100100 100 -141 100141 -141 100141]",
            "        L obj=1 idx=4 oct[-100 -100 100100 100 -141 100141 -141 100141]",
            "    I oct[-100 -100 100100 60100 -60141 100141 -141 160141]",
            "        I oct[70000 -100 100100 60100 39859 100141 80000 160141]",
            "            I oct[70000 -100 100100 60100 39859 100141 80000 160141]",
            "                L obj=1 idx=1 oct[99900 -100 100100 60100 39859 100141 99859 160141]",
            "                I oct[70000 10000 90000 30125 49823 80000 80000 110177]",
            "                    L obj=2 idx=0 oct[70000 10000 90000 20000 50000 80000 80000 110000]",
            "                    L obj=9 idx=0 oct[79875 14875 80125 30125 49823 65177 94823 110177]",
            "            L obj=1 idx=5 oct[99900 -100 100100 60100 39859 100141 99859 160141]",
            "        I oct[-100 -100 100100 60100 -60141 50000 -141 160141]",
            "            I oct[-100 59900 100100 60100 -60141 40141 59859 160141]",
            "                L obj=1 idx=2 oct[-100 59900 100100 60100 -60141 40141 59859 160141]",
            "                L obj=1 idx=6 oct[-100 59900 100100 60100 -60141 40141 59859 160141]",
            "            I oct[-100 -100 60000 60100 -60141 50000 -141 80425]",
            "                I oct[-100 -100 60000 60100 -60141 50000 -141 80425]",
            "                    L obj=1 idx=3 oct[-100 -100 100 60100 -60141 141 -141 60141]",
            "                    I oct[9875 9875 60000 40300 -30177 50000 19823 80425]",
            "                        I oct[19700 10000 60000 40300 -20424 50000 59576 80425]",
            "                            L obj=3 idx=0 oct[19700 39700 20300 40300 -20424 -19575 59576 60425]",
            "                            I oct[39700 10000 60000 40300 -424 50000 60000 80425]",
            "                                L obj=5 idx=0 oct[39700 39700 40300 40300 -424 425 79576 80425]",
            "                                I oct[49875 10000 60000 25125 24823 50000 60000 80177]",
            "                                    I oct[49875 10000 60000 25125 24823 50000 60000 80000]",
            "                                        L obj=6 idx=0 oct[50000 10000 60000 20000 30000 50000 60000 80000]",
            "                                        L obj=8 idx=0 oct[49875 14875 50125 25125 24823 35177 64823 75177]",
            "                                    L obj=7 idx=0 oct[54875 14875 55125 25125 29823 40177 69823 80177]",
            "                        I oct[9875 9875 30125 40125 -30177 20177 19823 60177]",
            "                            L obj=4 idx=0 oct[9875 39875 20125 40125 -30177 -19823 49823 60177]",
            "                            I oct[9875 9875 30125 30125 -177 20177 19823 60177]",
            "                                L obj=10 idx=0 oct[9875 9875 30125 10125 -177 20177 19823 40177]",
            "                                I oct[29875 9875 30125 30125 -177 20177 39823 60177]",
            "                                    L obj=10 idx=1 oct[29875 9875 30125 30125 -177 20177 39823 60177]",
            "                                    L obj=11 idx=0 oct[29875 9875 30125 30125 -177 20177 39823 60177]",
            "                L obj=1 idx=7 oct[-100 -100 100 60100 -60141 141 -141 60141]",
        ];
        assert_eq!(after_ab.len(), 37, "D2_AFTER_AB_TREE lines=37");
        let mismatches: Vec<String> = after_ab
            .iter()
            .zip(expected_after_ab.iter())
            .filter(|(got, want)| got.as_str() != **want)
            .map(|(got, want)| format!("got {got:?}, want {want:?}"))
            .collect();
        assert!(
            mismatches.is_empty(),
            "D2_AFTER_AB dump byte-compare: {mismatches:?}"
        );
        assert!(combine(&mut manager, &mut board, a), "D2_COMBINE true");
        assert_eq!(
            corners_of(&board, a),
            vec!["10000,10000", "30000,10000"],
            "D2_A_ALIVE corners (2 corners / 3 lines — alive)"
        );
        assert!(board.get(b).is_none(), "D2_B_ALIVE false");
        assert_eq!(manager.default_tree().leaf_count(), 17, "D2 post leaves");
        // The byte-identical dump (capture :341-374, prefix stripped).
        let dump = manager.default_tree().min_area_tree().dump_lines();
        let expected: Vec<&str> = vec![
            "I oct[-100 -100 100100 60100 -60141 100141 -141 160141]",
            "    I oct[-100 -100 100100 100 -141 100141 -141 100141]",
            "        L obj=1 idx=0 oct[-100 -100 100100 100 -141 100141 -141 100141]",
            "        L obj=1 idx=4 oct[-100 -100 100100 100 -141 100141 -141 100141]",
            "    I oct[-100 -100 100100 60100 -60141 100141 -141 160141]",
            "        I oct[70000 -100 100100 60100 39859 100141 80000 160141]",
            "            I oct[70000 -100 100100 60100 39859 100141 80000 160141]",
            "                L obj=1 idx=1 oct[99900 -100 100100 60100 39859 100141 99859 160141]",
            "                I oct[70000 10000 90000 30125 49823 80000 80000 110177]",
            "                    L obj=2 idx=0 oct[70000 10000 90000 20000 50000 80000 80000 110000]",
            "                    L obj=9 idx=0 oct[79875 14875 80125 30125 49823 65177 94823 110177]",
            "            L obj=1 idx=5 oct[99900 -100 100100 60100 39859 100141 99859 160141]",
            "        I oct[-100 -100 100100 60100 -60141 50000 -141 160141]",
            "            I oct[-100 59900 100100 60100 -60141 40141 59859 160141]",
            "                L obj=1 idx=2 oct[-100 59900 100100 60100 -60141 40141 59859 160141]",
            "                L obj=1 idx=6 oct[-100 59900 100100 60100 -60141 40141 59859 160141]",
            "            I oct[-100 -100 60000 60100 -60141 50000 -141 80425]",
            "                I oct[-100 -100 60000 60100 -60141 50000 -141 80425]",
            "                    L obj=1 idx=3 oct[-100 -100 100 60100 -60141 141 -141 60141]",
            "                    I oct[9875 9875 60000 40300 -30177 50000 19823 80425]",
            "                        I oct[19700 10000 60000 40300 -20424 50000 59576 80425]",
            "                            L obj=3 idx=0 oct[19700 39700 20300 40300 -20424 -19575 59576 60425]",
            "                            I oct[39700 10000 60000 40300 -424 50000 60000 80425]",
            "                                L obj=5 idx=0 oct[39700 39700 40300 40300 -424 425 79576 80425]",
            "                                I oct[49875 10000 60000 25125 24823 50000 60000 80177]",
            "                                    I oct[49875 10000 60000 25125 24823 50000 60000 80000]",
            "                                        L obj=6 idx=0 oct[50000 10000 60000 20000 30000 50000 60000 80000]",
            "                                        L obj=8 idx=0 oct[49875 14875 50125 25125 24823 35177 64823 75177]",
            "                                    L obj=7 idx=0 oct[54875 14875 55125 25125 29823 40177 69823 80177]",
            "                        I oct[9875 9875 30125 40125 -30177 20177 19823 60177]",
            "                            L obj=4 idx=0 oct[9875 39875 20125 40125 -30177 -19823 49823 60177]",
            "                            L obj=10 idx=0 oct[9875 9875 30125 10125 -177 20177 19823 40177]",
            "                L obj=1 idx=7 oct[-100 -100 100 60100 -60141 141 -141 60141]",
        ];
        assert_eq!(dump.len(), 33, "D2_POST_TREE lines=33");
        let mismatches: Vec<String> = dump
            .iter()
            .zip(expected.iter())
            .filter(|(got, want)| got.as_str() != **want)
            .map(|(got, want)| format!("got {got:?}, want {want:?}"))
            .collect();
        assert!(
            mismatches.is_empty(),
            "D2 dump byte-compare: {mismatches:?}"
        );
    }

    /// D3 — the closed ring: the START half wins (B's last corner sits
    /// on A's first corner), closing the ring through the once-joined
    /// trace. Capture: `D3_PRED ... atStart=false` is the PRED artifact
    /// (branchPred ran with atStart=false); the ACTUAL join was
    /// atStart-normal reverse=false skipLine=false, and the final
    /// corners pin that: `D3_A_ALIVE true corners=[10000,30000
    /// 10000,10000 30000,10000 30000,30000 10000,30000]`,
    /// `D3_B_ALIVE false`.
    #[test]
    fn d3_closed_ring_through_the_start_half() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[
                (10_000, 10_000),
                (30_000, 10_000),
                (30_000, 30_000),
                (10_000, 30_000),
            ],
        );
        let b = insert(
            &mut manager,
            &mut board,
            &[(10_000, 30_000), (10_000, 10_000)],
        );
        assert_eq!([a.get(), b.get()], [10, 11], "D3 A=10 B=11");
        assert!(combine(&mut manager, &mut board, a), "D3_COMBINE true");
        assert_eq!(
            corners_of(&board, a),
            vec![
                "10000,30000",
                "10000,10000",
                "30000,10000",
                "30000,30000",
                "10000,30000"
            ],
            "D3_A_ALIVE corners — the ring opens and closes at the join corner"
        );
        assert!(board.get(b).is_none(), "D3_B_ALIVE false");
    }

    // ------------------------------------------------------------------
    // Y: the connection walk + cycles
    // ------------------------------------------------------------------

    /// Y1 — the pure square (`/tmp/epic-t11-combine-hang.out`, every
    /// row literal). The walk probes: stored corners, per-endpoint and
    /// union contact sets, the via's drill contacts, ALL TWELVE
    /// pairwise (firstCommonLayer, normalContactPoint) inputs, then
    /// the walk's result from three different start items. The walk
    /// TERMINATES (the recon hang prediction was wrong) — through
    /// fork detection, since no visited set exists.
    #[test]
    fn y1_pure_square_walk_rows() {
        let (mut manager, mut board) = fresh();
        let t1 = insert(
            &mut manager,
            &mut board,
            &[(20_000, 20_000), (40_000, 20_000)],
        );
        let t2 = insert(
            &mut manager,
            &mut board,
            &[(40_000, 20_000), (40_000, 40_000)],
        );
        let t3 = insert(
            &mut manager,
            &mut board,
            &[(40_000, 40_000), (20_000, 40_000)],
        );
        let t4 = insert(
            &mut manager,
            &mut board,
            &[(20_000, 40_000), (20_000, 20_000)],
        );
        assert_eq!(
            [t1.get(), t2.get(), t3.get(), t4.get()],
            [10, 11, 12, 13],
            "Y1 T1..T4 = 10..13"
        );

        // Y1_PROBE rows: corners, routable, start/end/all contacts.
        let probes: &[ProbeRow] = &[
            (10, ["20000,20000", "40000,20000"], &[13], &[11], &[13, 11]),
            (
                11,
                ["40000,20000", "40000,40000"],
                &[10],
                &[12, 5],
                &[12, 10, 5],
            ),
            (
                12,
                ["40000,40000", "20000,40000"],
                &[11, 5],
                &[13, 4],
                &[13, 11, 5, 4],
            ),
            (
                13,
                ["20000,40000", "20000,20000"],
                &[12, 4],
                &[10],
                &[12, 10, 4],
            ),
        ];
        for &(id, corners, start, end, all) in probes {
            let id = ItemId::new(id);
            assert_eq!(
                corners_of(&board, id),
                corners.to_vec(),
                "Y1_PROBE id={}",
                id.get()
            );
            assert!(
                is_routable(&board, id),
                "Y1_PROBE routable=true id={}",
                id.get()
            );
            assert_eq!(
                raw(&contacts::start_contacts(&manager, &mut board, id)),
                start.to_vec(),
                "Y1_PROBE start id={}",
                id.get()
            );
            assert_eq!(
                raw(&contacts::end_contacts(&manager, &mut board, id)),
                end.to_vec(),
                "Y1_PROBE end id={}",
                id.get()
            );
            assert_eq!(
                raw(&item_normal_contacts(&manager, &mut board, id)),
                all.to_vec(),
                "Y1_PROBE all id={}",
                id.get()
            );
        }
        // Y1_VIA id=5 all=[12:T,11:T].
        assert_eq!(
            raw(&item_normal_contacts(&manager, &mut board, ItemId::new(5))),
            vec![12, 11],
            "Y1_VIA id=5 all"
        );

        // All twelve Y1_PAIR rows: (x, y) -> (fcl, ncp).
        let mut pair = |x: ItemId, y: ItemId| {
            (
                first_common_layer(&mut board, x, y),
                normal_contact_point(&mut board, x, y),
            )
        };
        let pairs: &[PairRow] = &[
            (10, 11, 0, Some((40_000, 20_000))),
            (10, 12, 0, None),
            (10, 13, 0, Some((20_000, 20_000))),
            (11, 10, 0, Some((40_000, 20_000))),
            (11, 12, 0, Some((40_000, 40_000))),
            (11, 13, 0, None),
            (12, 10, 0, None),
            (12, 11, 0, Some((40_000, 40_000))),
            (12, 13, 0, Some((20_000, 40_000))),
            (13, 10, 0, Some((20_000, 20_000))),
            (13, 11, 0, None),
            (13, 12, 0, Some((20_000, 40_000))),
        ];
        for &(x, y, fcl, ncp) in pairs {
            assert_eq!(
                pair(ItemId::new(x), ItemId::new(y)),
                (fcl, ncp.map(|(px, py)| ip(px, py))),
                "Y1_PAIR x={x} y={y} fcl={fcl} ncp={ncp:?}"
            );
        }

        // The walk results.
        assert!(is_cycle(&manager, &mut board, t1), "Y1_ISCYCLE t1=true");
        assert_eq!(
            raw(&get_connection_items(&manager, &mut board, t1)),
            vec![13, 11, 10],
            "Y1_CONN_ITEMS_RETURNED [13:T,11:T,10:T]"
        );
        assert_eq!(
            raw(&get_connection_items(&manager, &mut board, ItemId::new(12))),
            vec![12],
            "Y1_CONN_12 [12:T]"
        );
        assert_eq!(
            raw(&get_connection_items(&manager, &mut board, ItemId::new(13))),
            vec![13, 11, 10],
            "Y1_CONN_13 [13:T,11:T,10:T]"
        );

        // The derived drill↔trace contact point (via 5 center
        // (40000,40000) — the B_ITEM center row — sits on trace 11's
        // last corner per the Y1_PROBE id=11 row): the
        // DrillItem.normalContactPoint(Trace) body returns the center.
        assert_eq!(
            normal_contact_point(&mut board, ItemId::new(5), ItemId::new(11)),
            Some(ip(40_000, 40_000)),
            "drill center on trace 11's endpoint"
        );
    }

    /// Y2 — square + pendant, plus the parse trace E at the NW corner
    /// (the fork source). Capture: `Y2_ISCYCLE t1=true`,
    /// `Y2_TAIL_BEFORE e0=null e1=null`, `Y2_CONN_ITEMS [11:T,10:T]`,
    /// `Y2_REMOVE_IF_CYCLE true`, POST items 12/13/14 alive (10 and 11
    /// gone), `Y2_WITNESS_ID 15 (ids 10..14 burned)`.
    #[test]
    fn y2_square_plus_pendant_removal() {
        let (mut manager, mut board) = fresh();
        let t1 = insert(
            &mut manager,
            &mut board,
            &[(20_000, 20_000), (40_000, 20_000)],
        );
        let t2 = insert(
            &mut manager,
            &mut board,
            &[(40_000, 20_000), (40_000, 40_000)],
        );
        let t3 = insert(
            &mut manager,
            &mut board,
            &[(40_000, 40_000), (20_000, 40_000)],
        );
        let t4 = insert(
            &mut manager,
            &mut board,
            &[(20_000, 40_000), (20_000, 20_000)],
        );
        let t5 = insert(
            &mut manager,
            &mut board,
            &[(20_000, 20_000), (20_000, 10_000)],
        );
        assert_eq!(
            [t1.get(), t2.get(), t3.get(), t4.get(), t5.get()],
            [10, 11, 12, 13, 14],
            "Y2 T1..T5 = 10..14"
        );
        assert!(is_cycle(&manager, &mut board, t1), "Y2_ISCYCLE t1=true");
        assert_eq!(
            get_trace_tail(&manager, &mut board, &ip(20_000, 20_000), 0, &[1]),
            None,
            "Y2_TAIL_BEFORE e0=null"
        );
        assert_eq!(
            get_trace_tail(&manager, &mut board, &ip(40_000, 20_000), 0, &[1]),
            None,
            "Y2_TAIL_BEFORE e1=null"
        );
        assert_eq!(
            raw(&get_connection_items(&manager, &mut board, t1)),
            vec![11, 10],
            "Y2_CONN_ITEMS [11:T,10:T] (the walk forks at 12 via the NW trio)"
        );
        assert!(
            remove_if_cycle(&mut manager, &mut board, t1),
            "Y2_REMOVE_IF_CYCLE true"
        );
        assert!(board.get(t1).is_none(), "10 removed");
        assert!(board.get(t2).is_none(), "11 removed");
        for id in [t3, t4, t5] {
            assert!(board.get(id).is_some(), "Y2 POST: {:?} alive", id.get());
        }
        let w = insert(
            &mut manager,
            &mut board,
            &[(60_000, 20_000), (65_000, 20_000)],
        );
        assert_eq!(w.get(), 15, "Y2_WITNESS_ID 15 (ids 10..14 burned)");
    }

    /// Y3 — the via triangle: both start contacts of T1 are SEEDED, so
    /// the DFS is blocked by its own corner (comeFrom precedes
    /// searchItem). Capture: `Y3_T1_START [11:T,5:V] END []`,
    /// `Y3_ISCYCLE t1=false`, `Y3_REMOVE_IF_CYCLE false`, all alive.
    #[test]
    fn y3_via_triangle_is_not_a_cycle_from_t1() {
        let (mut manager, mut board) = fresh();
        let t1 = insert(
            &mut manager,
            &mut board,
            &[(40_000, 40_000), (50_000, 40_000)],
        );
        let t2 = insert(
            &mut manager,
            &mut board,
            &[(60_000, 40_000), (40_000, 40_000)],
        );
        assert_eq!([t1.get(), t2.get()], [10, 11], "Y3 T1=10 T2=11");
        assert_eq!(
            raw(&contacts::start_contacts(&manager, &mut board, t1)),
            vec![11, 5],
            "Y3_T1_START [11:T,5:V]"
        );
        assert_eq!(
            raw(&contacts::end_contacts(&manager, &mut board, t1)),
            Vec::<u32>::new(),
            "Y3_T1_END empty"
        );
        assert!(!is_cycle(&manager, &mut board, t1), "Y3_ISCYCLE t1=false");
        assert!(
            !remove_if_cycle(&mut manager, &mut board, t1),
            "Y3_REMOVE_IF_CYCLE false"
        );
        assert!(
            board.get(t1).is_some() && board.get(t2).is_some(),
            "Y3 all alive"
        );
    }

    /// Y5 — the getTraceTail acceptance rows. Capture:
    /// `Y5_TAIL start=10 end=10 mid=null foreignNet=null` — endpoints
    /// with an empty matching contact set are tails; a MID corner is
    /// not; a foreign net never matches.
    #[test]
    fn y5_trace_tail_acceptance_rows() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(10_000, 10_000), (20_000, 10_000)],
        );
        assert_eq!(a.get(), 10, "Y5 A=10");
        let layer = board.trace_layer(a).expect("layer");
        assert_eq!(
            get_trace_tail(&manager, &mut board, &ip(10_000, 10_000), layer, &[1]),
            Some(a),
            "Y5_TAIL start=10"
        );
        assert_eq!(
            get_trace_tail(&manager, &mut board, &ip(20_000, 10_000), layer, &[1]),
            Some(a),
            "Y5_TAIL end=10"
        );
        assert_eq!(
            get_trace_tail(&manager, &mut board, &ip(15_000, 10_000), layer, &[1]),
            None,
            "Y5_TAIL mid=null"
        );
        assert_eq!(
            get_trace_tail(&manager, &mut board, &ip(10_000, 10_000), layer, &[2]),
            None,
            "Y5_TAIL foreignNet=null"
        );
    }

    /// Y6 — a plain L pair is not a cycle. Capture:
    /// `Y6_ISCYCLE a=false`, `Y6_REMOVE_IF_CYCLE false`, both alive.
    #[test]
    fn y6_non_cycle_refusal() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(20_000, 20_000), (30_000, 20_000)],
        );
        let b = insert(
            &mut manager,
            &mut board,
            &[(30_000, 20_000), (30_000, 30_000)],
        );
        assert_eq!([a.get(), b.get()], [10, 11], "Y6 A=10 B=11");
        assert!(!is_cycle(&manager, &mut board, a), "Y6_ISCYCLE a=false");
        assert!(
            !remove_if_cycle(&mut manager, &mut board, a),
            "Y6_REMOVE_IF_CYCLE false"
        );
        assert!(
            board.get(a).is_some() && board.get(b).is_some(),
            "Y6 both alive"
        );
    }

    /// Y7 — the overlap cycle: A and B share BOTH endpoints (B traces
    /// back over A), the isOverlap shortcut fires, and the
    /// Trace↔Trace contact point of the pair is NULL (both corners
    /// touch), so the connection set is the receiver alone. Phase 2
    /// then eats B (no tail existed at either endpoint before).
    /// Capture: `Y7_A_START [11:T] END [11:T]`, `Y7_ISCYCLE a=true`,
    /// `Y7_CONN_ITEMS [10:T]`, `Y7_REMOVE_IF_CYCLE true`, POST dump
    /// shows BOTH 10 and 11 gone, `Y7_WITNESS_ID 12`.
    #[test]
    fn y7_overlap_cycle_phase2_eats_the_partner() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(10_000, 10_000), (30_000, 10_000)],
        );
        let b = insert(
            &mut manager,
            &mut board,
            &[(30_000, 10_000), (10_000, 10_000)],
        );
        assert_eq!([a.get(), b.get()], [10, 11], "Y7 A=10 B=11");
        assert_eq!(
            raw(&contacts::start_contacts(&manager, &mut board, a)),
            vec![11],
            "Y7_A_START [11:T]"
        );
        assert_eq!(
            raw(&contacts::end_contacts(&manager, &mut board, a)),
            vec![11],
            "Y7_A_END [11:T]"
        );
        assert!(is_cycle(&manager, &mut board, a), "Y7_ISCYCLE a=true");
        assert_eq!(
            raw(&get_connection_items(&manager, &mut board, a)),
            vec![10],
            "Y7_CONN_ITEMS [10:T] (the pair's contact point is NULL)"
        );
        assert!(
            remove_if_cycle(&mut manager, &mut board, a),
            "Y7_REMOVE_IF_CYCLE true"
        );
        assert!(board.get(a).is_none(), "Y7 POST: 10 gone");
        assert!(board.get(b).is_none(), "Y7 POST: 11 gone (phase 2)");
        assert_eq!(board.item_count(), 9, "9 parse + 2 - 2");
        let w = insert(
            &mut manager,
            &mut board,
            &[(60_000, 20_000), (65_000, 20_000)],
        );
        assert_eq!(w.get(), 12, "Y7_WITNESS_ID 12");
    }
}
