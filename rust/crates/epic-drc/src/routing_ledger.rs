//! The #933 survivor-3 recount driver (upstream 339e8bb50, Java
//! `NetRoutingLedger`; state in `epic_board::routing_ledger`,
//! notes at `Board::insert_item`/`remove_item`/`set_item_nets`).
//! Java replaced every per-read `DesignRulesChecker` full scan in
//! the autorouter's hot loop (count reads per pass × the batch
//! loop × the optimizer) with: build once, note incrementally,
//! recount DIRTY NETS only.
//!
//! Faces, mirroring Java exactly:
//! - [`ensure_built`]: the first read does the full walk (raw lists
//!   + a per-net count) and caches everything.
//! - [`flush`]: recounts each dirty net (delta-applied to the
//!   total), then ALWAYS recomputes `maximum_connections` — even
//!   with nothing dirty (Java `flushDirty`).
//! - [`maximum_connections`]: builds but does NOT flush — the value
//!   can be stale until a count read flushes (Java's own quirk,
//!   pinned below).
//!
//! DIVERGENCES (documented, mirrored on purpose):
//! - **Lists are sized at build time.** A net number allocated after
//!   the build is silently skipped by the notes (Java's
//!   `netNumber < itemsByNet.length` array-bounds guard has the same
//!   face). No routing-loop flow allocates new net numbers.
//! - **An empty dirty net recounts to 0 directly**, without calling
//!   the row counter: `all_incompletes` never feeds the counter an
//!   empty list, and the count of an empty net is 0 by definition
//!   (Java's `NetIncompletes` over an empty list yields 0).
//!
//! The reader swap sites (Java `BatchAutorouter:586-592`,
//! `BatchOptimizer:1145`, `BoardStatistics:297`) are
//! `epic_router::pipeline::batch::calculate_incomplete_count` and
//! the connections block of
//! `epic_router::pipeline::board_statistics` — every consumer
//! routes through those two, so no other call site changes.

use std::collections::BTreeSet;

use epic_board::board::Board;
use epic_board::routing_ledger::RoutingLedgerCore;
use epic_board::tree_manager::SearchTreeManager;

use crate::incompletes::{max_connections, net_incompletes_row, raw_net_item_lists};

/// Java `NetRoutingLedger.incompleteCount` — flush, then the total.
pub fn incomplete_count(manager: &SearchTreeManager, board: &mut Board) -> i32 {
    with_core(manager, board, |core, manager, board| {
        flush(core, manager, board);
        core.incomplete_total
    })
}

/// Java `NetRoutingLedger.incompleteNetNumbers` — flush, then the
/// ascending set of nets with a strictly positive cached count.
pub fn incomplete_net_numbers(manager: &SearchTreeManager, board: &mut Board) -> BTreeSet<i32> {
    with_core(manager, board, |core, manager, board| {
        flush(core, manager, board);
        core.net_numbers_with_incompletes()
    })
}

/// Java `NetRoutingLedger.maximumConnections` — build WITHOUT
/// flushing: the returned value can be stale until some count read
/// flushes (Java's exact face; pinned in the tests).
pub fn maximum_connections(manager: &SearchTreeManager, board: &mut Board) -> i32 {
    with_core(manager, board, |core, manager, board| {
        ensure_built(core, manager, board);
        core.maximum_connections
    })
}

/// Runs `f` with the ledger core DETACHED from the board — the
/// recount needs `(manager, board)` and the core's lists at once, so
/// the core leaves (an unbuilt default sits in its place until the
/// driver returns it).
fn with_core<R>(
    manager: &SearchTreeManager,
    board: &mut Board,
    f: impl FnOnce(&mut RoutingLedgerCore, &SearchTreeManager, &mut Board) -> R,
) -> R {
    let mut core = board.take_routing_ledger();
    let result = f(&mut core, manager, board);
    board.restore_routing_ledger(core);
    result
}

