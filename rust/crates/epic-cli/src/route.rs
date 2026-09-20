//! The `route` flow (M3-T13): DSN read -> board build + trace
//! normalization -> the T12 batch driver -> SES write + result manifest.
//!
//! Java mirror: `HeadlessBoardManager`'s load-route-save walk
//! (`DsnReader` -> `BasicBoard` -> `BatchAutorouter` -> `SES`), with the
//! manifest carried by `RoutingResultManifest` (schema v1). Two
//! deliberate deviations, both SEAM-documented:
//!
//! 1. **The non-determinism family is OMITTED** from the emitted
//!    manifest (generated_at, all duration_seconds, cpu/memory rows,
//!    resource_usage, cpu_score). This is a DELIBERATE DIVERGENCE, not
//!    a Gson nulls-parity claim: Java EMITS those keys with real
//!    payloads (they are never null in this flow), the harness mirror
//!    reader ignores them, and T13 chooses not to emit them.
//!    `settings_snapshot`/`bounds` fall in the same bucket (Java emits
//!    both — non-null whenever the job ran); `optimizer_score`'s
//!    absence reflects the missing T13 optimizer stage (brief-permitted
//!    and SEAM-documented), not null-elision.
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
use epic_board::items::{FixedState, ItemData};
use epic_board::tree_manager::SearchTreeManager;
use epic_drc::clearance::all_clearance_violation_depths;
use epic_dsn::reader::{DsnReadResult, read_board};
use epic_dsn::ses::writer::write_session;
use epic_dsn::ses_board::{ItemIr, SesBoard};
use epic_dsn::sink::{FixedStateIr, TraceIr, ViaIr};
use epic_geometry::int_point::IntPoint;
use epic_geometry::point::Point;
use epic_geometry::rounding::java_round;
use epic_router::pipeline::batch::{BatchDriver, BatchSettings, StopFace, StopReason};
use epic_router::pipeline::board_statistics::BoardStatistics;
use epic_router::pipeline::event_sink::DriverSink;
use epic_router::pipeline::pass_runner::RouterCounters;
use serde::Serialize;
use sha2::Digest;

use crate::settings::{
    DsnLayer, MergedSettings, ParsedRouteArgs, ResolvedRouteSettings,
    apply_board_specific_optimizations, merge,
};

// ---------------------------------------------------------------------------
// the driver sink: stderr echo + the manifest's counters source
// ---------------------------------------------------------------------------

/// The CLI's driver sink: log rows echo to stderr (the headless host's
/// FRLogger backend), and the LAST `board_updated` counters are kept —
/// the manifest's `phases.autorouter.passes_completed` backfill source
/// (T12 quality MINOR-2: typed counters, never row parsing).
#[derive(Debug, Default)]
pub struct CliDriverSink {
    /// The last progress event's counters (None before the first).
    pub last_counters: Option<RouterCounters>,
}

impl DriverSink for CliDriverSink {
    fn info(&mut self, message: &str) {
        eprintln!("Info: {message}");
    }
    fn warn(&mut self, message: &str) {
        eprintln!("Warning: {message}");
    }
    fn board_updated(&mut self, counters: &RouterCounters) {
        self.last_counters = Some(counters.clone());
    }
}

// ---------------------------------------------------------------------------
// the manifest (schema v1 — harness/src/manifest.rs mirror names)
// ---------------------------------------------------------------------------

/// The autorouter phase's slice. `duration_seconds` is deliberately
/// ABSENT from the type (the non-determinism omission — see module
/// docs); only the deterministic pass count rides.
#[derive(Serialize, Default)]
pub struct ManifestPhaseDetail {
    /// The backfilled pass count: the last counters' `passCount` when
    /// it exceeds 0; absent otherwise (Java emits the phase pass count
    /// only when the phase ran).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passes_completed: Option<i64>,
}

