//! The `route` flow (M3-T13): DSN read -> board build + trace
//! normalization -> the M4-T10 pipeline (`pipeline/full.rs`, the
//! `RoutingPipeline.run()` equivalent) -> SES write + result manifest.
//!
//! Java mirror: `HeadlessBoardManager`'s load-route-save walk
//! (`DsnReader` -> `BasicBoard` -> the `RoutingPipeline` -> `SES`), with
//! the manifest carried by `RoutingResultManifest` (schema v1). Two
//! deliberate deviations, both SEAM-documented:
//!
//! 1. **The non-determinism family is OMITTED** from the emitted
//!    manifest (generated_at, all duration_seconds, cpu/memory rows,
//!    resource_usage, cpu_score). This is a DELIBERATE DIVERGENCE, not
//!    a Gson nulls-parity claim: Java EMITS those keys with real
//!    payloads (they are never null in this flow), the harness mirror
//!    reader ignores them, and T13 chooses not to emit them.
//!    `settings_snapshot`/`bounds` fall in the same bucket (Java emits
//!    both — non-null whenever the job ran). The per-phase faces are
//!    the T10 Java-faithful fill: snapshot rows (deterministic subset)
//!    with each phase's score flavor and `score_source`, phase-
//!    attributed `passes_completed`, and the top-level `optimizer_score`
//!    iff-gate (`RoutingResultManifest.java:200-202`).
//! 2. **The final-state mapping** follows the Java scheduler, not the
//!    driver's internal stops: `Ok(true)` -> COMPLETED; `Ok(false)` is
//!    CANCELLED only for an EXTERNAL stop (`StopReason::UserStop`) —
//!    every internal stop (MaxPasses, stagnation, restore exhaustion)
//!    still lands COMPLETED, because Java's job scheduler treats the
//!    batch loop's own stop as a normal end of work. A driver ERROR
//!    (`BatchLoopError`) -> TERMINATED. TIMED_OUT is unreachable in T13
//!    (no wall-clock budget behind `--deterministic-budgets=on`, the
//!    default).

use epic_board::board::Board;
use epic_board::tree_manager::SearchTreeManager;
use epic_drc::clearance::all_clearance_violation_depths;
use epic_dsn::reader::{DsnReadResult, read_board};
use epic_dsn::ses::writer::write_session;
use epic_dsn::ses_board::SesBoard;
use epic_router::pipeline::batch::{StopFace, final_state_for};
use epic_router::pipeline::board_statistics::{BoardStatistics, RouterSettingsScoring};
use epic_router::pipeline::event_sink::DriverSink;
use epic_router::pipeline::full::{PipelineOutcome, PipelinePhases};
use serde::Serialize;
use sha2::Digest;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use epic_engine::export::{filename_without_extension, project_routed_items};
use epic_engine::session::apply_copper_to_edge_clearance_override;
use epic_engine::settings::{
    DsnLayer, MergedSettings, ParsedRouteArgs, ResolvedRouteSettings,
    apply_board_specific_optimizations, apply_meander_activation, apply_pairs_activation,
    apply_tuning_activation, build_batch_settings, merge,
};

// ---------------------------------------------------------------------------
// the driver sink: stderr echo + the manifest's counters source
// ---------------------------------------------------------------------------

/// The CLI's driver sink: log rows echo to stderr (the headless host's
/// FRLogger backend). The sink carries NO state: the manifest's
/// per-stage pass counts read the pipeline OUTCOME
/// (`PipelineOutcome::autoroute_passes_completed`, the optimizer's
/// `OptimizerStageFaces.outcome`) — the old last-counters-per-phase map
/// mis-attributed after a multi-pass autoroute, because the
/// optimizer's per-item reroutes stamp `phase="autoroute"` counters
/// rows through the same shared pass tail.
#[derive(Debug, Default)]
pub struct CliDriverSink;

impl DriverSink for CliDriverSink {
    fn info(&mut self, message: &str) {
        eprintln!("Info: {message}");
    }
    fn warn(&mut self, message: &str) {
        eprintln!("Warning: {message}");
    }
}

// ---------------------------------------------------------------------------
// host-layer hardening (readiness-fix M2 / M5 / m2 / E2)
// ---------------------------------------------------------------------------

/// The readiness-fix M2 pre-flight: the output path is probed BEFORE
/// any design bytes are read, so a typo in `-do`'s parent directory
/// costs nothing (previously the only writability probe was the SES
/// write AFTER the whole pipeline). `create_dir_all` the parent, then
/// write+delete a uniquely-named probe file (pid-keyed) in it.
///
/// CALLED FROM `main`'s arg-validation position (right after
/// `parse_route_args`), so a failure is the usage class (exit 2) —
/// `run_route`'s `Err` maps to 1, and the charter's exit-code table
/// stays untouched. `parse_route_args` itself stays pure (it is shared
/// with the harness faces, which must not gain a directory-creating
/// side effect).
///
/// # Errors
///
/// An uncreatable parent or an unwritable directory is a hard `Err`
/// (the usage message names the `-do` path).
pub fn preflight_output_path(args: &ParsedRouteArgs) -> Result<(), String> {
    let Some(ses) = args.ses.as_ref() else {
        return Ok(());
    };
    let ses_path = std::path::Path::new(ses);
    let parent = match ses_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        // A bare filename (`-do out.ses`): the parent is the cwd.
        _ => std::path::Path::new("."),
    };
    if let Err(error) = std::fs::create_dir_all(parent) {
        return Err(format!(
            "output directory not usable: -do {ses} (parent {}): {error}",
            parent.display()
        ));
    }
    let probe = parent.join(format!(".epic-cli-write-probe-{}", std::process::id()));
    let probe_result = std::fs::write(&probe, b"");
    // The probe is removed whatever the outcome — a failure still
    // leaves no residue in the user's directory.
    let _ = std::fs::remove_file(&probe);
    probe_result.map_err(|error| format!("output path not writable: -do {ses}: {error}"))
}

// ---------------------------------------------------------------------------
// the SIGINT/SIGTERM graceful stop (readiness-fix M5)
// ---------------------------------------------------------------------------

/// The slot the signal handler reads: a raw pointer to the shared
/// [`AtomicBool`] (owned by the `Arc` `run_route` keeps alive across
/// the pipeline). Set immediately before the handlers install; reset
/// never (the process exits with the run).
static STOP_FLAG_SLOT: std::sync::atomic::AtomicPtr<AtomicBool> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// The signal handler (SIGINT and SIGTERM): atomic loads + ONE
/// AtomicBool store — async-signal-safe by construction, no
/// allocation, no locks (the charter's requirement).
#[allow(unsafe_code)] // raw-pointer deref IS the signal-handler idiom; the pointee outlives the handler (Arc held by run_route)
extern "C" fn handle_stop_signal(signal: i32) {
    let _ = signal;
    let flag = STOP_FLAG_SLOT.load(std::sync::atomic::Ordering::Relaxed);
    if !flag.is_null() {
        #[allow(unsafe_code)]
        unsafe {
            (*flag).store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// Null the signal slot (the R2-2 face, review finding 3): the slot
/// holds a raw pointer into the `Arc`'s flag; the `Arc` drops when
/// `run_route` returns, so the slot must go back to null the moment
/// the pipeline is no longer running — a signal in the post-run window
/// hits the handler's null check, never a dangling pointee.
fn clear_stop_signal_slot() {
    STOP_FLAG_SLOT.store(std::ptr::null_mut(), std::sync::atomic::Ordering::Relaxed);
}

/// Install the SIGINT/SIGTERM handlers against `flag`. Raw
/// `libc::sigaction` (no new dependency class: `libc` was already in
/// the tree transitively). Errors are the caller's WARN-and-degrade
/// face (flagless pipeline = the pre-change behavior).
#[allow(unsafe_code)] // the sigaction FFI IS the unsafe surface; zeroed struct + two calls, no aliasing
fn install_stop_signal_handlers(flag: &Arc<AtomicBool>) -> Result<(), String> {
    STOP_FLAG_SLOT.store(
        Arc::as_ptr(flag) as *mut AtomicBool,
        std::sync::atomic::Ordering::Relaxed,
    );
    // No signals blocked beyond the delivered one (the zeroed mask);
    // no SA_RESTART semantics requested — a pending pipeline poll sees
    // the flag on its next check.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = handle_stop_signal as *const () as usize;
    action.sa_flags = 0;
    let sigint = unsafe { libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut()) };
    let sigterm = unsafe { libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut()) };
    if sigint != 0 || sigterm != 0 {
        return Err("sigaction failed".to_string());
    }
    Ok(())
}

/// The M5 wiring seam: the `StopFace` handed to the pipeline,
/// carrying the CLI's shared stop flag as an EXTERNAL-ONLY face
/// (`StopFace::from_external_flag` — the engine never writes the
/// flag, so an internal stop cannot leak into the host's cancel flag
/// and abort the optimizer stage on a signal-free run; the drift that
/// proved the need is fix-round evidence 05c). Pinned by
/// `cli_stop_face_carries_the_flag`.
fn cli_stop_face(flag: &Arc<AtomicBool>) -> StopFace {
    StopFace::from_external_flag(Some(Arc::clone(flag)))
}

/// The readiness-fix m2 face: the ONE glanceable stdout row (stdout was
/// previously unused). Values are the SAME stats the manifest carries
/// (incomplete_count, the full-violation clearance total, the
/// two-decimal router score, the final state) — exit codes untouched.
fn result_summary_row(incomplete: i64, violations: i64, score: f64, final_state: &str) -> String {
    format!(
        "result: incomplete={incomplete} violations={violations} score={score:.2} final={final_state}"
    )
}

/// The readiness-fix E2 face: the ONE load-phase Info row (the parse ->
/// board build -> normalize -> DRC-seed walk was previously minutes of
/// silence on a large board). stderr-only; never the manifest.
fn board_loaded_row(items: usize, nets: usize, pre_existing: i64, seconds: f64) -> String {
    format!(
        "board loaded: {items} items, {nets} nets, {pre_existing} pre-existing violations, {seconds:.1}s"
    )
}

// ---------------------------------------------------------------------------
// the manifest (schema v1 — harness/src/manifest.rs mirror names)
// ---------------------------------------------------------------------------

/// The per-phase slice (Java `RoutingResultManifest.PhaseDetail`). The
/// duration/cpu/memory rows are deliberately ABSENT from the type (the
/// non-determinism omission — module docs); the deterministic rows are
/// the before/after snapshots (Java `PhaseSnapshot`'s deterministic
/// subset) + the pass count.
#[derive(Serialize, Default)]
pub struct ManifestPhaseDetail {
    /// Java `PhaseDetail.before` (T10; `None` = the phase stayed empty
    /// under Java's capture gates).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<ManifestPhaseSnapshot>,
    /// Java `PhaseDetail.after`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<ManifestPhaseSnapshot>,
    /// The phase-attributed pass count: the autorouter row backfills
    /// from the AUTOROUTE-phase counters only (the fanout stage's
    /// counters must not leak in — the T7/T9 attribution pins); the
    /// optimizer row comes from the stage outcome (Java's explicit `0`
    /// bypass face included). Java's fanout row NEVER sets it
    /// (AutorouteBatchLoop.java:231-246).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passes_completed: Option<i64>,
}

/// One phase-boundary snapshot (Java `RoutingResultManifest.
/// PhaseSnapshot`'s deterministic subset: the boundary board-statistics
/// subset, the two scores, the phase's own `score`, and the
/// `score_source` tag). Java also emits `duration_seconds`/`cpu` at the
/// DETAIL level — omitted family.
#[derive(Serialize)]
pub struct ManifestPhaseSnapshot {
    /// Java `PhaseSnapshot.boardStatistics` — the deterministic subset
    /// (connections, clearance violations, via count, trace length).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board_statistics: Option<ManifestPhaseStats>,
    /// Java `PhaseSnapshot.score` — the phase's own flavor (router for
    /// the autorouter row, optimizer for the optimizer row, NONE for
    /// fanout — Java never sets any score row there).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<JavaDecimal>,
    /// Java `PhaseSnapshot.routerScore` (`fromBoardStatistics`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub router_score: Option<JavaDecimal>,
    /// Java `PhaseSnapshot.optimizerScore` (`fromBoardStatistics`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimizer_score: Option<JavaDecimal>,
    /// Java `PhaseSnapshot.scoreSource` — `"current"` on scored
    /// boundaries, `"not_applicable"` on fanout's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_source: Option<&'static str>,
}

/// The phase-boundary board-statistics subset (Java `BoardStatistics`'s
/// deterministic rows the dispatch names: score/vias/length — the
/// connections + clearance-violation faces ride too). Deliberately a
/// SEPARATE struct from the top-level `ManifestBoardStats`: extending
/// the top-level face would rotate the disabled-face control for a
/// NON-per-phase reason, outside T10's sanctioned rotation envelope.
#[derive(Serialize)]
pub struct ManifestPhaseStats {
    /// `BoardStatistics.connections`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connections: Option<ManifestConnections>,
    /// `BoardStatistics.clearanceViolations` (the load-seed
    /// subtraction face, same as the top-level walk).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clearance_violations: Option<ManifestClearanceViolations>,
    /// `BoardStatistics.vias.total_count`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vias: Option<ManifestViasTotal>,
    /// `BoardStatistics.traces.total_length_mm`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traces: Option<ManifestTracesLength>,
}

/// `vias { total_count }`.
#[derive(Serialize)]
pub struct ManifestViasTotal {
    pub total_count: i64,
}

/// `traces { total_length_mm }` — the jar's `%.2f` adapter literal
/// (`73.87`, `0.00`; see [`JavaDecimal`]).
#[derive(Serialize)]
pub struct ManifestTracesLength {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_length_mm: Option<JavaDecimal>,
}

/// The three phase slots (Java always emits the object; empty slots
/// serialize as `{}`).
#[derive(Serialize, Default)]
pub struct ManifestPhases {
    /// The fanout stage — populated when the stage gate captured the
    /// boundary pair (Java `AutorouteBatchLoop.java:93-98/231-247`),
    /// empty `{}` when fanout is disabled.
    pub fanout: ManifestPhaseDetail,
    /// The batch autorouter — populated when the batch loop ran with
    /// the router enabled.
    pub autorouter: ManifestPhaseDetail,
    /// The optimizer — populated whenever the stage gate opened
    /// (Java's bypass fill included: `passes_completed: 0` + identical
    /// before/after).
    pub optimizer: ManifestPhaseDetail,
}

/// The fixture identification: the input file name + the SHA-256 of the
/// INPUT DSN bytes (not the output — the manifest identifies what was
/// routed).
#[derive(Serialize)]
pub struct ManifestFixture<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<&'a str>,
}

/// The post-route board statistics subset the mirror reads.
#[derive(Serialize)]
pub struct ManifestBoardStats {
    /// The connection counts (`None` mirrors the skipped face).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connections: Option<ManifestConnections>,
    /// The clearance-violation counts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clearance_violations: Option<ManifestClearanceViolations>,
}

/// `connections { incomplete_count, maximum_count }`.
#[derive(Clone, Debug, Serialize)]
pub struct ManifestConnections {
    pub incomplete_count: i64,
    pub maximum_count: i64,
}

/// `clearance_violations { total_count, router_introduced_count }` —
/// the REAL violation walk (`DesignRulesChecker.getAllClearanceViolations`
/// face), not the outline-only count (CLAUDE.md DRC-completeness note).
/// `router_introduced_count` = total minus the count recorded at load
/// (`BasicBoard.preExistingClearanceViolationsCount`, saturated at 0).
#[derive(Clone, Debug, Serialize)]
pub struct ManifestClearanceViolations {
    pub total_count: i64,
    pub router_introduced_count: i64,
}

/// The M6-T6 ADVISORY pour-islands face (one row per filled pour) —
/// beyond-Java (upstream has no island face at all; design :14). The
/// detector (`epic_board::islands`) is pure analysis: it changes NO
/// route decision, NO SES byte, NO verdict face. Emitted ONLY for
/// boards that carry at least one filled pour (`skip_serializing_if`)
/// — pour-free boards keep byte-identical manifests, so the manifest
/// determinism canary does NOT rotate (the charter's anticipated
/// rotation is designed AWAY; verified at the bm08 determinism run).
#[derive(Clone, Debug, Serialize)]
pub struct ManifestPourIslands {
    /// The ConductionArea item id.
    pub item_id: u32,
    /// The pour's net name.
    pub net: String,
    /// The pour's 0-based layer.
    pub layer: i32,
    /// Connected metal regions (0 = fully covered pour).
    pub region_count: usize,
    /// Floating islands (regions unreachable from the net's pins/vias).
    pub island_count: usize,
    /// SHA-256 over the canonical per-region rows (deterministic,
    /// thread-count-invariant).
    pub digest: String,
}

/// The M6-T7 settings-ON manifest face: the global plan (map + guides
/// + planned order), condensed to deterministic scalars and digests.
#[derive(Serialize, Clone, Debug)]
pub struct ManifestGlobalPlan {
    /// The congestion map's square cell side (board units).
    pub cell: i64,
    /// The cell capacity per signal layer.
    pub capacity: Vec<i64>,
    /// The overflow vector per signal layer.
    pub overflow: Vec<u64>,
    /// SHA-256 over the canonical occupancy rows.
    pub map_digest: String,
    /// The guide count.
    pub guide_count: usize,
    /// The planned-order count.
    pub order_count: usize,
    /// SHA-256 over the canonical planned-order rows.
    pub order_digest: String,
}

/// The M7-T3 meander-need report row (T4's input contract) — one row
/// per constrained net whose routed length violates its resolved
/// net-class bounds, ordered by net number ascending. Machine-readable
/// by construction: T4's engine consumes the rows as its deficit list.
/// Lengths are raw f64 board-DBU (NOT the manifest's `%.2f` Java
/// adapter face — these rows are an epic-side contract, not a Java
/// mirror face, and T4 needs the full precision).
#[derive(Clone, Debug, Serialize)]
pub struct ManifestLengthNeed {
    /// The 1-based net number (the deterministic row key).
    pub net_number: i32,
    /// The net name (the DSN-declared name, case-preserved).
    pub net_name: String,
    /// The resolved `min` bound (0.0 = unconstrained on that side).
    pub min_length: f64,
    /// The resolved `max` bound (0.0 = unconstrained on that side).
    pub max_length: f64,
    /// The net's routed trace length (`Board::net_trace_length`).
    pub trace_length: f64,
    /// The `calcLengthViolation` port's verdict: positive = over-max
    /// excess, negative = under-min deficit (never 0 on a present row).
    pub violation: f64,
}

/// The M7-T6 advisory PAIR report row (one per RESOLVED declaration,
/// (leader, follower) ASC). Advisory by construction: pure post-route
/// board reads (the M6-T6 island-detector pattern) — no route change
/// rides the key; the route-change face (the coupling preference + the
/// match stage) is gated separately on the resolved declaration list.
#[derive(Clone, Debug, Serialize)]
pub struct ManifestPairRow {
    /// The declaration's 0-based index in the resolved list.
    pub pair_id: usize,
    pub leader_net_number: i32,
    pub leader_net_name: String,
    pub follower_net_number: i32,
    pub follower_net_name: String,
    /// The anchor: the LONGER member's net number at the match stage.
    pub anchor_net_number: i32,
    /// Final routed lengths (board DBU).
    pub leader_length: f64,
    pub follower_length: f64,
    /// `|leader − follower|` at the final board.
    pub delta: f64,
    /// `delta <= PAIR_DELTA_DBU` (equality allowed).
    pub matched: bool,
    /// The shared-corridor measure (parallel-within-window length).
    pub coupled_length: f64,
    /// Whether the match stage landed a wave on the shorter member.
    pub matched_by_meander: bool,
    pub added_length: f64,
    pub dent_count: i64,
}

/// The M7-T6 advisory UNRESOLVED-declaration row (a name resolving to
/// several subnet nets — a fromto split — or to none). Recorded, never
/// guessed.
#[derive(Clone, Debug, Serialize)]
pub struct ManifestPairUnresolved {
    pub name_a: String,
    pub name_b: String,
}

/// One F1 pin auto-assignment row (a performed re-netting; the
/// manifest face of `epic_engine::pin_assign::PinSwapRow`). Advisory
/// by construction — the mutation itself already rode the route.
#[derive(Clone, Debug, Serialize)]
pub struct ManifestPinSwap {
    /// The component REF the caller named.
    pub component: String,
    /// The pin's name in its package.
    pub pin: String,
    pub old_net_number: i32,
    pub old_net_name: String,
    pub new_net_number: i32,
    pub new_net_name: String,
}

/// One F2 current-driven widening row (the manifest face of
/// `epic_engine::current_width::CurrentWidthRow`). Advisory by
/// construction — the mutation itself already rode the route head.
#[derive(Clone, Debug, Serialize)]
pub struct ManifestWidthLayer {
    /// The 0-based layer index.
    pub layer: i32,
    /// The class's half width BEFORE the widening (board units).
    pub old_half_width: i32,
    /// The IPC-2221-floored half width AFTER (board units).
    pub new_half_width: i32,
}

/// One F2 current-driven widening (see [`ManifestWidthLayer`]).
#[derive(Clone, Debug, Serialize)]
pub struct ManifestCurrentWidth {
    pub net_number: i32,
    pub net_name: String,
    /// The requested current (A).
    pub amps: f64,
    /// The class the net LEFT (name).
    pub old_class: String,
    /// The synthetic widened class the net now rides (name).
    pub new_class: String,
    /// The WIDENED layers only (a layer already wide enough is
    /// omitted).
    pub layers: Vec<ManifestWidthLayer>,
}

/// One F3 synthesized ground pour (the manifest face of
/// `epic_engine::pour::PourRow`). Advisory by construction — the
/// insert itself already rode the route head.
#[derive(Clone, Debug, Serialize)]
pub struct ManifestSynthPour {
    pub net_number: i32,
    pub net_name: String,
    /// The poured layer (0-based board index).
    pub layer_no: i32,
    /// The layer's own name.
    pub layer_name: String,
    /// The net's on-board pin count at synthesis time.
    pub pin_count: usize,
}

/// The whole emitted manifest. Every optional member is
/// skip-serialized, so the non-determinism family never appears even
/// as a key.
#[derive(Serialize)]
pub struct RouteManifest<'a> {
    pub schema_version: u32,
    pub app_version: &'a str,
    pub git_sha: &'a str,
    pub fixture: ManifestFixture<'a>,
    pub phases: ManifestPhases,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board_statistics: Option<ManifestBoardStats>,
    /// The M6-T6 advisory pour-islands face (additive, version-tolerant:
    /// the harness manifest mirror ignores unknown fields). Absent for
    /// pour-free boards (the schema-evolution face).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pour_islands: Vec<ManifestPourIslands>,
    /// The M6-T7 settings-ON global-plan face. Absent at defaults
    /// (skip-if-None — the byte-identity face).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_plan: Option<ManifestGlobalPlan>,
    /// The M7-T3 meander-need report (T4's input contract). Absent on
    /// constraint-free boards AND when no constrained net violates its
    /// bounds (skip-if-empty — the zero-rotation face; the empty-Vec
    /// face is the contract's "nothing to meander" answer, so the key
    /// vanishes entirely).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub length_report: Vec<ManifestLengthNeed>,
    /// The M7-T6 advisory pair report (empty = absent from the
    /// rendered manifest — the zero-rotation face; no declared pairs,
    /// no key).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pair_report: Vec<ManifestPairRow>,
    /// The M7-T6 advisory unresolved-declaration rows (skip-if-empty).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pair_unresolved: Vec<ManifestPairUnresolved>,
    /// The F1 pin auto-assignment rows (empty = absent from the
    /// rendered manifest — the zero-rotation face; the canary
    /// manifests never rotate).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pin_assign_rows: Vec<ManifestPinSwap>,
    /// The F1 unresolved refs with reasons (skip-if-empty).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pin_assign_unresolved: Vec<String>,
    /// The F2 current-driven width rows (empty = absent — the
    /// zero-rotation face; canary manifests never rotate).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub current_width_rows: Vec<ManifestCurrentWidth>,
    /// The F2 unresolved requests with reasons (skip-if-empty).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub current_width_unresolved: Vec<String>,
    /// The F3 synthesized ground pours (empty = absent — the
    /// zero-rotation face; canary manifests never rotate).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pour_synthesis_rows: Vec<ManifestSynthPour>,
    /// The F3 unresolved pour requests with reasons (skip-if-empty).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pour_synthesis_unresolved: Vec<String>,
    /// The router score, 0-1000 (`None` when no scoring face — the
    /// Java iff-gate; unreachable in the T13 flow which always carries
    /// DefaultSettings' scoring box). The `%.2f` adapter literal face.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_score: Option<JavaDecimal>,
    /// The optimizer score — Java `:200-202`'s iff-gate: emitted iff
    /// the optimizer phase's before/after faces exist (the stage gate
    /// opened), i.e. whenever the optimizer stage RAN (the guard
    /// bypass included — Java's noroute face emits it on the bypass).
    /// The disabled control (`--optimizer.enabled=false`) keeps the key
    /// absent — the run-based iff-gate. The `%.2f` adapter literal face.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimizer_score: Option<JavaDecimal>,
    pub final_state: &'a str,
    pub exit_code: i32,
    pub output_written: bool,
}

