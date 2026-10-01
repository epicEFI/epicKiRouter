//! The full-pipeline assembly (M4-T10): the port of Java
//! `autoroute/pipeline/RoutingPipeline.java`. `run()` sequences the
//! ROUTING stage (fanout + batch autorouter — the T12/T7 driver) and
//! the OPTIMIZATION stage (the T9 optimizer) under Java's exact gates.
//!
//! **`runRoutingStage` (`:92-119`).** The router-enabled gate is
//! `getRunRouter() && (maxPasses == null || maxPasses >= 0)`. When the
//! router is not enabled and fanout is enabled, Java runs the
//! FANOUT-ONLY mode: a temporary `maxPasses = 0` override around the
//! same batch loop (`:104-113`), restored in a `finally`. The override
//! is load-bearing exactly on the `maxPasses < 0` face: the batch loop
//! recomputes `isRouterEnabled` from the live settings
//! (`AutorouteBatchLoop.java:249-253`) and seeds its pass loop with
//! `continueAutorouting = isRouterEnabled` (`:274`) — with
//! `runRouter == false` the loop never enters a pass (fanout only, the
//! `--router.enabled=off` face); with `runRouter == true` and
//! `maxPasses < 0` the override flips the recomputation TRUE and the
//! loop runs with the pass-count gate disabled (`:308-311` reads
//! `maxPasses > 0`, so `0` = unlimited, `RouterSettings.java:939`).
//! (The CLI `validate()` normalizes negatives to 0 before the pipeline
//! sees them, so the `-1` face is settings-reachable, not
//! argv-reachable.)
//!
//! **`runOptimizationStage` (`:121-134`).** Gated on the optimizer
//! existing (`getRunOptimizer()`) AND `!job.thread.isStopRequested()`.
//! STOP-GATE SEMANTICS (the T1b two-face port — this block REPLACES the
//! T9-era collapsed-face text, which the T1 measurement FALSIFIED: under
//! the collapse the optimizer stage never entered on ANY Tier A fixture
//! while Java's full-flow baselines carry 932.18 s of optimizer time on
//! exactly the incomplete=0 boards): Java keeps TWO stop faces on the
//! job thread (`StoppableThread.java`: `requestStop()` → ALL,
//! `requestStopAutoRouter()` → AUTO_ROUTER_ONLY). The routing-stage
//! gates read `isStopAutoRouterRequested()` (`:102`, `:105` — true for
//! BOTH non-NONE states); the optimization-stage gate reads
//! `isStopRequested()` (`:122` — ALL only, raised by external stops and
//! the max-items path `AutoroutePassRunner.java:211-218`), so an
//! internally-stopped board (stagnation, pass exhaustion) reaches the
//! stage and Java's `BatchOptimizer.evaluatePreFlightGuards`
//! (`:155-225`) does the skipping inside. The port maps the faces onto
//! [`StopFace`]: `request()`/`is_requested()` = AUTO_ROUTER_ONLY face;
//! `request_full()`/`is_full_stop_requested()` = the ALL face (the
//! max-items site raises `request_full()`); the gate here consumes the
//! ALL face only. PORT RESIDUALS, both documented, neither gate-side:
//! (1) `request()` also stores the shared flag when one exists (the
//! engines' mid-search interrupt), so a FLAGGED face cannot
//! distinguish external from driver raises after the fact — flagged
//! faces keep the pre-T1b collapsed face; production headless faces
//! carry no flag (`StopFace::default()`), where the mapping is exact.
//! (2) WIRED at M10-T1 (buglog 224 closed): the optimizer's loop reads
//! a stage face that SHARES the parent's shared flag
//! (`StopFace::from_flag(parent_stop.flag().cloned())` at
//! `run_optimization_stage`), so an external raise mid-optimization is
//! visible to the loop's polls — exactly Java, whose loop reads the
//! shared `isStopRequested()` face via `job.thread`
//! (`BatchOptimizer.java:384`). A FLAGLESS parent (the CLI production
//! face) yields field-identically the pre-T1 `StopFace::default()`, so
//! the CLI path is byte-invariant BY CONSTRUCTION (the measured
//! re-proof lives in the M10-T1 battery). The T9-era fresh-flagless
//! artifact this wiring replaces left the external-during-optimization
//! interrupt dead (the M9-T7 battery's bm01/bm10 watchdog faces,
//! buglog 224). (Fix provenance: M5-T1's measurement-only battery
//! (`logs/M5-T1/`) named the gate; T1b landed the port; M10-T1 wires
//! the flag.)
//!
//! **`finishAutoroute` (`RoutingBoard.java:899-904`).** RECON INSIDE,
//! recorded: the call clears the retained `AutorouteEngine`
//! expansion-room database and `clearAllItemTemporaryAutorouteData()`
//! (`AutorouteEngine.java:307-316`, `RoutingBoard.java:1241-1245`) —
//! memory-cache cleanup only. Nothing after the pipeline reads the
//! engine (the SES/manifest faces never touch it), the headless
//! `autoroute()` calls re-init with `retain = false`, and the port's
//! maze engine holds no retained engine between stages. No port body:
//! the call is a documented no-op here.
//!
//! **Stage transitions.** Java writes `job.stage` (a public FIELD — no
//! event and no headless listener: `RoutingJobSchedulerActionThread`
//! registers only a board-updated listener and an afterRouting
//! `StageListener`, both telemetry-only). The port records the same
//! transitions in [`PipelineOutcome::stage_transitions`] so the
//! stage-order pins have a witness. The per-stage
//! STARTED/RUNNING/FINISHED task-state EVENTS are emitted by the
//! stages themselves (Java `BatchAutorouter`/`BatchOptimizer` —
//! already mirrored by the T12 driver and the T9 optimizer through the
//! sink), so T10 adds no new emission face and the events corpus
//! cannot rotate from this module.
//!
//! **`normalizeRouterAlgorithm` (`:136-147`).** Java's ctor warns and
//! resets a non-current algorithm. The router side is banked T12 (the
//! driver carries one algorithm); the optimizer side is the T9 stage's
//! first row (`normalizeAlgorithm` moved — SEAM T9).

