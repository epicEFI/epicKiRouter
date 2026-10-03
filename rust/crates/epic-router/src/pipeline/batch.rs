//! Java `autoroute/pipeline/BatchAutorouter.java` +
//! `AutorouteBatchLoop.java` — the multi-pass batch driver (T12):
//! the item queue ([`get_autoroute_items`]), the per-connection
//! settings record ([`BatchSettings`]), the tail-removal face, and
//! the pass loop ([`BatchDriver::run`]) with the board-history
//! restore gate, the pass-local + global stagnation trackers, and the
//! task-state events. The fanout pre-pass stage (Java `:86-246`) is
//! LIVE (M4-T7): the stage body is [`crate::pipeline::fanout`], wired
//! behind the same gates (see the bank below).
//!
//! ## Banks (SEAM carries the dossier)
//!
//! * **`alreadyRoutedBoardHashes` is dead in Java** — the consult
//!   block is commented out (`AutorouteBatchLoop.java:297-306`,
//!   "Same-hash stop disabled because ripup budgets and random seeds
//!   change per-pass"); only the `clear()` calls survive. The port
//!   keeps the doc, drops the field (a live unused field would trip
//!   `dead_code`).
//! * **The fanout stage's runtime-metrics faces are banked** — Java
//!   tracks per-stage CPU seconds, allocated MB and peak heap MB
//!   (`AutorouteBatchLoop.java:105-107/:132-134/:185-211`) and renders
//!   them into the summary row; the port has no runtime-metrics face,
//!   so the summary row banks them to `0.00 total CPU seconds,
//!   0.00 GB total allocated, and 0.0 MB peak heap usage` (the
//!   duration faces stay real). Java's fanout manifest phase writes
//!   only before/after snapshots + duration + cpu
//!   (`:231-246`, never `passesCompleted`); the port manifest's
//!   non-determinism projection serializes that as the empty `{}`
//!   slot, so the renderer needs no fanout face — only the
//!   autorouter backfill's phase filter.
//! * **`IllegalArgumentException` → `Result`.** Java's run abort
//!   (`:59-61`) throws after emitting the CANCELLED event; the port
//!   returns [`BatchLoopError::NoActiveSignalLayers`] with the same
//!   event emitted first.
//! * **The job seam** (`job.state == TIMED_OUT`, `resourceUsage`,
//!   `saveIntermediateStages` snapshot events, the session summary
//!   fields) is the host's business; the port's stop face is the only
//!   stop source and the TIMED_OUT task-state variant is unreachable
//!   without a job (the CANCELLED face covers the stopped exit).
//! * **Wall-clock faces are banked with the deterministic-budget
//!   family**: the pass-duration wall clock (rendered into the
//!   pass-completed info row, asserted never in pins) stays real;
//!   the 250 ms board-update throttle and the per-interval progress
//!   statistics snapshot are dropped with the GUI progress event —
//!   the interval COUNTER survives (pinnable), the snapshot was only
//!   readable through that event.
//! * **`hasIgnoredNets` ≡ false on parsed boards** (Java writes the
//!   flag only on GUI edit paths) — the queue gate's second conjunct
//!   is a constant true; no port field.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use epic_board::board::Board;
use epic_board::contacts::item_connected_set;
use epic_board::id::ItemId;
use epic_board::items::ItemData;
use epic_board::routing_board_insert::{opt_changed_area, remove_trace_tails};
use epic_board::trace_ops::{StopConnectionOption, is_routable};
use epic_board::trace_tightener::{TraceCostFactor, TraceTightenerSeam};
use epic_board::tree_manager::SearchTreeManager;
use epic_drc::incompletes::all_incompletes;

use crate::control::{AutorouteControl, ExpansionCostFactor, RouterSettingsIr};
use crate::engine::item_to_string;
use crate::pipeline::board_hash::board_hash;
use crate::pipeline::board_history::{BoardHistory, restore_from_snapshot};
use crate::pipeline::board_statistics::{BoardStatistics, RouterSettingsScoring};
use crate::pipeline::event_sink::DriverSink;
use crate::pipeline::fanout::{
    FanoutPassStatus, FanoutProgressListener, fanout_board, smd_pin_connection_counts,
};
use crate::pipeline::pass_runner::{RouterCounters, run_pass};

// ---------------------------------------------------------------------------
// the Java constants (BatchAutorouter.java:55-80 / AutorouteBatchLoop)
// ---------------------------------------------------------------------------

/// Java `BOARD_RANK_LIMIT = BoardHistory.MAX_HISTORY_SIZE` (`:62`).
pub const BOARD_RANK_LIMIT: i32 = BoardHistory::MAX_HISTORY_SIZE as i32;
/// Java `MAXIMUM_TRIES_ON_THE_SAME_BOARD` (`:63`).
pub const MAXIMUM_TRIES_ON_THE_SAME_BOARD: i32 = 3;
/// Java `STOP_AT_PASS_MINIMUM` (`:64`).
pub const STOP_AT_PASS_MINIMUM: i32 = 8;
/// Java `STOP_AT_PASS_MODULO` (`:65`).
pub const STOP_AT_PASS_MODULO: i32 = 4;
/// Java `STAGNATION_PASS_LIMIT` (`:66`).
pub const STAGNATION_PASS_LIMIT: i32 = 10;
/// Java `FANOUT_RECOVERY_STAGNATION_PASSES` (`:67`).
pub const FANOUT_RECOVERY_STAGNATION_PASSES: i32 = 3;
/// Java `PROGRESS_STATISTICS_ITEM_INTERVAL` (`:68`).
pub const PROGRESS_STATISTICS_ITEM_INTERVAL: i32 = 10;
/// Java `STAGNATION_SCORE_THRESHOLD` (`:69`).
pub const STAGNATION_SCORE_THRESHOLD: f32 = 0.5;

// ---------------------------------------------------------------------------
// the fanout stage listener (Java AutorouteBatchLoop.java:141-183)
// ---------------------------------------------------------------------------

/// Java's inline `BatchFanout.fanoutBoard` progress listener: every
/// status fires a `board_updated` event with `phase="fanout"` counters,
/// and a completed pass logs the pass summary row. Java's closure reads
/// `router.board.getHash()` INSIDE the `passCompleted` branch (`:162`)
/// — the after-pass hash; the port's status carries that hash
/// precomputed at the same publish point.
struct BatchFanoutListener;