/// The router-free telemetry the manifest renderer consumes — the flow
/// fills it, tests construct it directly.
#[derive(Debug, Default)]
pub struct RouteTelemetry {
    /// The final-state face (`final_state_for` / TERMINATED).
    pub final_state: String,
    /// The AUTOROUTE stage's completed pass count (the pipeline
    /// outcome's face — `PipelineOutcome::autoroute_passes_completed`;
    /// 0 = no pass ran, which renders as an absent row).
    pub autoroute_passes_completed: i32,
    /// Post-route connections (`None` = the stop face).
    pub connections: Option<ManifestConnections>,
    /// Post-route clearance violations (`None` = the skip face).
    pub clearance_violations: Option<ManifestClearanceViolations>,
    /// The router score (`None` = the no-scoring-face gate).
    pub normalized_score: Option<f64>,
    /// Whether a session file was actually written with content — the
    /// flow's real write outcome (Java `fromJob`'s `outputWritten`,
    /// `RoutingResultManifest.java:191`), not a constant.
    pub output_written: bool,
    /// The pipeline's per-phase boundary faces (M4-T10) — `None` faces
    /// leave the phase slot the empty `{}` projection (Java's capture
    /// gates: fanout disabled, router disabled/loop unrun, optimizer
    /// gated out).
    pub phases: PipelinePhases,
    /// The load-time violation seed — the per-phase clearance rows'
    /// `router_introduced` subtraction face (Java's
    /// `BoardStatistics.routerIntroducedCount` uses the same seed on
    /// every snapshot).
    pub pre_existing_violations: i32,
    /// The score box (the per-phase snapshot scores' input).
    pub scoring: Option<RouterSettingsScoring>,
    /// The FINAL board's optimizer score (Java `:201`'s value source —
    /// the manifest's final `BoardStatistics`); EMISSION is gated on
    /// the optimizer phase faces in the renderer (the `:200-202` iff).
    pub final_optimizer_score: Option<f64>,
    /// The M6-T6 advisory pour-islands face (empty = no filled pours;
    /// absent from the rendered manifest then).
    pub pour_islands: Vec<ManifestPourIslands>,
    /// The M6-T7 settings-ON global-plan face (None = OFF; absent from
    /// the rendered manifest then).
    pub global_plan: Option<ManifestGlobalPlan>,
    /// The M7-T3 meander-need report (empty = absent from the rendered
    /// manifest — the zero-rotation face).
    pub length_report: Vec<ManifestLengthNeed>,
    /// The M7-T6 advisory pair report (empty = absent).
    pub pair_report: Vec<ManifestPairRow>,
    /// The M7-T6 advisory unresolved-declaration rows (empty = absent).
    pub pair_unresolved: Vec<ManifestPairUnresolved>,
    /// The F1 pin auto-assignment rows (empty = absent).
    pub pin_assign_rows: Vec<ManifestPinSwap>,
    /// The F1 unresolved refs with reasons (empty = absent).
    pub pin_assign_unresolved: Vec<String>,
    /// The F2 current-driven width rows (empty = absent).
    pub current_width_rows: Vec<ManifestCurrentWidth>,
    /// The F2 unresolved requests with reasons (empty = absent).
    pub current_width_unresolved: Vec<String>,
    /// The F3 synthesized ground pours (empty = absent).
    pub pour_synthesis_rows: Vec<ManifestSynthPour>,
    /// The F3 unresolved pour requests with reasons (empty = absent).
    pub pour_synthesis_unresolved: Vec<String>,
}

/// The exit-code mapping (Java `MainResult`): 0 iff the state is
/// COMPLETED (or the unreachable TIMED_OUT) AND the output file exists
/// with content; everything else 1.
#[must_use]
pub fn exit_code_for(final_state: &str, output_written: bool) -> i32 {
    match (final_state, output_written) {
        ("COMPLETED", true) | ("TIMED_OUT", true) => 0,
        _ => 1,
    }
}

/// Java's manifest float face, VALUE step: `GsonProvider.GSON`
/// serializes every manifest float through
/// `TwoDecimalFloatAdapter`/`TwoDecimalDoubleAdapter`
/// (`GsonProvider.java:39-47`/`:57-65`) — `String.format(Locale.ROOT,
/// "%.2f", v)` through `BigDecimal`, i.e. HALF_UP at 2 decimals (NOT
/// `Float.toString`'s shortest round-trip form — the T10 first-cut
/// rationale was wrong, corrected by the spec review). The HALF_UP
/// rounding is done arithmetically: Rust's `format!` rounds
/// half-to-EVEN on exact ties, and ties DO occur (f32 eighths —
/// `v = odd/8` makes `v * 100` an exact half-integer) where Java rounds
/// away from zero; the dyadic product `|v| * 100` is exact in f64, so
/// `floor(|v| * 100 + 0.5)` is Java-exact. Sign: the port's stats can
/// produce `-0.0` where the jar's produce `+0.0` — the jar's `%.2f`
/// face of that value reads `0.00`, matched here.
fn java_two_decimal(value: f32) -> f64 {
    let x = f64::from(value);
    if !x.is_finite() {
        return x;
    }
    let rounded = (x.abs() * 100.0 + 0.5).floor() / 100.0;
    if x < 0.0 { -rounded } else { rounded }
}

/// Java's manifest float face, TEXT step: the jar writes the BigDecimal
/// LITERAL (`925.00` — trailing zeros preserved — `GsonProvider.java:42`
/// float adapter / `:62` double adapter, `out.value(BigDecimal)`);
/// serde_json numbers are shortest-form (`925.0`). The raw-literal
/// wrapper reproduces the text exactly.
/// The input is an ALREADY-ROUNDED value ([`java_two_decimal`]): the
/// nearest f64 of a 2-decimal value re-formats at 2 decimals to
/// itself (its error is orders of magnitude below the 0.005 tie
/// bound), so `format!("{:.2}")` is the literal face.
#[derive(Serialize)]
#[serde(transparent)]
pub struct JavaDecimal(Box<serde_json::value::RawValue>);

impl JavaDecimal {
    /// The `%.2f` literal of a rounded value — `None` on non-finite:
    /// Java's adapters check `!Float.isFinite(value)` and emit
    /// `out.nullValue()` (`GsonProvider.java:39`/`:59`), which Gson
    /// renders as the field OMITTED — the port's `None` feeding
    /// `skip_serializing_if` is exactly that face (no current stats
    /// path produces non-finite; the boundary is made total for the
    /// T15+ faces that will feed it).
    fn of(value: f64) -> Option<Self> {
        if !value.is_finite() {
            return None;
        }
        // A "%.2f"-formatted finite value is always a valid JSON number
        // literal, so the constructor cannot fail.
        Some(Self(
            serde_json::value::RawValue::from_string(format!("{value:.2}"))
                .expect("formatted number literal is always valid JSON"),
        ))
    }
}

/// The per-phase score flavor (Java's per-phase fill arms,
/// `RoutingResultManifest.java:148-159` + the three stage fills).
#[derive(Clone, Copy)]
enum PhaseScoreKind {
    /// `not_applicable`, ALL score rows stay absent — Java's fanout
    /// fill (`AutorouteBatchLoop.java:234-239`) constructs a bare
    /// `PhaseSnapshot` with ONLY `boardStatistics` +
    /// `scoreSource="not_applicable"`; Gson omits the null
    /// `routerScore`/`optimizerScore`/`score`.
    Fanout,
    /// `current`, `score = routerScore` (the autorouter fill).
    Autorouter,
    /// `current`, `score = optimizerScore` (the optimizer fill,
    /// `BatchOptimizer.java:299-305`/`:519-526`).
    Optimizer,
}

/// One phase-boundary snapshot face (Java `RoutingResultManifest.
/// PhaseSnapshot`): the deterministic stats subset + both scores + the
/// phase's own `score` flavor and `score_source` tag.
fn phase_snapshot(
    stats: &BoardStatistics,
    scoring: Option<&RouterSettingsScoring>,
    pre_existing: i32,
    kind: PhaseScoreKind,
) -> ManifestPhaseSnapshot {
    let router_score = stats.get_router_score(scoring);
    let optimizer_score = stats.get_optimizer_score(scoring);
    let (score, score_source, router_score_row, optimizer_score_row) = match kind {
        // Java's fanout fill sets NONE of the three score rows
        // (spec-review F1, oracle-verified: jar fanout-ran manifest
        // `fanout.before` keys = board_statistics + score_source only).
        PhaseScoreKind::Fanout => (None, "not_applicable", None, None),
        PhaseScoreKind::Autorouter => (
            JavaDecimal::of(java_two_decimal(router_score)),
            "current",
            JavaDecimal::of(java_two_decimal(router_score)),
            JavaDecimal::of(java_two_decimal(optimizer_score)),
        ),
        PhaseScoreKind::Optimizer => (
            JavaDecimal::of(java_two_decimal(optimizer_score)),
            "current",
            JavaDecimal::of(java_two_decimal(router_score)),
            JavaDecimal::of(java_two_decimal(optimizer_score)),
        ),
    };
    ManifestPhaseSnapshot {
        board_statistics: Some(ManifestPhaseStats {
            connections: stats
                .connections
                .maximum_count
                .map(|maximum_count| ManifestConnections {
                    incomplete_count: i64::from(stats.connections.incomplete_count.unwrap_or(0)),
                    maximum_count: i64::from(maximum_count),
                }),
            clearance_violations: Some(clearance_face(
                stats.clearance_violations.total_count,
                pre_existing,
            )),
            vias: Some(ManifestViasTotal {
                total_count: i64::from(stats.vias.total_count),
            }),
            traces: Some(ManifestTracesLength {
                total_length_mm: stats
                    .traces
                    .total_length_mm
                    .and_then(|v| JavaDecimal::of(java_two_decimal(v))),
            }),
        }),
        score,
        router_score: router_score_row,
        optimizer_score: optimizer_score_row,
        score_source: Some(score_source),
    }
}

/// Renders the manifest JSON (compact — Gson's default is pretty but
/// the mirror parses any whitespace; compact keeps the CLI output
/// stable). The iff-gates live in the Option members.
#[must_use]
#[allow(clippy::too_many_lines)] // the Java per-phase fill faces, inline
pub fn render_manifest(
    telemetry: &RouteTelemetry,
    app_version: &str,
    git_sha: &str,
    fixture: ManifestFixture<'_>,
) -> String {
    let mut phases = ManifestPhases::default();
    // The pass-count fill: the AUTOROUTE STAGE's own completed count
    // (the pipeline outcome — `PipelineOutcome::autoroute_passes_
    // completed`), only when a pass actually ran (> 0). Java's faces
    // are per-stage the same way: the fanout phase never sets
    // `passesCompleted` (AutorouteBatchLoop.java `:231-246` writes only
    // the before/after snapshots into `resultPhaseMetrics.fanout`), and
    // the OPTIMIZER row reads its own stage outcome (Java
    // `BatchOptimizer.java:523`). The OLD seam — the LAST
    // `phase="autoroute"` counters row — clobbers after a multi-pass
    // autoroute: the optimizer's per-item reroutes ride the same pass
    // tail stamping `phase="autoroute"` (pass_runner's shared
    // `finish_pass`), so t7_ripup at `--router.congestion_global=on`
    // reported 1 after a 2-pass autoroute + optimizer.
    phases.autorouter.passes_completed = (telemetry.autoroute_passes_completed > 0)
        .then(|| i64::from(telemetry.autoroute_passes_completed));
    // The T10 per-phase fill (Java `RoutingResultManifest.java:87-159`
    // + the three stage fill sites): the phases the pipeline captured,
    // each with its own score flavor.
    let snapshot = |stats: &BoardStatistics, kind: PhaseScoreKind| {
        phase_snapshot(
            stats,
            telemetry.scoring.as_ref(),
            telemetry.pre_existing_violations,
            kind,
        )
    };
    if let Some((before, after)) = &telemetry.phases.fanout {
        phases.fanout.before = Some(snapshot(before, PhaseScoreKind::Fanout));
        phases.fanout.after = Some(snapshot(after, PhaseScoreKind::Fanout));
    }
    if let Some((before, after)) = &telemetry.phases.autorouter {
        phases.autorouter.before = Some(snapshot(before, PhaseScoreKind::Autorouter));
        phases.autorouter.after = Some(snapshot(after, PhaseScoreKind::Autorouter));
    }
    if let Some(faces) = &telemetry.phases.optimizer {
        phases.optimizer.before = Some(snapshot(&faces.before, PhaseScoreKind::Optimizer));
        phases.optimizer.after = Some(snapshot(&faces.after, PhaseScoreKind::Optimizer));
        // Java `BatchOptimizer.java:523` (`passesCompleted =
        // currentPass`) / `:309-313` (the explicit bypass `0`).
        phases.optimizer.passes_completed = Some(i64::from(faces.outcome.passes_completed.max(0)));
    }
    let manifest = RouteManifest {
        schema_version: 1,
        app_version,
        git_sha,
        fixture,
        phases,
        // The board-statistics iff-gate: Java emits the object only when
        // the statistics walk ran (the T13 flow always does, but the
        // renderer stays honest to the mirror's optional face).
        board_statistics: match (&telemetry.connections, &telemetry.clearance_violations) {
            (None, None) => None,
            _ => Some(ManifestBoardStats {
                connections: telemetry.connections.clone(),
                clearance_violations: telemetry.clearance_violations.clone(),
            }),
        },
        pour_islands: telemetry.pour_islands.clone(),
        global_plan: telemetry.global_plan.clone(),
        length_report: telemetry.length_report.clone(),
        pair_report: telemetry.pair_report.clone(),
        pair_unresolved: telemetry.pair_unresolved.clone(),
        pin_assign_rows: telemetry.pin_assign_rows.clone(),
        pin_assign_unresolved: telemetry.pin_assign_unresolved.clone(),
        current_width_rows: telemetry.current_width_rows.clone(),
        current_width_unresolved: telemetry.current_width_unresolved.clone(),
        pour_synthesis_rows: telemetry.pour_synthesis_rows.clone(),
        pour_synthesis_unresolved: telemetry.pour_synthesis_unresolved.clone(),
        normalized_score: telemetry.normalized_score.and_then(JavaDecimal::of),
        // Java `:200-202` — the optimizer_score iff-gate: the key rides
        // only when the optimizer phase's before/after faces exist.
        optimizer_score: if telemetry.phases.optimizer.is_some() {
            telemetry.final_optimizer_score.and_then(JavaDecimal::of)
        } else {
            None
        },
        final_state: &telemetry.final_state,
        exit_code: exit_code_for(&telemetry.final_state, telemetry.output_written),
        output_written: telemetry.output_written,
    };
    // Serialization of plain data cannot fail.
    serde_json::to_string(&manifest).expect("manifest serialization cannot fail")
}

// ---------------------------------------------------------------------------
// the board -> SES projection
// ---------------------------------------------------------------------------

/// The SHA-256 hex of the input bytes (lowercase, the `sha2` default).
fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// The manifest's clearance-violations face (`BoardStatistics.java:
/// 390-393`): `routerIntroducedCount = max(0, total - preExisting)`,
/// with the pre-existing side seeded at load (the flow's step 2b,
/// `HeadlessBoardManager.java:793`) — a board that LOADS with
/// violations must not report them as router-introduced. Separated
/// from the flow so the subtraction face is directly pinnable.
fn clearance_face(total_count: Option<i32>, pre_existing: i32) -> ManifestClearanceViolations {
    let total = total_count.unwrap_or(0);
    ManifestClearanceViolations {
        total_count: i64::from(total),
        // Java's face is Math.max(0, ...) — the floor is ZERO, not the
        // i32::MIN that saturating_sub would give (caught by the PG3
        // pin on the (3, 5) world).
        router_introduced_count: 0.max(i64::from(total) - i64::from(pre_existing)),
    }
}

/// The M6-T7 plan -> manifest-face projection (deterministic scalars
/// + digests only).
fn manifest_global_plan(plan: &epic_router::global::plan::GlobalPlan) -> ManifestGlobalPlan {
    let map = plan.map();
    ManifestGlobalPlan {
        cell: map.cell_size(),
        capacity: (0..map.total_overflow().len())
            .map(|layer| map.capacity(layer))
            .collect(),
        overflow: map.total_overflow().to_vec(),
        map_digest: map.occupancy_digest(),
        guide_count: plan.guides().len(),
        order_count: plan.net_order().len(),
        order_digest: plan.order_digest(),
    }
}

/// The M7-T6 advisory pair-report assembly (the M6-T6
/// island-detector pattern: pure post-route board reads, NO route
/// change rides the rows). Per resolved pair: the final lengths, the
/// delta, the matched verdict against
/// `PAIR_DELTA_DBU`, the shared-corridor measure, and the match
/// stage's outcome (same (leader, follower) ASC order as the specs).
pub fn pair_report_rows(
    board: &mut epic_board::board::Board,
    specs: &[epic_router::pipeline::pairs::PairSpec],
    stage: &[epic_router::pipeline::pairs::PairStageOutcome],
) -> Vec<ManifestPairRow> {
    let mut rows = Vec::new();
    for (index, spec) in specs.iter().enumerate() {
        let leader_length = board.net_trace_length(spec.leader);
        let follower_length = board.net_trace_length(spec.follower);
        let delta = (leader_length - follower_length).abs();
        let rules = board.rules();
        let name = |net: i32| -> String {
            rules
                .nets
                .get(net)
                .map_or_else(|| format!("net#{net}"), |net_row| net_row.name.clone())
        };
        let stage_row = stage
            .iter()
            .find(|row| row.leader == spec.leader && row.follower == spec.follower);
        rows.push(ManifestPairRow {
            pair_id: index,
            leader_net_number: spec.leader,
            leader_net_name: name(spec.leader),
            follower_net_number: spec.follower,
            follower_net_name: name(spec.follower),
            // DISPLAY DEFAULT, not an anchored verdict: when the pair
            // stage did not run (no stage row — the face inert/OFF),
            // the row's anchor defaults to the LEADER for display;
            // `matched` above is computed from the LENGTHS, never from
            // this field.
            anchor_net_number: stage_row.map_or(spec.leader, |row| row.anchor_net),
            leader_length,
            follower_length,
            delta,
            matched: delta <= epic_router::pipeline::pairs::PAIR_DELTA_DBU,
            coupled_length: epic_router::pipeline::pairs::coupled_length(
                board,
                spec.leader,
                spec.follower,
                epic_router::pipeline::pairs::COUPLING_WINDOW_DBU,
            ),
            matched_by_meander: stage_row.is_some_and(|row| row.landed),
            added_length: stage_row.map_or(0.0, |row| row.added_length),
            dent_count: stage_row.map_or(0, |row| row.dent_count),
        });
    }
    rows
}

/// The M7-T3 meander-need report assembly (T4's input contract). Walks
/// the nets in NUMBER order (the deterministic row order), keeps the
/// constrained ones (`min > 0 || max > 0` — the T1 delivery gate), and
/// emits a row per net whose `calcLengthViolation` port verdict is
/// non-zero: net number/name, resolved bounds, routed length, and the
/// signed violation (positive = over-max excess, negative = under-min
/// deficit). Present only when the tuning regime is ON — the CALLER
/// gates on `board.tuning_active()` (an empty return on a
/// constraint-free board and the caller gate are the same face: no key
/// in the manifest at all, the zero-rotation pin). Deterministic by
/// construction: pure board reads, net-number order, no float search.
pub fn length_needs(manager: &SearchTreeManager, board: &mut Board) -> Vec<ManifestLengthNeed> {
    let mut rows = Vec::new();
    let incompletes = epic_drc::incompletes::all_incompletes(manager, board).1;
    let rules = board.rules();
    for (net_number, net) in rules.nets.iter() {
        let (min, max) = rules.net_class_length_bounds(net_number);
        if min <= 0.0 && max <= 0.0 {
            continue;
        }
        let trace_length = board.net_trace_length(net_number);
        let net_has_incompletes = incompletes
            .iter()
            .any(|row| row.net_no == net_number && row.incomplete_count > 0);
        let violation = rules.length_violation(net_number, trace_length, net_has_incompletes);
        if violation == 0.0 {
            continue;
        }
        rows.push(ManifestLengthNeed {
            net_number,
            net_name: net.name.clone(),
            min_length: min,
            max_length: max,
            trace_length,
            violation,
        });
    }
    rows
}

// ---------------------------------------------------------------------------
// F4: the pre-route interview
// ---------------------------------------------------------------------------

/// The question's interactive prompt (the CLI phrasing; the GUI
/// dialog asks the same thing its own way).
fn interview_prompt(question: &epic_engine::interview::InterviewQuestion) -> String {
    use epic_engine::interview::InterviewQuestion;
    match question {
        InterviewQuestion::GroundPour {
            net_name,
            pin_count,
        } => format!(
            "net {net_name} ({pin_count} pins) has no copper pour — \
             synthesize one on the last signal layer? [Y/n]"
        ),
        InterviewQuestion::DiffPair { net_a, net_b } => format!(
            "{net_a} and {net_b} look like a differential pair — \
             tune them as a pair? [y/N]"
        ),
        InterviewQuestion::CurrentWidth {
            net_name,
            pin_count,
        } => format!(
            "{net_name} ({pin_count} pins) looks like a power rail — \
             current in amps for its width (blank = keep class width)"
        ),
    }
}