use epic_board::board::Board;
use epic_board::tree_manager::SearchTreeManager;

use crate::pipeline::batch::{
    BatchDriver, BatchLoopError, BatchSettings, StageBoundaries, StopFace, StopReason,
};
use crate::pipeline::board_statistics::BoardStatistics;
use crate::pipeline::event_sink::DriverSink;
use crate::pipeline::optimizer::{BatchOptimizerStage, OptimizerOutcome, OptimizerSettingsIr};
use crate::pipeline::tuning;

/// Java `core.RoutingStage` — the job's stage field. Field writes in
/// Java (no event); the port records them for the stage-order pins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoutingStage {
    /// Java `RoutingStage.IDLE`.
    Idle,
    /// Java `RoutingStage.ROUTING`.
    Routing,
    /// Java `RoutingStage.OPTIMIZATION`.
    Optimization,
}

/// The optimization stage's manifest faces: the boundary statistics
/// pair (Java `BatchOptimizer`'s `initialStats` at `runBatchLoop` entry
/// and `finalStats` after the incumbent restore at exit — the port
/// captures the same moments at the pipeline seams (stage entry /
/// stage exit); a preflight bypass leaves the board untouched, so the
/// pair is identical there, matching Java's bypass fill
/// `after = before`) + the outcome
/// ([`OptimizerOutcome::passes_completed`] IS Java's `passesCompleted`
/// — `0` on the bypass face).
#[derive(Debug)]
pub struct OptimizerStageFaces {
    /// Java `phase.before`'s stats.
    pub before: BoardStatistics,
    /// Java `phase.after`'s stats.
    pub after: BoardStatistics,
    /// Java `passesCompleted` / `isTimedOut`.
    pub outcome: OptimizerOutcome,
}

/// The pipeline's per-phase manifest faces — the raw boundary
/// statistics pairs the CLI renderer consumes (`None` = the phase
/// stayed empty under Java's capture gates).
#[derive(Debug, Default)]
pub struct PipelinePhases {
    /// Java `fanoutBeforeStats`/`fanoutAfterStats`.
    pub fanout: Option<(BoardStatistics, BoardStatistics)>,
    /// The autorouter boundary pair.
    pub autorouter: Option<(BoardStatistics, BoardStatistics)>,
    /// The optimizer stage faces.
    pub optimizer: Option<OptimizerStageFaces>,
}

/// The pipeline outcome — Java `run()` returns nothing; the port's flow
/// reads these for the final-state mapping and the per-phase manifest
/// faces.
pub struct PipelineOutcome {
    /// `runRoutingStage`'s batch-loop result. `Err` only for the
    /// all-layers-disabled abort — Java's batch loop throws out of
    /// `run()`, so `runOptimizationStage` and the IDLE write never
    /// execute; the port mirrors that (no optimization, no Idle
    /// transition).
    pub routing: Result<bool, BatchLoopError>,
    /// Java `router.stop_reason` (port-added observability).
    pub stop_reason: Option<StopReason>,
    /// The driver's post-routing AUTO_ROUTER_ONLY stop face (Java
    /// `isStopAutoRouterRequested()`, true for ANY stop — internal
    /// stagnation/pass-exhaustion raises included). Port-added
    /// observability, NOT the optimization gate's input: that gate reads
    /// the FULL-stop face only (Java `isStopRequested()` = ALL; see
    /// `stop_full` on `RoutingStageResult` and the module docs'
    /// STOP-GATE SEMANTICS).
    pub stop_after_routing: bool,
    /// The driver's stage-boundary captures (fanout + autorouter).
    pub boundaries: StageBoundaries,
    /// The optimization stage's faces when it ran (the
    /// `run_optimizer && !stop` gate passed).
    pub optimizer: Option<OptimizerStageFaces>,
    /// Java `job.stage` transitions in order. The final entry is Idle
    /// unless the routing stage aborted (Java's throw skips it).
    pub stage_transitions: Vec<RoutingStage>,
    /// M6-T7: the global plan (default-OFF stage — `Some` only when
    /// `congestion_global` is ON).
    pub global_plan: Option<crate::global::plan::GlobalPlan>,
    /// M7-T4/T5: the meander stage's report (the T4 outcome rows plus
    /// the T5 match-group rows; empty/default unless the stage ran —
    /// the stage-ordering pins' witness).
    pub meander_report: crate::pipeline::tuning::MeanderStageReport,
    /// M7-T6: the pair stage's outcomes (one row per RESOLVED pair,
    /// (leader, follower) ASC; empty when the declaration list is
    /// empty — the zero-activation face — or the routing stage
    /// aborted).
    pub pair_stage: Vec<crate::pipeline::pairs::PairStageOutcome>,
    /// M8-T3: the gloss BUS stage's report (the group rows + the
    /// hug/spread moves; empty/default unless `router.gloss.bus` is ON
    /// — the flag is the only gate, the two-regime law). Rides the
    /// aesthetics SIDECAR via epic-cli, NEVER the manifest.
    pub gloss_report: crate::pipeline::gloss::GlossBusReport,
    /// M8-T4: the gloss FLOW stage's report (the jog/stub/miter rows;
    /// empty/default unless `router.gloss.flow` is ON — the flag is
    /// the only caller gate, the two-regime law). Rides the aesthetics
    /// SIDECAR via epic-cli, NEVER the manifest.
    pub flow_report: crate::pipeline::gloss::GlossFlowReport,
    /// M8-T5: the gloss VIA-PLACE stage's report (the per-via rows;
    /// empty/default unless `router.gloss.via_place` is ON — the flag
    /// is the only caller gate, the two-regime law). Rides the
    /// aesthetics SIDECAR via epic-cli, NEVER the manifest.
    pub via_place_report: crate::pipeline::gloss::GlossViaPlaceReport,
    /// M8-T6: the gloss TEARDROPS stage's report (the per-junction
    /// rows; empty/default unless `router.gloss.teardrops` is ON — the
    /// flag is the only caller gate, the two-regime law). Rides the
    /// aesthetics SIDECAR via epic-cli, NEVER the manifest.
    pub teardrops_report: crate::pipeline::gloss::GlossTeardropsReport,
}