impl FanoutProgressListener for BatchFanoutListener {
    fn on_progress(&mut self, status: &FanoutPassStatus, board: &Board, sink: &mut dyn DriverSink) {
        let counters = RouterCounters {
            phase: "fanout".to_string(),
            pass_count: status.pass_no,
            queued_to_be_routed_count: status.pins_to_go,
            routed_count: status.routed_count,
            skipped_count: 0,
            ripped_count: 0,
            failed_to_be_routed_count: status.not_routed_count + status.insert_error_count,
            incomplete_count: status.incomplete_count,
            fanout_extra_vias_count: status.extra_vias_this_pass,
        };
        sink.board_updated(&counters);
        // The M9 snapshot hook (event_sink.rs): mirrors the
        // `board_updated` fire immediately — the default sink is a
        // no-op, so the parity stream is byte-stable.
        sink.board_snapshot(board);

        if status.pass_completed {
            // Java `String.format(Locale.US, ...)` — the plural arms
            // and the RAW ripup costs ride verbatim (`:162-183`).
            let plural = |count: i32| if count == 1 { "" } else { "s" };
            sink.info(&format!(
                "Fanout pass #{} on board '{}' completed in {:.2} seconds with {} SMD pin{} \
                 fanouted, {} not routed, {} insert error{}, +{} extra via{} ({} SMD pin{} \
                 still to check in pass, ripup costs={}).",
                status.pass_no,
                status.board_hash,
                (status.pass_duration_millis as f64) / 1000.0,
                status.routed_count,
                plural(status.routed_count),
                status.not_routed_count,
                status.insert_error_count,
                plural(status.insert_error_count),
                status.extra_vias_this_pass,
                plural(status.extra_vias_this_pass),
                status.pins_to_go,
                plural(status.pins_to_go),
                status.ripup_costs,
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// the stop face
// ---------------------------------------------------------------------------

/// The batch stop plumbing (Java's shared `StoppableThread`): the
/// loop polls [`Self::is_requested`], the pass runner's maxItems face
/// calls [`Self::request`], and an external owner (the CLI signal
/// handler, a GUI button) can raise the shared flag from another
/// thread. The internal `local` bit makes a driver-side stop survive
/// without any shared flag.
#[derive(Debug, Default, Clone)]
pub struct StopFace {
    /// Java `thread.stopRequested` — the shared flag (the engines get
    /// clones as their `stoppableThread`).
    flag: Option<Arc<AtomicBool>>,
    /// The EXTERNAL-ONLY face (the readiness fix-round's CLI signal
    /// wiring): the engine's own raises never write the shared flag —
    /// `request` keeps the local bits, and only the external owner's
    /// store is visible through the loads. A face the engine can WRITE
    /// lets an internal stop (stagnation/max-items) leak into the
    /// host's cancel flag and abort the optimizer stage on a
    /// signal-free run — the byte-identity break witnessed live (the
    /// push_shove golden drift, fix-round evidence 05c).
    external_only: bool,
    /// The driver-local stop bit (Java's `stopRequested` on the same
    /// thread object; the split is a port ownership artifact).
    local: bool,
    /// Set only by the pass runner's maxItems face — the stop-CAUSE
    /// mark [`Self::take_max_items_faced`] consumes.
    max_items_faced: bool,
    /// Java `StopRequestState.ALL` — the FULL stop (external requests
    /// and the max-items path). Sticky; the pipeline's optimizer-stage
    /// gate reads exactly this face (Java `RoutingPipeline.java:122`).
    full_stop: bool,
}

impl StopFace {
    /// A face over a pre-existing shared flag (`None` = local only).
    /// The engine MAY write the flag: every internal raise stores it
    /// (the two-face port's outbound direction — the session/GUI
    /// wiring relies on it).
    #[must_use]
    pub fn from_flag(flag: Option<Arc<AtomicBool>>) -> Self {
        Self {
            flag,
            external_only: false,
            local: false,
            max_items_faced: false,
            full_stop: false,
        }
    }

    /// A face over a pre-existing shared flag the engine never
    /// WRITES: only the external owner's store is visible (through
    /// the loads); every internal raise stays on the local bits.
    /// Field-identical to [`Self::default`] when the flag never
    /// raises — the byte-identity face the host-layer cancel wiring
    /// needs. The readiness fix-round's CLI seam uses this.
    #[must_use]
    pub fn from_external_flag(flag: Option<Arc<AtomicBool>>) -> Self {
        Self {
            flag,
            external_only: true,
            local: false,
            max_items_faced: false,
            full_stop: false,
        }
    }

    /// The external-only mode mark (the pipeline's stage-face
    /// propagation reads it).
    #[must_use]
    pub fn is_external_only(&self) -> bool {
        self.external_only
    }

    /// Java `isStopAutoRouterRequested()` (`StoppableThread.java:40-42`).
    #[must_use]
    pub fn is_requested(&self) -> bool {
        self.local
            || self
                .flag
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Relaxed))
    }

    /// Java `isStopRequested()` (`StoppableThread.java:28-30`) — the
    /// FULL stop: a driver-side [`Self::request_full`] (the max-items
    /// face) or the shared flag (an external owner's raise). The
    /// flagged-face residual (this port's auto-router-only raises also
    /// store the shared flag) is documented ONCE, in `pipeline::full`'s
    /// module docs (STOP-GATE SEMANTICS, residual 1).
    #[must_use]
    pub fn is_full_stop_requested(&self) -> bool {
        self.full_stop
            || self
                .flag
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Relaxed))
    }

    /// Java `requestStopAutoRouter()` (`StoppableThread.java:33-37`).
    /// The external-only face keeps the store local (the shared flag
    /// is the EXTERNAL owner's channel alone).
    pub fn request(&mut self) {
        self.local = true;
        if !self.external_only
            && let Some(flag) = &self.flag
        {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// Java `requestStop()` (`StoppableThread.java:23-25`) — the FULL
    /// stop (external requests and the max-items path,
    /// `AutoroutePassRunner.java:211-218`). Satisfies BOTH stop faces
    /// (`isStopAutoRouterRequested` reads `state != NONE`), raises the
    /// shared flag (ALL interrupts a running search — the engines'
    /// `isStopRequested` checks), and leaves the sticky marker the
    /// optimizer-stage gate consumes.
    pub fn request_full(&mut self) {
        self.full_stop = true;
        self.request();
    }

    /// The shared flag, handed to the per-connection engines as their
    /// `stoppableThread` (Java's `router.thread` IS the shared stop
    /// source the engines consult). The M10-T1 wiring face: the
    /// optimizer stage constructs its own face OVER the parent's flag
    /// so external raises are visible mid-optimization — Java's
    /// `BatchOptimizer` reads the shared `isStopRequested()` face via
    /// `job.thread`, `BatchOptimizer.java:384`.
    #[must_use]
    pub fn flag(&self) -> Option<&Arc<AtomicBool>> {
        self.flag.as_ref()
    }

    /// Whether THIS face object itself was raised (the `local` bit or
    /// the sticky `full_stop` mark — never the shared flag). The M8-T7
    /// partitioned candidate executor's carry-back: a worker's private
    /// face over the shared flag reports a max-items raise it performed
    /// during its candidate's reroute, which the coordinator replays
    /// onto its own face at the raising candidate's reduction position
    /// (the sequential raise timing), while shared-flag raises (an
    /// external owner) are out of the byte contract (the executor's
    /// doc).
    #[must_use]
    pub fn is_locally_raised(&self) -> bool {
        self.local || self.full_stop
    }

    /// The pass runner's maxItems face (`AutoroutePassRunner:211-220`)
    /// marks ITSELF here when it raises the stop, so the driver can
    /// attribute [`StopReason::MaxItemsReached`] exactly — an external
    /// raise of the shared flag during a max-items-bounded run is NOT
    /// the face (quality MINOR-3; Java's diagnosis is the face's info
    /// row, the port's is this bit).
    pub(crate) fn mark_max_items_faced(&mut self) {
        self.max_items_faced = true;
    }

    /// Reads and clears the face mark (`take` semantics — a reused
    /// face cannot leak the mark into a later run).
    pub(crate) fn take_max_items_faced(&mut self) -> bool {
        std::mem::take(&mut self.max_items_faced)
    }
}

/// Why the run stopped (a PORT-ADDED observability seam — log-only;
/// Java's diagnosis is the info rows). `None` = still running / the
/// loop exited through `continue_autorouting == false`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// The `currentPass > maxPasses` gate (`:308-313`).
    MaxPasses,
    /// The pass runner's maxItems face (`AutoroutePassRunner:211-220`).
    MaxItemsReached,
    /// `restoreBoard` returned null (`:346-352`).
    RestoreExhausted,
    /// The restored board's rank exceeds [`BOARD_RANK_LIMIT`] (`:357-361`).
    RestoreRankLimit,
    /// The pass-local stagnation tracker (`:524-536`).
    StagnationLocal,
    /// The global best tracker (`:538-545`).
    StagnationGlobal,
    /// An external raise of the stop face.
    UserStop,
}

/// The final-state mapping (route.rs:449-462) — formerly DUPLICATED at
/// two hosts: epic-cli `route.rs` held the PUBLIC copy (its `:447` fn),
/// epic-engine `session.rs` a PRIVATE 12-line duplicate rather than an
/// import (`final_state_for` was `pub` in epic-cli, which epic-engine
/// cannot depend on). Two 12-line mappings of a frozen [`StopReason`]
/// enum; the parity pin (CANCELLED on the external stop) and the
/// workspace battery held both faces. M10-T2 hoisted the mapping HERE,
/// beside [`StopReason`], and BOTH hosts import it — the
/// epic-engine-cannot-depend-on-epic-cli constraint is RESOLVED. The
/// body is byte-verbatim from the hoisted originals.
#[must_use]
pub fn final_state_for(run_ok: bool, stop_reason: Option<StopReason>) -> &'static str {
    if run_ok {
        return "COMPLETED";
    }
    match stop_reason {
        // An EXTERNAL stop is the only CANCELLED face.
        Some(StopReason::UserStop) => "CANCELLED",
        // Internal stops (MaxPasses, MaxItems, stagnation, restore
        // exhaustion) are normal end-of-work for the scheduler.
        _ => "COMPLETED",
    }
}

/// Java `AutorouteBatchLoop.run`'s abort face: Java throws
/// `IllegalArgumentException("Cannot start autorouter: all layers are
/// disabled.")` after emitting the CANCELLED event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchLoopError {
    /// No active signal layer — the router can never place a trace.
    NoActiveSignalLayers,
}

impl std::fmt::Display for BatchLoopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoActiveSignalLayers => {
                write!(f, "Cannot start autorouter: all layers are disabled.")
            }
        }
    }
}

impl std::error::Error for BatchLoopError {}

// ---------------------------------------------------------------------------
// the settings record
// ---------------------------------------------------------------------------

