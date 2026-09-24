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
//! `continueAutorouting = isRouterEnabled` (`:276`) — with
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
//! STOP-GATE SEMANTICS (the T9 MIN-2 correction, carried verbatim — do
//! not re-introduce a divergence while extracting): Java's router-side
//! AUTO_ROUTER_ONLY stops do NOT satisfy `isStopRequested()` — the
//! stage RUNS there and preflight guard 1 (incompletes > 0) does the
//! skipping; only the max-items path (`AutoroutePassRunner.java:211-218`)
//! and external stops raise the full stop. The port's single shared
//! `StopFace` collapses the two faces — the same net outcome (no
//! optimization on partially-routed boards), with the one corner
//! divergence (a fully-routed-but-stopped board runs no-op reroute
//! passes in Java and skips the stage here). The gate input is the
//! DRIVER's post-routing stop face, read before the driver drops it.
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
    /// The driver's post-routing stop face — the optimization gate's
    /// input (read before the driver drops it; the collapsed-face
    /// semantics live in the module docs).
    pub stop_after_routing: bool,
    /// The driver's stage-boundary captures (fanout + autorouter).
    pub boundaries: StageBoundaries,
    /// The optimization stage's faces when it ran (the
    /// `run_optimizer && !stop` gate passed).
    pub optimizer: Option<OptimizerStageFaces>,
    /// Java `job.stage` transitions in order. The final entry is Idle
    /// unless the routing stage aborted (Java's throw skips it).
    pub stage_transitions: Vec<RoutingStage>,
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
        RoutingStageResult::pass_through(stop.is_requested())
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
    /// The stage-boundary captures.
    pub boundaries: StageBoundaries,
}

impl RoutingStageResult {
    fn of(driver: &mut BatchDriver<'_>, routing: Result<bool, BatchLoopError>) -> Self {
        Self {
            routing,
            stop_reason: driver.stop_reason,
            stop_requested: driver.stop.is_requested(),
            boundaries: std::mem::take(&mut driver.phase_boundaries),
        }
    }

    fn pass_through(stop_requested: bool) -> Self {
        Self {
            routing: Ok(true),
            stop_reason: None,
            stop_requested,
            boundaries: StageBoundaries::default(),
        }
    }
}

/// Java `RoutingPipeline.runOptimizationStage` (`:121-134`). `None` =
/// the gate kept the stage out (`run_optimizer == false` is Java's
/// `optimizer == null` face; `stop_requested` is the driver's
/// post-routing stop face — the collapsed-face semantics live in the
/// module docs). When the stage runs, the boundary statistics pair is
/// captured at Java's own moments (stage entry / stage exit — a
/// preflight bypass leaves the board untouched, so `after == before`
/// there, matching Java's bypass fill).
#[allow(clippy::too_many_arguments)]
pub fn run_optimization_stage(
    manager: &mut SearchTreeManager,
    board: &mut Board,
    settings: BatchSettings,
    optimizer_settings: OptimizerSettingsIr,
    run_optimizer: bool,
    stop_requested: bool,
    sink: &mut dyn DriverSink,
    transitions: &mut Vec<RoutingStage>,
) -> Option<OptimizerStageFaces> {
    // Java `:122-124` — the two-gate early return.
    if !run_optimizer || stop_requested {
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
    // The stage stop face is the T9 wiring face (a fresh face — the
    // CLI flow has no external stop source; Java shares `job.thread`,
    // banked in SEAM).
    let mut stage_stop = StopFace::default();
    let mut optimizer = BatchOptimizerStage::new(
        manager,
        board,
        settings,
        optimizer_settings,
        &mut stage_stop,
    );
    let outcome = optimizer.run_batch_loop(sink);
    drop(optimizer);
    // Java `:508-510` moment — `finalStats` after the incumbent
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
            stage.stop_requested,
            sink,
            &mut transitions,
        )
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
    /// cheap. NOTE the collapsed-stop consequence: the internal stop
    /// this world raises is the FULL stop in the port's single-flag
    /// model, so the optimizer gate closes — the stage-order pin needs
    /// the no-stop face and uses [`Self::completed_world`] instead.
    fn bounded_world() -> (SearchTreeManager, Board, BatchSettings, OptimizerSettingsIr) {
        let (manager, board) = parse_fixture();
        let mut settings = BatchSettings::new(settings_ir(), v2_scoring());
        settings.max_passes = Some(1);
        settings.max_items = Some(20);
        let optimizer = opt_settings();
        (manager, board, settings, optimizer)
    }

    /// The no-stop pipeline world: the locator fixture routes to
    /// COMPLETION (the T9 suite's routed-world face — `Ok(true)`, no
    /// stop reason), so the post-routing stop face is CLEAN and the
    /// optimizer gate opens (Java's router-side AUTO_ROUTER_ONLY stops
    /// would also leave it open; the port's collapsed flag requires the
    /// router to actually finish).
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
    /// (the stage gate read the POST-routing stop face).
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
    /// autorouter passes (Java `AutorouteBatchLoop.java:276`
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

    /// THE STOP-REQUEST SHORT-CIRCUIT PIN: a stop raised BEFORE the
    /// pipeline skips BOTH stage bodies (Java `:102` and `:105` both
    /// require `!isStopAutoRouterRequested()`; the optimizer gate at
    /// `:122` then sees the full stop) — transitions still record
    /// ROUTING (the `:99` write happens under
    /// `routerEnabled || fanoutEnabled`) and IDLE, the boundaries stay
    /// empty, and the optimizer stage never opens. Crossing arms: the
    /// enabled/no-stop face runs (the stage-order pin); the
    /// disabled/no-stop face skips the optimizer WITHOUT a stop (the
    /// second half here).
    #[test]
    fn t10_stop_request_short_circuits_both_stages() {
        // World 1: stop raised, optimizer enabled — everything skips.
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
        // raises MaxItemsReached, which IS the full stop in the port's
        // collapsed flag — the optimizer-disabled arm has no gate to
        // consult either way.)
        assert_eq!(
            outcome.stage_transitions,
            vec![RoutingStage::Routing, RoutingStage::Idle]
        );
    }
}
