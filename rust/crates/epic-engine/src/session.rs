//! The headless application session — the M9-T2 `Session` (the Java
//! `HeadlessBoardManager` role: load -> settings -> route -> cancel ->
//! export). The session path IS the CLI path: [`Session::route`] and
//! [`Session::load_dsn`] drive the EXACT `run_route` sequence with the
//! SAME moved faces (the settings predicates from
//! [`crate::settings`], the SES projection from [`crate::export`],
//! the copper-to-edge override relocated here), and the acceptance
//! proof is byte-identity of the exported SES vs `epic-cli route` on
//! the same fixture x settings (the harness
//! `session_parity_pin.rs`).
//!
//! Deviations from the T2 dispatch sketch, all deliberate and
//! in-code-documented:
//!
//! 1. `export_ses` takes `&mut self` (the projection mutates the
//!    `SesBoard` carrier, `route.rs:1112` — the dispatch's own
//!    refinement, kept).
//! 2. `deterministic_budgets` defaults to TRUE, not the dispatch's
//!    `false`: the CLI default is ON — `resolve` seeds
//!    `deterministic_budgets.unwrap_or(true)` (settings.rs:1926, the
//!    `:2809` pin "default ON") and `--deterministic-budgets=off`
//!    arms the fanout's WALL-CLOCK `max_milliseconds_per_pin` budget
//!    (`fanout.rs:812`), i.e. OFF is the NONDETERMINISTIC face. A
//!    session default of `false` would make the workflow parity pin
//!    nondeterministic. The dispatch's "false = the CLI default"
//!    contradicts its own anchor (`route.rs:939` feeds
//!    `args.deterministic_budgets`, which stays `None` at defaults
//!    and resolves ON).
//! 3. `apply_copper_to_edge_clearance_override` is RELOCATED here
//!    from epic-cli route.rs (byte-verbatim, the T1 pattern) — the
//!    third relocation, which the dispatch's "two relocations" count
//!    missed: the load prelude it specifies (step 3) calls this
//!    function, epic-engine cannot depend on epic-cli (the dependency
//!    arrow is the other way), and duplicating the Java-anchored body
//!    would break the same-code law. Pure move; the CLI call site
//!    re-imports; byte-invariance is proven by the standing battery.
//!
//! The session mode has NO output-file deletion: route.rs:880-882
//! is CLI `-do` semantics (a failed RUN must not leave the previous
//! run's session in place); the session's export is EXPLICIT — the
//! host owns the file lifecycle, [`Session::export_ses`] only ever
//! writes behind the COMPLETED/TIMED_OUT gate.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use epic_board::board::Board;
use epic_board::items::ItemData;
use epic_board::tree_manager::SearchTreeManager;
use epic_drc::clearance::all_clearance_violation_depths;
use epic_drc::incompletes::airline_segments;
use epic_dsn::reader::{DsnReadResult, read_board};
use epic_dsn::ses::writer::write_session;
use epic_dsn::ses_board::SesBoard;
use epic_dsn::sink::ItemClassIr;
use epic_dsn::state::Unit as DsnUnit;
use epic_geometry::rounding::java_round;
use epic_router::global::map::CongestionMap;
use epic_router::pipeline::batch::{StopFace, final_state_for};
use epic_router::pipeline::board_statistics::BoardStatistics;
use epic_router::pipeline::event_sink::DriverSink;
use epic_router::pipeline::full;
use epic_router::pipeline::pass_runner::RouterCounters;

use crate::export::{filename_without_extension, project_routed_items};
use crate::settings::{
    CliLayer, DEFAULT_COPPER_TO_EDGE_CLEARANCE_UM, DsnLayer, MergedSettings, ResolvedRouteSettings,
    SessionLayer, apply_board_specific_optimizations, apply_meander_activation,
    apply_pairs_activation, apply_tuning_activation, build_batch_settings, merge, merge_session,
    validate,
};
use crate::snapshot::{
    AirLinePrimitive, BoardSnapshot, CongestionCell, CongestionHeatmap, MarkerPhase, NetTuningInfo,
    PointPrimitive, board_snapshot,
};

// ---------------------------------------------------------------------------
// the relocated load-time copper-to-edge override (epic-cli route.rs,
// byte-verbatim at `4a0d7b3f5` — see the module-docs deviation note 3)
// ---------------------------------------------------------------------------

/// Java `HeadlessBoardManager.BOARD_EDGE_CLEARANCE_CLASS_NAME` (`:88`).
const BOARD_EDGE_CLEARANCE_CLASS_NAME: &str = "board_edge";

