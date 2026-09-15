//! The per-net normalize DRIVER — Java `BasicBoard.normalizeAllTraces`
//! (`BasicBoard.java:799-885`), the read-path call site
//! `io/specctra/parser/Wiring.java:343-353` (end of the wiring scope,
//! inside a try/catch whose EXCEPTION path pushes
//! `"Wiring: normalization of traces failed"` into
//! `scopeParameter.warnings` — a REAL `warnings.add` site. No fixture
//! throws there (the D12 WARN_COUNT parity has been green throughout);
//! the port has no panic path in this module by design and does NOT
//! emit that warning).
//!
//! The loop (Java `:800-885`, ported 1:1):
//!
//! 1. **Grouping pass** — a DESCENDING-id walk of the item list
//!    (`itemList.startReadObject()`, the `ConcurrentSkipListMap` order —
//!    the board arena's default iteration here), collecting every
//!    `instanceof PolylineTrace && isOnTheBoard()` item ONCE PER net
//!    number of `netNumbers` into `Map<Integer, List<PolylineTrace>>`.
//!    `instanceof PolylineTrace` is [`ItemData::Trace`] (the port's only
//!    trace kind — every board trace is a polyline trace).
//! 2. **Per net**, `while (somethingChanged)`: the iteration cap
//!    [`MAX_NORMALIZE_ITERATIONS`] fires LOG-ONLY (Java
//!    `FRLogger.warn`, `:833-845` — NOT a `warnings.add` site; D12
//!    parity is unaffected and the port emits nothing). Per trace of
//!    the group, if it is still on the board:
//!    `normalize(null)` true → changed; ELSE IF `!isUserFixed() &&
//!    removeIfCycle(trace)` true → changed (the `&&` short-circuits —
//!    `removeIfCycle` runs only for non-user-fixed traces; note the
//!    gate is `isUserFixed` ONLY — the `isDeletionForbidden` gate lives
//!    inside split/normalize, and a `(type protect)` trace therefore
//!    refuses normalize internally while the driver skips
//!    removeIfCycle for it via this gate alone). `normalize(null)` is
//!    [`trace_ops::normalize`] with `clip = None` — the null clip
//!    lifts the split's segment-bbox restriction
//!    (`PolylineTrace.normalize(IntOctagon)` `:801-803` delegates to
//!    `PolylineTraceNormalization.normalize(this, null)`).
//! 3. If the iteration changed something, re-collect THIS net's group
//!    only (descending walk again, `:860-882`, gated
//!    `containsNet(netNumber) && instanceof PolylineTrace &&
//!    isOnTheBoard()` — a multi-net trace re-enters the group once even
//!    though it was grouped once per net). The Java re-collect walk is
//!    wrapped in a `catch (ConcurrentModificationException)` retry
//!    (`:868-874`; the grouping walk has the same guard at
//!    `:806-813`) — STRUCTURALLY UNREACHABLE: no mutation happens
//!    inside either collect walk (all mutation is in the per-trace
//!    loop between walks). Java-defensive; the port re-collects
//!    plainly.
//!
//! ## Across-net processing order (the HashMap trap, resolved)
//!
//! Java groups into a `HashMap<Integer, ...>` and processes
//! `tracesByNet.entrySet()` — an arbitrary order. The port uses a
//! [`BTreeMap`] (ascending net number). The order is UNOBSERVABLE
//! through board state: every effect of a net's loop stays inside that
//! net — `split` skips `!sharesNet` items
//! (`PolylineTrace.java:513-515`), `combine` requires `netsEqual`,
//! contacts are queried with `ignoreNet = false` (same-net only), and
//! `removeIfCycle` walks `getConnectionItems` over those same normal
//! contacts. A multi-net trace (nets `[1,2]`) appears in two groups,
//! but each group's processing of it is idempotent w.r.t. the other
//! net's work — the normalizeall spike's isolation witness
//! (`/tmp/epic-t13-normalize-all.out` `CA2_AGREE asc=true desc=true`:
//! ascending-order and descending-order group processing end on the
//! byte-identical board, and the real Java driver — whose small-Integer
//! HashMap iterates ascending — agrees with both). If an observable
//! cross-net channel ever appears, this module is WRONG and must stop.