/// Show or ask the interview (the `--interview` face). SHOW prints
/// every question with the `--router.` fragment that answers it and
/// routes unchanged; ON reads stdin line-by-line and patches
/// `merged` — before ANY consumer runs, so F1/F2/F3 and the F3 ask
/// see the answers as ordinary settings (an interviewed pour retires
/// the ask through the ask's own requested-nets filter). A
/// non-terminal stdin under ON degrades to SHOW: a piped run must
/// never hang.
fn run_interview(
    mode: epic_engine::settings::InterviewMode,
    questions: &[epic_engine::interview::InterviewQuestion],
    merged: &mut MergedSettings,
) {
    use epic_engine::interview::InterviewQuestion;
    use epic_engine::settings::InterviewMode;
    use std::io::IsTerminal as _;
    if questions.is_empty() {
        eprintln!("interview: the board and settings raise no questions");
        return;
    }
    let interactive = mode == InterviewMode::On && std::io::stdin().is_terminal();
    if mode == InterviewMode::On && !interactive {
        eprintln!("interview: stdin is not a terminal — showing the questions instead of asking");
    }
    for question in questions {
        let prompt = interview_prompt(question);
        if !interactive {
            eprintln!("interview: {prompt}");
            eprintln!("interview:   answer with {}", question.setting_hint());
            continue;
        }
        eprint!("interview: {prompt} ");
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
            return; // EOF mid-interview: stop asking, route what we have
        }
        let answer = line.trim();
        match question {
            InterviewQuestion::GroundPour { net_name, .. } => {
                let yes = answer.is_empty()
                    || answer.eq_ignore_ascii_case("y")
                    || answer.eq_ignore_ascii_case("yes");
                if yes {
                    merged
                        .pour_nets
                        .get_or_insert_with(Vec::new)
                        .push(net_name.clone());
                    eprintln!("interview: will pour {net_name}");
                }
            }
            InterviewQuestion::DiffPair { net_a, net_b } => {
                if answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes") {
                    merged
                        .tuning_pairs
                        .get_or_insert_with(Vec::new)
                        .push((net_a.clone(), net_b.clone()));
                    eprintln!("interview: will tune {net_a}:{net_b}");
                }
            }
            InterviewQuestion::CurrentWidth { net_name, .. } => {
                if answer.is_empty() {
                    continue;
                }
                match answer.parse::<f64>() {
                    Ok(amps) if amps.is_finite() && amps > 0.0 => {
                        merged.current_nets.get_or_insert_with(Vec::new).push(
                            epic_engine::current_width::CurrentNetRequest {
                                net: net_name.clone(),
                                amps,
                            },
                        );
                        eprintln!("interview: will width {net_name} for {amps}A");
                    }
                    _ => eprintln!("interview: skipped {net_name} (not a positive current)"),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// the flow
// ---------------------------------------------------------------------------

/// The whole `route` run: returns the PROCESS EXIT CODE. Parse warnings
/// and driver rows echo to stderr; the SES file and (optionally) the
/// manifest land on disk. The caller (`main`) guarantees
/// `args.dsn`/`args.ses` are present (`parse_route_args` enforces).
///
/// # Errors
///
/// Unreadable input, a parse error, or an unwritable session output is
/// a hard `Err` (exit 1 at the caller); an unwritable manifest is
/// LOG-ONLY (Java parity, `Freerouting.java:367-374`); a routing that
/// ends CANCELLED/TERMINATED writes no session and returns the mapped
/// exit code.
pub fn run_route(args: &ParsedRouteArgs) -> Result<i32, String> {
    // The E2 load-row clock: starts BEFORE the first input byte.
    let load_started = std::time::Instant::now();
    let dsn_path = std::path::Path::new(
        args.dsn
            .as_ref()
            .expect("parse_route_args enforces -de before run_route"),
    );
    let ses_path = std::path::Path::new(
        args.ses
            .as_ref()
            .expect("parse_route_args enforces -do before run_route"),
    );

    // The CLI-layer warnings (deprecations, WARN-and-continue rows).
    for warning in &args.layer.warnings {
        eprintln!("Warning: {warning}");
    }

    // 1. Read the DSN (bytes kept for the fixture hash).
    let bytes = std::fs::read(dsn_path)
        .map_err(|error| format!("cannot read design {}: {error}", dsn_path.display()))?;
    let mut ses = SesBoard::new();
    match read_board(&bytes, &mut ses) {
        DsnReadResult::Success { warnings } | DsnReadResult::OutlineMissing { warnings } => {
            // OutlineMissing: Java logs "no outline" and proceeds with
            // the default boundary — WARN-and-continue here too.
            for warning in warnings {
                eprintln!("Warning: {warning}");
            }
        }
        DsnReadResult::ParseError { location, detail } => {
            return Err(format!("parse error at {location}: {detail}"));
        }
        DsnReadResult::IoError => {
            return Err(format!("I/O error reading {}", dsn_path.display()));
        }
    }

    // Java deletes a pre-existing output file right after the input
    // load succeeds (Freerouting.java:122-127) — a failed RUN must not
    // leave the previous run's session in place masquerading as fresh
    // output. (On an input LOAD failure Java aborts BEFORE this point
    // and keeps the old file — the ordering is pinned by the contrast
    // face of the stale-output test.)
    if ses_path.exists() && std::fs::remove_file(ses_path).is_err() {
        eprintln!("Warning: Couldn't delete the file '{}'", ses_path.display());
    }

    // 1b. Resolve the settings: Default(0) -> Dsn(20) -> Cli(60), then
    //     the merger's trailing validate() (SettingsMerger.java:189).
    //     Hoisted ABOVE the board build because the copper-to-edge
    //     override consumes the merged settings at its parse-time call
    //     point (Java `:346`, mid-parse — the job's routerSettings
    //     exist there). Java applies no DSN or CLI source to
    //     `copperToEdgeClearanceUm` (only `DefaultSettings.java:155`
    //     seeds it), so the value the override sees here equals Java's
    //     at both of its call sites.
    let dsn_layer = DsnLayer::from_metadata(
        ses.metadata.autoroute_settings.as_ref(),
        usize::try_from(ses.metadata.layer_count).unwrap_or(0),
    );
    let mut merged = merge(&MergedSettings::default(), &dsn_layer, &args.layer);
    for warning in epic_engine::settings::validate(&mut merged) {
        eprintln!("Warning: {warning}");
    }

    // 2. Build the live board + the search tree, normalize the traces
    //    (bug-131 convention: the parse's corner collapse must not leak
    //    into the router's geometry).
    let mut board = Board::from_ses_board(&ses);
    let mut manager = SearchTreeManager::new();
    manager.reinsert_tree_items(&mut board);
    // The manager's parse-time copper-to-edge override (Java
    // `createBoard`, `:346` — fired by the DSN parser at
    // `Structure.java:1268` BEFORE the wiring scope's items exist and
    // BEFORE `Wiring.java:347` normalizeAllTraces): the outline is on
    // the board, nothing else has consumed its class yet. The post-load
    // second application (`:750`) is a skip-gate no-op / idempotent, so
    // this one call carries the whole state. The hole-clearance
    // override (`:347`/`:751`, `applyHoleClearanceOverride`) is NOT
    // ported — no Rust caller reads `rules.hole_clearance` beyond the
    // DSN's own parse value; porting it is its own task.
    apply_copper_to_edge_clearance_override(&merged, &mut manager, &mut board);
    epic_board::normalize_all::normalize_all_traces(&mut manager, &mut board);

    // 2a-b. P3 (#925a, upstream 14b28b6ff): the DRC clearance-tolerance
    //       override applies BEFORE the load-time seed — Java's order
    //       (settings load `:598-623` precedes the deferred DRC seed
    //       `:788-793`), so the pre-existing count is taken with the
    //       caller's tolerance (the 1.0 default drops sub-µm rows from
    //       the seed exactly as from the final walk). Re-applied after
    //       the interview below (an interviewed answer must land); the
    //       pre-sink warn rides the eprintln Warning channel (the
    //       parse-warnings idiom — the driver sink does not exist yet).
    if let Some(value) = merged.drc_clearance_tolerance_um
        && let Err(bad) = epic_engine::drc_tolerance::apply_clearance_tolerance(&mut board, value)
    {
        eprintln!(
            "Warning: ignoring router.drc.clearance_tolerance_um (must be finite and >= 0): {bad}"
        );
    }

    // 2b. The load-time violation seed (`HeadlessBoardManager.java:
    //     788-793`: `preExistingClearanceViolationsCount =
    //     getAllClearanceViolations().size()` before any routing —
    //     deferred to a background thread there, synchronous here).
    //     Java's deferred DRC reads POST-override state (the `:346`
    //     parse-time application long preceded it), so this seed runs
    //     after the override above — the manifest's pre-existing vs
    //     router-introduced split measures the promoted-outline board
    //     exactly as Java's does.
    let (pre_total, _) = all_clearance_violation_depths(&mut manager, &mut board);
    board.pre_existing_clearance_violations_count = i32::try_from(pre_total).unwrap_or(i32::MAX);

    // The driver sink moves UP here (the readiness-fix E2 emit point):
    // ONE stderr Info line after the load phase (parse -> board build
    // -> normalize -> the DRC seed) — the minutes of silence on a
    // large board were a UX gap, not a diagnostic. The duration is a
    // local Instant (stderr-only; never the manifest — the
    // non-determinism omission stands).
    let mut sink = CliDriverSink;
    sink.info(&board_loaded_row(
        board.item_count(),
        board.rules().nets.iter().count(),
        pre_total,
        load_started.elapsed().as_secs_f64(),
    ));

    // F4 (Rust-only): the pre-route interview — the board-derived
    // questions, shown (`--interview=show`) or asked on a terminal
    // (`--interview=on`) BEFORE any consumer runs. Answers patch
    // `merged` here, so F1/F2/F3 and the F3 ask see them as ordinary
    // settings (an interviewed pour retires the ask through the
    // ask's own requested-nets filter — no special case either
    // side). OFF at the default: byte-identical scripted runs.
    if args.interview != epic_engine::settings::InterviewMode::Off {
        let questions = epic_engine::interview::interview_questions(&board, &merged);
        run_interview(args.interview, &questions, &mut merged);
    }

    // M11-T2 (#931): the board bounding box grows to cover every
    // item at the route head — the SAME head placement as
    // `Session::route` (Java `HeadlessBoardManager.startRouting:835`,
    // before `reduceNetsOfRouteItems()`): edge-connector pins placed
    // outside the outline leave the parse box too small, and the
    // outline's outside keepout (built over the bbox) then cuts into
    // their pads.
    board.expand_bounding_box_to_include_all_items();

    // P3 (#925a) re-apply from the POST-INTERVIEW merged view (the
    // seed-pass apply at 2a-b above governs the pre-existing count;
    // this pass lands an interviewed answer and matches
    // `Session::route`'s route-head re-apply — the same value in the
    // common case, idempotent). An invalid value warns + keeps the
    // board default (Java HeadlessBoardManager parity), never fails
    // the run.
    if let Some(value) = merged.drc_clearance_tolerance_um
        && let Err(bad) = epic_engine::drc_tolerance::apply_clearance_tolerance(&mut board, value)
    {
        sink.warn(&format!(
            "ignoring router.drc.clearance_tolerance_um (must be finite and >= 0): {bad}"
        ));
    }

    // F1 (Rust-only): the pin auto-assignment face — the SAME head
    // placement as `Session::route` (after the merge, before every
    // geometry/pass face), so the CLI and session flows share one
    // apply fn and one ordering. Unresolved refs warn and never fail
    // the run; the report rides the manifest (skip-if-empty).
    let pin_assign_report = merged
        .assign_pins
        .as_ref()
        .map(|refs| epic_engine::pin_assign::apply_pin_assignments(&mut board, &mut manager, refs));
    if let Some(report) = &pin_assign_report {
        for reason in &report.unresolved {
            sink.warn(&format!("pin assignment: {reason}"));
        }
    }
    // F2 (Rust-only): the current-driven width face — directly after
    // pin assignment (the SAME head placement as `Session::route`),
    // so the widened net classes exist before the geometry pass reads
    // any width. Unresolved nets and applied-with-warning faces warn
    // and never fail the run; the report rides the manifest
    // (skip-if-empty).
    let current_width_report = merged.current_nets.as_ref().map(|requests| {
        epic_engine::current_width::apply_current_widths(
            &mut board,
            requests,
            merged.current_copper_oz.unwrap_or(1.0),
            merged.current_temp_rise_c.unwrap_or(10.0),
        )
    });
    if let Some(report) = &current_width_report {
        for reason in &report.unresolved {
            sink.warn(&format!("current width: {reason}"));
        }
        for warning in &report.warnings {
            sink.warn(&format!("current width: {warning}"));
        }
    }
    // F3 (Rust-only): the ground-pour synthesis face — directly after
    // the current-width face (the SAME head placement as
    // `Session::route`), so the synthesized pours exist before the
    // geometry pass and the pipeline's plane handling. Unresolved
    // requests warn and never fail the run; the report rides the
    // manifest (skip-if-empty).
    let pour_report = merged.pour_nets.as_ref().map(|nets| {
        let requests: Vec<epic_engine::pour::PourRequest> = nets
            .iter()
            .map(|net| epic_engine::pour::PourRequest {
                net: net.clone(),
                layer: merged.pour_layer.clone(),
            })
            .collect();
        epic_engine::pour::synthesize_pours(&mut board, &mut manager, &requests)
    });
    if let Some(report) = &pour_report {
        for reason in &report.unresolved {
            sink.warn(&format!("ground pour: {reason}"));
        }
    }
    // F3: THE ASK — Tyler's complaint verbatim ("it routed gnd - so it
    // doesnt ask if you want to do a gnd pour"): every ground-like net
    // still without a pour after synthesis surfaces as a NOTE before
    // the routing starts. Deliberately on STDERR, NOT through the
    // sink — the event stream is a golden-pinned face, and the ask is
    // a host-console concern (the F4 interview layer will make this a
    // dialog). Nets the caller already requested are not re-asked
    // (their refusal already warned through the sink).
    let requested_nets: Vec<String> = merged
        .pour_nets
        .clone()
        .unwrap_or_default()
        .iter()
        .map(|net| net.to_lowercase())
        .collect();
    for candidate in epic_engine::pour::pour_candidates(&board) {
        if requested_nets.contains(&candidate.net_name.to_lowercase()) {
            continue;
        }
        eprintln!(
            "note: net {} ({} pins) has no copper pour — routing as traces; pass --router.pour.nets={} to synthesize one",
            candidate.net_name, candidate.pin_count, candidate.net_name
        );
    }
    // #152 (upstream d9694ab82/d0d876e30, PR #889): the plane-routing
    // overrides — same head placement as `Session::route` (after the
    // pour faces, before the geometry pass). `plane.nets` promotes
    // resolved nets to plane routing (notes warn through the sink);
    // `plane.as_obstacle` flips every SIGNAL-layer pour's obstacle
    // flag (a change-request: absent = never called, default runs
    // byte-stable).
    if let Some(names) = merged.plane_nets.clone() {
        for note in epic_engine::plane_nets::apply_plane_nets(&mut board, &names) {
            sink.warn(&note);
        }
    }
    if let Some(value) = merged.plane_as_obstacle {
        epic_board::plane_obstacle::change_plane_as_obstacle(&mut board, &mut manager, value);
    }
    // 3. The unconditional geometry pass (bug-compat fact 1; Java
    //    `applyRouterSettingsForLoadedBoard` `:749`, which runs after
    //    the `:346` override — the pass reads only the board's
    //    bounds/layers and writes settings, so the two orders are
    //    state-equivalent, but this placement mirrors Java's).
    apply_board_specific_optimizations(&mut merged, &board);
    let resolved = ResolvedRouteSettings::resolve(&merged, args.deterministic_budgets);

    // 4. The pipeline (M4-T10, the `RoutingPipeline.run()` equivalent —
    //    Java `RoutingPipeline.java:86-134`): the routing stage (fanout
    //    + batch autorouter, the T12/T7 driver) then the optimization
    //    stage (the T9 optimizer), ONE pipeline shape — the T9 inline
    //    wiring moved, not forked. The stop-gate semantics (the T9
    //    MIN-2 collapsed-face doc) live on the pipeline module now
    //    (`pipeline/full.rs`); the stage-boundary captures ride the
    //    outcome for the manifest's per-phase faces.
    let mut batch = build_batch_settings(&resolved);
    // M7-T2: the tuning activation — input-driven (a net-class length
    // declaration anywhere on the board), with `router.tuning` as the
    // explicit override/kill-switch on top.
    apply_tuning_activation(&mut batch, &resolved, board.rules());
    // M7-T4: the MEANDER stage's tri-state — `None` rides the resolved
    // tuning activation (input-driven); `on` forces (a no-op on a
    // constraint-free board — no declaration, no deficit rows); `off`
    // kills the meander stage ALONE (the T3 honoring gate stays armed
    // off `tuning_active`).
    apply_meander_activation(&mut batch, &resolved);
    // M7-T6: the PAIR activation — the resolved declaration list (the
    // third activation input) resolved at the board seam; the
    // unresolved rows ride the advisory report. The engine predicate
    // returns the unresolved rows as RAW `(name_a, name_b)` tuples
    // (the M9-T1 move: the manifest row type stays in the host); the
    // mapping to the manifest's advisory rows happens HERE.
    let (pair_specs, unresolved_raw) = apply_pairs_activation(&mut batch, &resolved, &board);
    let unresolved_pairs: Vec<ManifestPairUnresolved> = unresolved_raw
        .into_iter()
        .map(|(name_a, name_b)| ManifestPairUnresolved { name_a, name_b })
        .collect();
    // M7-T3: the honoring faces read the RESOLVED activation off the
    // board (never the CLI tri-state) — one write here, next to the
    // tuning activation — before any routing/optimization pass runs.
    board.set_tuning_active(batch.tuning_active);
    // M5-T7: the executor thread count. The DEFAULT is 1 — the golden
    // sequential path — even though the merged surface resolves Java's
    // `max(1, cores-1)` parity default in validate(): the executor
    // engages only on an EXPLICIT `router.max_threads`/`-mt` (the
    // validated value is then already clamped/warned by the Java-parity
    // arms). Byte-identical output at every N by construction.
    batch.max_threads = if args.layer.max_threads.is_some() {
        // An explicit CLI face survives merge+validate as `Some` (0
        // survives; only `< 0` / `> cores` are rewritten by validate).
        merged
            .max_threads
            .expect("explicit CLI max_threads is Some post-validate")
    } else {
        1
    };
    // M8-T7: the optimizer candidate loop's partition count. The
    // DEFAULT is 1 — the mandated 1-thread parity face, byte-for-byte
    // the pre-T7 walk — engaged only on an EXPLICIT
    // `optimizer.threads` (the tri-state; `<= 1` clamps to the
    // sequential face). The partitioned path is thread-count-invariant
    // by construction (the reduction-order argument at
    // optimizer.rs's executor doc).
    batch.optimizer_threads = if args.layer.optimizer_threads.is_some() {
        merged.optimizer_threads.unwrap_or(1).max(1).unsigned_abs() as usize
    } else {
        1
    };
    // The readiness-fix M5 face: the stop flag + handlers install AS
    // LATE AS POSSIBLE (right before the pipeline; the --version and
    // --help arms never reach this code). No signal => the flag never
    // raises => the flagged StopFace is field-behaviorally the
    // StopFace::default() it replaces (byte-identical requirement (a)).
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_face = match install_stop_signal_handlers(&stop_flag) {
        Ok(()) => cli_stop_face(&stop_flag),
        Err(message) => {
            eprintln!("Warning: {message}; a SIGINT/SIGTERM will kill the run outright");
            StopFace::default()
        }
    };
    let outcome = epic_router::pipeline::full::run(
        &mut manager,
        &mut board,
        batch,
        resolved.optimizer.clone(),
        resolved.run_optimizer,
        stop_face,
        &mut sink,
    );
    // The R2-2 face: the pipeline is done — null the slot BEFORE the
    // flag's Arc can ever drop (the rest of the run is synchronous
    // post-processing on the way out of run_route).
    clear_stop_signal_slot();
    let run_result = outcome.routing;
    let stop_reason = outcome.stop_reason;

    let final_state: String = match &run_result {
        Err(_) => "TERMINATED".to_string(),
        Ok(true) => "COMPLETED".to_string(),
        Ok(false) => final_state_for(false, stop_reason).to_string(),
    };
    if final_state == "CANCELLED" {
        // The readiness-fix M5(b) face: the SIGINT/SIGTERM path — the
        // external stop maps through the EXISTING final-state mapping
        // (final_state_for: UserStop -> CANCELLED, pinned), no SES is
        // written (the COMPLETED/TIMED_OUT output gate), the manifest
        // write below is still reached, and the exit code is the
        // EXISTING CANCELLED mapping (exit 1, pinned at
        // exit_code_mapping).
        eprintln!("Info: run cancelled by signal; no session file written");
    }

    // 5. Post-route statistics (the manifest's board face).
    let stats = BoardStatistics::new(&mut manager, &mut board);
    // Both top-level scores render through the manifest's %.2f adapter
    // face (GsonProvider's TwoDecimal adapters — F2).
    let router_score = java_two_decimal(stats.get_router_score(Some(&resolved.scoring)));
    let normalized_score = Some(router_score);
    // Java `:201` — the value source of the top-level optimizer score
    // (the manifest's final boardStatistics); the RENDERER applies the
    // `:200-202` iff-gate on the optimizer phase faces.
    let final_optimizer_score =
        java_two_decimal(stats.get_optimizer_score(Some(&resolved.scoring)));
    let connections = stats.connections.maximum_count.map(|maximum_count| {
        // Both counts ride together (the same walk fills them; the
        // incomplete face mirrors Java's null when the walk skipped).
        ManifestConnections {
            incomplete_count: i64::from(stats.connections.incomplete_count.unwrap_or(0)),
            maximum_count: i64::from(maximum_count),
        }
    });
    let clearance_violations = Some(clearance_face(
        stats.clearance_violations.total_count,
        board.pre_existing_clearance_violations_count,
    ));
    // The M6-T6 advisory detector face: pure post-route analysis (no
    // board mutation, no route effect at any setting). Emitted into the
    // manifest ONLY when the board carries a filled pour.
    let pour_islands: Vec<ManifestPourIslands> = epic_board::islands::detect_pour_islands(&board)
        .into_iter()
        .map(|pour| ManifestPourIslands {
            item_id: pour.pour_item_id,
            net: pour.net,
            layer: pour.layer,
            region_count: pour.region_count,
            island_count: pour.island_count,
            digest: pour.digest,
        })
        .collect();
    // The M7-T3 meander-need report (T4's input contract): present only
    // when the tuning regime resolved ON (the input-driven activation —
    // a constraint-free board never emits the key, the zero-rotation
    // face), and only rows for constrained nets whose routed length
    // violates their bounds.
    let length_report: Vec<ManifestLengthNeed> = if board.tuning_active() {
        length_needs(&manager, &mut board)
    } else {
        Vec::new()
    };
    // The M7-T6 advisory pair report (empty = no declared pairs, no
    // key — the zero-rotation face). The match stage's outcomes ride
    // the pipeline outcome; the rows are computed from the FINAL
    // board.
    let pair_report: Vec<ManifestPairRow> = if pair_specs.is_empty() {
        Vec::new()
    } else {
        pair_report_rows(&mut board, &pair_specs, &outcome.pair_stage)
    };
    // The pipeline's per-phase faces (the render builds the snapshot
    // rows from the raw boundary pairs).
    let PipelineOutcome {
        boundaries,
        optimizer,
        gloss_report,
        flow_report,
        via_place_report,
        teardrops_report,
        autoroute_passes_completed,
        ..
    } = outcome;
    let phases = PipelinePhases {
        fanout: boundaries.fanout_before.zip(boundaries.fanout_after),
        autorouter: boundaries
            .autorouter_before
            .zip(boundaries.autorouter_after),
        optimizer,
    };

    // The SES design name is the job name WITHOUT extension
    // (`RoutingJob.java:519` -> `SesWriter.write`: the writer's
    // `.replace(".dsn", ".ses")` is a no-op on it, so no `.dsn` lands
    // in the session scope). The MANIFEST keeps the raw input name —
    // it identifies the input file.
    let input_name = dsn_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("board.dsn");
    // 6. Project the routed items and write the session file — GATED on
    //    the job state exactly like Java's writeCliOutputIfAvailable
    //    (Freerouting.java:265-267): only COMPLETED/TIMED_OUT produce
    //    output; every failure face leaves NO session file.
    let output_written = if matches!(final_state.as_str(), "COMPLETED" | "TIMED_OUT") {
        project_routed_items(&board, &mut ses);
        let ses_text = write_session(&ses, filename_without_extension(input_name));
        std::fs::write(ses_path, ses_text)
            .map_err(|error| format!("cannot write session {}: {error}", ses_path.display()))?;
        std::fs::metadata(ses_path)
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
    } else {
        false
    };
    let exit_code = exit_code_for(&final_state, output_written);

    // The readiness-fix m2 face: the ONE glanceable stdout row, from
    // the SAME stats the manifest carries (the string is built here —
    // before `final_state`/`normalized_score` move into the telemetry —
    // and printed at the very end of the run; exit codes untouched).
    let summary_row = result_summary_row(
        i64::from(stats.connections.incomplete_count.unwrap_or(0)),
        i64::from(stats.clearance_violations.total_count.unwrap_or(0)),
        router_score,
        &final_state,
    );

    // 7. The manifest. The F1 rows/unresolved pair and the F2
    // rows/unresolved pair, each extracted in ONE consumption of the
    // Option (skip-if-empty at render time).
    let (pin_assign_rows, pin_assign_unresolved) = pin_assign_report
        .map(|report| {
            let rows = report
                .rows
                .iter()
                .map(|row| ManifestPinSwap {
                    component: row.component.clone(),
                    pin: row.pin.clone(),
                    old_net_number: row.old_net_number,
                    old_net_name: row.old_net_name.clone(),
                    new_net_number: row.new_net_number,
                    new_net_name: row.new_net_name.clone(),
                })
                .collect();
            (rows, report.unresolved)
        })
        .unwrap_or_default();
    let (current_width_rows, current_width_unresolved) = current_width_report
        .map(|report| {
            let rows = report
                .rows
                .iter()
                .map(|row| ManifestCurrentWidth {
                    net_number: row.net_number,
                    net_name: row.net_name.clone(),
                    amps: row.amps,
                    old_class: row.old_class.clone(),
                    new_class: row.new_class.clone(),
                    layers: row
                        .layers
                        .iter()
                        .map(
                            |&(layer, old_half_width, new_half_width)| ManifestWidthLayer {
                                layer,
                                old_half_width,
                                new_half_width,
                            },
                        )
                        .collect(),
                })
                .collect();
            (rows, report.unresolved)
        })
        .unwrap_or_default();
    let (pour_synthesis_rows, pour_synthesis_unresolved) = pour_report
        .map(|report| {
            let rows = report
                .rows
                .iter()
                .map(|row| ManifestSynthPour {
                    net_number: row.net_number,
                    net_name: row.net_name.clone(),
                    layer_no: row.layer_no,
                    layer_name: row.layer_name.clone(),
                    pin_count: row.pin_count,
                })
                .collect();
            (rows, report.unresolved)
        })
        .unwrap_or_default();
    let telemetry = RouteTelemetry {
        final_state,
        autoroute_passes_completed,
        connections,
        clearance_violations,
        normalized_score,
        output_written,
        phases,
        pre_existing_violations: board.pre_existing_clearance_violations_count,
        scoring: Some(resolved.scoring.clone()),
        final_optimizer_score: Some(final_optimizer_score),
        pour_islands,
        global_plan: outcome.global_plan.as_ref().map(manifest_global_plan),
        length_report,
        pair_report,
        pair_unresolved: unresolved_pairs,
        pin_assign_rows,
        pin_assign_unresolved,
        current_width_rows,
        current_width_unresolved,
        pour_synthesis_rows,
        pour_synthesis_unresolved,
    };
    let manifest = render_manifest(
        &telemetry,
        env!("CARGO_PKG_VERSION"),
        &std::env::var("FREEROUTING_GIT_SHA").unwrap_or_else(|_| "unknown".to_string()),
        ManifestFixture {
            filename: Some(input_name),
            sha256: Some(&sha256_hex(&bytes)),
        },
    );
    if let Some(result_json) = &args.result_json {
        // Java computes the exit code BEFORE the manifest write and
        // treats a manifest IOException as log-only (Freerouting.java:
        // 166-168, 367-374) — an unwritable --result-json path must not
        // flip the SES-derived exit code.
        if let Err(error) = std::fs::write(result_json, manifest) {
            eprintln!("Error: cannot write manifest {result_json}: {error}");
        }
    }
    if let Some(aesthetics_path) = &args.dump_aesthetics {
        // The M8-T1 sidecar door: the four aesthetics metrics over the
        // FINAL board, written at manifest-write time. ROTATION TRAP:
        // the metrics land in THIS sidecar file, NEVER into the
        // manifest — the manifest bytes are pinned by the version-blind
        // manifest canary (the raw-bytes cf607714… retired at M10-T5),
        // and a new always-on manifest key would
        // rotate every manifest golden on every board. The same
        // log-only face as the manifest write: an unwritable sidecar
        // path never flips the SES-derived exit code. Byte-stable for
        // a fixed run (fixed key order, 3-decimal rounding at the
        // render edge only).
        let metrics = epic_board::aesthetics::aesthetics_metrics(&board, board.rules());
        // The trailing newline: the sidecar becomes byte-compatible
        // with the goldens' `println!` face (nothing pins sidecar
        // bytes; the smoke's byte-identity was run-to-run, which a
        // constant suffix preserves).
        // M8-T3: the gloss `bus_groups` block rides THIS sidecar only
        // (serde-skip-when-empty — default runs and the parse-only
        // reference door never carry it: the 24 committed reference
        // goldens and every default-run sidecar stay byte-identical).
        // NEVER the manifest (the version-blind manifest canary pins
        // manifest bytes; the raw cf607714… retired at M10-T5).
        let mut sidecar = epic_board::aesthetics::render_json_value(&metrics);
        if !gloss_report.is_empty() {
            let groups: Vec<serde_json::Value> = gloss_report
                .groups
                .iter()
                .map(|group| {
                    serde_json::json!({
                        "horizontal": group.horizontal,
                        "layer": group.layer,
                        "member_names": group.member_names,
                        "members": group.members,
                        "moves_landed": group.moves_landed,
                        "moves_rejected": group.moves_rejected,
                        "rows": group.rows.iter().map(|row| serde_json::json!({
                            "from_pos": row.from_pos,
                            "landed": row.landed,
                            "net": row.net,
                            "net_name": row.net_name,
                            "target_pos": row.target_pos,
                        })).collect::<Vec<serde_json::Value>>(),
                    })
                })
                .collect();
            sidecar["bus_groups"] = serde_json::Value::Array(groups);
        }
        if gloss_report.gated {
            // The quality-r1 F6 sibling key: the stage short-circuited
            // on incompletes (the bus_groups array is empty by
            // construction here) — the sidecar can tell the honest
            // hold from a genuinely group-free board. Default runs
            // never reach the stage; nothing pinned changes.
            sidecar["bus_groups_gated"] = serde_json::Value::Bool(true);
        }
        // M8-T4: the gloss `gloss_flow` block — SAME sidecar, same
        // skip-when-empty law (never the manifest: the version-blind
        // manifest-canary digest). The
        // gated hold surfaces as the DISTINCT `gloss_flow_gated`
        // sibling key from day one (the T3 fix-round lesson — no
        // gated-face conflation).
        if !flow_report.is_empty() {
            // Row locator contract (quality-r1 NIT; documented on
            // `FlowCandidateRow`): corner_x/corner_y = the stub's
            // FIRST corner — stable under the landing (which removes
            // a corner after it).
            let rows: Vec<serde_json::Value> = flow_report
                .rows
                .iter()
                .map(|row| {
                    serde_json::json!({
                        "corner_x": row.corner_x,
                        "corner_y": row.corner_y,
                        "kind": row.kind.label(),
                        "landed": row.landed,
                        "net": row.net,
                        "net_name": row.net_name,
                    })
                })
                .collect();
            sidecar["gloss_flow"] = serde_json::Value::Array(rows);
        }
        if flow_report.gated {
            sidecar["gloss_flow_gated"] = serde_json::Value::Bool(true);
        }
        // M8-T5: the gloss `gloss_via_place` block — SAME sidecar, same
        // skip-when-empty law (never the manifest: the version-blind
        // manifest-canary digest). The gated
        // hold surfaces as the DISTINCT `gloss_via_place_gated` sibling
        // key from day one (the T3 fix-round lesson — no gated-face
        // conflation). Row locator contract: from_x/from_y = the via
        // center at stage entry, to_x/to_y = the landed center (equal
        // to from when no candidate landed).
        if !via_place_report.is_empty() {
            let rows: Vec<serde_json::Value> = via_place_report
                .rows
                .iter()
                .map(|row| {
                    serde_json::json!({
                        "from_x": row.from_x,
                        "from_y": row.from_y,
                        "landed": row.landed,
                        "net": row.net,
                        "net_name": row.net_name,
                        "to_x": row.to_x,
                        "to_y": row.to_y,
                        "via_id": row.via_id,
                    })
                })
                .collect();
            sidecar["gloss_via_place"] = serde_json::Value::Array(rows);
        }
        if via_place_report.gated {
            sidecar["gloss_via_place_gated"] = serde_json::Value::Bool(true);
        }
        // M8-T6: the gloss `gloss_teardrops` block — SAME sidecar, same
        // skip-when-empty law (never the manifest: the version-blind
        // manifest-canary digest). The gated
        // hold surfaces as the DISTINCT `gloss_teardrops_gated` sibling
        // key from day one. Row locator contract: at_x/at_y = the
        // junction (pad center), landed = the taper wrote its wires.
        if !teardrops_report.is_empty() {
            let rows: Vec<serde_json::Value> = teardrops_report
                .rows
                .iter()
                .map(|row| {
                    serde_json::json!({
                        "at_x": row.at_x,
                        "at_y": row.at_y,
                        "landed": row.landed,
                        "net": row.net,
                        "net_name": row.net_name,
                        "pad_diameter": row.pad_diameter,
                        "trace_id": row.trace_id,
                    })
                })
                .collect();
            sidecar["gloss_teardrops"] = serde_json::Value::Array(rows);
        }
        if teardrops_report.gated {
            sidecar["gloss_teardrops_gated"] = serde_json::Value::Bool(true);
        }
        let json = format!(
            "{}\n",
            serde_json::to_string_pretty(&sidecar)
                .expect("serde_json Value serialization cannot fail")
        );
        if let Err(error) = std::fs::write(aesthetics_path, json) {
            eprintln!("Error: cannot write aesthetics sidecar {aesthetics_path}: {error}");
        }
    }
    println!("{summary_row}");
    Ok(exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use epic_board::id::ItemId;
    use epic_board::items::ItemData;
    use epic_dsn::coordinate_transform::CoordinateTransform;
    use epic_dsn::layer_structure::{Layer, LayerStructure};
    use epic_dsn::ses_board::ItemIr;
    use epic_dsn::sink::{BoardRulesIr, BoardSink, CreateBoardIr};
    use epic_dsn::sink::{FixedStateIr, TraceIr};
    use epic_engine::settings::parse_route_args;
    use epic_geometry::int_box::IntBox;
    use epic_geometry::int_point::IntPoint;
    use epic_geometry::point::Point;
    use epic_geometry::tile_shape::TileShape;
    use epic_router::pipeline::batch::{BatchDriver, StopReason};

    /// The final-state mapping (module-docs rule): run-finished ->
    /// COMPLETED; external stop -> CANCELLED; every internal stop ->
    /// COMPLETED (the Java scheduler ignores the batch loop's own
    /// stops).
    #[test]
    fn final_state_mapping() {
        assert_eq!(final_state_for(true, None), "COMPLETED");
        assert_eq!(
            final_state_for(false, Some(StopReason::UserStop)),
            "CANCELLED"
        );
        assert_eq!(
            final_state_for(false, Some(StopReason::MaxPasses)),
            "COMPLETED"
        );
        assert_eq!(
            final_state_for(false, Some(StopReason::StagnationGlobal)),
            "COMPLETED"
        );
        assert_eq!(
            final_state_for(false, Some(StopReason::RestoreExhausted)),
            "COMPLETED"
        );
        assert_eq!(final_state_for(false, None), "COMPLETED");
    }

    /// The exit-code mapping: 0 iff COMPLETED/TIMED_OUT with output.
    #[test]
    fn exit_code_mapping() {
        assert_eq!(exit_code_for("COMPLETED", true), 0);
        assert_eq!(exit_code_for("TIMED_OUT", true), 0);
        assert_eq!(exit_code_for("COMPLETED", false), 1);
        assert_eq!(exit_code_for("CANCELLED", true), 1);
        assert_eq!(exit_code_for("TERMINATED", true), 1);
    }

    // -------------------------------------------------------------------
    // the readiness-fix pins (M2 / M5 / m2 / E2 helpers)
    // -------------------------------------------------------------------

    /// Readiness-fix m2: the glanceable stdout row's exact formatting.
    #[test]
    fn result_summary_row_formatting() {
        assert_eq!(
            result_summary_row(3, 7, 812.3456, "COMPLETED"),
            "result: incomplete=3 violations=7 score=812.35 final=COMPLETED"
        );
        assert_eq!(
            result_summary_row(0, 0, 0.0, "CANCELLED"),
            "result: incomplete=0 violations=0 score=0.00 final=CANCELLED"
        );
    }

    /// Readiness-fix E2: the load-phase Info row's exact formatting
    /// (stderr-only; the manifest never carries the duration).
    #[test]
    fn board_loaded_row_formatting() {
        assert_eq!(
            board_loaded_row(1_234, 56, 2, 1.942),
            "board loaded: 1234 items, 56 nets, 2 pre-existing violations, 1.9s"
        );
    }

    /// Readiness-fix M5: the handler fn called DIRECTLY raises the
    /// shared flag (the charter's unit pin; async-signal-safety is the
    /// body's atomic-ops-only construction).
    #[test]
    fn stop_signal_handler_raises_the_shared_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        STOP_FLAG_SLOT.store(
            Arc::as_ptr(&flag) as *mut AtomicBool,
            std::sync::atomic::Ordering::Relaxed,
        );
        handle_stop_signal(libc::SIGINT);
        assert!(
            flag.load(std::sync::atomic::Ordering::Relaxed),
            "the handler must raise the shared flag"
        );
        // SIGTERM rides the same handler.
        let other = Arc::new(AtomicBool::new(false));
        STOP_FLAG_SLOT.store(
            Arc::as_ptr(&other) as *mut AtomicBool,
            std::sync::atomic::Ordering::Relaxed,
        );
        handle_stop_signal(libc::SIGTERM);
        assert!(other.load(std::sync::atomic::Ordering::Relaxed));
        // Restore the null slot (no dangling pointee behind us).
        STOP_FLAG_SLOT.store(std::ptr::null_mut(), std::sync::atomic::Ordering::Relaxed);
    }

    /// Readiness-fix M5: the wiring seam — the StopFace handed to the
    /// pipeline raises iff the CLI's shared flag does (both the
    /// never-raised face (requirement (a): behavior byte-identical)
    /// and the raised face).
    #[test]
    fn cli_stop_face_carries_the_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        let face = cli_stop_face(&flag);
        assert!(
            !face.is_requested(),
            "no signal => the face is indistinguishable from StopFace::default()"
        );
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(face.is_requested(), "a raise must reach the pipeline face");
    }

    /// The R2-2 face (review finding 3): the slot clears to null — a
    /// signal after the pipeline must hit the handler's null check,
    /// never a dangling pointee (the Arc drops at `run_route`'s exit).
    #[test]
    fn clear_stop_signal_slot_nulls_the_slot() {
        let flag = Arc::new(AtomicBool::new(false));
        STOP_FLAG_SLOT.store(
            Arc::as_ptr(&flag) as *mut AtomicBool,
            std::sync::atomic::Ordering::Relaxed,
        );
        assert!(
            !STOP_FLAG_SLOT
                .load(std::sync::atomic::Ordering::Relaxed)
                .is_null()
        );
        clear_stop_signal_slot();
        assert!(
            STOP_FLAG_SLOT
                .load(std::sync::atomic::Ordering::Relaxed)
                .is_null(),
            "the post-run window must not hold a live pointer"
        );
    }

    /// Readiness-fix M2: an unwritable output parent is the usage
    /// class BEFORE any design byte is read — the helper receives only
    /// the parsed argv, so the input file cannot be touched by
    /// construction; this pin witnesses the immediate `Err` and that
    /// the input marker file survives.
    #[test]
    fn preflight_rejects_unwritable_output_parent() {
        let dir = std::env::temp_dir().join(format!("epic-fix-m2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        // A REGULAR FILE where the -do parent directory would be.
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, b"not a directory").expect("blocker file");
        let dsn = dir.join("board.dsn");
        std::fs::write(&dsn, b"(pcbGNUGNU ..)").expect("dsn marker");
        let args = parse_route_args(&[
            "-de".to_string(),
            dsn.to_string_lossy().into_owned(),
            "-do".to_string(),
            blocker.join("out.ses").to_string_lossy().into_owned(),
        ])
        .expect("args parse");
        let error = preflight_output_path(&args).expect_err("unwritable parent must reject");
        assert!(
            error.contains("output directory not usable"),
            "the usage-class message names the -do parent: {error}"
        );
        // The input file is untouched (the failure is immediate).
        assert_eq!(
            std::fs::read(&dsn).expect("dsn still present"),
            b"(pcbGNUGNU ..)",
            "the pre-flight must not touch the input"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Readiness-fix M2: the success face — the parent dir is created,
    /// the probe leaves NO residue, and the run is accepted.
    #[test]
    fn preflight_ok_creates_parent_and_leaves_no_probe() {
        let dir = std::env::temp_dir().join(format!("epic-fix-m2-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let target = dir.join("nested").join("out.ses");
        let args = parse_route_args(&[
            "-de".to_string(),
            dir.join("board.dsn").to_string_lossy().into_owned(),
            "-do".to_string(),
            target.to_string_lossy().into_owned(),
        ])
        .expect("args parse");
        preflight_output_path(&args).expect("creatable parent accepted");
        assert!(dir.join("nested").is_dir(), "the parent was created");
        let residue: Vec<_> = std::fs::read_dir(dir.join("nested"))
            .expect("read the created dir")
            .filter_map(std::result::Result::ok)
            .collect();
        assert!(
            residue.is_empty(),
            "the probe file must be deleted: {residue:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The pass-count fill: the AUTOROUTE stage's own completed count
    /// (`RouteTelemetry::autoroute_passes_completed`, the pipeline
    /// outcome's face), rendered only when a pass ran (> 0). A 0 face
    /// (no pass ran — router disabled, fanout-only, or a stop before
    /// the first pass) renders an absent row, matching Java's fanout
    /// phase which never sets `passesCompleted`
    /// (AutorouteBatchLoop.java:231-246).
    #[test]
    fn manifest_passes_completed_backfill() {
        let mut telemetry = RouteTelemetry {
            final_state: "COMPLETED".to_string(),
            pour_islands: Vec::new(),
            length_report: Vec::new(),
            pair_report: Vec::new(),
            pair_unresolved: Vec::new(),
            pin_assign_rows: Vec::new(),
            pin_assign_unresolved: Vec::new(),
            current_width_rows: Vec::new(),
            current_width_unresolved: Vec::new(),
            pour_synthesis_rows: Vec::new(),
            pour_synthesis_unresolved: Vec::new(),
            global_plan: None,
            autoroute_passes_completed: 7,
            ..RouteTelemetry::default()
        };
        let json = render_manifest(&telemetry, "0.1.0", "deadbeef", fixture("b.dsn", "aa"));
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(
            value
                .pointer("/phases/autorouter/passes_completed")
                .and_then(serde_json::Value::as_i64),
            Some(7)
        );

        // A zero pass count (no pass ran) stays absent.
        telemetry.autoroute_passes_completed = 0;
        let json = render_manifest(&telemetry, "0.1.0", "deadbeef", fixture("b.dsn", "aa"));
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert!(
            value
                .pointer("/phases/autorouter/passes_completed")
                .is_none()
        );
    }

    /// The fanout-only face: when no autoroute pass ran
    /// (`autoroute_passes_completed == 0` — the fanout-only mode's
    /// `max_passes=0` first-iteration break), the pass row is absent
    /// and the fanout phase slot itself stays the empty `{}`
    /// projection (Java's fanout phase metrics carry only before/after
    /// snapshots + duration + cpu, AutorouteBatchLoop.java:231-246 —
    /// all in the manifest's non-determinism omission family).
    #[test]
    fn manifest_fanout_only_run_reports_no_autorouter_passes() {
        let telemetry = RouteTelemetry {
            final_state: "COMPLETED".to_string(),
            pour_islands: Vec::new(),
            length_report: Vec::new(),
            pair_report: Vec::new(),
            pair_unresolved: Vec::new(),
            pin_assign_rows: Vec::new(),
            pin_assign_unresolved: Vec::new(),
            current_width_rows: Vec::new(),
            current_width_unresolved: Vec::new(),
            pour_synthesis_rows: Vec::new(),
            pour_synthesis_unresolved: Vec::new(),
            global_plan: None,
            autoroute_passes_completed: 0,
            ..RouteTelemetry::default()
        };
        let json = render_manifest(&telemetry, "0.1.0", "deadbeef", fixture("b.dsn", "aa"));
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert!(
            value
                .pointer("/phases/autorouter/passes_completed")
                .is_none(),
            "a run with no autoroute pass must not report one: {json}"
        );
        assert_eq!(
            value["phases"]["fanout"],
            serde_json::json!({}),
            "the fanout phase slot is the empty projection"
        );
    }

    /// The stage-attribution witness (the run-1 face of the seam bug
    /// this fixed): after a 2-pass autoroute the OPTIMIZER stage runs
    /// more passes of its own, and BOTH rows report their OWN stage's
    /// count — `phases.autorouter.passes_completed == 2` (the stage
    /// outcome) while `phases.optimizer.passes_completed == 3` (its
    /// own outcome, Java `BatchOptimizer.java:523`). The old seam (the
    /// LAST `phase="autoroute"` counters row) reported 1 here — the
    /// optimizer's per-item reroutes stamp `phase="autoroute"` rows
    /// through the same shared pass tail.
    #[test]
    fn both_stage_rows_report_their_own_passes_after_an_optimizer_stage() {
        let optimizer_faces = epic_router::pipeline::full::OptimizerStageFaces {
            before: scoreable_stats(),
            after: scoreable_stats(),
            outcome: epic_router::pipeline::optimizer::OptimizerOutcome {
                passes_completed: 3,
                is_timed_out: false,
            },
        };
        let phases = PipelinePhases {
            optimizer: Some(optimizer_faces),
            ..PipelinePhases::default()
        };
        let telemetry = RouteTelemetry {
            final_state: "COMPLETED".to_string(),
            pour_islands: Vec::new(),
            length_report: Vec::new(),
            pair_report: Vec::new(),
            pair_unresolved: Vec::new(),
            pin_assign_rows: Vec::new(),
            pin_assign_unresolved: Vec::new(),
            current_width_rows: Vec::new(),
            current_width_unresolved: Vec::new(),
            pour_synthesis_rows: Vec::new(),
            pour_synthesis_unresolved: Vec::new(),
            global_plan: None,
            autoroute_passes_completed: 2,
            phases,
            ..RouteTelemetry::default()
        };
        let json = render_manifest(&telemetry, "0.1.0", "deadbeef", fixture("b.dsn", "aa"));
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(
            value
                .pointer("/phases/autorouter/passes_completed")
                .and_then(serde_json::Value::as_i64),
            Some(2),
            "the AUTOROUTE row reports its own stage's count: {json}"
        );
        assert_eq!(
            value
                .pointer("/phases/optimizer/passes_completed")
                .and_then(serde_json::Value::as_i64),
            Some(3),
            "the OPTIMIZER row reports its own stage outcome: {json}"
        );
    }

    /// The M4-T9 stage wiring (Java `RoutingPipeline.runOptimizationStage`):
    /// the disabled face (`--optimizer.enabled=false`) must reproduce
    /// the pre-T9 flow byte-exactly — here pinned as self-consistency
    /// of two identical disabled runs — and the enabled default (the
    /// DefaultSettings seed) completes. (Review NIT-1 precision: this
    /// pin observes the exit code and the manifest only — the stage's
    /// ENTRY is witnessed by the harness determinism logs
    /// (`determinism_fullflow_t9.log`: the "Optimization stage started"
    /// row + the 823.37 -> 823.50 movement), not by this pin; and the
    /// disabled arm is a self-consistency face, not a comparison
    /// against the recorded T7 digests — those are gated by the
    /// harness-level digest controls.)
    #[test]
    fn t9_route_wiring_disabled_face_and_enabled_default() {
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../harness/fixtures/locator-spike/t9_locator45.dsn"
        );
        let out_dir = std::env::temp_dir().join(format!("epic-t9-wiring-{}", std::process::id()));
        std::fs::create_dir_all(&out_dir).expect("temp dir");

        let run = |tag: &str, extra: &[&str]| {
            let ses_path = out_dir.join(format!("out-{tag}.ses"));
            let manifest_path = out_dir.join(format!("out-{tag}.json"));
            let mut argv = vec![
                "-de".to_string(),
                fixture_path.to_string(),
                "-do".to_string(),
                ses_path.to_string_lossy().into_owned(),
                "--result-json".to_string(),
                manifest_path.to_string_lossy().into_owned(),
            ];
            argv.extend(extra.iter().map(|s| s.to_string()));
            let args = parse_route_args(&argv).expect("args parse");
            let exit = run_route(&args).expect("route run");
            (exit, ses_path, manifest_path)
        };

        // The disabled face: two runs, byte-identical ses + manifest.
        let (exit_a, ses_a, man_a) = run("off1", &["--optimizer.enabled=false"]);
        let (exit_b, ses_b, man_b) = run("off2", &["--optimizer.enabled=false"]);
        assert_eq!(exit_a, 0);
        assert_eq!(exit_b, 0);
        let ses_a_bytes = std::fs::read(&ses_a).expect("ses a");
        let ses_b_bytes = std::fs::read(&ses_b).expect("ses b");
        assert_eq!(ses_a_bytes, ses_b_bytes, "disabled face is deterministic");
        assert_eq!(
            std::fs::read(&man_a).expect("man a"),
            std::fs::read(&man_b).expect("man b"),
        );
        assert!(!ses_a_bytes.is_empty());

        // The enabled default: the stage gate opens (run_optimizer=true,
        // the router completed without a stop) and the run completes.
        let (exit_on, _ses_on, man_on) = run("on", &[]);
        assert_eq!(exit_on, 0, "the enabled flow completes");
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&man_on).expect("manifest"))
                .expect("json");
        assert_eq!(value["final_state"], "COMPLETED");

        let _ = std::fs::remove_dir_all(&out_dir);
    }

    fn fixture<'a>(filename: &'a str, sha: &'a str) -> ManifestFixture<'a> {
        ManifestFixture {
            filename: Some(filename),
            sha256: Some(sha),
        }
    }

    /// The flow pins' temp directory (quality-review NIT-2: one
    /// scaffolding helper instead of a hand-rolled block per pin).
    fn temp_out(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("epic-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// THE NON-FINITE RENDER BOUNDARY (quality-review MINOR-2): Java's
    /// adapters check `!Float.isFinite(value)` and emit
    /// `out.nullValue()` (`GsonProvider.java:39`/`:59`) — the field
    /// OMITTED. `JavaDecimal::of` mirrors that as `None` (feeding
    /// `skip_serializing_if`); `java_two_decimal` passes non-finite
    /// through (the face decision lives at the text step). Pinned on
    /// the same boundary: the -0.0 -> positive `0.00` sign face, and
    /// the f32-eighth HALF_UP tie (`2.125` -> `2.13` — Rust's
    /// half-to-EVEN `format!` would print `2.12`) — cerebrum 16's
    /// exact-boundary discipline.
    #[test]
    fn t10_non_finite_render_boundary_is_java_null_value_face() {
        assert!(JavaDecimal::of(f64::NAN).is_none());
        assert!(JavaDecimal::of(f64::INFINITY).is_none());
        assert!(JavaDecimal::of(f64::NEG_INFINITY).is_none());

        // Finite face: the %.2f literal text.
        let finite = JavaDecimal::of(java_two_decimal(925.0)).expect("finite face");
        assert_eq!(finite.0.get(), "925.00");

        // The -0.0 sign face: positive zero, the jar's face.
        let zero = JavaDecimal::of(java_two_decimal(-0.0)).expect("zero face");
        assert_eq!(zero.0.get(), "0.00");

        // The HALF_UP tie face on an f32 eighth.
        let tie = JavaDecimal::of(java_two_decimal(2.125)).expect("tie face");
        assert_eq!(tie.0.get(), "2.13");

        // The value step passes non-finite through; the text step owns
        // the nullValue decision.
        assert!(java_two_decimal(f32::NAN).is_nan());
        assert!(java_two_decimal(f32::INFINITY).is_infinite());
    }

    /// Real (scoreable) boundary statistics from the tiny two-layer
    /// test board: `new_empty()` panics under the scorer's Java-NPE
    /// parity arm (maximumCount is None there), so the render pins
    /// carry a genuine walk.
    fn scoreable_stats() -> BoardStatistics {
        let ses = two_layer_ses();
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        BoardStatistics::new(&mut manager, &mut board)
    }

    /// THE OPTIMIZER-SCORE IFF-GATE (Java RoutingResultManifest.java:200-202),
    /// both arms plus the fanout crossing: the top-level key rides iff
    /// the OPTIMIZER phase's before/after faces exist — not with
    /// fanout/autorouter faces alone, and not when the stage was
    /// disabled or gated out. A mutant emitting unconditionally fails
    /// the absent arm; a mutant gating on ANY phase face fails the
    /// fanout arm (cerebrum 13: the crossing cell is the pin).
    #[test]
    fn t10_optimizer_score_iff_gate_both_arms() {
        let opt_faces = || epic_router::pipeline::full::OptimizerStageFaces {
            before: scoreable_stats(),
            after: scoreable_stats(),
            outcome: epic_router::pipeline::optimizer::OptimizerOutcome {
                passes_completed: 2,
                is_timed_out: false,
            },
        };
        let telemetry_with = |phases: PipelinePhases| RouteTelemetry {
            final_state: "COMPLETED".to_string(),
            pour_islands: Vec::new(),
            length_report: Vec::new(),
            pair_report: Vec::new(),
            pair_unresolved: Vec::new(),
            pin_assign_rows: Vec::new(),
            pin_assign_unresolved: Vec::new(),
            current_width_rows: Vec::new(),
            current_width_unresolved: Vec::new(),
            pour_synthesis_rows: Vec::new(),
            pour_synthesis_unresolved: Vec::new(),
            global_plan: None,
            autoroute_passes_completed: 0,
            connections: Some(ManifestConnections {
                incomplete_count: 0,
                maximum_count: 5,
            }),
            clearance_violations: Some(ManifestClearanceViolations {
                total_count: 0,
                router_introduced_count: 0,
            }),
            normalized_score: Some(1000.0),
            output_written: true,
            phases,
            pre_existing_violations: 0,
            scoring: Some(RouterSettingsScoring::default()),
            final_optimizer_score: Some(823.5),
        };

        // Arm 1: the optimizer stage ran -> the key is present.
        let json = render_manifest(
            &telemetry_with(PipelinePhases {
                optimizer: Some(opt_faces()),
                ..PipelinePhases::default()
            }),
            "0.1.0",
            "deadbeef",
            fixture("b.dsn", "aa"),
        );
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(
            value["optimizer_score"].as_f64(),
            Some(823.5),
            "iff-gate open arm: {json}"
        );
        assert_eq!(
            value
                .pointer("/phases/optimizer/passes_completed")
                .and_then(serde_json::Value::as_i64),
            Some(2)
        );

        // Arm 2: no faces at all (stage disabled/gated out) -> absent.
        let json = render_manifest(
            &telemetry_with(PipelinePhases::default()),
            "0.1.0",
            "deadbeef",
            fixture("b.dsn", "aa"),
        );
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert!(
            value.get("optimizer_score").is_none(),
            "iff-gate closed arm: {json}"
        );

        // Arm 3 (crossing): fanout + autorouter faces WITHOUT the
        // optimizer -> still absent (the gate is the OPTIMIZER phase).
        let json = render_manifest(
            &telemetry_with(PipelinePhases {
                fanout: Some((scoreable_stats(), scoreable_stats())),
                autorouter: Some((scoreable_stats(), scoreable_stats())),
                optimizer: None,
            }),
            "0.1.0",
            "deadbeef",
            fixture("b.dsn", "aa"),
        );
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert!(
            value.get("optimizer_score").is_none(),
            "fanout/autorouter faces must not open the gate: {json}"
        );
        assert!(
            value.pointer("/phases/fanout/before").is_some(),
            "fanout faces render"
        );
    }

    /// THE T10 PER-PHASE ATTRIBUTION PIN: each phase carries ITS OWN
    /// source. Fanout snapshots: `score_source not_applicable`, NO
    /// score rows, NO `passes_completed` (Java AutorouteBatchLoop.java:
    /// 231-246); the autorouter row reads the stage outcome (absent
    /// here — no autoroute pass ran); the OPTIMIZER row's pass count
    /// comes from the stage outcome (0), not any counters event.
    #[test]
    fn t10_phase_rows_attribution_crossing_cells() {
        let telemetry = RouteTelemetry {
            final_state: "COMPLETED".to_string(),
            pour_islands: Vec::new(),
            length_report: Vec::new(),
            pair_report: Vec::new(),
            pair_unresolved: Vec::new(),
            pin_assign_rows: Vec::new(),
            pin_assign_unresolved: Vec::new(),
            current_width_rows: Vec::new(),
            current_width_unresolved: Vec::new(),
            pour_synthesis_rows: Vec::new(),
            pour_synthesis_unresolved: Vec::new(),
            global_plan: None,
            autoroute_passes_completed: 0,
            connections: Some(ManifestConnections {
                incomplete_count: 2,
                maximum_count: 6,
            }),
            clearance_violations: Some(ManifestClearanceViolations {
                total_count: 1,
                router_introduced_count: 0,
            }),
            normalized_score: Some(900.0),
            output_written: true,
            phases: PipelinePhases {
                fanout: Some((scoreable_stats(), scoreable_stats())),
                optimizer: Some(epic_router::pipeline::full::OptimizerStageFaces {
                    before: scoreable_stats(),
                    after: scoreable_stats(),
                    outcome: epic_router::pipeline::optimizer::OptimizerOutcome {
                        passes_completed: 0,
                        is_timed_out: false,
                    },
                }),
                ..PipelinePhases::default()
            },
            pre_existing_violations: 1,
            scoring: Some(RouterSettingsScoring::default()),
            final_optimizer_score: Some(950.25),
        };
        let json = render_manifest(&telemetry, "0.1.0", "deadbeef", fixture("b.dsn", "aa"));
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        // Fanout rows: snapshots present, score_source not_applicable,
        // no score rows, and NO passes_completed.
        assert!(value.pointer("/phases/fanout/before").is_some());
        assert_eq!(
            value
                .pointer("/phases/fanout/before/score_source")
                .and_then(serde_json::Value::as_str),
            Some("not_applicable")
        );
        assert!(
            value.pointer("/phases/fanout/before/score").is_none()
                && value
                    .pointer("/phases/fanout/before/router_score")
                    .is_none()
                && value
                    .pointer("/phases/fanout/before/optimizer_score")
                    .is_none(),
            "Java's fanout fill omits ALL THREE score rows (spec-review F1): {json}"
        );
        assert!(
            value.pointer("/phases/fanout/passes_completed").is_none(),
            "fanout phase never carries a pass count: {json}"
        );
        // Optimizer row: pass count from the OUTCOME (0), not the
        // fanout counters (1).
        assert_eq!(
            value
                .pointer("/phases/optimizer/passes_completed")
                .and_then(serde_json::Value::as_i64),
            Some(0),
            "the outcome is the source, counters must not leak: {json}"
        );
        // The top-level gate opens (optimizer faces exist).
        assert_eq!(value["optimizer_score"].as_f64(), Some(950.25));
        // No autorouter faces -> no autorouter snapshot, no pass count.
        assert!(
            value
                .pointer("/phases/autorouter/passes_completed")
                .is_none()
        );
    }

    /// THE FULL-FLOW MANIFEST FACES (feature pin — the per-phase rows
    /// do not exist pre-T10; the manifest digest rotation IS the
    /// pre-refactor witness, see evidence/pre-refactor-witness.md): two
    /// runs through the real flow on the locator fixture (fanout +
    /// optimizer ON by default, no pass bound so the router completes
    /// and the optimizer gate opens) must be byte-equal, and the
    /// manifest carries Java's per-phase faces — fanout snapshots with
    /// `not_applicable`, autorouter with pass count, optimizer rows +
    /// the top-level `optimizer_score` (the `:200-202` iff-gate arm).
    #[test]
    fn t10_full_flow_manifest_carries_the_java_phase_faces() {
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../harness/fixtures/locator-spike/t9_locator45.dsn"
        );
        let out_dir = temp_out("t10-full");
        let run = |tag: &str| {
            let ses_path = out_dir.join(format!("out-{tag}.ses"));
            let manifest_path = out_dir.join(format!("out-{tag}.json"));
            let args = parse_route_args(&route_argv(
                std::path::Path::new(fixture_path),
                &ses_path,
                Some(&manifest_path),
            ))
            .expect("args parse");
            let exit = run_route(&args).expect("route run");
            (exit, ses_path, manifest_path)
        };
        let (exit_a, ses_a, man_a) = run("a");
        let (exit_b, ses_b, man_b) = run("b");
        assert_eq!(exit_a, 0);
        assert_eq!(exit_b, 0);
        assert_eq!(
            std::fs::read(&ses_a).expect("ses a"),
            std::fs::read(&ses_b).expect("ses b"),
            "ses byte-equal x2"
        );
        assert_eq!(
            std::fs::read(&man_a).expect("man a"),
            std::fs::read(&man_b).expect("man b"),
            "manifest byte-equal x2"
        );
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&man_a).expect("manifest"))
                .expect("json");
        // Fanout phase: populated with snapshots, not_applicable, no
        // pass count, and NO score rows (the F1 face, oracle-verified
        // against the jar's fanout-ran probe).
        assert!(
            value.pointer("/phases/fanout/before").is_some(),
            "fanout ran: {value}"
        );
        assert_eq!(
            value
                .pointer("/phases/fanout/before/score_source")
                .and_then(serde_json::Value::as_str),
            Some("not_applicable")
        );
        assert!(
            value
                .pointer("/phases/fanout/before/router_score")
                .is_none()
                && value
                    .pointer("/phases/fanout/before/optimizer_score")
                    .is_none(),
            "fanout rows carry no score fields (F1): {value}"
        );
        assert!(value.pointer("/phases/fanout/passes_completed").is_none());
        // Autorouter phase: snapshots + pass count.
        assert!(value.pointer("/phases/autorouter/before").is_some());
        assert_eq!(
            value
                .pointer("/phases/autorouter/before/score_source")
                .and_then(serde_json::Value::as_str),
            Some("current")
        );
        let passes = value
            .pointer("/phases/autorouter/passes_completed")
            .and_then(serde_json::Value::as_i64)
            .expect("the router ran");
        assert!(passes >= 1);
        // Optimizer phase + the iff-gate arm.
        assert!(value.pointer("/phases/optimizer/before").is_some());
        assert!(
            value
                .get("optimizer_score")
                .and_then(serde_json::Value::as_f64)
                .is_some()
        );
        let _ = std::fs::remove_dir_all(&out_dir);
    }

    /// THE FANOUT-ONLY FLOW FACE — mixed anchoring (spec-review F5b):
    /// the ORACLE-ANCHORED rows are the optimizer-bypass faces, from
    /// the jar's `bm08-noroute-manifest.json` (router off, FANOUT OFF —
    /// the T8 capture did not fan out) and the T10 fix-round probe
    /// (`logs/M4-T10/evidence/jar-fanout-ran-probe.log`: jar on bm08
    /// with `--router.autorouter.enabled=false --router.fanout.enabled=
    /// true` — the Java CLI couples router-off to fanout-off, so the
    /// true fanout-ran world needs the explicit flag): fanout rows carry
    /// NO score fields (F1), the autorouter phase stays the empty `{}`,
    /// the optimizer stage runs and is guard-bypassed
    /// (`passes_completed: 0`) with before/after FILLED, and the
    /// top-level `optimizer_score` IS emitted (the bypass sets
    /// before/after, so the iff-gate opens). The fanout-row-POPULATED
    /// assertions are port-side self-faces (the port's `--router.
    /// enabled=off` leaves fanout on by default; the Java coupling is
    /// not ported — recorded as a settings-layer divergence bank).
    #[test]
    fn t10_fanout_only_flow_face_matches_the_jar_noroute_face() {
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../harness/fixtures/locator-spike/t9_locator45.dsn"
        );
        let out_dir = temp_out("t10-fanout-only");
        let ses_path = out_dir.join("out.ses");
        let manifest_path = out_dir.join("out.json");
        let mut argv = route_argv(
            std::path::Path::new(fixture_path),
            &ses_path,
            Some(&manifest_path),
        );
        argv.push("--router.enabled=off".to_string());
        let args = parse_route_args(&argv).expect("args parse");
        let exit = run_route(&args).expect("route run");
        assert_eq!(exit, 0, "the fanout-only run completes (jar COMPLETED)");
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).expect("manifest"))
                .expect("json");
        assert!(
            value.pointer("/phases/fanout/before").is_some(),
            "fanout ran: {value}"
        );
        assert!(
            value.pointer("/phases/autorouter/before").is_none(),
            "no autorouter boundary"
        );
        assert!(
            value
                .pointer("/phases/autorouter/passes_completed")
                .is_none(),
            "no passes ran"
        );
        assert_eq!(
            value
                .pointer("/phases/optimizer/passes_completed")
                .and_then(serde_json::Value::as_i64),
            Some(0),
            "guard-1 bypass: explicit 0 (jar face)"
        );
        assert!(
            value
                .get("optimizer_score")
                .and_then(serde_json::Value::as_f64)
                .is_some(),
            "the bypass still emits the top-level score (jar face)"
        );
        let _ = std::fs::remove_dir_all(&out_dir);
    }

    /// The manifest's REQUIRED members and the OMITTED non-determinism
    /// family: the emitted bytes contain NONE of generated_at /
    /// duration / cpu / memory / resource_usage / settings_snapshot /
    /// bounds / optimizer_score, even as keys, while every mirror-
    /// required member is present (schema v1). The autoroute pass row
    /// renders from the stage face alone (2) — no counters text can
    /// leak because none is carried.
    #[test]
    fn manifest_omits_non_determinism_family_and_keeps_required() {
        let telemetry = RouteTelemetry {
            final_state: "COMPLETED".to_string(),
            pour_islands: Vec::new(),
            length_report: Vec::new(),
            pair_report: Vec::new(),
            pair_unresolved: Vec::new(),
            pin_assign_rows: Vec::new(),
            pin_assign_unresolved: Vec::new(),
            current_width_rows: Vec::new(),
            current_width_unresolved: Vec::new(),
            pour_synthesis_rows: Vec::new(),
            pour_synthesis_unresolved: Vec::new(),
            global_plan: None,
            autoroute_passes_completed: 2,
            connections: Some(ManifestConnections {
                incomplete_count: 1,
                maximum_count: 3,
            }),
            clearance_violations: Some(ManifestClearanceViolations {
                total_count: 4,
                router_introduced_count: 2,
            }),
            normalized_score: Some(812.5),
            output_written: true,
            // No phase faces and no optimizer score in this world — the
            // T13-era omission assertions stay honest (the top-level
            // `optimizer_score` key is now the T10 iff-gate, and it
            // holds NONE faces -> absent here).
            phases: PipelinePhases::default(),
            pre_existing_violations: 0,
            scoring: None,
            final_optimizer_score: None,
        };
        let json = render_manifest(&telemetry, "0.1.0", "deadbeef", fixture("b.dsn", "aa"));
        for omitted in [
            "generated_at",
            "duration",
            "cpu",
            "memory",
            "resource_usage",
            "settings_snapshot",
            "bounds",
            "optimizer_score",
        ] {
            assert!(
                !json.contains(omitted),
                "the non-determinism family must be omitted: found '{omitted}' in {json}"
            );
        }
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["app_version"], "0.1.0");
        assert_eq!(value["git_sha"], "deadbeef");
        assert_eq!(value["fixture"]["filename"], "b.dsn");
        assert_eq!(value["fixture"]["sha256"], "aa");
        assert_eq!(value["final_state"], "COMPLETED");
        assert_eq!(value["exit_code"], 0);
        assert_eq!(value["output_written"], true);
        assert_eq!(
            value["board_statistics"]["connections"]["incomplete_count"],
            1
        );
        assert_eq!(
            value["board_statistics"]["clearance_violations"]["router_introduced_count"],
            2
        );
        assert_eq!(value["normalized_score"], 812.5);
    }

    /// The iff-gates: with no statistics and no score, the members are
    /// absent (the deliberate omission family — module docs item 1:
    /// Java emits these keys only alongside real payloads, and no
    /// payload exists here) while the object still parses.
    #[test]
    fn manifest_iff_gates() {
        let telemetry = RouteTelemetry {
            final_state: "CANCELLED".to_string(),
            ..RouteTelemetry::default()
        };
        let json = render_manifest(&telemetry, "0.1.0", "deadbeef", fixture("b.dsn", "aa"));
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert!(value.get("normalized_score").is_none());
        assert!(value.get("optimizer_score").is_none());
        assert!(value.get("board_statistics").is_none());
        assert_eq!(value["exit_code"], 1, "CANCELLED without output is exit 1");

        // Even WITH output on disk, a non-COMPLETED state stays exit 1
        // (the pair gate in exit_code_for).
        let telemetry = RouteTelemetry {
            final_state: "CANCELLED".to_string(),
            output_written: true,
            ..RouteTelemetry::default()
        };
        let json = render_manifest(&telemetry, "0.1.0", "deadbeef", fixture("b.dsn", "aa"));
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(value["output_written"], true);
        assert_eq!(
            value["exit_code"], 1,
            "CANCELLED with output is still exit 1"
        );
    }

    /// The SHA-256 of the INPUT bytes (a known vector: "abc").
    #[test]
    fn sha256_of_input_bytes() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// PG3 (review F3): the router_introduced face subtracts the
    /// LOAD-TIME seed (`BoardStatistics.java:390-393` = max(0, total -
    /// preExisting); the seed itself is the flow's step 2b,
    /// `HeadlessBoardManager.java:793`). A board that loads with 5
    /// violations and ends with 9 reports 4 router-introduced, NOT 9;
    /// a post-route total BELOW the seed (router ripped up violating
    /// load-time wiring) floors at 0; an unmeasured walk reports 0/0.
    #[test]
    fn router_introduced_subtracts_pre_existing_seed() {
        let face = clearance_face(Some(9), 5);
        assert_eq!(face.total_count, 9);
        assert_eq!(face.router_introduced_count, 4);

        let face = clearance_face(Some(3), 5);
        assert_eq!(face.total_count, 3);
        assert_eq!(face.router_introduced_count, 0, "saturating floor");

        let face = clearance_face(None, 5);
        assert_eq!((face.total_count, face.router_introduced_count), (0, 0));

        let face = clearance_face(Some(7), 0);
        assert_eq!(face.router_introduced_count, 7, "no seed -> all introduced");
    }

    /// F1 end-to-end (review F1): a NEGATIVE `max_passes` must route,
    /// not silently skip — validate() resets it to 0 (= unlimited)
    /// with a warn before the driver's router-enabled gate reads it.
    /// Bounded with `max_items=1` so the witness stays cheap. Mutant
    /// face: dropping the validate() call leaves -5 in the batch
    /// settings, the gate reads it as router-disabled, and the run
    /// "completes" having routed nothing (0 passes, 0 path rows).
    #[test]
    fn negative_max_passes_routes_after_validate_reset() {
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../harness/fixtures/locator-spike/t9_locator45.dsn"
        );
        let out_dir = std::env::temp_dir().join(format!("epic-t13-f1-{}", std::process::id()));
        std::fs::create_dir_all(&out_dir).expect("temp dir");
        let ses_path = out_dir.join("out.ses");
        let manifest_path = out_dir.join("out.json");

        let argv = vec![
            "-de".to_string(),
            fixture_path.to_string(),
            "-do".to_string(),
            ses_path.to_string_lossy().into_owned(),
            "--result-json".to_string(),
            manifest_path.to_string_lossy().into_owned(),
            "--router.autorouter.max_passes=-5".to_string(),
            "--router.autorouter.max_items=1".to_string(),
        ];
        let args = parse_route_args(&argv).expect("args parse");
        let exit = run_route(&args).expect("route run");

        assert_eq!(exit, 0, "the reset-to-unlimited face routes");
        let manifest_text = std::fs::read_to_string(&manifest_path).expect("manifest written");
        let value: serde_json::Value = serde_json::from_str(&manifest_text).expect("json");
        assert_eq!(value["final_state"], "COMPLETED");
        let passes = value
            .pointer("/phases/autorouter/passes_completed")
            .and_then(serde_json::Value::as_i64)
            .expect("at least one pass ran (a dropped validate() leaves none)");
        assert!(passes >= 1);

        let ses_text = std::fs::read_to_string(&ses_path).expect("ses written");
        assert!(
            ses_text.contains("(path "),
            "the board ROUTED (a dropped validate() leaves 0 path rows)"
        );

        let _ = std::fs::remove_dir_all(&out_dir);
    }

    /// F3 wiring witness (review F3/PG3): on a violation-bearing input
    /// (harness/corpus/craft/drc-main.dsn, golden drc-0015 = 3
    /// RAW-walk violations) the manifest must NOT report the
    /// load-time rows as router-introduced — the seed walk (flow step
    /// 2b) runs BEFORE the driver. The discriminating face moved with
    /// #925a: the FINAL face is now 0 (see below), where a
    /// deleted-seed mutant is invisible (`max(0, 0-0) == max(0, 0-2)
    /// == 0`), so the pin reads the FANOUT-BEFORE phase face —
    /// total 2 with router_introduced 0. Mutant faces: a deleted-seed
    /// block reports before-introduced = 2 (the un-subtracted walk);
    /// a gate-removed (#925a-deleted) gate reports before-total = 3.
    ///
    /// P3 note (the deliberate #925a flip, verified by probe
    /// 2026-10-02): the golden 3's third row is the OUTLINE vs a
    /// pin-crossing-the-edge pair whose RULE clearance is 0 (the
    /// copper-to-edge override writes board_edge cells for classes
    /// 1 and up only — Java `:534-539` — leaving this pin's class
    /// cell at the append-init 0) and whose measured clearance is 0
    /// (the raw shapes overlap) — shortfall EXACTLY 0.0. The STRICT gate
    /// drops it at EVERY tolerance (0.0 > 0.0 is false), so no
    /// tolerance — including the 0.0 control — restores the pre-925a
    /// face; upstream #925a behaves identically. The honest CLI load
    /// face is 2 (the trace rows, shortfalls 2000/1649 µm), pinned
    /// with the gate face by
    /// `clearance_tolerance_gate_drops_zero_shortfall_edge_pin_row_e2e`
    /// below.
    #[test]
    fn pre_existing_violations_seeded_from_load() {
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../harness/corpus/craft/drc-main.dsn"
        );
        let out_dir = std::env::temp_dir().join(format!("epic-t13-f3-{}", std::process::id()));
        std::fs::create_dir_all(&out_dir).expect("temp dir");
        let ses_path = out_dir.join("out.ses");
        let manifest_path = out_dir.join("out.json");

        let argv = vec![
            "-de".to_string(),
            fixture_path.to_string(),
            "-do".to_string(),
            ses_path.to_string_lossy().into_owned(),
            "--result-json".to_string(),
            manifest_path.to_string_lossy().into_owned(),
            "--router.autorouter.max_passes=1".to_string(),
            "--router.autorouter.max_items=1".to_string(),
        ];
        let args = parse_route_args(&argv).expect("args parse");
        let exit = run_route(&args).expect("route run");
        assert_eq!(exit, 0);

        let manifest_text = std::fs::read_to_string(&manifest_path).expect("manifest written");
        let value: serde_json::Value = serde_json::from_str(&manifest_text).expect("json");
        let before = value
            .pointer("/phases/fanout/before/board_statistics/clearance_violations")
            .expect("fanout-before clearance face present");
        let before_total = before["total_count"]
            .as_i64()
            .expect("before total count present");
        let before_introduced = before["router_introduced_count"]
            .as_i64()
            .expect("before router-introduced count present");
        assert_eq!(
            before_total, 2,
            "the post-#925a load face: the two trace rows (a gate-removed mutant reports 3, \
             the raw-walk golden drc-0015)"
        );
        assert_eq!(
            before_introduced, 0,
            "load-time rows must be excluded from router_introduced (a deleted-seed mutant \
             reports the un-subtracted 2)"
        );
        let final_total = value["board_statistics"]["clearance_violations"]["total_count"]
            .as_i64()
            .expect("final total count present");
        assert_eq!(
            final_total, 0,
            "the bounded route rips the two pin-less trace rows — the honest post-#925a \
             final face"
        );

        let _ = std::fs::remove_dir_all(&out_dir);
    }

    /// P3 (#925a) end-to-end gate face on the SAME craft at the DEFAULT
    /// tolerance (no flag): the golden 3's third row — outline vs the
    /// edge-crossing pin, RULE clearance 0 (the copper-to-edge override
    /// writes board_edge cells for classes >= 1 only) and measured
    /// clearance 0 (the raw shapes overlap), shortfall EXACTLY 0.0 —
    /// is dropped by the STRICT gate at the load seed (fanout.before =
    /// 2), and the bounded route rips the two remaining trace rows
    /// (final total 0). Pinned through the full CLI manifest path
    /// (parse → seed walk → phases → final). A gate-removed mutant
    /// reports before = 3 and fails the first assert. The two tests
    /// deliberately overlap on the 2/0 face: THIS one pins the gate
    /// (its `#925a`-deleted mutant), the seeding test above pins the
    /// seed subtraction (its deleted-seed mutant) — orthogonal kills.
    #[test]
    fn clearance_tolerance_gate_drops_zero_shortfall_edge_pin_row_e2e() {
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../harness/corpus/craft/drc-main.dsn"
        );
        let out_dir = std::env::temp_dir().join(format!("epic-p3-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&out_dir).expect("temp dir");
        let ses_path = out_dir.join("out.ses");
        let manifest_path = out_dir.join("out.json");

        let argv = vec![
            "-de".to_string(),
            fixture_path.to_string(),
            "-do".to_string(),
            ses_path.to_string_lossy().into_owned(),
            "--result-json".to_string(),
            manifest_path.to_string_lossy().into_owned(),
            "--router.autorouter.max_passes=1".to_string(),
            "--router.autorouter.max_items=1".to_string(),
        ];
        let args = parse_route_args(&argv).expect("args parse");
        let exit = run_route(&args).expect("route run");
        assert_eq!(exit, 0);

        let manifest_text = std::fs::read_to_string(&manifest_path).expect("manifest written");
        let value: serde_json::Value = serde_json::from_str(&manifest_text).expect("json");
        let before = value["phases"]["fanout"]["before"]["board_statistics"]
            ["clearance_violations"]["total_count"]
            .as_i64()
            .expect("phase before-count present");
        assert_eq!(
            before, 2,
            "the STRICT gate drops the zero-shortfall edge-pin row at the load seed \
             (golden drc-0015 = 3 without the gate)"
        );
        let total = value["board_statistics"]["clearance_violations"]["total_count"]
            .as_i64()
            .expect("total count present");
        assert_eq!(total, 0, "the bounded route rips the two trace rows");

        let _ = std::fs::remove_dir_all(&out_dir);
    }
    /// SES written + parses, manifest fields asserted, exit 0.
    #[test]
    fn smoke_route_locator_fixture() {
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../harness/fixtures/locator-spike/t9_locator45.dsn"
        );
        let input = std::fs::read(fixture_path).expect("fixture present");
        let input_sha = sha256_hex(&input);
        let out_dir = std::env::temp_dir().join(format!("epic-t13-smoke-{}", std::process::id()));
        std::fs::create_dir_all(&out_dir).expect("temp dir");
        let ses_path = out_dir.join("t9_locator45.ses");
        let manifest_path = out_dir.join("t9_locator45.json");

        let argv = vec![
            "-de".to_string(),
            fixture_path.to_string(),
            "-do".to_string(),
            ses_path.to_string_lossy().into_owned(),
            "--result-json".to_string(),
            manifest_path.to_string_lossy().into_owned(),
            "--router.autorouter.max_passes=1".to_string(),
        ];
        let args = parse_route_args(&argv).expect("args parse");
        let exit = run_route(&args).expect("route run");

        assert_eq!(exit, 0, "one bounded pass on the fixture completes");
        let manifest_text = std::fs::read_to_string(&manifest_path).expect("manifest written");
        let value: serde_json::Value = serde_json::from_str(&manifest_text).expect("json");
        assert_eq!(value["final_state"], "COMPLETED");
        assert_eq!(value["exit_code"], 0);
        assert_eq!(value["output_written"], true);
        assert_eq!(
            value["fixture"]["sha256"], input_sha,
            "SHA of the INPUT bytes"
        );
        let passes = value
            .pointer("/phases/autorouter/passes_completed")
            .and_then(serde_json::Value::as_i64)
            .expect("one pass ran");
        assert!(passes >= 1);
        assert!(
            value["normalized_score"].as_f64().is_some(),
            "the router score face is present"
        );

        // The SES carries the routed wiring: the locator fixture is
        // UNROUTED on input, so any `(wire` + `(path` row in the session
        // is a routed trace (no SES reader exists in epic-dsn — the
        // M1b reader is DSN-only — so the assertion is on the emitted
        // text, matching the Java session shape).
        let ses_text = std::fs::read_to_string(&ses_path).expect("ses written");
        // F4 face: the job name carries NO extension — the stripped
        // name in BOTH the session header and the base_design row
        // (RoutingJob.java:519 -> SesWriter.write), and no ".dsn"
        // anywhere in the emitted document.
        assert!(
            ses_text.starts_with("(session \"t9_locator45\""),
            "session header carries the extension-less job name: {ses_text:.80}"
        );
        assert!(
            ses_text.contains("(base_design \"t9_locator45\")"),
            "base_design row carries the extension-less name"
        );
        assert!(!ses_text.contains(".dsn"), "no .dsn in the session");
        let routed_rows = ses_text.matches("(path ").count();
        assert!(
            routed_rows >= 1,
            "at least one routed (path ...) row, got {routed_rows} in:\n{ses_text}"
        );

        let _ = std::fs::remove_dir_all(&out_dir);
    }

    // -------------------------------------------------------------------
    // the quality-round pins (review Q1-Q7, Q10)
    // -------------------------------------------------------------------

    /// The no-signal-layer variant of the locator fixture: both
    /// `(type signal)` rows become `(type power)` carrying a
    /// `(use_net N001)` name. The parse succeeds, but the driver
    /// refuses to start ("Cannot start autorouter: all layers are
    /// disabled") and the run ends TERMINATED. Binary witness on
    /// `2b62b09bc`: exit 1 with a 4290-byte SES of the unrouted board
    /// on disk (Java leaves NO output — `Freerouting.java:265-267`).
    ///
    /// PARITY DECISION (2026-10-02, upstream #935 / a917044ff): the
    /// pre-#935 variant patched in BARE `(type power)` rows — planeless
    /// and nameless, which `promotePowerLayersWithoutPlane` (the P1
    /// port) now PROMOTES to signal, so the bare patch routes and
    /// COMPLETES instead of terminating. The `(use_net ...)` names keep
    /// both layers genuinely non-signal under upstream HEAD semantics
    /// (the promotion skips net-named layers, and
    /// `insertMissingPowerPlanes` synthesizes their full-bbox planes) —
    /// the post-#935 shape of "a board with nothing to route on".
    fn no_signal_layer_dsn() -> String {
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../harness/fixtures/locator-spike/t9_locator45.dsn"
        );
        let text = std::fs::read_to_string(fixture_path).expect("fixture present");
        let patched = text.replace(
            "      (type signal)\n",
            "      (type power)\n      (use_net N001)\n",
        );
        assert_eq!(
            patched.matches("(use_net N001)").count(),
            2,
            "both layer rows patched to named power layers"
        );
        patched
    }

    fn route_argv(
        dsn: &std::path::Path,
        ses: &std::path::Path,
        json: Option<&std::path::Path>,
    ) -> Vec<String> {
        let mut argv = vec![
            "-de".to_string(),
            dsn.to_string_lossy().into_owned(),
            "-do".to_string(),
            ses.to_string_lossy().into_owned(),
        ];
        if let Some(json) = json {
            argv.push("--result-json".to_string());
            argv.push(json.to_string_lossy().into_owned());
        }
        argv
    }

    /// Q1 (quality review): the session write is GATED on the job state
    /// (Java `writeCliOutputIfAvailable`, Freerouting.java:265-267) and
    /// the manifest carries the REAL write outcome
    /// (`RoutingResultManifest.java:191`). A TERMINATED run leaves no
    /// SES on disk and reports `output_written: false` / exit 1.
    /// Mutant faces: removing the gate re-creates the witnessed stray
    /// SES; hardcoding `output_written: true` in the renderer flips the
    /// manifest booleans.
    #[test]
    fn terminated_run_writes_no_ses_and_reports_output_unwritten() {
        let out_dir = std::env::temp_dir().join(format!("epic-t13-q1-{}", std::process::id()));
        std::fs::create_dir_all(&out_dir).expect("temp dir");
        let dsn_path = out_dir.join("nolayers.dsn");
        std::fs::write(&dsn_path, no_signal_layer_dsn()).expect("dsn written");
        let ses_path = out_dir.join("out.ses");
        let manifest_path = out_dir.join("out.json");

        let args = parse_route_args(&route_argv(&dsn_path, &ses_path, Some(&manifest_path)))
            .expect("args parse");
        let exit = run_route(&args).expect("the flow returns the mapped code, not an Err");

        assert_eq!(exit, 1, "no output on TERMINATED");
        assert!(
            !ses_path.exists(),
            "a TERMINATED run must leave NO session file (the Java gate)"
        );
        let manifest_text = std::fs::read_to_string(&manifest_path).expect("manifest written");
        let value: serde_json::Value = serde_json::from_str(&manifest_text).expect("json");
        assert_eq!(value["final_state"], "TERMINATED");
        assert_eq!(value["output_written"], false);
        assert_eq!(value["exit_code"], 1);

        let _ = std::fs::remove_dir_all(&out_dir);
    }

    /// Q3 (quality review): the pre-existing `-do` file is deleted
    /// right after the input load succeeds (Freerouting.java:122-127) —
    /// a failed RUN must not leave the PREVIOUS session in place
    /// masquerading as fresh output. The CONTRAST face pins the Java
    /// ORDERING: an input LOAD failure aborts BEFORE the delete
    /// (`Freerouting.java:114-118` returns before `:122`), so the stale
    /// file SURVIVES there.
    #[test]
    fn stale_output_deleted_after_input_load() {
        let out_dir = std::env::temp_dir().join(format!("epic-t13-q3-{}", std::process::id()));
        std::fs::create_dir_all(&out_dir).expect("temp dir");
        let dsn_path = out_dir.join("nolayers.dsn");
        std::fs::write(&dsn_path, no_signal_layer_dsn()).expect("dsn written");

        // World 1: the run gets past the load, then TERMINATES — the
        // stale session must be GONE.
        let ses_path = out_dir.join("out.ses");
        std::fs::write(&ses_path, "(session \"stale\")").expect("stale planted");
        let args = parse_route_args(&route_argv(&dsn_path, &ses_path, None)).expect("args parse");
        let exit = run_route(&args).expect("flow runs");
        assert_eq!(exit, 1);
        assert!(
            !ses_path.exists(),
            "the stale session is deleted before the run (a deleted-delete mutant keeps it)"
        );

        // World 2 (contrast): an UNREADABLE input aborts BEFORE the
        // delete — the stale file survives. (A mutant that moves the
        // delete ahead of the load fails here.)
        let dsn_missing = out_dir.join("does-not-exist.dsn");
        let ses_path2 = out_dir.join("stale2.ses");
        std::fs::write(&ses_path2, "(session \"stale\")").expect("stale planted");
        let args2 =
            parse_route_args(&route_argv(&dsn_missing, &ses_path2, None)).expect("args parse");
        assert!(
            run_route(&args2).is_err(),
            "an unreadable input is a hard error"
        );
        assert!(
            ses_path2.exists(),
            "load failure: Java keeps the stale file (delete comes AFTER the load)"
        );

        let _ = std::fs::remove_dir_all(&out_dir);
    }

    /// Q2 (quality review): a manifest-write failure is LOG-ONLY —
    /// Java computes `cliExitCode` before the manifest write and the
    /// IOException is swallowed (Freerouting.java:166-168, 367-374),
    /// so a completed run with an unwritable `--result-json` path
    /// still exits 0 with the SES on disk.
    #[test]
    fn manifest_write_failure_keeps_ses_exit_code() {
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../harness/fixtures/locator-spike/t9_locator45.dsn"
        );
        let out_dir = std::env::temp_dir().join(format!("epic-t13-q2-{}", std::process::id()));
        std::fs::create_dir_all(&out_dir).expect("temp dir");
        let ses_path = out_dir.join("out.ses");
        // The parent directory does not exist -> the manifest write fails.
        let manifest_path = out_dir.join("no-such-dir").join("out.json");

        let mut argv = route_argv(
            std::path::Path::new(fixture_path),
            &ses_path,
            Some(&manifest_path),
        );
        argv.push("--router.autorouter.max_passes=1".to_string());
        let args = parse_route_args(&argv).expect("args parse");
        let exit = run_route(&args).expect("the manifest failure is log-only, not an Err");

        assert_eq!(
            exit, 0,
            "the SES-derived exit code survives the manifest failure"
        );
        assert!(
            ses_path.exists(),
            "the session file is complete despite the manifest failure"
        );

        let _ = std::fs::remove_dir_all(&out_dir);
    }

    /// Q4 (quality review): the fanout -> `remove_unconnected_vias`
    /// wiring (the job ctor's `!isFanoutEnabled()`); fanout-off ALSO
    /// flips the batch tail sweep, so the coupling is
    /// double-load-bearing for T14. The pin asserts the REAL batch face
    /// through the same builder the flow uses (the `= false` mutant
    /// passed the whole suite before this pin existed).
    /// A two-signal-layer shell for the projection pins (the board
    /// build needs the layer structure + rules only).
    fn two_layer_ses() -> SesBoard {
        let mut ses = SesBoard::new();
        ses.create_board(CreateBoardIr {
            bounding_box: IntBox::new(IntPoint::new(0, 0), IntPoint::new(100_000, 60_000)),
            layer_structure: LayerStructure::new(vec![
                Layer::new("F.Cu", 0, true),
                Layer::new("B.Cu", 1, true),
            ]),
            outline_shapes: Vec::new(),
            outline_clearance_class: Some("default".to_string()),
            rules: BoardRulesIr::new(2),
            transform: CoordinateTransform::new(10.0, 0.0, 0.0),
        });
        ses
    }

    /// Q7b (quality review): the projection chain carries the fixed
    /// state through the round trip (ses -> Board ->
    /// `project_routed_items` -> ses) — the emitted session inherits
    /// the item's fixed state verbatim.
    #[test]
    fn projection_preserves_fixed_state() {
        let corners = [IntPoint::new(0, 0), IntPoint::new(10_000, 0)];
        let mut ses_in = two_layer_ses();
        ses_in.push_routed_item(ItemIr::Trace {
            id: 500,
            trace: TraceIr {
                layer_no: 0,
                half_width: 500,
                corners: corners.to_vec(),
                polyline: TraceIr::polyline_of_corners(&corners),
                nets: vec![1],
                clearance_class: 1,
                fixed: FixedStateIr::UserFixed,
            },
        });
        let board = Board::from_ses_board(&ses_in);
        let mut ses_out = two_layer_ses();
        project_routed_items(&board, &mut ses_out);

        let projected: Vec<(i32, FixedStateIr)> = ses_out
            .items
            .iter()
            .filter_map(|item| match item {
                ItemIr::Trace { id, trace } => Some((*id, trace.fixed)),
                _ => None,
            })
            .collect();
        assert_eq!(
            projected,
            vec![(500, FixedStateIr::UserFixed)],
            "id and fixed state survive the projection verbatim"
        );
    }
    /// PIN W-210 (buglog 210, the disposition witness): a PRE-ROUTED
    /// input emits ONE `(wire` per on-board trace — the Java oracle's
    /// own session on the same world shape carries one wire per net
    /// (the instrument, logs/M7-T5/evidence/): Java's
    /// `SesWriter.writeNet` walks the live board only. The regression
    /// face is the flow's own shape: the parse board retains the input
    /// `(wiring` items and the projection runs into THAT board.
    /// Block-walking sexpr (buglog 206: no single-line regex): the
    /// N1 scope is delimited by its `(net N1` opener and the balanced
    /// close, and every `(wire` block inside is counted.
    ///
    /// COUPLING DISCLOSURE (the AMENDMENT-5 §7b bank, discharged at the
    /// M7-T6 route.rs touch): this pin's walker parses the
    /// `write_session` OUTPUT TEXT and is therefore coupled to the
    /// writer's line layout — a `write_session` reformat (line breaks,
    /// indentation, sexpr folding) can break the walker WITHOUT any
    /// semantic change, and this pin failing is a REASONABLE response
    /// to that (the pin guards the emitted face's shape as well as its
    /// content); it does NOT guard `write_session` correctness in
    /// isolation.
    #[test]
    fn projection_supersedes_input_wiring_one_wire_per_on_board_trace() {
        use epic_dsn::sink::NetIr;
        use epic_geometry::polyline::Polyline;
        let mut ses_in = two_layer_ses();
        ses_in.nets = vec![
            NetIr {
                name: "N1".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 0,
            },
            NetIr {
                name: "N2".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 0,
            },
        ];
        // The input wiring: trace 500 (net N1) and via-less trace 501
        // (net N2) — a PRE-ROUTED design.
        let trace = |id: i32, net: i32, y: i32| ItemIr::Trace {
            id,
            trace: TraceIr {
                layer_no: 0,
                half_width: 500,
                corners: vec![IntPoint::new(0, y), IntPoint::new(40_000, y)],
                polyline: TraceIr::polyline_of_corners(&[
                    IntPoint::new(0, y),
                    IntPoint::new(40_000, y),
                ]),
                nets: vec![net],
                clearance_class: 1,
                fixed: FixedStateIr::Unfixed,
            },
        };
        ses_in.push_routed_item(trace(500, 1, 10_000));
        ses_in.push_routed_item(trace(501, 2, 20_000));
        // The live board, then a geometry change on trace 500 (the
        // meander stage's write face) — the pre-route copy the parse
        // board still holds is now STALE.
        let mut board = Board::from_ses_board(&ses_in);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        let wave: Vec<Point> = vec![
            Point::Int(IntPoint::new(0, 10_000)),
            Point::Int(IntPoint::new(0, 12_000)),
            Point::Int(IntPoint::new(4_000, 12_000)),
            Point::Int(IntPoint::new(4_000, 10_000)),
            Point::Int(IntPoint::new(40_000, 10_000)),
        ];
        board.replace_trace_geometry(&mut manager, ItemId::new(500), Polyline::from_points(&wave));
        // The flow's own shape: project into the parse board itself.
        project_routed_items(&board, &mut ses_in);
        let ses_text = write_session(&ses_in, "w210");
        // The walk (multi-line sexpr, scope-balanced — buglog 206): a
        // net scope opens with `(net <name>` and closes when the paren
        // depth (counted from the opener's line) returns to zero. Per
        // net: the `(wire` block count; for N1 also the path corner
        // lines (pure `x y` integer lines).
        let mut counts: Vec<(String, usize)> = Vec::new();
        let mut n1_path_tokens: Vec<String> = Vec::new();
        let mut current: Option<usize> = None;
        let mut depth = 0usize;
        for line in ses_text.lines() {
            let trimmed = line.trim();
            if current.is_none() {
                if let Some(rest) = trimmed.strip_prefix("(net ") {
                    let name = rest.trim().to_string();
                    counts.push((name, 0));
                    current = Some(counts.len() - 1);
                    depth = 1;
                }
                continue;
            }
            let index = current.expect("the pin world invariant");
            if trimmed == "(wire" {
                counts[index].1 += 1;
            }
            if !trimmed.contains('(') && !trimmed.contains(')') {
                let tokens: Vec<&str> = trimmed.split_whitespace().collect();
                if tokens.len() == 2
                    && tokens.iter().all(|t| t.parse::<i64>().is_ok())
                    && counts[index].0 == "N1"
                {
                    n1_path_tokens.push(trimmed.to_string());
                }
            }
            depth += trimmed.matches('(').count();
            depth -= trimmed.matches(')').count();
            if depth == 0 {
                current = None;
            }
        }
        assert_eq!(counts.len(), 2, "both nets emit their scope");
        assert_eq!(
            counts[0],
            ("N1".to_string(), 1),
            "one wire for the on-board trace — the stale pre-route copy is superseded"
        );
        assert_eq!(
            counts[1],
            ("N2".to_string(), 1),
            "net N2's unchanged trace emits once"
        );
        assert_eq!(
            n1_path_tokens,
            vec![
                "0 100000",
                "0 120000",
                "40000 120000",
                "40000 100000",
                "400000 100000"
            ],
            "the emitted N1 wire is the LIVE geometry (the wave), not the stale \
             pre-route (DSN space: the session transform's x10 um scale)"
        );
        // The board-level population: exactly two trace items survive.
        let trace_items = ses_in
            .items
            .iter()
            .filter(|item| matches!(item, ItemIr::Trace { .. }))
            .count();
        assert_eq!(trace_items, 2, "no parse copy rides next to the projection");
    }

    /// The M4-T8 jar-anchored END-TO-END score pin: the production
    /// flow THROUGH THE ROUTING STAGE (parse -> normalize -> load-time
    /// violation seed -> settings resolution -> fanout stage -> batch
    /// autorouter -> post-route statistics -> `get_optimizer_score`
    /// over the resolved V2 box; driven via `BatchDriver` directly, so
    /// the optimization stage that M4-T10 wired into `full::run` is
    /// deliberately OUT of this pin's world) on the bm08 tier fixture,
    /// against the jar's manifest face. The jar (e7f9bdf1, full flow)
    /// renders `optimizer_score: 823.79` — the TwoDecimalFloatAdapter
    /// view of the pre-optimizer board the optimizer RESTORED (pass
    /// regressed, incumbent kept), which is exactly the board this
    /// pin's post-routing endpoint equals (T10 note: the jar's
    /// restore is byte-exact, so the anchor still holds against the
    /// live pipeline too). Evidence: the 823.79 face is the full-flow
    /// manifest's (logs/M4-T8/evidence/jar-bounds/bm08-manifest.json);
    /// the exact component faces below are the optimizer-disabled
    /// manifest's (bm08-noopt-manifest.json); bounds/difficulty are
    /// pinned at the parse face in `board_statistics_bounds`.
    #[test]
    fn t8_bm08_full_flow_optimizer_score_matches_the_jar() {
        let dsn_path = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm08.dsn"
        ));
        let bytes = std::fs::read(dsn_path).expect("bm08 fixture present");
        let mut ses = SesBoard::new();
        match read_board(&bytes, &mut ses) {
            DsnReadResult::Success { .. } => {}
            other => panic!("expected Success, got {other:?}"),
        }
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        epic_board::normalize_all::normalize_all_traces(&mut manager, &mut board);
        let (pre_total, _) = all_clearance_violation_depths(&mut manager, &mut board);
        board.pre_existing_clearance_violations_count =
            i32::try_from(pre_total).unwrap_or(i32::MAX);

        let dsn_layer = DsnLayer::from_metadata(
            ses.metadata.autoroute_settings.as_ref(),
            usize::try_from(ses.metadata.layer_count).unwrap_or(0),
        );
        let mut merged = merge(
            &MergedSettings::default(),
            &dsn_layer,
            &epic_engine::settings::CliLayer::default(),
        );
        for warning in epic_engine::settings::validate(&mut merged) {
            eprintln!("Warning: {warning}");
        }
        apply_board_specific_optimizations(&mut merged, &board);
        let resolved = ResolvedRouteSettings::resolve(&merged, None);
        let batch = build_batch_settings(&resolved);

        let mut sink = CliDriverSink;
        let mut driver = BatchDriver::new(&mut manager, &mut board, batch, StopFace::default());
        let run_result = driver.run(&mut sink);
        drop(driver);
        assert!(
            matches!(run_result, Ok(true)),
            "bm08 must route to completion"
        );

        let stats = BoardStatistics::new(&mut manager, &mut board);
        let score = stats.get_optimizer_score(Some(&resolved.scoring));
        assert!(
            score > 0.0 && score < 1000.0,
            "the routed bm08 board must sit INSIDE the score range (all three penalties live): {score}"
        );

        // ANCHOR — each face cited to its own capture (spec-review
        // MINOR-2): the composed score 823.79 is the FULL-FLOW
        // manifest's face ONLY (jar-bounds/bm08-manifest.json, where
        // the optimizer ran and restored the incumbent, proven
        // byte-exact: bm08.ses == bm08-noopt.ses == 03e3260a...); the
        // component faces 94.96 mm / 1 via / 21 bends come from the
        // OPTIMIZER-DISABLED manifest (jar-bounds/bm08-noopt-manifest.json),
        // which carries no TOP-LEVEL/final optimizer_score key (it DOES
        // carry per-phase scores — phases.autorouter.before.optimizer_score
        // 925.0 and .after 823.79 — so a grep hits; the ANCHOR for the
        // composed score is the full-flow manifest only). The Rust route is
        // NOT byte-equal
        // to the jar's on this fanout-ON profile (Rust ses ec22a0f2...,
        // the T7-recorded self-determinism face) — the routed-geometry
        // drift is 0.04% of trace length, pre-T8, and exactly the
        // class the router-compare's RELATIVE score policy tolerates
        // (score >= J - 2%*J). The pin therefore asserts that policy
        // floor, the exact count faces, and the bounded length face.
        assert!(
            score >= 823.79_f32 - 0.02 * 823.79_f32,
            "the relative-score policy floor (jar 823.79 - 2%): {score}"
        );
        // The count components are exact on both sides.
        assert_eq!(stats.vias.total_count, 1, "jar: vias 1");
        // M11-T4 rotation (buglog 251/252, #931 cluster F): the
        // unconditional corner-touch door inserts opened diagonal
        // passages on this board — the route is ~2.5 mm SHORTER with
        // two more bends (92.43 mm / 23 bends vs the jar's 94.96 /
        // 21), and the composed optimizer score RISES above the
        // jar's own face (840.93 vs 823.79 — the relative floor
        // above still binds, and it is the policy face). The jar
        // anchors stay the committed bm08-noopt manifest; these two
        // literals follow the routed T4 face.
        assert_eq!(
            stats.bends.total_count, 23,
            "the T4 diagonal-passage face (jar anchors 21)"
        );
        assert_eq!(
            stats.difficulty.difficulty_d,
            Some(80.0),
            "jar: difficulty D 80"
        );
        // The length face: the T4 diagonal passages sit the route
        // ~2.5 mm UNDER the jar face (94.96) — bound the ROUTED
        // face at 0.1 mm.
        let length_mm = stats.traces.total_length_mm.expect("length present");
        assert!(
            (f64::from(length_mm) - 92.43).abs() <= 0.1,
            "routed trace length within 0.1 mm of the T4 face: {length_mm}"
        );
        // The score's lower-bound input on this same board is pinned
        // raw-bits-exact at the parse face (bounds module, bm08 pins).
        assert_eq!(
            stats.bounds.min_bend_count,
            Some(15),
            "the routed board keeps the load-time bounds capture"
        );
    }
    // -------------------------------------------------------------------
    // The copper-to-edge override pins (M5-T3, buglog 189). The measured
    // band edges (cerebrum 16): the class-1 outline leaf's north edge is
    // -911136 BEFORE the override (the T12 direct-read face) and
    // -911636 AFTER (the Java CLI face); both pinned as LITERALS at the
    // exact-boundary world, with the +/-1 faces mutation-verified — the
    // durable mutation record lives in SEAM.md's T3 dossier and buglog
    // 189. The witness drives the REAL 45-degree
    // completion (epic-index complete_shape) over the REAL class-1
    // compensated outline shapes with T2's exact fork-row input.
    // -------------------------------------------------------------------

    /// The bm06-faithful minimal world — bm06's OWN boundary path
    /// (scripts/benchmark/fixtures/DAC2020_boards/DAC2020_bm06.dsn
    /// lines 23-25), so the parse stays at scale factor 10 (bm06's
    /// raw coordinates are far under the `calc_scale_factor` overflow
    /// shrink, 5 x |coor| x resolution vs CRIT_INT 2^25) and the north
    /// edge centerline lands at exactly -910036 DBU. `(rule
    /// (clearance 200))` -> v(1,1) = 2000; the outline half width is
    /// the `outline::half_width()` constant 100; no DSN outline
    /// clearance class -> the outline sits at the fallback AREA class,
    /// so the CLI override fires at the 250 um default.
    const COPPER_WORLD_DSN: &str = r#"(pcb t3-copper-world.dsn
  (parser
    (string_quote ')
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (rule (clearance 200))
    (boundary
      (path pcb 0  176001 -119004  121001 -119004  121001 -91003.6  176001 -91003.6
            176001 -119004)
    )
  )
)
"#;

    /// The skip-gate contrast world: SAME geometry, plus an EXPLICIT
    /// boundary `(clearance_class edge)` — the outline leaves the
    /// fallback class, so the default-value override must NOT fire.
    const COPPER_WORLD_EXPLICIT_EDGE_DSN: &str = r#"(pcb t3-copper-world-edge.dsn
  (parser
    (string_quote ')
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (rule (clearance 200))
    (boundary
      (path pcb 0  176001 -119004  121001 -119004  121001 -91003.6  176001 -91003.6
            176001 -119004)
      (clearance_class edge)
    )
  )
)
"#;

    /// The #935 near-pin world — COPPER_WORLD's own boundary plus ONE
    /// round pad near the north edge: padstack circle diameter 400 um
    /// (radius 200), the pin center placed at -91253.6 (250 um below
    /// the -91003.6 north centerline), so the pad's bounding-box top
    /// corners land at -91053.6 — 50 um / 500 DBU from the edge line,
    /// inside the outline. Every other corner measures >= 4500 DBU.
    /// `outline_minimum_pin_gap` is therefore exactly 500 DBU, a
    /// fifth of the 2500 DBU default — the cap world for the a917044ff
    /// port.
    const COPPER_WORLD_NEAR_PIN_DSN: &str = r#"(pcb t3-copper-world-nearpin.dsn
  (parser
    (string_quote ')
    (space_in_quoted_tokens on)
  )
  (resolution um 10)
  (unit um)
  (structure
    (layer F.Cu (type signal))
    (layer B.Cu (type signal))
    (rule (clearance 200))
    (boundary
      (path pcb 0  176001 -119004  121001 -119004  121001 -91003.6  176001 -91003.6
            176001 -119004)
    )
  )
  (placement
    (component U1
      (place U1 140000 -91253.6 front 0)
    )
  )
  (library
    (padstack PAD_NEAR
      (shape (circle F.Cu 400 0 0))
    )
    (image U1
      (pin PAD_NEAR 1 0 0)
    )
  )
  (network
    (net N1 (pins U1-1))
    (class DEF N1
      (rule (width 200))
    )
  )
)
"#;

    /// Parses a copper world into the post-load board + manager +
    /// outline id (the CLI load walk minus the override: read_board ->
    /// from_ses_board -> reinsert_tree_items -> normalize).
    fn copper_world_from(dsn: &'static str) -> (SearchTreeManager, Board, ItemId) {
        let mut ses = SesBoard::new();
        match read_board(dsn.as_bytes(), &mut ses) {
            DsnReadResult::Success { .. } => {}
            other => panic!("copper world parse failed: {other:?}"),
        }
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        epic_board::normalize_all::normalize_all_traces(&mut manager, &mut board);
        let outline = board
            .iter_descending()
            .find(|entry| matches!(entry.data, ItemData::BoardOutline { .. }))
            .map(|entry| entry.id)
            .expect("the world carries an outline");
        (manager, board, outline)
    }

    fn copper_world() -> (SearchTreeManager, Board, ItemId) {
        copper_world_from(COPPER_WORLD_DSN)
    }

    /// The class-1 45-degree compensated outline tiles (the shapes the
    /// autoroute tree inserts and the completion restrains against),
    /// one per layer.
    fn class1_outline_tiles(board: &mut Board, outline: ItemId) -> Vec<Option<TileShape>> {
        epic_board::tree_shapes::item_tree_shapes(
            board,
            epic_index::search_tree::SearchTreeVariant::FortyfiveDegree,
            1,
            outline,
        )
    }

    /// The outline's class-1 NORTH leaf face: the inner (south) face
    /// of the north-edge segment's tile = max over tiles of ll.y (y
    /// grows upward; the north tile's lower face is the room's north
    /// restraint). This is the quantity buglog 189 measured: -911136
    /// pre-override, -911636 post.
    fn north_leaf_face(board: &mut Board, outline: ItemId) -> i32 {
        class1_outline_tiles(board, outline)
            .iter()
            .map(|tile| {
                tile.clone()
                    .expect("a tile per segment")
                    .bounding_box()
                    .ll
                    .y
            })
            .max()
            .expect("the outline carries tiles")
    }

    /// The completion objects view for the witness — the PRODUCTION
    /// accessors (`EngineShapeView`'s item arm): the outline is a
    /// trace obstacle for every net (its nets list is empty), the
    /// shapes/layers come from the board's per-tree shape cache the
    /// `get_autoroute_tree` insert filled.
    struct WitnessObjects<'a>(&'a Board, u64);
    impl epic_index::CompleteShapeObjects for WitnessObjects<'_> {
        // HARDCODED, and coincides with the production face for THIS
        // world only: the pinned outline is net-less, and production
        // (`EngineShapeView` -> `neighbours.rs` -> `Board::
        // is_trace_obstacle_read`, board.rs) answers `true` for a
        // net-less outline against every net. Do NOT extend this arm to
        // worlds with net-carrying obstacles — call the production fn
        // there.
        fn is_trace_obstacle(&self, _object_key: u64, _net_number: i32) -> bool {
            true
        }
        fn shape_layer(&self, object_key: u64, shape_index: u32) -> i32 {
            let id = ItemId::new(u32::try_from(object_key).expect("item key fits u32"));
            self.0
                .item_shape_layer_read(id, shape_index as i32)
                .expect("live tree shapes carry a layer")
        }
        fn tree_shape(&self, object_key: u64, shape_index: u32) -> Option<TileShape> {
            let id = ItemId::new(u32::try_from(object_key).expect("item key fits u32"));
            self.0
                .tree_shape_precalc_peek(id, self.1)
                .and_then(|shapes| shapes.get(shape_index as usize).cloned().flatten())
        }
        fn is_complete_free_space(&self, _object_key: u64) -> bool {
            false
        }
    }

    /// The north-face runner: builds the class-1 45-degree tree the way
    /// the engine's lazy build does (get_autoroute_tree), then drives
    /// the REAL completion with T2's exact fork-row input and returns
    /// the north face (max uy over the completed rooms).
    fn completion_north_face(manager: &mut SearchTreeManager, board: &mut Board) -> i32 {
        let tree_index = manager.get_autoroute_tree(board, 1);
        let tree_object_id = manager.trees()[tree_index].object_id();
        let objects = WitnessObjects(board, tree_object_id);
        let tree = &manager.trees()[tree_index];
        // T2's fork-row input (logs/M5-T2/evidence/
        // instrument_agent_rows_run1.log row 2): the first fanout
        // search's start-room completion, pin U9-1, net 21, layer 0.
        let oct_tile = |o: epic_geometry::int_octagon::IntOctagon| {
            TileShape::RegularTileShape(
                epic_geometry::regular_tile_shape::RegularTileShape::IntOctagon(o),
            )
        };
        let in_shape = oct_tile(TileShape::from_8_ints(
            -33_554_432,
            -1_019_136,
            1_376_811,
            33_554_432,
            -33_554_432,
            2_395_947,
            -33_554_432,
            33_554_432,
        ));
        let contained = oct_tile(TileShape::from_8_ints(
            1_371_811, -1_016_636, 1_371_811, -1_016_636, 2_388_447, 2_388_447, 355_175, 355_175,
        ));
        let query = epic_index::CompleteShapeQuery {
            room_shape: Some(&in_shape),
            contained: Some(&contained),
            layer: 0,
            net_number: 21,
            ignore_object: None,
            ignore_shape: None,
        };
        let bbox = board.bounding_box().expect("the world has a bbox");
        let rooms = epic_index::complete_shape::complete_shape(tree, &objects, &query, &bbox);
        assert!(!rooms.is_empty(), "the completion returned rooms");
        rooms
            .iter()
            .map(|room| room.shape.bounding_box().ur.y)
            .max()
            .expect("rooms non-empty")
    }

    /// PIN (the band's SOUTH edge — the pre-state): without the
    /// override the class-1 outline leaf's north face is -911136 — the
    /// exact T12 direct-read / pre-fix Rust face.
    #[test]
    fn band_edge_before_override_is_the_direct_read_face_911136() {
        let (mut manager, mut board, outline) = copper_world();
        // World self-check: the fallback AREA class carries the outline
        // and the matrix diagonal is bm06's 2000.
        let area = board.rules().default_item_clearance_classes
            [epic_dsn::sink::ItemClassIr::Area as usize];
        assert_eq!(
            board.item_clearance_class(outline),
            Some(area),
            "fallback outline class"
        );
        assert_eq!(
            board.rules().clearance.get_value(1, 1, 0),
            2000,
            "v(1,1) = 200 um x 10"
        );
        let leaf = north_leaf_face(&mut board, outline);
        assert_eq!(
            leaf, -911_136,
            "pre-override north edge (growth 100 + (2000 - 1000) = 1100)"
        );
        // The completion consumes the same state: north face -911136.
        let face = completion_north_face(&mut manager, &mut board);
        assert_eq!(face, -911_136, "the pre-state completion face");
    }

    /// PIN (the band's NORTH edge — the fixed state): after the
    /// override the outline is promoted to `board_edge` at 2500 DBU,
    /// the class-1 leaf's north face is -911636 — the exact Java CLI
    /// face (T2 LFDUMP), and the completion delivers it.
    #[test]
    fn band_edge_after_override_is_the_cli_face_911636() {
        let (mut manager, mut board, outline) = copper_world();
        let merged = MergedSettings::default();
        apply_copper_to_edge_clearance_override(&merged, &mut manager, &mut board);
        let edge = board.rules().clearance.get_no("board_edge");
        assert!(edge > 0, "the board_edge class was appended");
        assert_eq!(
            board.item_clearance_class(outline),
            Some(edge),
            "outline promoted"
        );
        // The symmetric full set: every class >= 1 on both directions,
        // 2500; class 0 stays at the append init v(1,0).
        let classes = board.rules().clearance.class_count() as i32;
        for class_no in 1..classes {
            assert_eq!(
                board.rules().clearance.get_value(edge, class_no, 0),
                2500,
                "v(edge, {class_no})"
            );
            assert_eq!(
                board.rules().clearance.get_value(class_no, edge, 0),
                2500,
                "v({class_no}, edge)"
            );
        }
        assert_eq!(
            board.rules().clearance.get_value(edge, 0, 0),
            board.rules().clearance.get_value(1, 0, 0),
            "class 0 untouched (the append init v(1,0))"
        );
        let leaf = north_leaf_face(&mut board, outline);
        assert_eq!(
            leaf, -911_636,
            "post-override north edge (growth 100 + (2500 - 1000) = 1600)"
        );
        let face = completion_north_face(&mut manager, &mut board);
        assert_eq!(
            face, -911_636,
            "the CLI completion face at the first fanout search"
        );
    }

    /// PIN (the even lattice): the matrix's even-rounding absorbs the
    /// -1 DBU face — applying 249.9 um (2499 DBU) stores 2500 and the
    /// boundary value is UNCHANGED. This is the mutation-verified
    /// "minus" direction's honest face: through the value knob the
    /// smallest DOWN step Java can express is -2 DBU (the next even
    /// lattice point).
    #[test]
    fn the_minus_one_value_face_is_absorbed_by_the_even_lattice() {
        let (mut manager, mut board, outline) = copper_world();
        let merged = MergedSettings {
            copper_to_edge_clearance_um: Some(249.9),
            ..MergedSettings::default()
        };
        apply_copper_to_edge_clearance_override(&merged, &mut manager, &mut board);
        let leaf = north_leaf_face(&mut board, outline);
        assert_eq!(
            leaf, -911_636,
            "2499 DBU stores 2500 (odd rounds up) — the boundary holds"
        );
    }

    /// PIN (the #935 default-value cap, a917044ff): a pin whose pad
    /// sits 500 DBU from the outline caps the DEFAULT 250 um at
    /// `floor(500) = 500` — every `board_edge` cell on EVERY layer
    /// drops from 2500 to 500, both directions — because the default
    /// is only a guess and must never exceed the gap the input design
    /// already has. The pinless worlds above keep 2500:
    /// `outline_minimum_pin_gap` answers +INF there (the no-pin arm),
    /// so their 2500 literals are the cap-inert contrast.
    #[test]
    fn default_edge_clearance_is_capped_by_the_pin_to_outline_gap() {
        let (mut manager, mut board, outline) = copper_world_from(COPPER_WORLD_NEAR_PIN_DSN);
        assert_eq!(
            board.outline_minimum_pin_gap(outline),
            500.0,
            "the pad's bbox top corners are 500 DBU south of the north edge"
        );
        let merged = MergedSettings::default();
        apply_copper_to_edge_clearance_override(&merged, &mut manager, &mut board);
        let edge = board.rules().clearance.get_no("board_edge");
        assert!(edge > 0, "the board_edge class was appended");
        let classes = board.rules().clearance.class_count() as i32;
        let layers = board.rules().clearance.layer_count() as i32;
        for layer in 0..layers {
            for class_no in 1..classes {
                assert_eq!(
                    board.rules().clearance.get_value(edge, class_no, layer),
                    500,
                    "v(board_edge, {class_no}, {layer}) = floor(min_pin_gap)"
                );
                assert_eq!(
                    board.rules().clearance.get_value(class_no, edge, layer),
                    500,
                    "v({class_no}, board_edge, {layer}) = floor(min_pin_gap)"
                );
            }
        }
        // The cap-inert contrast: the PINLESS world answers +INF and
        // keeps the full 2500 default (pinned above as -911_636).
        let (_manager2, mut board2, outline2) = copper_world();
        assert!(
            board2.outline_minimum_pin_gap(outline2).is_infinite(),
            "no pins -> +INF, the cap cannot fire"
        );
    }

    /// PIN (the #935 cap gate): the cap rides the SAME predicate as
    /// the skip-gate — `usesDefaultEdgeClearanceValue` — so an
    /// EXPLICIT 400 um value applies UNCAPPED (4000 DBU) on the very
    /// world where the default was capped to 500. The user's explicit
    /// number is theirs; only the guess gets second-guessed.
    #[test]
    fn explicit_edge_clearance_is_not_capped_by_the_pin_to_outline_gap() {
        let (mut manager, mut board, outline) = copper_world_from(COPPER_WORLD_NEAR_PIN_DSN);
        assert_eq!(
            board.outline_minimum_pin_gap(outline),
            500.0,
            "same near-pin world"
        );
        let merged = MergedSettings {
            copper_to_edge_clearance_um: Some(400.0),
            ..MergedSettings::default()
        };
        apply_copper_to_edge_clearance_override(&merged, &mut manager, &mut board);
        let edge = board.rules().clearance.get_no("board_edge");
        assert!(edge > 0, "the board_edge class was appended");
        let classes = board.rules().clearance.class_count() as i32;
        let layers = board.rules().clearance.layer_count() as i32;
        for layer in 0..layers {
            for class_no in 1..classes {
                assert_eq!(
                    board.rules().clearance.get_value(edge, class_no, layer),
                    4000,
                    "explicit 400 um -> 4000 DBU, uncapped"
                );
            }
        }
    }

    /// PIN (the skip gate, contrast pair — DNR mode 7): the EXPLICIT
    /// boundary class at the default value is UNTOUCHED; the SAME world
    /// at a non-default value (300 um) IS promoted. Both arms assert
    /// the class AND the leaf, so an inverted or literal-gate mutant
    /// trips at least one arm.
    #[test]
    fn skip_gate_explicit_class_contrast() {
        // Arm 1 (untouched face): explicit class + default value.
        let (mut manager, mut board, outline) = copper_world_from(COPPER_WORLD_EXPLICIT_EDGE_DSN);
        let merged = MergedSettings::default();
        let before = board.item_clearance_class(outline);
        let leaf_before = north_leaf_face(&mut board, outline);
        apply_copper_to_edge_clearance_override(&merged, &mut manager, &mut board);
        assert_eq!(
            board.item_clearance_class(outline),
            before,
            "explicit DSN outline class untouched at the default value"
        );
        assert_eq!(
            board.rules().clearance.get_no("board_edge"),
            -1,
            "no class appended"
        );
        let leaf_after = north_leaf_face(&mut board, outline);
        assert_eq!(leaf_after, leaf_before, "leaf unmoved");

        // Arm 2 (fires face): explicit class + NON-default value.
        let (mut manager, mut board, outline) = copper_world_from(COPPER_WORLD_EXPLICIT_EDGE_DSN);
        let merged = MergedSettings {
            copper_to_edge_clearance_um: Some(300.0),
            ..MergedSettings::default()
        };
        apply_copper_to_edge_clearance_override(&merged, &mut manager, &mut board);
        assert_eq!(
            board.item_clearance_class(outline),
            Some(board.rules().clearance.get_no("board_edge")),
            "non-default value promotes even an explicit-class outline"
        );
    }

    /// PIN (the negative-um guard): -1 um warns and skips — the outline
    /// stays at the fallback class and no class is appended.
    #[test]
    fn negative_um_guard_skips() {
        let (mut manager, mut board, outline) = copper_world();
        let merged = MergedSettings {
            copper_to_edge_clearance_um: Some(-1.0),
            ..MergedSettings::default()
        };
        let before = board.item_clearance_class(outline);
        apply_copper_to_edge_clearance_override(&merged, &mut manager, &mut board);
        assert_eq!(
            board.item_clearance_class(outline),
            before,
            "negative skips"
        );
        assert_eq!(
            board.rules().clearance.get_no("board_edge"),
            -1,
            "no class appended"
        );
    }

    /// The M6-T6 advisory pour-islands face: ABSENT for a pour-free
    /// board (the schema-evolution face — pour-free manifests keep
    /// their bytes), PRESENT with the count+digest rows when pours
    /// exist.
    #[test]
    fn manifest_pour_islands_face_absent_without_pours_present_with() {
        let telemetry = RouteTelemetry {
            final_state: "COMPLETED".to_string(),
            pour_islands: vec![ManifestPourIslands {
                item_id: 17,
                net: "GND".to_string(),
                layer: 1,
                region_count: 2,
                island_count: 1,
                digest: "a".repeat(64),
            }],
            ..RouteTelemetry::default()
        };
        let json = render_manifest(&telemetry, "0.1.0", "deadbeef", fixture("b.dsn", "aa"));
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        let row = value.pointer("/pour_islands/0").expect("the face rides");
        assert_eq!(
            row.pointer("/net").and_then(serde_json::Value::as_str),
            Some("GND")
        );
        assert_eq!(
            row.pointer("/region_count")
                .and_then(serde_json::Value::as_u64),
            Some(2)
        );
        assert_eq!(
            row.pointer("/island_count")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert_eq!(
            row.pointer("/digest")
                .and_then(serde_json::Value::as_str)
                .map(str::len),
            Some(64)
        );

        // Pour-free: the key is ABSENT (the byte-invariance face).
        let telemetry = RouteTelemetry {
            final_state: "COMPLETED".to_string(),
            ..RouteTelemetry::default()
        };
        let json = render_manifest(&telemetry, "0.1.0", "deadbeef", fixture("b.dsn", "aa"));
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert!(
            value.pointer("/pour_islands").is_none(),
            "pour-free manifests keep their bytes: the key must not ride"
        );
    }

    // ---- M7-T3: the meander-need report face (T4's input contract) ----

    /// A SesBoard world for the report pins: nets slow(1)/free(2)/
    /// long(3)/short(4); class 1 "tuned" carries (min, max), class 0
    /// "default" is unconstrained. Traces: net 1 = 60000 (in range),
    /// net 2 = 10000 (unconstrained), net 3 = 150000, net 4 = 30000.
    fn length_report_board(min: f64, max: f64) -> Board {
        use epic_dsn::sink::NetClassIr;
        let class = NetClassIr {
            name: "tuned".to_string(),
            trace_clearance_class: 1,
            trace_half_widths: vec![1500],
            active_routing_layers: vec![true],
            default_item_clearance_classes: [0, 1, 1, 1, 1, 1],
            via_rule: None,
            pull_tight: true,
            shove_fixed: false,
            min_trace_length: min,
            max_trace_length: max,
            nets: Vec::new(),
        };
        let mut ses = two_layer_ses();
        ses.net_classes.push(class);
        ses.nets = vec![
            epic_dsn::sink::NetIr {
                name: "slow".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 1,
            },
            epic_dsn::sink::NetIr {
                name: "free".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 0,
            },
            epic_dsn::sink::NetIr {
                name: "long".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 1,
            },
            epic_dsn::sink::NetIr {
                name: "short".to_string(),
                subnet_number: 1,
                contains_plane: false,
                net_class: 1,
            },
        ];
        let trace = |id: i32, net: i32, x: i32| ItemIr::Trace {
            id,
            trace: TraceIr {
                layer_no: 0,
                half_width: 500,
                corners: vec![IntPoint::new(0, 0), IntPoint::new(x, 0)],
                polyline: TraceIr::polyline_of_corners(&[IntPoint::new(0, 0), IntPoint::new(x, 0)]),
                nets: vec![net],
                clearance_class: 1,
                fixed: FixedStateIr::Unfixed,
            },
        };
        ses.push_routed_item(trace(500, 1, 60_000));
        ses.push_routed_item(trace(501, 2, 10_000));
        ses.push_routed_item(trace(502, 3, 150_000));
        ses.push_routed_item(trace(503, 4, 30_000));
        Board::from_ses_board(&ses)
    }

    /// The report contract: only constrained nets whose routed length
    /// violates the resolved bounds get rows, in NET-NUMBER order, with
    /// the signed violation (positive = over-max excess, negative =
    /// under-min deficit; the pinless world guarantees no incompletes,
    /// so the under-min arm fires). In-range and unconstrained nets
    /// stay silent. Values derive from the world: slow 60000 in
    /// [50000, 100000] -> no row; free 10000 unconstrained -> no row;
    /// long 150000 > max 100000 -> +50000; short 30000 < min 50000 ->
    /// -20000.
    #[test]
    fn length_needs_reports_violating_constrained_nets_in_number_order() {
        let mut board = length_report_board(50_000.0, 100_000.0);
        board.set_tuning_active(true);
        let manager = SearchTreeManager::new();
        let rows = length_needs(&manager, &mut board);
        assert_eq!(rows.len(), 2, "long + short only");
        assert_eq!(rows[0].net_number, 3);
        assert_eq!(rows[0].net_name, "long");
        assert_eq!(rows[0].min_length, 50_000.0);
        assert_eq!(rows[0].max_length, 100_000.0);
        assert_eq!(rows[0].trace_length, 150_000.0);
        assert_eq!(rows[0].violation, 50_000.0, "positive = over-max excess");
        assert_eq!(rows[1].net_number, 4);
        assert_eq!(rows[1].net_name, "short");
        assert_eq!(rows[1].trace_length, 30_000.0);
        assert_eq!(rows[1].violation, -20_000.0, "negative = under-min deficit");
    }

    /// The zero-rotation negative: a CONSTRAINT-FREE board (all class
    /// bounds 0.0) yields no rows even with the tuning flag on, and a
    /// fresh board reads tuning_active() = false (the parity regime
    /// default). The manifest face: the empty report leaves NO key in
    /// the rendered JSON (the byte-identity face).
    #[test]
    fn length_report_absent_on_constraint_free_board() {
        let mut board = length_report_board(0.0, 0.0);
        board.set_tuning_active(true);
        let manager = SearchTreeManager::new();
        assert!(
            length_needs(&manager, &mut board).is_empty(),
            "constraint-free: nothing to report"
        );
        let fresh = Board::default();
        assert!(!fresh.tuning_active(), "the parity-regime default");
        let telemetry = RouteTelemetry {
            final_state: "COMPLETED".to_string(),
            ..RouteTelemetry::default()
        };
        let json = render_manifest(&telemetry, "0.1.0", "deadbeef", fixture("b.dsn", "aa"));
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert!(
            value.get("length_report").is_none(),
            "no violations = no key = the zero-rotation face"
        );
    }

    /// The manifest row shape (machine-readable, T4's input): the
    /// rendered /length_report rows carry every field verbatim.
    #[test]
    fn manifest_length_report_rows_render_machine_readably() {
        let telemetry = RouteTelemetry {
            final_state: "COMPLETED".to_string(),
            length_report: vec![ManifestLengthNeed {
                net_number: 7,
                net_name: "clk".to_string(),
                min_length: 100.5,
                max_length: 0.0,
                trace_length: 40.25,
                violation: -60.25,
            }],
            ..RouteTelemetry::default()
        };
        let json = render_manifest(&telemetry, "0.1.0", "deadbeef", fixture("b.dsn", "aa"));
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        let row = value.pointer("/length_report/0").expect("the face rides");
        assert_eq!(
            row.pointer("/net_number")
                .and_then(serde_json::Value::as_i64),
            Some(7)
        );
        assert_eq!(
            row.pointer("/net_name").and_then(serde_json::Value::as_str),
            Some("clk")
        );
        assert_eq!(
            row.pointer("/min_length")
                .and_then(serde_json::Value::as_f64),
            Some(100.5)
        );
        assert_eq!(
            row.pointer("/max_length")
                .and_then(serde_json::Value::as_f64),
            Some(0.0)
        );
        assert_eq!(
            row.pointer("/trace_length")
                .and_then(serde_json::Value::as_f64),
            Some(40.25)
        );
        assert_eq!(
            row.pointer("/violation")
                .and_then(serde_json::Value::as_f64),
            Some(-60.25)
        );
    }

    /// The FIRST committed tuning fixture's golden face (`rust/harness/
    /// fixtures/tuning/min_stair_tuning.dsn` — the corpus's first
    /// input-driven-activation fixture): parse -> deliver -> activate ->
    /// report, all from the COMMITTED file. The N1 class declares
    /// `(circuit (length -1 50000))` = min 500000 board DBU (x10); the
    /// pre-routed N1 staircase is 480000, so `apply_tuning_activation`
    /// must resolve the flag ON from the declaration alone and the
    /// report must carry the honest under-min deficit row (derived:
    /// 480000 - 500000 = -20000). N2 (class kicad_default, no
    /// declaration) never gets a row.
    #[test]
    fn tuning_fixture_golden_face_activation_and_report_row() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../rust/harness/fixtures/tuning/min_stair_tuning.dsn");
        let bytes = std::fs::read(&path).expect("committed fixture");
        let mut ses = SesBoard::new();
        match read_board(&bytes, &mut ses) {
            DsnReadResult::Success { .. } => {}
            other => panic!("parse {other:?}"),
        }
        let mut board = Board::from_ses_board(&ses);
        // The flow's board prefix (route.rs step 2): tree + normalize.
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        epic_board::normalize_all::normalize_all_traces(&mut manager, &mut board);
        assert!(
            board.rules().has_length_constraints(),
            "the committed fixture declares a length rule"
        );
        let resolved = ResolvedRouteSettings::resolve(&MergedSettings::default(), None);
        let mut batch = build_batch_settings(&resolved);
        apply_tuning_activation(&mut batch, &resolved, board.rules());
        assert!(batch.tuning_active, "the declaration activates (None)");
        board.set_tuning_active(batch.tuning_active);
        let rows = length_needs(&manager, &mut board);
        assert_eq!(rows.len(), 1, "N1 only");
        let row = &rows[0];
        assert_eq!(row.net_number, 1);
        assert_eq!(row.net_name, "N1");
        assert_eq!(row.min_length, 500_000.0);
        assert_eq!(row.max_length, 0.0);
        assert_eq!(row.trace_length, 480_000.0);
        assert_eq!(row.violation, -20_000.0);
    }

    // ------------------------------------------------------------------
    // M7-T4: the meander activation + the fixture golden faces
    // ------------------------------------------------------------------

    /// The committed tuning fixtures' route through the real flow
    /// (the in-tree golden faces; the DSN files are the committed
    /// capture, the pins re-run it in-process).
    fn run_tuning_fixture(
        tag: &str,
        name: &str,
        extra: &[&str],
    ) -> (i32, std::path::PathBuf, std::path::PathBuf) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../harness/fixtures/tuning/")
            .join(format!("{name}.dsn"));
        let tmp =
            std::env::temp_dir().join(format!("m7t4_{}_{}_{}", name, tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).expect("the pin world invariant");
        let ses_path = tmp.join("out.ses");
        let manifest_path = tmp.join("manifest.json");
        let argv = vec![
            "route".to_string(),
            "-de".to_string(),
            path.to_string_lossy().into_owned(),
            "-do".to_string(),
            ses_path.to_string_lossy().into_owned(),
            "--result-json".to_string(),
            manifest_path.to_string_lossy().into_owned(),
        ];
        let mut argv: Vec<String> = argv;
        argv.extend(extra.iter().map(|s| s.to_string()));
        let args = parse_route_args(&argv).expect("args parse");
        let exit = run_route(&args).expect("route run");
        (exit, ses_path, manifest_path)
    }

    /// PIN F3 (the mixed-invariance world, criterion 3): the
    /// constraint-free net's emitted wiring is BYTE-IDENTICAL between
    /// the tuning-ON and tuning-OFF runs of the SAME board, while the
    /// two tuned nets get their deficits filled in flow (the ON run's
    /// length_report is EMPTY — violation 0 — and the OFF run keeps
    /// both honest rows). Determinism x2 (two ON runs byte-identical)
    /// and the -mt 1/3 threads witness ride the same fixture.
    #[test]
    fn meander_multi_fixture_invariance_and_flow() {
        let (exit_on, ses_on, man_on) = run_tuning_fixture("on1", "meander_multi_tuning", &[]);
        assert_eq!(exit_on, 0);
        let (exit_on2, ses_on2, man_on2) = run_tuning_fixture("on2", "meander_multi_tuning", &[]);
        assert_eq!(exit_on2, 0);
        let (exit_gate, ses_gate, man_gate) = run_tuning_fixture(
            "gate",
            "meander_multi_tuning",
            &["--router.tuning.meander=off"],
        );
        assert_eq!(exit_gate, 0);
        let (exit_mt3, ses_mt3, man_mt3) =
            // PRE-TOKENIZED (the false-green fix, buglog 209): the
            // short-flag parser matches the flag TOKEN "mt" and reads
            // the value from the NEXT argv element (settings.rs:390) —
            // a single "-mt 3" element is the unknown-flag "mt 3" and
            // falls into the Java-parity SILENT skip, leaving the
            // effective argv identical to the plain run (the vacuous
            // first-round witness).
            run_tuning_fixture("mt3", "meander_multi_tuning", &["-mt", "3"]);
        assert_eq!(exit_mt3, 0);
        let (exit_mt1, ses_mt1, man_mt1) =
            run_tuning_fixture("mt1", "meander_multi_tuning", &["-mt", "1"]);
        assert_eq!(exit_mt1, 0);
        // Determinism x2 + threads: all ON runs byte-identical.
        let ses_on_bytes = std::fs::read(&ses_on).expect("the pin world invariant");
        assert_eq!(
            ses_on_bytes,
            std::fs::read(&ses_on2).expect("the pin world invariant"),
            "determinism x2"
        );
        assert_eq!(
            ses_on_bytes,
            std::fs::read(&ses_mt1).expect("the pin world invariant"),
            "-mt 1 invariance"
        );
        assert_eq!(
            ses_on_bytes,
            std::fs::read(&ses_mt3).expect("the pin world invariant"),
            "-mt 3 invariance"
        );
        assert_eq!(
            std::fs::read(&man_on).expect("the pin world invariant"),
            std::fs::read(&man_on2).expect("the pin world invariant"),
            "manifest determinism x2"
        );
        assert_eq!(
            std::fs::read(&man_on).expect("the pin world invariant"),
            std::fs::read(&man_mt3).expect("the pin world invariant"),
            "manifest -mt invariance"
        );
        assert_eq!(
            std::fs::read(&man_on).expect("the pin world invariant"),
            std::fs::read(&man_mt1).expect("the pin world invariant"),
            "manifest -mt 1 invariance"
        );
        // ON: both deficits filled -> no length_report key.
        let man_text = std::fs::read_to_string(&man_on).expect("the pin world invariant");
        assert!(
            !man_text.contains("length_report"),
            "both deficits filled in flow: no honest row stands"
        );
        // The mixed-invariance face: the FREE net's wiring bytes are
        // identical ON vs OFF.
        let nf = |ses: &std::path::Path| -> Vec<u8> {
            let text = std::fs::read_to_string(ses).expect("the pin world invariant");
            let mut out = Vec::new();
            for line in text.lines() {
                if line.contains("net NF") {
                    out.extend_from_slice(line.as_bytes());
                    out.push(b'\n');
                }
            }
            out
        };
        // The kill-switch-alone face: tuning stays ON (the honoring
        // gate armed) while the meander stage is dead — both honest
        // rows stand in net-number order, and the free net is STILL
        // byte-identical.
        let man_gate = std::fs::read_to_string(&man_gate).expect("the pin world invariant");
        assert!(
            man_gate.contains("length_report"),
            "the meander kill-switch leaves the deficits standing"
        );
        assert!(man_gate.contains("750000"), "the resolved min");
        // M11-T4 rotation (#931 cluster F): the new door face routes
        // N1 slightly longer — the honest deficit re-derives from the
        // new length (750000 - 701285.72 = -48714.28) — and N2's
        // route now meets its min NATURALLY, so its honest row is
        // gone: the gate world has ONE standing row.
        assert!(
            man_gate.contains("-48714.28"),
            "the N1 deficit (world-derived from the T4 route)"
        );
        let gate_rows: serde_json::Value =
            serde_json::from_str(&man_gate).expect("the manifest parses");
        let honest_rows = gate_rows["length_report"]
            .as_array()
            .expect("the honest rows stand");
        assert_eq!(
            honest_rows.len(),
            1,
            "one honest row stands — N2 meets its min on the T4 route: {honest_rows:?}"
        );
        assert_eq!(honest_rows[0]["net_name"], "N1");
        assert_eq!(
            nf(&ses_on),
            nf(&ses_gate),
            "the constraint-free net is byte-identical with the stage killed too"
        );
        // The REGIME-off face (router.tuning=off): no tuning at all.
        let (exit_off, ses_off, man_off) =
            run_tuning_fixture("off", "meander_multi_tuning", &["--router.tuning=off"]);
        assert_eq!(exit_off, 0);
        assert!(
            !std::fs::read_to_string(&man_off)
                .expect("the pin world invariant")
                .contains("length_report"),
            "the regime-off run emits no report (tuning_active false)"
        );
        assert_eq!(
            nf(&ses_on),
            nf(&ses_off),
            "the constraint-free net is byte-identical ON vs OFF"
        );
        assert!(!nf(&ses_on).is_empty(), "the free net routed in both runs");
    }

    /// PIN F1 (the easy-accordion world, `meander_room_tuning.dsn`):
    /// the pre-routed 700000 straight run with min 740000 — the
    /// deficit 40000 is filled IN FLOW (A=10000, 2 dents, window
    /// 40000), so the manifest carries NO length_report key and the
    /// run completes. The world's values are the committed DSN's.
    #[test]
    fn meander_room_fixture_fills_deficit_in_flow() {
        let (exit, ses, man) = run_tuning_fixture("room", "meander_room_tuning", &[]);
        assert_eq!(exit, 0);
        let man_text = std::fs::read_to_string(&man).expect("the pin world invariant");
        assert!(
            !man_text.contains("length_report"),
            "the deficit is filled in flow: no honest row stands"
        );
        assert!(man_text.contains("COMPLETED"));
        // The SES carries the wave: the emitted N1 wiring is longer
        // than the pre-route (700000) by exactly one window (40000).
        // The emitted coordinates are BOARD DBU (the x10 um scale).
        let ses_text = std::fs::read_to_string(&ses).expect("the pin world invariant");
        let mut in_n1_path = false;
        let mut pts: Vec<i64> = Vec::new();
        for line in ses_text.lines() {
            if line.contains("(net N1") {
                in_n1_path = true;
                continue;
            }
            if in_n1_path {
                if line.contains("(path F.Cu") {
                    continue;
                }
                let trimmed = line.trim();
                if trimmed == ")" {
                    break;
                }
                for token in trimmed.split_whitespace() {
                    if let Ok(v) = token.parse::<i64>() {
                        pts.push(v);
                    }
                }
            }
        }
        assert!(pts.len() >= 6, "the wave added corners");
        let mut manhattan = 0i64;
        let mut i = 0;
        while i + 3 < pts.len() {
            manhattan += (pts[i + 2] - pts[i]).abs() + (pts[i + 3] - pts[i + 1]).abs();
            i += 2;
        }
        assert_eq!(
            manhattan, 740_000,
            "min reached exactly in flow (board DBU)"
        );
    }

    /// PIN F2 (the partially-blocked world, `meander_blocked_tuning.dsn`):
    /// the keepout fence sits at the exact height that blocks EVERY
    /// legal amplitude (A=10000 teeth cross it; A=5000 teeth top face
    /// 256000, gap to the fence face 258000 is 2000 < the probe's
    /// clearance + 16 safety margin), so the HONEST STOP is the flow
    /// face: the run completes with the deficit row standing, exact
    /// world-derived values (min 740000, routed 700000, violation
    /// -40000), and the trace unmeandered.
    #[test]
    fn meander_blocked_fixture_honest_row_in_flow() {
        let (exit, ses, man) = run_tuning_fixture("blocked", "meander_blocked_tuning", &[]);
        assert_eq!(exit, 0);
        let man_text = std::fs::read_to_string(&man).expect("the pin world invariant");
        assert!(man_text.contains("length_report"), "the honest row stands");
        assert!(man_text.contains("740000"), "the resolved min");
        assert!(man_text.contains("700000"), "the unmeandered length");
        assert!(man_text.contains("-40000"), "the exact deficit");
        // The SES: the emitted N1 wiring is the UNCHANGED straight run
        // (two corners only — no wave landed). Board DBU out.
        let ses_text = std::fs::read_to_string(&ses).expect("the pin world invariant");
        let mut in_n1_path = false;
        let mut pts: Vec<i64> = Vec::new();
        for line in ses_text.lines() {
            if line.contains("(net N1") {
                in_n1_path = true;
                continue;
            }
            if in_n1_path {
                if line.contains("(path F.Cu") {
                    continue;
                }
                let trimmed = line.trim();
                if trimmed == ")" {
                    break;
                }
                for token in trimmed.split_whitespace() {
                    if let Ok(v) = token.parse::<i64>() {
                        pts.push(v);
                    }
                }
            }
        }
        assert_eq!(pts.len(), 4, "the straight run only: no wave landed");
    }

    /// M7-T4 (the AMENDMENT-3 bank), re-pinned for #931 cluster G:
    /// the min_stair input-violation PAIR, named. The committed
    /// fixture's PB1/PB2 pads overlap — the ONE parse-time clearance
    /// violation this pin has held world-exactly since M7. #931
    /// drops the same-component guard on the same-net Pin-Pin
    /// exemption, and PB1 (CB1) / PB2 (CB2) share net N2 ACROSS
    /// components: under upstream-post semantics the pair is EXEMPT
    /// and the input board carries ZERO violations — the parse-level
    /// #931 witness (a guard-restoring mutant re-blocks the pair and
    /// dies on the first assert). The knob-off arm walks the frozen
    /// pre-#925b face, where the historical derivation still holds:
    /// exactly the named pair returns.
    #[test]
    fn min_stair_tuning_input_violation_pair_is_named() {
        // The pair derivation (world-exact): the N2 pins PB1/PB2 sit
        // 2000 DSN apart (y 20000 vs 18000) with +-2000-DSN half-size
        // square pads -> the pads OVERLAP (half the pad height), the
        // 0.0 actual of a shape-pair already overlapping in 2D. Item
        // ids at the committed tree: outline 1, pins 2-5 (PA1, PA2,
        // PB1, PB2), pre-routed wires 6 (N1) and 7 (N2).
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../harness/fixtures/tuning/min_stair_tuning.dsn");
        let bytes = std::fs::read(&path).expect("the pin world invariant");
        let mut ses = SesBoard::new();
        match read_board(&bytes, &mut ses) {
            DsnReadResult::Success { .. } => {}
            other => panic!("parse {other:?}"),
        }
        let mut board = Board::from_ses_board(&ses);
        let mut manager = SearchTreeManager::new();
        manager.reinsert_tree_items(&mut board);
        epic_board::normalize_all::normalize_all_traces(&mut manager, &mut board);
        let (total, _depths) =
            epic_drc::clearance::all_clearance_violation_depths(&mut manager, &mut board);
        assert_eq!(
            total, 0,
            "post-#931: same-net pins ACROSS components (CB1/CB2, net N2) are exempt"
        );
        // The frozen face (the P4 corpus law's read): the exemption
        // family off — the named pair returns, world-exact.
        board.rules_mut().same_component_pin_exemptions = false;
        let (total, depths) =
            epic_drc::clearance::all_clearance_violation_depths(&mut manager, &mut board);
        println!("PAIR total={total} depths={depths:?}");
        let mut pairs: Vec<String> = Vec::new();
        for d in &depths {
            pairs.push(format!(
                "item {} vs item {} (layer {}, expected {} / actual {})",
                d.a, d.b, d.layer, d.expected_clearance, d.actual_clearance
            ));
        }
        assert_eq!(total, 1, "the frozen face names exactly one pair");
        assert_eq!(
            pairs[0], "item 4 vs item 5 (layer 0, expected 2000 / actual 0)",
            "the NAMED pair: the N2 pins PB1/PB2 overlapping pads"
        );
        assert!(pairs.len() == 1);
    }
}