/// Java `HeadlessBoardManager.applyCopperToEdgeClearanceOverride`
/// (`HeadlessBoardManager.java:470-556`, ported body `:471-548`; the
/// trailing `FRLogger.debug` success log at `:550-555` is deliberately
/// omitted — the port has no debug-logging surface here). The load-time
/// settings
/// behavior the port lacked ENTIRELY — the buglog-189 root cause
/// (M5-T2): on every manager DSN load whose outline sits at the
/// fallback default-area clearance class, the CLI promotes the outline
/// item to the appended `board_edge` class at 2500 DBU, so the class-1
/// autoroute tree's outline leaf grows
/// `halfWidth + (2500 − compValue(1,0))` = 1600 instead of 1100 and
/// the first fanout search's start-room completion lands 500 DBU
/// further out (bm06: north edge −911636 vs the port's −911136).
///
/// Placement: Java applies this from the MANAGER load flow at two call
/// sites — `:346` (`createBoard`, fired by the DSN parser at
/// `Structure.java:1268`, when only the default tree exists) and `:750`
/// (`applyRouterSettingsForLoadedBoard`, the post-load face). The
/// skip-gate (`:509-511`) makes the second application a no-op on the
/// default-value path (idempotent on the non-default path: the append
/// dedups, the set rewrites the same values, the remove/insert is
/// stable), so ONE parse-time call reproduces the CLI's final state —
/// and Java's EFFECTIVE ordering puts this state BEFORE everything the
/// Rust flow does next: the wiring scope's `normalizeAllTraces`
/// (`Wiring.java:347`), `applyBoardSpecificOptimizations` (`:749`),
/// and the deferred pre-existing-violation seed (`:788-793`), whose
/// background DRC reads the promoted board. The port therefore fires
/// the call right after the default-tree fill, BEFORE
/// `normalize_all_traces` and the seed, in every flow that replicates
/// the manager walk: [`run_route`], and the harness's in-process
/// detail/alloc replicators. It is NOT part of `DsnReader.readBoard` —
/// the direct-read flows (every harness oracle probe, the events
/// corpus capture: `RouteEventProbe.java:198-205` calls
/// `DsnReader.readBoard` directly) bypass the manager and MUST NOT
/// apply this. One value note: Java's `:346` face reads the job's
/// routerSettings before the DSN layer is applied, the Rust call reads
/// the fully merged settings — but NO settings source writes
/// `copperToEdgeClearanceUm` in the frozen tree (only
/// `DefaultSettings.java:155` seeds it), so the value is identical.
///
/// Sequence, Java-exact (`:471-548`):
/// 1. Null guards (`:471-476`) — the `Option` here; the board/rules/
///    matrix null guards are structural impossibilities in the port
///    (`BoardRules` always carries a matrix).
/// 2. Negative µm → warn + skip (`:478-484`).
/// 3. Outline unavailable → warn + skip (`:491-496`).
/// 4. Skip-gate (`:504-511`): `usesDefaultEdgeClearanceValue &&
///    !usesFallbackOutlineClass` → untouched. An EXPLICIT DSN outline
///    class survives the default value; a fallback-class outline IS
///    promoted even at the default (both faces pinned in epic-cli's
///    route.rs test module, which exercises the relocated fn through
///    the re-import).
/// 5. µm → board units (`:513-520`):
///    `round(um * max(1, resolution))`, `Unit.scale(.., UM, unit)` —
///    bm06: 250 µm × resolution 10 = 2500 DBU.
///
///    Step 5b, inserted by a917044ff (upstream #935) between 5 and 6:
///    the DEFAULT-VALUE pin-gap cap — `floor(max(0,
///    outline.minimumPinGap()))` when that gap is smaller than the
///    converted default. The default is only a guess and must never
///    exceed what the input design already has between its pins and
///    the outline; inert without pins (+∞); never applied to explicit
///    values. (Java re-runs the whole override after pin insertion
///    via `edgeClearanceAppliedByOverride`; the port's single
///    post-load call already has the pins — see the inline note.)
///
/// 6. `board_edge` class: `get_no` (`:523-524`), `append_class` when
///    absent (`:525`), warn+skip when still absent (`:528-532`).
/// 7. Full symmetric set for EVERY class ≥ 1 on EVERY layer
///    (`:534-539`) — class 0 stays at the `append_class` init value
///    `v(1, 0)`; the even-rounding of `set_value` applies.
/// 8. The TARGETED reclassification (`:541-548` — the T2 spec-review
///    port guidance): `remove` → `set_item_clearance_class` →
///    `clear_derived_data` → `insert`. NOT a bulk reinsert.
pub fn apply_copper_to_edge_clearance_override(
    settings: &MergedSettings,
    manager: &mut SearchTreeManager,
    board: &mut Board,
) {
    let Some(configured_clearance_um) = settings.copper_to_edge_clearance_um else {
        return; // Java `:471-476`.
    };
    if configured_clearance_um < 0.0 {
        eprintln!(
            "Warning: Ignoring router.copper_to_edge_clearance_um because it is negative: {configured_clearance_um}"
        );
        return;
    }
    // Java's `board.rules == null || rules.clearanceMatrix == null`
    // guards (`:485-489`) are structural impossibilities here.
    let Some(outline_id) = board
        .iter_descending()
        .find(|entry| matches!(entry.data, ItemData::BoardOutline { .. }))
        .map(|entry| entry.id)
    else {
        eprintln!(
            "Warning: Ignoring router.copper_to_edge_clearance_um because the board outline is unavailable."
        );
        return;
    };
    let default_area_class_no =
        board.rules().default_item_clearance_classes[ItemClassIr::Area as usize];
    let uses_fallback_outline_class =
        board.item_clearance_class(outline_id) == Some(default_area_class_no);
    let uses_default_edge_clearance_value =
        (configured_clearance_um - DEFAULT_COPPER_TO_EDGE_CLEARANCE_UM).abs() < 1e-9;
    // Keep explicit DSN outline-clearance classes untouched when only
    // the global default is active (Java `:504-511`).
    if uses_default_edge_clearance_value && !uses_fallback_outline_class {
        return;
    }
    let board_resolution = board.communication().resolution.max(1);
    let configured_clearance_board_units = java_round(DsnUnit::scale(
        configured_clearance_um * f64::from(board_resolution),
        DsnUnit::Um,
        board.communication().unit,
    )) as i32;
    // Java a917044ff `:544-557` (upstream #935): the default value is
    // only a guess — never demand more edge clearance than the input
    // design already has between its pins and the outline, or a pin
    // near the edge blocks every connection. Explicit values are
    // never capped (the gate is `usesDefaultEdgeClearanceValue`, the
    // same predicate as the skip-gate). The FRLogger.debug row stays
    // comment-only (the parity-warnings law). No re-run flag either:
    // Java's FIRST call fires at parse time (`Structure.java:1268`,
    // before the pins exist — `minimumPinGap` is +∞, the cap inert)
    // and re-runs after pin insertion through
    // `edgeClearanceAppliedByOverride`; this port calls the override
    // ONCE post-load with the pins present — the capped second-call
    // state directly.
    let configured_clearance_board_units = if uses_default_edge_clearance_value {
        let minimum_pin_gap = board.outline_minimum_pin_gap(outline_id);
        if minimum_pin_gap < f64::from(configured_clearance_board_units) {
            minimum_pin_gap.floor().max(0.0) as i32
        } else {
            configured_clearance_board_units
        }
    } else {
        configured_clearance_board_units
    };
    let matrix = &mut board.rules_mut().clearance;
    let mut board_edge_class_no = matrix.get_no(BOARD_EDGE_CLEARANCE_CLASS_NAME);
    if board_edge_class_no < 0 {
        matrix.append_class(BOARD_EDGE_CLEARANCE_CLASS_NAME);
        board_edge_class_no = matrix.get_no(BOARD_EDGE_CLEARANCE_CLASS_NAME);
    }
    if board_edge_class_no < 0 {
        eprintln!(
            "Warning: Unable to create/find the board_edge clearance class for copper-to-edge override."
        );
        return;
    }
    // The full symmetric set: EVERY class >= 1 on EVERY layer, BOTH
    // directions (Java `:534-539`). Class 0 stays at the `append_class`
    // init value `v(1, 0)` — Java's loop starts at class 1.
    for layer in 0..matrix.layer_count() as i32 {
        for class_no in 1..matrix.class_count() as i32 {
            matrix.set_value(
                board_edge_class_no,
                class_no,
                layer,
                configured_clearance_board_units,
            );
            matrix.set_value(
                class_no,
                board_edge_class_no,
                layer,
                configured_clearance_board_units,
            );
        }
    }
    // The TARGETED reclassification (Java `:541-548`, the T2 spec-review
    // port guidance): remove the outline from every live tree, reclass
    // it, clear its derived caches, reinsert. NOT a bulk reinsert.
    manager.remove(board, outline_id);
    board.set_item_clearance_class(outline_id, board_edge_class_no);
    board.clear_derived_data(outline_id);
    manager.insert(board, outline_id);
}