/// The batch driver's resolved settings — Java `BatchAutorouter`'s
/// cached fields (`:110-158` ctor faces) over the IR router settings.
#[derive(Clone, Debug)]
pub struct BatchSettings {
    /// Java `router.settings` — the IR the control is built from.
    pub router_settings: RouterSettingsIr,
    /// Java `job.routerSettings` — the score face (history + gates).
    pub scoring: RouterSettingsScoring,
    /// Java `settings.autorouter.maxPasses` (`null` = unlimited).
    pub max_passes: Option<i32>,
    /// Java `settings.autorouter.maxItems` (`null` = unlimited).
    pub max_items: Option<i32>,
    /// Java `settings.isFanoutEnabled()` (default true).
    pub fanout_enabled: bool,
    /// The explicit ctor's `removeUnconnectedVias`; the job ctor
    /// derives `!settings.isFanoutEnabled()`.
    pub remove_unconnected_vias: bool,
    /// Java `settings.getStartRipupCosts()` (cached `:121`).
    pub start_ripup_costs: i32,
    /// Java `settings.tracePullTightAccuracy`, `null → 500` (`:122`).
    pub pull_tight_accuracy: i32,
    /// Java `settings.getNeckWidthUm()` (the necked-retry gate, um).
    pub neck_width_um: f64,
    /// Java `settings.isStrictDrc()`.
    pub strict_drc: bool,
    /// Java `settings.getRunRouter()`.
    pub run_router: bool,
    /// Java `router.maxThreads` — the T7 partitioned executor's thread
    /// count (0/1 = the golden sequential path; `>= 2` engages
    /// [`crate::pipeline::pass_runner::run_partitioned`]). The output is
    /// byte-identical at every N (the conflicted points — board
    /// mutations — serialize in the golden order; see the executor's
    /// doc). The DEFAULT is 1: today's behavior, while the merged
    /// settings surface keeps Java's `max(1, cores-1)` parity default
    /// for its own validation face.
    pub max_threads: i32,
    /// M8-T7 RUST-ONLY: the OPTIMIZER candidate loop's partition count
    /// (`optimizer.threads`, default 1 — no Java counterpart: Java's
    /// `optimizer.maxThreads` face is the settings-surface parity box
    /// whose pool is the Java worker fleet, ported here as this count).
    /// 1 (or 0) = the mandated 1-thread sequential face, byte-for-byte
    /// the pre-T7 walk. `>= 2` = the deterministic partitioned
    /// candidate executor (M8-T7): candidates partition by
    /// `item_id mod n`, evaluate against the STABLE pass-start base
    /// (each on its own worker-board deep copy — the sequential face's
    /// own `:1234` clone), and reduce in candidate order, so the final
    /// board state is thread-count-invariant BY CONSTRUCTION — under
    /// deterministic budgets with none of the executor's three racy
    /// faces (wall deadline / external stop / an in-candidate max-items
    /// raise) live, see the executor's residual note in optimizer.rs:
    /// the winner rule is `ItemRouteResult::compare`, a total order
    /// whose final rung is the unique `item_id` — the fold's argmin is
    /// unique and grouping-invariant (the argument recorded in
    /// `logs/M8-T7/investigation.md` §2). The DEFAULT is 1.
    pub optimizer_threads: usize,
    /// The explicit ctor's `withPreferredDirections` — only feeds the
    /// [`Self::trace_costs`] resolution at construction.
    pub with_preferred_directions: bool,
    /// The deterministic-corpus profile switch: `true` spends the
    /// Java ladder value as call ticks (SEAM: units deviation);
    /// `false` builds the wall-clock `TimeLimit` face.
    pub deterministic_budgets: bool,
    /// Java `router.settings.getViaCosts()` (`:37-40` read face).
    pub via_costs: i32,
    /// Java `router.settings.getPlaneViaCosts()` (plane nets).
    pub plane_via_costs: i32,
    /// Java `router.isRetainAutorouteDatabase()`.
    pub retain_autoroute_database: bool,
    /// Java `getTraceCosts()` (`:130-137` — resolved once per run).
    pub trace_costs: Vec<ExpansionCostFactor>,
    /// M6-T6 RUST-ONLY: the connectivity clamp (`router.plane_island_clamp`,
    /// default OFF — no Java counterpart). When ON, a fully-floating
    /// pour (every metal region unreachable from the net's pins/vias,
    /// `epic_board::islands`) cannot answer CONNECTED_TO_PLANE. Dead
    /// code at defaults; its route effect is measured at T8/T9.
    pub plane_island_clamp: bool,
    /// M6-T7 RUST-ONLY: the global-planning master switch
    /// (`router.congestion_global`, default OFF). When ON, the pass
    /// queue is reordered by the congestion-aware plan
    /// (`pass_runner::begin_pass`) and the plan face lands in the
    /// manifest (`epic-cli` route.rs). Dead code at defaults.
    pub congestion_global: bool,
    /// M6-T7 RUST-ONLY: the L/Z pattern-routing fast path
    /// (`router.congestion_global.pattern`, default OFF; requires the
    /// master ON). Dead code at defaults.
    pub congestion_global_pattern: bool,
    /// M6-T8 RUST-ONLY: the PathFinder negotiated-congestion scheduler
    /// (`router.congestion_global.pathfinder`, default OFF; requires
    /// the master ON). When ON, the pass loop derives per-net
    /// negotiated rip-up bases (present + history congestion mix,
    /// `global/history.rs`) that REPLACE the linear
    /// `start_ripup_costs * pass` ladder, and skips the snapshot-restore
    /// arm (the history is the convergence memory). Dead code at
    /// defaults.
    pub congestion_global_pathfinder: bool,
    /// M6-T9 RUST-ONLY: the push-and-shove insertion flag
    /// (`router.push_shove`, default OFF — no Java counterpart; a
    /// top-level flag on the family pattern, no master). When ON, a
    /// maze obstacle room the M3 shove probe has verified shovable may
    /// WAIVE its rip-up charge within the bounded per-search shove
    /// budget (`maze/ripup.rs` `PUSH_SHOVE_ROOM_BUDGET`), so the
    /// detail insertion displaces the neighbor trace
    /// (`epic_board::trace_shover::insert`) instead of ripping it.
    /// Dead code at defaults.
    pub push_shove: bool,
    /// M7-T2 RUST-ONLY: the effective tuning activation
    /// (`router.tuning` tri-state resolved against the board at the
    /// route.rs seam — input-driven: a net-class length declaration
    /// anywhere on the board activates; `Some(false)` keeps it off
    /// even there). The M7 tuning faces (min-length honoring T3,
    /// meander T4, match T5, pairs T6) read THIS field, never the
    /// resolved tri-state. Dead code at defaults (no declaration ⇒
    /// false).
    pub tuning_active: bool,
    /// M7-T4 RUST-ONLY: the MEANDER stage's resolved activation —
    /// `router.tuning.meander` resolved against the board seam in
    /// route.rs (`None` = rides [`Self::tuning_active`]; `on` forces
    /// the stage on a tuning-inert board, where the stage is a
    /// natural no-op — no declaration, no deficit rows; `off` kills
    /// the meander stage ALONE, the honoring gate stays). Dead code
    /// at defaults.
    pub meander_active: bool,
    /// M7-T6 RUST-ONLY: the RESOLVED differential-pair declarations
    /// (`router.tuning.pairs` — net names resolved to numbers at the
    /// route.rs board seam; `pipeline/pairs.rs`). The pair face's
    /// activation input is this list ALONE (the third activation
    /// input; it does NOT require [`Self::tuning_active`]): non-empty
    /// ⇒ the pass leader-first order, the follower maze coupling
    /// preference, and the post-routing match stage arm; empty (the
    /// Default) ⇒ zero activation, byte-identical routing.
    pub pairs: Vec<crate::pipeline::pairs::PairSpec>,
    /// M8-T3 RUST-ONLY: the GLOSS BUS stage's resolved activation
    /// (`router.gloss.bus`, default OFF — no Java counterpart). When
    /// ON, the post-tuning gloss stage runs the parallel-bus group
    /// detector + the hug/spread re-spacing pass
    /// (`pipeline/gloss.rs`); the report rides the aesthetics SIDECAR,
    /// never the manifest. Dead code at defaults (the two-regime law:
    /// an OFF run never reaches the stage — byte-identical).
    pub bus_active: bool,
    /// M8-T4 RUST-ONLY: the GLOSS FLOW stage's resolved activation
    /// (`router.gloss.flow`, default OFF — no Java counterpart). When
    /// ON, the gloss stage slot runs the 45° flow pass AFTER the bus
    /// pass (flow after spread — the AM2 composition law): jog/stub
    /// elimination + the miter/recorner bridge (`pipeline/gloss.rs`);
    /// the report rides the aesthetics SIDECAR, never the manifest.
    /// Dead code at defaults (the two-regime law).
    pub flow_active: bool,
    /// M8-T5 RUST-ONLY: the GLOSS VIA-PLACE stage's resolved activation
    /// (`router.gloss.via_place`, default OFF — no Java counterpart).
    /// When ON, the gloss stage slot runs the return-path-aware via
    /// placement pass AFTER the flow pass (the terminal slot — the
    /// recorded slot decision in gloss.rs); the report rides the
    /// aesthetics SIDECAR, never the manifest. Dead code at defaults
    /// (the two-regime law).
    pub via_place_active: bool,
    /// M8-T6 RUST-ONLY: the GLOSS TEARDROPS stage's resolved activation
    /// (`router.gloss.teardrops`, default OFF — no Java counterpart).
    /// When ON, the gloss stage slot runs the graded-width teardrop
    /// pass AFTER the via-place pass (the terminal slot — the recorded
    /// slot decision in gloss.rs); the report rides the aesthetics
    /// SIDECAR, never the manifest. Dead code at defaults (the
    /// two-regime law).
    pub teardrops_active: bool,
}

impl BatchSettings {
    /// The JOB ctor face (`:110-122`): everything derived from the
    /// settings like Java's `BatchAutorouter(job, thread, board)`
    /// chain — `removeUnconnectedVias = !isFanoutEnabled()`,
    /// preferred directions kept, accuracy 500 fallback. The via-cost
    /// pair carries the product defaults (50 / 5) — the parity
    /// surface is the PLANE SWAP plumbing (`route`'s
    /// `plane ? planeViaCosts : viaCosts`), not the default value;
    /// callers seed their own settings source over the pub fields.
    #[must_use]
    pub fn new(router_settings: RouterSettingsIr, scoring: RouterSettingsScoring) -> Self {
        let start_ripup_costs = router_settings.start_ripup_costs;
        let trace_costs = AutorouteControl::resolve_trace_costs(&router_settings, true);
        Self {
            router_settings,
            scoring,
            max_passes: None,
            max_items: None,
            fanout_enabled: true,
            remove_unconnected_vias: false,
            start_ripup_costs,
            pull_tight_accuracy: 500,
            neck_width_um: 0.0,
            strict_drc: false,
            run_router: true,
            max_threads: 1,
            optimizer_threads: 1,
            with_preferred_directions: true,
            deterministic_budgets: true,
            via_costs: 50,
            plane_via_costs: 5,
            retain_autoroute_database: false,
            trace_costs,
            plane_island_clamp: false,
            congestion_global: false,
            congestion_global_pattern: false,
            congestion_global_pathfinder: false,
            push_shove: false,
            tuning_active: false,
            meander_active: false,
            pairs: Vec::new(),
            bus_active: false,
            flow_active: false,
            via_place_active: false,
            teardrops_active: false,
        }
    }
}

// ---------------------------------------------------------------------------
// the driver
// ---------------------------------------------------------------------------

/// The batch driver (Java `BatchAutorouter` + `AutorouteBatchLoop`).
/// Borrows the manager and the board for the run; the fanout state
/// (`fanout_timed_out`) rides with the M4 stage.
pub struct BatchDriver<'a> {
    /// Java `router.board`'s manager side.
    pub manager: &'a mut SearchTreeManager,
    /// Java `router.board`.
    pub board: &'a mut Board,
    /// The resolved settings.
    pub settings: BatchSettings,
    /// Java `router.thread`.
    pub stop: StopFace,
    /// Java `router.totalItemsRouted` — attempts across all passes.
    pub total_items_routed: i32,
    /// Java `router.progressItemsSinceStatistics`.
    pub progress_items_since_statistics: i32,
    /// The AUTOROUTE stage's completed pass count — the loop's own
    /// `current_pass` after each pass that RAN (port-added
    /// observability; Java's manifest reads its stage outcomes the
    /// same way — `AutorouteBatchLoop.java:231-246` never consults the
    /// counters stream). 0 when no pass ran (router disabled,
    /// fanout-only's `max_passes=0` first-iteration break, or a stop
    /// before pass 1). The event stream cannot carry this face: the
    /// optimizer's per-item reroutes ride the same pass tail stamping
    /// `phase="autoroute"` (`pass_runner`'s shared `finish_pass`), so
    /// the LAST `phase="autoroute"` counters row is the optimizer's
    /// pass 1 after any multi-pass autoroute + optimizer run.
    pub passes_completed: i32,
    /// Why the run stopped (port-added observability; see [`StopReason`]).
    pub stop_reason: Option<StopReason>,
    /// Java `router.fanoutRecoveryApplied` — the one-time recovery
    /// fired.
    pub fanout_recovery_applied: bool,
    /// Java `router.fanoutTimedOut` (`BatchAutorouter.java:80`, read
    /// via `isFanoutTimedOut` `:331-333`) — the fanout stage deadline
    /// fired.
    pub fanout_timed_out: bool,
    /// The stage-boundary statistics the pipeline's per-phase manifest
    /// faces read (M4-T10; see [`StageBoundaries`]).
    pub phase_boundaries: StageBoundaries,
    /// M6-T7: the global plan captured at the pass loop's opening
    /// (default-OFF stage — `Some` only when
    /// `BatchSettings::congestion_global` is ON).
    pub global_plan: Option<crate::global::plan::GlobalPlan>,
    /// M6-T8: the PathFinder scheduler (default OFF — `Some` only when
    /// `congestion_global` AND `congestion_global_pathfinder` are ON).
    /// Carries the per-resource history across passes
    /// (`global/history.rs` docs).
    pub pathfinder: Option<crate::global::history::PathFinder>,
}

/// The stage-boundary statistics Java `AutorouteBatchLoop` captures for
/// the manifest's per-phase faces: `fanoutBeforeStats`/`fanoutAfterStats`
/// (`:93-98` opens with the fanout gate — BEFORE the SMD-pin skip arm —
/// and `:231-247` closes after the stage bodies) and
/// `autorouterBeforeStats`/`autorouterAfterStats` (the `:261` capture
/// inside the router-enabled gate; the post-loop phase fill re-reads
/// the board after the completed-board tail sweep — gate `:603`,
/// after-capture `:604`, fill `:603-626`). The `None`
/// faces mirror Java's capture gates exactly: fanout disabled, the
/// router disabled (or the fanout-only override inactive), or the loop
/// never ran (the all-layers abort throws before every capture).
#[derive(Debug, Default)]
pub struct StageBoundaries {
    /// Java `fanoutBeforeStats`.
    pub fanout_before: Option<BoardStatistics>,
    /// Java `fanoutAfterStats`.
    pub fanout_after: Option<BoardStatistics>,
    /// Java `autorouterBeforeStats`.
    pub autorouter_before: Option<BoardStatistics>,
    /// The post-loop board statistics (Java `autorouterAfterStats`,
    /// captured after the tail sweep inside the phase fill).
    pub autorouter_after: Option<BoardStatistics>,
}