use std::collections::BTreeMap;

use crate::board::Board;
use crate::id::ItemId;
use crate::items::ItemData;
use crate::trace_ops::{contains_net, is_user_fixed, normalize, remove_if_cycle};
use crate::tree_manager::SearchTreeManager;

/// Java `BasicBoard.MAX_NORMALIZE_ITERATIONS` (`BasicBoard.java:64`).
///
/// The cap check is STRICTLY GREATER (`++iterationCount >
/// MAX_NORMALIZE_ITERATIONS`, `:833`): the 2000th iteration still runs.
/// Unreachable from real work by the same argument as the T12
/// normalization depth cap: every iteration either changes the board
/// (a split/combine/cycle-removal that converges the net toward its
/// normalized state) or the loop exits; hitting the cap requires a
/// split→combine→split oscillation the combine T-junction guards
/// preclude. It is a hang guard, not a semantic step. Unlike the T12
/// depth cap there is no private recursion seam to probe at the
/// boundary (the counter lives in this module's only function), so the
/// `>` discipline is pinned through [`iteration_cap_reached`].
pub const MAX_NORMALIZE_ITERATIONS: i32 = 2000;

/// The loop's cap test, extracted so the STRICTLY-GREATER comparison
/// has a pin (the 2000th iteration still runs; the 2001st is the first
/// refused).
#[inline]
fn iteration_cap_reached(iteration_count: i32) -> bool {
    iteration_count > MAX_NORMALIZE_ITERATIONS
}