/// The three phase slots (Java always emits the object; empty slots
/// serialize as `{}`).
#[derive(Serialize, Default)]
pub struct ManifestPhases {
    /// The fanout stage — empty in T13 (the stage lands in M4).
    pub fanout: ManifestPhaseDetail,
    /// The batch autorouter.
    pub autorouter: ManifestPhaseDetail,
    /// The optimizer — empty in T13 (no optimizer stage).
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
    /// The router score, 0-1000 (`None` when no scoring face — the
    /// Java iff-gate; unreachable in the T13 flow which always carries
    /// DefaultSettings' scoring box).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_score: Option<f64>,
    /// The optimizer score — ALWAYS absent in T13: no optimizer stage,
    /// so there are no optimizer snapshots to score (the iff-gate).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimizer_score: Option<f64>,
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
    /// The last progress counters (the pass-count backfill source).
    pub last_counters: Option<RouterCounters>,
    /// Post-route connections (`None` = the skip face).
    pub connections: Option<ManifestConnections>,
    /// Post-route clearance violations (`None` = the skip face).
    pub clearance_violations: Option<ManifestClearanceViolations>,
    /// The router score (`None` = the no-scoring-face gate).
    pub normalized_score: Option<f64>,
    /// Whether a session file was actually written with content — the
    /// flow's real write outcome (Java `fromJob`'s `outputWritten`,
    /// `RoutingResultManifest.java:191`), not a constant.
    pub output_written: bool,
}

/// The final-state mapping (module docs for the Java-scheduler rule).
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