impl<'a> BatchDriver<'a> {
    /// Java's two-step construction collapsed: the settings record is
    /// the caller's (build it with [`BatchSettings::new`] and adjust
    /// the pub fields), the stop face over an optional shared flag.
    #[must_use]
    pub fn new(
        manager: &'a mut SearchTreeManager,
        board: &'a mut Board,
        settings: BatchSettings,
        stop: StopFace,
    ) -> Self {
        Self {
            manager,
            board,
            settings,
            stop,
            total_items_routed: 0,
            progress_items_since_statistics: 0,
            passes_completed: 0,
            stop_reason: None,
            fanout_recovery_applied: false,
            fanout_timed_out: false,
            phase_boundaries: StageBoundaries::default(),
            global_plan: None,
            pathfinder: None,
        }
    }

    /// Java `calculateIncompleteCount` — the driver's total-incompletes
    /// read (the `DesignRulesChecker.calculateAllIncompletes` +
    /// `getIncompleteCount()` pair). Delegates to the free function the
    /// pass runner shares.
    pub fn calculate_incomplete_count(&mut self) -> i32 {
        calculate_incomplete_count(self.manager, self.board)
    }

    /// Java `removeTails(stopConnectionOption)` (`:487-503`).
    /// Delegates to the free function the pass runner shares.
    pub fn remove_tails(&mut self, stop_connection_option: StopConnectionOption) {
        remove_tails(
            self.manager,
            self.board,
            stop_connection_option,
            self.settings.pull_tight_accuracy,
            &self.settings.trace_costs,
            self.stop.flag(),
            TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP,
            self.settings.deterministic_budgets,
        );
    }