// ---------------------------------------------------------------------------
// the load classification
// ---------------------------------------------------------------------------

/// Which success-shaped arm [`read_board`] landed on — the session
/// treats both as warn-and-continue (the CLI face,
/// route.rs:859-864), but the caller of [`Session::load_dsn`]
/// distinguishes them through [`LoadError::OutlineMissing`].
#[derive(Debug, PartialEq, Eq)]
enum ReadOutcome {
    /// `DsnReadResult::Success`.
    Loaded,
    /// `DsnReadResult::OutlineMissing` — the default boundary stands.
    LoadedOutlineMissing,
}

/// The four-arm `read_board` mapping (route.rs:856-871, session
/// face): the two success arms collect their parity warnings and
/// continue; `ParseError` is a hard `LoadError::Parse` (the CLI's
/// `parse error at {location}: {detail}` text); `IoError` is a hard
/// `LoadError::Io` — the port reads in-memory bytes so the reader
/// never constructs it (epic-dsn reader.rs module docs), and the arm
/// is pinned by direct construction in this module's unit tests.
fn classify_read_result(
    result: DsnReadResult,
    warnings: &mut Vec<String>,
) -> Result<ReadOutcome, LoadError> {
    let outcome = if matches!(result, DsnReadResult::OutlineMissing { .. }) {
        ReadOutcome::LoadedOutlineMissing
    } else {
        ReadOutcome::Loaded
    };
    match result {
        DsnReadResult::Success { warnings: rows }
        | DsnReadResult::OutlineMissing { warnings: rows } => {
            warnings.extend(rows);
            Ok(outcome)
        }
        DsnReadResult::ParseError { location, detail } => Err(LoadError::Parse(format!(
            "parse error at {location}: {detail}"
        ))),
        DsnReadResult::IoError => Err(LoadError::Io(
            "I/O error reading the in-memory design (the port reads bytes; the reader \
             reserves this arm for stream failures, never constructed from bytes)"
                .to_string(),
        )),
    }
}

// ---------------------------------------------------------------------------
// the public faces
// ---------------------------------------------------------------------------

/// The [`Session::load_dsn`] failure faces (the dispatch shape):
/// `Parse`/`Io` are hard failures; `OutlineMissing` is the
/// WARN-AND-CONTINUE face — the loaded session is RETURNED so the
/// caller can route it (the CLI proceeds with the default boundary,
/// route.rs:859-864). Manual `Debug`: `Session` deliberately
/// carries none (its fields are engine-internal), so the arm prints
/// without it.
pub enum LoadError {
    /// `DsnReadResult::ParseError` — Java's `parse error at ...`.
    Parse(String),
    /// `DsnReadResult::IoError` (never produced from in-memory bytes —
    /// shape parity; see [`classify_read_result`]).
    Io(String),
    /// The board loaded without a usable outline; the session is
    /// handed BACK (warn-and-continue — the default boundary stands,
    /// exactly the CLI's face). The parse warnings ride the session's
    /// [`Session::warnings`]. Boxed (clippy::result_large_err /
    /// large_enum_variant): the dispatch sketch's bare `Session`
    /// payload is >2 KB — the box is the standard spelling of the
    /// SAME shape, callers match-and-deref.
    OutlineMissing(Box<Session>),
}

impl std::fmt::Debug for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Parse(detail) => f.debug_tuple("Parse").field(detail).finish(),
            LoadError::Io(detail) => f.debug_tuple("Io").field(detail).finish(),
            LoadError::OutlineMissing(_) => {
                f.debug_tuple("OutlineMissing").field(&"<Session>").finish()
            }
        }
    }
}

/// The [`Session::route`] failure face (the Q6 product decision,
/// M10-T2, pre-made at charter level): re-route stays DISABLED — ONE
/// route per `Session`; a re-route requires a fresh session
/// ([`Session::load_dsn`]). The probe (2026-09-30, pre-change tree,
/// `logs/M10-T2/evidence/10-q6-probe.log`) measured the prior face:
/// a second call returned `Ok` SILENTLY — the pipeline re-ran on the
/// live board and overwrote `final_state`/`statistics` with no
/// refusal and no panic (e1_ripup: two COMPLETED runs, rev 651 twice,
/// identical SES bytes — a no-op re-route HERE, but a silent
/// geometry-mutating re-optimization on any board the pull-tight
/// stage can still tighten).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteError {
    /// A route run already completed on this session. ANY prior final
    /// state arms the guard (`final_state` is set at the end of EVERY
    /// [`Session::route`] call — COMPLETED, CANCELLED, TERMINATED, and
    /// TIMED_OUT alike), and the guard fires BEFORE any settings
    /// merge or pipeline work: the board, the statistics, and the
    /// stored final state are untouched.
    AlreadyRouted {
        /// The prior run's final state.
        final_state: String,
    },
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyRouted { final_state } => write!(
                f,
                "cannot route: the session has already routed once (final state \
                 {final_state}); a re-route requires a fresh session (Session::load_dsn)"
            ),
        }
    }
}

impl std::error::Error for RouteError {}