/// Renders the manifest JSON (compact — Gson's default is pretty but
/// the mirror parses any whitespace; compact keeps the CLI output
/// stable). The iff-gates live in the Option members.
#[must_use]
pub fn render_manifest(
    telemetry: &RouteTelemetry,
    app_version: &str,
    git_sha: &str,
    fixture: ManifestFixture<'_>,
) -> String {
    let mut phases = ManifestPhases::default();
    // The pass-count backfill: the LAST progress event's pass number,
    // only when the pass runner actually counted a pass (> 0).
    phases.autorouter.passes_completed = telemetry
        .last_counters
        .as_ref()
        .map(|counters| counters.pass_count)
        .filter(|pass_count| *pass_count > 0)
        .map(i64::from);
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
        normalized_score: telemetry.normalized_score,
        // No optimizer stage in T13 -> no optimizer snapshots -> always
        // absent (the iff-gate).
        optimizer_score: None,
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

/// The rounded corner view of a board polyline corner (`corner_to_int`,
/// `io.specctra.scope.Wiring`): `Point::Int` passes through; a rational
/// corner rounds each axis of its float approximation with `Math.round`
/// semantics.
fn corner_to_int(point: Point) -> IntPoint {
    match point {
        Point::Int(p) => p,
        Point::Rational(r) => {
            let float = r.to_float();
            IntPoint::new(java_round(float.x) as i32, java_round(float.y) as i32)
        }
    }
}

/// The fixed-state transfer (`Item.fixedState` — the ladders are
/// 1:1 across the crates).
fn fixed_to_ir(fixed: FixedState) -> FixedStateIr {
    match fixed {
        FixedState::Unfixed => FixedStateIr::Unfixed,
        FixedState::ShoveFixed => FixedStateIr::ShoveFixed,
        FixedState::UserFixed => FixedStateIr::UserFixed,
        FixedState::SystemFixed => FixedStateIr::SystemFixed,
    }
}

/// Projects the LIVE board's routed items (traces + vias, ascending id)
/// into the parse SES board. Deliberately NOT the
/// `BoardSink::insert_trace` face: those apply parse drop guards and
/// fresh ids; the projection preserves the LIVE item's id verbatim
/// (`SesBoard::push_routed_item`).
fn project_routed_items(board: &Board, ses: &mut SesBoard) {
    for entry in board.iter_ascending() {
        if !entry.on_the_board {
            // Java's item walks see live items only.
            continue;
        }
        let id = i32::try_from(entry.id.get()).unwrap_or(0);
        match &entry.data {
            ItemData::Trace {
                layer,
                half_width,
                lines,
            } => {
                let corners: Vec<IntPoint> = (0..i32::try_from(lines.corner_count()).unwrap_or(0))
                    .filter_map(|index| lines.corner(index))
                    .map(corner_to_int)
                    .collect();
                ses.push_routed_item(ItemIr::Trace {
                    id,
                    trace: TraceIr {
                        layer_no: *layer,
                        half_width: *half_width,
                        corners,
                        polyline: lines.clone(),
                        nets: entry.nets.clone(),
                        clearance_class: entry.clearance_class,
                        fixed: fixed_to_ir(entry.fixed),
                    },
                });
            }
            ItemData::Via {
                center,
                padstack_no,
                attach_smd_allowed,
            } => {
                ses.push_routed_item(ItemIr::Via {
                    id,
                    via: ViaIr {
                        padstack_no: *padstack_no,
                        location: *center,
                        nets: entry.nets.clone(),
                        clearance_class: entry.clearance_class,
                        fixed: fixed_to_ir(entry.fixed),
                        attach_smd_allowed: *attach_smd_allowed,
                    },
                });
            }
            // Pins, keepouts, conduction areas and outlines are parse
            // items — already in `ses.items` from the read.
            _ => {}
        }
    }
}

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

/// The `BatchSettings` assembly from the resolved product — the flow's
/// step 4, separated so the wiring is directly pinnable. The
/// load-bearing line is the job-ctor derivation
/// `removeUnconnectedVias = !isFanoutEnabled()`: fanout-off ALSO flips
/// the batch tail sweep, so a silently broken coupling changes routing
/// outcomes, not just via counts.
fn build_batch_settings(resolved: &ResolvedRouteSettings) -> BatchSettings {
    let mut batch = BatchSettings::new(resolved.router_settings.clone(), resolved.scoring.clone());
    batch.max_passes = resolved.max_passes;
    batch.max_items = resolved.max_items;
    batch.fanout_enabled = resolved.fanout_enabled;
    // The job ctor derives removeUnconnectedVias = !isFanoutEnabled().
    batch.remove_unconnected_vias = !resolved.fanout_enabled;
    batch.pull_tight_accuracy = resolved.pull_tight_accuracy;
    batch.strict_drc = resolved.strict_drc;
    batch.run_router = resolved.run_router;
    batch.deterministic_budgets = resolved.deterministic_budgets;
    // The via-cost pair: the merged scoring box (DefaultSettings seeds
    // 50/5; the CLI/DSN layers can override).
    let scoring_box = resolved.scoring.scoring.clone().unwrap_or_default();
    batch.via_costs = scoring_box.via_costs.unwrap_or(50);
    batch.plane_via_costs = scoring_box.plane_via_costs.unwrap_or(5);
    batch
}

/// Java `BoardFileDetails.getFilenameWithoutExtension`
/// (`BoardFileDetails.java:205-210`): the name up to the LAST dot when
/// it contains one, else unchanged. This — not the raw file name — is
/// the job name Java hands `SesWriter.write` (`RoutingJob.java:519`), so
/// the session scope's design face carries NO `.dsn` (the writer's
/// `.replace(".dsn", ".ses")` is a no-op on it).
fn filename_without_extension(filename: &str) -> &str {
    match filename.rfind('.') {
        Some(dot) => &filename[..dot],
        None => filename,
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

    // 2. Build the live board + the search tree, normalize the traces
    //    (bug-131 convention: the parse's corner collapse must not leak
    //    into the router's geometry).
    let mut board = Board::from_ses_board(&ses);
    let mut manager = SearchTreeManager::new();
    manager.reinsert_tree_items(&mut board);
    epic_board::normalize_all::normalize_all_traces(&mut manager, &mut board);

    // 2b. The load-time violation seed (`HeadlessBoardManager.java:
    //     788-793`: `preExistingClearanceViolationsCount =
    //     getAllClearanceViolations().size()` before any routing —
    //     deferred to a background thread there, synchronous here, so
    //     the manifest's router_introduced_count subtracts the REAL
    //     load-time face and never reads the not-yet-measured 0).
    let (pre_total, _) = all_clearance_violation_depths(&mut manager, &mut board);
    board.pre_existing_clearance_violations_count = i32::try_from(pre_total).unwrap_or(i32::MAX);

    // 3. Resolve the settings: Default(0) -> Dsn(20) -> Cli(60), then
    //    the merger's trailing validate() (SettingsMerger.java:189),
    //    then the unconditional geometry pass (bug-compat fact 1).
    let dsn_layer = DsnLayer::from_metadata(
        ses.metadata.autoroute_settings.as_ref(),
        usize::try_from(ses.metadata.layer_count).unwrap_or(0),
    );
    let mut merged = merge(&MergedSettings::default(), &dsn_layer, &args.layer);
    for warning in crate::settings::validate(&mut merged) {
        eprintln!("Warning: {warning}");
    }
    apply_board_specific_optimizations(&mut merged, &board);
    let resolved = ResolvedRouteSettings::resolve(&merged, args.deterministic_budgets);

    // 4. The batch driver (single-threaded by construction).
    let batch = build_batch_settings(&resolved);

    let mut sink = CliDriverSink::default();
    let mut driver = BatchDriver::new(&mut manager, &mut board, batch, StopFace::default());
    let run_result = driver.run(&mut sink);
    let stop_reason = driver.stop_reason;
    drop(driver);

    let final_state: String = match &run_result {
        Err(_) => "TERMINATED".to_string(),
        Ok(true) => "COMPLETED".to_string(),
        Ok(false) => final_state_for(false, stop_reason).to_string(),
    };

    // 5. Post-route statistics (the manifest's board face).
    let stats = BoardStatistics::new(&mut manager, &mut board);
    let normalized_score = Some(f64::from(stats.get_router_score(Some(&resolved.scoring))));
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
    let last_counters = sink.last_counters.take();

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

    // 7. The manifest.
    let telemetry = RouteTelemetry {
        final_state,
        last_counters,
        connections,
        clearance_violations,
        normalized_score,
        output_written,
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
    Ok(exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::parse_route_args;
    use epic_dsn::coordinate_transform::CoordinateTransform;
    use epic_dsn::layer_structure::{Layer, LayerStructure};
    use epic_dsn::sink::{BoardRulesIr, BoardSink, CreateBoardIr};
    use epic_geometry::int_box::IntBox;

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

    /// The counters' pass-count backfill: the LAST counters' passCount
    /// when > 0; absent when there were no events or the count is 0.
    #[test]
    fn manifest_passes_completed_backfill() {
        let counters = RouterCounters {
            phase: "AUTOROUTE".to_string(),
            pass_count: 7,
            queued_to_be_routed_count: 0,
            skipped_count: 0,
            ripped_count: 0,
            failed_to_be_routed_count: 0,
            routed_count: 0,
            incomplete_count: 0,
            fanout_extra_vias_count: 0,
        };
        let mut telemetry = RouteTelemetry {
            final_state: "COMPLETED".to_string(),
            last_counters: Some(counters),
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
        telemetry
            .last_counters
            .as_mut()
            .expect("counters")
            .pass_count = 0;
        let json = render_manifest(&telemetry, "0.1.0", "deadbeef", fixture("b.dsn", "aa"));
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert!(
            value
                .pointer("/phases/autorouter/passes_completed")
                .is_none()
        );

        // No counters at all: absent too.
        telemetry.last_counters = None;
        let json = render_manifest(&telemetry, "0.1.0", "deadbeef", fixture("b.dsn", "aa"));
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert!(
            value
                .pointer("/phases/autorouter/passes_completed")
                .is_none()
        );
    }

    fn fixture<'a>(filename: &'a str, sha: &'a str) -> ManifestFixture<'a> {
        ManifestFixture {
            filename: Some(filename),
            sha256: Some(sha),
        }
    }

    /// The manifest's REQUIRED members and the OMITTED non-determinism
    /// family: the emitted bytes contain NONE of generated_at /
    /// duration / cpu / memory / resource_usage / settings_snapshot /
    /// bounds / optimizer_score, even as keys, while every mirror-
    /// required member is present (schema v1).
    #[test]
    fn manifest_omits_non_determinism_family_and_keeps_required() {
        let counters = RouterCounters {
            phase: "AUTOROUTE".to_string(),
            pass_count: 2,
            queued_to_be_routed_count: 3,
            skipped_count: 0,
            ripped_count: 1,
            failed_to_be_routed_count: 0,
            routed_count: 2,
            incomplete_count: 1,
            fanout_extra_vias_count: 0,
        };
        let telemetry = RouteTelemetry {
            final_state: "COMPLETED".to_string(),
            last_counters: Some(counters),
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

    /// The rounded corner view: Int passes through; a rational corner
    /// rounds with Java Math.round per axis — the 1.5 / -2.5 pair
    /// discriminates floor(x+0.5) from truncation and from
    /// half-away-from-zero (-2.5 -> -2, not -3).
    #[test]
    fn corner_to_int_java_round_view() {
        use num_bigint::BigInt;
        assert_eq!(
            corner_to_int(Point::Int(IntPoint::new(7, -9))),
            IntPoint::new(7, -9)
        );
        assert_eq!(
            corner_to_int(Point::get_instance_big(
                BigInt::from(3),
                BigInt::from(-5),
                BigInt::from(2)
            )),
            IntPoint::new(2, -2)
        );
        // The negated-denominator normalization (z < 0 flips) lands on
        // the same rational point.
        assert_eq!(
            corner_to_int(Point::get_instance_big(
                BigInt::from(-3),
                BigInt::from(5),
                BigInt::from(-2)
            )),
            IntPoint::new(2, -2)
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
    /// load-time violations) the manifest must NOT report the
    /// load-time rows as router-introduced — the seed walk (flow step
    /// 2b) runs BEFORE the driver, so `router_introduced < total`.
    /// Mutant face: deleting the seed block leaves the count at 0 and
    /// `router_introduced == total`.
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
        let total = value["board_statistics"]["clearance_violations"]["total_count"]
            .as_i64()
            .expect("total count present");
        let introduced =
            value["board_statistics"]["clearance_violations"]["router_introduced_count"]
                .as_i64()
                .expect("router-introduced count present");
        assert!(
            total >= 1,
            "the craft board loads with violations (golden drc-0015 = 3), got {total}"
        );
        assert!(
            introduced < total,
            "load-time rows must be excluded from router_introduced \
             (a deleted-seed mutant reports {introduced} == {total})"
        );

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
    /// `(type signal)` rows become `(type power)`. The parse succeeds,
    /// but the driver refuses to start ("Cannot start autorouter: all
    /// layers are disabled") and the run ends TERMINATED. Binary
    /// witness on `2b62b09bc`: exit 1 with a 4290-byte SES of the
    /// unrouted board on disk (Java leaves NO output —
    /// `Freerouting.java:265-267`).
    fn no_signal_layer_dsn() -> String {
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../harness/fixtures/locator-spike/t9_locator45.dsn"
        );
        let text = std::fs::read_to_string(fixture_path).expect("fixture present");
        let patched = text.replace("(type signal)", "(type power)");
        assert_ne!(patched, text, "both layer types patched");
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
    #[test]
    fn batch_settings_wiring_fanout_to_remove_unconnected_vias() {
        let default_batch = build_batch_settings(&ResolvedRouteSettings::resolve(
            &MergedSettings::default(),
            None,
        ));
        assert!(
            default_batch.fanout_enabled,
            "the DefaultSettings seed fans out"
        );
        assert!(
            !default_batch.remove_unconnected_vias,
            "fanout on -> the tail sweep is NOT run"
        );

        let merged = MergedSettings {
            fanout_enabled: Some(false),
            ..MergedSettings::default()
        };
        let batch = build_batch_settings(&ResolvedRouteSettings::resolve(&merged, None));
        assert!(!batch.fanout_enabled);
        assert!(
            batch.remove_unconnected_vias,
            "fanout off -> the tail sweep removes unconnected vias"
        );
    }

    /// Q6 (quality review): the LAST-dot cut (BoardFileDetails.java:
    /// 205-210) — a two-dot input name must keep the middle segment
    /// (the first-dot mutant emitted `(session my` for `my.bad.dsn`).
    #[test]
    fn filename_without_extension_cuts_at_last_dot() {
        assert_eq!(filename_without_extension("my.bad.dsn"), "my.bad");
        assert_eq!(filename_without_extension("plain.dsn"), "plain");
        assert_eq!(filename_without_extension("noext"), "noext");
    }

    /// Q7a (quality review): the fixed-state ladder is permutation-
    /// sensitive — each `FixedState` maps to the SAME-named IR variant
    /// (the UserFixed/SystemFixed arm swap passed the whole suite and
    /// was invisible to every compare before this pin).
    #[test]
    fn fixed_to_ir_ladder_is_permutation_sensitive() {
        assert!(matches!(
            fixed_to_ir(FixedState::Unfixed),
            FixedStateIr::Unfixed
        ));
        assert!(matches!(
            fixed_to_ir(FixedState::ShoveFixed),
            FixedStateIr::ShoveFixed
        ));
        assert!(matches!(
            fixed_to_ir(FixedState::UserFixed),
            FixedStateIr::UserFixed
        ));
        assert!(matches!(
            fixed_to_ir(FixedState::SystemFixed),
            FixedStateIr::SystemFixed
        ));
    }

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
}