    /// Java `AutorouteBatchLoop.run` (`:40-651`) — the multi-pass
    /// loop. `Ok(true)` = the router finished its own accord
    /// (`!stopRequested`), `Ok(false)` = a stop was raised.
    #[allow(clippy::too_many_lines)] // the Java body is one flat walk
    pub fn run(&mut self, sink: &mut dyn DriverSink) -> Result<bool, BatchLoopError> {
        // Java `:47-59` — the active-signal-layer walk over the
        // SETTINGS mask against the BOARD's layer kinds.
        let layer_active = self.settings.router_settings.layer_active.clone();
        let any_routable = layer_active
            .iter()
            .enumerate()
            .any(|(i, &active)| active && self.board.layers().layers[i].is_signal);
        if !any_routable {
            sink.warn("Cannot start autorouter: all layers are disabled.");
            let hash = board_hash(self.board);
            sink.task_state("CANCELLED", 0, &hash);
            return Err(BatchLoopError::NoActiveSignalLayers);
        }
        sink.task_state("STARTED", 0, &board_hash(self.board));

        // Java `router.initialUnroutedCount` (session summary — the
        // host's summary rendering is banked; the count is kept).
        let _initial_unrouted_count = self.calculate_incomplete_count();

        let mut history = BoardHistory::new(self.settings.scoring.clone());

        // The fanout stage (Java `:86-246`, M4-T7): the `:86-90` check
        // row is unconditional, the stage bodies sit behind the
        // fanout-enabled gate.
        let smd_pins = i32::try_from(self.board.smd_pin_count()).unwrap_or(i32::MAX);
        sink.debug(&format!(
            "Checking fanout pre-pass. settings.fanout.enabled={}, smdPins={}",
            self.settings.fanout_enabled, smd_pins
        ));
        if self.settings.fanout_enabled {
            // Java `:93-98` — the fanout phase's before-boundary opens
            // with the stage gate, BEFORE the SMD-pin skip arm (a
            // no-SMD board still fills the phase row).
            self.phase_boundaries.fanout_before =
                Some(BoardStatistics::new(self.manager, self.board));
            if smd_pins == 0 {
                sink.info("Fanout stage is enabled but skipped because the board has no SMD pins.");
            } else {
                // Java `:108-133` — the stage-start walk (`:108-121`)
                // and info row (`:122-133`).
                let (net_connected_smd_pins, already_connected_at_start) =
                    smd_pin_connection_counts(self.manager, self.board);
                let pins_to_fanout = net_connected_smd_pins - already_connected_at_start;
                let hash = board_hash(self.board);
                sink.info(&format!(
                    "Fanout stage started on board '{hash}' with {pins_to_fanout} of {smd_pins} \
                     SMD pins needing fanout ({already_connected_at_start} already connected, \
                     {} netless).",
                    smd_pins - net_connected_smd_pins,
                ));

                // Java `:132-183` — the stage run with the progress
                // listener, then `:184` the timed-out flag.
                let summary = fanout_board(
                    self.manager,
                    self.board,
                    &self.settings,
                    self.stop.flag(),
                    Some(&mut BatchFanoutListener),
                    sink,
                );
                self.fanout_timed_out = summary.is_timed_out;

                // Java `:186-211` — the summary row. The CPU/GB/heap
                // faces are banked to 0.00/0.00/0.0 (no runtime-metrics
                // face; see the module bank); the thread-stop arm reads
                // the driver's stop face (`router.thread
                // .isStopAutoRouterRequested()`).
                let completion_status = if summary.is_timed_out {
                    "completed with timeout:"
                } else if self.stop.is_requested() {
                    "interrupted:"
                } else {
                    "completed:"
                };
                sink.info(&format!(
                    "Fanout stage {completion_status} started with {} total SMD pins, completed \
                     in {:.2} seconds, escaped pins: {}, using 0.00 total CPU seconds, 0.00 GB \
                     total allocated, and 0.0 MB peak heap usage.",
                    summary.escape_statistics.total_smd_pins,
                    (summary.total_duration_millis as f64) / 1000.0,
                    summary.escape_statistics.to_display(),
                ));
            }
            // Java `:231-246` — the fanout phase capture closes after
            // the stage bodies (the skip arm included) — the T7 module
            // bank ("no fanout phase faces") consumed by M4-T10.
            self.phase_boundaries.fanout_after =
                Some(BoardStatistics::new(self.manager, self.board));
        }

        // Java `:249-252`.
        let is_router_enabled = self.settings.run_router
            && (self.settings.max_passes.is_none()
                || self.settings.max_passes.is_some_and(|m| m >= 0));
        if is_router_enabled {
            let hash = board_hash(self.board);
            // Java `:261` — the autorouter phase's before-boundary (the
            // same walk the baseline-score row reads).
            let stats_before = BoardStatistics::new(self.manager, self.board);
            let score_before = stats_before.get_router_score(Some(&self.settings.scoring));
            self.phase_boundaries.autorouter_before = Some(stats_before);
            let unrouted = self.calculate_incomplete_count();
            sink.info(&format!(
                "Auto-routing stage started on board '{hash}' with baseline score \
                 {score_before:.2} for {unrouted} unrouted item{}.",
                if unrouted == 1 { "" } else { "s" }
            ));
        }
        let mut continue_autorouting = is_router_enabled;

        // M6-T7: the global-planning stage (default OFF) — the plan
        // face the manifest carries (map + guides + the planned order)
        // captures the post-fanout board state, before the pass loop.
        if self.settings.congestion_global {
            self.global_plan = Some(crate::global::plan::GlobalPlan::build(self.board));
        }

        // M6-T8: the PathFinder scheduler (default OFF; requires the
        // master + the `.pathfinder` sub-flag — the `.pattern`
        // precedent for sub-flag gating).
        let pathfinder_active =
            self.settings.congestion_global && self.settings.congestion_global_pathfinder;
        if pathfinder_active {
            self.pathfinder = Some(crate::global::history::PathFinder::new());
        }

        let mut current_pass: i32 = 1;
        let mut consecutive_no_improvement_passes: i32 = 0;
        let mut last_best_score = f32::NEG_INFINITY;
        let mut global_best_score = f32::NEG_INFINITY;
        let mut pass_of_best_score: i32 = 0;
        let mut incompletes_at_best_score: i32 = 0;
        // `alreadyRoutedBoardHashes` is dead in Java (the consult
        // block is commented out at :297-306; only clear() calls
        // survive) — doc-only here, no port field.

        while continue_autorouting && !self.stop.is_requested() {
            // Java `job.state == TIMED_OUT → requestStop` — the job
            // seam is banked; the port's stop face is the only source.
            let mut current_board_hash = board_hash(self.board);

            if let Some(max_passes) = self.settings.max_passes
                && max_passes > 0
                && current_pass > max_passes
            {
                self.stop.request();
                self.stop_reason = Some(StopReason::MaxPasses);
                break;
            }

            sink.task_state("RUNNING", current_pass, &current_board_hash);
            history.add(self.manager, self.board);

            // Java `:324-329` / `:383-389` — the pass's trace pair,
            // MODELED not verbatim (quality MINOR-4, banked in SEAM):
            // Java's traceEntry is SILENT (a perfData timestamp,
            // FRLogger.java:159-172) and traceExit emits ONE row of a
            // different shape ("Method '…' was performed in X.",
            // FRLogger.java:194-224); the port emits two
            // entry-formatted rows. Log-only, asserted never in pins;
            // the load-bearing face is the pass wall clock below
            // feeding the pass-completed row.
            let pass_started = std::time::Instant::now();
            if sink.is_trace_enabled() {
                sink.trace(&format!(
                    "BatchAutorouter.autoroute_pass #{current_pass} on board '{current_board_hash}'"
                ));
            }
            // The scheduler seam: the pass's negotiated bases derive
            // from the pass-START board + the persisted history
            // (`global/history.rs` docs); `None` = the linear ladder.
            let pathfinder_pass = match self.pathfinder.as_mut() {
                Some(scheduler) => {
                    let start_costs = self.settings.start_ripup_costs;
                    Some(scheduler.begin_pass(self.board, start_costs))
                }
                None => None,
            };
            continue_autorouting = run_pass(
                self.manager,
                self.board,
                &self.settings,
                current_pass,
                &mut self.total_items_routed,
                &mut self.progress_items_since_statistics,
                &mut self.stop,
                sink,
                pathfinder_pass.as_ref(),
            );
            if let Some(scheduler) = self.pathfinder.as_mut() {
                scheduler.end_pass(self.board);
            }
            // Every pass that RUNS records itself (the loop-START
            // max-passes break never reaches here, so a capped run
            // keeps the LAST executed pass's number — `max_passes=1`
            // leaves 1, matching the counters face the capped pins
            // observe).
            self.passes_completed = current_pass;
            if sink.is_trace_enabled() {
                sink.trace(&format!(
                    "BatchAutorouter.autoroute_pass #{current_pass} on board '{current_board_hash}'"
                ));
            }
            if self.stop.is_requested() {
                // The maxItems face is the pass-side stop source; the
                // distinguishing info row already fired there. The
                // pass runner MARKS the face when it fires — an
                // external stop on the shared flag must not be
                // attributed here (quality MINOR-3).
                if self.stop_reason.is_none() && self.stop.take_max_items_faced() {
                    self.stop_reason = Some(StopReason::MaxItemsReached);
                }
            }

            let mut board_statistics_after = BoardStatistics::new(self.manager, self.board);
            let mut board_score_after =
                board_statistics_after.get_router_score(Some(&self.settings.scoring));

            if (history.size() >= STOP_AT_PASS_MINIMUM as usize || self.stop.is_requested())
                && ((current_pass % STOP_AT_PASS_MODULO == 0
                    && current_pass >= STOP_AT_PASS_MINIMUM)
                    || self.stop.is_requested())
                // M6-T8: the negotiated stage replaces the
                // snapshot-restore oscillation dampener with the
                // persisted history (module docs at `global/history.rs`)
                // — the restore arm is skipped when it is on. Every
                // OTHER stop face in this gate still runs unchanged
                // (the T8 quality Q4 naming): the stagnation window
                // (`STOP_AT_PASS_MINIMUM` / `STOP_AT_PASS_MODULO`),
                // the stop-flag arm, and the restore-exhausted break
                // below are untouched — only the
                // `history.restore_board` arm is gated off.
                && !pathfinder_active
            {
                // Strict `>` so equally-scored boards do NOT
                // trigger a restore (Java `:338-343` comment).
                if history.get_max_score() > board_score_after {
                    match history.restore_board(MAXIMUM_TRIES_ON_THE_SAME_BOARD) {
                        None => {
                            sink.info(
                                "The router was not able to improve the board, \
                                     stopping the auto-router.",
                            );
                            self.stop.request();
                            self.stop_reason = Some(StopReason::RestoreExhausted);
                            break;
                        }
                        Some(restored) => {
                            let rank = history.get_rank(&restored);
                            if rank > BOARD_RANK_LIMIT {
                                self.stop.request();
                                self.stop_reason = Some(StopReason::RestoreRankLimit);
                                break;
                            }
                            // Java swaps `router.board`; the port
                            // restores in place + rebuilds the
                            // manager (the T12 restore model). Java's
                            // `router.board.getStatistics()` is fresh
                            // construction (`RoutingBoard.java:1410`),
                            // so `BoardStatistics::new` here is exact.
                            restore_from_snapshot(self.manager, self.board, &restored);
                            consecutive_no_improvement_passes = 0;
                            board_statistics_after = BoardStatistics::new(self.manager, self.board);
                            board_score_after = board_statistics_after
                                .get_router_score(Some(&self.settings.scoring));
                            last_best_score = board_score_after;
                            current_board_hash = board_hash(self.board);
                            sink.debug(&format!(
                                "Restoring an earlier board that has the score of {}.",
                                format_score(
                                    board_score_after,
                                    board_statistics_after
                                        .connections
                                        .incomplete_count
                                        .expect("connections included"),
                                    board_statistics_after
                                        .clearance_violations
                                        .total_count
                                        .expect("violations included"),
                                )
                            ));
                        }
                    }
                }
            }

            let pass_duration = pass_started.elapsed().as_secs_f64();
            // Java `:391-414` — the CPU-suffix arm needs `job.resourceUsage`
            // (banked; the "." else-arm is the no-job face) and the
            // `!isOptimizerAutorouter` gate is constant true for this
            // driver (the optimizer's variant of the loop lands with the
            // M-optimizer milestone and suppresses this row there).
            sink.info(&format!(
                "Auto-routing pass #{} on board '{}' was completed in {:.2} seconds with score {}.",
                current_pass,
                current_board_hash,
                pass_duration,
                format_score(
                    board_score_after,
                    board_statistics_after
                        .connections
                        .incomplete_count
                        .expect("connections included"),
                    board_statistics_after
                        .clearance_violations
                        .total_count
                        .expect("violations included"),
                )
            ));

            // Java `:416-444` — the per-net unrouted rows.
            let (_total, rows) = all_incompletes(self.manager, self.board);
            let mut breakdown = String::new();
            for row in &rows {
                if row.incomplete_count > 0 {
                    sink.trace(&format!(
                        "compare_unrouted_net pass={}, net={}, incomplete={}",
                        current_pass, row.net_no, row.incomplete_count
                    ));
                    if !breakdown.is_empty() {
                        breakdown.push(',');
                    }
                    breakdown.push_str(&format!("{}={}", row.net_no, row.incomplete_count));
                }
            }
            let total: usize = rows.iter().map(|row| row.incomplete_count).sum();
            sink.trace(&format!(
                "compare_unrouted_breakdown pass={current_pass}, total={total}, breakdown={breakdown}"
            ));

            // Java `saveIntermediateStages` snapshot events — banked
            // GUI seam (no snapshot event bus in the port).

            // Stagnation detection (Java `:460-555`).
            let mut incompletes_now = board_statistics_after
                .connections
                .incomplete_count
                .expect("connections included");
            if current_pass >= STOP_AT_PASS_MINIMUM && continue_autorouting {
                // --- pass-local counter (resets after board restores) ---
                if board_score_after > last_best_score + STAGNATION_SCORE_THRESHOLD {
                    consecutive_no_improvement_passes = 0;
                    last_best_score = board_score_after;
                } else {
                    consecutive_no_improvement_passes += 1;

                    // One-time fanout-enabled recovery (Java
                    // `:482-505`); the gate face is live even though
                    // the fanout STAGE itself is M4.
                    if self.settings.fanout_enabled
                        && !self.fanout_recovery_applied
                        && incompletes_now > 0
                        && consecutive_no_improvement_passes >= FANOUT_RECOVERY_STAGNATION_PASSES
                    {
                        let incompletes_before_recovery = incompletes_now;
                        self.remove_tails(StopConnectionOption::None);
                        board_statistics_after = BoardStatistics::new(self.manager, self.board);
                        board_score_after =
                            board_statistics_after.get_router_score(Some(&self.settings.scoring));
                        last_best_score = board_score_after;
                        consecutive_no_improvement_passes = 0;
                        self.fanout_recovery_applied = true;
                        incompletes_now = board_statistics_after
                            .connections
                            .incomplete_count
                            .expect("connections included");
                        sink.debug(&format!(
                            "Applied one-time fanout recovery cleanup (removed fanout \
                             tails/vias). Incompletes: {incompletes_before_recovery} -> \
                             {incompletes_now}."
                        ));
                    }

                    if consecutive_no_improvement_passes >= STAGNATION_PASS_LIMIT {
                        let report = build_unrouted_connections_report(self.manager, self.board);
                        sink.info(&format!(
                            "The router's score ({board_score_after:.2}) has not improved by \
                             more than {STAGNATION_SCORE_THRESHOLD} points in the last \
                             {STAGNATION_PASS_LIMIT} passes ({incompletes_now} item{} still \
                             unconnected). Stopping the auto-router.\n\
                             The following connections could not be routed -- please review \
                             your design (e.g. check pad clearances, trace width rules, and \
                             available routing space):\n{report}",
                            if incompletes_now == 1 { "" } else { "s" }
                        ));
                        self.stop.request();
                        self.stop_reason = Some(StopReason::StagnationLocal);
                        break;
                    }
                }

                // --- global best tracker (not reset by restores) ---
                if board_score_after > global_best_score + STAGNATION_SCORE_THRESHOLD {
                    global_best_score = board_score_after;
                    pass_of_best_score = current_pass;
                    incompletes_at_best_score = incompletes_now;
                } else if current_pass - pass_of_best_score >= STAGNATION_PASS_LIMIT {
                    let report = build_unrouted_connections_report(self.manager, self.board);
                    sink.info(&format!(
                        "The router's best score ({global_best_score:.2}) has not improved by \
                         more than {STAGNATION_SCORE_THRESHOLD} points since pass \
                         #{pass_of_best_score}. Stopping the auto-router after {current_pass} \
                         passes ({incompletes_at_best_score} item{} still unconnected).\n\
                         The following connections could not be routed -- please review your \
                         design (e.g. check pad clearances, trace width rules, and available \
                         routing space):\n{report}",
                        if incompletes_at_best_score == 1 {
                            ""
                        } else {
                            "s"
                        }
                    ));
                    self.stop.request();
                    self.stop_reason = Some(StopReason::StagnationGlobal);
                    break;
                }
            } else if incompletes_now == 0 && board_score_after > STAGNATION_SCORE_THRESHOLD {
                // Fully routed AND positive score — genuine success
                // (Java `:547-555`).
                consecutive_no_improvement_passes = 0;
                last_best_score = board_score_after;
            }

            if continue_autorouting && !self.stop.is_requested() {
                current_pass += 1;
            }
        }

        // Finish with the best board ever seen (Java `:566-587`).
        let current_final_score = BoardStatistics::new(self.manager, self.board)
            .get_router_score(Some(&self.settings.scoring));
        let best_history_score = history.get_max_score();
        if best_history_score > current_final_score
            && let Some(best_board) = history.restore_best_board()
        {
            let current_stats = BoardStatistics::new(self.manager, self.board);
            restore_from_snapshot(self.manager, self.board, &best_board);
            let best_stats = BoardStatistics::new(self.manager, self.board);
            sink.debug(&format!(
                "The final board state (score {}) is worse than the best board seen during \
                     routing (score {}). Restoring the best board as the final result.",
                format_score(
                    current_final_score,
                    current_stats
                        .connections
                        .incomplete_count
                        .expect("connections included"),
                    current_stats
                        .clearance_violations
                        .total_count
                        .expect("violations included"),
                ),
                format_score(
                    best_stats.get_router_score(Some(&self.settings.scoring)),
                    best_stats
                        .connections
                        .incomplete_count
                        .expect("connections included"),
                    best_stats
                        .clearance_violations
                        .total_count
                        .expect("violations included"),
                ),
            ));
        }

        // Java `:591-601` — the completed-board tail cleanup.
        let was_router_run = self.settings.run_router
            && (self.settings.max_passes.is_none()
                || self.settings.max_passes.is_some_and(|m| m >= 0));
        if was_router_run
            && !(self.settings.remove_unconnected_vias
                || continue_autorouting
                || self.stop.is_requested())
        {
            self.remove_tails(StopConnectionOption::None);
        }

        // Java `:603-626` — the autorouter phase's after-boundary: the
        // phase fill's own walk (gate `:603`, after-capture `:604`),
        // taken after the completed-board tail sweep (`:591-601`),
        // gated on the fill's own gate (before captured).
        if self.phase_boundaries.autorouter_before.is_some() {
            self.phase_boundaries.autorouter_after =
                Some(BoardStatistics::new(self.manager, self.board));
        }

        history.clear();

        if !self.stop.is_requested() {
            sink.task_state("FINISHED", current_pass, &board_hash(self.board));
        } else {
            // The TIMED_OUT variant needs the job seam (banked); the
            // port's stopped exits all read CANCELLED.
            if self.stop_reason.is_none() {
                self.stop_reason = Some(StopReason::UserStop);
            }
            sink.task_state("CANCELLED", current_pass, &board_hash(self.board));
        }

        Ok(!self.stop.is_requested())
    }
}