/// Java `RoutingPipeline.runRoutingStage` (`:92-119`). Takes the
/// pipeline-owned settings by `&mut` so the fanout-only override's
/// restore face is observable; the driver gets a clone. (Java's method
/// reads its state from class fields — the free function carries the
/// same inputs explicitly.)
#[allow(clippy::too_many_arguments)]
pub fn run_routing_stage(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    settings: &mut BatchSettings,
    stop: &StopFace,
    sink: &mut dyn DriverSink,
    transitions: &mut Vec<RoutingStage>,
) -> RoutingStageResult {
    // Java `:93-96` — the router-enabled gate.
    let router_enabled = settings.run_router
        && (settings.max_passes.is_none() || settings.max_passes.is_some_and(|max| max >= 0));

    // Java `:98-100` — the stage field write.
    if router_enabled || settings.fanout_enabled {
        transitions.push(RoutingStage::Routing);
    }

    // Java `:115` — `job.board.finishAutoroute()`: the recon is
    // recorded in the module docs — a memory-cache no-op face here.
    // Java `:116-118` — the afterRouting StageListeners (headless: one
    // telemetry-only listener; banked).

    if router_enabled && !stop.is_requested() {
        // Java `:102-103` — the routing branch: the driver gets a
        // clone; the pipeline-owned settings stay authoritative.
        let mut driver = BatchDriver::new(manager, board, settings.clone(), stop.clone());
        let routing = driver.run(sink);
        RoutingStageResult::of(&mut driver, routing)
    } else if settings.fanout_enabled && !stop.is_requested() {
        // Java `:104-113` — the fanout-only mode: the temporary
        // `maxPasses = 0` override, run, restore (Java's `finally`).
        let original_max_passes = settings.max_passes;
        settings.max_passes = Some(0);
        let mut driver = BatchDriver::new(manager, board, settings.clone(), stop.clone());
        let routing = driver.run(sink);
        let stage = RoutingStageResult::of(&mut driver, routing);
        // The Java `finally` restore — on the pipeline-owned face, so
        // the pin observes the restored value after the call.
        settings.max_passes = original_max_passes;
        stage
    } else {
        // Both branches skipped (a stop already raised, or nothing
        // enabled): Java runs neither branch — the board passes
        // through untouched to `finishAutoroute`.
        RoutingStageResult::pass_through(stop.is_requested(), stop.is_full_stop_requested())
    }
}

/// `run_routing_stage`'s product (Java's method returns void; the
/// pipeline needs the driver's post-run faces).
#[doc(hidden)]
pub struct RoutingStageResult {
    /// The batch loop's result.
    pub routing: Result<bool, BatchLoopError>,
    /// Java `driver.stop_reason`.
    pub stop_reason: Option<StopReason>,
    /// The driver's post-run stop face.
    pub stop_requested: bool,
    /// The FULL-stop face after routing (Java `isStopRequested()` =
    /// `StopRequestState.ALL`): external stops and the max-items path
    /// (`AutoroutePassRunner.java:211-218`) — the ONLY stop face the
    /// optimizer-stage gate consumes (Java `:122`).
    pub stop_full: bool,
    /// The stage-boundary captures.
    pub boundaries: StageBoundaries,
    /// M6-T7: the driver's global plan (default-OFF stage — `Some`
    /// only when `congestion_global` is ON).
    pub global_plan: Option<crate::global::plan::GlobalPlan>,
}

impl RoutingStageResult {
    fn of(driver: &mut BatchDriver<'_>, routing: Result<bool, BatchLoopError>) -> Self {
        Self {
            routing,
            stop_reason: driver.stop_reason,
            stop_requested: driver.stop.is_requested(),
            stop_full: driver.stop.is_full_stop_requested(),
            boundaries: std::mem::take(&mut driver.phase_boundaries),
            global_plan: driver.global_plan.take(),
        }
    }

    fn pass_through(stop_requested: bool, stop_full: bool) -> Self {
        Self {
            routing: Ok(true),
            stop_reason: None,
            stop_requested,
            stop_full,
            boundaries: StageBoundaries::default(),
            global_plan: None,
        }
    }
}

