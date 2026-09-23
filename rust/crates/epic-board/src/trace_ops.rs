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
//! # The SPLIT family and normalize (M2 Task 12)
//!
//! The same module also ports Java `PolylineTrace.split(IntOctagon)`
//! (`:465-691`), `split(Point)` (`:699-712`), the private
//! `split(int, Line)` seam (`:719-760` with
//! `splitInsideDrillPadProhibited` `:768-792`), and
//! `PolylineTraceNormalization.normalize` (depth cap 16). Jar-pinned
//! by `rust/harness/oracle/SplitSpike.java` (capture
//! `/tmp/epic-t12-split.out`, run twice and diffed — deterministic);
//! pins quote LITERAL capture rows, and absolute ids replay the
//! oracle's full insertion sequence per case.
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
//! # D18 CLOSED: the merge fast paths (M4-T6)
//!
//! Java branches on `hasDefaultEntries()`: with default tree entries
//! it calls `mergeEntriesInFront/AtEnd` (an in-place entry-list
//! splice, capture `branch=MERGE_ENTRIES`), otherwise
//! `replaceGeometry`. The port ORIGINALLY always took the
//! [`replace_geometry`] collapse and documented the I-node topology
//! difference as benign (leaf sets and counts matched everywhere in
//! the T11/T12 captures). M4-T6 disproved the benign reading: the
//! search-tree skeleton is HISTORY-DEPENDENT (the T5 bounds
//! tightness invariant makes the STRUCTURE the observable), and the
//! different REM/INS interleave inside the autorouter's insert-tail
//! churn steered a later insert descent differently — the t7_ripup
//! events divergence's second root cause (first divergence:
//! completion interval s20→s21, java 40 vs rust 48 tree ops).
//! [`SearchTreeManager::merge_entries_in_front`],
//! [`SearchTreeManager::merge_entries_at_end`],
//! [`SearchTreeManager::change_entries`], and the faithful
//! [`SearchTreeManager::reuse_entries_after_cutout`] now reproduce
//! Java's per-tree op sequence exactly (surviving leaves keep their
//! tree position and are only re-labeled; the two/three replaced
//! entries are removed; the link entries inserted in index order).
//! The branch conditions are Java's (`joinedPolyline.lines.length
//! != newLineCount || !hasDefaultEntries`), so cases where JAVA
//! itself took `branch=REPLACE_GEOMETRY` (capture `D2_PRED
//! branch=REPLACE_GEOMETRY newCount=4 joinedLen=3`) still collapse
//! here and stay pinned byte-identical on the full tree dump.
//!
//! # The shrink rule and the loop's on-board recheck
//!
//! After replacing the geometry, Java removes the receiver when
//! `this.lines.lines.length < 3` (`:324-327` / `:448-451`) and then
//! ALWAYS removes the joined trace — through the REPOSITORY
//! (`BoardItemRepository.removeItem`, `:170-199`), whose
//! deletion-forbidden skip (`:189`) comes first, so a forbidden item
//! silently stays. The `< 3` count is NOT a "single-segment
//! leftover" from ordinary joins: a constructed `Polyline` carries
//! corners+1 stored lines, so even two single-segment traces joined
//! end-to-end give 3 lines and ONE merged survivor (capture
//! `DEGEN_PRED ... joinedLen=3`, `DEGEN_A_ALIVE true corners=
//! [60000,30000 85000,30000]`). The rule fires when the ctor
//! DEGENERATED the join — a join path that spikes out and back (the
//! other trace retracing the receiver EXACTLY) cleans to the EMPTY
//! polyline: capture `DEGEN2_PRED_START ... newCount=5 joinedLen=0
//! joinedCorners=NONE survivor=EMPTY`, `DEGEN2_COMBINE true`,
//! `DEGEN2_A_ALIVE false B_ALIVE false`, tree back to the 31-row
//! baseline, `DEGEN2_NEXT_ID 12` (both ids burned). An OVERRUN (a
//! collinear extension past the retrace) keeps 3 lines and merges
//! instead: `DEGEN3_A_ALIVE true corners=[10000,10000 0,10000]`.
//! The `isOnTheBoard()` recheck in the combine loop exists for
//! exactly the self-removal case.
//!
//! # The connection walk (NONE-only form)
//!
//! `getConnectionItems` (`Item.java:746-825`) seeds with the item's
//! normal contacts, requires a `normalContactPoint` against the
//! receiver, applies the EXACTLY-ONE-CONTACT contract at the receiver
//! corner (trace receivers only:
//! `getNormalContacts(prevPoint, false).size() != 1` skips the
//! branch), then walks: each step demands `isRoutable` — a KIND
//! gate: the base `Item.isRoutable()` is FALSE (`Item.java:908-910`),
//! only `Trace`/`Via` override it, so a PIN is never routable and the
//! walk both EXCLUDES it from the result set and STOPS there, while a
//! via IS walked into (capture `PINWALK_ROUTABLE pin=false via=true
//! trace=true`; `PINWALK_FROM_TRACE_A [10:T]`,
//! `PINWALK_FROM_PIN [10:T]`, `PINWALK_FROM_VIA [11:T,5:V]`) — stops
//! at vias whose `StopConnectionOption` is VIA/FANOUT_VIA (the
//! port's surface only produces NONE, which never stops — the check
//! is unreachable and absent; `isFanoutVia` is M3), and continues to
//! the unique next contact whose (layer, point) differs from the
//! arrival pair. A NULL
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
//! # The tailAtEndpointBefore flag is structurally always false
//!
//! Java queries `getTraceTail(endpoint)` BEFORE the removal and
//! gates the phase-2 pass on `!tailAtEndpointBefore[i]`
//! (`:1352-1358`). That flag can never be true on a reachable board,
//! so the gate is always taken (Y2 pins the before-queries null on
//! both endpoints; Y7 pins the pass firing; Y5 pins the tail
//! predicate itself on non-cycle endpoints). The argument, in two
//! steps:
//!
//! 1. `isCycle() == true` implies BOTH endpoint contact sets of the
//!    cycle trace are non-empty. The isOverlap shortcut needs the
//!    start and end contact sets to INTERSECT (both non-empty). The
//!    DFS closes only through the OPPOSITE endpoint: the seed
//!    endpoint's contacts are all pre-seeded into the shared visited
//!    set (and recurse with `comeFrom == the receiver`, so their
//!    back-edge to the receiver is comeFrom-skipped before the
//!    search-item check — Y3), so the search item can only be
//!    re-discovered as a contact of a walked item at the other
//!    endpoint's corner.
//! 2. `getTraceTail(location)` returns a trace only when the matching
//!    endpoint contact set is EMPTY. Any same-net, same-layer trace
//!    whose endpoint sits at a cycle trace's endpoint contacts the
//!    cycle trace there (corner-to-corner, layer-exact, net-shared) —
//!    its set is non-empty. And the cycle trace's own endpoint sets
//!    are non-empty by step 1.
//!
//! The tempting self-tail construction (a free end on the cycle
//! trace, cycle closing through the other end) is impossible for the
//! same reason: a free end cannot close a cycle (step 1).
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

use epic_geometry::int_octagon::IntOctagon;
use epic_geometry::line::Line;
use epic_geometry::line_segment::LineSegment;
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

/// Java `board.model.structure.StopConnectionOption` — the walk-stop
/// selector of [`get_connection_items`]. Ordinals match Java's enum
/// (NONE, VIA, FANOUT_VIA).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopConnectionOption {
    /// Java `NONE` — never stop.
    None,
    /// Java `VIA` — stop at any via.
    Via,
    /// Java `FANOUT_VIA` — stop at fanout vias only.
    FanoutVia,
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

/// Java `Item.isShoveFixed()` base (`Item.java:881-883`) with the
/// `Trace` override (`Trace.java:238-253`): the base is
/// `fixedState >= SHOVE_FIXED`; the Trace override ADDS "some NORMAL
/// net number of the trace whose net class is shove-fixed". Via, Pin
/// and the area kinds use the BASE only (no override).
///
/// A missing id reads as not-shove-fixed (Java would NPE; unreachable
/// through the seam).
#[must_use]
pub fn is_shove_fixed(board: &Board, id: ItemId) -> bool {
    let Some(entry) = board.get(id) else {
        return false;
    };
    if entry.fixed >= FixedState::ShoveFixed {
        return true;
    }
    if matches!(entry.data, ItemData::Trace { .. }) {
        for &net_no in &entry.nets {
            if is_normal_net_number(net_no)
                && let Some(net) = board.rules().nets.get(net_no)
                && let Some(net_class) = board.rules().net_class(net.net_class)
                && net_class.shove_fixed
            {
                return true;
            }
        }
    }
    false
}

/// Java `Item.isRoutable()` — the BASE class returns FALSE
/// (`Item.java:908-910`); only `Trace` (`Trace.java:206-209`) and
/// `Via` (`Via.java:147-150`) override it with
/// `!isUserFixed() && netCount() > 0`. A PIN is therefore NEVER
/// routable, whatever its nets: the connection walk EXCLUDES it from
/// the result set and STOPS when it is the next hop, while a via IS
/// routable and is walked into (capture `PINWALK_ROUTABLE
/// pin=false via=true trace=true`, `PINWALK_FROM_TRACE_A [10:T]`,
/// `PINWALK_FROM_PIN [10:T]`, `PINWALK_FROM_VIA [11:T,5:V]`).
/// The T12 item-queue gate (`BatchAutorouter.getAutorouteItems` /
/// `Item.isRoutable`) — public with its first cross-crate consumer.
pub fn is_routable(board: &Board, id: ItemId) -> bool {
    match board.get(id) {
        Some(entry) => {
            matches!(entry.data, ItemData::Trace { .. } | ItemData::Via { .. })
                && !is_user_fixed(entry)
                && !entry.nets.is_empty()
        }
        None => false,
    }
}

/// Java `Item.firstCommonLayer(Item)` (`Item.java:324-332`):
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
/// * Trace ↔ Trace (`Trace.java:131-152`): EXACT layer equality
///   first; then `contactAtFirstCorner` / `contactAtLastCorner` —
///   exactly one of the two must hold (both = null, the doubled-back
///   Y7 case; neither = null, the mid-corner Y1 case); the answer is
///   the receiver's first or last corner respectively.
/// * Drill ↔ Trace (`DrillItem.java:340-349`): sharesLayer and the
///   drill center ON a trace endpoint → the center.
/// * Drill ↔ Drill (`DrillItem.java:332-337`): sharesLayer and equal
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