/// Java `BatchAutorouter.calculateIncompleteCount` (`:556-564`) — the
/// `DesignRulesChecker.calculateAllIncompletes` +
/// `getIncompleteCount()` pair: the SUM of every net's Kruskal airline
/// count (`NetIncompletes.count()`), NOT `maxConnections` — the two
/// coincide only while nothing is routed (`all_incompletes`' first
/// tuple slot is the endpoint lower bound; never use it as the
/// incomplete total). Free so the pass runner shares it with the
/// driver.
pub fn calculate_incomplete_count(manager: &mut SearchTreeManager, board: &mut Board) -> i32 {
    let (_max_connections, rows) = all_incompletes(manager, board);
    let total: usize = rows.iter().map(|row| row.incomplete_count).sum();
    i32::try_from(total).unwrap_or(i32::MAX)
}

/// Java `BatchAutorouter.TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP` (`:43`,
/// 1000) — the pull-tight time limit the tail-removal sweep hands to
/// `optChangedArea`. Wall clock in Java; behind `deterministic_budgets`
/// the port spends the same value as consultation ticks (the
/// `RouteBudget` pattern). `AutorouteConnectionRouter` declares its
/// own twin (`:22`) — `connection_router.rs` carries that duplicate.
pub(crate) const TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP: i32 = 1000;

/// Java `BatchAutorouter.removeTails(stopConnectionOption)`
/// (`:487-503`): marking session → the all-net tail sweep → the
/// pull-tight consumption (Java `:492-498`: `optChangedArea(new
/// int[0], null, tracePullTightAccuracy, traceCosts, thread,
/// TIME_LIMIT_TO_PREVENT_ENDLESS_LOOP)`). Free so the pass runner
/// shares it with the driver.
#[allow(clippy::too_many_arguments)] // the Java read set, kept flat
pub fn remove_tails(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    stop_connection_option: StopConnectionOption,
    pull_tight_accuracy: i32,
    trace_costs: &[ExpansionCostFactor],
    stoppable: Option<&Arc<AtomicBool>>,
    time_limit_millis: i32,
    deterministic_budgets: bool,
) {
    board.start_marking_changed_area();
    // Java passes -1 (all nets).
    remove_trace_tails(manager, board, -1, stop_connection_option);
    // Java feeds `this.traceCosts` into the via arm of the tightener;
    // the port converts to the epic-board mirror (T5 activates the
    // via arm itself).
    let tightener_trace_costs: Vec<TraceCostFactor> = trace_costs.iter().map(Into::into).collect();
    opt_changed_area(
        manager,
        board,
        &mut TraceTightenerSeam,
        &[],
        None,
        pull_tight_accuracy,
        None,
        0,
        Some(&tightener_trace_costs),
        stoppable,
        time_limit_millis,
        deterministic_budgets,
    );
}

/// Java `FRLogger.formatScore` (`FRLogger.java:78`): `"0.00"` US
/// format + `" (N unrouted and M violations)"`.
#[must_use]
pub fn format_score(score: f32, incomplete: i32, violations: i32) -> String {
    format!(
        "{score:.2} ({incomplete} unrouted and {violations} violation{})",
        if violations == 1 { "" } else { "s" }
    )
}

// ---------------------------------------------------------------------------
// the item queue (BatchAutorouter.getAutorouteItems, :345-409)
// ---------------------------------------------------------------------------

/// Java `BatchAutorouter.getAutorouteItems` (`:345-409`): walk the
/// item list (descending id — the `itemList.startReadObject` order),
/// queue every NON-routable CONNECTABLE (a pin, a conduction area, or
/// a fixed/netless trace/via) once per net whose connected set does
/// not cover the net's connectable population. The same item may
/// queue once per qualifying net; single-net items already reached as
/// connected company are `handled` and never seed.
pub fn get_autoroute_items(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    sink: &mut dyn DriverSink,
) -> Vec<ItemId> {
    let mut autoroute_item_list: Vec<ItemId> = Vec::new();
    let mut handled_items: std::collections::BTreeSet<ItemId> = std::collections::BTreeSet::new();
    // The Java walk reads the live `itemList` (on-board items only —
    // removals leave the undo list); collect the seeds first so the
    // per-net connected-set walks can take the board mutably.
    let seed_ids: Vec<ItemId> = board
        .iter_descending()
        .filter(|entry| entry.on_the_board)
        .map(|entry| entry.id)
        .collect();
    for id in seed_ids {
        // The gate scope: all board borrows must end before the
        // per-net connected-set walks below take the board mutably,
        // so the gates yield only the cloned net list.
        let nets: Vec<i32> = {
            let Some(entry) = board.get(id) else {
                continue;
            };
            // Java `instanceof Connectable && instanceof Item` —
            // {Pin, Via, Trace, ConductionArea}.
            if !matches!(
                entry.data,
                ItemData::Pin { .. }
                    | ItemData::Via { .. }
                    | ItemData::Trace { .. }
                    | ItemData::ConductionArea { .. }
            ) {
                continue;
            }
            if is_routable(board, id) {
                continue;
            }
            if handled_items.contains(&id) {
                continue;
            }
            entry.nets.clone()
        };
        for current_net_number in nets {
            let connected_set = item_connected_set(manager, board, id, current_net_number);
            for connected_item in &connected_set {
                if board
                    .get(connected_item.0)
                    .is_some_and(|e| e.nets.len() <= 1)
                {
                    handled_items.insert(connected_item.0);
                }
            }
            let net_item_count = board.get_connectable_items(current_net_number).len();
            // `(connectedSet.size() < netItemCount) && !hasIgnoredNets()`
            // — the ignored-nets conjunct is constant true (banked).
            if connected_set.len() >= net_item_count {
                continue;
            }
            let net = board.rules().nets.get(current_net_number);
            // The plane-net skip: items already connected to the pour
            // would answer CONNECTED_TO_PLANE immediately (Java
            // `:383-389`).
            let already_connected_to_plane = net.is_some_and(|n| n.contains_plane)
                && connected_set.iter().any(|cid| {
                    board
                        .get(cid.0)
                        .is_some_and(|e| matches!(e.data, ItemData::ConductionArea { .. }))
                });
            if already_connected_to_plane {
                continue;
            }
            autoroute_item_list.push(id);
            let net_name =
                net.map_or_else(|| format!("net#{current_net_number}"), |n| n.name.clone());
            sink.debug(&format!(
                "Queuing item for routing: {} on net '{}' (connected: {}/{})",
                java_simple_name(board, id),
                net_name,
                connected_set.len(),
                net_item_count
            ));
        }
    }
    autoroute_item_list
}

/// Java `getClass().getSimpleName()` for the queue/dump rows: the
/// routed kinds map to their Java class names. One source of truth
/// lives on [`ItemData::java_simple_name`]; the off-board fallback
/// mirrors the callers' null arms.
#[must_use]
pub fn java_simple_name(board: &Board, id: ItemId) -> &'static str {
    board
        .get(id)
        .map(|entry| entry.data.java_simple_name())
        .unwrap_or("Item")
}

// ---------------------------------------------------------------------------
// the unrouted report (AutorouteUnroutedReport.build, :19-79)
// ---------------------------------------------------------------------------

/// Java `AutorouteUnroutedReport.build` (`:19-54`): the airline
/// endpoints grouped per net, rendered as the stagnation report
/// appendix. Empty board → the no-connections line.
pub fn build_unrouted_connections_report(
    manager: &mut SearchTreeManager,
    board: &mut Board,
) -> String {
    let (_total, rows) = all_incompletes(manager, board);
    if rows.iter().all(|row| row.edges.is_empty()) {
        return "  (no unrouted connections found)".to_string();
    }
    let mut report = String::new();
    for row in &rows {
        if row.edges.is_empty() {
            continue;
        }
        let net_name = board
            .rules()
            .nets
            .get(row.net_no)
            .map_or_else(|| format!("net#{}", row.net_no), |net| net.name.clone());
        let count = row.edges.len();
        report.push_str(&format!(
            "  Net '{net_name}' ({count} unrouted connection{}):\n",
            if count == 1 { "" } else { "s" }
        ));
        for edge in &row.edges {
            let from = describe_item(board, edge[0]);
            let to = describe_item(board, edge[1]);
            report.push_str(&format!("    - {from}  ->  {to}\n"));
        }
    }
    // Java `stripTrailing` — drop the final newline.
    report.strip_suffix('\n').unwrap_or(&report).to_string()
}

/// Java `AutorouteUnroutedReport.describeItem` (`:60-79`): a pin
/// renders `ComponentName-PinName` (the package pin name), falling
/// back to `component.name (pin #N)`; everything else falls back to
/// Java's `item.toString()`.
#[must_use]
pub fn describe_item(board: &Board, item_key: i64) -> String {
    let Ok(raw) = u32::try_from(item_key) else {
        return format!("item #{item_key}");
    };
    let id = ItemId::new(raw);
    let Some(entry) = board.get(id) else {
        return format!("item #{raw}");
    };
    if let ItemData::Pin { pin_index, .. } = &entry.data {
        let component = u32::try_from(entry.component_id)
            .ok()
            .and_then(|cid| board.components().get(cid));
        if let Some(component) = component {
            let package_pin_name = board
                .library()
                .package(component.package_no())
                .and_then(|package| package.get_pin(*pin_index))
                .map(|pin| pin.name.clone());
            if let Some(pin_name) = package_pin_name {
                return format!("{}-{}", component.name, pin_name);
            }
            return format!("{} (pin #{pin_index})", component.name);
        }
    }
    // Java's fall-through (`item != null ? item.toString() : ...`) —
    // the ENGINE's pinned `Item.toString` port (single source; the
    // engine one is mutation-verified by the t11 battery).
    item_to_string(board, u64::from(id.get()))
}