/// One route run's summary (the dispatch shape). `passes` is the
/// AUTOROUTE phase's final pass count — the last
/// `board_updated` counters row whose `phase == "autoroute"`
/// (route.rs:1078's face; 0 when the phase emitted no row — an
/// external stop before pass 1, a router-disabled run, or the
/// all-layers-disabled abort).
#[derive(Debug, Clone)]
pub struct RouteSummary {
    /// The final-state mapping (`route.rs:1015-1019`): TERMINATED on a
    /// driver error, COMPLETED on `Ok(true)`, else
    /// [`final_state_for`]'s face (CANCELLED only for an EXTERNAL
    /// stop).
    pub final_state: String,
    /// `stats.connections.incomplete_count` (0 when the walk skipped —
    /// the CLI's manifest face).
    pub incomplete_count: i64,
    /// `stats.clearance_violations.total_count` (0 when `None`).
    pub violations_total: i64,
    /// The autoroute phase's last pass count (see the struct docs).
    pub passes: i32,
}

/// The post-route statistics face ([`Session::statistics`]):
/// [`BoardStatistics::new`]'s full walk + the incomplete/violation
/// faces + both scores on the SAME Java two-decimal face the manifest
/// ships (route.rs:1024-1032; the session's consumer is the T3+ event
/// stream, which needs no separate raw face — if one is ever needed,
/// add a field, do not reinterpret these).
///
/// `PartialEq`, not `Eq`: the score fields are f64 (and
/// [`BoardStatistics`] is `PartialEq`-only over its f32 fields). The
/// consumer is the T2 fix-round pin's whole-struct equality witness —
/// equality, never ordering, is the contract.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionStatistics {
    /// The full `BoardStatistics::new` walk.
    pub stats: BoardStatistics,
    /// `connections.incomplete_count` (0 when the walk skipped).
    pub incomplete_count: i64,
    /// `clearance_violations.total_count` (0 when `None`).
    pub violations_total: i64,
    /// The router score, `java_two_decimal`-faced (the manifest's
    /// `normalized_score` face).
    pub router_score: f64,
    /// The optimizer score, two-decimal-faced (route.rs `:201`'s value
    /// source).
    pub optimizer_score: f64,
}

impl SessionStatistics {
    /// The pre-route face: `BoardStatistics::new_empty()` and zero
    /// faces (an honest empty — `statistics()` is non-optional by the
    /// dispatch shape, and the walk needs `&mut` faces the read-only
    /// getter cannot take; see [`Session::statistics`]).
    #[must_use]
    fn empty() -> Self {
        Self {
            stats: BoardStatistics::new_empty(),
            incomplete_count: 0,
            violations_total: 0,
            router_score: 0.0,
            optimizer_score: 0.0,
        }
    }
}

/// The headless application session: one loaded board + its search
/// tree + the parse SES carrier + the two session-owned settings
/// layers + the shared stop flag. Constructed ONLY through
/// [`Session::load_dsn`].
pub struct Session {
    /// The live board.
    board: Board,
    /// The spatial index over the board.
    manager: SearchTreeManager,
    /// The parse SES carrier (the export's input face).
    ses: SesBoard,
    /// The DSN settings layer parsed at load (priority 20).
    dsn_layer: DsnLayer,
    /// The session-owned settings layer (priority > 60; merged above
    /// the CLI layer on every [`Session::route`]).
    session_layer: SessionLayer,
    /// The shared cancel flag ([`Session::request_cancel`] raises;
    /// `StopFace::from_flag` reads).
    stop_flag: Arc<AtomicBool>,
    /// The load-time violation seed (route.rs:930-931, the
    /// `HeadlessBoardManager.java:788-793` face).
    pre_existing_violations: i32,
    /// The last run's final state (the export gate's input).
    final_state: Option<String>,
    /// The parse + validate warnings surfaced so far (the CLI echoes
    /// its load-time warnings to stderr; the session COLLECTS them —
    /// the host decides how to render).
    warnings: Vec<String>,
    /// The budget discipline flag (DEVIATION NOTE 2: default TRUE —
    /// the CLI's resolved default). Settable via
    /// [`Session::set_deterministic_budgets`].
    deterministic_budgets: bool,
    /// The input file name the SES design face is derived from (the
    /// CLI reads it off the `-de` path, route.rs:1103-1106; the
    /// session gets bytes, so the host sets it — default `board.dsn`,
    /// the CLI's own fallback spelling).
    input_name: String,
    /// The post-route statistics ([`Session::route`] fills;
    /// [`Session::statistics`] returns it — empty before the first
    /// run).
    statistics: Option<SessionStatistics>,
    /// Whether the LAST [`Session::route`] resolved the
    /// `congestion_global` family ON (the resolved batch flag — the
    /// probe's reachable ON path is the `SessionLayer`/`CliLayer`
    /// `router.congestion_global` field). The congestion slot of
    /// [`Session::snapshot_with_overlays`] populates IFF this is set:
    /// the heatmap is a RECOMPUTED `CongestionMap::build` over the
    /// boundary board state, shown only when congestion-aware
    /// planning engaged. A session that never routed carries `false`
    /// → `None` (the Option-inertness pin).
    congestion_engaged: bool,
    /// The last run's F1 pin auto-assignment report (the route head's
    /// `assign.pins` face — `None` until a route ran with the face
    /// declared; an empty-refs run stores the empty report). Read via
    /// [`Session::pin_assign_report`].
    last_pin_assign_report: Option<crate::pin_assign::PinAssignReport>,
}

