//! Java `autoroute/pipeline/BatchOptimizer.java` — the rip-and-reroute
//! optimizer stage that follows the autorouter (M4-T9), ported
//! SEQUENTIAL-DETERMINISTIC: Java evaluates candidates on a fixed
//! thread pool and races them, then adopts one winning board; the port
//! evaluates candidates one at a time in `ReadSortedRouteItems` order
//! and applies the same winner rule the Java CODE states
//! (`winningCandidate == null || res.result.improvedOver(winning)`),
//! which in a one-thread world IS deterministic. The stage is the
//! deterministic analog of the threaded shape, not a byte-port of the
//! race — see "The one-winner mapping" below.
//!
//! ## The stage face
//!
//! Java sequences the stage through `RoutingPipeline.run()`:
//! `runOptimizationStage` gates on `job.routerSettings.getRunOptimizer()`
//! (the `optimizer.enabled` box, default TRUE from `DefaultSettings.java:
//! 178`) AND `!job.thread.isStopRequested()` (the gate at
//! `RoutingPipeline.java:122`, method `:121-134`). SPEC-REVIEW
//! CORRECTION (MIN-2): in Java that full stop is
//! raised only by the max-items path (`AutoroutePassRunner.java:218`)
//! and EXTERNAL stops — the router-side stops (max-passes, stagnation,
//! restore exhaustion) raise AUTO_ROUTER_ONLY
//! (`AutorouteBatchLoop.java:311/349/356/512/543`), which does NOT
//! satisfy the gate, so Java lets the stage RUN on them and preflight
//! guard 1 (incompletes > 0) is what skips it. The port's single shared
//! stop flag collapses the two faces — the same net outcome (no
//! optimization on partially-routed boards), with the one corner where
//! they differ: a fully-routed-but-stopped board runs no-op reroute
//! passes in Java and skips the stage in the port. The CLI wiring owns
//! the gate; this module owns `runBatchLoop` (`:278-547`).
//!
//! ## Banks (SEAM carries the dossier)
//!
//! * **The thread pool is IMPLEMENTED as the RUST-ONLY
//!   `optimizer.threads` opt-in** (M8-T7; Java `:656-686` pools
//!   `maxThreads` workers): the default OFF = the single deterministic
//!   walk (the `maxThreads = 1` analog — the pool floor Java itself
//!   computes for a 1-core host); `n >= 2` = the partitioned candidate
//!   executor in this file (its doc carries the contract + residual).
//! * **`resultMap` + the PRIORITIZED strategy arm are banked**
//!   (`:583-619`): `itemSelectionStrategy` is a `transient` field only
//!   the GUI writes (`GuiBoardManager`); the headless flow always sees
//!   the `strategy == null → SEQUENTIAL` default (`:588-591`), whose
//!   arm IS the plain `ReadSortedRouteItems` walk ported here. The
//!   resultMap bookkeeping feeds only that arm.
//! * **Runtime metrics are banked to the fanout precedent**: Java
//!   tracks per-stage CPU seconds, allocated GB and peak heap MB
//!   (`:333-339`, `:500-517`); the port renders the summary row with
//!   the `0.00 total CPU seconds, 0.00 GB total allocated, and 0.0 MB
//!   peak heap usage` literals (durations stay real).
//! * **The GUI progress faces are banked**: `currentPosition`
//!   (`:927-934`, `:1050-1052` — the "currently optimizing" marker),
//!   the 1000 ms `ProgressThrottler` and the `BoardStatistics` payload
//!   of the board-update events (the port ships the TYPED counters —
//!   Java's optimizer counters carry only `passCount`; the port sets
//!   `phase = "optimizer"` where Java leaves the field null, a
//!   port-rendering deviation documented on the type).
//! * **The stage wall deadline is wall-profile-only** (the fanout
//!   precedent): `timeoutString` parses into a
//!   `System.currentTimeMillis` deadline only behind
//!   `deterministic_budgets = false`; the deterministic profile leaves
//!   it unset.
//! * **The dead `if (stillUnroutedItems && ... && updatedRoutingBoard
//!   == null) {}` block** (`BatchAutorouter.java:271-273`) is dropped —
//!   an empty statement in Java.

use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use epic_board::board::Board;
use epic_board::contacts::{
    all_contacts, end_contacts, item_connected_set_stopping_at_plane, start_contacts,
};
use epic_board::id::ItemId;
use epic_board::items::ItemData;
use epic_board::rules_surf::DEFAULT_CLEARANCE_CLASS;
use epic_board::trace_ops::{
    get_connection_items, is_shove_fixed, is_user_fixed, remove_item_through_repository,
};
use epic_board::trace_shover::combine_traces;
use epic_board::tree_manager::SearchTreeManager;
use epic_geometry::point::Point;
use epic_geometry::rounding::java_round;

use crate::control::AutorouteControl;
use crate::pipeline::batch::{
    BatchSettings, StopFace, TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP, calculate_incomplete_count,
    remove_tails,
};
use crate::pipeline::board_hash::board_hash;
use crate::pipeline::board_history::restore_from_snapshot;
use crate::pipeline::board_statistics::BoardStatistics;
use crate::pipeline::event_sink::BufferedDriverSink;
use crate::pipeline::event_sink::BufferedSinkRow;
use crate::pipeline::event_sink::DriverSink;
use crate::pipeline::fanout::parse_timespan_string;
use crate::pipeline::pass_runner::{RouterCounters, run_single_thread};

// ---------------------------------------------------------------------------
// the resolved optimizer settings (Java settings.OptimizerSettings)
// ---------------------------------------------------------------------------

/// The resolved optimizer-settings group (Java
/// `settings.OptimizerSettings`, seeded by `DefaultSettings.java:177-191`).
/// The nullable-source fields stay `Option` so the read sites reproduce
/// Java's exact null arms (e.g. `:693-700`'s `12`/`50` fallbacks); the
/// two factors are `Integer`/`Float` unboxed at every read site in the
/// merged flow (post-merge never null), so they resolve to scalars.
#[derive(Clone, Debug, PartialEq)]
pub struct OptimizerSettingsIr {
    /// Java `algorithm` (`@SerializedName "algorithm"`); the stage
    /// normalizes any other value to `freerouting-optimizer` with a
    /// warn (`BatchOptimizer.java:93-103`).
    pub algorithm: String,
    /// Java `maxPasses` (`@SerializedName "max_passes"`) — `None` =
    /// unlimited (the `:380` null check).
    pub max_passes: Option<i32>,
    /// Java `maxItems` (`@SerializedName "max_items"`) — `None` =
    /// unlimited.
    pub max_items: Option<i32>,
    /// Java `optimizationImprovementThreshold` (`@SerializedName
    /// "improvement_threshold"`) — the pass-stop percentage, sanitized
    /// at loop start (`:350-373`).
    pub improvement_threshold: Option<f32>,
    /// Java `enablePreflightGuards` (`@SerializedName
    /// "enable_preflight_guards"`). The bypass is
    /// `Boolean.FALSE.equals(...)` (`:156-159`) — ONLY an explicit
    /// false bypasses; unset runs the guards.
    pub enable_preflight_guards: Option<bool>,
    /// Java `maxConsecutiveFailures` (`@SerializedName
    /// "max_consecutive_failures"`) — the early-stop bound from pass 2
    /// on (`:699-702`, null → 50).
    pub max_consecutive_failures: Option<i32>,
    /// Java `maxConsecutiveFailuresPass1` (`@SerializedName
    /// "max_consecutive_failures_pass1"`) — the pass-1 canary bound
    /// (`:695-698`, null → 12).
    pub max_consecutive_failures_pass1: Option<i32>,
    /// Java `additionalRipupCostFactorAtStart` — the increased-ripup
    /// multiplier while [`Self::use_increased`] holds (`:884-887`,
    /// default 10).
    pub additional_ripup_cost_factor_at_start: i32,
    /// Java `traceRipupCostFactor` — the trace-candidate discount
    /// applied AFTER the start factor (`:889-892`, default 0.6).
    pub trace_ripup_cost_factor: f32,
    /// Java `maxAutoroutePasses` — the reroute pass budget per
    /// candidate (`:894-895`: `optimizer != null ? maxAutoroutePasses
    /// : 1`; the merged flow always carries the box, so the resolved
    /// default 6 is the live face).
    pub max_autoroute_passes: i32,
    /// Java `timeoutString` (`@SerializedName "timeout"`) — the stage
    /// wall deadline, wall-profile-only (the module banks).
    pub timeout_string: Option<String>,
}

// `DefaultSettings.java:177-191` — the seed values live in the CLI
// resolver's `MergedSettings::default` (the single authoritative
// defaults source; the settings-merger trap — CLAUDE.md — forbids a
// second one on the IR type, so [`OptimizerSettingsIr`] has no
// `Default` impl).
//
// ---------------------------------------------------------------------------
// the item-route result (Java autoroute.ItemRouteResult)
// ---------------------------------------------------------------------------

/// Java `autoroute.ItemRouteResult` — one candidate's before/after
/// comparison. The `improved` ladder is Java's verbatim
/// (`ItemRouteResult.java:31-59`), INCLUDING its cross-measure quirk:
/// `traceLengthBefore` is the WHOLE BOARD's weighted length
/// (`totalWeightedLength`, `BatchOptimizer.java:633`) while
/// `traceLengthAfter` is the plain `traces.totalLength` — on an
/// all-unfixed board the weighted face dominates, so a candidate that
/// reroutes back to identical geometry still reads "improved" at the
/// equal-incomplete/equal-via rung. That is Java's behavior, not the
/// port's invention; the pass-level acceptance gates (strict score
/// improvement) are what actually discipline the outcome.
#[derive(Clone, Debug)]
pub struct ItemRouteResult {
    /// Java `itemId`.
    pub item_id: i32,
    /// Java `improvementPercentage` (computed in the ctor).
    pub improvement_percentage: f32,
    via_count_before: i32,
    via_count_after: i32,
    trace_length_before: f64,
    trace_length_after: f64,
    incomplete_count_before: i32,
    incomplete_count_after: i32,
    improved: bool,
}

impl ItemRouteResult {
    /// Java's unimproved ctor (`ItemRouteResult.java:18-21`): the
    /// 1-arg form builds with `(0, 0, 0, 0, 0, 1)` and clears the
    /// ladder's verdict.
    #[must_use]
    pub fn unimproved(item_id: i32) -> Self {
        let mut result = Self::new(item_id, 0, 0, 0.0, 0.0, 0, 1);
        result.improved = false;
        result
    }

    /// Java's comparing ctor (`ItemRouteResult.java:23-59`).
    #[must_use]
    #[allow(clippy::too_many_arguments)] // the Java signature, kept 1:1
    pub fn new(
        item_id: i32,
        via_count_before: i32,
        via_count_after: i32,
        trace_length_before: f64,
        trace_length_after: f64,
        incomplete_count_before: i32,
        incomplete_count_after: i32,
    ) -> Self {
        let improved = if incomplete_count_after < incomplete_count_before {
            true
        } else if incomplete_count_after > incomplete_count_before {
            false
        } else if via_count_after < via_count_before {
            true
        } else if via_count_after > via_count_before {
            false
        } else {
            trace_length_after < trace_length_before
        };
        // Java: `viaCountBefore != 0 && traceLengthBefore != 0 ? 1.0 -
        // (((viaCountAfter / viaCountBefore) + (traceLengthAfter /
        // traceLengthBefore)) / 2) : 0`. QUALITY-REVIEW MIN-1 (jar probe
        // `harness/oracle/ItemRouteResultProbe.java`, evidence
        // `probe_ItemRouteResult_pct.log`): the via ratio is JAVA
        // INTEGER division — two `int` fields, `(viaCountAfter /
        // viaCountBefore)` truncates (2→1 gives 0, and 4→2 also gives 0)
        // and promotes only when added to the double length ratio. The
        // jar answers 0.5 on the 2→1/100→100 world (a float-division
        // port answers 0.25) and 0.5 on the 4→2 world (float would give
        // 0.25) — the two worlds together discriminate int from float
        // division in both directions. Rust `/` on `i32` truncates
        // toward zero like Java (via counts are never negative); the
        // length ratio stays real division; the f32 cast happens on the
        // whole expression.
        let improvement_percentage = if via_count_before != 0 && trace_length_before != 0.0 {
            (1.0 - ((f64::from(via_count_after / via_count_before)
                + trace_length_after / trace_length_before)
                / 2.0)) as f32
        } else {
            0.0
        };
        Self {
            item_id,
            improvement_percentage,
            via_count_before,
            via_count_after,
            trace_length_before,
            trace_length_after,
            incomplete_count_before,
            incomplete_count_after,
            improved,
        }
    }

    /// Java `compareTo` (`ItemRouteResult.java:69-91`) — the ordering
    /// the winner rule consumes (incomplete, then via, then length,
    /// then itemId). `Ordering::Equal` on the first three rungs is the
    /// itemId tie.
    #[must_use]
    pub fn compare(&self, other: &Self) -> std::cmp::Ordering {
        self.incomplete_count_after
            .cmp(&other.incomplete_count_after)
            .then(self.via_count_after.cmp(&other.via_count_after))
            .then(self.trace_length_after.total_cmp(&other.trace_length_after))
            .then(self.item_id.cmp(&other.item_id))
    }

    /// Java `improvedOver(r)` = `compareTo(r) < 0`.
    #[must_use]
    pub fn improved_over(&self, other: &Self) -> bool {
        self.compare(other) == std::cmp::Ordering::Less
    }

    /// Java `improved()`.
    #[must_use]
    pub fn improved(&self) -> bool {
        self.improved
    }

    /// Java `updateImproved(boolean)`.
    pub fn update_improved(&mut self, improved: bool) {
        self.improved = improved;
    }

    /// Java `improvementPercentage()`.
    #[must_use]
    pub fn improvement_percentage(&self) -> f32 {
        self.improvement_percentage
    }

    /// Java `viaCount()`.
    #[must_use]
    pub fn via_count(&self) -> i32 {
        self.via_count_after
    }

    /// Java `viaCountBefore()` — the pre-candidate board face (the
    /// per-BOARD via count, not the item's).
    #[must_use]
    pub fn via_count_before(&self) -> i32 {
        self.via_count_before
    }

    /// Java `incompleteCountBefore()`.
    #[must_use]
    pub fn incomplete_count_before(&self) -> i32 {
        self.incomplete_count_before
    }

    /// Java `incompleteCount()`.
    #[must_use]
    pub fn incomplete_count(&self) -> i32 {
        self.incomplete_count_after
    }

    /// Java `traceLength()` — the whole-board `traces.totalLength`
    /// face after the reroute.
    #[must_use]
    pub fn trace_length(&self) -> f64 {
        self.trace_length_after
    }

    /// Java `lengthReduced()`.
    #[must_use]
    pub fn length_reduced(&self) -> f64 {
        self.trace_length_before - self.trace_length_after
    }

    /// Java `viaCountReduced()`.
    #[must_use]
    pub fn via_count_reduced(&self) -> i32 {
        self.via_count_before - self.via_count_after
    }
}

// ---------------------------------------------------------------------------
// the free decision faces (each pinned against Java's arms)
// ---------------------------------------------------------------------------

/// Java `BatchOptimizer.containsOnlyUnfixedTraces` (`:110-117`): every
/// item is an unfixed trace; vacuously true on the empty set.
fn contains_only_unfixed_traces(board: &Board, items: &[ItemId]) -> bool {
    items.iter().all(|&id| {
        board.get(id).is_some_and(|entry| {
            !is_user_fixed(entry) && matches!(entry.data, ItemData::Trace { .. })
        })
    })
}

/// Java `BatchOptimizer.optimizerCandidateRejectionReason` (`:565-581`):
/// completeness and DRC count are vetoes; the optimizer score ranks
/// candidates that pass both — a strictly-better score accepts, equal
/// or worse rejects. `None` = accepted.
#[must_use]
pub fn optimizer_candidate_rejection_reason(
    incumbent_incomplete_count: i32,
    incumbent_clearance_violation_count: i32,
    incumbent_optimizer_score: f32,
    candidate: &BoardStatistics,
    candidate_optimizer_score: f32,
) -> Option<&'static str> {
    let candidate_incomplete = candidate.connections.incomplete_count.unwrap_or(0);
    let candidate_violations = candidate.clearance_violations.total_count.unwrap_or(0);
    if candidate_incomplete > incumbent_incomplete_count {
        return Some("CONNECTIVITY_REGRESSION");
    }
    if candidate_violations > incumbent_clearance_violation_count {
        return Some("DRC_COUNT_REGRESSION");
    }
    // Java `!(candidateScore > incumbentScore)` — the negated form is
    // load-bearing for NaN (a NaN candidate score REJECTS, exactly like
    // Java's partially-ordered float comparison); total_cmp reproduces
    // that truth table (Only Greater is false when either side is NaN).
    if candidate_optimizer_score.total_cmp(&incumbent_optimizer_score)
        != std::cmp::Ordering::Greater
    {
        return Some("OPTIMIZER_SCORE_NOT_IMPROVED");
    }
    None
}