// ---------------------------------------------------------------------------
// structural tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::RouterSettingsIr;
    use crate::pipeline::board_statistics::RouterSettingsScoring;
    use crate::pipeline::event_sink::CaptureDriverSink;
    use crate::pipeline::pass_runner::RouterCounters;
    use crate::test_util::parse;
    use epic_board::tree_manager::SearchTreeManager;

    /// The T9/T10c locator-world fixture (2 layers, `unit um`,
    /// resolution 10). Its own nets 33 and 98 are pin PAIRS (components
    /// #97/#98 and #99/#100) — 2 incompletes, 4 queue attempts on the
    /// bare board; nets 49/94 are single pins (never queue).
    fn parse_fixture() -> (SearchTreeManager, Board) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../harness/fixtures/locator-spike/t9_locator45.dsn");
        let text = std::fs::read_to_string(&path).expect("fixture present");
        parse(&text)
    }

    /// The jar world's cost table (the t11 capture row `ctrl_costs`).
    fn settings_ir() -> RouterSettingsIr {
        RouterSettingsIr {
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

    /// The starved driver settings: only the fixture's UNUSED layer (1)
    /// active → Java's maze reads `ctrl.layerActive` at every expansion
    /// and every attempt fails deterministically with zero expansion
    /// work → the multi-pass loop runs in milliseconds with a CONSTANT
    /// score (the legacy score of a fully-unrouted board clamps at 0).
    fn starved_settings() -> BatchSettings {
        let mut settings = BatchSettings::new(settings_ir(), RouterSettingsScoring::default());
        settings.router_settings.layer_active = vec![false, true];
        settings
    }

    /// Java `:47-61` — no active signal layer: the warn row, the
    /// CANCELLED event at pass 0, and the `IllegalArgumentException`
    /// → `Result` abort face (never reaches the pass loop).
    #[test]
    fn t12_driver_no_active_layers_aborts() {
        let (mut manager, mut board) = parse_fixture();
        let mut settings = starved_settings();
        settings.router_settings.layer_active = vec![false, false];
        let mut sink = CaptureDriverSink::default();
        let mut driver = BatchDriver::new(&mut manager, &mut board, settings, StopFace::default());
        let outcome = driver.run(&mut sink);
        assert_eq!(
            outcome,
            Err(BatchLoopError::NoActiveSignalLayers),
            "the abort face"
        );
        assert_eq!(
            BatchLoopError::NoActiveSignalLayers.to_string(),
            "Cannot start autorouter: all layers are disabled.",
            "Java's exception message"
        );
        assert!(
            sink.any_contains("Cannot start autorouter: all layers are disabled."),
            "the warn row"
        );
        let states_text = sink.joined("task_state");
        assert!(
            states_text.starts_with("task_state state=CANCELLED pass=0 hash="),
            "cancelled before any pass: {states_text}"
        );
        assert_eq!(states_text.lines().count(), 1, "no STARTED/RUNNING events");
    }

    /// The fanout pre-pass OBSERVABILITY faces (Java `AutorouteBatchLoop
    /// :86-90` + `:100-103`): the check row fires UNCONDITIONALLY (even
    /// with the router disabled via `maxPasses < 0`), the skip row only
    /// when fanout is enabled AND the board has no SMD pins. World 1:
    /// all pins removed → `smdPins=0` + the skip row. World 2 (positive
    /// control, the bare fixture): the check row carries the real SMD
    /// count (100 — the M4 dossier's "4/100") and NO skip row.
    #[test]
    fn t12_driver_fanout_pre_pass_rows() {
        let (mut manager, mut board) = parse_fixture();
        let pins: Vec<ItemId> = board
            .iter_ascending()
            .filter(|entry| matches!(entry.data, ItemData::Pin { .. }))
            .map(|entry| entry.id)
            .collect();
        for id in pins {
            manager.remove(&mut board, id);
            board.remove_item(id);
        }
        let mut settings = starved_settings();
        settings.max_passes = Some(-1); // router disabled; the rows fire anyway
        let mut sink = CaptureDriverSink::default();
        let mut driver = BatchDriver::new(&mut manager, &mut board, settings, StopFace::default());
        let outcome = driver.run(&mut sink).expect("run completes");
        assert!(outcome, "no stop source");
        let debug = sink.joined("debug");
        assert!(
            debug.contains("Checking fanout pre-pass. settings.fanout.enabled=true, smdPins=0"),
            "the unconditional check row: {debug}"
        );
        assert!(
            sink.any_contains(
                "Fanout stage is enabled but skipped because the board has no SMD pins."
            ),
            "the empty-SMD skip row: {debug}"
        );

        // Positive control: the bare fixture keeps its 100 SMD pins —
        // the check row fires with the real count, no skip row.
        let (mut manager, mut board) = parse_fixture();
        let mut settings = starved_settings();
        settings.max_passes = Some(-1);
        let mut sink = CaptureDriverSink::default();
        let mut driver = BatchDriver::new(&mut manager, &mut board, settings, StopFace::default());
        driver.run(&mut sink).expect("run completes");
        let debug = sink.joined("debug");
        assert!(
            debug.contains("Checking fanout pre-pass. settings.fanout.enabled=true, smdPins=100"),
            "the check row carries the real SMD count: {debug}"
        );
        assert!(
            !sink.any_contains("Fanout stage is enabled but skipped"),
            "no skip row with SMD pins present: {debug}"
        );
    }

    /// The `currentPass > maxPasses` gate (`:308-313`): passes 1..3 run,
    /// the loop re-enters with currentPass=4, the gate raises the stop,
    /// and the run reports MaxPasses + CANCELLED + `Ok(false)`.
    #[test]
    fn t12_driver_max_passes() {
        let (mut manager, mut board) = parse_fixture();
        let mut settings = starved_settings();
        settings.max_passes = Some(3);
        let mut sink = CaptureDriverSink::default();
        let mut driver = BatchDriver::new(&mut manager, &mut board, settings, StopFace::default());
        let outcome = driver.run(&mut sink);
        assert_eq!(outcome, Ok(false), "a stop was raised");
        let states_text = sink.joined("task_state");
        let states: Vec<&str> = states_text.lines().collect();
        assert_eq!(
            states.len(),
            5,
            "STARTED + RUNNING×3 + CANCELLED: {states_text}"
        );
        assert!(
            states[4].starts_with("task_state state=CANCELLED pass=4 hash="),
            "the loop re-entered with pass 4 and the gate fired: {states_text}"
        );
        let running_count = states
            .iter()
            .filter(|row| row.contains("state=RUNNING"))
            .count();
        assert_eq!(running_count, 3, "exactly three passes ran: {states_text}");
    }

    /// The T1b two-face mapping (Java `StoppableThread.java`): the
    /// auto-router-only raise satisfies the routing gate's face but NOT
    /// the full face; the FULL raise satisfies BOTH (ALL satisfies
    /// `isStopAutoRouterRequested()` too, `:40-42` vs `:28-30`).
    #[test]
    fn t1b_stop_face_two_face_mapping() {
        // AUTO_ROUTER_ONLY: `isStopAutoRouterRequested()` true,
        // `isStopRequested()` false.
        let mut stop = StopFace::default();
        assert!(!stop.is_requested());
        assert!(!stop.is_full_stop_requested());
        stop.request();
        assert!(stop.is_requested());
        assert!(
            !stop.is_full_stop_requested(),
            "the auto-router-only raise must not set the full face"
        );
        // ALL: both faces true.
        stop.request_full();
        assert!(stop.is_requested());
        assert!(stop.is_full_stop_requested());
        // A fresh face over a raised EXTERNAL flag reads as the full
        // stop (the external raise is Java's requestStop() from another
        // thread). The UN-raised flag face is field-for-field
        // StopFace::default(), whose row the asserts above already
        // cover.
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let stop = StopFace::from_flag(Some(flag));
        assert!(stop.is_requested());
        assert!(stop.is_full_stop_requested());
    }

    /// The readiness fix-round's external-only face pin: the engine's
    /// own raises stay LOCAL (the shared flag is never written), while
    /// an external owner's store is fully visible — and the writable
    /// `from_flag` face keeps its documented outbound store. This is
    /// the byte-identity seam the CLI cancel wiring rides (the
    /// flagged-face write-back broke it live: push_shove golden
    /// drift, fix-round evidence 05c).
    #[test]
    fn external_only_face_never_writes_the_shared_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        let mut face = StopFace::from_external_flag(Some(Arc::clone(&flag)));
        face.request();
        assert!(
            !flag.load(Ordering::Relaxed),
            "an internal raise must NOT store the shared flag"
        );
        assert!(face.is_requested(), "the raise is still real on the face");
        face.request_full();
        assert!(
            !flag.load(Ordering::Relaxed),
            "request_full stays local too"
        );
        assert!(face.is_full_stop_requested());
        // The external owner's channel still works.
        flag.store(true, Ordering::Relaxed);
        assert!(face.is_requested());
        assert!(face.is_full_stop_requested());
        // The writable face keeps its documented outbound store.
        let writable_flag = Arc::new(AtomicBool::new(false));
        let mut writable = StopFace::from_flag(Some(Arc::clone(&writable_flag)));
        writable.request();
        assert!(writable_flag.load(Ordering::Relaxed));
    }

    /// The pass runner's maxItems face (`AutoroutePassRunner:211-220`):
    /// the first attempt consumes the budget, the SECOND attempt sees
    /// `total >= maxItems`, logs the info row, and raises the stop
    /// mid-pass. The driver classifies it as MaxItemsReached.
    #[test]
    fn t12_driver_max_items_stops_mid_pass() {
        let (mut manager, mut board) = parse_fixture();
        let mut settings = starved_settings();
        settings.max_items = Some(1);
        let mut sink = CaptureDriverSink::default();
        let mut driver = BatchDriver::new(&mut manager, &mut board, settings, StopFace::default());
        let outcome = driver.run(&mut sink).expect("run completes");
        assert!(!outcome, "the stop was raised");
        assert_eq!(driver.total_items_routed, 1, "exactly one attempt ran");
        assert_eq!(
            driver.stop_reason,
            Some(StopReason::MaxItemsReached),
            "the driver classified the pass-side stop"
        );
        assert!(
            sink.any_contains("Max items limit reached (1). Stopping auto-router."),
            "the pass-side info row: {states}",
            states = sink.joined("info")
        );
        let states_text = sink.joined("task_state");
        assert!(
            states_text.contains("task_state state=CANCELLED pass=1 hash="),
            "cancelled inside pass 1: {states_text}"
        );
        // The pass bailed mid-queue: 1 attempt of 4, items_to_go = 3.
        let updates_text = sink.joined("board_updated");
        assert!(
            updates_text.contains(
                "phase=autoroute pass=1 queued=3 skipped=0 ripped=0 failed=1 routed=0 \
                 incomplete=2 fanout_extra_vias=0"
            ),
            "the aborted pass's final fill: {updates_text}"
        );
        // The stop bypasses the restore gate's size/modulo arms, so
        // the gate IS evaluated here — with equal best/current scores
        // (0.0 unrouted) the strict `>` must skip the restore (Java
        // `:338-343` "equally-scored boards do not trigger a restore").
        assert!(
            !sink.any_contains("Restoring an earlier board"),
            "equal best/current scores must not restore"
        );
    }

    /// A capture sink that raises an EXTERNAL stop (the shared flag)
    /// on the pass-1 final-fill counters row — the deterministic
    /// mid-run hook for the [`StopReason::MaxItemsReached`]
    /// misattribution pin below.
    struct StopOnPassOneFillSink {
        inner: CaptureDriverSink,
        flag: Arc<AtomicBool>,
        fired: bool,
    }

    impl DriverSink for StopOnPassOneFillSink {
        fn is_trace_enabled(&self) -> bool {
            self.inner.is_trace_enabled()
        }
        fn info(&mut self, message: &str) {
            self.inner.info(message);
        }
        fn warn(&mut self, message: &str) {
            self.inner.warn(message);
        }
        fn debug(&mut self, message: &str) {
            self.inner.debug(message);
        }
        fn trace(&mut self, message: &str) {
            self.inner.trace(message);
        }
        fn task_state(&mut self, state: &str, pass: i32, hash: &str) {
            self.inner.task_state(state, pass, hash);
        }
        fn board_updated(&mut self, counters: &RouterCounters) {
            // The AUTOROUTE pass-1 FINAL fill (4 failed attempts on the
            // starved world) — by this row every attempt has run, so the
            // maxItems face never fired (total 4 < 5 at every attempt
            // head) and the flip is purely external. The phase
            // discriminator is M4-T7-load-bearing: the now-live fanout
            // stage's pass-1 fill carries the SAME (pass=1, failed=4)
            // pair on this world, and hooking it would stop the run
            // before the batch loop's first pass.
            if !self.fired
                && counters.phase == "autoroute"
                && counters.pass_count == 1
                && counters.failed_to_be_routed_count == 4
            {
                self.fired = true;
                self.flag.store(true, Ordering::Relaxed);
            }
            self.inner.board_updated(counters);
        }
    }

    /// An EXTERNAL stop raised during a max-items-bounded run must NOT
    /// be attributed to [`StopReason::MaxItemsReached`] — the pre-fix
    /// classification tested the SETTING (`max_items.is_some()`), not
    /// the cause (quality MINOR-3). World: starved settings with
    /// `max_items = Some(5)` (the 4 failed attempts never reach it —
    /// the face never fires, no info row) and the shared flag flipped
    /// by the sink on the pass-1 final-fill row. The run must report
    /// [`StopReason::UserStop`] and CANCELLED after pass 1. Reverting
    /// the cause-mark fix makes this pin fail (the old code reports
    /// MaxItemsReached here).
    #[test]
    fn t12_driver_external_stop_is_not_max_items() {
        let (mut manager, mut board) = parse_fixture();
        let mut settings = starved_settings();
        settings.max_items = Some(5);
        let flag = Arc::new(AtomicBool::new(false));
        let mut sink = StopOnPassOneFillSink {
            inner: CaptureDriverSink::default(),
            flag: Arc::clone(&flag),
            fired: false,
        };
        let stop = StopFace::from_flag(Some(flag));
        let mut driver = BatchDriver::new(&mut manager, &mut board, settings, stop);
        let outcome = driver.run(&mut sink).expect("run completes");
        assert!(!outcome, "a stop was raised");
        assert_eq!(
            driver.stop_reason,
            Some(StopReason::UserStop),
            "an external stop is NOT MaxItemsReached"
        );
        assert_eq!(driver.total_items_routed, 4, "all 4 attempts ran");
        assert!(
            !sink.inner.any_contains("Max items limit reached"),
            "the maxItems face never fired: {info}",
            info = sink.inner.joined("info")
        );
        let states_text = sink.inner.joined("task_state");
        assert!(
            states_text.contains("task_state state=CANCELLED pass=1 hash="),
            "cancelled after pass 1: {states_text}"
        );
    }

    /// The completion world (layers `[true, true]` on the bare
    /// fixture): pass 1 routes each net pair's first pin and the
    /// second attempt answers NoUnconnectedNets (skipped) — the final
    /// counters row reads `routed=2 skipped=2 incomplete=0`; pass 2
    /// finds an empty queue and the run FINISHES on its own accord.
    /// The `incomplete=0` final row is the discriminator that the
    /// incompletes total is the SUM of the per-net counts, NOT
    /// `maxConnections` (which stays 2 on this world — the two coincide
    /// only while nothing is routed).
    #[test]
    fn t12_driver_finishes_with_skip_tally() {
        let (mut manager, mut board) = parse_fixture();
        let mut settings = BatchSettings::new(settings_ir(), RouterSettingsScoring::default());
        settings.max_passes = Some(10);
        // The fanout stage is LIVE as of M4-T7: this pin studies the
        // AUTOROUTE stage's skip tally (an exact 2-row board_updated
        // census), and the fanout listener's own counter rows (plus any
        // routed escapes) would perturb it. Fanout-on completion
        // dynamics are pinned by the T7 battery.
        settings.fanout_enabled = false;
        let mut sink = CaptureDriverSink::default();
        let mut driver = BatchDriver::new(&mut manager, &mut board, settings, StopFace::default());
        let outcome = driver.run(&mut sink).expect("run completes");
        assert!(outcome, "the router finished its own accord (no stop)");
        assert_eq!(driver.stop_reason, None, "no stop source fired");
        let states_text = sink.joined("task_state");
        let states: Vec<&str> = states_text.lines().collect();
        assert_eq!(
            states.len(),
            4,
            "STARTED + RUNNING×2 + FINISHED: {states_text}"
        );
        assert!(states[0].starts_with("task_state state=STARTED pass=0 hash="));
        assert!(states[1].starts_with("task_state state=RUNNING pass=1 hash="));
        assert!(states[2].starts_with("task_state state=RUNNING pass=2 hash="));
        assert!(
            states[3].starts_with("task_state state=FINISHED pass=2 hash="),
            "the loop exits WITHOUT re-entering a pass 3: {states_text}"
        );
        assert!(
            sink.any_contains("with baseline score"),
            "the router-enabled stage-start row fired"
        );
        let info = sink.joined("info");
        assert!(
            info.contains("for 2 unrouted items."),
            "the baseline incompletes read the pair total: {info}"
        );
        // Pass 1's counter rows: the pre-pass tally (4 queued, 2
        // incompletes) and the final fill (2 routed + 2 skipped, ZERO
        // incompletes — the routed world separates the sum from
        // maxConnections).
        let updates_text = sink.joined("board_updated");
        let updates: Vec<&str> = updates_text.lines().collect();
        assert_eq!(
            updates.len(),
            2,
            "pass 2's empty queue emits no counter rows: {updates_text}"
        );
        assert_eq!(
            updates[0],
            "phase=autoroute pass=1 queued=4 skipped=0 ripped=0 failed=0 routed=0 \
             incomplete=2 fanout_extra_vias=0",
        );
        assert_eq!(
            updates[1],
            "phase=autoroute pass=1 queued=0 skipped=2 ripped=0 failed=0 routed=2 \
             incomplete=0 fanout_extra_vias=0",
            "the completion tally: sum-based incompletes hit 0"
        );
        assert!(
            info.contains("was completed in")
                && info.contains("pass #1")
                && info.contains("pass #2"),
            "both pass-completed rows fired: {info}"
        );
    }

    /// The pass-local stagnation tracker (Java `:524-536`) with fanout
    /// DISABLED: the tracker starts at pass 8 (`STOP_AT_PASS_MINIMUM`),
    /// pass 8 itself is consumed by the first-improvement arm
    /// (`last_best_score` starts at −∞), and the constant 0.0 score
    /// then accumulates 10 no-improvement passes → StagnationLocal at
    /// pass 18. No fanout recovery row may appear.
    #[test]
    fn t12_driver_stagnation_local_without_fanout() {
        let (mut manager, mut board) = parse_fixture();
        let mut settings = starved_settings();
        settings.fanout_enabled = false;
        let mut sink = CaptureDriverSink::default();
        let mut driver = BatchDriver::new(&mut manager, &mut board, settings, StopFace::default());
        let outcome = driver.run(&mut sink).expect("run completes");
        assert!(!outcome);
        assert_eq!(driver.stop_reason, Some(StopReason::StagnationLocal));
        assert!(
            !driver.fanout_recovery_applied,
            "fanout disabled → recovery never fires"
        );
        let states_text = sink.joined("task_state");
        assert!(
            states_text.contains("task_state state=CANCELLED pass=18 hash="),
            "8 (improvement reset) + 10 stagnant passes: {states_text}"
        );
        let running = states_text
            .lines()
            .filter(|row| row.contains("state=RUNNING"))
            .count();
        assert_eq!(running, 18, "all 18 passes ran: {states_text}");
        let info = sink.joined("info");
        assert!(
            info.contains(
                "has not improved by more than 0.5 points in the last 10 passes \
                 (2 items still unconnected)"
            ),
            "the local stagnation row: {info}"
        );
        assert!(
            info.contains("The following connections could not be routed")
                && info.contains("Net 'NET_33' (1 unrouted connection)")
                && info.contains("Net 'NET_98' (1 unrouted connection)"),
            "the unrouted-report appendix: {info}"
        );
        assert!(
            !sink.any_contains("Applied one-time fanout recovery cleanup"),
            "no recovery without fanout"
        );
        // The final best-board restore is gated by a strict `>` too —
        // with the constant 0.0 score the best EQUALS the final, and
        // Java must keep the current board (no restore row).
        assert!(
            !sink.any_contains("Restoring the best board as the final result"),
            "equal final/best scores must not trigger the final restore"
        );
    }

    /// The fanout-enabled contrast (Java `:482-505` + `:538-545`): the
    /// one-time recovery fires at pass 11 (counter == 3, incompletes >
    /// 0) and RESETS the local counter — so the LOCAL tracker would not
    /// fire until pass 21, but the GLOBAL tracker (best at pass 8)
    /// reaches its 10-pass window at pass 18 first → StagnationGlobal.
    #[test]
    fn t12_driver_stagnation_global_with_fanout_recovery() {
        let (mut manager, mut board) = parse_fixture();
        let settings = starved_settings(); // fanout_enabled defaults true
        let mut sink = CaptureDriverSink::default();
        let mut driver = BatchDriver::new(&mut manager, &mut board, settings, StopFace::default());
        let outcome = driver.run(&mut sink).expect("run completes");
        assert!(!outcome);
        assert_eq!(driver.stop_reason, Some(StopReason::StagnationGlobal));
        assert!(
            driver.fanout_recovery_applied,
            "the one-time recovery fired"
        );
        let info = sink.joined("info");
        assert!(
            info.contains(
                "has not improved by more than 0.5 points since pass #8. \
                 Stopping the auto-router after 18 passes (2 items still unconnected)"
            ),
            "the global stagnation row: {info}"
        );
        let states_text = sink.joined("task_state");
        assert!(
            states_text.contains("task_state state=CANCELLED pass=18 hash="),
            "the global window closes at 8 + 10: {states_text}"
        );
        let debug = sink.joined("debug");
        assert!(
            debug.contains(
                "Applied one-time fanout recovery cleanup (removed fanout \
                 tails/vias). Incompletes: 2 -> 2."
            ),
            "the recovery row: {debug}"
        );
        // Exactly ONE recovery (one-time flag) and it fired at pass 11
        // — the local counter's 10-pass window (reset at 11) would only
        // close at pass 21, after the global fire at 18.
        assert_eq!(
            debug
                .lines()
                .filter(|row| row.contains("Applied one-time fanout recovery cleanup"))
                .count(),
            1,
            "one-time: {debug}"
        );
    }

    /// `formatScore` (Java `FRLogger.java:78`): the US `0.00` format
    /// plus the unrouted/violation suffix with singular/plural arms.
    #[test]
    fn t12_format_score_exact() {
        assert_eq!(
            format_score(1234.5, 1, 1),
            "1234.50 (1 unrouted and 1 violation)"
        );
        assert_eq!(
            format_score(0.0, 2, 0),
            "0.00 (2 unrouted and 0 violations)"
        );
    }
}