impl Session {
    /// The load face (the run_route prelude, route.rs:856-931):
    /// bytes -> parse -> DSN layer -> board build + tree fill + the
    /// copper-to-edge override + trace normalization -> the
    /// pre-existing-violation seed.
    ///
    /// The ONE ordering subtlety (documented per the dispatch): the
    /// override consumes a MERGED model, and at load there is no CLI
    /// layer yet — so the load-time merge sees Default+DSN+SESSION
    /// only. This mirrors the CLI exactly: there the override sees
    /// Default+DSN+CLI at parse time because the CLI layer exists from
    /// argv. Both orderings observe the same copper-to-edge VALUE:
    /// Java applies no DSN or CLI source to `copperToEdgeClearanceUm`
    /// (only `DefaultSettings.java:155` seeds it — the
    /// route.rs:884-890 comment, stated not re-derived), and
    /// `SessionLayer` carries no twin for that field for the same
    /// reason (the T2 re-audit's load-time-only exclusion).
    ///
    /// `validate` is NOT run at load: its output feeds only the
    /// override here (whose consumed field validate never touches);
    /// the route-time merge runs it (warnings via the sink).
    ///
    /// # Errors
    ///
    /// [`LoadError::Parse`]/[`LoadError::Io`] hard-fail;
    /// [`LoadError::OutlineMissing`] returns the loaded session.
    pub fn load_dsn(bytes: &[u8], session: SessionLayer) -> Result<Session, LoadError> {
        let mut warnings: Vec<String> = Vec::new();
        let mut ses = SesBoard::new();
        // 1. The four-arm read (route.rs:856-871). NO output-file
        //    deletion here — that is CLI `-do` semantics
        //    (route.rs:880-882); the session's export is explicit
        //    (module docs).
        let outcome = classify_read_result(read_board(bytes, &mut ses), &mut warnings)?;
        // 2. The DSN settings layer (route.rs:893-896).
        let dsn_layer = DsnLayer::from_metadata(
            ses.metadata.autoroute_settings.as_ref(),
            usize::try_from(ses.metadata.layer_count).unwrap_or(0),
        );
        // 3. The load-time merged model for the override:
        //    Default + DSN + SESSION (no CLI yet — module docs).
        let mut load_merged = merge(&MergedSettings::default(), &dsn_layer, &CliLayer::default());
        merge_session(&mut load_merged, &session);
        // 4. Board build + tree fill + override + normalization
        //    (route.rs:905-919; the override placement comment
        //    lives on the relocated fn).
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        apply_copper_to_edge_clearance_override(&load_merged, &mut manager, &mut board);
        epic_board::normalize_all::normalize_all_traces(&mut manager, &mut board);
        // 5. The load-time violation seed (route.rs:930-931).
        let (pre_total, _) = all_clearance_violation_depths(&mut manager, &mut board);
        board.pre_existing_clearance_violations_count =
            i32::try_from(pre_total).unwrap_or(i32::MAX);
        let pre_existing = board.pre_existing_clearance_violations_count;
        let session = Session {
            board,
            manager,
            ses,
            dsn_layer,
            session_layer: session,
            stop_flag: Arc::new(AtomicBool::new(false)),
            pre_existing_violations: pre_existing,
            final_state: None,
            warnings,
            // DEVIATION NOTE 2: the CLI's resolved default is ON
            // (`resolve`'s `unwrap_or(true)`, the `:2809` pin).
            deterministic_budgets: true,
            input_name: "board.dsn".to_string(),
            statistics: None,
            congestion_engaged: false,
            last_pin_assign_report: None,
        };
        match outcome {
            ReadOutcome::Loaded => Ok(session),
            ReadOutcome::LoadedOutlineMissing => Err(LoadError::OutlineMissing(Box::new(session))),
        }
    }

    /// One route run (the run_route settings block + pipeline,
    /// route.rs:897-1042): the fresh merge (Default -> DSN -> CLI ->
    /// SESSION) -> validate -> the board-specific pass -> resolve ->
    /// the batch build + activation predicates -> the explicit-
    /// engagement thread faces -> `full::run` over the caller's sink
    /// behind the session's stop flag -> the final-state mapping and
    /// the statistics walk.
    ///
    /// ONE route per session (the Q6 product decision, M10-T2): a
    /// second call returns [`RouteError::AlreadyRouted`] BEFORE any
    /// merge or pipeline work; a re-route requires a fresh session
    /// ([`Session::load_dsn`]). The prior face was a documented
    /// "re-runnable" silent re-run (the probe citation on
    /// [`RouteError`]).
    /// The route-time validate warnings surface through the SINK
    /// (`warn`) — the session's host-log face; the load-time parse
    /// warnings live on [`Session::warnings`].
    ///
    /// The pairs activation's raw tuple output is deliberately
    /// dropped here: the manifest mapping is CLI-only (route.rs
    /// `:1073-1076`); the batch mutation is what the pipeline reads.
    ///
    /// # Errors
    ///
    /// [`RouteError::AlreadyRouted`] on any second call (the guard is
    /// the session's stored `final_state` — every prior run's face).
    pub fn route(
        &mut self,
        cli: &CliLayer,
        sink: &mut dyn DriverSink,
    ) -> Result<RouteSummary, RouteError> {
        // The Q6 guard (M10-T2): re-route stays DISABLED — one route
        // per session; a re-route requires a fresh Session (load_dsn).
        // Fires BEFORE any merge/pipeline work (nothing mutated).
        if let Some(final_state) = &self.final_state {
            return Err(RouteError::AlreadyRouted {
                final_state: final_state.clone(),
            });
        }
        // The settings order (route.rs:897-900, session face): the
        // T1 slot merged ABOVE the CLI layer, then the trailing
        // validate.
        let mut merged = merge(&MergedSettings::default(), &self.dsn_layer, cli);
        merge_session(&mut merged, &self.session_layer);
        for warning in validate(&mut merged) {
            sink.warn(&warning);
        }
        // F1 (Rust-only): the pin auto-assignment face runs at the
        // route head — AFTER the merge (the list can arrive from any
        // layer) and BEFORE every geometry/activation pass (the
        // mutated netlist must be the one the pipeline routes).
        // Unresolved refs warn through the sink and never fail the
        // run; the report is kept for the host/manifest.
        if let Some(refs) = merged.assign_pins.clone() {
            let report =
                crate::pin_assign::apply_pin_assignments(&mut self.board, &mut self.manager, &refs);
            for reason in &report.unresolved {
                sink.warn(&format!("pin assignment: {reason}"));
            }
            self.last_pin_assign_report = Some(report);
        }
        // 3. The unconditional geometry pass (route.rs:938-939).
        apply_board_specific_optimizations(&mut merged, &self.board);
        let resolved = ResolvedRouteSettings::resolve(&merged, Some(self.deterministic_budgets));
        // 4. The batch build + the three activation predicates
        //    (route.rs:949-971; the raw tuples are fine here — the
        //    manifest mapping is CLI-only).
        let mut batch = build_batch_settings(&resolved);
        apply_tuning_activation(&mut batch, &resolved, self.board.rules());
        apply_meander_activation(&mut batch, &resolved);
        let (_pair_specs, _unresolved_pairs) =
            apply_pairs_activation(&mut batch, &resolved, &self.board);
        // The honoring face reads the RESOLVED activation off the
        // board (route.rs:974-977).
        self.board.set_tuning_active(batch.tuning_active);
        // The threads faces (route.rs:981-1001) — the explicit-
        // engagement law extended to the session layer: the executor
        // engages on an EXPLICIT CLI *or* SESSION value; the default
        // stays 1 (the golden sequential path) even though the merged
        // surface resolves Java's parity default in validate().
        batch.max_threads = if cli.max_threads.is_some() || self.session_layer.max_threads.is_some()
        {
            merged
                .max_threads
                .expect("explicit CLI/session max_threads is Some post-validate")
        } else {
            1
        };
        batch.optimizer_threads =
            if cli.optimizer_threads.is_some() || self.session_layer.optimizer_threads.is_some() {
                merged.optimizer_threads.unwrap_or(1).max(1).unsigned_abs() as usize
            } else {
                1
            };
        // The pipeline (route.rs:1003-1012) — the session's StopFace
        // rides the shared flag (`StopFace::default()` is the flagless
        // equivalent the CLI uses). The congestion engagement is
        // recorded BEFORE the batch moves (the resolved
        // `congestion_global` master — the snapshot attach step's
        // gate).
        self.congestion_engaged = batch.congestion_global;
        let mut pass_sink = PassTrackingSink::new(sink);
        let outcome = full::run(
            &mut self.manager,
            &mut self.board,
            batch,
            resolved.optimizer.clone(),
            resolved.run_optimizer,
            StopFace::from_flag(Some(Arc::clone(&self.stop_flag))),
            &mut pass_sink,
        );
        // The final-state mapping (route.rs:1015-1019).
        let final_state: String = match &outcome.routing {
            Err(_) => "TERMINATED".to_string(),
            Ok(true) => "COMPLETED".to_string(),
            Ok(false) => final_state_for(false, outcome.stop_reason).to_string(),
        };
        // 5. The post-route statistics walk (route.rs:1022-1042,
        //    summary faces). `BoardStatistics::new` needs `&mut` —
        //    which is why `statistics()` returns the STORED walk (the
        //    read-only getter cannot re-walk).
        let stats = BoardStatistics::new(&mut self.manager, &mut self.board);
        let incomplete_count = i64::from(stats.connections.incomplete_count.unwrap_or(0));
        let violations_total = i64::from(stats.clearance_violations.total_count.unwrap_or(0));
        let session_stats = SessionStatistics {
            router_score: java_two_decimal(stats.get_router_score(Some(&resolved.scoring))),
            optimizer_score: java_two_decimal(stats.get_optimizer_score(Some(&resolved.scoring))),
            incomplete_count,
            violations_total,
            stats,
        };
        self.statistics = Some(session_stats.clone());
        self.final_state = Some(final_state.clone());
        self.pre_existing_violations = self.board.pre_existing_clearance_violations_count;
        Ok(RouteSummary {
            final_state,
            incomplete_count,
            violations_total,
            passes: pass_sink.last_autoroute_pass(),
        })
    }