/// Java `ensureBuilt`: the first read walks the whole board — the
/// raw per-net lists and a per-net count for every non-empty net
/// (empty nets count 0 by definition), plus the maximum.
fn ensure_built(core: &mut RoutingLedgerCore, manager: &SearchTreeManager, board: &mut Board) {
    if core.built {
        return;
    }
    core.items_by_net = raw_net_item_lists(board);
    let mut counts = vec![0i32; core.items_by_net.len()];
    let mut total = 0i32;
    for (index, list) in core.items_by_net.iter().enumerate() {
        if list.is_empty() {
            continue;
        }
        let count = recount(core, manager, board, index);
        counts[index] = count;
        total = total.saturating_add(count);
    }
    core.incomplete_by_net = counts;
    core.incomplete_total = total;
    core.maximum_connections = max_i32(max_connections(board, &core.items_by_net));
    core.dirty_nets.clear();
    core.built = true;
    core.builds += 1;
}

/// Java `flushDirty`: recount every dirty net (delta-applied to the
/// total), clear the dirty set, then ALWAYS recompute the maximum
/// (Java recomputes it unconditionally — cheap, and it keeps the
/// endpoint count honest after pin/pour notes).
fn flush(core: &mut RoutingLedgerCore, manager: &SearchTreeManager, board: &mut Board) {
    ensure_built(core, manager, board);
    if !core.dirty_nets.is_empty() {
        let dirty: Vec<i32> = core.dirty_nets.iter().copied().collect();
        core.dirty_nets.clear();
        for net_no in dirty {
            let index = (net_no - 1) as usize;
            let count = recount(core, manager, board, index);
            core.incomplete_total = core
                .incomplete_total
                .saturating_add(count)
                .saturating_sub(core.incomplete_by_net[index]);
            core.incomplete_by_net[index] = count;
            core.net_recounts += 1;
        }
    }
    core.maximum_connections = max_i32(max_connections(board, &core.items_by_net));
}

/// One net's count — empty lists short-circuit to 0 (the divergence
/// note above); the row counter is the SAME face the full scan uses,
/// so a built ledger and a fresh scan are count-identical by
/// construction.
fn recount(
    core: &RoutingLedgerCore,
    manager: &SearchTreeManager,
    board: &mut Board,
    index: usize,
) -> i32 {
    if core.items_by_net[index].is_empty() {
        return 0;
    }
    let net_no = index as i32 + 1;
    let count =
        net_incompletes_row(manager, board, net_no, &core.items_by_net[index]).incomplete_count;
    max_i32(count as i64)
}