/// Java `Item.getConnectionItems` (`Item.java:737-822`) — the fork-
/// terminated connected walk (module docs). Descending ids (the
/// `TreeSet` under `Item.compareTo`). The walk collects traces and
/// vias from `item_id` until the next fork or terminal item; `stop`
/// mirrors Java's `StopConnectionOption` — `Via` stops the walk at ANY
/// via, `FanoutVia` only at fanout vias (the `isFanoutVia` read is a
/// fanout-seam carry, banked M3; the arm is present but does not
/// stop), `None` never stops.
pub fn get_connection_items(
    manager: &SearchTreeManager,
    board: &mut Board,
    item_id: ItemId,
    stop: StopConnectionOption,
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
        // The arrival point against the RECEIVER (:753-757).
        let Some(mut prev_contact_point) = normal_contact_point(board, item_id, current) else {
            continue;
        };
        let mut prev_contact_layer = first_common_layer(board, item_id, current);
        // The exactly-one-contact contract, TRACE receivers only
        // (:758-766).
        if receiver_is_trace
            && contacts::normal_contacts(manager, board, item_id, &prev_contact_point, false).len()
                != 1
        {
            continue;
        }
        // The walk (:770-822): forks and dead ends terminate.
        loop {
            if !is_routable(board, current) {
                break;
            }
            // The Via stop options (:775-783).
            if matches!(
                board.get(current).map(|entry| &entry.data),
                Some(ItemData::Via { .. })
            ) {
                match stop {
                    StopConnectionOption::Via => break,
                    // Java `currentItem.isFanoutVia(result)` — the
                    // fanout-via predicate is fanout-seam surface
                    // (banked M3); until then the arm does not stop.
                    StopConnectionOption::FanoutVia => {}
                    StopConnectionOption::None => {}
                }
            }
            result.insert(Reverse(current));
            let mut next: Option<(ItemId, Point, i32)> = None;
            let mut fork_found = false;
            for tmp_contact in item_normal_contacts(manager, board, current) {
                let tmp_layer = first_common_layer(board, current, tmp_contact);
                // Java's `tmpLayer < 0 → continue` guard (:796-797) is
                // structurally unreachable ON THE WALK — every walked
                // pair came out of a layer-matched contact set, so the
                // two layer intervals overlap — but it is kept as a
                // literal port of the shape; the -1 branch of
                // [`first_common_layer`] itself is pinned instead by
                // the capture's R2_FCL_AB row (the R2 test).
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
pub fn remove_item_through_repository(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    id: ItemId,
) {
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
///
/// `tail_at_endpoint_before` is structurally always false (module
/// docs: both endpoint contact sets are non-empty whenever
/// `isCycle()` held, so [`get_trace_tail`] can never hit at either
/// endpoint) — the flag is kept because it is a literal port of the
/// Java shape, and its inputs are pinned by Y2 (`Y2_TAIL_BEFORE
/// e0=null e1=null`).
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
    let connection_items =
        get_connection_items(manager, board, trace_id, StopConnectionOption::None);
    remove_items(manager, board, &connection_items);
    // The phase-2 tail pass (:1359-1366).
    for (i, endpoint) in endpoints.iter().enumerate() {
        if !tail_at_endpoint_before[i]
            && let Some(point) = endpoint
            && let Some(tail) = get_trace_tail(manager, board, point, layer, &nets)
        {
            let tail_items = get_connection_items(manager, board, tail, StopConnectionOption::None);
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
/// shape cache). The SLOW arm of the D18 branch (module docs): Java
/// reaches this when the ctor collapsed the join (`lines.length !=
/// newLineCount`) or a trace lacks default-tree entries; the fast
/// arms [`SearchTreeManager::merge_entries_in_front`],
/// [`SearchTreeManager::merge_entries_at_end`] and
/// [`SearchTreeManager::change_entries`] cover the rest.
pub(crate) fn replace_geometry(
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

/// Java `PolylineTrace.change(Polyline)` (`PolylineTrace.java:937-1002`)
/// — the pull-tight geometry-replacement tail. Structure:
///
/// * `:940-944` off-board defensive arm (unreachable from the ported
///   surface — every caller gates on-board): `setPolyline` only, via
///   [`Board::set_trace_polyline`]. The fold of Java's `saveForUndo`
///   into that setter is behavior-identical (idempotent per level).
/// * `:946` `additionalUpdateAfterChange` — EMPTY in `BasicBoard`
///   (`:1228`), a GUI-subclass hook; nothing to port.
/// * `:948` `itemList.saveForUndo` — folded into the collapse below.
/// * `:950-975` the two diff scans with the early-outs ("both
///   polylines are equal, no change necessary"). Java compares
///   REFERENCES (`newPolyline.lines[i] != lines.lines[i]`); the port
///   compares values with `Line::fast_equals` (the documented proxy:
///   Java's reference-diff on value-equal lines drives a value-neutral
///   tree replace, so skipping it leaves the same end state).
/// * `:977-985` `keepAtStartCount`/`keepAtEndCount` feed
///   `changeEntries` — ported as
///   [`SearchTreeManager::change_entries`] (the kept head/tail
///   leaves never leave the tree); the scans remain for their
///   early-outs.
/// * `:990-1000` the live clip read
///   (`changedArea?.getArea(getLayer())`) and `normalize(clipShape)`
///   under try/catch → `FRLogger.error`. The port's
///   [`normalize`] returns a bool and cannot throw, so the catch is
///   unrepresentable; the change stands either way (the result is
///   consumed exactly as Java's swallowed exception is).
pub(crate) fn change_trace_geometry(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
    new_polyline: Polyline,
) {
    // Java `:940-944`: the defensive arm — plain field write.
    if !board.is_on_the_board(trace_id) {
        board.set_trace_polyline(trace_id, new_polyline);
        return;
    }
    let old_lines = board.trace_polyline(trace_id).cloned().unwrap_or_else(|| {
        // Java `lines` would be null on a foreign id — unreachable
        // (the receiver guards); the empty polyline keeps the scans
        // total.
        Polyline::new(Vec::new())
    });
    let new_len = new_polyline.lines.len() as i32;
    let old_len = old_lines.lines.len() as i32;
    // Java `:950-961`: the first-diff scan over the shared prefix.
    let mut index_of_first_different_line = new_len.min(old_len);
    for i in 0..index_of_first_different_line {
        if !new_polyline.lines[i as usize].fast_equals(&old_lines.lines[i as usize]) {
            index_of_first_different_line = i;
            break;
        }
    }
    if index_of_first_different_line == new_len.min(old_len) {
        // both polylines are equal, no change necessary
        return;
    }
    // Java `:963-975`: the last-diff scan from the ENDS of both
    // arrays (i <= lastIndex keeps both indexes in range even for
    // different lengths).
    let mut index_of_last_different_line = -1;
    for i in 1..=new_len.min(old_len) {
        if !new_polyline.lines[(new_len - i) as usize]
            .fast_equals(&old_lines.lines[(old_len - i) as usize])
        {
            index_of_last_different_line = new_len - i;
            break;
        }
    }
    if index_of_last_different_line < 0 {
        return;
    }
    // Java `:977-985`: `keepAtStartCount = max(firstDiff - 2, 0)`,
    // `keepAtEndCount = max(newLen - lastDiff - 3, 0)` feed
    // `changeEntries` — the kept-head/kept-tail leaf-reuse fast path
    // (the replaced middle leaves are removed, the fresh middle
    // inserted; the kept leaves never leave the tree). The polyline
    // swap happens AFTER the entry surgery, exactly as in Java.
    let keep_at_start_count = (index_of_first_different_line - 2).max(0);
    let keep_at_end_count = (new_len - index_of_last_different_line - 3).max(0);
    manager.change_entries(
        board,
        trace_id,
        &new_polyline,
        keep_at_start_count,
        keep_at_end_count,
    );
    board.set_trace_polyline(trace_id, new_polyline);
    // Java `:991-1000`: the clip is read LIVE (the sweep's joins so
    // far on this layer), then the normalization runs and may remove
    // or split the trace — exactly as in Java.
    let layer = board.trace_layer(trace_id).unwrap_or(0);
    let clip = board.changed_area.as_ref().map(|area| area.get_area(layer));
    let _ = normalize(manager, board, trace_id, clip.as_ref());
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
    // The D18 branch (:304-323): the fast `mergeEntriesInFront` arm
    // runs when the ctor did NOT collapse lines AND both traces carry
    // default-tree entries; the collapse or a missing entry array
    // falls back to the full remove/reinsert. The fast arm re-labels
    // the surviving leaves IN PLACE (survivor tree positions are
    // part of the observable tree skeleton), clears the joined-from
    // trace's arrays, and only then swaps the polyline in.
    if joined_line_count != new_count || !manager.has_default_entries(trace_id, other_id) {
        replace_geometry(manager, board, trace_id, joined);
    } else {
        let to_no = other_lines.len() - usize::from(skip_line);
        manager.merge_entries_in_front(
            board,
            other_id,
            trace_id,
            &joined,
            (other_lines.len() - 3) as i32,
            to_no as i32,
        );
        manager.clear_search_tree_entries(board, other_id);
        board.set_trace_polyline(trace_id, joined);
    }
    // The shrink rule (:324-327) — a degenerated (empty) joined
    // polyline removes the receiver ITSELF (through the repository, so
    // a deletion-forbidden receiver would survive).
    if joined_line_count < 3 {
        remove_item_through_repository(manager, board, trace_id);
    }
    remove_item_through_repository(manager, board, other_id);
    // Java `:328-330`: the join is marked AFTER the removals, guarded
    // by `board instanceof RoutingBoard` — the session-machinery test
    // (joinChangedArea itself null-guards).
    if let Some(area) = board.changed_area.as_mut() {
        area.join_point(&start_corner.to_float(), a_layer);
    }
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
    // The mirrored skip test (:415): the RECEIVER's
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
    // The D18 branch (:428-447): the mirrored fast `mergeEntriesAtEnd`
    // arm — gate identical to the start half's. The receiver is the
    // merge's TO trace; the link window runs over the RECEIVER's line
    // indices (`thisLines.length - 3` .. `toNo`).
    if joined_line_count != new_count || !manager.has_default_entries(trace_id, other_id) {
        replace_geometry(manager, board, trace_id, joined);
    } else {
        let to_no = a_array.len() - usize::from(skip_line);
        manager.merge_entries_at_end(
            board,
            other_id,
            trace_id,
            &joined,
            (a_array.len() - 3) as i32,
            to_no as i32,
        );
        manager.clear_search_tree_entries(board, other_id);
        board.set_trace_polyline(trace_id, joined);
    }
    if joined_line_count < 3 {
        remove_item_through_repository(manager, board, trace_id);
    }
    remove_item_through_repository(manager, board, other_id);
    // Java `:452-454`: the end-corner join, the mirror of the
    // combineAtStart one.
    if let Some(area) = board.changed_area.as_mut() {
        area.join_point(&end_corner.to_float(), a_layer);
    }
    true
}

/// Java `PolylineTrace.combine()` (`:175-192`): the loop — start
/// first each iteration, on-board rechecked (the shrink rule can
/// remove the receiver mid-loop). The START-first order is
/// observable: D3 is the HALF-ORDER witness (mutation check —
/// flipping the loop to end-first rotates the ring's corner list to
/// `[10000,10000 30000,10000 30000,30000 10000,30000 10000,10000]`
/// and fails `d3_closed_ring_through_the_start_half`); the C3/C5
/// final pins are order-INsensitive (opposite-end joins commute on
/// those shapes), so D3 is the only test that catches a swapped
/// loop.
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

// ---------------------------------------------------------------------------
// split (PolylineTrace.java:465-792) — the found/own seam
// ---------------------------------------------------------------------------

/// Java `PolylineTrace.split(IntOctagon)`
/// (`PolylineTrace.java:465-691`): looks up traces intersecting this
/// trace and splits them at the intersection points; on an overlap
/// the traces are split at their first and last common point. Found
/// cycles are removed. If nothing is split the result contains just
/// `trace_id` — even when the trace was removed by a DRILL split and
/// is off the board (`result.add(this)`, the DRL1 capture row
/// `DRL1_RESULT size=1 [10:...]` + `DRL1_A_ON_BOARD false`).
///
/// # The loop shape (all traps jar-pinned by SplitSpike)
///
/// Per segment `i` (the loop bound is `i < lines.len() - 2`, INT
/// arithmetic — segments of a trace with n stored lines are
/// `0..n-3`): the clip bbox filter first (`:475-480`, an
/// [`IntOctagon::intersects_box`] against the segment bounding box),
/// then the receiver's tree shape for segment i — Java
/// `getTreeShape` (`Item.java:213-226`) delegates to
/// `getPrecalculatedTreeShapes` (`Item.java:228-239`), which
/// RECALCULATES via `calculateTreeShapes` when the info is absent —
/// removal neither nulls the board field nor permanently drops the
/// shapes, so a REMOVED receiver still queries tree entries. The
/// empty result for such a receiver comes from the loop-top
/// `!isOnTheBoard()` guard (`:488-491`), NOT from a null-shape
/// query; the `result.add(this)` tail (`:682-684`) is reachable only
/// when the removal happens at the walk's LAST entry — DRL1
/// (last-entry, `[dead this]` size=1) vs DRL2 (first-entry, size=0)
/// pin both.
///
/// The ENTRY walk is a Java `Iterator` over a snapshot collection
/// that is RE-FILLED in place after the first successful found-split
/// (`:583-590` — "reread the overlapping tree entries and reset the
/// iterator, because the board has changed"); the port walks an
/// index over a re-queried [`Vec`]. The on-board check sits at the
/// TOP of every entry iteration, BEFORE fetching the next entry
/// (`:488-491`) — the DRL2 early-empty path (drill split as the
/// FIRST entry: the removal happens mid-iteration, the next
/// iteration returns the still-empty result).
///
/// Per entry: own-entry skips first (`:496-515` — the
/// `[i-1, i+1]` neighbour window, then the zero-length corner
/// equality, `i < foundIdx ? corner(i+1)==corner(foundIdx) :
/// corner(foundIdx+1)==corner(i)`), the raw `sharesNet` gate (no
/// `>0` guard — [`crate::contacts::shares_net_no`]), then the kind
/// dispatch: `PolylineTrace` (found split with the receiver-asymmetric
/// [`LineSegment::intersection`] — the FOUND segment is the RECEIVER
/// of the query, so the returned line is the CURRENT segment's
/// middle; the own split flips the receivers), `DrillItem` — the
/// Java `instanceof DrillItem` catches Pin AND Via (`:654-660`): a
/// perpendicular split through the drill center is executed with the
/// result DISCARDED and `ownTraceSplit` stays false (DRL1: the
/// deleted receiver itself ends up in the result; DRL2: early empty
/// return) — and `ConductionArea` with the `!isUserFixed()` +
/// ignore-cycles + BOTH-endpoint-contact guards (`:661-676`, AR1).
/// The cycle removal after any split is TWO-PASS over `splitPieces`
/// then `result` (`:614-650`) — "remove cycles in the own split
/// pieces last to preserve them, if possible".
///
/// `board.additionalUpdateAfterChange` for results larger than one
/// (`:685-689`) is observer-surface (D12) and not ported.
pub fn split_clip(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
    clip: Option<&IntOctagon>,
) -> Vec<ItemId> {
    let mut result: Vec<ItemId> = Vec::new();
    let (polyline, layer, _half_width, nets, _fixed) = match trace_facts(board, trace_id) {
        Some(facts) => facts,
        // A REMOVED receiver is IN the contract: the own-split
        // recursion (`:606-609`) calls `.split(clipShape)` on pieces
        // the found-trace split has just deleted (X4: piece 12 dies in
        // A's found-trace split before its own recursion runs). Java
        // answers through the loop-top guard (`:488-491`,
        // `!isOnTheBoard()` -> `return result`) — an EMPTY collection;
        // the `result.add(this)` tail is never reached. The arena
        // lookup losing the item is exactly that state (X4L same).
        None => return Vec::new(),
    };
    if !nets_normal(&nets) {
        // only normal nets are split
        result.push(trace_id);
        return result;
    }
    let mut own_trace_split = false;
    // Java: `i < this.lines.lines.length - 2` in INT arithmetic; the
    // receiver's lines are stable through the loop (an own split
    // breaks out, a found split never touches them).
    for i in 0..polyline.lines.len().saturating_sub(2) {
        if let Some(clip_shape) = clip {
            let current_segment = segment_at(&polyline, i, trace_id, "split clip segment");
            if !clip_shape.intersects_box(&current_segment.bounding_box()) {
                continue;
            }
        }
        // Java `getTreeShape(defaultTree, i)` RECALCULATES the shapes
        // when the searchTreesInfo is absent (`Item.java:228-239`) and
        // a removed receiver's object survives removal, so Java still
        // runs the entry query for a dead receiver — the walk then
        // dies at the loop-top `!isOnTheBoard()` guard (`:488-491`)
        // before any entry is processed. The Rust arena drops removed
        // items, so `tree_shape_precalc` has nothing to compute from;
        // the explicit on-board gate short-circuits to the same
        // observable empty return. (A null-shape query would return
        // nothing — `ShapeSearchTree.java:392-394` — but that branch
        // is not how Java exits here.)
        let current_shape = if board.is_on_the_board(trace_id) {
            let object_id = manager.default_tree().object_id();
            let tree = manager.default_tree();
            board
                .tree_shape_precalc(
                    trace_id,
                    object_id,
                    tree.variant,
                    tree.compensated_clearance_class,
                )
                .get(i)
                .cloned()
                .flatten()
        } else {
            None
        };
        let Some(current_shape) = current_shape else {
            continue;
        };
        let current_segment = segment_at(&polyline, i, trace_id, "split current segment");
        // look for intersecting traces with the i-th line segment
        let mut entries = manager.overlapping_tree_entries(
            board,
            SearchTreeManager::DEFAULT_TREE_INDEX,
            &current_shape,
            layer,
            &[],
        );
        let mut entry_index = 0usize;
        while entry_index < entries.len() {
            if !board.is_on_the_board(trace_id) {
                // this trace has been deleted in a cleanup operation
                return result;
            }
            let found_entry = entries[entry_index];
            entry_index += 1;
            let Some(found_id) = SearchTreeManager::item_of_key(found_entry.object_key) else {
                continue;
            };
            let found_idx = found_entry.shape_index_in_object as i32;
            let found_nets = match board.get(found_id) {
                Some(item) => item.nets.clone(),
                None => continue,
            };
            if found_id == trace_id {
                if found_idx >= i as i32 - 1 && found_idx <= i as i32 + 1 {
                    // don't split own trace at this line or at neighbour lines
                    continue;
                }
                // try to handle intermediate segments of length 0 by
                // comparing end corners
                if (i as i32) < found_idx {
                    if polyline.corner(i as i32 + 1) == polyline.corner(found_idx) {
                        continue;
                    }
                } else if polyline.corner(found_idx + 1) == polyline.corner(i as i32) {
                    continue;
                }
            }
            if !contacts::shares_net_no(&found_nets, &nets) {
                continue;
            }
            let found_is_trace = matches!(
                board.get(found_id).map(|entry| &entry.data),
                Some(ItemData::Trace { .. })
            );
            if found_is_trace {
                let found_lines = match board.trace_polyline(found_id) {
                    Some(lines) => lines.clone(),
                    None => continue,
                };
                let found_index = (found_entry.shape_index_in_object as usize)
                    .min(found_lines.lines.len().saturating_sub(2));
                let found_segment =
                    segment_at(&found_lines, found_index, found_id, "split found segment");
                let mut intersecting_lines = found_segment.intersection(&current_segment);
                let mut split_pieces: Vec<ItemId> = Vec::new();
                // try splitting the found trace first
                let mut found_trace_split = false;
                if found_id != trace_id {
                    for line in &intersecting_lines {
                        if let Some(pieces) =
                            split_at_line(manager, board, found_id, found_index as i32 + 1, line)
                        {
                            for piece in pieces.into_iter().flatten() {
                                found_trace_split = true;
                                split_pieces.push(piece);
                            }
                            if found_trace_split {
                                // reread the overlapping tree entries and reset
                                // the iterator, because the board has changed.
                                // Java passes the SAME accumulated LinkedList to
                                // the query — the tree walk only APPENDS
                                // (`ShapeSearchTree.java:437-438`, no clear) —
                                // and rewinds to the head (`:586-588`):
                                // already-processed entries re-walk in their old
                                // order (stale entries refuse through the
                                // off-board guard of split(int, Line)), the
                                // split pieces' entries land at the TAIL. A
                                // replace-style fresh query would inline them in
                                // fresh tree order (REQ2 pins the difference).
                                let fresh = manager.overlapping_tree_entries(
                                    board,
                                    SearchTreeManager::DEFAULT_TREE_INDEX,
                                    &current_shape,
                                    layer,
                                    &[],
                                );
                                entries.extend(fresh);
                                entry_index = 0;
                                break;
                            }
                        }
                    }
                    if !found_trace_split {
                        split_pieces.push(found_id);
                    }
                }
                // now try splitting the own trace — the RECEIVER flips
                intersecting_lines = current_segment.intersection(&found_segment);
                for line in &intersecting_lines {
                    if let Some(pieces) =
                        split_at_line(manager, board, trace_id, i as i32 + 1, line)
                    {
                        own_trace_split = true;
                        // this trace was split itself into 2.
                        if let Some(piece) = pieces[0] {
                            result.extend(split_clip(manager, board, piece, clip));
                        }
                        if let Some(piece) = pieces[1] {
                            result.extend(split_clip(manager, board, piece, clip));
                        }
                        break;
                    }
                }
                if found_trace_split || own_trace_split {
                    // something was split,
                    // remove cycles containing a split piece
                    // (Java pass 0 iterates splitPieces, pass 1 reassigns
                    // the iterator over `result` — the two collections
                    // are disjoint per pass).
                    for piece in &split_pieces {
                        remove_if_cycle(manager, board, *piece);
                    }
                    // remove cycles in the own split pieces last
                    // to preserve them, if possible
                    for piece in &result {
                        remove_if_cycle(manager, board, *piece);
                    }
                }
                if own_trace_split {
                    break;
                }
            } else if matches!(
                board.get(found_id).map(|entry| &entry.data),
                Some(ItemData::Pin { .. }) | Some(ItemData::Via { .. })
            ) {
                // Java `instanceof DrillItem`: Pin AND Via.
                let split_point = drill_center(board, found_id);
                if let Some(split_point) = split_point
                    && current_segment.contains(&split_point)
                {
                    let split_line = Line::new_with_direction(
                        split_point,
                        current_segment.get_line().direction().turn_45_degree(2),
                    );
                    // the split result is DISCARDED — ownTraceSplit
                    // stays false (DRL1/DRL2).
                    split_at_line(manager, board, trace_id, i as i32 + 1, &split_line);
                }
            } else if !board.get(trace_id).is_some_and(is_user_fixed)
                && matches!(
                    board.get(found_id).map(|entry| &entry.data),
                    Some(ItemData::ConductionArea { .. })
                )
            {
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
                let start = contacts::start_contacts(manager, board, trace_id);
                let end = contacts::end_contacts(manager, board, trace_id);
                if !ignore_areas && start.contains(&found_id) && end.contains(&found_id) {
                    // this trace can be removed because of cycle with
                    // conduction area
                    remove_item_through_repository(manager, board, trace_id);
                    return result;
                }
            }
        }
        if own_trace_split {
            break;
        }
    }
    if !own_trace_split {
        result.push(trace_id);
    }
    // board.additionalUpdateAfterChange(currentItem) for every result
    // item when result.len() > 1 (:685-689) — observer surface, D12.
    result
}

/// The `LineSegment(lines, i + 1)` construction — the middle line is
/// the i-th SEGMENT line. `context` only sharpens the unreachable
/// out-of-range case (Java would throw).
fn segment_at(lines: &Polyline, i: usize, id: ItemId, context: &str) -> LineSegment {
    let end = i + 2;
    assert!(
        end < lines.lines.len(),
        "segment {i} out of range for trace {} ({context})",
        id.get()
    );
    LineSegment::new(
        lines.lines[i].clone(),
        lines.lines[i + 1].clone(),
        lines.lines[end].clone(),
    )
}

/// The center of a drill item (Java `DrillItem.getCenter()`): a Via
/// stores its center; a Pin derives it from the component placement
/// ([`Board::pin_center`]).
fn drill_center(board: &Board, id: ItemId) -> Option<Point> {
    match board.get(id).map(|entry| &entry.data) {
        Some(ItemData::Via { center, .. }) => Some(Point::Int(*center)),
        Some(ItemData::Pin { .. }) => board.pin_center(id),
        _ => None,
    }
}

/// Java `Trace.split(Point)` (`:699-712`): splits this trace into two
/// at `point` — a PERPENDICULAR split line (`direction.turn45Degree(2)`)
/// through the first segment containing the point. `None` where Java
/// returns null (the point lies on no segment, or every candidate
/// split was refused).
pub fn split_at_point(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
    point: &Point,
) -> Option<[Option<ItemId>; 2]> {
    let polyline = board.trace_polyline(trace_id)?.clone();
    for i in 0..polyline.lines.len().saturating_sub(2) {
        let current_segment = segment_at(&polyline, i, trace_id, "split at point");
        if current_segment.contains(point) {
            let split_line = Line::new_with_direction(
                point.clone(),
                current_segment.get_line().direction().turn_45_degree(2),
            );
            if let Some(result) = split_at_line(manager, board, trace_id, i as i32 + 1, &split_line)
            {
                return Some(result);
            }
        }
    }
    None
}

/// Java `PolylineTrace.split(int lineIndex, Line newEndLine)`
/// (`:719-760`) — the private mutation seam. Guard ladder: off-board
/// -> null; deletion-forbidden -> null (splitting would leave the
/// original on the board AND insert duplicate pieces, so
/// normalizeTraces would never converge); the polyline split -> null;
/// the pad prohibition -> null. THEN remove + re-insert both pieces
/// preserving layer, half width, nets, clearance class and fixed
/// state. A piece can legitimately be `None` with the OTHER piece
/// inserted (insertTraceWithoutCleaning refuses a closed piece below
/// USER_FIXED and still burns the id — the PAD4 capture row
/// `SPLIT_POINT [11:[...] null]`).
fn split_at_line(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
    line_index: i32,
    new_end_line: &Line,
) -> Option<[Option<ItemId>; 2]> {
    if !board.is_on_the_board(trace_id) {
        return None;
    }
    if is_deletion_forbidden(board, trace_id) {
        return None;
    }
    let (polyline, layer, half_width, nets, fixed) = trace_facts(board, trace_id)?;
    let split_polylines = polyline.split(line_index, new_end_line)?;
    // Java also warns-and-returns on `splitPolylines.length != 2`;
    // the Rust [`Polyline::split`] returns exactly two or `None`.
    if split_inside_drill_pad_prohibited(manager, board, trace_id, line_index, new_end_line) {
        return None;
    }
    // Java reads `clearanceClassIndex()` on the trace OBJECT AFTER the
    // removal (`:741`, reads at `:749`/`:757`) — but removal only
    // detaches the object,
    // its field survives. The arena lookup would return None here, so
    // the value is captured BEFORE the removal instead (same value).
    let clearance_class = board.item_clearance_class(trace_id).unwrap_or(0);
    remove_item_through_repository(manager, board, trace_id);
    let piece0 = insert_trace_without_cleaning(
        manager,
        board,
        split_polylines[0].clone(),
        layer,
        half_width,
        &nets,
        clearance_class,
        fixed,
    );
    let piece1 = insert_trace_without_cleaning(
        manager,
        board,
        split_polylines[1].clone(),
        layer,
        half_width,
        &nets,
        clearance_class,
        fixed,
    );
    Some([piece0, piece1])
}

/// Java `splitInsideDrillPadProhibited(int, Line)` (`:768-792`): is
/// the intersection of the `line_index`-th line with `line` inside
/// the pad of a PIN? The split is allowed only at the pin CENTER or
/// at a same-net trace ENDPOINT. Returns true (prohibited) when a
/// same-net pin's pad was found without either allowance.
///
/// Bugs kept deliberately (jar-pinned PAD1-PAD6):
/// * the kind test is `instanceof Pin` ONLY — a Via at the split
///   point is IGNORED despite the Java doc comment saying "drill
///   item";
/// * the trace-endpoint precedence quirk: Java
///   `currentTrace != this && first.equals(isect) || last.equals(isect)`
///   parses as `(currentTrace != this && first.equals(isect)) ||
///   last.equals(isect)` — the RECEIVER's OWN last corner at the
///   intersection ALLOWS a split inside a foreign pin's pad (the
///   PAD4 row `SPLIT_POINT [11:[...] null]` only exists through this
///   quirk).
/// * the net gate is the raw [`contacts::shares_net_no`] — a pin on a
///   foreign net is skipped entirely (PAD1: the split PROCEEDS inside
///   the foreign pad).
fn split_inside_drill_pad_prohibited(
    manager: &SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
    line_index: i32,
    line: &Line,
) -> bool {
    let Some(polyline) = board.trace_polyline(trace_id) else {
        return false;
    };
    let polyline = polyline.clone();
    let Some(index) = usize::try_from(line_index)
        .ok()
        .filter(|&idx| idx < polyline.lines.len())
    else {
        // Java would throw AIOOBE; unreachable through the callers.
        return false;
    };
    // Java's Line.intersection never returns null (parallel pairs are
    // excluded by the caller's isParallel guard); the finite form
    // maps the unreachable parallel case to "not prohibited".
    let Some(intersection) = polyline.lines[index].intersection(line) else {
        return false;
    };
    let Some(layer) = board.trace_layer(trace_id) else {
        return false;
    };
    let nets = match board.get(trace_id) {
        Some(entry) => entry.nets.clone(),
        None => return false,
    };
    let overlap_items = manager.pick_items(board, &intersection, layer);
    let mut pad_found = false;
    for item_id in overlap_items {
        let (item_nets, item_is_pin, item_is_trace) = match board.get(item_id) {
            Some(entry) => (
                entry.nets.clone(),
                matches!(entry.data, ItemData::Pin { .. }),
                matches!(entry.data, ItemData::Trace { .. }),
            ),
            None => continue,
        };
        if !contacts::shares_net_no(&item_nets, &nets) {
            continue;
        }
        if item_is_pin {
            if let Some(center) = board.pin_center(item_id)
                && center == intersection
            {
                return false; // split always at the center of a drill item.
            }
            pad_found = true;
        } else if item_is_trace {
            let (first, last) = match board.trace_polyline(item_id) {
                Some(lines) => (first_corner(lines), last_corner(lines)),
                None => (None, None),
            };
            // The precedence quirk above — parens are JAVA's, not the
            // "obvious" reading.
            if (item_id != trace_id && first.as_ref() == Some(&intersection))
                || last.as_ref() == Some(&intersection)
            {
                return false;
            }
        }
    }
    pad_found
}

// ---------------------------------------------------------------------------
// normalize (PolylineTraceNormalization.java)
// ---------------------------------------------------------------------------

/// Java `BasicBoard.insertTrace(Polyline, ...)` (`BasicBoard.java:210-243`)
/// — the CLEANING insert: [`insert_trace_without_cleaning`], then
/// `newTrace.normalize(clipShape)`. Java `:223-229` is the NULL-CLIP
/// face of the changed-area session: `clipShape = null` unless the
/// board is a `RoutingBoard` with an ACTIVE session, and the normalize
/// runs UNCONDITIONALLY either way (a null clip means unbounded
/// normalization). The Java try/catch demotes a normalization panic to
/// an FRLogger.warn + debug line (D12 log-only); the port has no panic
/// path in [`normalize`] and returns its `bool` — the CALLER's result,
/// not the wrapper's, mirrors Java (whose insertTrace returns void).
#[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
pub fn insert_trace(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    polyline: Polyline,
    layer: i32,
    half_width: i32,
    nets: &[i32],
    clearance_class: i32,
    fixed: FixedState,
) -> Option<ItemId> {
    let new_trace = insert_trace_without_cleaning(
        manager,
        board,
        polyline,
        layer,
        half_width,
        nets,
        clearance_class,
        fixed,
    )?;
    let clip = board.changed_area.as_ref().map(|area| area.get_area(layer));
    let _changed = normalize(manager, board, new_trace, clip.as_ref());
    Some(new_trace)
}

/// Java `PolylineTraceNormalization.MAX_NORMALIZATION_DEPTH`.
pub(crate) const MAX_NORMALIZATION_DEPTH: i32 = 16;

/// Java `PolylineTrace.normalize(IntOctagon)` — the entry point at
/// depth 0 (`PolylineTraceNormalization.java:20-22`).
pub fn normalize(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
    clip: Option<&IntOctagon>,
) -> bool {
    normalize_rec(manager, board, trace_id, clip, 0)
}

/// Java `PolylineTraceNormalization.normalize(trace, clip, depth)`
/// (`:24-132`). `depth > 16` returns FALSE with the trace kept
/// (`:26-41`; the FRLogger.debug line is D12) — STRICTLY greater:
/// depth 16 still normalizes. The result starts as
/// `splitPieces.size() != 1` and can only be forced true afterwards.
/// Per piece, in split order: combine first, then — checked BEFORE
/// the combine else-if — the degenerate removal (`cornerCount == 2`
/// and `first == last`), gated on `!isDeletionForbidden` (a
/// USER_FIXED degenerate is kept silently, NORM1; the UNFIXED one is
/// removed, NORM2); else if combined, recurse at `depth + 1` and
/// force the result true (NORM3). The observer brackets (`:49-57`,
/// `:128-130`) are D12.
fn normalize_rec(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    trace_id: ItemId,
    clip: Option<&IntOctagon>,
    normalization_depth: i32,
) -> bool {
    if normalization_depth > MAX_NORMALIZATION_DEPTH {
        // Java: FRLogger.debug("...max normalization depth...");
        return false;
    }
    let split_pieces = split_clip(manager, board, trace_id, clip);
    let mut result = split_pieces.len() != 1;
    for piece_id in &split_pieces {
        if !board.is_on_the_board(*piece_id) {
            continue;
        }
        let trace_combined = combine(manager, board, *piece_id);
        let degenerate = match board.trace_polyline(*piece_id) {
            Some(lines) => {
                lines.corner_count() == 2
                    && first_corner(lines) == last_corner(lines)
                    && first_corner(lines).is_some()
            }
            None => false,
        };
        if degenerate {
            // remove trace with only 1 corner — only if deletion is
            // allowed (checked BEFORE the combine else-if).
            if !is_deletion_forbidden(board, *piece_id) {
                remove_item_through_repository(manager, board, *piece_id);
                result = true;
            }
            // Java: FRLogger.debug for the forbidden degenerate — D12.
        } else if trace_combined {
            normalize_rec(manager, board, *piece_id, clip, normalization_depth + 1);
            result = true;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::parse_board_from_text;
    use epic_geometry::int_box::IntBox;
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
    /// Capture contrast: `R5_CONTACTS ... bForbidden=true`. The
    /// NON-signal-layer conduction-area branch (the `!is_signal`
    /// arm) is deliberately UNPINNED here: the PURE board declares
    /// only signal layers, and adding a plane layer to the fixture
    /// would re-index every baseline item/tree row this file pins —
    /// construction is disproportionate to the branch's risk.
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
        // The two `C2_POST_TREE_ROW` leaf rows (capture, prefix
        // stripped), pinned as a SORTED SET — D18: Java took
        // MERGE_ENTRIES here, so only the geometry-derived leaf rows
        // are stable, not their position in the I-node history.
        let mut got_leaves: Vec<String> =
            survivor.iter().map(|row| row.trim().to_owned()).collect();
        got_leaves.sort();
        let mut expected_leaves: Vec<&str> = vec![
            "L obj=10 idx=0 oct[9875 29875 30125 30125 -20177 177 39823 60177]",
            "L obj=10 idx=1 oct[29875 29875 30125 50125 -20177 177 59823 80177]",
        ];
        expected_leaves.sort_unstable();
        assert_eq!(
            got_leaves, expected_leaves,
            "C2_POST_TREE_ROW survivors (sorted set)"
        );
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
        // C4_POST_TREE (capture `--C4_TREE--`, `C4_POST_TREE lines=33`):
        // 16 baseline + 3 inserted - 2 removed = 17 leaves; the merged
        // survivor carries ONE leaf row.
        assert_eq!(
            manager.default_tree().leaf_count(),
            17,
            "C4_POST_TREE leaves"
        );
        let survivor = leaf_rows_for(&manager, a);
        assert_eq!(survivor.len(), 1, "C4 survivor leaf count");
        assert!(
            survivor[0].ends_with("oct[9875 49875 70125 50125 -40177 20177 59823 120177]"),
            "C4_POST_TREE_ROW survivor: {survivor:?}"
        );
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
        // C5_POST_TREE (capture `--C5_TREE--`, `C5_POST_TREE lines=33`):
        // 17 leaves, one merged survivor leaf.
        assert_eq!(
            manager.default_tree().leaf_count(),
            17,
            "C5_POST_TREE leaves"
        );
        let survivor = leaf_rows_for(&manager, a);
        assert_eq!(survivor.len(), 1, "C5 survivor leaf count");
        assert!(
            survivor[0].ends_with("oct[9875 19875 70125 20125 -10177 50177 29823 90177]"),
            "C5_POST_TREE_ROW survivor: {survivor:?}"
        );
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
        // The disjoint-interval -1 branch of firstCommonLayer —
        // capture `R2_FCL_AB -1` (the ONLY cross-layer pair on this
        // board; the walk's `tmp_layer < 0` guard never sees it,
        // because walked pairs always come from layer-matched
        // contact sets).
        assert_eq!(first_common_layer(&mut board, a, b), -1, "R2_FCL_AB -1");
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
        // Capture `R3_B_USER_FIXED false B_FORBIDDEN false`:
        // SHOVE_FIXED does NOT count as user-fixed and is NOT
        // deletion-forbidden — a port mutant gating on
        // `>= ShoveFixed` flips the first probe.
        assert!(
            !is_user_fixed(board.get(b).expect("R3 b alive")),
            "R3_B_USER_FIXED false (SHOVE_FIXED < USER_FIXED)"
        );
        assert!(!is_deletion_forbidden(&board, b), "R3_B_FORBIDDEN false");
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
        // D1_POST_TREE (capture `--D1_TREE--`, `D1_POST_TREE lines=33`):
        // 17 leaves, one merged survivor leaf.
        assert_eq!(
            manager.default_tree().leaf_count(),
            17,
            "D1_POST_TREE leaves"
        );
        let survivor = leaf_rows_for(&manager, a);
        assert_eq!(survivor.len(), 1, "D1 survivor leaf count");
        assert!(
            survivor[0].ends_with("oct[9875 9875 20125 10125 -177 10177 19823 30177]"),
            "D1_POST_TREE_ROW survivor: {survivor:?}"
        );
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
    /// `D3_B_ALIVE false`. This test is also the combine-loop
    /// HALF-ORDER witness: an end-first loop rotates the survivor to
    /// `[10000,10000 30000,10000 30000,30000 10000,30000
    /// 10000,10000]` (mutation-verified — see [`combine`]'s docs);
    /// the C3/C5 shapes cannot catch that swap.
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
        // D3_POST_TREE (capture `--D3_TREE--`, `D3_POST_TREE lines=39`):
        // 16 baseline leaves + the ring's FOUR survivor leaves (idx
        // 0..3, pinned byte-for-tail below) = 20.
        assert_eq!(
            manager.default_tree().leaf_count(),
            20,
            "D3_POST_TREE leaves"
        );
        let survivor = leaf_rows_for(&manager, a);
        assert_eq!(survivor.len(), 4, "D3 survivor leaf count");
        // D3 is a MERGE_ENTRIES case in Java (D18): the I-node topology
        // may differ from the port's replaceGeometry history, so the
        // four leaf rows are pinned as a SORTED SET — the row TEXTS are
        // the capture's, the order is history-dependent.
        let mut got_leaves: Vec<String> =
            survivor.iter().map(|row| row.trim().to_owned()).collect();
        got_leaves.sort();
        let mut expected_leaves: Vec<&str> = vec![
            "L obj=10 idx=3 oct[9875 29875 30125 30125 -20177 177 39823 60177]",
            "L obj=10 idx=0 oct[9875 9875 10125 30125 -20177 177 19823 40177]",
            "L obj=10 idx=1 oct[9875 9875 30125 10125 -177 20177 19823 40177]",
            "L obj=10 idx=2 oct[29875 9875 30125 30125 -177 20177 39823 60177]",
        ];
        expected_leaves.sort_unstable();
        assert_eq!(
            got_leaves, expected_leaves,
            "D3_POST_TREE_ROW survivors (sorted set)"
        );
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
            raw(&get_connection_items(
                &manager,
                &mut board,
                t1,
                StopConnectionOption::None
            )),
            vec![13, 11, 10],
            "Y1_CONN_ITEMS_RETURNED [13:T,11:T,10:T]"
        );
        assert_eq!(
            raw(&get_connection_items(
                &manager,
                &mut board,
                ItemId::new(12),
                StopConnectionOption::None
            )),
            vec![12],
            "Y1_CONN_12 [12:T]"
        );
        assert_eq!(
            raw(&get_connection_items(
                &manager,
                &mut board,
                ItemId::new(13),
                StopConnectionOption::None
            )),
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
            raw(&get_connection_items(
                &manager,
                &mut board,
                t1,
                StopConnectionOption::None
            )),
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
            raw(&get_connection_items(
                &manager,
                &mut board,
                a,
                StopConnectionOption::None
            )),
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

    // ------------------------------------------------------------------
    // Fix round (spec review of 333f5499): PINWALK / DEGEN / DEGEN2 /
    // DEGEN3 — capture `/tmp/epic-t11-combine.out` `--PINWALK--`,
    // `--DEGEN--`, `--DEGEN2--`, `--DEGEN3--` sections.
    // ------------------------------------------------------------------

    /// The spike's insNet: an insert carrying ANOTHER item's net
    /// numbers (the pin-routability case needs an OTHER-net trace).
    fn insert_nets(
        manager: &mut SearchTreeManager,
        board: &mut Board,
        corners: &[(i32, i32)],
        nets: &[i32],
    ) -> ItemId {
        let template = ItemId::new(4);
        let layer = board.trace_layer(template).expect("parse trace 4");
        let half_width = board.trace_half_width(template).expect("parse trace 4");
        let clearance_class = board.item_clearance_class(template).expect("parse trace 4");
        insert_trace_without_cleaning(
            manager,
            board,
            poly(corners),
            layer,
            half_width,
            nets,
            clearance_class,
            FixedState::Unfixed,
        )
        .expect("insert succeeded")
    }

    /// The spike's `board.insertVia(via.getPadstack(), center, ...)`:
    /// clone via 5's padstack record into a NEW via at `center` and
    /// run the same alloc → insert → tree-broadcast path the trace
    /// helpers use (the port's `insertVia` facade is M3; the quality
    /// probes only need the item + index state). PAD_C600 spans ONE
    /// layer, so Java's splitTraces tail is a no-op for these probe
    /// placements — no trace crosses the touched centers, nothing
    /// above moves.
    fn insert_via_fixture(
        manager: &mut SearchTreeManager,
        board: &mut Board,
        center: IntPoint,
        nets: &[i32],
    ) -> ItemId {
        let template = ItemId::new(5);
        let (padstack_no, attach_smd_allowed, clearance_class) = match board.get(template) {
            Some(entry) => match &entry.data {
                ItemData::Via {
                    padstack_no,
                    attach_smd_allowed,
                    ..
                } => (*padstack_no, *attach_smd_allowed, entry.clearance_class),
                _ => unreachable!("template 5 is the parse via"),
            },
            None => unreachable!("template 5 is the parse via"),
        };
        let id = board.alloc_id();
        board.insert_item(ItemEntry {
            id,
            data: ItemData::Via {
                center,
                padstack_no,
                attach_smd_allowed,
            },
            nets: nets.to_vec(),
            clearance_class,
            component_id: 0,
            fixed: FixedState::Unfixed,
            on_the_board: false,
        });
        manager.insert(board, id);
        id
    }

    /// PINWALK — the isRoutable KIND gate. The base `Item.isRoutable()`
    /// returns FALSE (`Item.java:908-910`); only `Trace`
    /// (`Trace.java:206-209`) and `Via` (`Via.java:147-150`) override
    /// it — a same-net PIN is a contact the walk touches, but the walk
    /// must EXCLUDE it from the result set AND stop when it is the
    /// next hop, while a via IS walked into. The contrast lives on one
    /// board: A (id 10) carries the PIN's net (2) and ends at the pin;
    /// B (id 11) carries MINE (1) and ends at the via. Capture:
    /// `PINWALK_NETS pin=2 a=2 b=1 via=1`, `PINWALK_ROUTABLE pin=false
    /// via=true trace=true`, `PINWALK_A_CONTACTS [3:P]`,
    /// `PINWALK_FROM_TRACE_A [10:T]`, `PINWALK_FROM_PIN [10:T]`,
    /// `PINWALK_FROM_VIA [11:T,5:V]`, `PINWALK_POST_ITEM` for every id
    /// 1..11 (all walks read-only). A port that treats ANY item with
    /// nets as routable returns `[10,3]` / `[10,3]` / `[11,5,3]` here;
    /// a gate that forgets VIA returns `[10]` on the last probe. The
    /// APPENDED probes (capture `PINWALK_VIA2_ID 12` …
    /// `PINWALK_NETLESS_ID 14 ROUTABLE false`) close the gate's other
    /// blind spots: via 12 at via 5's own center exercises the
    /// Drill↔Drill dispatch (dead on a one-via board) and shows the
    /// via's contact set growing; traces 13/14 prove the two
    /// NON-kind clauses — a port gating on kind ALONE marks the
    /// USER_FIXED trace routable, a port gating on nets ALONE marks
    /// the netless trace routable.
    #[test]
    fn pin_routable_kind_gate() {
        let (mut manager, mut board) = fresh();
        let pin_nets = board.get(ItemId::new(3)).expect("parse pin 3").nets.clone();
        assert_eq!(pin_nets, vec![2], "PINWALK_NETS pin=2");
        let a = insert_nets(
            &mut manager,
            &mut board,
            &[(20_000, 40_000), (20_000, 30_000)],
            &pin_nets,
        );
        let b = insert(
            &mut manager,
            &mut board,
            &[(40_000, 40_000), (50_000, 40_000)],
        );
        assert_eq!([a.get(), b.get()], [10, 11], "PINWALK A=10 B=11");
        // PINWALK_ROUTABLE pin=false via=true trace=true.
        assert!(
            !is_routable(&board, ItemId::new(3)),
            "PINWALK_ROUTABLE pin=false"
        );
        assert!(
            is_routable(&board, ItemId::new(5)),
            "PINWALK_ROUTABLE via=true"
        );
        assert!(is_routable(&board, a), "PINWALK_ROUTABLE trace=true");
        // PINWALK_A_CONTACTS [3:P] — the pin IS in the contact set the
        // walk consumes.
        assert_eq!(
            raw(&contacts::all_contacts(&manager, &mut board, a)),
            vec![3],
            "PINWALK_A_CONTACTS [3:P]"
        );
        assert_eq!(
            raw(&get_connection_items(
                &manager,
                &mut board,
                a,
                StopConnectionOption::None
            )),
            vec![10],
            "PINWALK_FROM_TRACE_A [10:T] (pin touched, excluded, walk stopped)"
        );
        assert_eq!(
            raw(&get_connection_items(
                &manager,
                &mut board,
                ItemId::new(3),
                StopConnectionOption::None
            )),
            vec![10],
            "PINWALK_FROM_PIN [10:T] (result.add(this) is isRoutable-gated)"
        );
        assert_eq!(
            raw(&get_connection_items(
                &manager,
                &mut board,
                ItemId::new(5),
                StopConnectionOption::None
            )),
            vec![11, 5],
            "PINWALK_FROM_VIA [11:T,5:V] (a via IS routable and lands in the set)"
        );
        // PINWALK_POST dump: every item survives (read-only walks).
        for id in 1..=11 {
            assert!(
                board.get(ItemId::new(id)).is_some(),
                "PINWALK_POST_ITEM id={id} alive"
            );
        }
        // Quality-round probes (MINOR 3a/3c), after the pinned rows
        // above: a SECOND via at via 5's own center, a USER_FIXED
        // same-net trace and a NETLESS trace.
        let via2 = insert_via_fixture(
            &mut manager,
            &mut board,
            IntPoint::new(40_000, 40_000),
            &[1],
        );
        assert_eq!(via2.get(), 12, "PINWALK_VIA2_ID 12");
        assert_eq!(
            normal_contact_point(&mut board, ItemId::new(5), via2),
            Some(ip(40_000, 40_000)),
            "PINWALK_DRILL_DRILL_NCP 40000,40000"
        );
        assert_eq!(
            raw(&item_normal_contacts(&manager, &mut board, via2)),
            vec![11, 5],
            "PINWALK_VIA2_CONTACTS [11:T,5:V]"
        );
        assert_eq!(
            raw(&item_normal_contacts(&manager, &mut board, ItemId::new(5))),
            vec![12, 11],
            "PINWALK_VIA_CONTACTS_NOW [12:V,11:T]"
        );
        let fixed_trace = insert_with(
            &mut manager,
            &mut board,
            &[(75_000, 45_000), (85_000, 45_000)],
            0,
            0,
            FixedState::UserFixed,
        );
        assert_eq!(fixed_trace.get(), 13, "PINWALK_FIXED_ID 13");
        assert!(
            !is_routable(&board, fixed_trace),
            "PINWALK_FIXED_ID 13 ROUTABLE false (kind right, USER_FIXED clause)"
        );
        let netless_trace = insert_nets(
            &mut manager,
            &mut board,
            &[(75_000, 50_000), (85_000, 50_000)],
            &[],
        );
        assert_eq!(netless_trace.get(), 14, "PINWALK_NETLESS_ID 14");
        assert!(
            !is_routable(&board, netless_trace),
            "PINWALK_NETLESS_ID 14 ROUTABLE false (kind right, netCount clause)"
        );
    }

    /// DEGEN — two single-segment traces joined end-to-end do NOT
    /// collapse: a constructed `Polyline` carries corners+1 stored
    /// lines, so the join gives 3 lines and ONE merged survivor. (The
    /// review round predicted both traces would vanish here — the jar
    /// disagreed; spike-vs-recon correction.) Capture:
    /// `DEGEN_PRED branch=MERGE_ENTRIES atStart=false reverse=false
    /// skipLine=true newCount=3 joinedLen=3 joinedCorners=2
    /// survivor=[60000,30000 85000,30000]`, `DEGEN_COMBINE true`,
    /// `DEGEN_A_ALIVE true B_ALIVE false`, `DEGEN_POST_TREE lines=33`
    /// with the obj=10 leaf `oct[59875 29875 85125 30125 29823 55177
    /// 89823 115177]`, `DEGEN_NEXT_ID 12`.
    #[test]
    fn single_segment_join_merges_and_survives() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(60_000, 30_000), (70_000, 30_000)],
        );
        let b = insert(
            &mut manager,
            &mut board,
            &[(70_000, 30_000), (85_000, 30_000)],
        );
        assert_eq!([a.get(), b.get()], [10, 11], "DEGEN A=10 B=11");
        assert!(combine(&mut manager, &mut board, a), "DEGEN_COMBINE true");
        assert_eq!(
            corners_of(&board, a),
            vec!["60000,30000", "85000,30000"],
            "DEGEN_A_ALIVE corners"
        );
        assert!(board.get(b).is_none(), "DEGEN_B_ALIVE false");
        assert_eq!(
            manager.default_tree().leaf_count(),
            17,
            "DEGEN_POST_TREE leaves (16 baseline + merged - removed)"
        );
        let survivor = leaf_rows_for(&manager, a);
        assert_eq!(survivor.len(), 1, "DEGEN survivor leaf count");
        assert!(
            survivor[0].ends_with("oct[59875 29875 85125 30125 29823 55177 89823 115177]"),
            "DEGEN_POST_TREE_ROW survivor: {survivor:?}"
        );
        let w = insert(
            &mut manager,
            &mut board,
            &[(60_000, 35_000), (65_000, 35_000)],
        );
        assert_eq!(w.get(), 12, "DEGEN_NEXT_ID 12 (no id reuse)");
    }

    /// DEGEN2 — the TRUE degenerate collapse: B retraces A exactly
    /// (the L and its reverse), so the joined path spikes out and
    /// back and the canonicalizing ctor cleans it to the EMPTY
    /// polyline (`joinedLen=0 < 3`): the shrink rule removes the
    /// RECEIVER and then ALWAYS the other trace — nothing survives,
    /// the tree is back to the parse baseline, and the combine loop's
    /// `isOnTheBoard()` recheck ends the loop. Capture:
    /// `DEGEN2_PRED_START branch=REPLACE_GEOMETRY atStart=true
    /// reverse=false skipLine=true newCount=5 joinedLen=0
    /// joinedCorners=NONE survivor=EMPTY` (PRED_END identical),
    /// `DEGEN2_COMBINE true`, `DEGEN2_A_ALIVE false B_ALIVE false`,
    /// `DEGEN2_POST_TREE lines=31` (the 31-row baseline), the POST
    /// dump without ids 10/11, `DEGEN2_NEXT_ID 12` (both burned).
    #[test]
    fn degenerate_collapse_removes_both() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(10_000, 10_000), (30_000, 10_000), (30_000, 30_000)],
        );
        let b = insert(
            &mut manager,
            &mut board,
            &[(30_000, 30_000), (30_000, 10_000), (10_000, 10_000)],
        );
        assert_eq!([a.get(), b.get()], [10, 11], "DEGEN2 A=10 B=11");
        assert!(combine(&mut manager, &mut board, a), "DEGEN2_COMBINE true");
        assert!(
            board.get(a).is_none(),
            "DEGEN2_A_ALIVE false (removed by the shrink rule)"
        );
        assert!(board.get(b).is_none(), "DEGEN2_B_ALIVE false");
        assert_eq!(
            manager.default_tree().leaf_count(),
            16,
            "DEGEN2_POST_TREE back to the parse baseline"
        );
        assert_eq!(board.item_count(), 9, "DEGEN2_POST: 9 parse + 2 - 2");
        let w = insert(
            &mut manager,
            &mut board,
            &[(60_000, 35_000), (65_000, 35_000)],
        );
        assert_eq!(w.get(), 12, "DEGEN2_NEXT_ID 12 (both ids burned)");
    }

    /// DEGEN3 — the boundary: B doubles back over A's vertical leg
    /// and OVERRUNS past A's first corner (collinear extension, not
    /// an exact retrace). The ctor keeps 3 lines (the overrun is real
    /// geometry), so the shrink rule does NOT fire — A survives
    /// carrying the merged overrun. The START half REFUSES (zero
    /// contacts at A's first corner; its `joinedLen=0` PRED row is an
    /// artifact — branchPred probes the arithmetic without the
    /// contact gate), so the END half's merge runs. Capture:
    /// `DEGEN3_PRED_END ... newCount=5 joinedLen=3 joinedCorners=2
    /// survivor=[10000,10000 0,10000]`, `DEGEN3_COMBINE true`,
    /// `DEGEN3_A_ALIVE true corners=[10000,10000 0,10000] B_ALIVE
    /// false`, `DEGEN3_NEXT_ID 12`.
    #[test]
    fn degenerate_overrun_merges_instead() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(10_000, 10_000), (30_000, 10_000), (30_000, 30_000)],
        );
        let b = insert(
            &mut manager,
            &mut board,
            &[(30_000, 30_000), (30_000, 10_000), (0, 10_000)],
        );
        assert_eq!([a.get(), b.get()], [10, 11], "DEGEN3 A=10 B=11");
        assert!(combine(&mut manager, &mut board, a), "DEGEN3_COMBINE true");
        assert_eq!(
            corners_of(&board, a),
            vec!["10000,10000", "0,10000"],
            "DEGEN3_A_ALIVE corners (the overrun survivor)"
        );
        assert!(board.get(b).is_none(), "DEGEN3_B_ALIVE false");
        assert_eq!(
            manager.default_tree().leaf_count(),
            17,
            "DEGEN3 post leaves (16 baseline + merged - removed)"
        );
        let w = insert(
            &mut manager,
            &mut board,
            &[(60_000, 35_000), (65_000, 35_000)],
        );
        assert_eq!(w.get(), 12, "DEGEN3_NEXT_ID 12");
    }
    /// **REQ1 — the requery cascade** (capture `--REQ1--` :588-605): a
    /// U-trace B whose legs cross A at x=25000 and x=65000. Each found
    /// split of B is followed, in the same entry iteration, by A's own
    /// split at that crossing, and the recursion restarts each piece
    /// with fresh queries — so B is split at BOTH crossings (12/13,
    /// then 13 -> 16/17) and A self-splits three times (14/15, 18/19,
    /// 20/21 at trace 9's end corner x=80000). Survivors 12/14/17/18/
    /// 20/21; the replaced 13/15/16/19 are absent; result carries the
    /// four outermost A pieces. Capture `REQ1_RESULT size=4
    /// [14:[20000,30000 25000,30000] 18:[25000,30000 65000,30000]
    /// 20:[65000,30000 80000,30000] 21:[80000,30000 90000,30000]]`,
    /// `REQ1_NEXT_ID 22`.
    #[test]
    fn req1_requery_second_crossing_cascade() {
        let (mut manager, mut board) = fresh();
        let _b = insert(
            &mut manager,
            &mut board,
            &[
                (25_000, 20_000),
                (25_000, 50_000),
                (65_000, 50_000),
                (65_000, 20_000),
            ],
        );
        let a = insert(
            &mut manager,
            &mut board,
            &[(20_000, 30_000), (90_000, 30_000)],
        );
        assert_eq!(a.get(), 11, "REQ1 A=11 (B=10)");
        let pieces = split_clip(&mut manager, &mut board, a, None);
        assert_eq!(
            raw(&pieces),
            vec![14, 18, 20, 21],
            "REQ1_RESULT size=4 [14:[20000,30000 25000,30000] \
             18:[25000,30000 65000,30000] 20:[65000,30000 80000,30000] \
             21:[80000,30000 90000,30000]]"
        );
        let survivors = [
            (12u32, &["25000,20000", "25000,30000"][..]),
            (14, &["20000,30000", "25000,30000"][..]),
            (17, &["65000,30000", "65000,20000"][..]),
            (18, &["25000,30000", "65000,30000"][..]),
            (20, &["65000,30000", "80000,30000"][..]),
            (21, &["80000,30000", "90000,30000"][..]),
        ];
        for (id, corners) in survivors {
            assert_eq!(
                corners_of(&board, ItemId::new(id)),
                corners,
                "REQ1_POST_ITEM id={id}"
            );
        }
        for gone in [10u32, 11, 13, 15, 16, 19] {
            assert!(
                !board.is_on_the_board(ItemId::new(gone)),
                "REQ1 replaced {gone} absent"
            );
        }
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 22, "REQ1_NEXT_ID 22");
    }

    /// **REQ2 — the append-and-rewind requery + the drilled piece left
    /// dead in the result** (capture `--REQ2--` :606-623): the same U
    /// as REQ1, but A runs through template via 5 (40000,40000). After
    /// the found split of B (12/13) and A's own split at x=25000
    /// (14/15), the walk of 15 finds 13's second leg and drills: piece
    /// 18 ([25000,40000 65000,40000]) is removed by the drill split
    /// DURING its own recursion — its own-split pieces 20/21 replace it
    /// on the board while the RESULT still carries the dead 18 (the
    /// DRL1 quirk through the recursion). Java requeries by APPENDING
    /// into the accumulated list and rewinding; on every construction
    /// tried (append vs replace vs no-requery flips) the outcome is
    /// identical — each entry kind is either idempotent under
    /// re-processing or removes the receiver, and the cascade flows
    /// through the recursion's fresh queries — so this pins the
    /// observable contract, not the list order. Capture
    /// `REQ2_RESULT size=3 [14:[20000,40000 25000,40000]
    /// 18:[25000,40000 65000,40000] 19:[65000,40000 90000,40000]]`,
    /// `REQ2_NEXT_ID 22`.
    #[test]
    fn req2_append_vs_replace_via_dead_piece() {
        let (mut manager, mut board) = fresh();
        let _b = insert(
            &mut manager,
            &mut board,
            &[
                (25_000, 20_000),
                (25_000, 50_000),
                (65_000, 50_000),
                (65_000, 20_000),
            ],
        );
        let a = insert(
            &mut manager,
            &mut board,
            &[(20_000, 40_000), (90_000, 40_000)],
        );
        assert_eq!(a.get(), 11, "REQ2 A=11 (B=10)");
        let pieces = split_clip(&mut manager, &mut board, a, None);
        assert_eq!(
            raw(&pieces),
            vec![14, 18, 19],
            "REQ2_RESULT size=3 [14:[20000,40000 25000,40000] \
             18:[25000,40000 65000,40000] 19:[65000,40000 90000,40000]]"
        );
        let survivors = [
            (12u32, &["25000,20000", "25000,40000"][..]),
            (14, &["20000,40000", "25000,40000"][..]),
            (17, &["65000,40000", "65000,20000"][..]),
            (19, &["65000,40000", "90000,40000"][..]),
            (20, &["25000,40000", "40000,40000"][..]),
            (21, &["40000,40000", "65000,40000"][..]),
        ];
        for (id, corners) in survivors {
            assert_eq!(
                corners_of(&board, ItemId::new(id)),
                corners,
                "REQ2_POST_ITEM id={id}"
            );
        }
        // the drilled 18 is dead on the board but IN the result.
        assert!(!board.is_on_the_board(ItemId::new(18)), "REQ2 18 drilled");
        for gone in [10u32, 11, 13, 15, 16] {
            assert!(
                !board.is_on_the_board(ItemId::new(gone)),
                "REQ2 replaced {gone} absent"
            );
        }
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 22, "REQ2_NEXT_ID 22");
    }

    // ------------------------------------------------------------------
    // T12 — the split family. Jar-pinned by
    // `rust/harness/oracle/SplitSpike.java` (capture
    // `/tmp/epic-t12-split.out`, full verbatim copy in
    // `.wolf/cache/bash/call_77b248d61e98472ba093a7da.log`; run twice
    // and diffed — deterministic). Every expected value below is a
    // LITERAL capture row (pin failure mode 8); absolute ids replay the
    // oracle's FULL insertion sequence per case.
    // ------------------------------------------------------------------

    /// [`PURE_DSN`] with the PIN on net MINE (net 1) instead of OTHER —
    /// byte-identical to `PURE_PAD_DSN` in `SplitSpike.java`. The parse
    /// item set (ids 1..9) is unchanged; only the pin's net assignment
    /// flips, which arms `splitInsideDrillPadProhibited`'s sharesNet
    /// gate for PAD5/PAD6.
    const PURE_PAD_DSN: &str = r#"(pcb t12-pad.dsn
  (parser
    (string_quote ")
    (space_in_quoted_tokens on)
  )
  (resolution um 1)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (boundary (rect pcb 0 0 100000 60000))
    (keepout (rect F.Cu 70000 10000 90000 20000))
    (rule (width 250) (clearance 14))
  )
  (placement
    (component CMP1
      (place CMP1 20000 40000 front 0)
    )
  )
  (library
    (padstack PAD_C600
      (shape (circle F.Cu 600 0 0))
    )
    (image CMP1
      (pin PAD_C600 P1 0 0)
    )
  )
  (network
    (net MINE (pins CMP1-P1))
    (net OTHER)
  )
  (wiring
    (wire (path F.Cu 250  20000 40000  10000 40000) (net MINE))
    (via PAD_C600 40000 40000 (net MINE))
    (wire (rect F.Cu 50000 10000 60000 20000) (net MINE))
    (wire (path F.Cu 250  55000 15000  55000 25000) (net MINE))
    (wire (path F.Cu 250  50000 15000  50000 25000) (net MINE))
    (wire (path F.Cu 250  80000 15000  80000 30000) (net MINE))
  )
)
"#;

    /// The PAD5/PAD6 board: same parse + creation-order tree fill as
    /// [`fresh`] (the geometry and ids are identical, so the baseline
    /// tree pins carry over; only the pin's net differs).
    fn fresh_pad() -> (SearchTreeManager, Board) {
        let mut board = parse_board_from_text(PURE_PAD_DSN);
        assert_eq!(board.item_count(), 9, "PAD board count=9");
        let mut manager = SearchTreeManager::new();
        manager.insert_items_creation_order(&mut board);
        (manager, board)
    }

    /// The capture's corner body of a RAW polyline (same format as
    /// [`corners_of`]).
    fn poly_corners(lines: &Polyline) -> Vec<String> {
        lines
            .corners()
            .iter()
            .map(|corner| match corner {
                Point::Int(p) => format!("{},{}", p.x, p.y),
                Point::Rational(_) => unreachable!("integer-board fixtures"),
            })
            .collect()
    }

    /// The spike's `new Line(...)`-built degenerate (`degeneratePoly`,
    /// `SplitSpike.java:812-817`): an exact retrace whose constructor
    /// normalization collapses to the 2-corner point polyline the
    /// capture prints (`ctorCorners=[30000,20000 30000,20000]`).
    fn degenerate_poly() -> Polyline {
        Polyline::new(vec![
            Line::new(ip(20_000, 20_000), ip(40_000, 20_000)),
            Line::new(ip(30_000, 20_000), ip(30_000, 30_000)),
            Line::new(ip(20_000, 20_000), ip(40_000, 20_000)),
        ])
    }

    /// [`insert_with`] for a RAW polyline (NORM1/NORM2 insert the
    /// collapsed degenerate directly).
    fn insert_poly_fixed(
        manager: &mut SearchTreeManager,
        board: &mut Board,
        lines: Polyline,
        fixed: FixedState,
    ) -> ItemId {
        let template = ItemId::new(4);
        let layer = board.trace_layer(template).expect("parse trace 4");
        let half_width = board.trace_half_width(template).expect("parse trace 4");
        let nets = board.get(template).expect("parse trace 4").nets.clone();
        let clearance_class = board.item_clearance_class(template).expect("parse trace 4");
        insert_trace_without_cleaning(
            manager,
            board,
            lines,
            layer,
            half_width,
            &nets,
            clearance_class,
            fixed,
        )
        .expect("insert succeeded")
    }

    /// The capture's `dumpIsect` line body: `a=x,y b=x,y` (the `Line`
    /// endpoints; the fields are public, the spike prints the same two
    /// points).
    fn line_pts(line: &Line) -> String {
        let (a, b) = match (&line.a, &line.b) {
            (Point::Int(a), Point::Int(b)) => (a, b),
            _ => unreachable!("integer-board fixtures"),
        };
        format!("a={},{} b={},{}", a.x, a.y, b.x, b.y)
    }

    /// **X1 — the perpendicular cross** (capture `--X1--`, `/tmp/
    /// epic-t12-split.out` :4-59). The receiver a (id 10) splits at the
    /// found trace's segment → [14,15]; the found trace b (id 11)
    /// splits FIRST (12,13) inside the same call; the receiver's end
    /// corner (50000,20000) touches F2 (id 8), whose found split
    /// fires the area-corner cascade — 8's lower half is cycle-removed
    /// and the upper half survives as 17. The FULL 39-row post-split
    /// tree dump is the byte pin (`X1_POST_TREE_ROW`, capture :20-58):
    /// it fixes the survivor id set (no 8, 10, 11, 16, 18) and every
    /// compensated octagon in one shot.
    #[test]
    fn x1_cross_split_with_tree() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(30_000, 20_000), (50_000, 20_000)],
        );
        let b = insert(
            &mut manager,
            &mut board,
            &[(40_000, 10_000), (40_000, 30_000)],
        );
        assert_eq!(a.get(), 10, "X1 A=10");
        assert_eq!(b.get(), 11, "X1 B=11");
        let pieces = split_clip(&mut manager, &mut board, a, None);
        assert_eq!(
            raw(&pieces),
            vec![14, 15],
            "X1_RESULT size=2 [14:[30000,20000 40000,20000] 15:[40000,20000 50000,20000]]"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(14)),
            vec!["30000,20000", "40000,20000"],
            "X1_POST_ITEM id=14"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(15)),
            vec!["40000,20000", "50000,20000"],
            "X1_POST_ITEM id=15"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(12)),
            vec!["40000,10000", "40000,20000"],
            "X1_POST_ITEM id=12"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(13)),
            vec!["40000,20000", "40000,30000"],
            "X1_POST_ITEM id=13"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(17)),
            vec!["50000,20000", "50000,25000"],
            "X1_POST_ITEM id=17 — the cascade survivor"
        );
        assert!(!board.is_on_the_board(a), "receiver 10 gone");
        assert!(!board.is_on_the_board(b), "found trace 11 gone");
        assert!(
            !board.is_on_the_board(ItemId::new(8)),
            "no X1_POST_ITEM id=8"
        );
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
            "                    I oct[9875 9875 60000 40300 -30177 50000 49823 80425]",
            "                        I oct[19700 9875 60000 40300 -20424 50000 49823 80425]",
            "                            I oct[19700 19875 40125 40300 -20424 20177 49823 60425]",
            "                                L obj=3 idx=0 oct[19700 39700 20300 40300 -20424 -19575 59576 60425]",
            "                                L obj=14 idx=0 oct[29875 19875 40125 20125 9823 20177 49823 60177]",
            "                            I oct[39700 9875 60000 40300 -424 50000 49823 80425]",
            "                                I oct[39700 19875 40300 40300 -424 20177 59823 80425]",
            "                                    L obj=5 idx=0 oct[39700 39700 40300 40300 -424 425 79576 80425]",
            "                                    L obj=13 idx=0 oct[39875 19875 40125 30125 9823 20177 59823 70177]",
            "                                I oct[39875 9875 60000 25125 19823 50000 49823 80177]",
            "                                    L obj=6 idx=0 oct[50000 10000 60000 20000 30000 50000 60000 80000]",
            "                                    I oct[39875 9875 55125 25125 19823 40177 49823 80177]",
            "                                        I oct[49875 14875 55125 25125 24823 40177 69823 80177]",
            "                                            L obj=7 idx=0 oct[54875 14875 55125 25125 29823 40177 69823 80177]",
            "                                            L obj=17 idx=0 oct[49875 19875 50125 25125 24823 30177 69823 75177]",
            "                                        I oct[39875 9875 50125 20125 19823 30177 49823 70177]",
            "                                            L obj=12 idx=0 oct[39875 9875 40125 20125 19823 30177 49823 60177]",
            "                                            L obj=15 idx=0 oct[39875 19875 50125 20125 19823 30177 59823 70177]",
            "                        L obj=4 idx=0 oct[9875 39875 20125 40125 -30177 -19823 49823 60177]",
            "                L obj=1 idx=7 oct[-100 -100 100 60100 -60141 141 -141 60141]",
        ];
        assert_eq!(dump.len(), 39, "X1_POST_TREE lines=39");
        let mismatches: Vec<String> = dump
            .iter()
            .zip(expected.iter())
            .filter(|(got, want)| got.as_str() != **want)
            .map(|(got, want)| format!("got {got:?}, want {want:?}"))
            .collect();
        assert!(
            mismatches.is_empty(),
            "X1_POST_TREE byte-compare: {mismatches:?}"
        );
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 18, "X1_NEXT_ID 18");
    }

    /// **X2 — the receiver-asymmetric `LineSegment::intersection`**
    /// (capture `--X2--` :60-66). The FOUND branch queries
    /// `found.intersection(current)` (the returned line is the CURRENT
    /// segment's middle — capture `X2_CROSS_FOUND_FIRST`), the own
    /// branch flips the receivers (`X2_CROSS_OWN`). On a collinear
    /// OVERLAP both orders return the same two zero-length border
    /// lines (rows 3-4); through a diagonal the perpendicular receiver
    /// wins the whole other line in the found order and vice versa
    /// (rows 5-6). Pure geometry — no board.
    #[test]
    fn x2_intersection_receiver_order() {
        let seg_a = segment_at(
            &poly(&[(30_000, 20_000), (50_000, 20_000)]),
            0,
            ItemId::new(1),
            "X2",
        );
        let seg_b = segment_at(
            &poly(&[(40_000, 10_000), (40_000, 30_000)]),
            0,
            ItemId::new(1),
            "X2",
        );
        let found_first = seg_b.intersection(&seg_a);
        assert_eq!(found_first.len(), 1, "X2_CROSS_FOUND_FIRST len=1");
        assert_eq!(
            line_pts(&found_first[0]),
            "a=30000,20000 b=50000,20000",
            "X2_CROSS_FOUND_FIRST"
        );
        let own = seg_a.intersection(&seg_b);
        assert_eq!(own.len(), 1, "X2_CROSS_OWN len=1");
        assert_eq!(
            line_pts(&own[0]),
            "a=40000,10000 b=40000,30000",
            "X2_CROSS_OWN"
        );

        let seg_c = segment_at(
            &poly(&[(20_000, 10_000), (50_000, 10_000)]),
            0,
            ItemId::new(1),
            "X2",
        );
        let seg_d = segment_at(
            &poly(&[(30_000, 10_000), (65_000, 10_000)]),
            0,
            ItemId::new(1),
            "X2",
        );
        let overlap_found = seg_d.intersection(&seg_c);
        let overlap_own = seg_c.intersection(&seg_d);
        assert_eq!(overlap_found.len(), 2, "X2_OVERLAP_FOUND_FIRST len=2");
        assert_eq!(
            line_pts(&overlap_found[0]),
            "a=30000,10000 b=30000,10001",
            "X2_OVERLAP_FOUND_FIRST[0]"
        );
        assert_eq!(
            line_pts(&overlap_found[1]),
            "a=50000,10000 b=50000,9999",
            "X2_OVERLAP_FOUND_FIRST[1]"
        );
        assert_eq!(overlap_own.len(), 2, "X2_OVERLAP_OWN len=2");
        assert_eq!(
            line_pts(&overlap_own[0]),
            "a=30000,10000 b=30000,10001",
            "X2_OVERLAP_OWN[0]"
        );
        assert_eq!(
            line_pts(&overlap_own[1]),
            "a=50000,10000 b=50000,9999",
            "X2_OVERLAP_OWN[1]"
        );

        let diag = segment_at(
            &poly(&[(60_000, 20_000), (70_000, 30_000)]),
            0,
            ItemId::new(1),
            "X2",
        );
        let vert = segment_at(
            &poly(&[(65_000, 15_000), (65_000, 35_000)]),
            0,
            ItemId::new(1),
            "X2",
        );
        let diag_found = vert.intersection(&diag);
        assert_eq!(diag_found.len(), 1, "X2_DIAG_FOUND_FIRST len=1");
        assert_eq!(
            line_pts(&diag_found[0]),
            "a=60000,20000 b=70000,30000",
            "X2_DIAG_FOUND_FIRST"
        );
        let diag_own = diag.intersection(&vert);
        assert_eq!(diag_own.len(), 1, "X2_DIAG_OWN len=1");
        assert_eq!(
            line_pts(&diag_own[0]),
            "a=65000,15000 b=65000,35000",
            "X2_DIAG_OWN"
        );
    }

    /// **X3 — the collinear overlap** (capture `--X3--` :67-81). The
    /// found split of b (11) produces the duplicate-span piece 12,
    /// which the two-pass cycle removal eats (RIC start∩end = {12's
    /// trace contacts}); the receiver survives as [14,15] and the
    /// found remainder keeps 13. Capture `X3_RESULT size=2
    /// [14:[20000,10000 30000,10000] 15:[30000,10000 50000,10000]]`,
    /// `X3_NEXT_ID 16` (12 burned, 13-15 live ids).
    #[test]
    fn x3_collinear_overlap_cycle_removal() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(20_000, 10_000), (50_000, 10_000)],
        );
        let b = insert(
            &mut manager,
            &mut board,
            &[(30_000, 10_000), (65_000, 10_000)],
        );
        assert_eq!(a.get(), 10, "X3 A=10");
        assert_eq!(b.get(), 11, "X3 B=11");
        let pieces = split_clip(&mut manager, &mut board, a, None);
        assert_eq!(
            raw(&pieces),
            vec![14, 15],
            "X3_RESULT size=2 [14:[20000,10000 30000,10000] 15:[30000,10000 50000,10000]]"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(14)),
            vec!["20000,10000", "30000,10000"],
            "X3_POST_ITEM id=14"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(15)),
            vec!["30000,10000", "50000,10000"],
            "X3_POST_ITEM id=15"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(13)),
            vec!["50000,10000", "65000,10000"],
            "X3_POST_ITEM id=13"
        );
        assert!(!board.is_on_the_board(a), "receiver 10 gone");
        assert!(!board.is_on_the_board(b), "found trace 11 gone");
        assert!(
            !board.is_on_the_board(ItemId::new(12)),
            "no X3_POST_ITEM id=12 — the duplicate-span cycle piece"
        );
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 16, "X3_NEXT_ID 16");
    }

    /// **X4 — the self-cross** (capture `--X4--` :82-95): a 5-corner
    /// trace whose last segment crosses its first; the found branch is
    /// the trace ITSELF (own-entry skip via the corner-equality
    /// forms), result is ONE piece plus the tail (14). Capture
    /// `X4_RESULT size=1 [11:[10000,10000 30000,10000]]`,
    /// `X4_NEXT_ID 15`.
    #[test]
    fn x4_self_split_reversal() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[
                (10_000, 10_000),
                (50_000, 10_000),
                (50_000, 40_000),
                (30_000, 40_000),
                (30_000, 0),
            ],
        );
        assert_eq!(a.get(), 10, "X4 A=10");
        let pieces = split_clip(&mut manager, &mut board, a, None);
        assert_eq!(
            raw(&pieces),
            vec![11],
            "X4_RESULT size=1 [11:[10000,10000 30000,10000]]"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(11)),
            vec!["10000,10000", "30000,10000"],
            "X4_POST_ITEM id=11"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(14)),
            vec!["30000,10000", "30000,0"],
            "X4_POST_ITEM id=14"
        );
        assert!(!board.is_on_the_board(a), "receiver 10 gone");
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 15, "X4_NEXT_ID 15");
    }

    /// **X4L — the loop trace** (capture `--X4L--` :96-109): a
    /// 7-corner trace that retraces its own diagonal corner
    /// (20000,20000 twice); the same self-split machinery cuts it at
    /// the retrace corner. Capture `X4L_RESULT size=1
    /// [11:[10000,10000 20000,20000]]`, `X4L_NEXT_ID 15`.
    #[test]
    fn x4l_loop_trace_split() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[
                (10_000, 10_000),
                (20_000, 20_000),
                (30_000, 20_000),
                (30_000, 30_000),
                (20_000, 30_000),
                (20_000, 20_000),
                (10_000, 30_000),
            ],
        );
        assert_eq!(a.get(), 10, "X4L L=10");
        let pieces = split_clip(&mut manager, &mut board, a, None);
        assert_eq!(
            raw(&pieces),
            vec![11],
            "X4L_RESULT size=1 [11:[10000,10000 20000,20000]]"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(11)),
            vec!["10000,10000", "20000,20000"],
            "X4L_POST_ITEM id=11"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(14)),
            vec!["20000,20000", "10000,30000"],
            "X4L_POST_ITEM id=14"
        );
        assert!(!board.is_on_the_board(a), "receiver 10 gone");
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 15, "X4L_NEXT_ID 15");
    }

    /// **DRL1 — the drill split through the VIA center** (capture
    /// `--DRL1--` :110-160). The receiver a (id 10) crosses via 5's
    /// center (40000,40000), the LAST overlapping entry; the drill
    /// branch splits at the center with the perpendicular line and
    /// DISCARDS the split result — `ownTraceSplit` stays false, so the
    /// END tail re-adds the (now removed) receiver itself:
    /// `DRL1_RESULT size=1 [10:[30000,40000 50000,40000]]` with
    /// `DRL1_A_ON_BOARD false` — a dead id in the returned collection
    /// (the Java DRL1 quirk, preserved verbatim). The FULL 35-row tree
    /// (`DRL1_POST_TREE_ROW`, capture :125-159) pins the survivor
    /// pieces 11/12 and every octagon.
    #[test]
    fn drl1_via_drill_split_keeps_dead_receiver_in_result() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(30_000, 40_000), (50_000, 40_000)],
        );
        assert_eq!(a.get(), 10, "DRL1 A=10");
        let pieces = split_clip(&mut manager, &mut board, a, None);
        assert_eq!(
            raw(&pieces),
            vec![10],
            "DRL1_RESULT size=1 [10:[30000,40000 50000,40000]]"
        );
        assert!(!board.is_on_the_board(a), "DRL1_A_ON_BOARD false");
        assert_eq!(
            corners_of(&board, ItemId::new(11)),
            vec!["30000,40000", "40000,40000"],
            "DRL1_POST_ITEM id=11"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(12)),
            vec!["40000,40000", "50000,40000"],
            "DRL1_POST_ITEM id=12"
        );
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
            "            I oct[-100 -100 60000 60100 -60141 50000 -141 90177]",
            "                I oct[-100 -100 60000 60100 -60141 50000 -141 90177]",
            "                    L obj=1 idx=3 oct[-100 -100 100 60100 -60141 141 -141 60141]",
            "                    I oct[9875 10000 60000 40300 -30177 50000 49823 90177]",
            "                        I oct[19700 10000 60000 40300 -20424 50000 59576 80425]",
            "                            I oct[19700 39700 40125 40300 -20424 177 59576 80177]",
            "                                L obj=3 idx=0 oct[19700 39700 20300 40300 -20424 -19575 59576 60425]",
            "                                L obj=11 idx=0 oct[29875 39875 40125 40125 -10177 177 69823 80177]",
            "                            I oct[39700 10000 60000 40300 -424 50000 60000 80425]",
            "                                L obj=5 idx=0 oct[39700 39700 40300 40300 -424 425 79576 80425]",
            "                                I oct[49875 10000 60000 25125 24823 50000 60000 80177]",
            "                                    I oct[49875 10000 60000 25125 24823 50000 60000 80000]",
            "                                        L obj=6 idx=0 oct[50000 10000 60000 20000 30000 50000 60000 80000]",
            "                                        L obj=8 idx=0 oct[49875 14875 50125 25125 24823 35177 64823 75177]",
            "                                    L obj=7 idx=0 oct[54875 14875 55125 25125 29823 40177 69823 80177]",
            "                        I oct[9875 39875 50125 40125 -30177 10177 49823 90177]",
            "                            L obj=4 idx=0 oct[9875 39875 20125 40125 -30177 -19823 49823 60177]",
            "                            L obj=12 idx=0 oct[39875 39875 50125 40125 -177 10177 79823 90177]",
            "                L obj=1 idx=7 oct[-100 -100 100 60100 -60141 141 -141 60141]",
        ];
        assert_eq!(dump.len(), 35, "DRL1_POST_TREE lines=35");
        let mismatches: Vec<String> = dump
            .iter()
            .zip(expected.iter())
            .filter(|(got, want)| got.as_str() != **want)
            .map(|(got, want)| format!("got {got:?}, want {want:?}"))
            .collect();
        assert!(
            mismatches.is_empty(),
            "DRL1_POST_TREE byte-compare: {mismatches:?}"
        );
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 13, "DRL1_NEXT_ID 13");
    }

    /// **DRL2 — the drill split at the FIRST entry** (capture
    /// `--DRL2--` :161-176): the new via 11 at (45000,40000) sits
    /// before the old via 5 in the entry walk, so the drill branch
    /// fires on it and the `break` skips via 5 entirely; the early
    /// empty return gives `DRL2_RESULT size=0 []` (contrast DRL1's
    /// [dead receiver]). Capture `DRL2_NEXT_ID 14`.
    #[test]
    fn drl2_first_entry_drill_split_empty_result() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(30_000, 40_000), (50_000, 40_000)],
        );
        assert_eq!(a.get(), 10, "DRL2 A=10");
        let via2 = insert_via_fixture(
            &mut manager,
            &mut board,
            IntPoint::new(45_000, 40_000),
            &[1],
        );
        assert_eq!(via2.get(), 11, "DRL2 via2=11");
        let pieces = split_clip(&mut manager, &mut board, a, None);
        assert_eq!(raw(&pieces), Vec::<u32>::new(), "DRL2_RESULT size=0 []");
        assert!(!board.is_on_the_board(a), "DRL2_A_ON_BOARD false");
        assert_eq!(
            corners_of(&board, ItemId::new(12)),
            vec!["30000,40000", "45000,40000"],
            "DRL2_POST_ITEM id=12"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(13)),
            vec!["45000,40000", "50000,40000"],
            "DRL2_POST_ITEM id=13"
        );
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 14, "DRL2_NEXT_ID 14");
    }

    /// **PAD1 — the FOREIGN pad does not prohibit** (capture `--PAD1--`
    /// :177-192): the crossing trace b (11) is net MINE like the
    /// receiver, but the PIN at (20000,40000) carries OTHER, so the
    /// sharesNet gate never arms and the split at (20000,40250) —
    /// inside the pad's 600 radius — proceeds. Capture
    /// `PAD1_RESULT size=2 [14:[10000,40250 20000,40250]
    /// 15:[20000,40250 30000,40250]]`, `PAD1_NEXT_ID 16`.
    #[test]
    fn pad1_foreign_pin_pad_split_proceeds() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(10_000, 40_250), (30_000, 40_250)],
        );
        let b = insert(
            &mut manager,
            &mut board,
            &[(20_000, 38_000), (20_000, 42_000)],
        );
        assert_eq!(a.get(), 10, "PAD1 A=10");
        assert_eq!(b.get(), 11, "PAD1 B=11");
        let pieces = split_clip(&mut manager, &mut board, a, None);
        assert_eq!(
            raw(&pieces),
            vec![14, 15],
            "PAD1_RESULT size=2 [14:[10000,40250 20000,40250] 15:[20000,40250 30000,40250]]"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(14)),
            vec!["10000,40250", "20000,40250"],
            "PAD1_POST_ITEM id=14"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(15)),
            vec!["20000,40250", "30000,40250"],
            "PAD1_POST_ITEM id=15"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(12)),
            vec!["20000,38000", "20000,40250"],
            "PAD1_POST_ITEM id=12"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(13)),
            vec!["20000,40250", "20000,42000"],
            "PAD1_POST_ITEM id=13"
        );
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 16, "PAD1_NEXT_ID 16");
    }

    /// **PAD2 — the foreign pin is gated OUT; the allow is the
    /// same-net trace ENDPOINT** (capture `--PAD2--` :193-206): the
    /// split point is pin 3's center, but pin 3 is net OTHER — the
    /// raw `sharesNet` gate skips the pin before the
    /// `center == isect` allow could ever run (PAD6 is the true
    /// center-allow pin, same-net). The split at (20000,40000) is
    /// allowed by the same-net trace 4's FIRST corner sitting on it
    /// — the found-trace endpoint clause of
    /// `splitInsideDrillPadProhibited`. Capture
    /// `PAD2_RESULT size=2 [11:[20000,30000 20000,40000]
    /// 12:[20000,40000 20000,50000]]`, `PAD2_NEXT_ID 13`.
    #[test]
    fn pad2_split_at_pin_center_allowed() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(20_000, 30_000), (20_000, 50_000)],
        );
        assert_eq!(a.get(), 10, "PAD2 A=10");
        let pieces = split_clip(&mut manager, &mut board, a, None);
        assert_eq!(
            raw(&pieces),
            vec![11, 12],
            "PAD2_RESULT size=2 [11:[20000,30000 20000,40000] 12:[20000,40000 20000,50000]]"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(11)),
            vec!["20000,30000", "20000,40000"],
            "PAD2_POST_ITEM id=11"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(12)),
            vec!["20000,40000", "20000,50000"],
            "PAD2_POST_ITEM id=12"
        );
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 13, "PAD2_NEXT_ID 13");
    }

    /// **PAD3 — the other-trace-endpoint allow** (capture `--PAD3--`
    /// :207-221): the perpendicular trace u (11) ENDS at the split
    /// point (20000,40250); the `(currentTrace != this &&
    /// first.equals(isect)) || last.equals(isect)` clause allows the
    /// split (the precedence quirk is exercised by PAD4's refusal).
    /// Capture `PAD3_RESULT size=2 [12:[10000,40250 20000,40250]
    /// 13:[20000,40250 30000,40250]]` with 11 intact,
    /// `PAD3_NEXT_ID 14`.
    #[test]
    fn pad3_other_trace_endpoint_allows() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(10_000, 40_250), (30_000, 40_250)],
        );
        let u = insert(
            &mut manager,
            &mut board,
            &[(20_000, 40_250), (20_000, 45_250)],
        );
        assert_eq!(a.get(), 10, "PAD3 A=10");
        assert_eq!(u.get(), 11, "PAD3 U=11");
        let pieces = split_clip(&mut manager, &mut board, a, None);
        assert_eq!(
            raw(&pieces),
            vec![12, 13],
            "PAD3_RESULT size=2 [12:[10000,40250 20000,40250] 13:[20000,40250 30000,40250]]"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(12)),
            vec!["10000,40250", "20000,40250"],
            "PAD3_POST_ITEM id=12"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(13)),
            vec!["20000,40250", "30000,40250"],
            "PAD3_POST_ITEM id=13"
        );
        assert_eq!(
            corners_of(&board, u),
            vec!["20000,40250", "20000,45250"],
            "PAD3_POST_ITEM id=11 intact"
        );
        assert!(board.is_on_the_board(u), "PAD3 id=11 alive");
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 14, "PAD3_NEXT_ID 14");
    }

    /// **PAD4 — the precedence quirk with the refused closed piece**
    /// (capture `--PAD4--` :222-234): w (id 10, net OTHER via
    /// `insert_nets`) is a closed 5-corner ring whose LAST corner is
    /// the split point (20000,40250). `Trace.split(Point)` splits the
    /// FIRST containing segment into `[first_piece, rest]` — the first
    /// piece [10000,40250..20000,40250] is allowed (its end corner
    /// equals the isect), but the REST is the closed remainder whose
    /// `Polyline.split` refuses → `PAD4_SPLIT_POINT
    /// [11:[10000,40250 20000,40250] null]` and id 12 is burned by the
    /// refused construction (`PAD4_NEXT_ID 13`).
    #[test]
    fn pad4_precedence_quirk_closed_piece_refused() {
        let (mut manager, mut board) = fresh();
        let w = insert_nets(
            &mut manager,
            &mut board,
            &[
                (10_000, 40_250),
                (30_000, 40_250),
                (30_000, 45_000),
                (20_000, 45_000),
                (20_000, 40_250),
            ],
            &[2],
        );
        assert_eq!(w.get(), 10, "PAD4 W=10 nets=[2]");
        let pieces = split_at_point(&mut manager, &mut board, w, &ip(20_000, 40_250));
        let [first, second] = pieces.expect("PAD4_SPLIT_POINT present");
        assert_eq!(first.map(|id| id.get()), Some(11), "PAD4 first piece id=11");
        assert_eq!(second, None, "PAD4 second piece null");
        assert_eq!(
            corners_of(&board, ItemId::new(11)),
            vec!["10000,40250", "20000,40250"],
            "PAD4_POST_ITEM id=11"
        );
        assert!(!board.is_on_the_board(w), "receiver 10 gone");
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 13, "PAD4_NEXT_ID 13 — id 12 burned");
    }

    /// **PAD5 — the REAL same-net pin-pad refusal** (capture `--PAD5--`
    /// :235-249, on [`fresh_pad`]): the pin now carries MINE, the
    /// sharesNet gate arms, the split point (20000,40250) is inside
    /// the pad, and neither allow clause applies (not the center; the
    /// found trace b's endpoints are NOT the isect) → the own split is
    /// refused and the receiver comes back INTACT:
    /// `PAD5_RESULT size=1 [10:[10000,40250 30000,40250]]`,
    /// `PAD5_A_ON_BOARD true B_ON_BOARD true`, `PAD5_NEXT_ID 12` —
    /// no piece ids allocated at all.
    #[test]
    fn pad5_same_net_pin_pad_refusal() {
        let (mut manager, mut board) = fresh_pad();
        assert_eq!(
            board.get(ItemId::new(3)).expect("parse pin 3").nets,
            vec![1],
            "PAD5 pinNets=[1]"
        );
        let a = insert(
            &mut manager,
            &mut board,
            &[(10_000, 40_250), (30_000, 40_250)],
        );
        let b = insert(
            &mut manager,
            &mut board,
            &[(20_000, 38_000), (20_000, 42_000)],
        );
        assert_eq!(a.get(), 10, "PAD5 A=10");
        assert_eq!(b.get(), 11, "PAD5 B=11");
        let pieces = split_clip(&mut manager, &mut board, a, None);
        assert_eq!(
            raw(&pieces),
            vec![10],
            "PAD5_RESULT size=1 [10:[10000,40250 30000,40250]]"
        );
        assert!(board.is_on_the_board(a), "PAD5_A_ON_BOARD true");
        assert!(board.is_on_the_board(b), "PAD5_B_ON_BOARD true");
        assert_eq!(
            corners_of(&board, a),
            vec!["10000,40250", "30000,40250"],
            "PAD5_POST_ITEM id=10 intact"
        );
        assert_eq!(
            corners_of(&board, b),
            vec!["20000,38000", "20000,42000"],
            "PAD5_POST_ITEM id=11 intact"
        );
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 12, "PAD5_NEXT_ID 12");
    }

    /// **PAD6 — the center allow in the same-net board** (capture
    /// `--PAD6--` :250-263): with the pin armed (net MINE), the split
    /// AT the pin center still short-circuits to allow — the
    /// `center == isect` check precedes any padFound effect. Capture
    /// `PAD6_RESULT size=2 [11:[20000,30000 20000,40000]
    /// 12:[20000,40000 20000,50000]]`, `PAD6_NEXT_ID 13`.
    #[test]
    fn pad6_same_net_pin_center_allow() {
        let (mut manager, mut board) = fresh_pad();
        let a = insert(
            &mut manager,
            &mut board,
            &[(20_000, 30_000), (20_000, 50_000)],
        );
        assert_eq!(a.get(), 10, "PAD6 A=10");
        let pieces = split_clip(&mut manager, &mut board, a, None);
        assert_eq!(
            raw(&pieces),
            vec![11, 12],
            "PAD6_RESULT size=2 [11:[20000,30000 20000,40000] 12:[20000,40000 20000,50000]]"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(11)),
            vec!["20000,30000", "20000,40000"],
            "PAD6_POST_ITEM id=11"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(12)),
            vec!["20000,40000", "20000,50000"],
            "PAD6_POST_ITEM id=12"
        );
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 13, "PAD6_NEXT_ID 13");
    }

    /// **AR1 — the area-cycle removal** (capture `--AR1--` :264-276):
    /// both endpoints of a (10) contact conduction area 6, so after
    /// the split the two-pass cycle removal eats every piece —
    /// `AR1_RESULT size=0 []`, `AR1_A_ON_BOARD false`,
    /// `AR1_NEXT_ID 11` (the piece ids were allocated then removed).
    #[test]
    fn ar1_area_cycle_removal() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(50_500, 10_500), (59_500, 10_500)],
        );
        assert_eq!(a.get(), 10, "AR1 A=10");
        let pieces = split_clip(&mut manager, &mut board, a, None);
        assert_eq!(raw(&pieces), Vec::<u32>::new(), "AR1_RESULT size=0 []");
        assert!(!board.is_on_the_board(a), "AR1_A_ON_BOARD false");
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 11, "AR1_NEXT_ID 11");
    }

    /// **AR2 — the ignoreCyclesWithAreas contrast** (capture `--AR2--`
    /// :277-290): flipping the net class flag (the GUI setter,
    /// `NetClass.java:148`) before the split makes the cycle walk
    /// ignore area contacts — the receiver survives UNCHANGED:
    /// `AR2_RESULT size=1 [10:[50500,10500 59500,10500]]`,
    /// `AR2_A_ON_BOARD true`, `AR2_NEXT_ID 11`. This is the pin that
    /// requires the REAL `ignore_cycles_with_areas` field — a
    /// hard-coded `false` stub passes AR1 and fails here.
    #[test]
    fn ar2_ignore_cycles_with_areas_keeps_trace() {
        let (mut manager, mut board) = fresh();
        let class_no = board.rules().nets.get(1).expect("net MINE").net_class;
        board.rules_mut().net_classes[class_no as usize].set_ignore_cycles_with_areas(true);
        assert!(
            board.rules().net_classes[class_no as usize].ignore_cycles_with_areas,
            "AR2 flipped: true"
        );
        let a = insert(
            &mut manager,
            &mut board,
            &[(50_500, 10_500), (59_500, 10_500)],
        );
        assert_eq!(a.get(), 10, "AR2 A=10");
        let pieces = split_clip(&mut manager, &mut board, a, None);
        assert_eq!(
            raw(&pieces),
            vec![10],
            "AR2_RESULT size=1 [10:[50500,10500 59500,10500]]"
        );
        assert!(board.is_on_the_board(a), "AR2_A_ON_BOARD true");
        assert_eq!(
            corners_of(&board, a),
            vec!["50500,10500", "59500,10500"],
            "AR2_POST_ITEM id=10 intact"
        );
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 11, "AR2_NEXT_ID 11");
    }

    /// **DEL1 — the deletion-forbidden refusal** (capture `--DEL1--`
    /// :291-305): both the receiver u1 (10) and the found u2 (11) are
    /// USER_FIXED; the found split refuses (`isDeletionForbidden`)
    /// AND the own split refuses, so the receiver is returned intact:
    /// `DEL1_RESULT size=1 [10:[10000,10000 30000,10000]]`, both
    /// fixed states still USER_FIXED, `DEL1_NEXT_ID 12`.
    #[test]
    fn del1_deletion_forbidden_refusal() {
        let (mut manager, mut board) = fresh();
        let u1 = insert_with(
            &mut manager,
            &mut board,
            &[(10_000, 10_000), (30_000, 10_000)],
            0,
            0,
            FixedState::UserFixed,
        );
        let u2 = insert_with(
            &mut manager,
            &mut board,
            &[(20_000, 8_000), (20_000, 12_000)],
            0,
            0,
            FixedState::UserFixed,
        );
        assert_eq!(u1.get(), 10, "DEL1 U1=10");
        assert_eq!(u2.get(), 11, "DEL1 U2=11");
        let pieces = split_clip(&mut manager, &mut board, u1, None);
        assert_eq!(
            raw(&pieces),
            vec![10],
            "DEL1_RESULT size=1 [10:[10000,10000 30000,10000]]"
        );
        assert!(board.is_on_the_board(u1), "receiver intact");
        assert!(board.is_on_the_board(u2), "found intact");
        assert_eq!(
            board.get(u1).expect("u1").fixed,
            FixedState::UserFixed,
            "DEL1_U1_FIXED USER_FIXED"
        );
        assert_eq!(
            board.get(u2).expect("u2").fixed,
            FixedState::UserFixed,
            "DEL1_U2_FIXED USER_FIXED"
        );
        assert_eq!(
            corners_of(&board, u1),
            vec!["10000,10000", "30000,10000"],
            "DEL1_POST_ITEM id=10 intact"
        );
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 12, "DEL1_NEXT_ID 12");
    }

    /// **DEL2 — the mixed case** (capture `--DEL2--` :306-321): the
    /// USER_FIXED found trace v (11) refuses ITS split, but the plain
    /// receiver w (10) still splits around it — the found refusal is
    /// per-trace, not a whole-call abort. Capture `DEL2_RESULT size=2
    /// [12:[10000,20000 20000,20000] 13:[20000,20000 30000,20000]]`,
    /// `DEL2_V_FIXED USER_FIXED V_ON_BOARD true`, `DEL2_NEXT_ID 14`.
    #[test]
    fn del2_mixed_fixed_found_refuses_receiver_splits() {
        let (mut manager, mut board) = fresh();
        let w = insert(
            &mut manager,
            &mut board,
            &[(10_000, 20_000), (30_000, 20_000)],
        );
        let v = insert_with(
            &mut manager,
            &mut board,
            &[(20_000, 18_000), (20_000, 22_000)],
            0,
            0,
            FixedState::UserFixed,
        );
        assert_eq!(w.get(), 10, "DEL2 W=10");
        assert_eq!(v.get(), 11, "DEL2 V=11");
        let pieces = split_clip(&mut manager, &mut board, w, None);
        assert_eq!(
            raw(&pieces),
            vec![12, 13],
            "DEL2_RESULT size=2 [12:[10000,20000 20000,20000] 13:[20000,20000 30000,20000]]"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(12)),
            vec!["10000,20000", "20000,20000"],
            "DEL2_POST_ITEM id=12"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(13)),
            vec!["20000,20000", "30000,20000"],
            "DEL2_POST_ITEM id=13"
        );
        assert!(board.is_on_the_board(v), "DEL2 V_ON_BOARD true");
        assert_eq!(
            board.get(v).expect("v").fixed,
            FixedState::UserFixed,
            "DEL2_V_FIXED USER_FIXED"
        );
        assert!(!board.is_on_the_board(w), "receiver 10 gone");
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 14, "DEL2_NEXT_ID 14");
    }

    /// Builds the three CLIP traces (the diagonal A crossing b and c)
    /// and returns their ids — the shared setup of CLIP1/CLIP2
    /// (capture `--CLIP1-- A=10 B=11 C=12`).
    fn clip_setup(manager: &mut SearchTreeManager, board: &mut Board) -> ItemId {
        let a = insert(
            manager,
            board,
            &[(20_000, 20_000), (60_000, 20_000), (70_000, 30_000)],
        );
        let b = insert(manager, board, &[(40_000, 10_000), (40_000, 30_000)]);
        let c = insert(manager, board, &[(65_000, 15_000), (65_000, 35_000)]);
        assert_eq!(a.get(), 10, "CLIP A=10");
        assert_eq!(b.get(), 11, "CLIP B=11");
        assert_eq!(c.get(), 12, "CLIP C=12");
        a
    }

    /// The clip octagon of CLIP1 — the bounding octagon of the box
    /// 62000-68000 x 24000-28000 (spike `makeClip`).
    fn clip_octagon() -> IntOctagon {
        let tile = TileShape::RegularTileShape(RegularTileShape::IntBox(IntBox::from_corners(
            62_000, 24_000, 68_000, 28_000,
        )));
        tile.bounding_octagon().expect("a box is bounded")
    }

    /// **CLIP1 — the clip-restricted split** (capture `--CLIP1--`
    /// :322-338): only the diagonal's segments INTERSECTING the clip
    /// octagon are considered, so the receiver splits exactly at the
    /// clip boundary corner (65000,25000) → [15,16]; c (12) still
    /// splits there (its intersection lies inside the clip), b (11)
    /// lies outside and stays whole. Capture `CLIP1_RESULT size=2
    /// [15:[20000,20000 60000,20000 65000,25000]
    /// 16:[65000,25000 70000,30000]]`, `CLIP1_NEXT_ID 17`.
    #[test]
    fn clip1_clip_restricted_split() {
        let (mut manager, mut board) = fresh();
        let a = clip_setup(&mut manager, &mut board);
        let clip = clip_octagon();
        let pieces = split_clip(&mut manager, &mut board, a, Some(&clip));
        assert_eq!(
            raw(&pieces),
            vec![15, 16],
            "CLIP1_RESULT size=2 [15:[20000,20000 60000,20000 65000,25000] 16:[65000,25000 70000,30000]]"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(15)),
            vec!["20000,20000", "60000,20000", "65000,25000"],
            "CLIP1_POST_ITEM id=15"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(16)),
            vec!["65000,25000", "70000,30000"],
            "CLIP1_POST_ITEM id=16"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(13)),
            vec!["65000,15000", "65000,25000"],
            "CLIP1_POST_ITEM id=13"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(14)),
            vec!["65000,25000", "65000,35000"],
            "CLIP1_POST_ITEM id=14"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(11)),
            vec!["40000,10000", "40000,30000"],
            "CLIP1_POST_ITEM id=11 intact"
        );
        assert!(!board.is_on_the_board(a), "receiver 10 gone");
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 17, "CLIP1_NEXT_ID 17");
    }

    /// **CLIP2 — the same board with NO clip** (capture `--CLIP2--`
    /// :339-358): every crossing splits — the receiver lands in four
    /// pieces (15, 19, 27, 28), and the found traces b/c contribute
    /// 13/14 and 25/26 plus 18/22 from F2 and F. The full cascade is
    /// the loop-order pin: each found split requeries and restarts
    /// the segment walk. Capture `CLIP2_RESULT size=4
    /// [15:[20000,20000 40000,20000] 19:[40000,20000 50000,20000]
    /// 27:[55000,20000 60000,20000 65000,25000] 28:[65000,25000
    /// 70000,30000]]`, `CLIP2_NEXT_ID 29`.
    #[test]
    fn clip2_no_clip_full_cascade() {
        let (mut manager, mut board) = fresh();
        let a = clip_setup(&mut manager, &mut board);
        let pieces = split_clip(&mut manager, &mut board, a, None);
        assert_eq!(
            raw(&pieces),
            vec![15, 19, 27, 28],
            "CLIP2_RESULT size=4 [15:[20000,20000 40000,20000] 19:[40000,20000 50000,20000] 27:[55000,20000 60000,20000 65000,25000] 28:[65000,25000 70000,30000]]"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(15)),
            vec!["20000,20000", "40000,20000"],
            "CLIP2_POST_ITEM id=15"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(19)),
            vec!["40000,20000", "50000,20000"],
            "CLIP2_POST_ITEM id=19"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(27)),
            vec!["55000,20000", "60000,20000", "65000,25000"],
            "CLIP2_POST_ITEM id=27"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(28)),
            vec!["65000,25000", "70000,30000"],
            "CLIP2_POST_ITEM id=28"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(13)),
            vec!["40000,10000", "40000,20000"],
            "CLIP2_POST_ITEM id=13"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(14)),
            vec!["40000,20000", "40000,30000"],
            "CLIP2_POST_ITEM id=14"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(18)),
            vec!["50000,20000", "50000,25000"],
            "CLIP2_POST_ITEM id=18"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(22)),
            vec!["55000,20000", "55000,25000"],
            "CLIP2_POST_ITEM id=22"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(25)),
            vec!["65000,15000", "65000,25000"],
            "CLIP2_POST_ITEM id=25"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(26)),
            vec!["65000,25000", "65000,35000"],
            "CLIP2_POST_ITEM id=26"
        );
        assert!(!board.is_on_the_board(a), "receiver 10 gone");
        assert!(
            !board.is_on_the_board(ItemId::new(8)),
            "no CLIP2_POST_ITEM id=8 — consumed by the cascade"
        );
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 29, "CLIP2_NEXT_ID 29");
    }

    /// **NORM1 — the degenerate USER_FIXED trace is KEPT** (capture
    /// `--NORM1--` :359-373): the exact-retrace polyline collapses at
    /// construction to the 2-corner point polyline
    /// (`NORM1_INSERT id=10 corners=[30000,20000 30000,20000]`);
    /// normalize's degenerate branch runs BEFORE the combine else-if,
    /// but `isDeletionForbidden` (USER_FIXED) refuses the removal —
    /// `NORM1_NORMALIZE false`, trace alive, `NORM1_NEXT_ID 11`.
    #[test]
    fn norm1_degenerate_user_fixed_kept() {
        let (mut manager, mut board) = fresh();
        let degenerate = degenerate_poly();
        assert_eq!(
            poly_corners(&degenerate),
            vec!["30000,20000", "30000,20000"],
            "NORM1 ctorCorners=[30000,20000 30000,20000]"
        );
        let d = insert_poly_fixed(&mut manager, &mut board, degenerate, FixedState::UserFixed);
        assert_eq!(d.get(), 10, "NORM1_INSERT id=10");
        assert_eq!(
            corners_of(&board, d),
            vec!["30000,20000", "30000,20000"],
            "NORM1_INSERT corners"
        );
        assert_eq!(
            board.get(d).expect("d").fixed,
            FixedState::UserFixed,
            "NORM1_INSERT fixed=USER_FIXED"
        );
        assert!(
            !normalize(&mut manager, &mut board, d, None),
            "NORM1_NORMALIZE false"
        );
        assert!(board.is_on_the_board(d), "NORM1_ALIVE true");
        assert_eq!(
            corners_of(&board, d),
            vec!["30000,20000", "30000,20000"],
            "NORM1_POST_ITEM id=10 corners intact"
        );
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 11, "NORM1_NEXT_ID 11");
    }

    /// **NORM2 — the degenerate UNFIXED trace is REMOVED** (capture
    /// `--NORM2--` :374-386): same construction, fixed state flipped
    /// to UNFIXED after the insert (`d.setFixedState`) — the
    /// degenerate branch removes the trace and forces the verdict:
    /// `NORM2_NORMALIZE true`, `NORM2_ALIVE false`, `NORM2_NEXT_ID 11`
    /// (no piece ids — the removal burns nothing).
    #[test]
    fn norm2_degenerate_unfixed_removed() {
        let (mut manager, mut board) = fresh();
        let d = insert_poly_fixed(
            &mut manager,
            &mut board,
            degenerate_poly(),
            FixedState::UserFixed,
        );
        assert_eq!(d.get(), 10, "NORM2 id=10");
        board.set_item_fixed(d, FixedState::Unfixed);
        assert!(
            normalize(&mut manager, &mut board, d, None),
            "NORM2_NORMALIZE true"
        );
        assert!(!board.is_on_the_board(d), "NORM2_ALIVE false");
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 11, "NORM2_NEXT_ID 11");
    }

    /// Builds the NORM3/NORM3P board: A (10) — B (11) joined at
    /// (40000,20000), C (12) vertical through the joint, with F2 (8)
    /// touching B's far end. Returns A.
    fn norm3_setup(manager: &mut SearchTreeManager, board: &mut Board) -> ItemId {
        let a = insert(manager, board, &[(30_000, 20_000), (40_000, 20_000)]);
        let b = insert(manager, board, &[(40_000, 20_000), (50_000, 20_000)]);
        let c = insert(manager, board, &[(35_000, 10_000), (35_000, 30_000)]);
        assert_eq!(a.get(), 10, "NORM3 A=10");
        assert_eq!(b.get(), 11, "NORM3 B=11");
        assert_eq!(c.get(), 12, "NORM3 C=12");
        a
    }

    /// **NORM3 — the recursion end state** (capture `--NORM3--`
    /// :387-405). One `normalize(A)` call: the split of A at C
    /// produces 15/16; combine(16) joins B's remainder; the found
    /// split of F2 (8) fires the area-corner cascade (17 removed, 18
    /// survives) and the recursion merges the chain into
    /// `16=[35000,20000 50000,20000 50000,25000]`.
    /// `NORM3_NORMALIZE true`, `NORM3_NEXT_ID 19`.
    #[test]
    fn norm3_recursion_end_state() {
        let (mut manager, mut board) = fresh();
        let a = norm3_setup(&mut manager, &mut board);
        assert!(
            normalize(&mut manager, &mut board, a, None),
            "NORM3_NORMALIZE true"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(16)),
            vec!["35000,20000", "50000,20000", "50000,25000"],
            "NORM3_POST_ITEM id=16"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(15)),
            vec!["30000,20000", "35000,20000"],
            "NORM3_POST_ITEM id=15"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(14)),
            vec!["35000,20000", "35000,30000"],
            "NORM3_POST_ITEM id=14"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(13)),
            vec!["35000,10000", "35000,20000"],
            "NORM3_POST_ITEM id=13"
        );
        assert!(!board.is_on_the_board(a), "receiver 10 gone");
        assert!(!board.is_on_the_board(ItemId::new(11)), "B=11 gone");
        assert!(!board.is_on_the_board(ItemId::new(12)), "C=12 gone");
        assert!(!board.is_on_the_board(ItemId::new(8)), "F2=8 gone");
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 19, "NORM3_NEXT_ID 19");
    }

    /// **NORM3P — the probe that isolates the area-corner cycle**
    /// (capture `--NORM3P--`, preserved log :73-104). Same board, but
    /// the recursion's steps replayed one at a time:
    /// `NORM3P_SPLIT size=2 [15:... 16:[35000,20000 40000,20000]]`,
    /// `NORM3P_COMBINE id=15 combined=false`,
    /// `NORM3P_COMBINE id=16 combined=true` (16 becomes
    /// [35000,20000 50000,20000]), `NORM3P_S16 size=1`,
    /// `NORM3P_RIC id=16 removed=false alive=true` — 16 itself is NOT
    /// a cycle — yet the split INSIDE it consumed F2 (8) and its lower
    /// half (17): the AFTER_RIC dump lists 18 but neither 8 nor 17.
    #[test]
    fn norm3p_probe_area_corner_cycle() {
        let (mut manager, mut board) = fresh();
        let a = norm3_setup(&mut manager, &mut board);
        let pieces = split_clip(&mut manager, &mut board, a, None);
        assert_eq!(
            raw(&pieces),
            vec![15, 16],
            "NORM3P_SPLIT size=2 [15:[30000,20000 35000,20000] 16:[35000,20000 40000,20000]]"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(16)),
            vec!["35000,20000", "40000,20000"],
            "NORM3P_SPLIT id=16"
        );
        assert!(
            !combine(&mut manager, &mut board, ItemId::new(15)),
            "NORM3P_COMBINE id=15 combined=false"
        );
        assert!(
            combine(&mut manager, &mut board, ItemId::new(16)),
            "NORM3P_COMBINE id=16 combined=true"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(16)),
            vec!["35000,20000", "50000,20000"],
            "NORM3P_COMBINE id=16 corners"
        );
        let s16 = split_clip(&mut manager, &mut board, ItemId::new(16), None);
        assert_eq!(
            raw(&s16),
            vec![16],
            "NORM3P_S16 size=1 [16:[35000,20000 50000,20000]]"
        );
        assert!(
            !remove_if_cycle(&mut manager, &mut board, ItemId::new(16)),
            "NORM3P_RIC id=16 removed=false alive=true"
        );
        assert!(board.is_on_the_board(ItemId::new(16)), "NORM3P_RIC alive");
        assert_eq!(
            corners_of(&board, ItemId::new(18)),
            vec!["50000,20000", "50000,25000"],
            "NORM3P_AFTER_RIC_ITEM id=18"
        );
        assert!(!board.is_on_the_board(ItemId::new(8)), "no AFTER_RIC id=8");
        assert!(
            !board.is_on_the_board(ItemId::new(17)),
            "no AFTER_RIC id=17"
        );
        assert!(!board.is_on_the_board(ItemId::new(11)), "B=11 gone");
        assert!(!board.is_on_the_board(ItemId::new(12)), "C=12 gone");
    }

    /// **NORM4 — the recursion chain across 36 inserted traces**
    /// (capture `--NORM4--`, preserved log :105-224). The cascade
    /// crosses every C×V intersection; pass 1's full dump (60 rows)
    /// pins the survivor id set — its discriminating subset is
    /// asserted here, including the absence of F2 (8) consumed by the
    /// same area-corner mechanism. The SECOND PASS re-normalizes every
    /// alive trace DESCENDING from a snapshot; the whole verdict row
    /// (`NORM4_SECOND_PASS`, preserved log :166) is pinned as one
    /// string. After it, 53's merged chain is re-inserted as 116 and
    /// 47 is consumed (capture `NORM4_PASS2_ITEM id=116
    /// corners=[48000,20000 50000,20000 50000,25000]`; no PASS2 id=53
    /// or id=47 rows), and `NORM4_NEXT_ID 118`.
    #[test]
    fn norm4_recursion_chain_two_pass() {
        let (mut manager, mut board) = fresh();
        let z = insert(
            &mut manager,
            &mut board,
            &[(15_000, 20_000), (17_000, 20_000)],
        );
        assert_eq!(z.get(), 10, "NORM4 Z=10");
        for i in 1..=18 {
            insert(
                &mut manager,
                &mut board,
                &[(15_000 + 2_000 * i, 20_000), (17_000 + 2_000 * i, 20_000)],
            );
        }
        for i in 1..=17 {
            insert(
                &mut manager,
                &mut board,
                &[(16_000 + 2_000 * i, 18_000), (16_000 + 2_000 * i, 22_000)],
            );
        }
        assert_eq!(board.item_count(), 45, "NORM4 items=45");
        assert!(
            normalize(&mut manager, &mut board, z, None),
            "NORM4_NORMALIZE true"
        );
        // PASS1 dump subset (preserved log :107-165).
        assert_eq!(
            corners_of(&board, ItemId::new(115)),
            vec!["50000,20000", "50000,25000"],
            "NORM4_PASS1_ITEM id=115"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(53)),
            vec!["48000,20000", "50000,20000"],
            "NORM4_PASS1_ITEM id=53"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(47)),
            vec!["50000,20000", "50000,22000"],
            "NORM4_PASS1_ITEM id=47"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(113)),
            vec!["18000,20000", "20000,20000"],
            "NORM4_PASS1_ITEM id=113"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(112)),
            vec!["15000,20000", "18000,20000"],
            "NORM4_PASS1_ITEM id=112"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(111)),
            vec!["18000,20000", "18000,22000"],
            "NORM4_PASS1_ITEM id=111"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(110)),
            vec!["18000,18000", "18000,20000"],
            "NORM4_PASS1_ITEM id=110"
        );
        assert!(!board.is_on_the_board(ItemId::new(8)), "no PASS1 id=8 row");
        // SECOND PASS: snapshot of alive traces, descending, each
        // normalize(None) — the spike walks the same snapshot (removed
        // traces still get a call and return false, like Java's
        // off-board split early-return).
        let ids: Vec<ItemId> = board
            .iter_descending()
            .filter(|entry| matches!(entry.data, ItemData::Trace { .. }))
            .map(|entry| entry.id)
            .collect();
        let mut verdicts: Vec<String> = Vec::new();
        for id in ids {
            let changed = normalize(&mut manager, &mut board, id, None);
            verdicts.push(format!("{}={}", id.get(), changed));
        }
        assert_eq!(
            verdicts.join(","),
            "115=true,113=false,112=false,111=false,110=false,109=false,107=false,106=false,\
             105=false,103=false,102=false,101=false,99=false,98=false,97=false,95=false,\
             94=false,93=false,91=false,90=false,89=false,87=false,86=false,85=false,83=false,\
             82=false,81=false,79=false,78=false,77=false,75=false,74=false,73=false,71=false,\
             70=false,69=false,67=false,66=false,65=false,63=false,62=false,61=false,59=false,\
             58=false,57=false,55=false,54=false,53=true,51=false,50=false,47=true,9=false,\
             7=false,4=false",
            "NORM4_SECOND_PASS (preserved log :166)"
        );
        // PASS2 dump facts (preserved log :167-223): the `53=true` /
        // `47=true` verdicts mean 53 was REPLACED — its merged chain
        // re-inserted as 116 — and 47 was consumed; the dump's tail is
        // 116, ..., 51, 50, then 9 (no 53, no 47, no 8).
        assert!(board.is_on_the_board(ItemId::new(116)), "PASS2 id=116");
        assert_eq!(
            corners_of(&board, ItemId::new(116)),
            vec!["48000,20000", "50000,20000", "50000,25000"],
            "NORM4_PASS2_ITEM id=116"
        );
        assert!(
            !board.is_on_the_board(ItemId::new(53)),
            "no PASS2 id=53 row"
        );
        assert!(
            !board.is_on_the_board(ItemId::new(47)),
            "no PASS2 id=47 row"
        );
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 118, "NORM4_NEXT_ID 118");
    }

    /// **NORM5A — the no-op normalize** (capture `--NORM5A--`
    /// :225-227): a lone trace with no contacts — split size 1,
    /// combine false, not degenerate → `NORM5A_NORMALIZE false`,
    /// alive intact.
    #[test]
    fn norm5a_normalize_noop() {
        let (mut manager, mut board) = fresh();
        let k = insert(
            &mut manager,
            &mut board,
            &[(60_000, 35_000), (65_000, 35_000)],
        );
        assert_eq!(k.get(), 10, "NORM5A K=10");
        assert!(
            !normalize(&mut manager, &mut board, k, None),
            "NORM5A_NORMALIZE false"
        );
        assert!(board.is_on_the_board(k), "NORM5A_ALIVE true");
        assert_eq!(
            corners_of(&board, k),
            vec!["60000,35000", "65000,35000"],
            "NORM5A_ALIVE corners"
        );
    }

    /// **NORM5B — the drill split under normalize** (capture
    /// `--NORM5B--` :228-242): the trace crosses via 5's center; the
    /// drill branch DISCARDS its split result, so splitPieces is just
    /// [receiver] (size 1) and the verdict is FALSE even though the
    /// board changed: `NORM5B_NORMALIZE false`,
    /// `NORM5B_D_ALIVE false`, `NORM5B_NEXT_ID 13`.
    #[test]
    fn norm5b_drill_split_under_normalize() {
        let (mut manager, mut board) = fresh();
        let d = insert(
            &mut manager,
            &mut board,
            &[(30_000, 40_000), (50_000, 40_000)],
        );
        assert_eq!(d.get(), 10, "NORM5B D=10");
        assert!(
            !normalize(&mut manager, &mut board, d, None),
            "NORM5B_NORMALIZE false"
        );
        assert!(!board.is_on_the_board(d), "NORM5B_D_ALIVE false");
        assert_eq!(
            corners_of(&board, ItemId::new(11)),
            vec!["30000,40000", "40000,40000"],
            "NORM5B_POST_ITEM id=11"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(12)),
            vec!["40000,40000", "50000,40000"],
            "NORM5B_POST_ITEM id=12"
        );
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 13, "NORM5B_NEXT_ID 13");
    }

    /// **NORM5C — the drill split at the FIRST entry** (capture
    /// `--NORM5C--` :243-258): with the new via 11 at (45000,40000)
    /// present, the drill branch fires on the FIRST overlapping
    /// entry; now the receiver's own split loop ALSO runs, the split
    /// result has two pieces and the verdict is TRUE:
    /// `NORM5C_NORMALIZE true`, `NORM5C_D_ALIVE false`,
    /// `NORM5C_NEXT_ID 14`.
    #[test]
    fn norm5c_first_entry_drill_via() {
        let (mut manager, mut board) = fresh();
        let d = insert(
            &mut manager,
            &mut board,
            &[(30_000, 40_000), (50_000, 40_000)],
        );
        assert_eq!(d.get(), 10, "NORM5C D=10");
        let via2 = insert_via_fixture(
            &mut manager,
            &mut board,
            IntPoint::new(45_000, 40_000),
            &[1],
        );
        assert_eq!(via2.get(), 11, "NORM5C via2=11");
        assert!(
            normalize(&mut manager, &mut board, d, None),
            "NORM5C_NORMALIZE true"
        );
        assert!(!board.is_on_the_board(d), "NORM5C_D_ALIVE false");
        assert_eq!(
            corners_of(&board, ItemId::new(12)),
            vec!["30000,40000", "45000,40000"],
            "NORM5C_POST_ITEM id=12"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(13)),
            vec!["45000,40000", "50000,40000"],
            "NORM5C_POST_ITEM id=13"
        );
        let witness = board.alloc_id();
        assert_eq!(witness.get(), 14, "NORM5C_NEXT_ID 14");
    }

    /// **DEPTH CAP — the boundary is exclusive** (`PolylineTrace-
    /// Normalization.java:26-41`): `depth > 16` returns false with the
    /// board untouched, depth 16 still normalizes. No SPIKE capture can
    /// reach the guard — the cap's only trace is Java's
    /// `FRLogger.debug`, which JUL drops below the console level (same
    /// discipline as the brief's depth-cap clause) — so this pin calls
    /// the private [`normalize_rec`] seam directly on the NORM3 board,
    /// which has a pending merge at any depth. Kills both mutations
    /// (`>` -> `>=`: depth 16 must still work; guard deleted: depth 17
    /// must not).
    #[test]
    fn normalization_depth_cap_is_exclusive() {
        assert_eq!(MAX_NORMALIZATION_DEPTH, 16, "Java's literal cap");
        // depth 16: still under the cap — the pending merge happens.
        let (mut manager, mut board) = fresh();
        let a = norm3_setup(&mut manager, &mut board);
        assert!(
            normalize_rec(&mut manager, &mut board, a, None, MAX_NORMALIZATION_DEPTH),
            "depth 16 still normalizes (guard is strictly greater)"
        );
        // The capped cascade is one recursion level SHALLOWER than at
        // depth 0 (NORM3): the inner normalize at depth 17 is capped,
        // so the area-corner step (piece 18 at depth 0) never fires.
        // Probe-verified depth-16 end state: 13=[35000,10000
        // 35000,20000], 14=[35000,20000 35000,30000] (C split),
        // 15=[30000,20000 35000,20000], 16=[35000,20000 50000,20000]
        // (A's split merged with B), NEXT_ID 17.
        assert_eq!(
            corners_of(&board, ItemId::new(16)),
            vec!["35000,20000", "50000,20000"],
            "depth-16 merge state (one level short of NORM3)"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(15)),
            vec!["30000,20000", "35000,20000"],
            "depth-16 piece 15"
        );
        assert!(!board.is_on_the_board(ItemId::new(11)), "B merged away");
        assert!(!board.is_on_the_board(ItemId::new(10)), "A split away");
        assert_eq!(board.alloc_id().get(), 17, "depth-16 burned 12..16");
        // depth 17: the cap — false, and the board is untouched.
        let (mut manager, mut board) = fresh();
        let a = norm3_setup(&mut manager, &mut board);
        assert!(
            !normalize_rec(
                &mut manager,
                &mut board,
                a,
                None,
                MAX_NORMALIZATION_DEPTH + 1
            ),
            "depth 17 returns false"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(11)),
            vec!["40000,20000", "50000,20000"],
            "B untouched by the capped call"
        );
        assert_eq!(board.alloc_id().get(), 13, "no ids burned past the cap");
    }

    /// **SPLIT_POINT NONE — the off-trace refusal** (`PolylineTrace-
    /// .split(Point)` :699-712 tail): a point on NO segment falls
    /// through the segment loop and Java returns null; the board is
    /// untouched. (No capture section — the spike exercises the allow
    /// paths; this pins the loop-through tail.)
    #[test]
    fn split_at_point_off_trace_refusal() {
        let (mut manager, mut board) = fresh();
        let a = insert(
            &mut manager,
            &mut board,
            &[(10_000, 10_000), (30_000, 10_000)],
        );
        assert_eq!(a.get(), 10, "A=10");
        assert!(
            split_at_point(&mut manager, &mut board, a, &ip(50_000, 50_000)).is_none(),
            "off-trace point -> None"
        );
        assert!(board.is_on_the_board(a), "A untouched");
        assert_eq!(
            corners_of(&board, a),
            vec!["10000,10000", "30000,10000"],
            "A corners unchanged"
        );
        assert_eq!(board.alloc_id().get(), 11, "no ids burned");
    }

    /// **PICK — the `pick_items` rows** (capture `--PICK--`, preserved
    /// log :259-264). The point query is the surrounding-box shape at
    /// the given layer (0), or ALL layers at -1; the result walks
    /// DESCENDING id. `PICK_EMPTY` pins the layer-0 outline-corner
    /// miss down to the bare outline.
    #[test]
    fn pick_items_point_layers() {
        let (manager, mut board) = fresh();
        assert_eq!(
            raw(&manager.pick_items(&mut board, &ip(20_000, 40_000), 0)),
            vec![4, 3],
            "PICK_PIN_CENTER [4:T,3:P]"
        );
        assert_eq!(
            raw(&manager.pick_items(&mut board, &ip(40_000, 40_000), 0)),
            vec![5],
            "PICK_VIA_CENTER [5:V]"
        );
        assert_eq!(
            raw(&manager.pick_items(&mut board, &ip(55_000, 15_000), 0)),
            vec![7, 6],
            "PICK_AREA [7:T,6:A]"
        );
        assert_eq!(
            raw(&manager.pick_items(&mut board, &ip(20_000, 40_000), -1)),
            vec![4, 3],
            "PICK_ALL_LAYERS [4:T,3:P]"
        );
        assert_eq!(
            raw(&manager.pick_items(&mut board, &ip(0, 0), 0)),
            vec![1],
            "PICK_EMPTY [1:BO]"
        );
    }
}