    /// The cancel face: raises the shared flag the pipeline's
    /// `StopFace` polls. Timing semantics are the PIPELINE's (the
    /// session path is the CLI path): a raise MID-RUN is the
    /// external-stop face (`StopReason::UserStop` -> CANCELLED, the
    /// hoisted `epic_router::pipeline::batch::final_state_for` —
    /// CANCELLED only for an EXTERNAL stop); a raise BEFORE
    /// [`Session::route`] hits
    /// the pipeline's PRE-STAGE check, which SKIPS the routing stage
    /// entirely (pass-through `Ok(true)` -> COMPLETED — full.rs
    /// `router_enabled && !stop.is_requested()`). Both faces are
    /// pinned by name in `session_workflow.rs`: the mid-run CANCELLED
    /// face in `cancel_mid_run_cancels_and_refuses_export`, the
    /// pre-route pass-through COMPLETED face in
    /// `pre_route_cancel_passes_through_completed` (this method's
    /// exercised caller).
    pub fn request_cancel(&self) {
        // Relaxed: the pipeline polls with `Ordering::Relaxed`
        // (batch.rs `is_requested`); the flag is a boolean hint whose
        // only contract is eventual visibility — matching the CLI's
        // signal-handler face.
        self.stop_flag.store(true, Ordering::Relaxed);
    }