/// Java `RoutingPipeline.runOptimizationStage` (`:121-134`). `None` =
/// the gate kept the stage out: `run_optimizer == false` is Java's
/// `optimizer == null` face, and `full_stop` is Java's
/// `isStopRequested()` = `StopRequestState.ALL` only (external stops +
/// the max-items path) — the AUTO_ROUTER_ONLY internal stops
/// (stagnation, pass exhaustion) do NOT close this gate; Java's
/// `BatchOptimizer.evaluatePreFlightGuards` does the skipping inside.
/// The two-face mapping and its residuals live in the module docs
/// (STOP-GATE SEMANTICS). When the stage runs, the boundary statistics
/// pair is captured at Java's own moments (stage entry / stage exit — a
/// preflight bypass leaves the board untouched, so `after == before`
/// there, matching Java's bypass fill).
///
/// `parent_stop` is the caller's stop face: M10-T1's wiring (buglog
/// 224's closure) shares its flag with the stage face, so an EXTERNAL
/// raise mid-optimization stops the loop (Java's loop reads the shared
/// `job.thread` face, `BatchOptimizer.java:384`). The sharing also
/// widens the OUTBOUND direction: the stage's internal `request_full`
/// now stores the PARENT's shared flag (Java-faithful — Java's
/// max-items `requestStop()` raises the shared thread flag,
/// `AutoroutePassRunner.java:211-218`; CLI-invariant — the CLI parent
/// is flagless). A flagless parent (the CLI production face) is
/// field-identical to the pre-T1 `StopFace::default()` — byte-invariance
/// is structural.
#[allow(clippy::too_many_arguments)]
pub fn run_optimization_stage(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    settings: BatchSettings,
    optimizer_settings: OptimizerSettingsIr,
    run_optimizer: bool,
    full_stop: bool,
    parent_stop: &StopFace,
    sink: &mut dyn DriverSink,
    transitions: &mut Vec<RoutingStage>,
) -> Option<OptimizerStageFaces> {
    // Java `:122-124` — the two-gate early return (the FULL-stop face
    // only; the two-face mapping and its residuals live in the fn doc
    // and the module block).
    if !run_optimizer || full_stop {
        return None;
    }

    // Java `:126` — the stage field write.
    transitions.push(RoutingStage::Optimization);
    // Java `:127-129` — the beforeOptimization StageListeners (headless:
    // none registered).

    // Java `:288-292` moment — the stage's `initialStats` capture sits
    // at runBatchLoop entry; the pipeline's walk here is the same
    // moment (construction has no board effect).
    let before = BoardStatistics::new(manager, board);
    // M10-T1: the stage face SHARES the parent's flag (buglog 224's
    // wiring fix) — a flagless parent yields exactly the old
    // `StopFace::default()`; a flagged parent makes external raises
    // visible to the optimizer's polls (optimizer.rs:907 the loop head
    // — the poll the M10-T1 raise pins' kill rides on — and
    // :1234/:1458/:1738/:1802; the site list verified by the T1
    // quality review). The sharing also widens the OUTBOUND direction:
    // the optimizer's internal `request_full` (the M8-T7
    // candidate-raise replay, optimizer.rs:1494) now STORES the
    // parent's shared flag, so an internal full-stop request is
    // visible to the session host — Java-faithful (Java's max-items
    // `requestStop()` raises the shared thread flag,
    // `AutoroutePassRunner.java:211-218`). The EXTERNAL-ONLY mode
    // propagates (the readiness fix-round): a host cancel flag the
    // engine must never write keeps the stage face write-shielded too,
    // so an internal stop cannot leak into the host's cancel flag
    // either.
    let mut stage_stop = if parent_stop.is_external_only() {
        StopFace::from_external_flag(parent_stop.flag().cloned())
    } else {
        StopFace::from_flag(parent_stop.flag().cloned())
    };
    let mut optimizer = BatchOptimizerStage::new(
        manager,
        board,
        settings,
        optimizer_settings,
        &mut stage_stop,
    );
    let outcome = optimizer.run_batch_loop(sink);
    drop(optimizer);
    // Java `:519-526` moment — `finalStats` after the incumbent
    // restore, at runBatchLoop exit.
    let after = BoardStatistics::new(manager, board);
    // Java `:131-133` — the afterOptimization StageListeners.

    Some(OptimizerStageFaces {
        before,
        after,
        outcome,
    })
}