fn max_i32(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::incompletes::{airline_segments, all_incompletes};
    use crate::test_util::{net_list, net_no, parse};
    use epic_board::items::{BoardItemType, FixedState};
    use epic_board::trace_ops::insert_trace_without_cleaning;
    use epic_board::trace_shover::remove_items;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::point::Point;
    use epic_geometry::polyline::Polyline;

    /// The T9/T10c locator world (the sibling `last_mile`/`batch`
    /// suites' fixture): nets 33/98 are pin PAIRS (one airline
    /// each), 49/94 single pins. Scan baseline: count 2, max 2.
    fn parse_fixture() -> (SearchTreeManager, Board) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../harness/fixtures/locator-spike/t9_locator45.dsn");
        let text = std::fs::read_to_string(&path).expect("fixture present");
        parse(&text)
    }

    fn pt(x: i32, y: i32) -> Point {
        Point::Int(IntPoint::new(x, y))
    }

    /// The upstream `NetRoutingLedgerTest.assertMatchesFullScan`
    /// idiom, as a superset: the ledger's three faces against a
    /// FRESH full scan, after every mutation. Rows are compared
    /// through their per-net counts (the scan's per-net values are
    /// the rows' `incomplete_count`).
    fn assert_matches_full_scan(manager: &SearchTreeManager, board: &mut Board) {
        let (scan_max, rows) = all_incompletes(manager, board);
        let scan_count: i64 = rows.iter().map(|row| row.incomplete_count as i64).sum();
        let scan_nets: BTreeSet<i32> = rows
            .iter()
            .filter(|row| row.incomplete_count > 0)
            .map(|row| row.net_no)
            .collect();
        assert_eq!(
            i64::from(incomplete_count(manager, board)),
            scan_count,
            "incomplete_count vs full scan"
        );
        assert_eq!(
            i64::from(maximum_connections(manager, board)),
            scan_max,
            "maximum_connections vs full scan"
        );
        assert_eq!(
            incomplete_net_numbers(manager, board),
            scan_nets,
            "incomplete_net_numbers vs full scan"
        );
    }

    /// The equivalence spine + the upstream test's movement arms:
    /// a trace ALONG the net-33 airline connects the pair (count 2→1
    /// with net 98 still open), removing it restores 2; a foreign
    /// net's trace changes nothing; a no-mutation re-read hits the
    /// cache (the perf witness: one build, no recounts).
    #[test]
    fn ledger_matches_full_scan_across_mutations() {
        let (mut manager, mut board) = parse_fixture();
        let net_33 = net_no(&board, "NET_33");
        let net_98 = net_no(&board, "NET_98");
        assert_matches_full_scan(&manager, &mut board);
        assert_eq!(incomplete_count(&manager, &mut board), 2);
        assert_eq!(maximum_connections(&manager, &mut board), 2);
        assert_eq!(
            incomplete_net_numbers(&manager, &mut board),
            BTreeSet::from([net_33, net_98])
        );
        let (builds, recounts) = (
            board.routing_ledger().builds,
            board.routing_ledger().net_recounts,
        );
        assert_eq!(builds, 1, "one build so far");
        assert_eq!(recounts, 0, "clean build, no incremental recounts");

        // The connecting trace: corner-to-corner along the airline.
        let airline = airline_segments(&manager, &mut board)
            .into_iter()
            .find(|airline| airline.net == net_33)
            .expect("the net-33 airline");
        let corners = |v: (i64, i64)| pt(v.0 as i32, v.1 as i32);
        let trace = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&corners(airline.from), &corners(airline.to)),
            0,
            500,
            &[net_33],
            1,
            FixedState::Unfixed,
        )
        .expect("connecting trace inserts");
        assert_matches_full_scan(&manager, &mut board);
        assert_eq!(
            incomplete_count(&manager, &mut board),
            1,
            "net 33 closed, net 98 still open (the upstream 0-after-insert face)"
        );
        assert_eq!(
            incomplete_net_numbers(&manager, &mut board),
            BTreeSet::from([net_98])
        );
        assert_eq!(board.routing_ledger().net_recounts, 1, "one dirty net");

        // Remove it: back to the parse face (the upstream
        // 1-after-removal arm, with net 98's still-open airline).
        assert!(remove_items(&mut manager, &mut board, &[trace]));
        assert_matches_full_scan(&manager, &mut board);
        assert_eq!(incomplete_count(&manager, &mut board), 2);

        // A FOREIGN net's floating trace: net 49's single pin gains a
        // trace (tail-filtered, no count change) — and the cache-hit
        // witness: this read recounts net 49 only.
        let foreign = insert_trace_without_cleaning(
            &mut manager,
            &mut board,
            Polyline::from_two_corners(&pt(900_000, 900_000), &pt(950_000, 900_000)),
            0,
            500,
            &[49],
            1,
            FixedState::Unfixed,
        )
        .expect("foreign trace inserts");
        let recounts_before = board.routing_ledger().net_recounts;
        assert_matches_full_scan(&manager, &mut board);
        assert_eq!(
            board.routing_ledger().net_recounts,
            recounts_before + 1,
            "exactly the dirtied net recounted"
        );
        assert!(remove_items(&mut manager, &mut board, &[foreign]));
        assert_matches_full_scan(&manager, &mut board);

        // No mutation between reads: no new builds, no recounts.
        let (builds, recounts) = (
            board.routing_ledger().builds,
            board.routing_ledger().net_recounts,
        );
        let count = incomplete_count(&manager, &mut board);
        let _ = maximum_connections(&manager, &mut board);
        let _ = incomplete_net_numbers(&manager, &mut board);
        assert_eq!(count, 2);
        assert_eq!(
            (
                board.routing_ledger().builds,
                board.routing_ledger().net_recounts
            ),
            (builds, recounts),
            "clean re-reads are pure cache hits"
        );
    }

    /// The Java no-flush quirk: `maximum_connections` builds but does
    /// NOT flush, so a noted mutation leaves the maximum stale until
    /// a count read flushes. Mutation via `set_item_nets` (the
    /// Rust-only hook): moving net 49's lone pin onto net 33 makes
    /// net 33 a 3-pin net (maximum 2→3 through 1+1+0 → 2+1+0) and
    /// empties net 49.
    #[test]
    fn maximum_is_stale_until_a_count_flushes() {
        let (manager, mut board) = parse_fixture();
        let net_33 = net_no(&board, "NET_33");
        assert_eq!(maximum_connections(&manager, &mut board), 2);
        // Net 49 is a NUMBER-only fact on this fixture (its name is
        // not "NET_49" — the sibling last_mile suite's pinned note),
        // so the pin comes from the raw list by index.
        let moved_pin = raw_net_item_lists(&board)[48]
            .iter()
            .copied()
            .find(|&id| {
                board
                    .get(id)
                    .is_some_and(|entry| entry.board_item_type() == BoardItemType::Pin)
            })
            .expect("net 49 has a pin");
        board.set_item_nets(moved_pin, vec![net_33]);
        assert_eq!(
            maximum_connections(&manager, &mut board),
            2,
            "the Java no-flush face: stale until a flush"
        );
        let _ = incomplete_count(&manager, &mut board);
        assert_eq!(
            maximum_connections(&manager, &mut board),
            3,
            "flushed: net 33 now three endpoints"
        );
        assert_matches_full_scan(&manager, &mut board);
    }

    /// The restore seam: `reset_transient_after_restore` (the port's
    /// invalidate site) drops the cache — the next read rebuilds from
    /// the RESTORED board, equivalence intact.
    #[test]
    fn restore_invalidates_and_rebuilds() {
        let (manager, mut board) = parse_fixture();
        let _ = incomplete_count(&manager, &mut board);
        assert!(board.routing_ledger().is_built());
        let mut restored = board.clone();
        restored.reset_transient_after_restore();
        assert!(!restored.routing_ledger().is_built());
        let mut fresh_manager = SearchTreeManager::new();
        fresh_manager.reinsert_tree_items(&mut restored);
        assert_matches_full_scan(&fresh_manager, &mut restored);
        assert_eq!(restored.routing_ledger().builds, 1, "a fresh build");
    }

    /// A clone with a BUILT core answers from its own state and its
    /// mutations never leak back — the optimizer worker-board model.
    #[test]
    fn worker_clone_answers_from_its_own_ledger() {
        let (manager, mut board) = parse_fixture();
        let net_33 = net_no(&board, "NET_33");
        let _ = incomplete_count(&manager, &mut board);
        let mut worker = board.clone();
        // The worker needs its OWN tree (the optimizer's partitioned
        // executor clones both): contacts — what makes a connecting
        // trace count — are answered through the tree manager.
        let mut worker_manager = SearchTreeManager::new();
        worker_manager.reinsert_tree_items(&mut worker);
        let airline = airline_segments(&manager, &mut board)
            .into_iter()
            .find(|airline| airline.net == net_33)
            .expect("the net-33 airline");
        let corners = |v: (i64, i64)| pt(v.0 as i32, v.1 as i32);
        let trace = insert_trace_without_cleaning(
            &mut worker_manager,
            &mut worker,
            Polyline::from_two_corners(&corners(airline.from), &corners(airline.to)),
            0,
            500,
            &[net_33],
            1,
            FixedState::Unfixed,
        )
        .expect("worker trace inserts");
        assert!(worker.get(trace).is_some_and(|entry| entry.on_the_board));
        assert_eq!(
            incomplete_count(&worker_manager, &mut worker),
            1,
            "the clone's ledger tracked the clone's insert"
        );
        assert_eq!(
            incomplete_count(&manager, &mut board),
            2,
            "the original is untouched"
        );
    }

    /// Emptying a net through removes: the dirty, now-empty net
    /// recounts to 0 (the empty-list short-circuit) and the total
    /// drops with it.
    #[test]
    fn emptied_net_recounts_to_zero() {
        let (mut manager, mut board) = parse_fixture();
        let _ = incomplete_count(&manager, &mut board);
        let pins: Vec<epic_board::id::ItemId> = net_list(&board, "NET_33")
            .into_iter()
            .filter(|&id| {
                board
                    .get(id)
                    .is_some_and(|entry| entry.board_item_type() == BoardItemType::Pin)
            })
            .collect();
        assert_eq!(pins.len(), 2, "the fixture's net-33 pair");
        // Pins are component items — remove_items refuses them, so
        // this drives the arena remove directly (the note fires in
        // remove_item itself) and rebuilds the tree.
        for pin in &pins {
            let _ = board.remove_item(*pin);
        }
        manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        assert_matches_full_scan(&manager, &mut board);
        assert_eq!(
            incomplete_net_numbers(&manager, &mut board),
            BTreeSet::from([net_no(&board, "NET_98")]),
            "net 33 emptied: only net 98 remains open"
        );
    }
}