    /// The shared stop flag itself — the host's MID-RUN cancel face:
    /// a worker thread (the GUI button) clones this Arc and stores
    /// `true` while [`Session::route`] runs, which is what turns a
    /// running pipeline into CANCELLED (the pre-route raise is the
    /// pass-through face — see [`Self::request_cancel`]).
    #[must_use]
    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stop_flag)
    }

    /// The revision tick for snapshot dirty-checks (board.rs `:466`,
    /// the u64 dirty tick).
    #[must_use]
    pub fn board_revision(&self) -> u64 {
        self.board.revision()
    }

    /// Read-only board access (the renders-never-mutates law:
    /// `&Board` ONLY — no mutation surface leaks through the session).
    #[must_use]
    pub fn board(&self) -> &Board {
        &self.board
    }

    /// The SES export, gated EXACTLY like route.rs:1110-1119: only
    /// COMPLETED/TIMED_OUT produce a file — every other face (and a
    /// no-route-yet session) returns `Err` WITHOUT touching `path`.
    /// Success: the SAME projection + writer the CLI step 6 uses (the
    /// relocated [`project_routed_items`] + [`write_session`] +
    /// [`filename_without_extension`] over [`Self::input_name`]),
    /// then one `fs::write`.
    ///
    /// # Errors
    ///
    /// The state gate (no file written), or the unwritable path (the
    /// CLI's `cannot write session ...` text).
    pub fn export_ses(&mut self, path: &Path) -> Result<(), String> {
        let Some(final_state) = &self.final_state else {
            return Err(
                "no session file: the board has not been routed yet (only COMPLETED/TIMED_OUT \
                 produce output)"
                    .to_string(),
            );
        };
        if !matches!(final_state.as_str(), "COMPLETED" | "TIMED_OUT") {
            return Err(format!(
                "no session file: final state {final_state} produces no output (only \
                 COMPLETED/TIMED_OUT do, the route.rs:1110-1119 gate)"
            ));
        }
        project_routed_items(&self.board, &mut self.ses);
        let ses_text = write_session(&self.ses, filename_without_extension(&self.input_name));
        std::fs::write(path, ses_text)
            .map_err(|error| format!("cannot write session {}: {error}", path.display()))
    }

    /// The post-route statistics (the STORED walk —
    /// `BoardStatistics::new` needs `&mut` faces, so the read-only
    /// getter cannot re-walk; the empty face before the first run).
    #[must_use]
    pub fn statistics(&self) -> SessionStatistics {
        self.statistics
            .clone()
            .unwrap_or_else(SessionStatistics::empty)
    }

    /// The parse warnings collected at load (the CLI echoes its
    /// load-time warnings to stderr; the session collects).
    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The last run's F1 pin auto-assignment report (`None` until a
    /// route ran with `assign.pins` declared — the field docs).
    #[must_use]
    pub fn pin_assign_report(&self) -> Option<&crate::pin_assign::PinAssignReport> {
        self.last_pin_assign_report.as_ref()
    }

    /// The input file name the SES design face is derived from (the
    /// CLI reads it off the `-de` path; the session gets bytes). MUST
    /// be set before [`Session::export_ses`] for byte parity with the
    /// CLI on the same input — the parity pin does exactly that.
    pub fn set_input_name(&mut self, input_name: String) {
        self.input_name = input_name;
    }

    /// The budget-discipline flag (see the field docs / DEVIATION
    /// NOTE 2). Default TRUE.
    pub fn set_deterministic_budgets(&mut self, deterministic_budgets: bool) {
        self.deterministic_budgets = deterministic_budgets;
    }

    /// The load-time violation seed (the manifest's
    /// `pre_existing_violations` face).
    #[must_use]
    pub fn pre_existing_violations(&self) -> i32 {
        self.pre_existing_violations
    }

    /// The depth-walk total at the CURRENT boundary —
    /// [`all_clearance_violation_depths`]'s first element, recomputed
    /// (the `&mut` walk the attach step runs; exposed so a pin can
    /// cross-check the attach step's marker count against the SAME
    /// walk at the SAME boundary — the T5 fix-round F1 face). NOT a
    /// hot path: one DRC walk per call.
    #[must_use]
    pub fn clearance_violation_depth_total(&mut self) -> i64 {
        let (total, _) = all_clearance_violation_depths(&mut self.manager, &mut self.board);
        total
    }

    /// The M9-T5 ATTACH STEP (the one `&mut` host of the overlay
    /// faces — the pure [`board_snapshot`] builder cannot run them):
    /// derive the pure snapshot, then attach all four overlay faces
    /// computed at the CURRENT session boundary.
    ///
    /// Per-face documentation:
    ///
    /// * **airlines** — [`airline_segments`] (the Kruskal-ACCEPTED
    ///   subset with ratsnest-corner endpoints), projected onto
    ///   [`AirLinePrimitive`]s, wire order preserved.
    /// * **violation_markers** — [`all_clearance_violation_depths`]
    ///   recomputed at the boundary; each row becomes a
    ///   [`ViolationMarker`] (center = the two items' bbox-center
    ///   midpoint, depth = `expected − actual` in board units,
    ///   pair_kind = the items' `BoardItemType` declaration
    ///   ordinals). PHASE: [`MarkerPhase::Parse`] before the first
    ///   route (the load seed boundary — the walk recomputed on the
    ///   unchanged board is the seed's own face),
    ///   [`MarkerPhase::PostRoute`] after any [`Session::route`].
    /// * **congestion** — populated ONLY when the last resolved route
    ///   engaged `congestion_global` (the `congestion_engaged` field
    ///   doc); the heatmap is a RECOMPUTED [`CongestionMap::build`]
    ///   over the boundary board state — the same pure builder the
    ///   global stage uses, NOT the mid-run artifact (no live map
    ///   survives a planning face). Cells with `overflow > 0` only,
    ///   raw occupancy (no net excluded).
    /// * **tuning** — `Some` iff `has_length_constraints()`; one
    ///   [`NetTuningInfo`] per net whose resolved
    ///   `net_class_length_bounds` carry a non-zero bound (the SAME
    ///   `> 0.0` predicate the activation uses), nets ascending,
    ///   `actual` from [`Board::net_trace_length`].
    #[must_use]
    pub fn snapshot_with_overlays(&mut self) -> BoardSnapshot {
        let phase = if self.final_state.is_some() {
            MarkerPhase::PostRoute
        } else {
            MarkerPhase::Parse
        };
        let mut snapshot = board_snapshot(&self.board);
        // Airlines: the `&mut` face over BOTH session fields (disjoint
        // borrows — manager shared, board mutable).
        snapshot.overlays.airlines = airline_segments(&self.manager, &mut self.board)
            .into_iter()
            .map(|segment| AirLinePrimitive {
                from: PointPrimitive {
                    x: segment.from.0,
                    y: segment.from.1,
                },
                to: PointPrimitive {
                    x: segment.to.0,
                    y: segment.to.1,
                },
                net: segment.net,
            })
            .collect();
        // Violation markers: the depth walk at the boundary.
        let (_total, rows) = all_clearance_violation_depths(&mut self.manager, &mut self.board);
        snapshot.overlays.violation_markers =
            crate::snapshot::violation_markers_from_rows(&rows, &self.board, phase);
        // Congestion: engaged-only, recomputed projection.
        if self.congestion_engaged {
            let map = CongestionMap::build(&mut self.board);
            snapshot.overlays.congestion = project_heatmap(&map);
        }
        // Tuning: constraint-declared only.
        if self.board.rules().has_length_constraints() {
            let max_net = self.board.rules().nets.max_net_number();
            let mut infos = Vec::new();
            for net in 1..=max_net {
                let (min, max) = self.board.rules().net_class_length_bounds(net);
                if min > 0.0 || max > 0.0 {
                    infos.push(NetTuningInfo {
                        net,
                        min,
                        max,
                        actual: self.board.net_trace_length(net),
                    });
                }
            }
            snapshot.overlays.tuning = Some(infos);
        }
        snapshot
    }
}