/// Java `RoutingPipeline.run()` (`:86-90`): the routing stage, then the
/// optimization stage, then the IDLE transition. On the routing
/// stage's abort (`BatchLoopError` — Java's batch loop THROWS) the
/// optimization stage and the IDLE write are skipped, exactly as the
/// exception unwinds Java's `run()`.
pub fn run(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    settings: BatchSettings,
    optimizer_settings: OptimizerSettingsIr,
    run_optimizer: bool,
    stop: StopFace,
    sink: &mut dyn DriverSink,
) -> PipelineOutcome {
    let mut transitions = Vec::new();
    let mut settings = settings;
    let meander_active = settings.meander_active;
    let pairs = settings.pairs.clone();
    let bus_active = settings.bus_active;
    let flow_active = settings.flow_active;
    let via_place_active = settings.via_place_active;
    let teardrops_active = settings.teardrops_active;
    let stage = run_routing_stage(manager, board, &mut settings, &stop, sink, &mut transitions);
    let routing = stage.routing;
    let optimizer = if routing.is_err() {
        // Java: the throw unwinds out of run() — no optimization, no IDLE.
        None
    } else {
        run_optimization_stage(
            manager,
            board,
            settings,
            optimizer_settings,
            run_optimizer,
            stage.stop_full,
            &stop,
            sink,
            &mut transitions,
        )
    };
    // M7-T4: the tuning MEANDER stage — the beyond-Java post-
    // optimization face, AFTER the tightener's last pass (a meandered
    // trace survives every later pass because there IS no later pass
    // in the flow; the T3 honoring gate protects a meandered net
    // against any default-ON shortener that would run after it, and
    // the stage-ordering pin owns that face). Emits NO stage
    // transition (Java's stage enum is the frozen parity surface —
    // the events corpus cannot rotate) and NO events. Inert at
    // `meander_active == false` (the default — the byte-invariance
    // regime).
    let meander_report = if routing.is_ok() && meander_active {
        tuning::run_meander_stage(manager, board, sink)
    } else {
        crate::pipeline::tuning::MeanderStageReport::default()
    };
    // M7-T6: the pair MATCH stage — the post-meander face (the class
    // faces own the class goals first; the pair face tops the pair
    // delta up). Gated on the RESOLVED declaration list alone (the
    // third activation input): empty ⇒ inert, byte-identical. Emits
    // NO stage transition and NO events (the meander-stage precedent).
    let pair_stage = if routing.is_ok() && !pairs.is_empty() {
        crate::pipeline::pairs::run_pair_stage(manager, board, &pairs, sink)
    } else {
        Vec::new()
    };
    // M8-T3: the gloss BUS stage — the parallel-bus group detector +
    // the hug/spread re-spacing pass (`pipeline/gloss.rs`), the
    // post-tuning, pre-report slot (after the meander and pair
    // stages). Gated on `router.gloss.bus` ALONE (the resolved
    // `bus_active`; default OFF — an OFF run never reaches the stage,
    // the two-regime byte-invariance law). Emits NO stage transition
    // (Java's stage enum is the frozen parity surface — the events
    // corpus cannot rotate) and NO events (the meander-stage
    // precedent). The report rides the aesthetics SIDECAR via
    // epic-cli, NEVER the manifest (the version-blind manifest canary
    // pins manifest bytes; the raw cf607714… retired at M10-T5).
    let gloss_report = if routing.is_ok() && bus_active {
        crate::pipeline::gloss::run_gloss_bus_stage(manager, board)
    } else {
        crate::pipeline::gloss::GlossBusReport::default()
    };
    // M8-T4: the gloss FLOW stage — 45° jog/stub elimination + the
    // miter/recorner bridge (`pipeline/gloss.rs`), the slot AFTER the
    // bus stage (flow after spread — the AM2 composition law:
    // spread/hug relocations create jogs/corners only the flow pass
    // cleans). Gated on `router.gloss.flow` ALONE (the resolved
    // `flow_active`; default OFF — an OFF run never reaches the stage,
    // the two-regime byte-invariance law) plus the stage's own two
    // gates (FORTYFIVE_DEGREE angle gate, incompletes gate). Emits NO
    // stage transition (the parity stage enum cannot rotate) and NO
    // events (the meander-stage precedent). The report rides the
    // aesthetics SIDECAR via epic-cli, NEVER the manifest.
    let flow_report = if routing.is_ok() && flow_active {
        crate::pipeline::gloss::run_gloss_flow_stage(manager, board)
    } else {
        crate::pipeline::gloss::GlossFlowReport::default()
    };
    // M8-T5: the gloss VIA-PLACE stage — the return-path-aware via
    // placement pass (`pipeline/gloss.rs`), the slot AFTER the flow
    // stage (the terminal slot — the recorded slot decision in
    // gloss.rs: the pass's arms land as single segments, so it creates
    // no flow work, while running before flow would let the pass's
    // wholesale arm rewrite discard flow landings on those arms).
    // Gated on `router.gloss.via_place` ALONE (the resolved
    // `via_place_active`; default OFF — an OFF run never reaches the
    // stage, the two-regime byte-invariance law) plus the stage's own
    // incompletes gate. Emits NO stage transition (the parity stage
    // enum cannot rotate) and NO events (the meander-stage
    // precedent). The report rides the aesthetics SIDECAR via
    // epic-cli, NEVER the manifest.
    let via_place_report = if routing.is_ok() && via_place_active {
        crate::pipeline::gloss::run_gloss_via_place_stage(manager, board)
    } else {
        crate::pipeline::gloss::GlossViaPlaceReport::default()
    };
    // M8-T6: the gloss TEARDROPS stage — the graded-width teardrop
    // pass (`pipeline/gloss.rs`), the slot AFTER the via-place stage
    // (the terminal slot — the recorded slot decision in gloss.rs:
    // via-place moves vias, teardrops attach at the moved positions).
    // Gated on `router.gloss.teardrops` ALONE (the resolved
    // `teardrops_active`; default OFF — an OFF run never reaches the
    // stage, the two-regime byte-invariance law) plus the stage's own
    // incompletes gate. Emits NO stage transition (the parity stage
    // enum cannot rotate) and NO events (the meander-stage
    // precedent). The report rides the aesthetics SIDECAR via
    // epic-cli, NEVER the manifest.
    let teardrops_report = if routing.is_ok() && teardrops_active {
        crate::pipeline::gloss::run_gloss_teardrops_stage(manager, board)
    } else {
        crate::pipeline::gloss::GlossTeardropsReport::default()
    };
    if routing.is_ok() {
        // Java `:89` — `this.job.stage = RoutingStage.IDLE;`.
        transitions.push(RoutingStage::Idle);
    }
    PipelineOutcome {
        routing,
        stop_reason: stage.stop_reason,
        stop_after_routing: stage.stop_requested,
        boundaries: stage.boundaries,
        optimizer,
        stage_transitions: transitions,
        global_plan: stage.global_plan,
        meander_report,
        pair_stage,
        gloss_report,
        flow_report,
        via_place_report,
        teardrops_report,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::board_statistics::{
        OptimizerScoreSettings, RouterScoreSettings, RouterScoringVersion,
        default_routing_cost_settings,
    };
    use crate::pipeline::event_sink::CaptureDriverSink;
    use crate::test_util::parse;

    /// The T9/T10 locator-world fixture (2 layers, `unit um`; 100 SMD
    /// pins all on layer 0).
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

    fn v2_scoring() -> crate::pipeline::board_statistics::RouterSettingsScoring {
        crate::pipeline::board_statistics::RouterSettingsScoring {
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

    /// The merged-flow optimizer defaults (`DefaultSettings.java:177-191`),
    /// guards OFF so the pipeline pins' optimizer stage actually runs
    /// its pass loop (Java `enable_preflight_guards=false` bypass).
    fn opt_settings() -> OptimizerSettingsIr {
        OptimizerSettingsIr {
            algorithm: "freerouting-optimizer".to_string(),
            max_passes: Some(1),
            max_items: None,
            improvement_threshold: Some(2.5),
            enable_preflight_guards: Some(false),
            max_consecutive_failures: Some(50),
            max_consecutive_failures_pass1: Some(12),
            additional_ripup_cost_factor_at_start: 10,
            trace_ripup_cost_factor: 0.6,
            max_autoroute_passes: 6,
            timeout_string: None,
        }
    }

    /// The pipeline pin world: fanout+router+optimizer ON, batch budget
    /// bounded (`max_passes=1`, `max_items=20`) so the world stays
    /// cheap. The internal stop it raises is MAX-PASSES (Java's
    /// `requestStopAutoRouter()` AUTO_ROUTER_ONLY face,
    /// `StoppableThread.java:33-37`) — post-T1b the optimizer gate
    /// OPENS on it (the gate pin's max-passes world witnesses that);
    /// the stage-order pin still uses [`Self::completed_world`] for the
    /// clean NO-STOP face so its stage-order claim stays decoupled from
    /// any stop face.
    fn bounded_world() -> (SearchTreeManager, Board, BatchSettings, OptimizerSettingsIr) {
        let (manager, board) = parse_fixture();
        let mut settings = BatchSettings::new(settings_ir(), v2_scoring());
        settings.max_passes = Some(1);
        settings.max_items = Some(20);
        let optimizer = opt_settings();
        (manager, board, settings, optimizer)
    }

    /// The bounded world with the maxItems budget tripping MID-PASS (the
    /// first attempt spends it, the second sees `total >= max_items`):
    /// the stop Java raises with `requestStop()` (`AutoroutePassRunner.java:211-218`)
    /// — the optimizer-stage gate must close on it.
    fn max_items_world() -> (SearchTreeManager, Board, BatchSettings, OptimizerSettingsIr) {
        let (manager, board, mut settings, optimizer) = bounded_world();
        settings.max_passes = Some(5);
        settings.max_items = Some(1);
        (manager, board, settings, optimizer)
    }

    /// The no-stop pipeline world: the locator fixture routes to
    /// COMPLETION (the T9 suite's routed-world face — `Ok(true)`, no
    /// stop reason), so the post-routing stop face is CLEAN (neither
    /// stop face raised) and the optimizer gate opens — the same face
    /// Java's `:122` gate sees with no stop requested.
    fn completed_world() -> (SearchTreeManager, Board, BatchSettings, OptimizerSettingsIr) {
        let (manager, board) = parse_fixture();
        let settings = BatchSettings::new(settings_ir(), v2_scoring());
        let optimizer = opt_settings();
        (manager, board, settings, optimizer)
    }

    /// THE STAGE-ORDER PIN: the optimizer never runs before the router.
    /// The transitions record is the witness — Java writes `job.stage`
    /// at `:99` (ROUTING), `:126` (OPTIMIZATION), `:89` (IDLE); a
    /// reordered `run()` (optimization before routing) produces
    /// `[Optimization, Routing, Idle]` and dies here on the first
    /// assert. The second assert witnesses the ordering EFFECT: the
    /// optimizer ran only after the batch loop had consumed its pass
    /// (the stage gate reads the post-routing FULL-stop face; this
    /// world raises no stop at all).
    #[test]
    fn t10_stage_order_optimizer_never_precedes_routing() {
        let (mut manager, mut board, settings, optimizer) = completed_world();
        let mut sink = CaptureDriverSink::default();
        let outcome = run(
            &mut manager,
            &mut board,
            settings,
            optimizer,
            true,
            StopFace::default(),
            &mut sink,
        );
        assert_eq!(
            outcome.stage_transitions,
            vec![
                RoutingStage::Routing,
                RoutingStage::Optimization,
                RoutingStage::Idle
            ],
            "ROUTING -> OPTIMIZATION -> IDLE, never reordered"
        );
        assert!(
            outcome.optimizer.is_some(),
            "the stage gate opened post-routing (no stop raised)"
        );
    }

    /// THE FANOUT-ONLY-MODE PIN (`--router.enabled=off` face): the
    /// router disabled + fanout enabled runs the fanout stage and ZERO
    /// autorouter passes (Java `AutorouteBatchLoop.java:274`
    /// `continueAutorouting = isRouterEnabled` with the recomputed gate
    /// FALSE), and the pipeline-owned `max_passes` is RESTORED after
    /// the temporary override (Java's `finally`, `:111`). The restore
    /// mutant (drop the reset) dies on the last assert; the
    /// skip-the-branch mutant (never enter the fanout-only arm) dies on
    /// the empty fanout boundary.
    #[test]
    fn t10_fanout_only_mode_restores_setting_and_skips_the_router() {
        let (mut manager, mut board, mut settings, _optimizer) = bounded_world();
        settings.run_router = false;
        settings.max_passes = Some(3);
        let mut transitions = Vec::new();
        let mut sink = CaptureDriverSink::default();
        let result = run_routing_stage(
            &mut manager,
            &mut board,
            &mut settings,
            &StopFace::default(),
            &mut sink,
            &mut transitions,
        );
        assert!(result.routing.is_ok());
        assert_eq!(transitions, vec![RoutingStage::Routing]);
        // The fanout stage ran; the batch pass loop never did.
        assert!(result.boundaries.fanout_before.is_some());
        assert!(result.boundaries.fanout_after.is_some());
        assert!(result.boundaries.autorouter_before.is_none());
        assert!(result.boundaries.autorouter_after.is_none());
        // Java `:111` — the restore face, observed on the caller's
        // settings (the override ran the loop with `Some(0)`).
        assert_eq!(settings.max_passes, Some(3), "the override is restored");
        assert!(result.stop_reason.is_none());
    }

    /// The override's LOAD-BEARING arm (module docs): `runRouter == true`
    /// with `maxPasses < 0` disables the router at the pipeline gate,
    /// the fanout-only branch fires, and the temporary `Some(0)` flips
    /// the batch loop's recomputed `isRouterEnabled` back TRUE — the
    /// loop RUNS (pass-count gate disabled; `max_items` bounds it).
    /// Mutant: drop the override and the recomputation stays FALSE —
    /// `autorouter_before` stays None and this pin dies.
    #[test]
    fn t10_fanout_only_override_load_bearing_on_negative_max_passes() {
        let (mut manager, mut board, mut settings, _optimizer) = bounded_world();
        settings.run_router = true;
        settings.max_passes = Some(-1);
        let mut transitions = Vec::new();
        let mut sink = CaptureDriverSink::default();
        let result = run_routing_stage(
            &mut manager,
            &mut board,
            &mut settings,
            &StopFace::default(),
            &mut sink,
            &mut transitions,
        );
        assert!(result.routing.is_ok());
        // The batch loop RAN (the override flipped the recomputation).
        assert!(result.boundaries.autorouter_before.is_some());
        assert!(result.boundaries.autorouter_after.is_some());
        // And the pipeline-owned settings carry the restored face.
        assert_eq!(settings.max_passes, Some(-1));
    }

    /// M10-T1's raise sink: stores `true` into the parent's shared flag
    /// the moment the optimization stage's first info row arrives — the
    /// deterministic external-raise face (Java's cancel button raising
    /// `job.thread` mid-optimization). No timing, no sleeps: the row is
    /// emitted at stage entry (optimizer.rs `run_batch_loop`, the
    /// "Optimization stage started" row) BEFORE the pass loop's first
    /// stop poll, so the raise is always visible to the loop.
    struct RaiseAtOptimizationStartSink {
        flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
        fired: bool,
        saw_interrupted: bool,
        saw_completed: bool,
    }

    impl DriverSink for RaiseAtOptimizationStartSink {
        fn info(&mut self, message: &str) {
            if !self.fired && message.contains("Optimization stage started") {
                self.fired = true;
                self.flag.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            if message.contains("Optimization stage interrupted:") {
                self.saw_interrupted = true;
            }
            if message.contains("Optimization stage completed:") {
                self.saw_completed = true;
            }
        }
    }

    /// THE M10-T1 WIRING PIN (buglog 224): a flagged parent + a raise
    /// at the optimization stage's entry row ⇒ the stage's pass loop
    /// OBSERVES the stop — the interrupted summary row is the loop's
    /// own stop witness ("Optimization stage interrupted:", the port of
    /// Java `BatchOptimizer.java:530`'s `interrupted:` literal), and
    /// `passes_completed == 0` pins WHEN (before pass 1).
    ///
    /// PROBE RECORD (the dispatch's probe-first law): NO outcome field
    /// propagates a stop face out of the stage —
    /// `OptimizerOutcome { passes_completed, is_timed_out }` is the
    /// whole face, and Java propagates none either
    /// (`RoutingPipeline.java:121-134`: `runOptimizationStage` is
    /// void; `run()` is void). Java-faithful observables are the
    /// stage's OWN rows (the `interrupted:` summary) + the pass count,
    /// and the HOST re-reads the shared flag — so no new outcome field
    /// is wired. The pass count + the summary row are this pin's face.
    ///
    /// DNR-16 minus-mutant witness (the wiring revert,
    /// `stage_stop = StopFace::default()`): the raise lands on a face
    /// nobody reads, the loop runs its pass, the summary reads
    /// `completed:` — the pin DIES on both asserts. Restored ⇒ green.
    #[test]
    fn m10_t1_optimizer_stage_shares_the_parent_flag() {
        let (mut manager, mut board, settings, optimizer) = completed_world();
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let parent = StopFace::from_flag(Some(std::sync::Arc::clone(&flag)));
        let mut sink = RaiseAtOptimizationStartSink {
            flag,
            fired: false,
            saw_interrupted: false,
            saw_completed: false,
        };
        let outcome = run(
            &mut manager,
            &mut board,
            settings,
            optimizer,
            true,
            parent,
            &mut sink,
        );
        assert_eq!(
            outcome.stage_transitions,
            vec![
                RoutingStage::Routing,
                RoutingStage::Optimization,
                RoutingStage::Idle
            ],
            "the stage ran (the raise happened mid-run, not before it)"
        );
        assert!(
            sink.fired,
            "the raise fired (the stage emitted its entry row)"
        );
        let faces = match outcome.optimizer {
            Some(faces) => faces,
            None => panic!("the optimizer stage must have run in this world"),
        };
        assert_eq!(
            faces.outcome.passes_completed, 0,
            "the loop observed the stop before pass 1"
        );
        assert!(
            sink.saw_interrupted,
            "the loop's own stop witness row must carry the interrupted summary"
        );
        assert!(
            !sink.saw_completed,
            "the loop must not have run to natural completion"
        );
        assert!(
            !faces.outcome.is_timed_out,
            "a user-stop interrupt is NOT a timeout — the outcome field's interrupt face (the \
             T1 quality review's survivor 8: a mutant setting is_timed_out on a user raise died \
             nowhere before this assert)"
        );
    }

    /// THE M10-T1 OVER-WIRING CONTROL PIN (DNR-16's plus direction): a
    /// flagged parent with NO raise must run the stage CLEANLY to
    /// natural completion — the wiring SHARES the flag, it does not
    /// raise anything. Kills the pre-raised-stage-face mutant
    /// (`stage_stop` constructed then `request_full()`ed — an
    /// over-eager wiring that stops every flagged-parent run): that
    /// mutant turns this run into `interrupted:`/0-passes and the pin
    /// DIES. The revert mutant (minus direction) is killed by the
    /// raise pin above; between the two pins BOTH wiring directions
    /// are mutant-witnessed.
    #[test]
    fn m10_t1_flagged_parent_without_a_raise_runs_clean() {
        let (mut manager, mut board, settings, optimizer) = completed_world();
        let parent = StopFace::from_flag(Some(std::sync::Arc::new(
            std::sync::atomic::AtomicBool::new(false),
        )));
        let mut sink = CaptureDriverSink::default();
        let outcome = run(
            &mut manager,
            &mut board,
            settings,
            optimizer,
            true,
            parent,
            &mut sink,
        );
        let faces = match outcome.optimizer {
            Some(faces) => faces,
            None => panic!("the optimizer stage must have run in this world"),
        };
        assert_eq!(
            faces.outcome.passes_completed, 1,
            "the un-raised flagged parent runs its full pass budget"
        );
        assert!(
            sink.any_contains("Optimization stage completed:"),
            "the natural-completion summary, not the interrupted one"
        );
        assert!(!sink.any_contains("Optimization stage interrupted:"));
    }

    /// THE STOP-REQUEST SHORT-CIRCUIT PIN (T1b two-face form): a FULL
    /// stop raised BEFORE the pipeline skips BOTH stage bodies (Java
    /// `:102` requires `!isStopAutoRouterRequested()` — ALL satisfies
    /// it — and the optimizer gate at `:122` reads
    /// `isStopRequested()` = ALL) — transitions still record
    /// ROUTING (the `:99` write happens under
    /// `routerEnabled || fanoutEnabled`) and IDLE, the boundaries stay
    /// empty, and the optimizer stage never opens. Crossing arms: the
    /// AUTO_ROUTER_ONLY pre-raise now OPENS the optimizer gate (the
    /// `t1b_` pin below); the disabled/no-stop face skips the optimizer
    /// WITHOUT a stop (the second half here).
    #[test]
    fn t10_stop_request_short_circuits_both_stages() {
        // World 1: FULL stop raised, optimizer enabled — everything skips.
        let (mut manager, mut board, settings, optimizer) = bounded_world();
        let mut stop = StopFace::default();
        stop.request_full();
        let mut sink = CaptureDriverSink::default();
        let outcome = run(
            &mut manager,
            &mut board,
            settings,
            optimizer,
            true,
            stop,
            &mut sink,
        );
        assert_eq!(
            outcome.stage_transitions,
            vec![RoutingStage::Routing, RoutingStage::Idle]
        );
        assert!(outcome.boundaries.fanout_before.is_none());
        assert!(outcome.boundaries.autorouter_before.is_none());
        assert!(
            outcome.optimizer.is_none(),
            "the full stop keeps the stage out"
        );
        assert!(outcome.stop_after_routing);

        // World 2 (contrast): optimizer DISABLED, no stop — the stage
        // stays out for the `optimizer == null` face, not a stop.
        let (mut manager, mut board, settings, optimizer) = bounded_world();
        let mut sink = CaptureDriverSink::default();
        let outcome = run(
            &mut manager,
            &mut board,
            settings,
            optimizer,
            false,
            StopFace::default(),
            &mut sink,
        );
        assert!(outcome.optimizer.is_none());
        // (No `stop_after_routing` face assert here: the bounded world
        // raises the MAX-PASSES stop — Java's `requestStopAutoRouter()`
        // AUTO_ROUTER_ONLY face (`StoppableThread.java:33-37`) — whose
        // observability face is true for unrelated-to-the-gate reasons;
        // the optimizer-disabled arm has no gate to consult either way.)
        assert_eq!(
            outcome.stage_transitions,
            vec![RoutingStage::Routing, RoutingStage::Idle]
        );
    }

    /// THE OPTIMIZER GATE'S INPUT FACE (spec-review M7, T1b form): the
    /// gate reads the DRIVER's post-routing FULL-stop face (Java
    /// `:122` = `StopRequestState.ALL` only). TWO worlds:
    /// (a) the MAX-ITEMS stop (Java's `AutoroutePassRunner.java:211-218`
    /// `requestStop()` — the maxItems world trips it mid-pass) CLOSES
    /// the gate;
    /// (b) the MAX-PASSES stop (Java's `requestStopAutoRouter()` face —
    /// the bounded world raises it after pass 1) does NOT — the stage
    /// opens on the stopped board and Java's preflight guards own the
    /// skipping. The pre-T1b collapsed gate read
    /// `stop_requested` for both and skipped (a) correctly but (b)
    /// wrongly; the reviewer's M7 mutant reads the PRE-run face and
    /// dies on world (a)'s first assert.
    #[test]
    fn t1b_optimizer_gate_reads_the_full_stop_face() {
        // (a) The max-items world: the FULL stop mid-pass — gate CLOSED.
        let (mut manager, mut board, settings, optimizer) = max_items_world();
        let mut sink = CaptureDriverSink::default();
        let outcome = run(
            &mut manager,
            &mut board,
            settings,
            optimizer,
            true,
            StopFace::default(),
            &mut sink,
        );
        assert!(
            outcome.stop_after_routing,
            "the max-items world raises a stop mid-pass"
        );
        assert_eq!(outcome.stop_reason, Some(StopReason::MaxItemsReached));
        assert!(
            outcome.optimizer.is_none(),
            "the max-items face is Java's requestStop() — the gate stays closed"
        );
        assert_eq!(
            outcome.stage_transitions,
            vec![RoutingStage::Routing, RoutingStage::Idle]
        );

        // (b) The bounded (max-passes) world: the AUTO_ROUTER_ONLY stop
        // after pass 1 — gate OPEN, the stage runs on the stopped board.
        let (mut manager, mut board, settings, optimizer) = bounded_world();
        let mut sink = CaptureDriverSink::default();
        let outcome = run(
            &mut manager,
            &mut board,
            settings,
            optimizer,
            true,
            StopFace::default(),
            &mut sink,
        );
        assert!(
            outcome.stop_after_routing,
            "the bounded world raises the max-passes stop after pass 1"
        );
        assert_eq!(outcome.stop_reason, Some(StopReason::MaxPasses));
        assert!(
            outcome.optimizer.is_some(),
            "the max-passes face is requestStopAutoRouter() — the gate opens"
        );
        assert_eq!(
            outcome.stage_transitions,
            vec![
                RoutingStage::Routing,
                RoutingStage::Optimization,
                RoutingStage::Idle
            ]
        );
    }

    /// THE T1b PARITY PIN: an AUTO_ROUTER_ONLY stop (the port's
    /// `request()`, Java `requestStopAutoRouter()`,
    /// `StoppableThread.java:33-37`) raised BEFORE the pipeline closes
    /// the ROUTING-stage gate (`:102` — `isStopAutoRouterRequested()`)
    /// but must NOT close the optimizer gate (`:122` reads
    /// `isStopRequested()` = ALL only). Pre-T1b the collapsed face
    /// skipped the stage here; Java's optimizer then runs (its loop
    /// also reads the ALL-only face, `BatchOptimizer.java:384`) and its
    /// own preflight guards do the skipping. Contrast arms: the FULL
    /// pre-raise skips (the short-circuit pin above); the max-items
    /// raise during routing closes the gate (the post-routing-face pin
    /// above).
    #[test]
    fn t1b_auto_router_only_stop_opens_the_optimizer_gate() {
        let (mut manager, mut board, settings, optimizer) = bounded_world();
        let mut stop = StopFace::default();
        stop.request();
        let mut sink = CaptureDriverSink::default();
        let outcome = run(
            &mut manager,
            &mut board,
            settings,
            optimizer,
            true,
            stop,
            &mut sink,
        );
        assert_eq!(
            outcome.stage_transitions,
            vec![
                RoutingStage::Routing,
                RoutingStage::Optimization,
                RoutingStage::Idle
            ]
        );
        assert!(
            outcome.optimizer.is_some(),
            "the AUTO_ROUTER_ONLY face leaves the optimizer gate open"
        );
    }
}