/// Java `BatchOptimizer.areAllViasMandatoryLayerTransitions`
/// (`:225-275`): true iff every non-user-fixed via connects ONLY SMD
/// pins and traces on its first net, at least TWO SMD pins, and those
/// pins sit on at least two DIFFERENT layers (the via is a mandatory
/// layer transition). An empty via set returns FALSE (`:227-229`);
/// a user-fixed via is SKIPPED, so an all-user-fixed via set returns
/// the vacuous TRUE (`:231-233` — Java's quirk, kept).
pub fn are_all_vias_mandatory_layer_transitions(
    manager: &SearchTreeManager,
    board: &mut Board,
) -> bool {
    let vias: Vec<ItemId> = board
        .iter_ascending()
        .filter(|entry| entry.on_the_board && matches!(entry.data, ItemData::Via { .. }))
        .map(|entry| entry.id)
        .collect();
    if vias.is_empty() {
        return false;
    }
    for via_id in vias {
        let (via_is_user_fixed, via_nets) = match board.get(via_id) {
            Some(entry) => (is_user_fixed(entry), entry.nets.clone()),
            None => continue,
        };
        if via_is_user_fixed {
            continue;
        }
        let Some(&first_net) = via_nets.first() else {
            return false;
        };
        let connected =
            item_connected_set_stopping_at_plane(manager, board, via_id, first_net, true);
        let mut smd_pin_layers: Vec<i32> = Vec::new();
        let mut only_smd_pins_and_traces = true;
        for contact in &connected {
            let contact_id = contact.0;
            if contact_id == via_id {
                continue;
            }
            match board.get(contact_id).map(|entry| &entry.data) {
                Some(ItemData::Pin { .. }) => {
                    let layers = (
                        board.item_first_layer(contact_id),
                        board.item_last_layer(contact_id),
                    );
                    match layers {
                        (Some(first), Some(last)) if first == last => {
                            smd_pin_layers.push(first);
                        }
                        _ => {
                            // A multi-layer pin (through-hole) breaks
                            // the "only SMD pins" face.
                            only_smd_pins_and_traces = false;
                            break;
                        }
                    }
                }
                Some(ItemData::Via { .. }) => {
                    only_smd_pins_and_traces = false;
                    break;
                }
                Some(ItemData::Trace { .. }) => {}
                _ => {
                    only_smd_pins_and_traces = false;
                    break;
                }
            }
        }
        if !only_smd_pins_and_traces || smd_pin_layers.len() < 2 {
            return false;
        }
        let first_layer = smd_pin_layers[0];
        if !smd_pin_layers.iter().any(|&layer| layer != first_layer) {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// the candidate reader (Java BatchOptimizer.ReadSortedRouteItems)
// ---------------------------------------------------------------------------

/// The sort key of one candidate (Java's `(FloatPoint minItemCoor, int
/// minItemLayer)` cursor pair). The sentinels are Java's
/// `Integer.MIN_VALUE`/`MAX_VALUE` widened to f64 (exact).
#[derive(Clone, Copy, Debug, PartialEq)]
struct RouteItemKey {
    x: f64,
    y: f64,
    layer: i32,
}

impl RouteItemKey {
    const MIN_X: f64 = i32::MIN as f64;
    const MIN_Y: f64 = i32::MIN as f64;
    const MAX_X: f64 = i32::MAX as f64;
    const MAX_Y: f64 = i32::MAX as f64;

    /// Java `:1091-1094` — the cursor starts below everything.
    fn start() -> Self {
        Self {
            x: Self::MIN_X,
            y: Self::MIN_Y,
            layer: -1,
        }
    }

    /// Java's eligibility test (`:1110-1114`, `:1147-1150`): strictly
    /// past the cursor in (x, y, layer) lexicographic order.
    fn is_past(&self, cursor: &Self) -> bool {
        self.x > cursor.x
            || (self.x == cursor.x
                && (self.y > cursor.y || (self.y == cursor.y && self.layer > cursor.layer)))
    }

    /// Java's strict-better test (`:1115-1119`, `:1151-1155`).
    fn is_better(&self, best: &Self) -> bool {
        self.x < best.x
            || (self.x == best.x
                && (self.y < best.y || (self.y == best.y && self.layer < best.layer)))
    }
}

/// Java `BatchOptimizer.ReadSortedRouteItems.next()` (`:1096-1177`),
/// drained to the full candidate order. Java scans the item list
/// TWICE per next() — vias first, then traces ("Read traces last to
/// prefer vias to traces at the same location", `:1128`) — each pass
/// keeping the single minimum key strictly past the cursor; the trace
/// pass's strict `<` still lets a LOWER-layer trace steal the
/// minimum from a same-(x, y) via (`:1151-1155` — the "preference" is
/// a tie rule, not an absolute). The item walks are the DESCENDING-id
/// `itemList` order (the SEAM walk-order fact), which is what breaks
/// (x, y, layer) ties.
#[must_use]
pub fn read_sorted_route_items(manager: &SearchTreeManager, board: &mut Board) -> Vec<ItemId> {
    // The two scans snapshot the live walk first so the `&mut Board`
    // queries (drill layers, contact unions) stay legal; the snapshot
    // is taken in the same DESCENDING-id `itemList` order and nothing
    // mutates inside the reader (Java re-reads the live list per
    // next(), the board is unchanged between them).
    let vias: Vec<(ItemId, f64, f64, bool)> = board
        .iter_descending()
        .filter(|entry| entry.on_the_board)
        .filter_map(|entry| match &entry.data {
            ItemData::Via { center, .. } => Some((
                entry.id,
                f64::from(center.x),
                f64::from(center.y),
                is_user_fixed(entry),
            )),
            _ => None,
        })
        .collect();
    let traces: Vec<(ItemId, i32, Point, Point, bool)> = board
        .iter_descending()
        .filter(|entry| entry.on_the_board)
        .filter_map(|entry| match &entry.data {
            ItemData::Trace { layer, lines, .. } => {
                let (Some(first), Some(last)) = (lines.first_corner(), lines.last_corner()) else {
                    return None;
                };
                Some((
                    entry.id,
                    *layer,
                    first,
                    last,
                    is_shove_fixed(board, entry.id),
                ))
            }
            _ => None,
        })
        .collect();
    let mut cursor = RouteItemKey::start();
    let mut result = Vec::new();
    loop {
        let mut best_key = RouteItemKey {
            x: RouteItemKey::MAX_X,
            y: RouteItemKey::MAX_Y,
            layer: i32::MAX,
        };
        let mut best_id: Option<ItemId> = None;
        // --- the via pass ---
        for &(id, x, y, user_fixed) in &vias {
            if user_fixed {
                continue;
            }
            let key = RouteItemKey {
                x,
                y,
                layer: board.drill_first_layer(id).unwrap_or(i32::MAX),
            };
            if key.is_past(&cursor) && key.is_better(&best_key) {
                best_key = key;
                best_id = Some(id);
            }
        }
        // --- the trace pass (may steal the minimum only strictly) ---
        for &(id, layer, ref first, ref last, shove_fixed) in &traces {
            if shove_fixed {
                continue;
            }
            let (first, last) = (first.to_float(), last.to_float());
            // Java `:1139-1145`: the compare corner is the
            // lexicographically GREATER endpoint.
            let compare = if first.x < last.x || (first.x == last.x && first.y < last.y) {
                last
            } else {
                first
            };
            let key = RouteItemKey {
                x: compare.x,
                y: compare.y,
                layer,
            };
            if !key.is_past(&cursor) || !key.is_better(&best_key) {
                continue;
            }
            // Java `:1156-1164`: a trace whose contact set (the no-arg
            // `getNormalContacts()` union) holds an UNFIXED via is
            // skipped entirely — its connection is re-routable through
            // the via candidate instead.
            let touches_unfixed_via = all_contacts(manager, board, id).into_iter().any(|contact| {
                board.get(contact).is_some_and(|entry| {
                    matches!(entry.data, ItemData::Via { .. }) && !is_user_fixed(entry)
                })
            });
            if !touches_unfixed_via {
                best_key = key;
                best_id = Some(id);
            }
        }
        match best_id {
            Some(id) => {
                cursor = best_key;
                result.push(id);
            }
            None => return result,
        }
    }
}

// ---------------------------------------------------------------------------
// the stage
// ---------------------------------------------------------------------------

/// Why the stage ended (port-added observability; log-only).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OptimizerOutcome {
    /// Java `passesCompleted` — the number of passes the loop entered.
    pub passes_completed: i32,
    /// Java `isTimedOut()` — the stage wall deadline fired.
    pub is_timed_out: bool,
}

/// The batch optimizer stage (Java `BatchOptimizer` over
/// `runBatchLoop`/`optRoutePass`/`optRouteItemOnBoard`). Construct, then
/// [`Self::run_batch_loop`].
pub struct BatchOptimizerStage<'a> {
    manager: &'a mut SearchTreeManager,
    board: &'a mut Board,
    /// Java `job.routerSettings` + the cached router faces the reroute
    /// shares (score box, pull-tight accuracy, budgets, neck width).
    settings: BatchSettings,
    /// The resolved optimizer group.
    optimizer: OptimizerSettingsIr,
    /// Java `router.thread` — the shared stop face.
    stop: &'a mut StopFace,
    /// Java `useIncreasedRipupCosts` (`:50`) — true until a pass fails
    /// to improve.
    use_increased_ripup_costs: bool,
    /// Java `minCumulativeTraceLength` (`:52`) — the weighted length
    /// captured at pass start.
    min_cumulative_trace_length: f64,
    /// Java `totalItemsOptimized` (`:54`).
    total_items_optimized: i32,
    /// Java `deadlineMs` (`:55`) — wall-profile-only.
    deadline_ms: Option<i64>,
    /// Java `isTimedOut` (`:56`).
    is_timed_out: bool,
    /// Java `bestBoard` (`:57`) — the incumbent snapshot.
    best_board: Option<Board>,
    /// Java `bestScore` (`:58`).
    best_score: f32,
    /// Java `bestIncompleteCount` (`:59`).
    best_incomplete_count: i32,
    /// Java `bestClearanceViolationCount` (`:60`).
    best_clearance_violation_count: i32,
}

impl<'a> BatchOptimizerStage<'a> {
    /// Java `getId()` (`:1054-1057`) — the only algorithm id the stage
    /// supports (`normalizeAlgorithm` resets anything else to it).
    pub const ALGORITHM_ID: &'static str = "freerouting-optimizer";

    /// The construction face (`BatchOptimizer.create`): the algorithm
    /// normalization warn rides at the top of [`Self::run_batch_loop`]
    /// (`:93-103` — Java warns when the configured algorithm differs
    /// and resets the field).
    #[must_use]
    pub fn new(
        manager: &'a mut SearchTreeManager,
        board: &'a mut Board,
        settings: BatchSettings,
        optimizer: OptimizerSettingsIr,
        stop: &'a mut StopFace,
    ) -> Self {
        Self {
            manager,
            board,
            settings,
            optimizer,
            stop,
            use_increased_ripup_costs: true,
            min_cumulative_trace_length: 0.0,
            total_items_optimized: 0,
            deadline_ms: None,
            is_timed_out: false,
            best_board: None,
            best_score: 0.0,
            best_incomplete_count: 0,
            best_clearance_violation_count: 0,
        }
    }

    /// Java `isTimedOut()`.
    #[must_use]
    pub fn is_timed_out(&self) -> bool {
        self.is_timed_out
    }

    /// The deadline consultation (`:385-389`, `:706-711`, `:919-921`).
    fn deadline_passed(&self) -> bool {
        self.deadline_ms
            .is_some_and(|deadline| now_millis() >= deadline)
    }

    /// The M8-T7 per-pass evaluation snapshot: everything the
    /// per-candidate body needs, extracted so the sequential face and
    /// the partitioned executor share ONE candidate path. Every field
    /// is constant during a pass (the sequential face only mutates
    /// `use_increased_ripup_costs` / `deadline_ms`-adjacent state AFTER
    /// the candidate loop), so snapshot-at-pass-start is behaviorally
    /// identical to read-per-candidate.
    fn eval_base(&self) -> CandidateEvalBase {
        CandidateEvalBase {
            settings: self.settings.clone(),
            additional_ripup_cost_factor_at_start: self
                .optimizer
                .additional_ripup_cost_factor_at_start,
            trace_ripup_cost_factor: self.optimizer.trace_ripup_cost_factor,
            use_increased_ripup_costs: self.use_increased_ripup_costs,
            min_cumulative_trace_length: self.min_cumulative_trace_length,
            max_autoroute_passes: self.optimizer.max_autoroute_passes,
            stop_flag: self.stop.flag().cloned(),
            deadline_ms: self.deadline_ms,
        }
    }

    /// Java `restoreIncumbentBoard` (`:553-557`): the incumbent
    /// snapshot becomes the live board by value (the deepCopy twin —
    /// [`restore_from_snapshot`] applies the transient reset and
    /// rebuilds the search trees).
    fn restore_incumbent_board(&mut self) {
        if let Some(best_board) = &self.best_board {
            restore_from_snapshot(self.manager, self.board, best_board);
        }
    }

    /// The winner adoption (`:788-789` `this.board = winningCandidate.
    /// board; this.job.board = this.board`): the worker copy becomes
    /// the live board WHOLESALE — no transient reset (Java adopts the
    /// deserialized-once worker object, not a re-deserialize) — and
    /// the trees are rebuilt for it.
    fn adopt_board(&mut self, worker_board: &Board) {
        *self.board = worker_board.clone();
        *self.manager = SearchTreeManager::new();
        self.manager.reinsert_tree_items(self.board);
    }

    /// Java `runBatchLoop` (`:278-547`). Returns the outcome
    /// (passes + the timeout flag).
    #[allow(clippy::too_many_lines)] // the Java body is one flat walk
    pub fn run_batch_loop(&mut self, sink: &mut dyn DriverSink) -> OptimizerOutcome {
        // Java `normalizeAlgorithm` (`:93-103`) — done in `create()`,
        // i.e. before any stage row; the port emits it as the stage's
        // first row (the port constructs the stage without a sink).
        // Java also WRITES the reset value back into the settings; the
        // port's field is per-stage and re-read nowhere, so the warn
        // alone is the same observable face.
        if self.optimizer.algorithm != Self::ALGORITHM_ID {
            sink.warn(&format!(
                "The algorithm '{}' is not supported by the batch autorouter. The default \
                 algorithm '{}' will be used instead.",
                self.optimizer.algorithm,
                Self::ALGORITHM_ID,
            ));
        }

        // Java `:279-283` — the debug row over the RAW cumulative
        // length (`BoardItemRepository.cumulativeTraceLength`, f64 sum
        // of trace lengths), `Math.round`ed.
        let cumulative_length: f64 = self
            .board
            .iter_descending()
            .filter(|entry| entry.on_the_board)
            .filter_map(|entry| match &entry.data {
                ItemData::Trace { .. } => self
                    .board
                    .trace_polyline(entry.id)
                    .map(epic_geometry::polyline::Polyline::length_approx_total),
                _ => None,
            })
            .sum();
        sink.debug(&format!(
            "Before optimization: Via count: {}, trace length: {}",
            self.board
                .iter_ascending()
                .filter(|entry| entry.on_the_board && matches!(entry.data, ItemData::Via { .. }))
                .count(),
            java_round(cumulative_length),
        ));

        // Java `:285`.
        self.use_increased_ripup_costs = true;

        // Java `:288-297` — the baseline statistics.
        let initial_stats = BoardStatistics::new(self.manager, self.board);
        let initial_router_score = initial_stats.get_router_score(Some(&self.settings.scoring));
        let initial_optimizer_score =
            initial_stats.get_optimizer_score(Some(&self.settings.scoring));
        let initial_incomplete = initial_stats.connections.incomplete_count.unwrap_or(0);
        let initial_violations = initial_stats.clearance_violations.total_count.unwrap_or(0);

        // Java `:299-314` — the preflight bypass.
        if let Some(bypass_reason) = self.evaluate_pre_flight_guards(&initial_stats) {
            sink.info(&format!("Skipping optimization stage: {bypass_reason}."));
            sink.task_state("FINISHED", 0, &board_hash(self.board));
            return OptimizerOutcome {
                passes_completed: 0,
                is_timed_out: false,
            };
        }

        // Java `:316-319` — the incumbent snapshot.
        // M6-T1b slice (storage-only, byte-invariant): compact the
        // item-undo slab ONCE at stage entry so the per-candidate
        // worker deep copies (rust: `opt_route_pass`'s `self.board.clone()`,
        // `:1234` at this writing) stop
        // carrying the routing history's unreachable residue. Java's
        // deepCopy never copies that residue (its `UndoableObjects`
        // skip-list holds only current nodes — the dead clones are
        // GC'd), so the compacted slab is the Java-copy-shaped state;
        // every live read surface (map order, values, levels, links,
        // undo/redo/digest walks) is preserved — the pin at
        // `undo.rs` (`compact_reachable_graph_preserves_digest_and_undo`)
        // pins the contract.
        self.board.compact_item_undo_history();
        self.best_board = Some(self.board.clone());
        self.best_score = initial_optimizer_score;
        self.best_incomplete_count = initial_incomplete;
        self.best_clearance_violation_count = initial_violations;

        sink.info(&format!(
            "Optimization stage started on board '{}'. Baseline router score: \
             {initial_router_score:.2}, optimizer score: {initial_optimizer_score:.2}, \
             incomplete connections: {initial_incomplete}, clearance violations: \
             {initial_violations}.",
            board_hash(self.board),
        ));

        // Java `:334-339` — the session resource baselines (the CPU /
        // allocation faces are banked; the wall clock stays real).
        let session_started = std::time::Instant::now();

        // Java `:341-348` — the stage deadline (wall-profile-only).
        if !self.settings.deterministic_budgets
            && let Some(timeout_string) = &self.optimizer.timeout_string
            && let Some(timeout_seconds) = parse_timespan_string(timeout_string)
        {
            self.deadline_ms = Some(now_millis() + timeout_seconds * 1000);
        }

        // Java `:350-373` — the threshold sanitation (Java writes back
        // into the settings object; the value is read nowhere else in
        // the run, so the local is the same face).
        let mut improvement_threshold = self.optimizer.improvement_threshold;
        if let Some(threshold) = improvement_threshold {
            if threshold.is_nan() || threshold.is_infinite() || threshold < 0.0 {
                sink.warn(&format!(
                    "Invalid optimizer improvement threshold: {threshold:.4}. Resetting to \
                     default {:.2}%.",
                    DEFAULT_OPTIMIZER_IMPROVEMENT_THRESHOLD,
                ));
                improvement_threshold = Some(DEFAULT_OPTIMIZER_IMPROVEMENT_THRESHOLD);
            } else if threshold > 0.0 && threshold < 0.1 {
                let scaled = threshold * 100.0;
                sink.info(&format!(
                    "Optimizer improvement threshold appears to be specified as a fraction \
                     ({threshold:.4}). Auto-scaling to percentage ({scaled:.2}%).",
                ));
                improvement_threshold = Some(scaled);
            }
        }

        // Java `:375-376`.
        sink.task_state("STARTED", 0, &board_hash(self.board));

        let mut score_improvement: f64;
        let mut current_pass: i32 = 0;
        while self
            .optimizer
            .max_passes
            .is_none_or(|max| current_pass < max)
            && self
                .optimizer
                .max_items
                .is_none_or(|max| self.total_items_optimized < max)
            && !self.stop.is_requested()
        {
            if self.deadline_passed() {
                self.is_timed_out = true;
                sink.info(&format!(
                    "Optimizer stage timed out before starting pass #{}",
                    current_pass + 1,
                ));
                break;
            }
            current_pass += 1;

            let score_before_pass = BoardStatistics::new(self.manager, self.board)
                .get_optimizer_score(Some(&self.settings.scoring));

            let current_board_hash = board_hash(self.board);
            sink.task_state("RUNNING", current_pass, &current_board_hash);

            // Java `:399` — the alternation "to create more variations".
            let with_preferred_directions = current_pass % 2 != 0;
            let _ = self.opt_route_pass(current_pass, with_preferred_directions, sink);

            if self.is_timed_out {
                break;
            }

            let pass_stats = BoardStatistics::new(self.manager, self.board);
            let score_after_pass = pass_stats.get_optimizer_score(Some(&self.settings.scoring));
            let rejection_reason = optimizer_candidate_rejection_reason(
                self.best_incomplete_count,
                self.best_clearance_violation_count,
                self.best_score,
                &pass_stats,
                score_after_pass,
            );
            match rejection_reason {
                None => {
                    self.best_score = score_after_pass;
                    self.best_incomplete_count =
                        pass_stats.connections.incomplete_count.unwrap_or(0);
                    self.best_clearance_violation_count =
                        pass_stats.clearance_violations.total_count.unwrap_or(0);
                    self.best_board = Some(self.board.clone());
                }
                Some(reason) => {
                    sink.info(&format!(
                        "Optimizer pass #{current_pass} candidate rejected: {reason}. Restoring \
                         incumbent (optimizer score {:.2}, incomplete connections: {}, clearance \
                         violations: {}).",
                        self.best_score,
                        self.best_incomplete_count,
                        self.best_clearance_violation_count,
                    ));
                    self.restore_incumbent_board();
                }
            }

            // Java `:435-437`: `(double)(scoreAfterPass - scoreBeforePass) / scoreBeforePass`
            // — the difference is CAST to f64 first, the division runs in f64.
            let pass_improvement_fraction = if score_before_pass > 0.0 {
                f64::from(score_after_pass - score_before_pass) / f64::from(score_before_pass)
            } else {
                0.0
            };
            let pass_improvement_percent = pass_improvement_fraction * 100.0;
            let pass_outcome = if score_after_pass > score_before_pass {
                "IMPROVED"
            } else if score_after_pass < score_before_pass {
                "REGRESSED"
            } else {
                "UNCHANGED"
            };
            let pass_improvement_str = if score_before_pass > 0.0 {
                format!("{pass_improvement_percent:.4}%")
            } else {
                "n/a (baseline was 0.00)".to_string()
            };
            sink.info(&format!(
                "Optimizer pass #{current_pass}: optimizer score {score_before_pass:.2} -> \
                 {score_after_pass:.2} ({pass_outcome}, {pass_improvement_str}), router score: \
                 {:.2}, incomplete connections: {}, clearance violations: {}.",
                pass_stats.get_router_score(Some(&self.settings.scoring)),
                pass_stats.connections.incomplete_count.unwrap_or(0),
                pass_stats.clearance_violations.total_count.unwrap_or(0),
            ));

            // Java `:460-466` — the increased-ripup drop.
            if self.use_increased_ripup_costs && score_after_pass <= score_before_pass {
                self.use_increased_ripup_costs = false;
                // Keep the optimizer going with normal ripup costs.
                score_improvement = -1.0;
            } else {
                score_improvement = pass_improvement_percent;
            }

            // Java `:468-478` — the threshold stop.
            if let Some(threshold) = improvement_threshold
                && score_improvement != -1.0
                && score_improvement < f64::from(threshold)
            {
                sink.info(&format!(
                    "Stopping optimizer because the improvement in this pass \
                     ({score_improvement:.4}%) is below the threshold ({threshold:.2}%).",
                ));
                break;
            }
        }

        // Java `:481`.
        // Java `:483-494` — the final best-restore.
        let final_board_score = BoardStatistics::new(self.manager, self.board)
            .get_optimizer_score(Some(&self.settings.scoring));
        if final_board_score < self.best_score && self.best_board.is_some() {
            sink.info(&format!(
                "Restoring best board achieved (score {:.2} vs final {final_board_score:.2}).",
                self.best_score,
            ));
            self.restore_incumbent_board();
        }
        self.best_board = None;

        sink.task_state("FINISHED", current_pass, &board_hash(self.board));

        // Java `:499-546` — the session summary (CPU/GB/heap banked to
        // the fanout literals; duration real).
        let completion_status = if self.is_timed_out {
            "completed with timeout:"
        } else if self.stop.is_requested() {
            "interrupted:"
        } else {
            "completed:"
        };
        let final_stats = BoardStatistics::new(self.manager, self.board);
        sink.info(&format!(
            "Optimization stage {completion_status} Baseline router score: \
             {initial_router_score:.2}, baseline optimizer score: \
             {initial_optimizer_score:.2}, final router score: {:.2}, final optimizer score: \
             {:.2}, completed in {:.2} seconds, using 0.00 total CPU seconds, 0.00 GB total \
             allocated, and 0.0 MB peak heap usage.",
            final_stats.get_router_score(Some(&self.settings.scoring)),
            final_stats.get_optimizer_score(Some(&self.settings.scoring)),
            session_started.elapsed().as_secs_f64(),
        ));

        OptimizerOutcome {
            passes_completed: current_pass,
            is_timed_out: self.is_timed_out,
        }
    }

    /// Java `evaluatePreFlightGuards` (`:155-219`): the skip reason, or
    /// `None` when optimization may run.
    pub fn evaluate_pre_flight_guards(&mut self, stats: &BoardStatistics) -> Option<String> {
        // Java `:156-159` — ONLY an explicit false bypasses.
        if self.optimizer.enable_preflight_guards == Some(false) {
            return None;
        }
        // Guard 1 (`:161-167`): incomplete connections.
        // Java unboxes the boxed `Integer` (`:296`) before the guard
        // call — a null never reaches here alive; `unwrap_or(0)` is
        // unreachable tolerance, not a port decision.
        let incomplete = stats.connections.incomplete_count.unwrap_or(0);
        if incomplete > 0 {
            return Some(format!(
                "the board has {incomplete} unrouted connection(s) (optimizer only runs on \
                 completely routed boards)",
            ));
        }
        let initial_score = stats.get_optimizer_score(Some(&self.settings.scoring));
        // Guard 2 (`:171-190`): zero vias and near-optimal routing.
        if stats.vias.total_count == 0 {
            if initial_score >= 950.0 {
                return Some(format!(
                    "the board has no vias to eliminate and initial optimizer score \
                     ({initial_score:.2}) is already >= 950.00",
                ));
            }
            let length_ok = stats
                .bounds
                .min_trace_length_mm
                .is_some_and(|min| min > 0.0)
                && stats.traces.total_length_mm.is_some_and(|total| {
                    total <= stats.bounds.min_trace_length_mm.unwrap_or(0.0) * 1.05
                });
            if length_ok {
                return Some(format!(
                    "the board has no vias to eliminate and trace length ({:.2} mm) is within \
                     5% of theoretical minimum ({:.2} mm)",
                    stats.traces.total_length_mm.unwrap_or(0.0),
                    stats.bounds.min_trace_length_mm.unwrap_or(0.0),
                ));
            }
        }
        // Guard 3 (`:192-211`): the score ceiling / theoretical optimum.
        if initial_score >= 995.0 {
            return Some(format!(
                "the initial optimizer score ({initial_score:.2}) is already at or near \
                 theoretical maximum (995.00)",
            ));
        }
        let near_min_length = stats
            .bounds
            .min_trace_length_mm
            .is_some_and(|min| min > 0.0)
            && stats.traces.total_length_mm.is_some_and(|total| {
                total <= stats.bounds.min_trace_length_mm.unwrap_or(0.0) * 1.02
            });
        let vias_at_min = match stats.bounds.min_via_count {
            None => true,
            Some(min_via) => stats.vias.total_count <= min_via,
        };
        if near_min_length && vias_at_min {
            return Some(format!(
                "total trace length ({:.2} mm) is already within 2% of the theoretical minimum \
                 ({:.2} mm)",
                stats.traces.total_length_mm.unwrap_or(0.0),
                stats.bounds.min_trace_length_mm.unwrap_or(0.0),
            ));
        }
        // Guard 4 (`:213-216`): every via a mandatory SMD transition.
        if are_all_vias_mandatory_layer_transitions(self.manager, self.board) {
            return Some(
                "all vias on the board are mandatory layer transitions between SMD pins that \
                 cannot be eliminated"
                    .to_string(),
            );
        }
        None
    }

    /// Java `optRoutePass` (`:626-817`) — one sequential sweep over the
    /// candidate order; at most ONE winning candidate is applied. The
    /// return value is Java's `routeImproved` percentage (unused by the
    /// loop caller — the loop recomputes its own pass deltas).
    fn opt_route_pass(
        &mut self,
        pass_no: i32,
        with_preferred_directions: bool,
        sink: &mut dyn DriverSink,
    ) -> f32 {
        let stats_before = BoardStatistics::new(self.manager, self.board);
        let pass_started = std::time::Instant::now();
        let counters = RouterCounters {
            phase: "optimizer".to_string(),
            pass_count: pass_no,
            ..RouterCounters::default()
        };
        sink.board_updated(&counters);
        // The M9 snapshot hook (event_sink.rs): mirrors the
        // `board_updated` fire immediately — the default sink is a
        // no-op, so the parity stream is byte-stable.
        sink.board_snapshot(self.board);

        // Java `:633` — the weighted length is the candidate baseline.
        self.min_cumulative_trace_length = f64::from(total_weighted_trace_length(self.board));

        // Java `:634` `prepareCandidateItems()` — the SEQUENTIAL walk
        // (the PRIORITIZED arm is banked; see the module docs).
        let mut candidate_item_ids = read_sorted_route_items(self.manager, self.board);

        // Java `:636-650` — the maxItems slice.
        if let Some(max_items) = self.optimizer.max_items
            && max_items > 0
        {
            let remaining = max_items - self.total_items_optimized;
            if remaining <= 0 {
                sink.info(&format!(
                    "Max items limit reached ({max_items}). Stopping optimizer."
                ));
                return 0.0;
            }
            if candidate_item_ids.len() > remaining as usize {
                candidate_item_ids.truncate(remaining as usize);
            }
        }

        if candidate_item_ids.is_empty() {
            return 0.0;
        }

        // Java `:662-675` — the trace entry row (the silent perfData
        // timestamp shape is banked; the row rides the trace channel).
        // M8-T7: Java prints its resolved `threadPoolSize` here
        // (`BatchOptimizer.java:663-672`); the default face stays the
        // literal 1.
        let threads = self.settings.optimizer_threads.max(1);
        sink.trace(&format!(
            "BatchOptRoute.opt_route_pass #{pass_no} with {} items, {} vias and {:.2} trace \
             length running on {threads} thread(s).",
            candidate_item_ids.len(),
            stats_before.items.via_count,
            stats_before.traces.total_length,
        ));

        // Java `:693-702` — the early-stop bound.
        let max_consecutive_failures = if pass_no == 1 {
            self.optimizer.max_consecutive_failures_pass1.unwrap_or(12)
        } else {
            self.optimizer.max_consecutive_failures.unwrap_or(50)
        };

        // The M8-T7 per-pass candidate evaluation snapshot: every field
        // is constant during the candidate loop, so one snapshot serves
        // both faces below.
        let base = self.eval_base();

        // MIN-2 (quality review): Java chunks this loop — the deadline
        // and stop are consulted once per CHUNK of `max(4×pool, 8)`
        // (`BatchOptimizer.java:689`, `:706-715`) and each chunk's
        // futures are drained in SUBMISSION order (`:735`). The port
        // consults per candidate (the deterministic analog — Java
        // overshoots a wall deadline by up to a chunk's work, the port
        // notices every candidate), and the drain order IS the candidate
        // order: that is why the winner rule transfers deterministically.
        let mut winning: Option<(ItemRouteResult, Board)> = None;
        let mut stopped_or_timed_out = false;
        let mut consecutive_failures: i32 = 0;

        if threads <= 1 {
            // THE GOLDEN PATH: byte-for-byte the pre-T7 candidate walk.
            for &item_id in &candidate_item_ids {
                if self.deadline_passed() {
                    sink.info("Optimizer stage timed out.");
                    self.is_timed_out = true;
                    stopped_or_timed_out = true;
                    break;
                }
                if self.stop.is_requested() {
                    stopped_or_timed_out = true;
                    break;
                }

                // Java `:1003` — the worker deep copy.
                let mut worker_board = self.board.clone();
                let mut worker_manager = SearchTreeManager::new();
                worker_manager.reinsert_tree_items(&mut worker_board);
                if worker_board.get(item_id).is_none() {
                    // Java `:1004-1007` — the defensive missing-item arm.
                    self.total_items_optimized += 1;
                    continue;
                }

                let result = opt_route_item_body(
                    &base,
                    self.stop,
                    &mut worker_manager,
                    &mut worker_board,
                    item_id,
                    with_preferred_directions,
                    sink,
                );
                self.total_items_optimized += 1;
                // Java `:742` resultMap bookkeeping — banked (PRIORITIZED).

                if result.improved() {
                    consecutive_failures = 0;
                    if winning
                        .as_ref()
                        .is_none_or(|(best, _)| result.improved_over(best))
                    {
                        winning = Some((result, worker_board));
                    }
                } else {
                    consecutive_failures += 1;
                    if consecutive_failures >= max_consecutive_failures {
                        sink.info(&format!(
                            "Stopping optimization pass #{pass_no} early after \
                             {consecutive_failures} consecutive items could not be improved.",
                        ));
                        break;
                    }
                }
            }
        } else {
            // THE PARTITIONED FACE (M8-T7): candidates partition by the
            // FIXED key `item_id mod n` (the charter's key; assignment
            // is a pure function of `(item_id, n)`); every worker
            // evaluates its candidates independently against the STABLE
            // pass-start base — the board is never mutated during the
            // loop, so unlike the M5-T7 batch walk there are no
            // conflicted points to serialize. Results reduce in
            // candidate-walk order (the golden order below), so the
            // board state is thread-count-invariant by construction —
            // UNDER DETERMINISTIC BUDGETS with none of the three racy
            // faces below live (see the residual note): the winner is
            // `argmin ItemRouteResult::compare` over the included
            // improved candidates, and `compare` is a total order whose
            // final rung is the unique `item_id` — unique minimum,
            // grouping-invariant (logs/M8-T7/investigation.md §2).
            // Sink rows ride per-candidate buffers and are flushed (or
            // discarded, past an early break) in the same walk,
            // reproducing the sequential stream.
            //
            // STOP/DEADLINE RESIDUAL (the racy faces — condensed from
            // investigation.md §2, what the carry-back and body-doc
            // pointers promise): three faces consult stop/deadline at
            // EVALUATION time on the worker — a live wall-clock
            // deadline, an external stop on the shared flag, and a
            // max-items gate raised INSIDE a candidate's reroute. Under
            // any of them worker results are evaluation-time racy; the
            // worst case: a worker-side `request_full` STORES THE
            // SHARED flag (`StopFace::request` writes it, batch.rs),
            // which mid-flight workers consult — so the perturbation is
            // scheduling-dependent and not even run-to-run stable. The
            // pinned and gated faces run deadline-free with no stop
            // (deterministic budgets), where all three faces are
            // constant and this face is byte-identical to the
            // sequential one. Java's own worker pool races the same
            // faces (`OptimizeCandidateTask` holds `thread` +
            // `deadlineMs`); this is ported parity, not a port
            // invention.
            let partition_count = threads;
            let mut partitions: Vec<Vec<(usize, ItemId)>> =
                (0..partition_count).map(|_| Vec::new()).collect();
            for (idx, &item_id) in candidate_item_ids.iter().enumerate() {
                let p = optimizer_partition_of(item_id, partition_count);
                partitions[p].push((idx, item_id));
            }
            let trace_enabled = sink.is_trace_enabled();

            struct CandidateRecord {
                idx: usize,
                result: ItemRouteResult,
                missing: bool,
                raised_stop: bool,
                rows: Vec<BufferedSinkRow>,
            }

            let base_ref: &CandidateEvalBase = &base;
            let base_board: &Board = self.board;
            let mut result_rxs = Vec::with_capacity(partition_count);
            // Per-partition fold results: each worker keeps only ITS
            // argmin board alongside the current candidate's clone —
            // ~2N+1 boards in flight (each worker: 1 current clone + 1
            // running partition-argmin; the coordinator drains ≤N
            // partition-best boards after the scope joins), and the
            // global winner is always one of them (the §2
            // grouping-invariance argument). The §3 cap arithmetic
            // stays valid: the bm01 N=4 run peaked at 467.4 MiB.
            let mut partition_best_rxs = Vec::with_capacity(partition_count);
            std::thread::scope(|scope| {
                for part in &partitions {
                    let (result_tx, result_rx) = std::sync::mpsc::channel::<Vec<CandidateRecord>>();
                    let (best_tx, best_rx) =
                        std::sync::mpsc::channel::<Option<(ItemRouteResult, Board)>>();
                    result_rxs.push(result_rx);
                    partition_best_rxs.push(best_rx);
                    scope.spawn(move || {
                        let mut records: Vec<CandidateRecord> = Vec::with_capacity(part.len());
                        let mut best: Option<(ItemRouteResult, Board)> = None;
                        for &(idx, item_id) in part {
                            // Java `:1003` — the worker deep copy (each
                            // candidate evaluates a private clone of the
                            // stable base, exactly as the sequential face).
                            let mut worker_board = base_board.clone();
                            let mut worker_manager = SearchTreeManager::new();
                            worker_manager.reinsert_tree_items(&mut worker_board);
                            if worker_board.get(item_id).is_none() {
                                // Java `:1004-1007` — the defensive
                                // missing-item arm.
                                records.push(CandidateRecord {
                                    idx,
                                    result: ItemRouteResult::unimproved(
                                        i32::try_from(item_id.get()).unwrap_or(i32::MAX),
                                    ),
                                    missing: true,
                                    raised_stop: false,
                                    rows: Vec::new(),
                                });
                                continue;
                            }
                            let mut buffer = BufferedDriverSink::new(trace_enabled);
                            let mut worker_stop = StopFace::from_flag(base_ref.stop_flag.clone());
                            let result = opt_route_item_body(
                                base_ref,
                                &mut worker_stop,
                                &mut worker_manager,
                                &mut worker_board,
                                item_id,
                                with_preferred_directions,
                                &mut buffer,
                            );
                            // The carry-back of a max-items raise this
                            // worker's private face performed during the
                            // candidate's reroute (the shared-flag face is
                            // out of the byte contract — the executor doc).
                            let raised_stop = worker_stop.is_locally_raised();
                            if result.improved()
                                && best.as_ref().is_none_or(|(best_result, _)| {
                                    result.improved_over(best_result)
                                })
                            {
                                best = Some((result.clone(), worker_board));
                            }
                            records.push(CandidateRecord {
                                idx,
                                result,
                                missing: false,
                                raised_stop,
                                rows: buffer.rows,
                            });
                        }
                        let _ = result_tx.send(records);
                        let _ = best_tx.send(best);
                    });
                }
            });
            let mut all_records: Vec<CandidateRecord> = Vec::new();
            let mut partition_boards: Vec<Option<Board>> =
                (0..partition_count).map(|_| None).collect();
            for (p, rx) in result_rxs.iter().enumerate() {
                match rx.recv() {
                    Ok(records) => {
                        partition_boards[p] = match partition_best_rxs[p].recv() {
                            Ok(best) => best.map(|(_result, board)| board),
                            // Unreachable backstop: a worker PANIC
                            // propagates at the scope JOIN before any
                            // drain runs (the real fail-fast, the
                            // M5-T7 discipline); this arm guards a
                            // worker exiting without sending —
                            // impossible in the closure's straight-line
                            // shape (every path ends in both sends).
                            Err(_) => panic!(
                                "optimizer partition worker exited without sending its results"
                            ),
                        };
                        all_records.extend(records);
                    }
                    // Unreachable backstop — same face as the inner
                    // arm above (the scope join fails fast on a real
                    // worker panic before the drains run).
                    Err(_) => {
                        panic!("optimizer partition worker exited without sending its results")
                    }
                }
            }

            // The deterministic reduction: the golden candidate-walk
            // order (idx unique, so this sort is total), replaying the
            // sequential loop's consults, counting, flushes, winner
            // scan and breaks in the same positions.
            all_records.sort_by_key(|record| record.idx);
            let mut winning_id: Option<(ItemRouteResult, ItemId)> = None;
            for record in &all_records {
                let item_id = candidate_item_ids[record.idx];
                if self.deadline_passed() {
                    sink.info("Optimizer stage timed out.");
                    self.is_timed_out = true;
                    stopped_or_timed_out = true;
                    break;
                }
                if self.stop.is_requested() {
                    stopped_or_timed_out = true;
                    break;
                }
                if record.missing {
                    // Java `:1004-1007` — the defensive missing-item arm.
                    self.total_items_optimized += 1;
                    continue;
                }
                BufferedSinkRow::flush_rows(&record.rows, sink);
                self.total_items_optimized += 1;
                // Java `:742` resultMap bookkeeping — banked (PRIORITIZED).

                let result = &record.result;
                if result.improved() {
                    consecutive_failures = 0;
                    if winning_id
                        .as_ref()
                        .is_none_or(|(best, _)| result.improved_over(best))
                    {
                        winning_id = Some((result.clone(), item_id));
                    }
                } else {
                    consecutive_failures += 1;
                    if consecutive_failures >= max_consecutive_failures {
                        sink.info(&format!(
                            "Stopping optimization pass #{pass_no} early after \
                             {consecutive_failures} consecutive items could not be improved.",
                        ));
                        break;
                    }
                }
                if record.raised_stop {
                    // Replay the sequential raise timing: the gate fired
                    // DURING this candidate's reroute, so subsequent
                    // candidates' loop-top stop checks must see it.
                    self.stop.request_full();
                }
            }
            if let Some((result, item_id)) = winning_id {
                let p = optimizer_partition_of(item_id, partition_count);
                let board = partition_boards[p].take().expect(
                    "the pass winner is its partition's argmin (the grouping-invariance \
                     argument, logs/M8-T7/investigation.md §2)",
                );
                winning = Some((result, board));
            }
        }

        let mut route_improved: f32 = 0.0;
        if !stopped_or_timed_out
            && let Some((result, worker_board)) = winning
            && result.improved()
        {
            self.adopt_board(&worker_board);
            self.min_cumulative_trace_length = f64::from(total_weighted_trace_length(self.board));
            sink.board_updated(&counters);
            // The M9 snapshot hook (event_sink.rs): mirrors the
            // `board_updated` fire immediately — the default sink is
            // a no-op, so the parity stream is byte-stable.
            sink.board_snapshot(self.board);
            route_improved = result.improvement_percentage();
        }

        // Java `:797-800`.
        if self.use_increased_ripup_costs && route_improved == 0.0 {
            self.use_increased_ripup_costs = false;
            route_improved = -1.0; // keep going with lower ripup costs
        }

        // Java `:802-815` — the completed row. The pass wall clock is
        // real (the fanout precedent); the CPU/GB/heap faces are
        // banked.
        let stats_after = BoardStatistics::new(self.manager, self.board);
        sink.board_updated(&counters);
        // The M9 snapshot hook (event_sink.rs): mirrors the
        // `board_updated` fire immediately — the default sink is a
        // no-op, so the parity stream is byte-stable.
        sink.board_snapshot(self.board);
        sink.info(&format!(
            "Optimizer pass #{pass_no} on board '{}' was completed in {:.2} seconds with the \
             score of {}.",
            board_hash(self.board),
            pass_started.elapsed().as_secs_f64(),
            crate::pipeline::batch::format_score(
                stats_after.get_optimizer_score(Some(&self.settings.scoring)),
                stats_after.connections.incomplete_count.unwrap_or(0),
                stats_after.clearance_violations.total_count.unwrap_or(0),
            ),
        ));
        route_improved
    }
}

/// The per-pass candidate evaluation snapshot (M8-T7): the shared
/// inputs of [`opt_route_item_body`] for BOTH the sequential face and
/// the partitioned executor. `Sync` by construction (plain data + the
/// `Arc` stop flag), so partition workers can hold it behind `&`.
struct CandidateEvalBase {
    /// The stage's driver face (the ctor's settings, unmutated during
    /// the pass; [`candidate_settings_for`] clones per candidate as the
    /// pre-T7 method did).
    settings: BatchSettings,
    /// Java `optimizer.additionalRipupCostFactorAtStart`.
    additional_ripup_cost_factor_at_start: i32,
    /// Java `optimizer.traceRipupCostFactor`.
    trace_ripup_cost_factor: f32,
    /// Java `useIncreasedRipupCosts` — constant during the pass (the
    /// drop fires only after the candidate loop).
    use_increased_ripup_costs: bool,
    /// Java `minCumulativeTraceLength` — the candidate baseline.
    min_cumulative_trace_length: f64,
    /// The reroute pass budget per candidate.
    max_autoroute_passes: i32,
    /// The stage stop face's SHARED flag (an external owner's raise).
    /// Workers get a private [`StopFace`] over this flag; a raise a
    /// worker performs itself (the max-items face) is carried back in
    /// the candidate record, never through the flag.
    stop_flag: Option<Arc<AtomicBool>>,
    /// Java `deadlineMs` — wall-profile-only.
    deadline_ms: Option<i64>,
}

/// The candidate ripup costs (free face of the former
/// `candidate_ripup_costs` method — a pure function, shared by the
/// sequential face and the partitioned executor).
fn candidate_ripup_costs_for(
    start_ripup_costs: i32,
    use_increased: bool,
    additional_factor: i32,
    trace_factor: f32,
    is_trace: bool,
) -> i32 {
    let mut ripup_costs = start_ripup_costs;
    if use_increased {
        ripup_costs = ripup_costs.wrapping_mul(additional_factor);
    }
    if is_trace {
        ripup_costs = i32::try_from(java_round(f64::from(trace_factor) * f64::from(ripup_costs)))
            .unwrap_or(i32::MAX);
    }
    ripup_costs
}

/// The candidate's reroute settings (free face of the former
/// `candidate_settings` method — see its doc).
fn candidate_settings_for(
    settings: &BatchSettings,
    additional_factor: i32,
    trace_factor: f32,
    use_increased: bool,
    with_preferred_directions: bool,
    is_trace: bool,
) -> BatchSettings {
    let mut settings = settings.clone();
    settings.remove_unconnected_vias = true;
    settings.start_ripup_costs = candidate_ripup_costs_for(
        settings.start_ripup_costs,
        use_increased,
        additional_factor,
        trace_factor,
        is_trace,
    );
    settings.trace_costs =
        AutorouteControl::resolve_trace_costs(&settings.router_settings, with_preferred_directions);
    settings
}

/// The deadline consultation as a free face (former
/// `BatchOptimizerStage::deadline_passed` body).
fn deadline_passed_at(deadline_ms: Option<i64>) -> bool {
    deadline_ms.is_some_and(|deadline| now_millis() >= deadline)
}

/// The candidate→partition assignment (the charter's FIXED key,
/// `item_id mod n`): a pure function of `(item_id, n)`, used at BOTH
/// executor sites (the partition build and the winner-board lookup) so
/// the shipped key has ONE form — the invariance contract holds under
/// any fixed key, but this exact one is what the fixed-key pin guards.
fn optimizer_partition_of(item_id: ItemId, n: usize) -> usize {
    // NIT-5 erratum: the fix-round charter's `usize::from(item_id.get())`
    // does not exist — std provides `From<u32>` only for u64/u128 (usize's
    // width is target-dependent). The widening `as` is lossless wherever
    // the engine runs (64-bit) and carries no tolerance arm.
    item_id.get() as usize % n
}

/// Java `optRouteItemOnBoard` (`:842-925`): rip the connection
/// containing `item_id` (plus adjacent unfixed trace contacts for a
/// trace candidate), reroute it with the optimizer budget, and
/// compare the faces. M8-T7: extracted verbatim from the former
/// `BatchOptimizerStage::opt_route_item_on_board` method (pure code
/// motion) so the sequential face and the partitioned executor run
/// the SAME candidate path; the sequential face passes `self.stop`,
/// a partition worker passes its private [`StopFace`] over the shared
/// flag (the executor's doc covers the stop-face residual).
#[allow(clippy::too_many_lines)] // the Java body is one flat walk
fn opt_route_item_body(
    base: &CandidateEvalBase,
    stop: &mut StopFace,
    worker_manager: &mut SearchTreeManager,
    worker_board: &mut Board,
    item_id: ItemId,
    with_preferred_directions: bool,
    sink: &mut dyn DriverSink,
) -> ItemRouteResult {
    // Java `:851-853` — the `null` unit keeps the board factors;
    // the connections walk stays ON (Java's 3-arg ctor defaults it
    // to true), though only the via/length faces are read.
    let stats_before = BoardStatistics::with_options(worker_manager, worker_board, false, true);
    let incomplete_before = calculate_incomplete_count(worker_manager, worker_board);

    let is_trace = worker_board
        .get(item_id)
        .is_some_and(|entry| matches!(entry.data, ItemData::Trace { .. }));

    // Java `:855-866` — the ripped set: the item itself, plus — for
    // a trace — each endpoint's contact set when it is all
    // unfixed traces.
    let mut ripped_items: BTreeSet<Reverse<ItemId>> = BTreeSet::new();
    ripped_items.insert(Reverse(item_id));
    if is_trace {
        let start = start_contacts(worker_manager, worker_board, item_id);
        if contains_only_unfixed_traces(worker_board, &start) {
            ripped_items.extend(start.into_iter().map(Reverse));
        }
        let end = end_contacts(worker_manager, worker_board, item_id);
        if contains_only_unfixed_traces(worker_board, &end) {
            ripped_items.extend(end.into_iter().map(Reverse));
        }
    }

    // Java `:868-871` — the containing connections.
    let mut ripped_connections: BTreeSet<Reverse<ItemId>> = BTreeSet::new();
    for ripped in &ripped_items {
        for contact in get_connection_items(
            worker_manager,
            worker_board,
            ripped.0,
            epic_board::trace_ops::StopConnectionOption::None,
        ) {
            ripped_connections.insert(Reverse(contact));
        }
    }

    // Java `:873-877` — a user-fixed item anywhere in the set vetoes.
    if ripped_connections
        .iter()
        .any(|contact| worker_board.get(contact.0).is_some_and(is_user_fixed))
    {
        return ItemRouteResult::unimproved(i32::try_from(item_id.get()).unwrap_or(i32::MAX));
    }

    // Java `:879-882` — remove, then re-combine the net.
    for contact in &ripped_connections {
        remove_item_through_repository(worker_manager, worker_board, contact.0);
    }
    let item_nets = worker_board
        .get(item_id)
        .map(|entry| entry.nets.clone())
        .unwrap_or_default();
    for net in &item_nets {
        combine_traces(worker_manager, worker_board, *net);
    }

    // Java `:884-904` — the reroute (`autoroutePassesForOptimizingItem`,
    // `BatchAutorouter.java:245-281`): full autoroute passes until
    // the board is complete or the budget burns, then the
    // `removeTails(NONE)` sweep.
    let candidate_settings = candidate_settings_for(
        &base.settings,
        base.additional_ripup_cost_factor_at_start,
        base.trace_ripup_cost_factor,
        base.use_increased_ripup_costs,
        with_preferred_directions,
        is_trace,
    );
    let mut still_unrouted = true;
    let mut reroute_pass_no = 1;
    while still_unrouted
        && !stop.is_requested()
        // Java `:894-895` — the reroute pass budget (`optimizer !=
        // null ? maxAutoroutePasses : 1`; the merged flow always
        // carries the box, so the resolved default 6 is the live face).
        && reroute_pass_no <= base.max_autoroute_passes
    {
        // SCOPE (M5-T7): the optimizer's reroute stays on the
        // sequential face — `run_pass`'s partitioned executor
        // engages only from the batch driver at this tree.
        still_unrouted = run_single_thread(
            worker_manager,
            worker_board,
            &candidate_settings,
            reroute_pass_no,
            &mut 0, // the reroute router's own totalItemsRouted (fresh per candidate)
            &mut 0, // its progress counter
            stop,
            sink,
            // The optimizer's inline reroute passes stay on the
            // linear ladder (the negotiated scheduler is the batch
            // pass loop's seam).
            None,
        );
        reroute_pass_no += 1;
    }
    remove_tails(
        worker_manager,
        worker_board,
        epic_board::trace_ops::StopConnectionOption::None,
        candidate_settings.pull_tight_accuracy,
        &candidate_settings.trace_costs,
        stop.flag(),
        TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP,
        candidate_settings.deterministic_budgets,
    );
    // Java returns the pass count from
    // `autoroutePassesForOptimizingItem` (`:280`). T10's
    // `phases.optimizer.passes_completed` face is fed from
    // `OptimizerOutcome.passes_completed` (the BATCH pass count,
    // Java `BatchOptimizer.java:523`) instead — the reservation in
    // the T9 comment was mis-scoped: a per-candidate reroute-pass
    // number was never the right source for a batch pass count.
    // The value is kept underscore-bound as dead observability.
    let _reroute_passes = if !still_unrouted {
        reroute_pass_no - 1
    } else {
        reroute_pass_no
    };

    // Java `:906-918` — the comparison faces.
    let stats_after = BoardStatistics::with_options(worker_manager, worker_board, false, true);
    let incomplete_after = calculate_incomplete_count(worker_manager, worker_board);
    let mut result = ItemRouteResult::new(
        i32::try_from(item_id.get()).unwrap_or(i32::MAX),
        stats_before.items.via_count,
        stats_after.items.via_count,
        base.min_cumulative_trace_length,
        f64::from(stats_after.traces.total_length),
        incomplete_before,
        incomplete_after,
    );

    // Java `:919-923` — the stop/deadline gate on the verdict.
    let route_improved =
        !stop.is_requested() && !deadline_passed_at(base.deadline_ms) && result.improved();
    result.update_improved(route_improved);
    result
}

/// Java `DEFAULT_OPTIMIZER_IMPROVEMENT_THRESHOLD`
/// (`DefaultSettings.java:129`) — the sanitation reset value.
pub const DEFAULT_OPTIMIZER_IMPROVEMENT_THRESHOLD: f32 = 2.5;

/// Java `BoardStatistics`' banked `totalWeightedLength`
/// (`BoardStatistics.java:265-288`), ported here because the only T9
/// consumer is the optimizer's candidate baseline: the f32 sum over
/// UNFIXED and SHOVE_FIXED traces of
/// `length * (halfWidth + clearanceValue(class, defaultClass, layer))`,
/// SHOVE_FIXED terms halved ("to produce less violations with pin exit
/// directions"), each term rounded to f32 before the add. The walk is
/// the DESCENDING-id `itemList` order (the f32 add sequence).
#[must_use]
pub fn total_weighted_trace_length(board: &Board) -> f32 {
    let mut total: f32 = 0.0;
    for entry in board.iter_descending() {
        if !entry.on_the_board {
            continue;
        }
        let ItemData::Trace {
            layer,
            half_width,
            lines,
            ..
        } = &entry.data
        else {
            continue;
        };
        let fixed = entry.fixed;
        if fixed != epic_board::items::FixedState::Unfixed
            && fixed != epic_board::items::FixedState::ShoveFixed
        {
            continue;
        }
        let length = lines.length_approx_total();
        let clearance =
            board.clearance_value(entry.clearance_class, DEFAULT_CLEARANCE_CLASS, *layer);
        let mut weighted = length * f64::from(*half_width + clearance);
        if fixed == epic_board::items::FixedState::ShoveFixed {
            weighted /= 2.0;
        }
        total += weighted as f32;
    }
    total
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|delta| i64::try_from(delta.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// the pins (crafted worlds; each pin names the Java face it guards)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::batch::BatchDriver;
    use crate::pipeline::board_statistics::{
        OptimizerScoreSettings, RouterScoreSettings, RouterScoringVersion, RouterSettingsScoring,
        default_routing_cost_settings,
    };
    use crate::pipeline::event_sink::CaptureDriverSink;
    use crate::test_util::parse;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::polyline::Polyline;

    /// The T9/T10c locator-world fixture (2 layers, `unit um`; 100 SMD
    /// pins all on layer 0; its nets 33/98 are the only two pin PAIRS).
    fn parse_fixture() -> (SearchTreeManager, Board) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../harness/fixtures/locator-spike/t9_locator45.dsn");
        let text = std::fs::read_to_string(&path).expect("fixture present");
        parse(&text)
    }

    fn settings_ir() -> crate::control::RouterSettingsIr {
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
        }
    }

    /// The merged-flow optimizer defaults (`DefaultSettings.java:177-191`).
    fn opt_settings() -> OptimizerSettingsIr {
        OptimizerSettingsIr {
            algorithm: "freerouting-optimizer".to_string(),
            max_passes: Some(3),
            max_items: None,
            improvement_threshold: Some(2.5),
            enable_preflight_guards: Some(false), // bypass: crafted worlds run
            max_consecutive_failures: Some(50),
            max_consecutive_failures_pass1: Some(12),
            additional_ripup_cost_factor_at_start: 10,
            trace_ripup_cost_factor: 0.6,
            max_autoroute_passes: 6,
            timeout_string: None,
        }
    }

    /// The production score face: DefaultSettings' three boxes.
    fn v2_scoring() -> RouterSettingsScoring {
        RouterSettingsScoring {
            scoring: Some(default_routing_cost_settings()),
            router_scoring: Some(RouterScoreSettings {
                version: RouterScoringVersion::V2Continuous,
                unrouted_free_fraction: Some(0.5),
                unrouted_first_half_weight: Some(1000.0 / 3.0),
                unrouted_second_half_weight: Some(2000.0 / 3.0),
                clearance_violation_count_weight: Some(25.0),
                clearance_violation_depth_weight: Some(300.0),
                clearance_violation_depth_scale: Some(1000.0),
            }),
            optimizer_scoring: Some(OptimizerScoreSettings {
                version: crate::pipeline::board_statistics::OptimizerScoringVersion::V2LowerBound,
                excess_wire_length_weight: Some(1000.0),
                excess_via_weight: Some(2000.0),
                excess_bend_weight: Some(500.0),
                length_floor: Some(1.0),
                difficulty_scale_floor: Some(1.0),
            }),
        }
    }

    /// Fanout-OFF routed world (no fanout vias; the pairs routed
    /// directly; 2 vias, 6 traces, len 132.03105 mm, score 951.88).
    fn routed_world_fanout_off() -> (SearchTreeManager, Board) {
        let (mut manager, mut board) = parse_fixture();
        let mut settings = BatchSettings::new(settings_ir(), v2_scoring());
        settings.fanout_enabled = false;
        settings.remove_unconnected_vias = true;
        let mut sink = CaptureDriverSink::default();
        let mut driver = BatchDriver::new(&mut manager, &mut board, settings, StopFace::default());
        assert_eq!(driver.run(&mut sink), Ok(true), "fanout-off world routes");
        (manager, board)
    }

    /// The pins' stage constructor five-liner, retired (review NIT-3):
    /// fresh `StopFace` + `CaptureDriverSink` + `BatchOptimizerStage`
    /// over the given borrows and settings.
    fn stage_on_world<'a>(
        manager: &'a mut SearchTreeManager,
        board: &'a mut Board,
        stop: &'a mut StopFace,
        optimizer: OptimizerSettingsIr,
        settings: BatchSettings,
    ) -> (BatchOptimizerStage<'a>, CaptureDriverSink) {
        let sink = CaptureDriverSink::default();
        let stage = BatchOptimizerStage::new(manager, board, settings, optimizer, stop);
        (stage, sink)
    }

    /// The `opt_settings()` tweak helper: clone-and-mutate the defaults.
    fn opt_with(
        mut optimizer: OptimizerSettingsIr,
        tweak: impl FnOnce(&mut OptimizerSettingsIr),
    ) -> OptimizerSettingsIr {
        tweak(&mut optimizer);
        optimizer
    }

    /// The bare fixture with the two pin PAIRS removed — every
    /// remaining net is a lone pin: 0 incompletes, 0 vias, 0 traces.
    fn pairless_world() -> (SearchTreeManager, Board) {
        let (mut manager, mut board) = parse_fixture();
        let pins: Vec<ItemId> = board
            .iter_ascending()
            .filter(|entry| entry.nets.contains(&33) || entry.nets.contains(&98))
            .filter(|entry| matches!(entry.data, ItemData::Pin { .. }))
            .map(|entry| entry.id)
            .collect();
        assert_eq!(pins.len(), 4, "two pin pairs");
        for id in pins {
            manager.remove(&mut board, id);
            board.remove_item(id);
        }
        (manager, board)
    }

    fn pt(x: i32, y: i32) -> Point {
        Point::Int(IntPoint::new(x, y))
    }

    fn insert_trace(
        manager: &mut SearchTreeManager,
        board: &mut Board,
        corners: [(i32, i32); 2],
        layer: i32,
        nets: &[i32],
        fixed: epic_board::items::FixedState,
    ) -> ItemId {
        let (a, b) = (
            pt(corners[0].0, corners[0].1),
            pt(corners[1].0, corners[1].1),
        );
        epic_board::trace_ops::insert_trace_without_cleaning(
            manager,
            board,
            Polyline::from_two_corners(&a, &b),
            layer,
            500,
            nets,
            1,
            fixed,
        )
        .expect("trace insert")
    }

    fn insert_test_via(
        manager: &mut SearchTreeManager,
        board: &mut Board,
        center: (i32, i32),
        nets: &[i32],
    ) -> ItemId {
        // Padstack ids are 1-BASED (`BoardLibrary::padstack`); use the
        // last padstack — on the hand board that is the crafted
        // `t9_via`, on the fixture a real via padstack.
        let padstack_no = board.library().padstacks.len() as i32;
        epic_board::drill_item_mover::insert_via(
            manager,
            board,
            padstack_no,
            IntPoint::new(center.0, center.1),
            nets,
            1,
            epic_board::items::FixedState::Unfixed,
            false,
        )
    }

    /// Hand-built 2-layer board factory for the pure reader pin.
    fn hand_board() -> Board {
        let mut board = Board::new();
        let rect = || {
            Some(epic_board::items::BoardShape::Tile(
                epic_geometry::tile_shape::TileShape::RegularTileShape(
                    epic_geometry::regular_tile_shape::RegularTileShape::IntBox(
                        epic_geometry::int_box::IntBox::new(
                            IntPoint::new(-250, -250),
                            IntPoint::new(250, 250),
                        ),
                    ),
                ),
            ))
        };
        // A fresh Board carries NO padstacks; the via padstack spans the
        // two layers the crafted vias transition between.
        let layer_count = 2;
        board
            .library_mut()
            .padstacks
            .push(epic_board::components::BoardPadstack {
                name: "t9_via".to_string(),
                shapes: vec![rect(); layer_count],
                drillable: true,
                placed_absolute: false,
                hole_only: false,
            });
        board
    }

    // -----------------------------------------------------------------------
    // ItemRouteResult (Java ItemRouteResult.java)
    // -----------------------------------------------------------------------

    /// The improved() ladder (`ItemRouteResult.java:31-59`), one world
    /// per rung: incomplete down/up; equal + via down/up; equal + length
    /// down/equal. The equal-length/equal-via/equal-incomplete face is
    /// NOT improved (no free wins).
    #[test]
    fn t9_item_route_result_ladder() {
        let mut result = ItemRouteResult::new(7, 2, 1, 100.0, 100.0, 0, 0);
        assert!(result.improved(), "equal faces, via 2 -> 1: improved");
        assert_eq!(result.via_count(), 1);
        assert_eq!(result.via_count_reduced(), 1);

        result = ItemRouteResult::new(7, 2, 2, 100.0, 90.0, 0, 0);
        assert!(result.improved(), "equal vias, length 100 -> 90: improved");
        assert!((result.length_reduced() - 10.0).abs() < f64::EPSILON);

        result = ItemRouteResult::new(7, 2, 3, 100.0, 100.0, 0, 0);
        assert!(!result.improved(), "via count rose: not improved");

        result = ItemRouteResult::new(7, 2, 2, 100.0, 110.0, 0, 0);
        assert!(!result.improved(), "length rose: not improved");

        result = ItemRouteResult::new(7, 2, 2, 100.0, 100.0, 0, 0);
        assert!(!result.improved(), "all faces equal: not improved");

        result = ItemRouteResult::new(7, 2, 2, 100.0, 100.0, 1, 0);
        assert!(result.improved(), "incomplete 1 -> 0 wins outright");
        result = ItemRouteResult::new(7, 2, 1, 100.0, 100.0, 0, 1);
        assert!(!result.improved(), "incomplete 0 -> 1 loses outright");

        // Java's 1-arg ctor: `(0,0,0,0,0,1)` then improved=false.
        result = ItemRouteResult::unimproved(42);
        assert!(!result.improved());
        assert_eq!(result.item_id, 42);
        assert_eq!(result.improvement_percentage(), 0.0);

        // improvementPercentage: `1 - ((viaAfter/viaBefore +
        // lenAfter/lenBefore)/2)` — the via ratio is JAVA INTEGER
        // DIVISION (jar probe W1: via 2→1, len 100→100 answers 0.5 —
        // int 1/2 = 0, not float 0.5; probe W2: 4→2 answers 0.5 too,
        // where float 2/4 = 0.5 would give 0.25 — the two worlds pin
        // the int-division drop in both directions); the length ratio
        // stays real (probe W5: via equal, len 100→50 -> 0.25); the
        // zero-baseline arm pins at 0 (probe W6).
        let pct = ItemRouteResult::new(7, 2, 1, 100.0, 100.0, 0, 0);
        assert!(
            (f64::from(pct.improvement_percentage()) - 0.5).abs() < 1e-6,
            "jar probe W1 face"
        );
        let pct_exact_ratio = ItemRouteResult::new(7, 4, 2, 100.0, 100.0, 0, 0);
        assert!(
            (f64::from(pct_exact_ratio.improvement_percentage()) - 0.5).abs() < 1e-6,
            "jar probe W2 face: the EXACT 4->2 ratio ALSO int-truncates to 0"
        );
        let pct_len = ItemRouteResult::new(7, 2, 2, 100.0, 50.0, 0, 0);
        assert!(
            (f64::from(pct_len.improvement_percentage()) - 0.25).abs() < 1e-6,
            "jar probe W5 face: the length ratio stays REAL division"
        );
        let zero = ItemRouteResult::new(7, 0, 0, 0.0, 0.0, 0, 0);
        assert_eq!(
            zero.improvement_percentage(),
            0.0,
            "via/len baseline 0 -> 0"
        );
    }

    /// The winner ordering (`compareTo`/`improvedOver`,
    /// `ItemRouteResult.java:69-91`): the
    /// incomplete rung dominates, then via count, then length, then
    /// itemId. THE TIE FACE: equal-metric outcomes rank by itemId, so
    /// the earlier-submitted candidate's outcome wins a tie — this is
    /// what makes the one-winner rule deterministic.
    #[test]
    fn t9_item_route_result_compare_and_item_id_tie() {
        let by_incomplete = ItemRouteResult::new(1, 2, 2, 100.0, 100.0, 0, 0);
        let worse = ItemRouteResult::new(2, 2, 2, 100.0, 100.0, 1, 1);
        assert!(by_incomplete.improved_over(&worse));
        assert!(!worse.improved_over(&by_incomplete));

        let by_via = ItemRouteResult::new(1, 3, 2, 100.0, 200.0, 0, 0);
        let worse_len = ItemRouteResult::new(2, 3, 3, 100.0, 50.0, 0, 0);
        assert!(
            by_via.improved_over(&worse_len),
            "2 vias beats 3 despite length"
        );
        assert!(!worse_len.improved_over(&by_via));

        let by_len = ItemRouteResult::new(1, 2, 2, 100.0, 90.0, 0, 0);
        let len_b = ItemRouteResult::new(2, 2, 2, 100.0, 95.0, 0, 0);
        assert!(
            by_len.improved_over(&len_b),
            "shorter board wins at equal vias"
        );

        // THE tie: equal incomplete/via/length -> lower itemId wins.
        let first = ItemRouteResult::new(10, 2, 1, 100.0, 90.0, 0, 0);
        let second = ItemRouteResult::new(11, 2, 1, 100.0, 90.0, 0, 0);
        assert!(first.improved_over(&second), "lower itemId wins the tie");
        assert!(!second.improved_over(&first));
    }

    // -----------------------------------------------------------------------
    // the acceptance gates (Java optimizerCandidateRejectionReason :565-581)
    // -----------------------------------------------------------------------

    /// Crossing cells for all three rejection gates: each veto fires on
    /// its regression AND does not fire on its adjacent non-regression,
    /// and the veto ORDER is connectivity > DRC > score.
    #[test]
    fn t9_rejection_gates_crossing_cells() {
        let base = || BoardStatistics {
            connections: crate::pipeline::board_statistics::ConnectionsCounts {
                maximum_count: Some(10),
                incomplete_count: Some(0),
            },
            clearance_violations: crate::pipeline::board_statistics::ClearanceViolationsStats {
                total_count: Some(2),
                ..crate::pipeline::board_statistics::ClearanceViolationsStats::default()
            },
            ..BoardStatistics::new_empty()
        };

        // CONNECTIVITY_REGRESSION: candidate 1 > incumbent 0 fires; the
        // adjacent equal face falls through to the next gate.
        let mut candidate = base();
        candidate.connections.incomplete_count = Some(1);
        assert_eq!(
            optimizer_candidate_rejection_reason(0, 5, 900.0, &candidate, 999.0),
            Some("CONNECTIVITY_REGRESSION"),
            "a MORE-COMPLETE-looking score must not smuggle in a connectivity regression",
        );

        // DRC_COUNT_REGRESSION: fires on 3 > 2; equal 2 falls through.
        let mut candidate = base();
        candidate.clearance_violations.total_count = Some(3);
        assert_eq!(
            optimizer_candidate_rejection_reason(0, 2, 900.0, &candidate, 999.0),
            Some("DRC_COUNT_REGRESSION"),
        );
        let candidate = base();
        assert_eq!(
            optimizer_candidate_rejection_reason(0, 2, 900.0, &candidate, 900.0),
            Some("OPTIMIZER_SCORE_NOT_IMPROVED"),
            "equal faces + equal score: the score gate fires",
        );
        assert_eq!(
            optimizer_candidate_rejection_reason(0, 2, 900.0, &candidate, 899.0),
            Some("OPTIMIZER_SCORE_NOT_IMPROVED"),
            "a WORSE score also rejects (strict improvement required)",
        );
        assert_eq!(
            optimizer_candidate_rejection_reason(0, 2, 900.0, &candidate, 900.5),
            None,
            "strictly better score with equal counts is ACCEPTED (None)",
        );
        // The ordering face: a better score cannot buy back a veto.
        let mut candidate = base();
        candidate.connections.incomplete_count = Some(5);
        assert_eq!(
            optimizer_candidate_rejection_reason(0, 2, 900.0, &candidate, 999.0),
            Some("CONNECTIVITY_REGRESSION"),
        );
        let mut candidate = base();
        candidate.clearance_violations.total_count = Some(9);
        assert_eq!(
            optimizer_candidate_rejection_reason(0, 2, 900.0, &candidate, 999.0),
            Some("DRC_COUNT_REGRESSION"),
        );
    }

    // -----------------------------------------------------------------------
    // ReadSortedRouteItems (Java :1086-1182) — the candidate ORDER
    // -----------------------------------------------------------------------

    /// The candidate order: ascending (x, y, layer); vias preferred at
    /// the same key (the trace pass runs last and only a STRICTLY lower
    /// layer can steal); skip user-fixed vias, shove-fixed traces and
    /// traces touching an unfixed via; descending-id tie-break at
    /// identical keys.
    #[test]
    fn t9_read_sorted_route_items_order() {
        let mut manager = SearchTreeManager::new();
        let mut board = hand_board();
        manager.reinsert_tree_items(&mut board);

        // Two vias at the SAME key (0,0): Java keeps only the first
        // item encountered in the descending walk (higher id) and the
        // cursor then excludes the duplicate key forever.
        let va = insert_test_via(&mut manager, &mut board, (0, 0), &[1]);
        let vb = insert_test_via(&mut manager, &mut board, (0, 0), &[1]);
        assert!(
            vb.get() > va.get(),
            "insertion order gives vb the higher id"
        );
        // trace_ta and trace_tb share the compare corner (100, 300) and
        // the layer — the same dedup applies.
        let ta = insert_trace(
            &mut manager,
            &mut board,
            [(100, 0), (100, 300)],
            0,
            &[2],
            epic_board::items::FixedState::Unfixed,
        );
        let tb = insert_trace(
            &mut manager,
            &mut board,
            [(0, 300), (100, 300)],
            0,
            &[3],
            epic_board::items::FixedState::Unfixed,
        );
        assert!(tb.get() > ta.get());
        // trace_fixed: would sort first (x = -100) but SHOVE_FIXED -> skipped.
        let fixed = insert_trace(
            &mut manager,
            &mut board,
            [(-100, 0), (-100, -500)],
            0,
            &[4],
            epic_board::items::FixedState::ShoveFixed,
        );
        // via_user: (50,50) would sort early but USER_FIXED -> skipped.
        let user_via = insert_test_via(&mut manager, &mut board, (50, 50), &[5]);
        board.set_item_fixed(user_via, epic_board::items::FixedState::UserFixed);
        // trace_toucher: compare corner (200,400) sits on unfixed via_tu
        // (same net, same layer, endpoint==center) -> skipped.
        let tu = insert_test_via(&mut manager, &mut board, (200, 400), &[6]);
        let toucher = insert_trace(
            &mut manager,
            &mut board,
            [(150, 400), (200, 400)],
            0,
            &[6],
            epic_board::items::FixedState::Unfixed,
        );

        let order = read_sorted_route_items(&manager, &mut board);
        let ids: Vec<u32> = order.iter().map(|id| id.get()).collect();

        // Expected: keys ascend (0,0) -> (100,300) -> (200,400); at each
        // duplicate key only the FIRST-encountered (higher id) survives.
        assert_eq!(
            ids,
            vec![vb.get(), tb.get(), tu.get()],
            "ascending keys, descending-id dedup at equal keys: {ids:?}"
        );
        assert!(
            !ids.contains(&va.get()),
            "the duplicate-key via never enumerates: {ids:?}"
        );
        assert!(
            !ids.contains(&ta.get()),
            "the duplicate-key trace never enumerates: {ids:?}"
        );
        assert!(
            !ids.contains(&fixed.get()),
            "shove-fixed traces are skipped: {ids:?}"
        );
        assert!(
            !ids.contains(&user_via.get()),
            "user-fixed vias are skipped: {ids:?}"
        );
        assert!(
            !ids.contains(&toucher.get()),
            "traces touching an unfixed via are skipped: {ids:?}"
        );
    }

    // -----------------------------------------------------------------------
    // areAllViasMandatoryLayerTransitions (Java :225-275)
    // -----------------------------------------------------------------------

    /// Real-board faces: the routed world's vias connect same-layer SMD
    /// pin pairs -> NOT all mandatory (crossesLayers=false arm); the
    /// all-user-fixed quirk returns TRUE (Java `:231-233` continue); a
    /// netless via breaks it; a twin via in the connected set breaks
    /// it; an empty via set is FALSE (`:227-229`).
    #[test]
    fn t9_are_all_vias_mandatory_worlds() {
        // (a) routed fanout-off world: two vias, each between two
        //     layer-0 SMD pins through traces — crossesLayers=false.
        let (mut manager, mut board) = routed_world_fanout_off();
        assert!(
            !are_all_vias_mandatory_layer_transitions(&manager, &mut board),
            "same-layer pin pairs mean the vias are NOT mandatory transitions",
        );

        // (b) the quirk: make every via user-fixed -> the loop skips
        //     them all and returns the vacuous TRUE.
        let via_ids: Vec<ItemId> = board
            .iter_ascending()
            .filter(|entry| matches!(entry.data, ItemData::Via { .. }))
            .map(|entry| entry.id)
            .collect();
        assert_eq!(via_ids.len(), 2);
        for id in &via_ids {
            board.set_item_fixed(*id, epic_board::items::FixedState::UserFixed);
        }
        assert!(
            are_all_vias_mandatory_layer_transitions(&manager, &mut board),
            "all-user-fixed vias: Java's vacuous-true quirk",
        );
        for id in &via_ids {
            board.set_item_fixed(*id, epic_board::items::FixedState::Unfixed);
        }

        // (c) a netless via -> netCount()==0 -> false.
        let _netless = insert_test_via(&mut manager, &mut board, (-50_000, 20_000), &[]);
        assert!(
            !are_all_vias_mandatory_layer_transitions(&manager, &mut board),
            "netCount()==0 arm",
        );

        // (d) empty via set -> false.
        let (manager2, mut board2) = pairless_world();
        assert!(
            !are_all_vias_mandatory_layer_transitions(&manager2, &mut board2),
            "an empty via set is FALSE, not vacuously true",
        );
    }

    // -----------------------------------------------------------------------
    // the preflight guards (Java evaluatePreFlightGuards :155-219)
    // -----------------------------------------------------------------------

    /// The shared crafted-world builder for the preflight boundary
    /// pins: a `BoardStatistics` carrying ONLY the fields the guards
    /// read, all four parameterized (the NIT-4 dedup — the two
    /// adjacent boundary tests previously carried body-identical
    /// test-local closures).
    fn crafted_stats(
        incomplete: Option<i32>,
        min_len: f32,
        total_len: f32,
        min_via: Option<i32>,
        via_count: i32,
    ) -> BoardStatistics {
        let mut s = BoardStatistics::new_empty();
        s.connections.incomplete_count = incomplete;
        s.vias.total_count = via_count;
        s.bounds.min_trace_length_mm = Some(min_len);
        s.bounds.min_via_count = min_via;
        s.traces.total_length_mm = Some(total_len);
        s
    }

    /// The shared V2 scoring face for the preflight boundary pins: the
    /// length weight is the only live weight (via/bend zeroed), floor
    /// 1.0 — `score = 1000 - weight x (total - min) / max(min, 1)`.
    fn crafted_scoring(length_weight: f32) -> RouterSettingsScoring {
        RouterSettingsScoring {
            scoring: Some(default_routing_cost_settings()),
            router_scoring: None,
            optimizer_scoring: Some(OptimizerScoreSettings {
                version: crate::pipeline::board_statistics::OptimizerScoringVersion::V2LowerBound,
                excess_wire_length_weight: Some(length_weight),
                excess_via_weight: Some(0.0),
                excess_bend_weight: Some(0.0),
                length_floor: Some(1.0),
                difficulty_scale_floor: Some(1.0),
            }),
        }
    }

    /// Guard matrix: guard 1 fires on the bare fixture (2 unrouted) and
    /// passes on the routed world; ONLY an explicit `Some(false)`
    /// bypasses (Java `Boolean.FALSE.equals`); unset (`None`) runs the
    /// guards like Java's null.
    #[test]
    fn t9_preflight_guard1_and_bypass() {
        let (mut manager, mut board) = parse_fixture();
        let mut stop = StopFace::default();
        let batch = BatchSettings::new(settings_ir(), v2_scoring());
        let mut opt = opt_settings();
        opt.enable_preflight_guards = Some(true);
        let mut stage = BatchOptimizerStage::new(&mut manager, &mut board, batch, opt, &mut stop);
        let stats = {
            let mut m2 = SearchTreeManager::new();
            let mut b2 = parse_fixture().1;
            m2.reinsert_tree_items(&mut b2);
            BoardStatistics::new(&mut m2, &mut b2)
        };
        // Guard 1 fires with the real incompletes count.
        let reason = stage.evaluate_pre_flight_guards(&stats);
        assert!(
            reason
                .as_deref()
                .is_some_and(|r| r.contains("unrouted connection(s)")
                    && r.contains("only runs on completely routed boards")),
            "guard 1 firing world: {reason:?}",
        );

        // The bypass: ONLY Some(false) skips the guards (same stats).
        stage.optimizer.enable_preflight_guards = Some(false);
        assert_eq!(
            stage.evaluate_pre_flight_guards(&stats),
            None,
            "explicit false bypasses ALL guards",
        );
        stage.optimizer.enable_preflight_guards = None;
        assert!(
            stage.evaluate_pre_flight_guards(&stats).is_some(),
            "unset (Java null) does NOT bypass — guards still run",
        );

        // The adjacent non-firing world: routed board -> guard 1 passes
        // (and, on this world, every later guard too -> None).
        let (mut manager3, mut board3) = routed_world_fanout_off();
        let stats3 = {
            let mut m4 = SearchTreeManager::new();
            m4.reinsert_tree_items(&mut board3);
            BoardStatistics::new(&mut m4, &mut board3)
        };
        let batch3 = BatchSettings::new(settings_ir(), v2_scoring());
        let mut stop3 = StopFace::default();
        let mut opt3 = opt_settings();
        opt3.enable_preflight_guards = Some(true);
        let mut stage3 =
            BatchOptimizerStage::new(&mut manager3, &mut board3, batch3, opt3, &mut stop3);
        assert_eq!(
            stage3.evaluate_pre_flight_guards(&stats3),
            None,
            "the routed world passes every guard",
        );
    }

    /// Guards 2/3 against CRAFTED statistics on a via-less board (the
    /// formulas are the pinned face; the weights are legitimate user
    /// settings). Mode 16: the 1.05/1.02 boundaries are pinned AT the
    /// exact edge and one step past it, both directions.
    #[test]
    fn t9_preflight_guards_2_3_boundary_faces() {
        let (mut manager, mut board) = pairless_world(); // 0 incompletes, 0 vias
        let mut stop = StopFace::default();
        let batch = BatchSettings::new(settings_ir(), v2_scoring());
        let mut opt = opt_settings();
        opt.enable_preflight_guards = Some(true); // guards ON — the pin targets them
        let mut stage = BatchOptimizerStage::new(&mut manager, &mut board, batch, opt, &mut stop);

        let scoring = crafted_scoring;
        let stats = |min_len: f32, total_len: f32, min_via: Option<i32>, via_count: i32| {
            crafted_stats(Some(0), min_len, total_len, min_via, via_count)
        };

        // Guard 2a: no vias + score >= 950. min=100, total=100 -> 0 excess
        // -> score 1000 >= 950.
        stage.settings.scoring = scoring(1000.0);
        let s = stats(100.0, 100.0, Some(1), 0);
        let reason = stage.evaluate_pre_flight_guards(&s);
        assert!(
            reason.as_deref().is_some_and(|r| r.contains(">= 950.00")),
            "guard 2a firing world: {reason:?}",
        );

        // Guard 2b: no vias + score < 950 (weight 2000: excess 5% ->
        // penalty 100 -> 900) + total <= min*1.05. THE EDGE (mode 16):
        // the boundary is the f32 product 100*1.05f (104.99999...), and
        // the pin stands ON it.
        let boundary_5 = 100.0f32 * 1.05f32;
        stage.settings.scoring = scoring(2000.0); // excess 5% -> penalty 100 -> score 900 < 950
        let s = stats(100.0, boundary_5, Some(1), 0);
        let reason = stage.evaluate_pre_flight_guards(&s);
        assert!(
            reason
                .as_deref()
                .is_some_and(|r| r.contains("within 5% of theoretical")),
            "guard 2b at the exact 1.05 boundary fires: {reason:?}",
        );
        // One step past the boundary: no fire (falls through to guard 3b
        // whose 1.02 face also fails, then guard 4: board has no vias ->
        // false) -> guards pass.
        let s = stats(100.0, boundary_5 + 0.01, Some(1), 0);
        assert_eq!(
            stage.evaluate_pre_flight_guards(&s),
            None,
            "guard 2b just past the boundary does not fire",
        );

        // Guard 3a: score >= 995 with a VIA on the board (guard 2's
        // via-less gate is closed by total_count=1).
        let s = stats(100.0, 100.0, Some(1), 1);
        let reason = stage.evaluate_pre_flight_guards(&s);
        assert!(
            reason
                .as_deref()
                .is_some_and(|r| r.contains("theoretical maximum (995.00)")),
            "guard 3a firing world: {reason:?}",
        );
        // Adjacent: score 960 at the ACTIVE weight 2000 (penalty
        // 2000 x 2% = 40) — stays below 995 (review NIT fix).
        let s = stats(100.0, 102.0, Some(1), 1);
        // Guard 3b: total == 100*1.02f (f32-exactly 102.0 here) AND
        // vias 1 <= min_via 1.
        assert_eq!(100.0f32 * 1.02f32, 102.0, "the 1.02 boundary lands exactly");
        let reason = stage.evaluate_pre_flight_guards(&s);
        assert!(
            reason
                .as_deref()
                .is_some_and(|r| r.contains("within 2% of the theoretical")),
            "guard 3b at the exact 1.02 boundary fires: {reason:?}",
        );
        // The min_via None arm: bounds.min_via_count == null counts as
        // "vias at minimum" (Java `minViaCount == null ||`).
        let s = stats(100.0, 102.0, None, 1);
        let reason = stage.evaluate_pre_flight_guards(&s);
        assert!(
            reason
                .as_deref()
                .is_some_and(|r| r.contains("within 2% of the theoretical")),
            "min_via_count == null satisfies the via arm: {reason:?}",
        );
        // One step past 1.02: no fire anywhere.
        let s = stats(100.0, 102.01, Some(1), 1);
        assert_eq!(
            stage.evaluate_pre_flight_guards(&s),
            None,
            "guard 3b just past the boundary does not fire",
        );
    }

    /// The M6-T1 exact edges (buglog 196; charter boundary list) in
    /// Java `BatchOptimizer.evaluatePreFlightGuards` (`:155-219`; the
    /// `>= 950.0f` / `>= 995.0f` thresholds live at `:171-211`):
    /// guard 1 at the 0-vs-1 incomplete edge (the interpolated count
    /// is part of the reason face), guard 2a at the 950.0 score edge
    /// with zero vias (exact edge + one step either side), guard 3a at
    /// the 995.0 edge with ONE via on the board — which is also the
    /// vias 0-vs-1 face at score >= 950 (the via count closes guard
    /// 2's zero-via gate, so the 3a message fires instead of 2a's).
    /// Scores are crafted through the V2 lower-bound formula
    /// (min=100, weight=1000: `score = 1000 - 10 x (total - min)`)
    /// with via/bend weights zeroed, so each edge value lands exactly
    /// in f32. The just-below worlds sit MID-WINDOW (score ~X.995,
    /// 82 ulps each way from both the true threshold and the re-killed
    /// `>=X.99` mutants) so a future faithful reassociation
    /// of the penalty arithmetic cannot silently flip them under a
    /// mutant.
    #[test]
    fn m6_t1_preflight_exact_score_and_count_edges() {
        let (mut manager, mut board) = pairless_world(); // 0 incompletes, 0 vias
        let mut stop = StopFace::default();
        let batch = BatchSettings::new(settings_ir(), v2_scoring());
        let mut opt = opt_settings();
        opt.enable_preflight_guards = Some(true); // guards ON — the pin targets them
        let mut stage = BatchOptimizerStage::new(&mut manager, &mut board, batch, opt, &mut stop);
        stage.settings.scoring = crafted_scoring(1000.0);
        let stats =
            |incomplete: Option<i32>, total_len: f32, min_via: Option<i32>, via_count: i32| {
                crafted_stats(incomplete, 100.0, total_len, min_via, via_count)
            };

        // Guard 1 at the exact 0-vs-1 edge; the full row is pinned so
        // an interpolation or wording drift fails the pin.
        let s = stats(Some(1), 100.0, Some(1), 0);
        assert_eq!(
            stage.evaluate_pre_flight_guards(&s).as_deref(),
            Some(
                "the board has 1 unrouted connection(s) (optimizer only runs on completely \
                 routed boards)"
            ),
            "guard 1 fires at exactly 1 incomplete with the exact Java row",
        );
        // One step down (0 incompletes) on a world where every later
        // guard also fails -> the guards pass.
        let s = stats(Some(0), 105.0005, Some(1), 0);
        assert_eq!(
            stage.evaluate_pre_flight_guards(&s),
            None,
            "0 incompletes on an all-guards-fail world does not fire",
        );

        // The 950.0 edge, zero vias. Exactly AT the edge: fires
        // (Java `>= 950.0f`); the interpolated score is part of the
        // pinned face.
        let s = stats(Some(0), 105.0, Some(1), 0);
        let reason = stage.evaluate_pre_flight_guards(&s);
        assert!(
            reason.as_deref().is_some_and(
                |r| r.contains("initial optimizer score (950.00) is already >= 950.00")
            ),
            "guard 2a at the exact 950.0 edge fires: {reason:?}",
        );
        // One step above the edge (score ~950.01): fires.
        let s = stats(Some(0), 104.999, Some(1), 0);
        let reason = stage.evaluate_pre_flight_guards(&s);
        assert!(
            reason.as_deref().is_some_and(
                |r| r.contains("initial optimizer score (950.01) is already >= 950.00")
            ),
            "guard 2a just above the 950.0 edge fires: {reason:?}",
        );
        // MID-WINDOW just-below world (score ~949.995, not a
        // threshold-adjacent rounding survivor): 2a stays shut and 2b
        // (1.05), 3a, 3b all fail on this shape -> guards pass.
        let s = stats(Some(0), 105.0005, Some(1), 0);
        assert_eq!(
            stage.evaluate_pre_flight_guards(&s),
            None,
            "guard 2a just below the 950.0 edge does not fire",
        );

        // The 995.0 edge with one via (score 995 - 10 x (total-100) =
        // 995.0 exactly at total=100.5): guard 3a fires.
        let s = stats(Some(0), 100.5, Some(1), 1);
        let reason = stage.evaluate_pre_flight_guards(&s);
        assert!(
            reason.as_deref().is_some_and(|r| r.contains(
                "initial optimizer score (995.00) is already at or near theoretical maximum \
                 (995.00)"
            )),
            "guard 3a at the exact 995.0 edge fires: {reason:?}",
        );
        // MID-WINDOW just-below world (score ~994.995): 3a stays shut;
        // the fall-through lands on 3b (total within the 1.02 face and
        // the via count at the minimum); the interpolated total is
        // part of the pinned face.
        let s = stats(Some(0), 100.5005, Some(1), 1);
        let reason = stage.evaluate_pre_flight_guards(&s);
        assert!(
            reason.as_deref().is_some_and(|r| r.contains(
                "total trace length (100.50 mm) is already within 2% of the theoretical \
                 minimum (100.00 mm)"
            )),
            "guard 3a just below the 995.0 edge does not fire (3b catches): {reason:?}",
        );
    }

    // -----------------------------------------------------------------------
    // the candidate ripup costs (Java :884-892)
    // -----------------------------------------------------------------------

    /// The cost ladder: base start ripup costs; x10 while increased;
    /// x0.6 (Math.round) for a trace candidate AFTER the start factor;
    /// the drop to plain costs once the increased face clears.
    #[test]
    fn t9_candidate_ripup_costs_ladder() {
        // M8-T7: the ladder now rides the free face shared with the
        // partitioned executor (`candidate_ripup_costs_for`) — a pure
        // function of the four inputs, so no stage is needed.
        assert_eq!(
            candidate_ripup_costs_for(50, true, 10, 0.6, false),
            500,
            "50 x 10 (via candidate)"
        );
        assert_eq!(
            candidate_ripup_costs_for(50, true, 10, 0.6, true),
            300,
            "round(0.6 x 50 x 10) (trace)"
        );
        assert_eq!(
            candidate_ripup_costs_for(50, false, 10, 0.6, false),
            50,
            "dropped: plain costs"
        );
        assert_eq!(
            candidate_ripup_costs_for(50, false, 10, 0.6, true),
            30,
            "round(0.6 x 50) — trace discount alone"
        );

        // MIN-4 (spec review S3): the ROUND-vs-TRUNC discriminator rows.
        // Domain: start_ripup_costs is an INTEGER through every settings
        // path (CLI/DSN parse i32, DefaultSettings 100). While the
        // increased face holds, the base is 10·start — always even, so
        // f32(0.6)·(10·start) lands within ~1.2e-5·start of 6·start —
        // a fraction ≪ 0.5, so round == trunc == 6·start
        // unavoidably. The DROPPED arm's base is start itself, and for
        // start ≡ 1 or 3 (mod 5) the real product's fraction is 0.6/0.8:
        // f32(0.6) = 0.60000002384… adds only ~2.4e-8·base, so the
        // fraction stays above 0.5 and `Math.round` rounds UP where
        // truncation would cut down. start = 51 -> 30.6000012… -> 31
        // (trunc 30); start = 3 -> 1.8000001… -> 2 (trunc 1).
        for (start, expected) in [(51, 31), (3, 2)] {
            assert_eq!(
                candidate_ripup_costs_for(start, false, 10, 0.6, true),
                expected,
                "start={start}: java_round rounds the 0.6-fraction UP (trunc would give {})",
                expected - 1,
            );
        }
    }

    // -----------------------------------------------------------------------
    // the threshold sanitation (Java :350-373)
    // -----------------------------------------------------------------------

    /// A fraction (< 0.1) auto-scales x100 with an info row; an invalid
    /// (negative) threshold resets to 2.5 with a warn; the sanitized
    /// value is what the stop decision quotes.
    #[test]
    fn t9_threshold_sanitation_rows() {
        let (mut manager, mut board) = routed_world_fanout_off();
        let mut stop = StopFace::default();
        let opt = opt_with(opt_settings(), |o| {
            o.enable_preflight_guards = Some(true);
            o.improvement_threshold = Some(0.025); // fraction -> 2.5%
            o.max_passes = Some(2); // pass 2 produces the threshold-stop row
        });
        let (mut stage, mut sink) = stage_on_world(
            &mut manager,
            &mut board,
            &mut stop,
            opt,
            BatchSettings::new(settings_ir(), v2_scoring()),
        );
        stage.run_batch_loop(&mut sink);
        assert!(
            sink.any_contains(
                "Optimizer improvement threshold appears to be specified as a fraction (0.0250). \
                 Auto-scaling to percentage (2.50%).",
            ),
            "the fraction auto-scale row: {sink:?}",
        );
        assert!(
            sink.any_contains("is below the threshold (2.50%)."),
            "the stop decision quotes the SANITIZED threshold: {sink:?}",
        );

        // The invalid arm: negative resets to the default with a warn.
        let (mut manager, mut board) = routed_world_fanout_off();
        let mut stop = StopFace::default();
        let opt = opt_with(opt_settings(), |o| {
            o.enable_preflight_guards = Some(true);
            o.improvement_threshold = Some(-1.0);
            o.max_passes = Some(2);
        });
        let (mut stage, mut sink) = stage_on_world(
            &mut manager,
            &mut board,
            &mut stop,
            opt,
            BatchSettings::new(settings_ir(), v2_scoring()),
        );
        stage.run_batch_loop(&mut sink);
        assert!(
            sink.any_contains(
                "Invalid optimizer improvement threshold: -1.0000. Resetting to default 2.50%.",
            ),
            "the invalid-threshold warn row: {sink:?}",
        );
        assert!(
            sink.any_contains("is below the threshold (2.50%)."),
            "the loop runs on the reset 2.5 default: {sink:?}",
        );

        // MIN-3 (spec review): the NON-FINITE arms of the SAME gate
        // (`is_nan() || is_infinite() || < 0`, Java `:353`) — NaN and
        // +Infinity through the resolved settings face, each resetting
        // to 2.5 with the warn, the loop then quoting the reset value.
        for (name, value) in [("NaN", f32::NAN), ("Infinity", f32::INFINITY)] {
            let (mut manager, mut board) = routed_world_fanout_off();
            let mut stop = StopFace::default();
            let opt = opt_with(opt_settings(), |o| {
                o.enable_preflight_guards = Some(true);
                o.improvement_threshold = Some(value);
                o.max_passes = Some(2);
            });
            let rendered = format!("{value:.4}"); // "NaN" / "inf"
            let (mut stage, mut sink) = stage_on_world(
                &mut manager,
                &mut board,
                &mut stop,
                opt,
                BatchSettings::new(settings_ir(), v2_scoring()),
            );
            stage.run_batch_loop(&mut sink);
            let expected_row = format!(
                "Invalid optimizer improvement threshold: {rendered}. Resetting to default 2.50%."
            );
            assert!(
                sink.any_contains(&expected_row),
                "the {name} arm resets to 2.5: {sink:?}",
            );
            assert!(
                sink.any_contains("is below the threshold (2.50%)."),
                "the loop runs on the reset default after {name}: {sink:?}",
            );
        }
    }

    // -----------------------------------------------------------------------
    // maxConsecutiveFailures arms (Java :693-702)
    // -----------------------------------------------------------------------

    /// With every layer deactivated the reroute can never reconnect a
    /// ripped connection, so ALL three real candidates fail to improve
    /// and the consecutive-failure counter climbs from candidate 1: the
    /// pass-1 bound quotes `maxConsecutiveFailuresPass1`, the pass-2
    /// bound quotes `maxConsecutiveFailures` (Java `:693-702`'s two
    /// arms). The null-settings fallbacks (12 / 50) are covered by the
    /// ABSENCE face on the same 3-candidate world: no early stop can
    /// fire at 12 within 3 candidates, which kills a mutant falling
    /// back to a small bound.
    #[test]
    fn t9_max_consecutive_failures_arms() {
        let dead_layers = || {
            let mut settings = BatchSettings::new(settings_ir(), v2_scoring());
            settings.router_settings.layer_active = vec![false, false];
            settings
        };
        let (mut manager, mut board) = routed_world_fanout_off();
        let mut stop = StopFace::default();
        let mut sink = CaptureDriverSink::default();
        let mut opt = opt_settings();
        opt.enable_preflight_guards = Some(true);
        opt.max_consecutive_failures_pass1 = Some(2);
        opt.max_consecutive_failures = Some(3);
        opt.max_passes = Some(2);
        // A ZERO threshold never trips the stop (0.0 < 0.0 is false), so
        // pass 2 runs even though pass 1 improved nothing — the only way
        // to reach the later-bound arm on an all-fail world.
        opt.improvement_threshold = Some(0.0);
        let mut stage =
            BatchOptimizerStage::new(&mut manager, &mut board, dead_layers(), opt, &mut stop);
        let outcome = stage.run_batch_loop(&mut sink);
        let info = sink.joined("info");
        assert!(
            info.contains("Stopping optimization pass #1 early after 2 consecutive items could not be improved."),
            "pass 1 quotes the pass-1 bound: {info}",
        );
        assert!(
            info.contains("Stopping optimization pass #2 early after 3 consecutive items could not be improved."),
            "pass 2 quotes the later bound: {info}",
        );
        assert_eq!(outcome.passes_completed, 2);

        // The null-settings face: bounds fall back to 12/50 — with 3
        // candidates neither is reachable, so NO early-stop row fires
        // (and a fallback-to-2/3 mutant would have produced one).
        let (mut manager, mut board) = routed_world_fanout_off();
        let mut stop = StopFace::default();
        let mut sink = CaptureDriverSink::default();
        let mut opt = opt_settings();
        opt.enable_preflight_guards = Some(true);
        opt.max_consecutive_failures_pass1 = None;
        opt.max_consecutive_failures = None;
        opt.max_passes = Some(2);
        opt.improvement_threshold = Some(0.0);
        let mut stage =
            BatchOptimizerStage::new(&mut manager, &mut board, dead_layers(), opt, &mut stop);
        let outcome = stage.run_batch_loop(&mut sink);
        let info = sink.joined("info");
        assert!(
            !info.contains("could not be improved"),
            "the 12/50 fallbacks never trip within 3 candidates: {info}",
        );
        assert_eq!(outcome.passes_completed, 2);
    }

    // -----------------------------------------------------------------------
    // E2E: reject + restore + threshold stop (Java runBatchLoop :407-479)
    // -----------------------------------------------------------------------

    /// Full stage on the routed fanout-off world with guards ON: each
    /// pass's outcome loses the strict-score gate and the incumbent is
    /// restored, then the 0% improvement stops the loop. (Review-NIT
    /// precision: pass 2's REGRESSED row `951.88 -> 942.92` directly
    /// OBSERVES an adopted-then-restored candidate; pass 1's UNCHANGED
    /// row is consistent with either adoption-at-equal-score or no
    /// adoption — the pin claims only what the rows show.) THE RESTORE
    /// TEETH:
    /// the final board hash equals the baseline hash — a skipped
    /// restore, a mutated-during-evaluation live board, or a corrupt
    /// manager all fail this pin.
    #[test]
    fn t9_e2e_reject_restore_threshold_stop() {
        let (mut manager, mut board) = routed_world_fanout_off();
        let baseline_hash = board_hash(&board);
        let (baseline_len, baseline_vias, baseline_score) = {
            let mut m2 = SearchTreeManager::new();
            m2.reinsert_tree_items(&mut board);
            let s = BoardStatistics::new(&mut m2, &mut board);
            (
                s.traces.total_length_mm,
                s.vias.total_count,
                s.get_optimizer_score(Some(&v2_scoring())),
            )
        };
        assert_eq!(baseline_vias, 2);
        assert_eq!(
            baseline_hash, "981c19c52694569cdb146e196320738470cc0aac711803c7eb3f1e88d16c2791",
            "the world's deterministic baseline face",
        );

        let batch = BatchSettings::new(settings_ir(), v2_scoring());
        let mut stop = StopFace::default();
        let mut sink = CaptureDriverSink::default();
        let mut opt = opt_settings();
        opt.enable_preflight_guards = Some(true);
        opt.max_passes = Some(3);
        let mut stage = BatchOptimizerStage::new(&mut manager, &mut board, batch, opt, &mut stop);
        let outcome = stage.run_batch_loop(&mut sink);

        assert_eq!(outcome.passes_completed, 2, "stop after pass 2 (0% < 2.5%)");
        assert!(!outcome.is_timed_out);
        let info = sink.joined("info");
        assert!(
            info.contains("Optimization stage started on board"),
            "guards passed (no skip row): {info}",
        );
        assert_eq!(
            info.matches("candidate rejected: OPTIMIZER_SCORE_NOT_IMPROVED")
                .count(),
            2,
            "both passes rejected on the score gate: {info}",
        );
        // Pass 1 (odd -> preferred directions kept): adopted candidate
        // scored equal -> UNCHANGED. Pass 2 (even -> preferred direction
        // REMOVED + increased ripup dropped): the different reroute
        // scored WORSE -> REGRESSED. Both rows are pinned verbatim.
        assert!(
            info.contains(
                "Optimizer pass #1: optimizer score 951.88 -> 951.88 (UNCHANGED, 0.0000%)"
            ),
            "pass 1 delta row: {info}",
        );
        assert!(
            info.contains(
                "Optimizer pass #2: optimizer score 951.88 -> 942.92 (REGRESSED, -0.9414%)"
            ),
            "pass 2 delta row (the with-preferred-directions alternation changes the reroute): {info}",
        );
        assert!(
            info.contains(
                "Stopping optimizer because the improvement in this pass (-0.9414%) is below the threshold (2.50%).",
            ),
            "the threshold-stop row: {info}",
        );
        assert!(
            !sink.any_contains("Restoring best board achieved"),
            "the final board IS the incumbent — no best-restore needed: {info}",
        );

        // THE RESTORE FACE: final state == baseline, byte-for-byte.
        assert_eq!(board_hash(&board), baseline_hash, "incumbent restored");
        let final_score = {
            let mut m2 = SearchTreeManager::new();
            m2.reinsert_tree_items(&mut board);
            let s = BoardStatistics::new(&mut m2, &mut board);
            assert_eq!(s.traces.total_length_mm, baseline_len);
            // The tree manager answers queries (a corrupt restore
            // leaves tombstones/double entries).
            assert_eq!(s.vias.total_count, 2);
            s.get_optimizer_score(Some(&v2_scoring()))
        };
        assert_eq!(final_score, baseline_score);
    }

    /// MIN-1 (spec review A3): THE winner-SELECTION world — first-improver
    /// and improvedOver-best pick DIFFERENT candidates here. Fanout-on
    /// routed world with `via_costs = 50`; candidates in order
    /// [123 (net-98 via), 152 (L0 trace), 132 (net-98 via)], every one
    /// improving, with outcome faces (incomplete, via, length-after):
    /// 123 -> (0, 2, 1320310.5), 152 -> (0, 2, 1300790.375) — SHORTER,
    /// 132 -> (0, 2, 1320310.5). compareTo ranks 152 strictly best on the
    /// length rung, so best-overall adopts 152's board `a13e3699…` even
    /// though 123 improved FIRST (its board `3bb0969a…` must be
    /// discarded). Kills a first-improver mutant, a last-improver
    /// mutant, and a worst-overall mutant in one world.
    #[test]
    fn t9_winner_rule_best_overall_not_first_improver() {
        let (mut manager, mut board) = {
            let (mut manager, mut board) = parse_fixture();
            let mut ir = settings_ir();
            ir.via_costs = 50;
            let settings = BatchSettings::new(ir, v2_scoring());
            let mut sink = CaptureDriverSink::default();
            let mut driver =
                BatchDriver::new(&mut manager, &mut board, settings, StopFace::default());
            assert_eq!(
                driver.run(&mut sink),
                Ok(true),
                "the via-costly world routes"
            );
            (manager, board)
        };
        let baseline_hash = board_hash(&board);
        let candidates = {
            let mut m2 = SearchTreeManager::new();
            m2.reinsert_tree_items(&mut board);
            read_sorted_route_items(&m2, &mut board)
        };
        let candidate_ids: Vec<u32> = candidates.iter().map(|id| id.get()).collect();
        assert_eq!(candidate_ids, vec![123, 152, 132], "candidate order face");
        // World validation (mode-13 discipline): the rules can only
        // differ when the best candidate is NOT the first one — 152
        // (the shorter reroute) sorts between the two net-98 vias.
        assert_ne!(
            candidate_ids.first(),
            Some(&152),
            "152 is not first in order"
        );

        let mut ir = settings_ir();
        ir.via_costs = 50;
        let batch = BatchSettings::new(ir, v2_scoring());
        let mut stop = StopFace::default();
        let mut sink = CaptureDriverSink::default();
        let mut opt = opt_settings();
        opt.enable_preflight_guards = Some(true);
        opt.max_passes = Some(1);
        let mut stage = BatchOptimizerStage::new(&mut manager, &mut board, batch, opt, &mut stop);
        let outcome = stage.run_batch_loop(&mut sink);
        assert_eq!(outcome.passes_completed, 1);
        let info = sink.joined("info");
        assert!(
            info.contains(
                "Optimizer pass #1 on board 'a13e3699283d2aa32c7a2e8739531e02d5321b6c3bdf8db9b123b358d65e212d'",
            ),
            "the adopted board is the BEST candidate's (152, the shorter reroute), not the \
             first improver's (123): {info}",
        );
        assert!(
            !info.contains("3bb0969ab1490643"),
            "the first improver's board (123) was discarded: {info}",
        );
        // The pass gate is a SEPARATE discipline: this world's baseline
        // optimizer score is ALSO 974.39, so the adopted best loses the
        // gate at an equal score and the incumbent is restored.
        assert!(
            info.contains("candidate rejected: OPTIMIZER_SCORE_NOT_IMPROVED"),
            "the adopted best still loses the pass gate at an equal score: {info}",
        );
        assert_eq!(board_hash(&board), baseline_hash, "incumbent restored");
    }

    /// The one-winner/order face END-TO-END: `max_items = 1` restricts
    /// the pass to THE FIRST candidate in ReadSortedRouteItems order
    /// (item 120 on this world). Its outcome is adopted at pass level
    /// and then rejected (equal score) — the completed row carries the
    /// ADOPTED board's hash (≠ baseline), proving exactly the first
    /// candidate's board was built; the final board is the restored
    /// baseline. An inverted sort order evaluates a different item and
    /// the completed-row hash rotates.
    #[test]
    fn t9_e2e_first_candidate_applied() {
        let (mut manager, mut board) = routed_world_fanout_off();
        let baseline_hash = board_hash(&board);
        let first_candidate = {
            let mut m2 = SearchTreeManager::new();
            m2.reinsert_tree_items(&mut board);
            read_sorted_route_items(&m2, &mut board)[0]
        };
        assert_eq!(first_candidate.get(), 120, "the first candidate in order");

        let batch = BatchSettings::new(settings_ir(), v2_scoring());
        let mut stop = StopFace::default();
        let mut sink = CaptureDriverSink::default();
        let mut opt = opt_settings();
        opt.enable_preflight_guards = Some(true);
        opt.max_items = Some(1);
        opt.max_passes = Some(1);
        let mut stage = BatchOptimizerStage::new(&mut manager, &mut board, batch, opt, &mut stop);
        let outcome = stage.run_batch_loop(&mut sink);

        assert_eq!(outcome.passes_completed, 1);
        let info = sink.joined("info");
        assert!(
            info.contains(
                "Optimizer pass #1 on board 'f3fcdc3cdd2fb520061c3054c29afe21a9657eabac1bea29309ed1c7eaf846f9'",
            ),
            "the pass-completed row carries the ADOPTED (candidate-local) board hash: {info}",
        );
        assert!(
            info.contains("candidate rejected: OPTIMIZER_SCORE_NOT_IMPROVED"),
            "the adopted candidate still loses the pass gate at an equal score: {info}",
        );
        assert_eq!(board_hash(&board), baseline_hash, "incumbent restored");
    }

    // -----------------------------------------------------------------------
    // M8-T7: the deterministic partitioned candidate executor
    // -----------------------------------------------------------------------

    /// The TEST-TIME feasibility assertion (a `#[test]` fn with trait
    /// bounds — it compiles when the test target builds, not a
    /// compile-time `const` check): the partitioned executor shares the
    /// base board across workers (read-only) and hands the adopted
    /// winner board back to the coordinator, so `Board` must be
    /// `Send + Sync`.
    #[test]
    fn t7_board_is_send_and_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<Board>();
        assert_sync::<Board>();
    }

    /// The via-costly winner-rule world (the
    /// `t9_winner_rule_best_overall_not_first_improver` face): 3
    /// candidates in order `[123, 152, 132]`, the best (152) is NOT
    /// the first improver (123).
    fn via_costly_routed_world() -> (SearchTreeManager, Board) {
        let (manager, mut board) = {
            let (mut manager, mut board) = parse_fixture();
            let mut ir = settings_ir();
            ir.via_costs = 50;
            let settings = BatchSettings::new(ir, v2_scoring());
            let mut sink = CaptureDriverSink::default();
            let mut driver =
                BatchDriver::new(&mut manager, &mut board, settings, StopFace::default());
            assert_eq!(
                driver.run(&mut sink),
                Ok(true),
                "the via-costly world routes"
            );
            (manager, board)
        };
        let mut m2 = SearchTreeManager::new();
        m2.reinsert_tree_items(&mut board);
        let candidates = read_sorted_route_items(&m2, &mut board);
        let candidate_ids: Vec<u32> = candidates.iter().map(|id| id.get()).collect();
        assert_eq!(candidate_ids, vec![123, 152, 132], "candidate order face");
        (manager, board)
    }

    /// The stage over a fresh clone of the routed world, configured
    /// exactly as the t9 winner-rule pin (1 pass, guards on) plus the
    /// M8-T7 partition count.
    fn run_via_costly_world_at_threads(
        world: (&SearchTreeManager, &Board),
        threads: usize,
        tweak: impl FnOnce(&mut BatchSettings, &mut OptimizerSettingsIr),
    ) -> (OptimizerOutcome, CaptureDriverSink, String) {
        let mut board = world.1.clone();
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let mut ir = settings_ir();
        ir.via_costs = 50;
        let mut batch = BatchSettings::new(ir, v2_scoring());
        batch.optimizer_threads = threads;
        let mut stop = StopFace::default();
        let mut sink = CaptureDriverSink::default();
        let mut opt = opt_settings();
        opt.enable_preflight_guards = Some(true);
        opt.max_passes = Some(1);
        tweak(&mut batch, &mut opt);
        let mut stage = BatchOptimizerStage::new(&mut manager, &mut board, batch, opt, &mut stop);
        let outcome = stage.run_batch_loop(&mut sink);
        let hash = board_hash(&board);
        (outcome, sink, hash)
    }

    /// The shared stream normalizer for the M8-T7 pins: drops exactly
    /// the three wall-clock/temporal families — the pass-elapsed info
    /// row ("was completed in"), the temporal stage-END row
    /// ("Optimization stage … completed in") — and rewrites the
    /// pass-entry trace row's thread count to a constant. EVERYTHING
    /// else, including the non-temporal stage-START row (board hash +
    /// baseline scores), flows through and must be byte-identical.
    fn normalize_stream_rows(sink: &CaptureDriverSink) -> Vec<(&'static str, String)> {
        sink.rows
            .iter()
            .filter(|(_, row)| {
                !(row.contains("was completed in")
                    || (row.contains("Optimization stage") && row.contains("completed in")))
            })
            .map(|(tag, row)| {
                let normalized = if let Some(rest) =
                    row.strip_prefix("BatchOptRoute.opt_route_pass #")
                {
                    if let Some(items_at) = rest.find(" items, ") {
                        let head = format!("BatchOptRoute.opt_route_pass #{}", &rest[..items_at]);
                        if let Some(tail_from) = rest.find(" running on ") {
                            format!(
                                "{head}{} running on N thread(s).",
                                &rest[items_at..tail_from]
                            )
                        } else {
                            row.clone()
                        }
                    } else {
                        row.clone()
                    }
                } else {
                    row.clone()
                };
                (*tag, normalized)
            })
            .collect()
    }

    /// THE THREADS-INVARIANCE PIN (the charter's acceptance gate, the
    /// M5-T7 hash-pin pattern): the same ≥3-candidate optimizer world
    /// at partition counts 1/2/3/4 produces the identical final board
    /// state and the identical event stream (timing-embedded rows
    /// filtered). The reduction order — candidate-walk order — is
    /// unchanged by the partitioning, and the winner
    /// (`ItemRouteResult::compare`, item-id-final total order) is
    /// grouping-invariant, so any divergence here is a real break of
    /// the by-construction identity argument.
    #[test]
    fn t7_optimizer_threads_rows_and_hash_identical_across_threads() {
        let (manager, board) = via_costly_routed_world();
        let baseline_hash = board_hash(&board);
        type FaceRecord = (
            OptimizerOutcome,
            Vec<(&'static str, String)>,
            Vec<(&'static str, String)>,
            String,
        );
        let mut faces: Vec<FaceRecord> = Vec::new();
        for threads in [1usize, 2, 3, 4] {
            let (outcome, sink, hash) =
                run_via_costly_world_at_threads((&manager, &board), threads, |_, _| {});
            let raw_rows: Vec<(&'static str, String)> = sink.rows.clone();
            assert_eq!(outcome.passes_completed, 1, "threads={threads}");
            // Three faces are EXPECTED to differ across partition
            // counts and are normalized for the comparison: the
            // pass-elapsed info row ("was completed in" — embeds the
            // pass wall), the temporal stage-END row ("Optimization
            // stage … completed in" — embeds the stage wall), and the
            // pass-entry trace row's thread count (the strip_prefix
            // normalization). Everything else — INCLUDING the
            // stage-START row (the non-temporal hash + baseline-scores
            // face) — must be byte-identical.
            let rows: Vec<(&'static str, String)> = normalize_stream_rows(&sink);
            faces.push((outcome, rows, raw_rows, hash));
        }
        let (outcome_1, rows_1, _raw_rows_1, hash_1) = &faces[0];
        for (index, face) in faces.iter().enumerate() {
            let n = index + 1;
            assert_eq!(&face.3, hash_1, "final board hash identical at threads={n}");
            assert_eq!(&face.1, rows_1, "event stream identical at threads={n}");
            assert_eq!(&face.0.passes_completed, &outcome_1.passes_completed);
            assert_eq!(&face.0.is_timed_out, &outcome_1.is_timed_out);
        }
        // The adopted winner is the BEST candidate (152, not the first
        // improver 123) at EVERY partition count — the reduction-order
        // pin: partitioning reorders evaluation, never the winner. The
        // adopted-board hash rides the pass-completed row (a
        // wall-clock face, hence checked on the RAW rows).
        for (n, (_, _, raw_rows, _)) in faces.iter().enumerate() {
            let n = n + 1;
            assert!(
                raw_rows.iter().any(|(_, row)| row.contains(
                    "Optimizer pass #1 on board \
                     'a13e3699283d2aa32c7a2e8739531e02d5321b6c3bdf8db9b123b358d65e212d'"
                )),
                "threads={n}: the adopted board is candidate 152's (the grouping-invariant \
                 argmin)",
            );
        }
        // The pass gate rejects at the equal score and the incumbent
        // is restored at every partition count.
        for (_, rows, _, hash) in &faces {
            assert!(
                rows.iter().any(
                    |(_, row)| row.contains("candidate rejected: OPTIMIZER_SCORE_NOT_IMPROVED")
                )
            );
            assert_eq!(*hash, baseline_hash, "incumbent restored");
        }
    }

    /// The candidate→partition assignment (the charter's FIXED key,
    /// `item_id mod n`), pinned at the PRODUCTION fn
    /// ([`optimizer_partition_of`]) used at both executor sites: a pure
    /// function of `(item_id, n)`, and the three candidates of the pin
    /// world split DIFFERENTLY at n=2 vs n=3 — the per-worker evaluation
    /// order changes with n while the reduction order (candidate walk
    /// order) does not.
    #[test]
    fn t7_partition_assignment_is_fixed_key() {
        let assignment = |id: u32, n: usize| optimizer_partition_of(ItemId::new(id), n);
        // n=3: 123→0, 152→2, 132→0 (two workers busy, distinct from
        // walk order pairing).
        assert_eq!(assignment(123, 3), 0);
        assert_eq!(assignment(152, 3), 2);
        assert_eq!(assignment(132, 3), 0);
        // n=2 splits differently (152 joins 132).
        assert_eq!(assignment(123, 2), 1);
        assert_eq!(assignment(152, 2), 0);
        assert_eq!(assignment(132, 2), 0);
        // n=1 is the golden sequential face: one partition holds all.
        for id in [123u32, 152, 132] {
            assert_eq!(assignment(id, 1), 0);
        }
    }

    /// THE REPLAY-ARM PIN (quality r1 MINOR-3): the `raised_stop`
    /// carry-back and the discard-past-early-break arm, pinned at
    /// N=1 vs N≥2 with a DETERMINISTIC mid-candidate stop raise — the
    /// reroute's COUNTED max-items gate (`BatchSettings.max_items =
    /// Some(1)`, no wall clock): the net-98 reroute queue is the pin
    /// pair's two pins, so the gate fires inside EVERY candidate's
    /// reroute exactly at its second net-attempt, on the candidate's
    /// worker-private face (the test faces carry no shared flag). At
    /// every N the walk must truncate after the FIRST candidate — its
    /// raise replays at its reduction position, the loop-top consults
    /// then break — so:
    /// (a) the normalized event streams are identical at 1/2/3 (Fix
    ///     B's narrowed normalization — the stage-start row included);
    /// (b) the outcome faces are identical (no winner adopted, board
    ///     restored; the batch loop ends at pass 1 BY THE STOP —
    ///     `max_passes` is `None`, so `passes_completed == 1` at every
    ///     N proves the carry-back raise reached the loop's own
    ///     consults exactly as in the sequential face);
    /// (c) the discard arm: the post-raise candidates are evaluated at
    ///     N≥2 but contribute NO rows — their own max-items raise rows
    ///     would each be visible, so EXACTLY ONE "Max items limit
    ///     reached" row at every N is the discard face.
    #[test]
    fn t7_stop_replay_carry_back_and_discard_identical() {
        let (manager, board) = via_costly_routed_world();
        let baseline_hash = board_hash(&board);
        let run = |threads: usize| {
            let (outcome, sink, hash) = run_via_costly_world_at_threads(
                (&manager, &board),
                threads,
                // The deterministic raise knobs: the counted reroute
                // gate + an unbounded batch pass loop (so the STOP, not
                // a pass cap, is what ends it).
                |batch, opt| {
                    batch.max_items = Some(1);
                    opt.max_passes = None;
                },
            );
            (outcome, normalize_stream_rows(&sink), hash)
        };
        let (outcome_1, rows_1, hash_1) = run(1);
        assert_eq!(
            outcome_1.passes_completed, 1,
            "threads=1: the carry-back raise (not a pass cap) ended the batch loop"
        );
        for threads in [2usize, 3] {
            let (outcome, rows, hash) = run(threads);
            assert_eq!(
                rows, rows_1,
                "stop-world event stream identical at threads={threads}"
            );
            assert_eq!(
                hash, hash_1,
                "stop-world final board hash identical at threads={threads}"
            );
            assert_eq!(
                outcome.passes_completed, 1,
                "threads={threads}: the carry-back raise (not a pass cap) ended the batch loop"
            );
            assert!(!outcome.is_timed_out);
        }
        // (c) the discard arm at every N (the raise rows of the
        // truncated candidates are discarded).
        let raise_rows = rows_1
            .iter()
            .filter(|(_, row)| row.contains("Max items limit reached"))
            .count();
        assert_eq!(
            raise_rows, 1,
            "exactly the FIRST candidate's raise survives the truncation: {rows_1:?}"
        );
        // No winner adopted — the raised candidate's verdict is
        // unimproved and the walk truncates: the incumbent board stands.
        assert_eq!(hash_1, baseline_hash, "no adoption in the stop world");
        // (b) corollary: the batch loop's stop is visible in the stream
        // through the interrupt-free completion face — the stage ended
        // with the pass count 1 (asserted above) and the world shows
        // the max-items stop's own info row family.
        assert!(
            rows_1
                .iter()
                .any(|(_, row)| row.contains("Stopping auto-router")),
            "the max-items gate's stop raise is in the stream"
        );
    }

    // -----------------------------------------------------------------------
    // the stage deadline (Java :341-348 / :385-389) — wall-profile-only
    // -----------------------------------------------------------------------

    /// Behind `deterministic_budgets` the timeout never arms (the fanout
    /// bank); on the wall profile a zero deadline times the stage out
    /// before pass 1 with the exact row.
    #[test]
    fn t9_deadline_faces() {
        // Deterministic profile: timeout_string is parsed but never arms.
        let (mut manager, mut board) = routed_world_fanout_off();
        let batch = BatchSettings::new(settings_ir(), v2_scoring());
        let mut stop = StopFace::default();
        let mut sink = CaptureDriverSink::default();
        let mut opt = opt_settings();
        opt.enable_preflight_guards = Some(true);
        opt.timeout_string = Some("0:0:0".to_string());
        opt.max_passes = Some(1);
        let mut stage = BatchOptimizerStage::new(&mut manager, &mut board, batch, opt, &mut stop);
        let outcome = stage.run_batch_loop(&mut sink);
        assert_eq!(outcome.passes_completed, 1, "the deadline never armed");
        assert!(!outcome.is_timed_out);
        assert!(!sink.any_contains("timed out"));

        // Wall profile: deadline = now -> timed out BEFORE pass 1.
        let (mut manager, mut board) = routed_world_fanout_off();
        let mut batch = BatchSettings::new(settings_ir(), v2_scoring());
        batch.deterministic_budgets = false;
        let mut stop = StopFace::default();
        let mut sink = CaptureDriverSink::default();
        let mut opt = opt_settings();
        opt.enable_preflight_guards = Some(true);
        opt.timeout_string = Some("0:0:0".to_string());
        let mut stage = BatchOptimizerStage::new(&mut manager, &mut board, batch, opt, &mut stop);
        let outcome = stage.run_batch_loop(&mut sink);
        assert_eq!(outcome.passes_completed, 0);
        assert!(outcome.is_timed_out);
        assert!(
            sink.any_contains("Optimizer stage timed out before starting pass #1"),
            "the timeout row: {sink:?}",
        );
    }

    // -----------------------------------------------------------------------
    // the algorithm normalization (Java :93-103)
    // -----------------------------------------------------------------------

    /// An unknown algorithm warns and the stage still runs (Java resets
    /// the field to `freerouting-optimizer`).
    #[test]
    fn t9_algorithm_normalize_warn() {
        let (mut manager, mut board) = routed_world_fanout_off();
        let batch = BatchSettings::new(settings_ir(), v2_scoring());
        let mut stop = StopFace::default();
        let mut sink = CaptureDriverSink::default();
        let mut opt = opt_settings();
        opt.algorithm = "hyper-optimizer".to_string();
        opt.max_passes = Some(1);
        let mut stage = BatchOptimizerStage::new(&mut manager, &mut board, batch, opt, &mut stop);
        stage.run_batch_loop(&mut sink);
        assert!(
            sink.any_contains(
                "The algorithm 'hyper-optimizer' is not supported by the batch autorouter. \
                 The default algorithm 'freerouting-optimizer' will be used instead.",
            ),
            "the normalization warn: {sink:?}",
        );
        assert!(
            sink.any_contains("Optimization stage started"),
            "the stage runs after normalizing: {sink:?}",
        );
        assert_eq!(BatchOptimizerStage::ALGORITHM_ID, "freerouting-optimizer");
    }

    // -----------------------------------------------------------------------
    // totalWeightedTraceLength (Java BoardStatistics.java:265-288)
    // -----------------------------------------------------------------------

    /// The weighted baseline: unfixed traces count length×(halfWidth +
    /// clearance), shove-fixed count HALF, user-fixed don't count at
    /// all; terms are f32-rounded per trace and the walk is descending.
    #[test]
    fn t9_total_weighted_trace_length_faces() {
        let (mut manager, mut board) = parse_fixture();
        // The fixture's clearance value for class 1 vs default on layer 0:
        let clearance = board.clearance_value(1, DEFAULT_CLEARANCE_CLASS, 0);
        let t_unfixed = insert_trace(
            &mut manager,
            &mut board,
            [(0, 0), (0, 10_000)],
            0,
            &[1],
            epic_board::items::FixedState::Unfixed,
        );
        let t_shove = insert_trace(
            &mut manager,
            &mut board,
            [(1_000, 0), (1_000, 10_000)],
            0,
            &[2],
            epic_board::items::FixedState::ShoveFixed,
        );
        let _t_user = insert_trace(
            &mut manager,
            &mut board,
            [(2_000, 0), (2_000, 10_000)],
            0,
            &[3],
            epic_board::items::FixedState::UserFixed,
        );
        let len = 10_000.0f64; // from_two_corners axis-aligned: exact
        let expected = (len * f64::from(500 + clearance)) as f32
            + ((len * f64::from(500 + clearance)) / 2.0) as f32;
        let total = total_weighted_trace_length(&board);
        assert!(
            (total - expected).abs() <= expected.abs() * 1e-6,
            "weighted length: got {total}, expected ~{expected} (clearance {clearance})",
        );
        // The ids burn monotonically but the walk order is irrelevant
        // for distinct traces; the user-fixed trace contributed 0.
        let _ = (t_unfixed, t_shove);
    }

    // -----------------------------------------------------------------------
    // the pass loop faces that need a STAGE but not routing surprises
    // -----------------------------------------------------------------------

    /// The counters the pass events carry: phase "optimizer" with the
    /// pass number (Java leaves the phase null and ships only passCount;
    /// the port's rendering is documented on RouterCounters).
    #[test]
    fn t9_pass_counters_phase_and_pass() {
        let (mut manager, mut board) = routed_world_fanout_off();
        let mut stop = StopFace::default();
        let opt = opt_with(opt_settings(), |o| {
            o.enable_preflight_guards = Some(true);
            o.max_passes = Some(1);
        });
        let (mut stage, mut sink) = stage_on_world(
            &mut manager,
            &mut board,
            &mut stop,
            opt,
            BatchSettings::new(settings_ir(), v2_scoring()),
        );
        stage.run_batch_loop(&mut sink);
        let optimizer_counters: Vec<&String> = sink
            .rows
            .iter()
            .filter(|(tag, row)| *tag == "board_updated" && row.contains("phase=optimizer"))
            .map(|(_, row)| row)
            .collect();
        assert!(
            optimizer_counters.len() >= 3,
            "pass-start + winner-applied(when adopted) + pass-end events: {optimizer_counters:?}",
        );
        assert!(
            optimizer_counters
                .iter()
                .all(|row| row.contains("phase=optimizer pass=1")),
            "every optimizer counter carries the stage phase and the pass number",
        );
    }

    /// The `Before optimization` debug row quotes the live via count and
    /// the RAW cumulative (unweighted) trace length, Math.round'ed.
    #[test]
    fn t9_before_optimization_debug_row() {
        let (mut manager, mut board) = routed_world_fanout_off();
        let mut stop = StopFace::default();
        let opt = opt_with(opt_settings(), |o| {
            o.enable_preflight_guards = Some(true);
            o.max_passes = Some(1);
        });
        let (mut stage, mut sink) = stage_on_world(
            &mut manager,
            &mut board,
            &mut stop,
            opt,
            BatchSettings::new(settings_ir(), v2_scoring()),
        );
        stage.run_batch_loop(&mut sink);
        // 2 vias; the raw cumulative length of the routed world in board
        // units is deterministic — pin the WHOLE row (review NIT-3).
        let debug = sink.joined("debug");
        assert!(
            debug.contains("Before optimization: Via count: 2, trace length: 1320311"),
            "the full deterministic debug row: {debug}",
        );
    }

    /// With `max_items` already exhausted the in-pass slice reports the
    /// limit row and the pass evaluates nothing (Java `:636-650`).
    /// Reaching it needs the loop gate open (max_items grown past
    /// totalItemsOptimized mid-run), which the crafted `max_items = 1,
    /// max_passes = 2` world exercises from the other side: pass 2 never
    /// starts because the loop gate closes first (Java `:380-384`).
    #[test]
    fn t9_max_items_loop_gate() {
        let (mut manager, mut board) = routed_world_fanout_off();
        let baseline_hash = board_hash(&board);
        let batch = BatchSettings::new(settings_ir(), v2_scoring());
        let mut stop = StopFace::default();
        let mut sink = CaptureDriverSink::default();
        let mut opt = opt_settings();
        opt.enable_preflight_guards = Some(true);
        opt.max_items = Some(1);
        opt.max_passes = Some(5);
        opt.improvement_threshold = Some(100.0); // never stop on threshold
        let mut stage = BatchOptimizerStage::new(&mut manager, &mut board, batch, opt, &mut stop);
        let outcome = stage.run_batch_loop(&mut sink);
        // Pass 1 consumes the single item budget; the loop gate
        // (totalItemsOptimized < maxItems) then closes for good.
        assert_eq!(
            outcome.passes_completed, 1,
            "the maxItems loop gate closed after pass 1"
        );
        let info = sink.joined("info");
        assert!(
            !info.contains("Optimizer pass #2"),
            "no second pass ran: {info}",
        );
        assert_eq!(
            board_hash(&board),
            baseline_hash,
            "single candidate rejected+restored"
        );
    }
}