/// The congestion heatmap projection ([`CongestionMap`] →
/// [`CongestionHeatmap`]): only cells with `overflow > 0` (the raw
/// occupancy read, no net excluded — the boundary projection, not a
/// routing-net read), signal-layer-ascending then row-major
/// `(iy, ix)` order (the map's own grid geometry, documented on the
/// wire type). The empty map yields `None` (nothing to draw).
fn project_heatmap(map: &CongestionMap) -> Option<CongestionHeatmap> {
    if map.is_empty() {
        return None;
    }
    let (nx, ny) = map.grid_dims();
    let layer_count = map.total_overflow().len();
    let mut cells = Vec::new();
    for signal_layer in 0..layer_count {
        for iy in 0..ny {
            for ix in 0..nx {
                let overflow = map.overflow(ix, iy, signal_layer, None);
                if overflow > 0 {
                    cells.push(CongestionCell {
                        ix: ix as u64,
                        iy: iy as u64,
                        signal_layer: signal_layer as u64,
                        overflow,
                        capacity: map.capacity(signal_layer),
                    })
                }
            }
        }
    }
    let (ox, oy) = map.grid_origin();
    Some(CongestionHeatmap {
        cell_size: map.cell_size(),
        origin: PointPrimitive { x: ox, y: oy },
        dims: (nx as u64, ny as u64),
        cells,
    })
}

/// Java's manifest float face, VALUE step (route.rs:470-476,
/// byte-verbatim): `%.2f` HALF_UP at 2 decimals, arithmetically.
fn java_two_decimal(value: f32) -> f64 {
    let x = f64::from(value);
    if !x.is_finite() {
        return x;
    }
    let rounded = (x.abs() * 100.0 + 0.5).floor() / 100.0;
    if x < 0.0 { -rounded } else { rounded }
}

/// The route-time sink wrapper: forwards EVERY row to the caller's
/// sink UNCHANGED (the tee-sink forwarding law, event_sink.rs
/// `:140-141`/`:164-177` precedent — `is_trace_enabled` mirrors the
/// inner sink exactly) and additionally keeps the last
/// `board_updated` counters row whose phase is `autoroute` — the
/// pass-count source of [`RouteSummary::passes`] (route.rs:1078's
/// face, which the CLI reads off its OWN sink; a `&mut dyn
/// DriverSink` exposes no counters, so the session observes them at
/// the same seam). The M9-T3 snapshot hook is forwarded too — a tee
/// handed to [`Session::route`] as the sink sits INSIDE this wrapper,
/// and without the forward its snapshots would never fire.
///
/// `pub` (not crate-private) ONLY so the forwarding-exhaustion guard
/// pin (`events_stream.rs`) can drive ALL
/// [`DRIVER_SINK_METHOD_COUNT`](epic_router::pipeline::event_sink::DRIVER_SINK_METHOD_COUNT)
/// methods through this wrapper against a capture inner sink — the
/// structural defense against a future trait method being silently
/// un-forwarded (the T2 quality-review forward note). Behavior is
/// unchanged; the CLI never touches this type.
pub struct PassTrackingSink<'a> {
    inner: &'a mut dyn DriverSink,
    last_autoroute_pass: i32,
}

impl<'a> PassTrackingSink<'a> {
    /// The wrapper face ([`Session::route`] uses it; the guard pin
    /// constructs directly).
    pub fn new(inner: &'a mut dyn DriverSink) -> Self {
        Self {
            inner,
            last_autoroute_pass: 0,
        }
    }

    /// The last `board_updated` counters row's pass count whose phase
    /// was `autoroute` (0 when none fired — see [`RouteSummary::passes`]).
    #[must_use]
    pub fn last_autoroute_pass(&self) -> i32 {
        self.last_autoroute_pass
    }
}

impl DriverSink for PassTrackingSink<'_> {
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
    fn is_trace_enabled(&self) -> bool {
        self.inner.is_trace_enabled()
    }
    fn task_state(&mut self, state: &str, pass: i32, hash: &str) {
        self.inner.task_state(state, pass, hash);
    }
    fn board_updated(&mut self, counters: &RouterCounters) {
        if counters.phase == "autoroute" {
            self.last_autoroute_pass = counters.pass_count;
        }
        self.inner.board_updated(counters);
    }
    fn board_snapshot(&mut self, board: &Board) {
        self.inner.board_snapshot(board);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epic_router::pipeline::batch::StopReason;

    /// The `IoError` arm: the port's reader never constructs it from
    /// bytes (epic-dsn reader.rs module docs), so the mapping is
    /// pinned by direct construction — the session face is a hard
    /// `LoadError::Io`, never a panic and never a silent continue.
    #[test]
    fn io_error_arm_is_a_hard_load_error() {
        let mut warnings = Vec::new();
        let classified = classify_read_result(DsnReadResult::IoError, &mut warnings);
        assert!(matches!(classified, Err(LoadError::Io(_))));
        assert!(warnings.is_empty());
    }

    /// The final-state mapping is the hoisted
    /// `epic_router::pipeline::batch::final_state_for` face (M10-T2):
    /// the external stop is the only CANCELLED; every internal stop
    /// and `Ok(true)` are COMPLETED.
    #[test]
    fn final_state_mapping_matches_the_cli_face() {
        assert_eq!(final_state_for(true, None), "COMPLETED");
        assert_eq!(
            final_state_for(false, Some(StopReason::UserStop)),
            "CANCELLED"
        );
        // Internal stops (the driver's own MaxPasses/stagnation faces).
        assert_eq!(final_state_for(false, None), "COMPLETED");
    }

    /// The two-decimal float face on the discriminating values
    /// (route.rs:470-476): ties round HALF_UP (-2.5e-2 style) and
    /// -0.0 renders as 0.0.
    #[test]
    fn java_two_decimal_faces() {
        assert_eq!(java_two_decimal(0.125_f32), 0.13);
        assert_eq!(java_two_decimal(0.135_f32), 0.14);
        assert_eq!(java_two_decimal(-0.0_f32), 0.0);
        assert_eq!(java_two_decimal(812.5_f32), 812.5);
    }

    /// The AM2-asked, T6-landed compile-time pin: `Session` is
    /// `Send + Sync` — the worker-thread face is now REAL (the
    /// desktop shell's worker thread owns and drives a `Session`),
    /// so the guarantee is pinned, not presumed. `Board` is pinned
    /// `Send + Sync` (M8-T7's `t7_board_is_send_and_sync`); this pin
    /// covers the WHOLE session (board + search tree + settings
    /// layers + the `Arc<AtomicBool>` stop flag).
    #[test]
    fn session_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Session>();
        // The worker ships the cancel flag to the GUI thread, so the
        // flag handle's own Send face is load-bearing too.
        fn assert_send<T: Send>() {}
        assert_send::<Arc<AtomicBool>>();
    }
}
