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
//!    resource_usage, cpu_score, settings_snapshot, bounds) — Gson
//!    omits nulls, the harness mirror marks everything optional, so a
//!    skipped key is byte-invisible to the parity gates.
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
        exit_code: exit_code_for(&telemetry.final_state, true),
        output_written: true,
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
/// Unreadable input, a parse error, an unwritable output, or an
/// unwritable manifest is a hard `Err` (exit 1 at the caller); a
/// routing that ends CANCELLED/without output still returns the mapped
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

    // 2. Build the live board + the search tree, normalize the traces
    //    (bug-131 convention: the parse's corner collapse must not leak
    //    into the router's geometry).
    let mut board = Board::from_ses_board(&ses);
    let mut manager = SearchTreeManager::new();
    manager.reinsert_tree_items(&mut board);
    epic_board::normalize_all::normalize_all_traces(&mut manager, &mut board);

    // 3. Resolve the settings: Default(0) -> Dsn(20) -> Cli(60), then
    //    the unconditional geometry pass (bug-compat fact 1).
    let dsn_layer = DsnLayer::from_metadata(
        ses.metadata.autoroute_settings.as_ref(),
        usize::try_from(ses.metadata.layer_count).unwrap_or(0),
    );
    let mut merged = MergedSettings::default();
    merged = merge(&merged, &dsn_layer, &args.layer);
    apply_board_specific_optimizations(&mut merged, &board);
    let resolved = ResolvedRouteSettings::resolve(&merged, args.deterministic_budgets);

    // 4. The batch driver (single-threaded by construction).
    let mut batch = BatchSettings::new(resolved.router_settings, resolved.scoring.clone());
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
    let pre_existing = board.pre_existing_clearance_violations_count;
    let clearance_total = stats.clearance_violations.total_count.unwrap_or(0);
    let clearance_violations = Some(ManifestClearanceViolations {
        total_count: i64::from(clearance_total),
        router_introduced_count: i64::from(clearance_total.saturating_sub(pre_existing)),
    });
    let last_counters = sink.last_counters.take();

    // 6. Project the routed items into the parse SES board and write
    //    the session file.
    project_routed_items(&board, &mut ses);
    let design_name = dsn_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("board.dsn");
    let ses_text = write_session(&ses, design_name);
    std::fs::write(ses_path, ses_text)
        .map_err(|error| format!("cannot write session {}: {error}", ses_path.display()))?;
    let output_written = std::fs::metadata(ses_path)
        .map(|metadata| metadata.len() > 0)
        .unwrap_or(false);
    let exit_code = exit_code_for(&final_state, output_written);

    // 7. The manifest.
    let telemetry = RouteTelemetry {
        final_state,
        last_counters,
        connections,
        clearance_violations,
        normalized_score,
    };
    let manifest = render_manifest(
        &telemetry,
        env!("CARGO_PKG_VERSION"),
        &std::env::var("FREEROUTING_GIT_SHA").unwrap_or_else(|_| "unknown".to_string()),
        ManifestFixture {
            filename: Some(design_name),
            sha256: Some(&sha256_hex(&bytes)),
        },
    );
    if let Some(result_json) = &args.result_json {
        std::fs::write(result_json, manifest)
            .map_err(|error| format!("cannot write manifest {result_json}: {error}"))?;
    }
    Ok(exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::parse_route_args;

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
    /// absent (Gson-omits-nulls parity) while the object still parses.
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

    /// The end-to-end smoke: the locator fixture, one pass, headless —
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
        assert!(ses_text.contains("(session"), "session header");
        assert!(ses_text.contains("t9_locator45"), "design name present");
        let routed_rows = ses_text.matches("(path ").count();
        assert!(
            routed_rows >= 1,
            "at least one routed (path ...) row, got {routed_rows} in:\n{ses_text}"
        );

        let _ = std::fs::remove_dir_all(&out_dir);
    }
}