/// Java `BasicBoard.normalizeAllTraces()` (`:799-885`) — normalizes the
/// traces of all nets; returns whether ANYTHING changed on the whole
/// board.
///
/// The caller owns the search-tree fill: the read-path equivalent is
/// [`Board::from_ses_board`] followed by
/// [`SearchTreeManager::insert_items_creation_order`] (the ASCENDING
/// creation-order fill — Java inserts every parsed item at creation
/// time; the DESCENDING rebuild fill produces a different tree
/// skeleton and different split/contact query orders, which is the
/// T11 id-churn trap).
pub fn normalize_all_traces(manager: &mut SearchTreeManager, board: &mut Board) -> bool {
    let mut result = false;
    // The grouping pass (:800-824): descending walk, once per net
    // number. BTreeMap = ascending net order (module docs: the Java
    // HashMap order is unobservable).
    let mut traces_by_net: BTreeMap<i32, Vec<ItemId>> = BTreeMap::new();
    for entry in board.iter_descending() {
        if !matches!(entry.data, ItemData::Trace { .. }) || !entry.on_the_board {
            continue;
        }
        for net in &entry.nets {
            traces_by_net.entry(*net).or_default().push(entry.id);
        }
    }

    for (net_number, mut net_traces) in traces_by_net {
        let mut something_changed = true;
        let mut iteration_count: i32 = 0;
        while something_changed {
            iteration_count += 1;
            if iteration_cap_reached(iteration_count) {
                // Java `:834-844`: FRLogger.warn with the net name —
                // LOG-ONLY (D12), no suppression flag (contrast the
                // SIBLING per-net `normalizeTraces(int)` `:710-798`,
                // which suppresses repeat oscillations and is called
                // only from the autorouter/optimizer —
                // FoundConnectionInserter.java:108, TraceTightener.java:
                // 452 — NOT from the read path; anchor-noted, not
                // ported).
                break;
            }
            something_changed = false;

            for trace_id in &net_traces {
                if !board.is_on_the_board(*trace_id) {
                    continue;
                }
                if normalize(manager, board, *trace_id, None) {
                    something_changed = true;
                    result = true;
                } else {
                    // Java reads `currentTrace.isUserFixed()` off the
                    // (possibly just-removed) object; a gone entry maps
                    // to "not user-fixed" here and the removeIfCycle
                    // on-board guard returns false exactly as Java's
                    // `!trace.isOnTheBoard()` first line does.
                    let not_user_fixed = board
                        .get(*trace_id)
                        .is_none_or(|entry| !is_user_fixed(entry));
                    if not_user_fixed && remove_if_cycle(manager, board, *trace_id) {
                        something_changed = true;
                        result = true;
                    }
                }
            }

            // If something changed, collect the traces for THIS net
            // again (:860-882) — this net only; descending walk.
            if something_changed {
                net_traces.clear();
                for entry in board.iter_descending() {
                    if matches!(entry.data, ItemData::Trace { .. })
                        && entry.on_the_board
                        && contains_net(&entry.nets, net_number)
                    {
                        net_traces.push(entry.id);
                    }
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::FixedState;
    use crate::test_util::{parse_board_from_path, parse_board_from_text};
    use crate::trace_ops::insert_trace_without_cleaning;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::point::Point;
    use epic_geometry::polyline::Polyline;

    /// The crafted board — byte-identical to `PURE_DSN` in
    /// `rust/harness/oracle/NormalizeAllSpike.java` (the T13 capture,
    /// `/tmp/epic-t13-normalize-all.out`; two runs diff clean). Parse
    /// items: 1 = outline, 2 = keepout, 3 = pin at (20000,40000) net
    /// OTHER, 4 = trace (20000,40000)-(10000,40000) net MINE hw 125,
    /// 5 = via (40000,40000), 6 = conduction area 50000-60000 x
    /// 10000-20000, 7/8/9 = traces. The in-read
    /// `normalizeAllTraces()` (Wiring.java:343-353) leaves it
    /// untouched. Post-parse insertions continue at id 10.
    const PURE_DSN: &str = "\
(pcb t13-pure.dsn\n\
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

    fn poly(corners: &[Point]) -> Polyline {
        Polyline::from_points(corners)
    }

    /// Parses the PURE board, fills the trees the READ-path way
    /// (creation order — the T11 two-path record), and returns
    /// (manager, board).
    fn fresh() -> (SearchTreeManager, Board) {
        let mut board = parse_board_from_text(PURE_DSN);
        assert_eq!(board.item_count(), 9, "parse ids 1..9");
        let mut manager = SearchTreeManager::new();
        manager.insert_items_creation_order(&mut board);
        (manager, board)
    }

    /// The spike's `ins`/`insFixed`/`insNet`: template = parse trace 4
    /// (layer, half width, class, nets — the DSN wire width 250 is the
    /// FULL width, the template carries 125), inserted through
    /// [`insert_trace_without_cleaning`] so the generator hands out the
    /// same ids as the capture (10, 11, ...).
    fn insert_like_spike(
        manager: &mut SearchTreeManager,
        board: &mut Board,
        corners: &[Point],
        nets: &[i32],
        fixed: FixedState,
    ) -> Option<ItemId> {
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
            fixed,
        )
    }

    fn corners_of(board: &Board, id: ItemId) -> Vec<String> {
        board
            .trace_polyline(id)
            .expect("trace exists")
            .corners()
            .iter()
            .map(|corner| match corner {
                Point::Int(point) => format!("{},{}", point.x, point.y),
                Point::Rational(_) => unreachable!("integer-board fixtures"),
            })
            .collect()
    }

    fn trace_ids(board: &Board) -> Vec<u32> {
        board
            .iter_descending()
            .filter(|entry| entry.on_the_board && matches!(entry.data, ItemData::Trace { .. }))
            .map(|entry| entry.id.get())
            .collect()
    }

    /// Java's `board.getItems().size()` — LIVE items only (the oracle's
    /// `items` stat source, DsnParseOracle.java:199).
    fn live_item_count(board: &Board) -> usize {
        board
            .iter_descending()
            .filter(|entry| entry.on_the_board)
            .count()
    }

    /// The seam pin for the trap-table cap literal: 2000 exactly, and
    /// the comparison STRICTLY greater (the 2000th iteration still
    /// runs; the 2001st is the first refused). Kills both mutations
    /// (`>` -> `>=` refuses the 2000th; the literal drifting breaks the
    /// first assert).
    #[test]
    fn iteration_cap_is_strictly_greater_than_2000() {
        assert_eq!(MAX_NORMALIZE_ITERATIONS, 2000, "Java's literal cap");
        assert!(
            !iteration_cap_reached(MAX_NORMALIZE_ITERATIONS),
            "the 2000th iteration still runs (strictly greater)"
        );
        assert!(
            iteration_cap_reached(MAX_NORMALIZE_ITERATIONS + 1),
            "the first refused iteration is 2001"
        );
    }

    /// CA1 (`CA1_*` capture rows): the 3-segment collinear chain folds
    /// inside iteration 1 (combine is itself iterative), iteration 2
    /// confirms no-op. Survivor = the DESCENDING-FIRST trace (id 12,
    /// combine's receiver), 10/11 consumed, NEXT_ID 13.
    #[test]
    fn ca1_collinear_chain_folds_to_descending_first_survivor() {
        let (mut manager, mut board) = fresh();
        insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(10000, 45000), ip(20000, 45000)],
            &[1],
            FixedState::Unfixed,
        );
        insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(20000, 45000), ip(30000, 45000)],
            &[1],
            FixedState::Unfixed,
        );
        insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(30000, 45000), ip(40000, 45000)],
            &[1],
            FixedState::Unfixed,
        );
        assert!(
            normalize_all_traces(&mut manager, &mut board),
            "CA1_DRIVER result=true"
        );
        assert_eq!(
            trace_ids(&board),
            vec![12, 9, 8, 7, 4],
            "CA1_AFTER: 12 survives, 10/11 consumed"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(12)),
            vec!["10000,45000", "40000,45000"],
            "CA1_AFTER_ITEM id=12 corners"
        );
        let witness = insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(60000, 35000), ip(65000, 35000)],
            &[1],
            FixedState::Unfixed,
        )
        .expect("witness inserts");
        assert_eq!(witness.get(), 13, "CA1_NEXT_ID 13");
    }

    /// CA2 (`CA2_*` rows): two nets, each a chain, plus the multi-net
    /// pair X,Y (nets [1,2]) — the fold happens in whichever group runs
    /// first; the other group's pass over it is a no-op. The port runs
    /// nets ASCENDING; the capture's twin_desc row (descending) and the
    /// real driver agree on the byte-identical end board — the
    /// isolation witness this module's docs lean on.
    #[test]
    fn ca2_two_nets_and_multi_net_pair_fold_order_independently() {
        let (mut manager, mut board) = fresh();
        insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(10000, 45000), ip(20000, 45000)],
            &[1],
            FixedState::Unfixed,
        );
        insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(20000, 45000), ip(30000, 45000)],
            &[1],
            FixedState::Unfixed,
        );
        insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(60000, 45000), ip(70000, 45000)],
            &[1, 2],
            FixedState::Unfixed,
        );
        insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(70000, 45000), ip(80000, 45000)],
            &[1, 2],
            FixedState::Unfixed,
        );
        insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(60000, 30000), ip(70000, 30000)],
            &[2],
            FixedState::Unfixed,
        );
        insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(70000, 30000), ip(80000, 30000)],
            &[2],
            FixedState::Unfixed,
        );
        assert!(
            normalize_all_traces(&mut manager, &mut board),
            "CA2_DRIVER result=true"
        );
        assert_eq!(
            trace_ids(&board),
            vec![15, 13, 11, 9, 8, 7, 4],
            "CA2_AFTER: 15/13/11 survive"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(15)),
            vec!["60000,30000", "80000,30000"],
            "CA2_AFTER_ITEM id=15"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(13)),
            vec!["60000,45000", "80000,45000"],
            "CA2_AFTER_ITEM id=13"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(11)),
            vec!["10000,45000", "30000,45000"],
            "CA2_AFTER_ITEM id=11"
        );
        assert_eq!(
            board.get(ItemId::new(13)).expect("13").nets,
            vec![1, 2],
            "CA2_AFTER_ITEM id=13 nets=[1, 2]"
        );
        let witness = insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(60000, 35000), ip(65000, 35000)],
            &[1],
            FixedState::Unfixed,
        )
        .expect("witness inserts");
        assert_eq!(witness.get(), 16, "CA2_NEXT_ID 16");
    }

    /// CA3 (`CA3_*` rows): the 4-trace square is pre-merged by
    /// normalize into ONE CLOSED trace (combine concatenates the
    /// two-trace junctions; the junction points touch nothing else) —
    /// removeIfCycle never fires (a single closed trace is not a
    /// `Trace.isCycle` cycle: its start contacts are pre-visited). The
    /// brief's "removeIfCycle fires" expectation is NOT what the jar
    /// does; the capture is.
    #[test]
    fn ca3_cycle_square_pre_merges_into_one_closed_trace() {
        let (mut manager, mut board) = fresh();
        for corners in [
            [ip(10000, 45000), ip(20000, 45000)],
            [ip(20000, 45000), ip(20000, 55000)],
            [ip(20000, 55000), ip(10000, 55000)],
            [ip(10000, 55000), ip(10000, 45000)],
        ] {
            insert_like_spike(
                &mut manager,
                &mut board,
                &corners,
                &[1],
                FixedState::Unfixed,
            );
        }
        assert!(
            normalize_all_traces(&mut manager, &mut board),
            "CA3_DRIVER result=true"
        );
        assert_eq!(
            trace_ids(&board),
            vec![13, 9, 8, 7, 4],
            "CA3_AFTER: the square is one closed trace, id 13"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(13)),
            vec![
                "10000,45000",
                "20000,45000",
                "20000,55000",
                "10000,55000",
                "10000,45000",
            ],
            "CA3_AFTER_ITEM id=13 corners (closed ring)"
        );
        let witness = insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(60000, 35000), ip(65000, 35000)],
            &[1],
            FixedState::Unfixed,
        )
        .expect("witness inserts");
        assert_eq!(witness.get(), 14, "CA3_NEXT_ID 14");
    }

    /// CA3B (`CA3B_*` rows): the out-and-back loop (both endpoints
    /// shared, clear of every parse trace) ALSO pre-merges into one
    /// closed trace — combine concatenates the two-trace junctions.
    #[test]
    fn ca3b_out_and_back_loop_pre_merges_into_one_closed_trace() {
        let (mut manager, mut board) = fresh();
        insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(60000, 45000), ip(65000, 50000), ip(70000, 45000)],
            &[1],
            FixedState::Unfixed,
        );
        insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(60000, 45000), ip(65000, 40000), ip(70000, 45000)],
            &[1],
            FixedState::Unfixed,
        );
        assert!(
            normalize_all_traces(&mut manager, &mut board),
            "CA3B_DRIVER result=true"
        );
        assert_eq!(
            trace_ids(&board),
            vec![11, 9, 8, 7, 4],
            "CA3B_AFTER: the loop is one closed trace, id 11"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(11)),
            vec![
                "70000,45000",
                "65000,50000",
                "60000,45000",
                "65000,40000",
                "70000,45000",
            ],
            "CA3B_AFTER_ITEM id=11 corners (closed loop)"
        );
        let witness = insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(60000, 35000), ip(65000, 35000)],
            &[1],
            FixedState::Unfixed,
        )
        .expect("witness inserts");
        assert_eq!(witness.get(), 12, "CA3B_NEXT_ID 12");
    }

    /// CA3C (`CA3C_*` rows): the REAL else-branch firing — a trace
    /// whose both endpoints contact the same-net conduction area 6.
    /// Combine cannot merge through an area; split's found-first pass
    /// splits it at trace 7's crossing (ids 11/12 burn) and the pieces
    /// cycle with the area; the driver's `!isUserFixed() &&
    /// removeIfCycle` branch removes every piece. The AREA survives
    /// (the T12 AR1 pin). NEXT_ID 13.
    #[test]
    fn ca3c_area_cycle_trace_removed_by_the_else_branch() {
        let (mut manager, mut board) = fresh();
        let inserted = insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(54000, 15000), ip(58000, 15000)],
            &[1],
            FixedState::Unfixed,
        )
        .expect("inserts");
        assert_eq!(inserted.get(), 10, "CA3C_BEFORE id=10");
        assert!(
            normalize_all_traces(&mut manager, &mut board),
            "CA3C_DRIVER result=true"
        );
        assert_eq!(
            trace_ids(&board),
            vec![9, 8, 7, 4],
            "CA3C_AFTER: the area-cycling trace is gone"
        );
        assert!(
            board.get(ItemId::new(6)).is_some(),
            "CA3C_AFTER: conduction area 6 survives"
        );
        let witness = insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(60000, 35000), ip(65000, 35000)],
            &[1],
            FixedState::Unfixed,
        )
        .expect("witness inserts");
        assert_eq!(witness.get(), 13, "CA3C_NEXT_ID 13");
    }

    /// CA4 (`CA4_*` rows): the same square USER_FIXED — normalize
    /// refuses (combine may not consume a deletion-forbidden trace) and
    /// the driver's `!isUserFixed()` gate skips removeIfCycle: board
    /// unchanged, result FALSE, single iteration.
    #[test]
    fn ca4_user_fixed_square_untouched_result_false() {
        let (mut manager, mut board) = fresh();
        for corners in [
            [ip(10000, 45000), ip(20000, 45000)],
            [ip(20000, 45000), ip(20000, 55000)],
            [ip(20000, 55000), ip(10000, 55000)],
            [ip(10000, 55000), ip(10000, 45000)],
        ] {
            insert_like_spike(
                &mut manager,
                &mut board,
                &corners,
                &[1],
                FixedState::UserFixed,
            );
        }
        assert!(
            !normalize_all_traces(&mut manager, &mut board),
            "CA4_DRIVER result=false"
        );
        assert_eq!(
            trace_ids(&board),
            vec![13, 12, 11, 10, 9, 8, 7, 4],
            "CA4_AFTER: all four traces unchanged"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(10)),
            vec!["10000,45000", "20000,45000"],
            "CA4_AFTER_ITEM id=10"
        );
        let witness = insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(60000, 35000), ip(65000, 35000)],
            &[1],
            FixedState::Unfixed,
        )
        .expect("witness inserts");
        assert_eq!(witness.get(), 14, "CA4_NEXT_ID 14");
    }

    /// CA5 (`CA5_*` rows): the crossing pair — normalize(null) splits
    /// BOTH at the crossing (the T12 surface), the T-junction combine
    /// guard refuses the refold, and the board converges on the
    /// 4-piece split state. NEXT_ID 16.
    #[test]
    fn ca5_crossing_pair_splits_into_four_pieces() {
        let (mut manager, mut board) = fresh();
        insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(25000, 20000), ip(25000, 35000)],
            &[1],
            FixedState::Unfixed,
        );
        insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(20000, 30000), ip(30000, 30000)],
            &[1],
            FixedState::Unfixed,
        );
        assert!(
            normalize_all_traces(&mut manager, &mut board),
            "CA5_DRIVER result=true"
        );
        assert_eq!(
            trace_ids(&board),
            vec![15, 14, 13, 12, 9, 8, 7, 4],
            "CA5_AFTER: ids 10/11 burned, pieces 12-15"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(12)),
            vec!["25000,20000", "25000,30000"],
            "CA5_AFTER_ITEM id=12"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(13)),
            vec!["25000,30000", "25000,35000"],
            "CA5_AFTER_ITEM id=13"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(14)),
            vec!["20000,30000", "25000,30000"],
            "CA5_AFTER_ITEM id=14"
        );
        assert_eq!(
            corners_of(&board, ItemId::new(15)),
            vec!["25000,30000", "30000,30000"],
            "CA5_AFTER_ITEM id=15"
        );
        let witness = insert_like_spike(
            &mut manager,
            &mut board,
            &[ip(60000, 35000), ip(65000, 35000)],
            &[1],
            FixedState::Unfixed,
        )
        .expect("witness inserts");
        assert_eq!(witness.get(), 16, "CA5_NEXT_ID 16");
    }

    /// CA6 (`CA6_*` rows): dsn-0151 itself
    /// (`fixtures/Issue723-CombineStackOverflow.dsn`). The parse's
    /// in-read `normalizeAllTraces()` has already folded the file's
    /// 4,000 collinear wire segments into ONE trace (stats items=2,
    /// traces=1, survivor id 4001 — a 30-corner serpentine, the full
    /// literal row pinned below); the driver run here is the SECOND
    /// call the `post_` golden fields make — a fixpoint (result
    /// false), and the exact survivor row is the golden regen's mass
    /// check.
    #[test]
    fn ca6_dsn_0151_second_call_is_a_fixpoint_on_survivor_4001() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/Issue723-CombineStackOverflow.dsn");
        let mut board = parse_board_from_path(&path.to_string_lossy());
        let mut manager = SearchTreeManager::new();
        manager.insert_items_creation_order(&mut board);
        // The IN-READ call (Wiring.java:343-353) — Java's parse runs it
        // inside the wiring scope, so the captured rows are already the
        // post state. Fold the 4,000 collinear segments now.
        assert!(
            normalize_all_traces(&mut manager, &mut board),
            "in-read fold changed the board"
        );
        // CA6_STATS items=2 pads=0 traces=1 vias=0 nets=1.
        assert_eq!(live_item_count(&board), 2, "CA6_STATS items=2");
        assert_eq!(trace_ids(&board), vec![4001], "CA6_TRACE survivor id=4001");
        // The second call the post_ golden fields make.
        assert!(
            !normalize_all_traces(&mut manager, &mut board),
            "CA6_DRIVER result=false (fixpoint)"
        );
        let survivor = board.trace_polyline(ItemId::new(4001)).expect("survivor");
        // The FULL literal CA6_TRACE_ITEM corner list (capture line 2
        // of the CA6 section): 30 corners — the 4000 collinear FILE
        // segments fold to a 29-segment serpentine plus the tail, NOT
        // a 4001-corner chain (an earlier draft pinned 4001 from the
        // narrative; the capture row is the truth).
        let corners: Vec<String> = survivor
            .corners()
            .iter()
            .map(|corner| match corner {
                Point::Int(point) => format!("{},{}", point.x, point.y),
                Point::Rational(_) => unreachable!("integer-board fixtures"),
            })
            .collect();
        assert_eq!(
            corners,
            vec![
                "1300000,-1070000",
                "1860000,-1070000",
                "1860000,-1068000",
                "1300000,-1068000",
                "1300000,-1066000",
                "1860000,-1066000",
                "1860000,-1064000",
                "1300000,-1064000",
                "1300000,-1062000",
                "1860000,-1062000",
                "1860000,-1060000",
                "1300000,-1060000",
                "1300000,-1058000",
                "1860000,-1058000",
                "1860000,-1056000",
                "1300000,-1056000",
                "1300000,-1054000",
                "1860000,-1054000",
                "1860000,-1052000",
                "1300000,-1052000",
                "1300000,-1050000",
                "1860000,-1050000",
                "1860000,-1048000",
                "1300000,-1048000",
                "1300000,-1046000",
                "1860000,-1046000",
                "1860000,-1044000",
                "1300000,-1044000",
                "1300000,-1042000",
                "1432000,-1042000",
            ],
            "CA6_TRACE_ITEM id=4001 corners (full literal row)"
        );
    }
}
